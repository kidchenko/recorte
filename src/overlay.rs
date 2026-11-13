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
//!
//! Until the drag starts there is a small menu at the top (see `menu`), to
//! choose between a snip and a recording of what is selected.

mod menu;
pub use menu::TOP as MENU_TOP;

use crate::capture::Frame;
use crate::editor::Palette;
use crate::hotkey::Action;
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
    /// The menu was used to choose between a snip and a recording. Every
    /// monitor's overlay is to show it: see [`Overlay::set_mode`].
    Mode(Action),
    Done(Region),
}

/// What was selected: a rectangle on the desktop, in physical pixels, and
/// the monitor it is on. A snip is cut from it ([`Overlay::cut`]); a
/// recording films it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// The monitor, as `(x, y, width, height)`.
    pub monitor: (i32, i32, u32, u32),
    /// The monitor's scale factor.
    pub scale: f64,
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
    /// What the selection is for, as the menu shows it.
    mode: Action,
    /// The menu's button under the pointer, and the one held down.
    hover: Option<menu::Hit>,
    pressed: Option<menu::Hit>,
    /// The pointer is over the menu, where it is an arrow and not a cross.
    over_menu: bool,
    look: Palette,
}

impl Overlay {
    pub fn new<T>(
        target: &EventLoopWindowTarget<T>,
        monitor: MonitorHandle,
        frame: &Frame,
        mode: Action,
        unfilmed: bool,
    ) -> Result<Self, String> {
        let pos = monitor.position();
        let size = monitor.size();
        let bright = frame.crop(pos.x, pos.y, size.width, size.height);
        let dim = bright.px.iter().map(|&p| dimmed(p)).collect();

        let builder = WindowBuilder::new()
            .with_title("Recorte")
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top(true)
            .with_visible(false)
            // Brought up while a recording films, it is left out of it.
            .with_content_protection(unfilmed)
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
            mode,
            hover: None,
            pressed: None,
            over_menu: false,
            look: Palette::new(traykit::theme::taskbar_is_dark()),
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

    /// Show `mode` as the one chosen in the menu.
    pub fn set_mode(&mut self, mode: Action) {
        if mode != self.mode {
            self.mode = mode;
            self.view.window().request_redraw();
        }
    }

    /// The menu's button at the pointer. None during a drag: the menu is
    /// put away then, to leave the whole screen to select on.
    fn menu_hit(&self) -> Option<menu::Hit> {
        if self.anchor.is_some() {
            return None;
        }
        let s = self.scale as f32;
        let (x, y) = menu::origin(self.bright.width, s);
        menu::hit((self.cursor.0 - x) as f32, (self.cursor.1 - y) as f32, s)
    }

    /// Whether the pointer is on the menu at all, between its buttons too.
    fn on_menu(&self) -> bool {
        let s = self.scale as f32;
        let (x, y) = menu::origin(self.bright.width, s);
        let (w, h) = menu::size(s);
        let (cx, cy) = (self.cursor.0 - x, self.cursor.1 - y);
        self.anchor.is_none() && cx >= 0 && cy >= 0 && cx < w as i32 && cy < h as i32
    }

    /// A selection worth taking: not a slip of the hand, and for a
    /// recording, not smaller than can be filmed.
    fn enough(&self, (_, _, w, h): (u32, u32, u32, u32)) -> bool {
        let least = match self.mode {
            Action::Snip => 2,
            Action::Record => crate::record::SMALLEST,
        };
        w >= least && h >= least
    }

    pub fn handle(&mut self, event: &WindowEvent) -> Outcome {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x.round() as i32, position.y.round() as i32);
                let hover = self.menu_hit();
                if hover != self.hover {
                    self.hover = hover;
                    self.view.window().request_redraw();
                }
                let over_menu = self.on_menu();
                if over_menu != self.over_menu {
                    self.over_menu = over_menu;
                    self.view.window().set_cursor_icon(if over_menu {
                        CursorIcon::Default
                    } else {
                        CursorIcon::Crosshair
                    });
                }
                if self.anchor.is_some() {
                    self.view.window().request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // On the menu it is a press of a button, or between two of
                // them, of nothing; anywhere else, the start of a selection.
                if self.hover.is_some() {
                    self.pressed = self.hover;
                } else if !self.on_menu() {
                    self.anchor = Some(self.cursor);
                }
                self.view.window().request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(pressed) = self.pressed.take() {
                    self.view.window().request_redraw();
                    // Let go somewhere else, a press is taken back.
                    if self.hover == Some(pressed) {
                        return match pressed {
                            menu::Hit::Mode(mode) => Outcome::Mode(mode),
                            menu::Hit::Close => Outcome::Cancel("closed from the menu"),
                        };
                    }
                    return Outcome::Nothing;
                }
                if let Some(rect) = self.selection().filter(|&r| self.enough(r)) {
                    return Outcome::Done(self.region(rect));
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
                        let whole = (0, 0, self.bright.width, self.bright.height);
                        let rect = self
                            .selection()
                            .filter(|&r| self.enough(r))
                            .unwrap_or(whole);
                        return Outcome::Done(self.region(rect));
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

    /// A selection in window pixels, as a place on the desktop.
    fn region(&self, (x, y, w, h): (u32, u32, u32, u32)) -> Region {
        let m = &self.bright;
        Region {
            x: m.x + x as i32,
            y: m.y + y as i32,
            width: w,
            height: h,
            monitor: (m.x, m.y, m.width, m.height),
            scale: self.scale,
        }
    }

    /// The picture of `region`, from the screen as it was frozen.
    pub fn cut(&self, region: &Region) -> Snip {
        let part = self
            .bright
            .crop(region.x, region.y, region.width, region.height);
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
        // The menu, unless a drag has put it away.
        let menu = if self.anchor.is_none() {
            let state = menu::State {
                mode: self.mode,
                hover: self.hover,
                pressed: self.pressed,
            };
            menu::draw(self.scale as f32, &state, &self.look)
                .map(|pm| (pm, menu::origin(fw, self.scale as f32)))
        } else {
            None
        };
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
            if let Some((pm, at)) = &menu {
                menu::blend(buf, w as usize, pm, *at);
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
