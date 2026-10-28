//! The markup window: the snip under a toolbar, drawn on with a pen, a
//! highlighter, arrows, boxes and blur, then copied or saved.
//!
//! Keys: P H A R B pick a tool, 1 to 6 a colour, [ and ] the size;
//! Ctrl+Z / Ctrl+Y undo and redo, Ctrl+C copies, Ctrl+S saves, Ctrl+N
//! starts a new snip, Esc closes.
//!
//! Each frame is the cached background (canvas colour plus the picture,
//! scaled to fit) copied, the marks drawn over it, then the toolbar. The
//! cache is rebuilt only when the window resizes or a blur changes the
//! picture itself.

mod shapes;
mod toolbar;

use crate::snip::Snip;
use crate::view::View;
use shapes::{Shape, Tool, COLORS, SIZES};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{ElementState, MouseButton, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::keyboard::{Key, ModifiersState};
use tao::monitor::MonitorHandle;
use tao::window::{CursorIcon, WindowBuilder, WindowId};
use tiny_skia::{
    FillRule, FilterQuality, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Point, Rect, Stroke,
    Transform,
};
pub use toolbar::rounded as rounded_rect;
use toolbar::{Hit, Palette};
use traykit::{linfo, lwarn};

/// Space around the picture, in logical pixels.
const MARGIN: f32 = 16.0;
/// How long Copy and Save show their tick.
const DONE_FOR: Duration = Duration::from_millis(1200);

pub enum Request {
    Nothing,
    Close,
    NewSnip,
}

pub struct Editor {
    view: View,
    /// The snip as taken.
    base: Pixmap,
    /// The snip with every blur applied: what the marks are drawn over.
    shown: Pixmap,
    shapes: Vec<Shape>,
    undone: Vec<Shape>,
    live: Option<Shape>,
    tool: Tool,
    /// The colour each tool last used, by `Tool::ALL` index, so switching to
    /// the highlighter does not turn it red.
    colors: [usize; 5],
    size: usize,
    /// The scale of the screen the snip came from: marks are sized to it.
    snip_scale: f32,
    cursor: (f32, f32),
    mods: ModifiersState,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    done: Option<(Hit, Instant)>,
    cache: Option<Cache>,
    look: Palette,
    save_dir: Option<PathBuf>,
}

struct Cache {
    size: (u32, u32),
    background: Pixmap,
    /// The picture's area, so marks stop at its edge.
    clip: Mask,
}

impl Editor {
    pub fn new<T>(
        target: &EventLoopWindowTarget<T>,
        snip: Snip,
        monitor: Option<MonitorHandle>,
        save_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        let base = Pixmap::from_vec(
            snip.rgba,
            tiny_skia::IntSize::from_wh(snip.width, snip.height).ok_or("empty snip")?,
        )
        .ok_or("could not hold the snip")?;

        // As big as the snip at its own scale, within most of the monitor.
        let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
        let (mw, mh) = monitor
            .as_ref()
            .map(|m| {
                let s = m.size().to_logical::<f64>(scale);
                (s.width, s.height)
            })
            .unwrap_or((1280.0, 800.0));
        let img_w = snip.width as f64 / snip.scale + 2.0 * MARGIN as f64;
        let img_h = snip.height as f64 / snip.scale + 2.0 * MARGIN as f64;
        let w = img_w.max(toolbar::min_width() as f64).min(mw * 0.85);
        let h = (img_h + toolbar::HEIGHT as f64).max(240.0).min(mh * 0.85);

        let mut builder = WindowBuilder::new()
            .with_title("Print")
            .with_visible(false)
            .with_inner_size(LogicalSize::new(w, h))
            .with_min_inner_size(LogicalSize::new(toolbar::min_width() as f64, 200.0))
            .with_window_icon(crate::icon::window_icon());
        if let Some(m) = &monitor {
            let (pw, ph) = (w * scale, h * scale);
            let pos = m.position();
            let size = m.size();
            builder = builder.with_position(PhysicalPosition::new(
                pos.x + ((size.width as f64 - pw) / 2.0).max(0.0) as i32,
                pos.y + ((size.height as f64 - ph) / 2.0).max(0.0) as i32,
            ));
        }
        let window = builder
            .build(target)
            .map_err(|e| format!("could not open the editor: {e}"))?;

        let mut editor = Self {
            view: View::new(window)?,
            shown: base.clone(),
            base,
            shapes: Vec::new(),
            undone: Vec::new(),
            live: None,
            tool: Tool::Pen,
            colors: [0, 1, 0, 0, 0],
            size: 1,
            snip_scale: snip.scale as f32,
            cursor: (0.0, 0.0),
            mods: ModifiersState::empty(),
            hover: None,
            pressed: None,
            done: None,
            cache: None,
            look: Palette::new(traykit::theme::taskbar_is_dark()),
            save_dir,
        };
        editor.redraw();
        editor.view.window().set_visible(true);
        editor.view.window().set_focus();
        Ok(editor)
    }

    pub fn id(&self) -> WindowId {
        self.view.window().id()
    }

    /// When the editor next needs a tick, for the event loop's wait.
    pub fn deadline(&self) -> Option<Instant> {
        self.done.map(|(_, at)| at + DONE_FOR)
    }

    pub fn tick(&mut self, now: Instant) {
        if self.deadline().is_some_and(|d| now >= d) {
            self.done = None;
            self.view.window().request_redraw();
        }
    }

    fn scale(&self) -> f32 {
        self.view.window().scale_factor() as f32
    }

    /// The picture's zoom and top-left corner in the window.
    fn fit(&self, (w, h): (u32, u32)) -> (f32, f32, f32) {
        let s = self.scale();
        let top = toolbar::HEIGHT * s;
        let margin = MARGIN * s;
        let (iw, ih) = (self.base.width() as f32, self.base.height() as f32);
        let aw = (w as f32 - 2.0 * margin).max(1.0);
        let ah = (h as f32 - top - 2.0 * margin).max(1.0);
        let zoom = (aw / iw).min(ah / ih).min(1.0);
        let ox = ((w as f32 - iw * zoom) / 2.0).round();
        let oy = (top + (h as f32 - top - ih * zoom) / 2.0).round();
        (zoom, ox, oy)
    }

    fn to_image(&self, (x, y): (f32, f32)) -> Point {
        let (zoom, ox, oy) = self.fit(self.view.size());
        Point::from_xy((x - ox) / zoom, (y - oy) / zoom)
    }

    fn color_index(&self) -> usize {
        self.colors[self.tool_index()]
    }

    fn tool_index(&self) -> usize {
        Tool::ALL.iter().position(|t| *t == self.tool).unwrap_or(0)
    }

    fn hit(&self, (x, y): (f32, f32)) -> Option<Hit> {
        let s = self.scale();
        if y >= toolbar::HEIGHT * s {
            return None;
        }
        let (w, _) = self.view.size();
        toolbar::layout(w as f32, s)
            .into_iter()
            .find(|b| b.contains(x, y))
            .map(|b| b.hit)
    }

    pub fn handle(&mut self, event: &WindowEvent) -> Request {
        let window = self.view.window();
        match event {
            WindowEvent::CloseRequested => return Request::Close,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.cache = None;
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = *m,
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                let hover = self.hit(self.cursor);
                if hover != self.hover {
                    self.hover = hover;
                    window.set_cursor_icon(
                        if hover.is_some() || self.cursor.1 < toolbar::HEIGHT * self.scale() {
                            CursorIcon::Default
                        } else {
                            CursorIcon::Crosshair
                        },
                    );
                    window.request_redraw();
                }
                if self.live.is_some() {
                    let p = self.to_image(self.cursor);
                    let p = if self.tool == Tool::Blur {
                        self.clamp(p)
                    } else {
                        p
                    };
                    if let Some(live) = &mut self.live {
                        live.extend(p);
                    }
                    self.view.window().request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(hit) = self.hit(self.cursor) {
                    self.pressed = Some(hit);
                    window.request_redraw();
                } else if self.cursor.1 >= toolbar::HEIGHT * self.scale() {
                    let mut at = self.to_image(self.cursor);
                    if self.tool == Tool::Blur {
                        at = self.clamp(at);
                    }
                    self.live = Some(Shape::new(
                        self.tool,
                        COLORS[self.color_index()],
                        SIZES[self.size],
                        self.snip_scale,
                        at,
                    ));
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(pressed) = self.pressed.take() {
                    window.request_redraw();
                    if self.hit(self.cursor) == Some(pressed) {
                        self.act(pressed);
                    }
                }
                if let Some(shape) = self.live.take() {
                    if shape.is_visible() {
                        self.push(shape);
                    }
                    self.view.window().request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if self.live.take().is_some() {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                return self.key(&event.logical_key);
            }
            _ => {}
        }
        Request::Nothing
    }

    fn key(&mut self, key: &Key) -> Request {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let shift = self.mods.shift_key();
        match key {
            Key::Escape => {
                if self.live.take().is_some() {
                    self.view.window().request_redraw();
                } else {
                    return Request::Close;
                }
            }
            Key::Character(c) => {
                let c = c.to_ascii_lowercase();
                match (ctrl, c.as_str()) {
                    (true, "c") => self.act(Hit::Copy),
                    (true, "s") => self.act(Hit::Save),
                    (true, "z") if shift => self.act(Hit::Redo),
                    (true, "z") => self.act(Hit::Undo),
                    (true, "y") => self.act(Hit::Redo),
                    (true, "w") => return Request::Close,
                    (true, "n") => return Request::NewSnip,
                    (false, "p") => self.act(Hit::Tool(Tool::Pen)),
                    (false, "h") => self.act(Hit::Tool(Tool::Marker)),
                    (false, "a") => self.act(Hit::Tool(Tool::Arrow)),
                    (false, "r") => self.act(Hit::Tool(Tool::Rect)),
                    (false, "b") => self.act(Hit::Tool(Tool::Blur)),
                    (false, "[") => self.size = self.size.saturating_sub(1),
                    (false, "]") => self.size = (self.size + 1).min(SIZES.len() - 1),
                    (false, d) if d.len() == 1 && ("1"..="6").contains(&d) => {
                        self.act(Hit::Color(d.parse::<usize>().unwrap_or(1) - 1))
                    }
                    _ => return Request::Nothing,
                }
                self.view.window().request_redraw();
            }
            _ => {}
        }
        Request::Nothing
    }

    fn act(&mut self, hit: Hit) {
        match hit {
            Hit::Tool(t) => self.tool = t,
            Hit::Color(i) => {
                let t = self.tool_index();
                self.colors[t] = i.min(COLORS.len() - 1);
            }
            Hit::Size => self.size = (self.size + 1) % SIZES.len(),
            Hit::Undo => {
                if let Some(s) = self.shapes.pop() {
                    let blur = s.tool == Tool::Blur;
                    self.undone.push(s);
                    if blur {
                        self.reblur();
                    }
                }
            }
            Hit::Redo => {
                if let Some(s) = self.undone.pop() {
                    self.push_keeping_redo(s);
                }
            }
            Hit::Copy => match self.export().copy() {
                Ok(()) => {
                    linfo!("editor: copied");
                    self.done = Some((Hit::Copy, Instant::now()));
                }
                Err(e) => lwarn!("editor: copy failed: {e}"),
            },
            Hit::Save => {
                let snip = self.export();
                let name = crate::snip::file_name(traykit::time::LocalTime::now());
                if let Some(path) = self.ask_path(&name) {
                    match snip.save(&path) {
                        Ok(()) => {
                            linfo!("editor: saved {}", path.display());
                            if let Some(dir) = path.parent() {
                                self.save_dir = Some(dir.to_path_buf());
                            }
                            self.done = Some((Hit::Save, Instant::now()));
                        }
                        Err(e) => lwarn!("editor: save failed: {e}"),
                    }
                }
            }
        }
        self.view.window().request_redraw();
    }

    fn push(&mut self, shape: Shape) {
        self.undone.clear();
        self.push_keeping_redo(shape);
    }

    fn push_keeping_redo(&mut self, shape: Shape) {
        let blur = shape.tool == Tool::Blur;
        self.shapes.push(shape);
        if blur {
            self.reblur();
        }
    }

    fn clamp(&self, p: Point) -> Point {
        Point::from_xy(
            p.x.clamp(0.0, self.base.width() as f32),
            p.y.clamp(0.0, self.base.height() as f32),
        )
    }

    /// Blocks about ten logical pixels wide: big enough that text under
    /// them is gone.
    fn block(&self) -> u32 {
        (10.0 * self.snip_scale).round().max(6.0) as u32
    }

    fn reblur(&mut self) {
        let mut shown = self.base.clone();
        for s in self.shapes.iter().filter(|s| s.tool == Tool::Blur) {
            if let Some(r) = s.rect() {
                shapes::pixelate(&mut shown, r, self.block());
            }
        }
        self.shown = shown;
        self.cache = None;
    }

    /// The picture with every mark painted in, at full resolution.
    fn export(&self) -> Snip {
        let mut pm = self.shown.clone();
        for s in self.shapes.iter().filter(|s| s.tool != Tool::Blur) {
            s.draw(&mut pm.as_mut(), Transform::identity(), None);
        }
        Snip {
            width: pm.width(),
            height: pm.height(),
            rgba: pm.take(),
            scale: self.snip_scale as f64,
        }
    }

    #[cfg(windows)]
    fn ask_path(&self, name: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save snip")
            .set_file_name(name)
            .add_filter("PNG image", &["png"])
            .set_parent(self.view.window());
        if let Some(dir) = &self.save_dir {
            let _ = std::fs::create_dir_all(dir);
            dialog = dialog.set_directory(dir);
        }
        let path = dialog.save_file()?;
        Some(if path.extension().is_none() {
            path.with_extension("png")
        } else {
            path
        })
    }

    /// No dialog here yet: straight into the folder.
    #[cfg(not(windows))]
    fn ask_path(&self, name: &str) -> Option<PathBuf> {
        Some(crate::snip::fresh_path(self.save_dir.as_ref()?, name))
    }

    fn ensure_cache(&mut self, size: (u32, u32)) -> Option<()> {
        if self.cache.as_ref().is_some_and(|c| c.size == size) {
            return Some(());
        }
        let (zoom, ox, oy) = self.fit(size);
        let mut background = Pixmap::new(size.0, size.1)?;
        background.fill(self.look.canvas);
        let quality = if zoom < 1.0 {
            FilterQuality::Bicubic
        } else {
            FilterQuality::Nearest
        };
        background.draw_pixmap(
            0,
            0,
            self.shown.as_ref(),
            &PixmapPaint {
                quality,
                ..PixmapPaint::default()
            },
            Transform::from_row(zoom, 0.0, 0.0, zoom, ox, oy),
            None,
        );
        let (iw, ih) = (
            self.base.width() as f32 * zoom,
            self.base.height() as f32 * zoom,
        );
        let area = Rect::from_xywh(ox, oy, iw, ih)?;
        let mut clip = Mask::new(size.0, size.1)?;
        clip.fill_path(
            &PathBuilder::from_rect(area),
            FillRule::Winding,
            false,
            Transform::identity(),
        );
        self.cache = Some(Cache {
            size,
            background,
            clip,
        });
        Some(())
    }

    pub fn redraw(&mut self) {
        let size = self.view.size();
        if size.0 == 0 || size.1 == 0 || self.ensure_cache(size).is_none() {
            return;
        }
        let (zoom, ox, oy) = self.fit(size);
        let t = Transform::from_row(zoom, 0.0, 0.0, zoom, ox, oy);
        let s = self.scale();
        let Some(cache) = &self.cache else { return };
        let mut frame = cache.background.clone();
        for shape in self.shapes.iter().chain(self.live.iter()) {
            if shape.tool != Tool::Blur {
                shape.draw(&mut frame.as_mut(), t, Some(&cache.clip));
            }
        }
        // A blur being dragged is shown as its box; it is applied on release.
        if let Some((x, y, w, h)) = self
            .live
            .as_ref()
            .filter(|l| l.tool == Tool::Blur)
            .and_then(Shape::rect)
        {
            if let Some(r) = Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)) {
                let path = PathBuilder::from_rect(r);
                let mut paint = Paint::default();
                paint.set_color_rgba8(255, 255, 255, 60);
                frame.fill_path(&path, &paint, FillRule::Winding, t, Some(&cache.clip));
                paint.set_color_rgba8(0, 0, 0, 200);
                let stroke = Stroke {
                    width: s / zoom,
                    dash: tiny_skia::StrokeDash::new(vec![4.0 * s / zoom, 3.0 * s / zoom], 0.0),
                    ..Stroke::default()
                };
                frame.stroke_path(&path, &paint, &stroke, t, None);
            }
        }
        let state = toolbar::State {
            tool: self.tool,
            color: self.color_index(),
            size: self.size,
            can_undo: !self.shapes.is_empty(),
            can_redo: !self.undone.is_empty(),
            hover: self.hover,
            pressed: self.pressed,
            done: self.done.map(|(h, _)| h),
        };
        toolbar::draw(&mut frame, s, &state, &self.look);

        let (fw, fh) = (frame.width(), frame.height());
        let data = frame.data();
        self.view.present(|buf, w, h| {
            if (w, h) != (fw, fh) {
                buf.fill(0);
                return;
            }
            for (dst, px) in buf.iter_mut().zip(data.as_chunks::<4>().0) {
                *dst = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
            }
        });
    }
}
