//! A recording of a part of the screen: a frame around the area, a small
//! bar at the top of the monitor to start and stop, and the film going into
//! an MP4.
//!
//! The area is chosen on the same overlay as a snip. Then nothing is filmed
//! yet: the frame and the bar wait, so there is time to get the screen
//! ready. The record button, or the hotkey again, starts it and the frame
//! turns from white to red; the hotkey again, or the stop button, stops it
//! and the file is saved. The cross closes it all without a file.
//!
//! Once started, the record button is a pause: the film holds, the frame is
//! white again, and what happens until it carries on is not in the film. The
//! bar also has the microphone and what the computer plays, each in the
//! recording or not, which can be changed while it films. Once there is a
//! film to lose, the cross is a bin, and has to be pressed twice.
//!
//! The bar can be dragged by any part of it that is not a button, for when
//! it is over something that is needed.
//!
//! The frame is a dashed hairline with a heavier bracket at each corner. It
//! is four windows, one a side, each cut down to the line and its brackets
//! so that the screen shows, and takes the mouse, everywhere else.
//!
//! The frame and the bar are windows that take no focus, so what is being
//! filmed keeps the keyboard, and that the system leaves out of a capture,
//! so they are not in the film even where they overlap the area.
//!
//! The filming itself is per system; see `win`.

mod bar;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::{clip, Recorder};

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other::{clip, Recorder};

use crate::editor::Palette;
use crate::overlay::{Region, MENU_TOP};
use crate::view::View;
use bar::{Hit, Phase};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{ElementState, MouseButton, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::window::{WindowBuilder, WindowId};
use tiny_skia::Pixmap;
use traykit::{linfo, lwarn};

/// The frame, in logical pixels: how thick and how long a corner's bracket
/// is, how thick the line between them, and how long a dash of it.
const CORNER: f64 = 3.0;
const ARM: f64 = 18.0;
const LINE: f64 = 1.0;
const DASH: f64 = 4.0;
/// The least an area can be a side, in pixels: the system's encoder takes
/// nothing much smaller.
pub const SMALLEST: u32 = 64;
/// How long after it starts a press on the bar or the hotkey is not taken:
/// the start takes a moment, and a second press made in that moment, to be
/// sure, would otherwise pause or stop it at once.
const SETTLE: Duration = Duration::from_millis(500);
/// How long the bin stays ready for its second press.
const ARMED: Duration = Duration::from_secs(3);
/// Half of the paused clock's blink.
const BLINK: Duration = Duration::from_millis(500);
/// Between the dashes, so the line shows over white as well as black.
const GAP_INK: u32 = 0x0020_2020;

/// What is filmed: a rectangle on the desktop, in physical pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    /// The selection, a pixel narrower or shorter where it has to be: H.264
    /// only has even sizes.
    fn of(region: &Region) -> Self {
        Self {
            x: region.x,
            y: region.y,
            width: region.width & !1,
            height: region.height & !1,
        }
    }
}

pub enum Request {
    Nothing,
    /// It is filming now.
    Started,
    /// Closed without a recording.
    Cancelled,
    Saved(PathBuf),
    Failed(String),
}

pub struct Recording {
    bar: View,
    /// The four sides of the frame, each a thin window.
    frame: Vec<Side>,
    /// A dash of the frame's line, in pixels.
    dash: i32,
    area: Area,
    /// The folder the film is saved to.
    dir: PathBuf,
    film: Option<Film>,
    /// The monitor it is on, which the bar is kept within.
    monitor: (i32, i32, u32, u32),
    /// The seconds the bar last showed.
    seconds: u64,
    /// The encoder is being set up, which the bar shows meanwhile.
    starting: bool,
    /// Until when a press is not taken: see `SETTLE`.
    settle: Option<Instant>,
    /// Since when it is paused, and which half of the clock's blink it is.
    paused: Option<Instant>,
    blink: bool,
    /// The bin was pressed: until when a second press throws the film away.
    armed: Option<Instant>,
    /// Where on the bar the pointer took hold of it, while it is dragged.
    grab: Option<(f32, f32)>,
    /// The microphone, and what the computer plays: in the film or not.
    mic: bool,
    system: bool,
    cursor: (f32, f32),
    hover: Option<Hit>,
    pressed: Option<Hit>,
    look: Palette,
}

/// One side of the frame.
struct Side {
    view: View,
    /// Left and right; the top and the bottom lie flat.
    vertical: bool,
    length: i32,
    /// How far its brackets reach in from each end.
    arm: i32,
}

/// A recording under way.
struct Film {
    recorder: Recorder,
    path: PathBuf,
    /// How long it had filmed when it last carried on, and when that was;
    /// `None` while it is paused.
    before: Duration,
    since: Option<Instant>,
}

impl Film {
    /// How long the film is at `now`, pauses left out.
    fn length(&self, now: Instant) -> Duration {
        let running = self.since.map(|s| now.saturating_duration_since(s));
        self.before + running.unwrap_or_default()
    }
}

impl Recording {
    /// Put the frame and the bar around `region`, ready to start, with the
    /// microphone and the computer's sound switched as given.
    pub fn new<T>(
        target: &EventLoopWindowTarget<T>,
        region: Region,
        dir: PathBuf,
        mic: bool,
        system: bool,
    ) -> Result<Self, String> {
        let area = Area::of(&region);
        if area.width < SMALLEST || area.height < SMALLEST {
            return Err(format!(
                "{}x{} is too small to record",
                area.width, area.height
            ));
        }
        let px = |logical: f64| (logical * region.scale).round().max(1.0) as i32;
        let (thick, line) = (px(CORNER), px(LINE));
        let frame = sides(area, region.monitor, thick)
            .into_iter()
            .enumerate()
            .map(|(i, rect)| {
                // Top, bottom, left, right. The line is on the area's side.
                let vertical = i >= 2;
                let length = if vertical { rect.3 } else { rect.2 } as i32;
                // The top and the bottom have the corners themselves.
                let arm = if vertical { px(ARM) - thick } else { px(ARM) };
                let arm = arm.clamp(0, length / 3);
                let line_at = if i % 2 == 0 { thick - line } else { 0 };
                let shape = shape(vertical, length, thick, line, arm, line_at);
                Ok(Side {
                    view: window(target, rect, &shape)?,
                    vertical,
                    length,
                    arm,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let size = (
            (bar::WIDTH as f64 * region.scale).round() as u32,
            (bar::HEIGHT as f64 * region.scale).round() as u32,
        );
        let (x, y) = bar_place(region.monitor, size.0, px(MENU_TOP as f64));
        let whole = [(0, 0, size.0 as i32, size.1 as i32)];
        let bar = window(target, (x, y, size.0, size.1), &whole)?;

        let mut recording = Self {
            bar,
            frame,
            dash: px(DASH),
            area,
            dir,
            film: None,
            monitor: region.monitor,
            seconds: 0,
            starting: false,
            settle: None,
            paused: None,
            blink: false,
            armed: None,
            grab: None,
            mic,
            system,
            cursor: (0.0, 0.0),
            hover: None,
            pressed: None,
            look: Palette::new(traykit::theme::taskbar_is_dark()),
        };
        // Painted before they are shown, so nothing flashes white.
        recording.paint();
        for view in recording.views() {
            view.window().set_visible(true);
        }
        Ok(recording)
    }

    fn views(&self) -> impl Iterator<Item = &View> {
        self.frame.iter().map(|s| &s.view).chain([&self.bar])
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.views().any(|v| v.window().id() == id)
    }

    /// Started, and not yet stopped.
    pub fn is_filming(&self) -> bool {
        self.film.is_some()
    }

    /// The microphone and the computer's sound, as the bar has them now.
    pub fn sound(&self) -> (bool, bool) {
        (self.mic, self.system)
    }

    fn phase(&self) -> Phase {
        match &self.film {
            None if self.starting => Phase::Starting,
            None => Phase::Ready,
            Some(film) if film.since.is_some() => Phase::Filming,
            Some(_) => Phase::Paused,
        }
    }

    /// The hotkey: start, or stop and keep. The stop button is the second
    /// half of it.
    pub fn toggle(&mut self) -> Request {
        if self.settling() {
            return Request::Nothing;
        }
        self.finish()
    }

    fn settling(&self) -> bool {
        self.settle.is_some_and(|until| Instant::now() < until)
    }

    fn finish(&mut self) -> Request {
        match self.film.take() {
            Some(film) => match film.recorder.stop() {
                Ok(()) => Request::Saved(film.path),
                Err(e) => Request::Failed(e),
            },
            None => self.start(),
        }
    }

    fn start(&mut self) -> Request {
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            return Request::Failed(format!("{}: {e}", self.dir.display()));
        }
        let name = crate::snip::file_name(traykit::time::LocalTime::now(), "mp4");
        let path = crate::snip::fresh_path(&self.dir, &name);
        // Setting up the encoder holds this thread for up to a second, so
        // show that the press was taken before, not after.
        self.starting = true;
        self.paint();
        let started = Recorder::start(self.area, &path, self.mic, self.system);
        self.starting = false;
        match started {
            Ok(recorder) => {
                linfo!(
                    "recording {}x{} at ({}, {}) to {}",
                    self.area.width,
                    self.area.height,
                    self.area.x,
                    self.area.y,
                    path.display()
                );
                self.film = Some(Film {
                    recorder,
                    path,
                    before: Duration::ZERO,
                    since: Some(Instant::now()),
                });
                self.seconds = 0;
                self.settle = Some(Instant::now() + SETTLE);
                self.paint();
                Request::Started
            }
            Err(e) => Request::Failed(e),
        }
    }

    /// The record button once it films: hold the film, or carry on.
    fn pause(&mut self) {
        let Some(film) = &mut self.film else {
            return;
        };
        match film.since.take() {
            Some(since) => {
                film.before += since.elapsed();
                film.recorder.set_paused(true);
                self.paused = Some(Instant::now());
            }
            None => {
                film.since = Some(Instant::now());
                film.recorder.set_paused(false);
                self.paused = None;
            }
        }
        self.blink = false;
        self.paint();
    }

    /// The microphone's button and the computer's sound's: in the film from
    /// here on, or not.
    fn switch(&mut self, hit: Hit) {
        if hit == Hit::Mic {
            self.mic = !self.mic;
        } else {
            self.system = !self.system;
        }
        if let Some(film) = &self.film {
            film.recorder.set_mic(self.mic);
            film.recorder.set_system(self.system);
        }
        self.bar.window().request_redraw();
    }

    /// The cross: close. Once it films it is a bin, which deletes the film,
    /// and so wants pressing twice: the first press only makes it ready.
    fn discard(&mut self) -> Request {
        let now = Instant::now();
        if self.film.is_some() && !self.armed.is_some_and(|until| now < until) {
            self.armed = Some(now + ARMED);
            self.bar.window().request_redraw();
            return Request::Nothing;
        }
        if let Some(film) = self.film.take() {
            if let Err(e) = film.recorder.stop() {
                lwarn!("{e}");
            }
            if let Err(e) = std::fs::remove_file(&film.path) {
                lwarn!("could not delete {}: {e}", film.path.display());
            }
        }
        Request::Cancelled
    }

    pub fn handle(&mut self, id: WindowId, event: &WindowEvent) -> Request {
        // The frame is only looked at.
        if id != self.bar.window().id() {
            return Request::Nothing;
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                match self.grab {
                    Some(grab) => self.drag(grab),
                    None => self.hover_on(bar::hit(self.cursor.0, self.cursor.1, self.scale())),
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.disarm();
                self.hover_on(None);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.pressed = self.hover;
                // Anywhere that is not a button is a handle to move it by.
                if self.hover.is_none() {
                    self.grab = Some(self.cursor);
                }
                self.bar.window().request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.grab = None;
                if let Some(pressed) = self.pressed.take() {
                    self.bar.window().request_redraw();
                    if self.hover == Some(pressed) {
                        if pressed != Hit::Discard {
                            self.disarm();
                        }
                        match pressed {
                            Hit::Main | Hit::Stop if self.settling() => {}
                            Hit::Main if self.film.is_none() => return self.start(),
                            Hit::Main => self.pause(),
                            Hit::Stop if self.film.is_some() => return self.finish(),
                            Hit::Stop => {}
                            Hit::Mic | Hit::System => self.switch(pressed),
                            Hit::Discard => return self.discard(),
                        }
                    }
                }
            }
            _ => {}
        }
        Request::Nothing
    }

    fn disarm(&mut self) {
        if self.armed.take().is_some() {
            self.bar.window().request_redraw();
        }
    }

    /// Move the bar with the pointer that holds it at `grab`, as far as the
    /// edges of its monitor.
    fn drag(&mut self, grab: (f32, f32)) {
        let window = self.bar.window();
        let (Ok(outer), Ok(inner)) = (window.outer_position(), window.inner_position()) else {
            return;
        };
        // The window is placed by its own corner and seen from that of its
        // pixels, which can be a few pixels in from it.
        let (dx, dy) = (inner.x - outer.x, inner.y - outer.y);
        let (w, h) = self.bar.size();
        let (mx, my, mw, mh) = self.monitor;
        let x = inner.x + (self.cursor.0 - grab.0).round() as i32;
        let y = inner.y + (self.cursor.1 - grab.1).round() as i32;
        let x = x.min(mx + mw as i32 - w as i32).max(mx);
        let y = y.min(my + mh as i32 - h as i32).max(my);
        if (x, y) != (inner.x, inner.y) {
            window.set_outer_position(PhysicalPosition::new(x - dx, y - dy));
        }
    }

    fn hover_on(&mut self, hover: Option<Hit>) {
        if hover != self.hover {
            self.hover = hover;
            self.bar.window().request_redraw();
        }
    }

    /// When the bar next needs a tick: the clock's next second while it
    /// runs, its next blink while it is paused, and the bin closing again.
    pub fn deadline(&self) -> Option<Instant> {
        let film = self.film.as_ref()?;
        let clock = match (film.since, self.paused) {
            (Some(since), _) => {
                let next = Duration::from_secs(self.seconds + 1);
                Some(since + next.saturating_sub(film.before))
            }
            (None, Some(paused)) => {
                let halves = paused.elapsed().as_millis() / BLINK.as_millis() + 1;
                Some(paused + BLINK * halves as u32)
            }
            (None, None) => None,
        };
        [clock, self.armed].into_iter().flatten().min()
    }

    pub fn tick(&mut self, now: Instant) -> Request {
        if self.armed.is_some_and(|until| now >= until) {
            self.disarm();
        }
        let blink = self.paused.is_some_and(|paused| {
            let halves = now.saturating_duration_since(paused).as_millis() / BLINK.as_millis();
            halves % 2 == 1
        });
        if blink != self.blink {
            self.blink = blink;
            self.bar.window().request_redraw();
        }
        let Some(film) = &self.film else {
            return Request::Nothing;
        };
        // It only stops by itself on an error, which stopping it returns.
        if film.recorder.is_over() {
            return self.finish();
        }
        let seconds = film.length(now).as_secs();
        if seconds != self.seconds {
            self.seconds = seconds;
            self.bar.window().request_redraw();
        }
        Request::Nothing
    }

    /// The bar is sized from the monitor's scale; this is that scale back,
    /// whatever the system made of the size.
    fn scale(&self) -> f32 {
        self.bar.size().1 as f32 / bar::HEIGHT
    }

    pub fn redraw(&mut self, id: WindowId) {
        if id == self.bar.window().id() {
            self.paint_bar();
        } else {
            self.paint_frame();
        }
    }

    fn paint(&mut self) {
        self.paint_frame();
        self.paint_bar();
    }

    /// Red and whole while it films; white and dashed while it waits or is
    /// paused, so it is not by its colour alone that the two are told apart.
    fn paint_frame(&mut self) {
        let filming = matches!(self.phase(), Phase::Filming | Phase::Starting);
        let ink = if filming {
            let [r, g, b] = bar::RED;
            (r as u32) << 16 | (g as u32) << 8 | b as u32
        } else {
            0x00ff_ffff
        };
        // One dash as long as any side is a line with no gaps.
        let dash = if filming { i32::MAX } else { self.dash };
        for side in &mut self.frame {
            let (vertical, length, arm) = (side.vertical, side.length, side.arm);
            // The window is cut to the line and the brackets, so all there
            // is to paint is how the colour changes along the side.
            side.view.present(|buf, w, _| {
                for (row, pixels) in buf.chunks_mut(w.max(1) as usize).enumerate() {
                    if vertical {
                        let on = inked(row as i32, length, arm, dash);
                        pixels.fill(if on { ink } else { GAP_INK });
                    } else {
                        for (x, px) in pixels.iter_mut().enumerate() {
                            let on = inked(x as i32, length, arm, dash);
                            *px = if on { ink } else { GAP_INK };
                        }
                    }
                }
            });
        }
    }

    fn paint_bar(&mut self) {
        let (w, h) = self.bar.size();
        let Some(mut pm) = Pixmap::new(w, h) else {
            return;
        };
        let state = bar::State {
            phase: self.phase(),
            seconds: self.seconds,
            mic: self.mic,
            system: self.system,
            blink: self.blink,
            armed: self.armed.is_some(),
            hover: self.hover,
            pressed: self.pressed,
        };
        bar::draw(&mut pm, self.scale(), &state, &self.look);
        self.bar.present_pixmap(&pm);
    }
}

impl Drop for Recording {
    /// Closed while filming (Recorte is quitting): keep what there is.
    fn drop(&mut self) {
        if let Some(film) = self.film.take() {
            match film.recorder.stop() {
                Ok(()) => linfo!("recording saved to {}", film.path.display()),
                Err(e) => lwarn!("{e}"),
            }
        }
    }
}

/// A bare window at `(x, y, width, height)` in physical pixels: on top of
/// everything, never focused, and left out of screen captures. Only `shape`
/// of it shows, rectangles from its corner.
///
/// The system has a smallest size for a window, far wider than the frame is
/// thick, and gives a window that asks for less that size without a word.
/// Cutting it to its shape is what makes a side of the frame thin.
fn window<T>(
    target: &EventLoopWindowTarget<T>,
    (x, y, width, height): (i32, i32, u32, u32),
    shape: &[(i32, i32, i32, i32)],
) -> Result<View, String> {
    let builder = WindowBuilder::new()
        .with_title("Recorte")
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_visible(false)
        .with_focused(false)
        .with_focusable(false)
        .with_content_protection(true)
        .with_position(PhysicalPosition::new(x, y))
        .with_inner_size(PhysicalSize::new(width, height));
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_skip_taskbar(true)
    };
    let window = builder
        .build(target)
        .map_err(|e| format!("could not open the recording frame: {e}"))?;
    clip(&window, (x, y), shape);
    View::new(window)
}

/// What shows of one side of the frame, as rectangles from its corner: a
/// bracket `arm` long at each end, the full `thick`ness, and between them a
/// line `line` thick, `line_at` pixels across the side.
fn shape(
    vertical: bool,
    length: i32,
    thick: i32,
    line: i32,
    arm: i32,
    line_at: i32,
) -> [(i32, i32, i32, i32); 3] {
    // Along the side, across it, and the sizes the same way.
    [
        (0, 0, arm, thick),
        (length - arm, 0, arm, thick),
        (0, line_at, length, line),
    ]
    .map(|(along, across, long, wide)| {
        if vertical {
            (across, along, wide, long)
        } else {
            (along, across, long, wide)
        }
    })
}

/// Whether the frame is in its own colour `along` a side: all of a bracket,
/// then every other dash of the line.
fn inked(along: i32, length: i32, arm: i32, dash: i32) -> bool {
    along < arm || along >= length - arm || (along - arm) / dash.max(1) % 2 == 0
}

/// The frame around `area`, `thick` pixels wide, as four rectangles: top,
/// bottom, left, right. It goes outside the area, and where the monitor ends
/// there it stays on the monitor, over the area's edge.
fn sides(area: Area, monitor: (i32, i32, u32, u32), thick: i32) -> [(i32, i32, u32, u32); 4] {
    let (mx, my, mw, mh) = monitor;
    let left = (area.x - thick).max(mx);
    let top = (area.y - thick).max(my);
    let right = (area.x + area.width as i32 + thick).min(mx + mw as i32);
    let bottom = (area.y + area.height as i32 + thick).min(my + mh as i32);
    let width = (right - left).max(1) as u32;
    // The left and right sides fit between the other two.
    let inner = (bottom - top - 2 * thick).max(1) as u32;
    let t = thick as u32;
    [
        (left, top, width, t),
        (left, bottom - thick, width, t),
        (left, top + thick, t, inner),
        (right - thick, top + thick, t, inner),
    ]
}

/// Where the bar goes: the middle of the monitor's top, `top` pixels down,
/// where the overlay's menu was a moment before.
fn bar_place(monitor: (i32, i32, u32, u32), width: u32, top: i32) -> (i32, i32) {
    let (mx, my, mw, _) = monitor;
    (mx + (mw as i32 - width as i32) / 2, my + top)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: (i32, i32, u32, u32) = (0, 0, 1920, 1080);

    fn area(x: i32, y: i32, width: u32, height: u32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn the_area_has_even_sides() {
        let region = Region {
            x: 10,
            y: 20,
            width: 301,
            height: 200,
            monitor: MONITOR,
            scale: 1.0,
        };
        assert_eq!(Area::of(&region), area(10, 20, 300, 200));
    }

    #[test]
    fn the_frame_goes_around_the_area() {
        let [top, bottom, left, right] = sides(area(100, 200, 300, 150), MONITOR, 3);
        assert_eq!(top, (97, 197, 306, 3));
        assert_eq!(bottom, (97, 350, 306, 3));
        assert_eq!(left, (97, 200, 3, 150));
        assert_eq!(right, (400, 200, 3, 150));
    }

    #[test]
    fn the_frame_stays_on_the_monitor() {
        // The whole monitor: nowhere outside to go.
        let [top, bottom, left, right] = sides(area(0, 0, 1920, 1080), MONITOR, 3);
        assert_eq!(top, (0, 0, 1920, 3));
        assert_eq!(bottom, (0, 1077, 1920, 3));
        assert_eq!(left, (0, 3, 3, 1074));
        assert_eq!(right, (1917, 3, 3, 1074));
        // A monitor left of the primary one has negative coordinates.
        let second = (-1280, 0, 1280, 1024);
        let [top, _, left, _] = sides(area(-1280, 100, 200, 100), second, 3);
        assert_eq!(top, (-1280, 97, 203, 3));
        assert_eq!(left, (-1280, 100, 3, 100));
    }

    #[test]
    fn the_bar_is_at_the_top_of_its_monitor() {
        assert_eq!(bar_place(MONITOR, 266, 20), (827, 20));
        assert_eq!(bar_place((-1280, 200, 1280, 1024), 266, 20), (-773, 220));
    }

    #[test]
    fn a_side_shows_its_brackets_and_its_line() {
        // The top: the line along its lower edge, next to the area.
        assert_eq!(
            shape(false, 306, 3, 1, 18, 2),
            [(0, 0, 18, 3), (288, 0, 18, 3), (0, 2, 306, 1)]
        );
        // The right: the line down its left edge.
        assert_eq!(
            shape(true, 150, 3, 1, 15, 0),
            [(0, 0, 3, 15), (0, 135, 3, 15), (0, 0, 1, 150)]
        );
    }

    #[test]
    fn the_line_is_dashed_between_the_brackets() {
        let on = |along| inked(along, 100, 18, 4);
        assert!(on(0) && on(17) && on(82) && on(99));
        // Four on, four off, from where the bracket ends.
        assert!(on(18) && on(21));
        assert!(!on(22) && !on(25));
        assert!(on(26));
        // While it films the dash is longer than the side: no gaps.
        assert!((0..100).all(|along| inked(along, 100, 18, i32::MAX)));
    }
}
