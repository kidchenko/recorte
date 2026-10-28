//! The frozen screen you drag a selection on: one borderless, topmost
//! window per monitor, showing that monitor's part of the capture, dimmed,
//! with the selection bright.
//!
//! One window per monitor rather than one across the desktop, because each
//! monitor can have its own scale factor and a window has only one.
//!
//! Every frame is two copies: the dimmed picture, then the selected rows
//! of the bright one over it. Both are prepared once, when the window opens,
//! so dragging costs a memory copy per mouse move, a few milliseconds even
//! at 4K.

use crate::capture::Frame;
use crate::snip::Snip;
use crate::view::View;
use tao::dpi::PhysicalPosition;
use tao::event::{ElementState, MouseButton, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::keyboard::Key;
use tao::monitor::MonitorHandle;
use tao::window::{CursorIcon, Fullscreen, WindowBuilder, WindowId};

/// How much of the screen's brightness is left outside the selection.
const DIM: u32 = 140; // of 255

pub enum Outcome {
    Nothing,
    /// Why, for the log: Esc, a right click, or the window being closed.
    Cancel(&'static str),
    Done(Snip),
}

pub struct Overlay {
    view: View,
    /// The monitor's part of the capture, and the same dimmed.
    bright: Frame,
    dim: Vec<u32>,
    scale: f64,
    cursor: (i32, i32),
    /// Where the drag started, in window pixels.
    anchor: Option<(i32, i32)>,
}

impl Overlay {
    pub fn new<T>(
        target: &EventLoopWindowTarget<T>,
        monitor: MonitorHandle,
        frame: &Frame,
    ) -> Result<Self, String> {
        let pos = monitor.position();
        let size = monitor.size();
        let bright = frame.crop(pos.x, pos.y, size.width, size.height);
        let dim = bright.px.iter().map(|&p| dimmed(p)).collect();

        let builder = WindowBuilder::new()
            .with_title("Print")
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top(true)
            .with_visible(false)
            .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        #[cfg(windows)]
        let builder = {
            use tao::platform::windows::WindowBuilderExtWindows;
            builder.with_skip_taskbar(true)
        };
        let window = builder
            .build(target)
            .map_err(|e| format!("could not open the overlay: {e}"))?;
        window.set_cursor_icon(CursorIcon::Crosshair);

        let mut overlay = Self {
            view: View::new(window)?,
            bright,
            dim,
            scale: monitor.scale_factor(),
            cursor: (0, 0),
            anchor: None,
        };
        // Painted before it is shown, so the screen never flashes white.
        overlay.redraw();
        overlay.view.window().set_visible(true);
        Ok(overlay)
    }

    pub fn id(&self) -> WindowId {
        self.view.window().id()
    }

    pub fn focus(&self) {
        self.view.window().set_focus();
    }

    pub fn handle(&mut self, event: &WindowEvent) -> Outcome {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x.round() as i32, position.y.round() as i32);
                if self.anchor.is_some() {
                    self.view.window().request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.anchor = Some(self.cursor);
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(rect) = self.selection() {
                    if rect.2 >= 2 && rect.3 >= 2 {
                        return Outcome::Done(self.cut(rect));
                    }
                }
                // A click without a drag selects nothing; keep waiting.
                self.anchor = None;
                self.view.window().request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => return Outcome::Cancel("right click"),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Escape => return Outcome::Cancel("Esc"),
                    // Enter with nothing selected takes the whole monitor.
                    Key::Enter => {
                        let rect = self.selection().unwrap_or((
                            0,
                            0,
                            self.bright.width,
                            self.bright.height,
                        ));
                        return Outcome::Done(self.cut(rect));
                    }
                    _ => {}
                }
            }
            WindowEvent::CloseRequested => return Outcome::Cancel("closed"),
            // Not a cancel, but the usual reason a snip goes wrong: another
            // window (Snipping Tool's own overlay, say) took the focus.
            WindowEvent::Focused(false) => traykit::linfo!("overlay lost the focus"),
            _ => {}
        }
        Outcome::Nothing
    }

    /// The selection as `(x, y, width, height)` in window pixels, clipped to
    /// the monitor.
    fn selection(&self) -> Option<(u32, u32, u32, u32)> {
        let (ax, ay) = self.anchor?;
        let (bx, by) = self.cursor;
        let clamp_x = |v: i32| v.clamp(0, self.bright.width as i32) as u32;
        let clamp_y = |v: i32| v.clamp(0, self.bright.height as i32) as u32;
        let (x0, x1) = (clamp_x(ax.min(bx)), clamp_x(ax.max(bx)));
        let (y0, y1) = (clamp_y(ay.min(by)), clamp_y(ay.max(by)));
        Some((x0, y0, x1 - x0, y1 - y0))
    }

    fn cut(&self, (x, y, w, h): (u32, u32, u32, u32)) -> Snip {
        let part = self
            .bright
            .crop(self.bright.x + x as i32, self.bright.y + y as i32, w, h);
        Snip {
            width: part.width,
            height: part.height,
            rgba: part.to_rgba(),
            scale: self.scale,
        }
    }

    pub fn redraw(&mut self) {
        let sel = self.selection();
        let (fw, fh) = (self.bright.width, self.bright.height);
        let bright = &self.bright.px;
        let dim = &self.dim;
        let border = self.scale.round().max(1.0) as u32;
        self.view.present(|buf, w, h| {
            // The window should be exactly the monitor; if the system made it
            // a little different, paint what overlaps and black elsewhere.
            let cw = w.min(fw) as usize;
            for row in 0..h.min(fh) as usize {
                let dst = &mut buf[row * w as usize..][..cw];
                dst.copy_from_slice(&dim[row * fw as usize..][..cw]);
            }
            if fw < w || fh < h {
                for row in 0..h as usize {
                    for col in 0..w as usize {
                        if col >= cw || row >= fh as usize {
                            buf[row * w as usize + col] = 0;
                        }
                    }
                }
            }
            let Some((x, y, sw, sh)) = sel else { return };
            let (x, y) = (x as usize, y as usize);
            let sw = (sw as usize).min(cw.saturating_sub(x));
            let sh = (sh as usize).min((h.min(fh) as usize).saturating_sub(y));
            for row in y..y + sh {
                let at = row * w as usize + x;
                buf[at..at + sw].copy_from_slice(&bright[row * fw as usize + x..][..sw]);
            }
            outline(buf, w as usize, (x, y, sw, sh), border as usize);
        });
    }
}

fn dimmed(p: u32) -> u32 {
    let ch = |shift: u32| (((p >> shift) & 0xff) * DIM / 255) << shift;
    ch(16) | ch(8) | ch(0)
}

/// A white frame just outside the selection, `t` pixels thick.
fn outline(buf: &mut [u32], stride: usize, (x, y, w, h): (usize, usize, usize, usize), t: usize) {
    let rows = buf.len() / stride;
    let x0 = x.saturating_sub(t);
    let y0 = y.saturating_sub(t);
    let x1 = (x + w + t).min(stride);
    let y1 = (y + h + t).min(rows);
    for row in y0..y1 {
        let line = &mut buf[row * stride..(row + 1) * stride];
        if row < y || row >= y + h {
            line[x0..x1].fill(0x00ff_ffff);
        } else {
            line[x0..x.min(x1)].fill(0x00ff_ffff);
            line[(x + w).min(x1)..x1].fill(0x00ff_ffff);
        }
    }
}

/// The monitor under the pointer, for which overlay gets the keyboard.
pub fn monitor_at<T>(target: &EventLoopWindowTarget<T>) -> Option<MonitorHandle> {
    let PhysicalPosition { x, y } = target.cursor_position().ok()?;
    target.monitor_from_point(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outline_surrounds_the_selection() {
        let mut buf = vec![0u32; 6 * 5];
        outline(&mut buf, 6, (2, 1, 2, 2), 1);
        let white = |c: usize, r: usize| buf[r * 6 + c] == 0x00ff_ffff;
        assert!(white(1, 0) && white(4, 0) && white(1, 2) && white(4, 3));
        assert!(!white(2, 1) && !white(3, 2), "the inside stays as it was");
        assert!(!white(0, 0) && !white(5, 4));
    }

    #[test]
    fn dimming_keeps_black_black() {
        assert_eq!(dimmed(0), 0);
        assert_eq!(dimmed(0x00ff_ffff), 0x008c_8c8c);
    }
}
