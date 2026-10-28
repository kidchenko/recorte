//! The strip above the picture: the tools, the colours, the size, undo and
//! redo, and copy and save on the right.
//!
//! Every glyph is a few path commands on a 20x20 grid, drawn at the window's
//! scale. No icon font, no images: the toolbar is sharp at any DPI and costs
//! nothing in the binary.

use super::shapes::{Tool, COLORS, SIZES};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

/// Logical pixels.
pub const HEIGHT: f32 = 50.0;
const PAD: f32 = 8.0;
const BUTTON: f32 = 34.0;
const SWATCH: f32 = 26.0;
const GAP: f32 = 2.0;
const SEPARATOR: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Tool(Tool),
    Color(usize),
    Size,
    Undo,
    Redo,
    Copy,
    Save,
}

#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub hit: Hit,
    /// Physical pixels.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Button {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

pub struct State {
    pub tool: Tool,
    pub color: usize,
    pub size: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    /// Copy or Save just worked: show a tick on it for a moment.
    pub done: Option<Hit>,
}

pub struct Palette {
    pub canvas: Color,
    bar: Color,
    line: Color,
    fg: Color,
    faint: Color,
    hover: Color,
    selected: Color,
    accent: Color,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        let (r, g, b) = traykit::theme::accent(dark);
        let accent = Color::from_rgba8(r, g, b, 255);
        let c = |v: u32| Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 255);
        if dark {
            Self {
                canvas: c(0x161616),
                bar: c(0x252525),
                line: c(0x333333),
                fg: c(0xe8e8e8),
                faint: c(0x707070),
                hover: c(0x353535),
                selected: c(0x404040),
                accent,
            }
        } else {
            Self {
                canvas: c(0xdedede),
                bar: c(0xf6f6f6),
                line: c(0xd8d8d8),
                fg: c(0x1f1f1f),
                faint: c(0xb0b0b0),
                hover: c(0xe6e6e6),
                selected: c(0xdadada),
                accent,
            }
        }
    }
}

/// Narrowest window that fits every button, in logical pixels.
pub fn min_width() -> f32 {
    let left = PAD
        + Tool::ALL.len() as f32 * (BUTTON + GAP)
        + SEPARATOR
        + COLORS.len() as f32 * (SWATCH + GAP)
        + SEPARATOR
        + (BUTTON + GAP)
        + SEPARATOR
        + 2.0 * (BUTTON + GAP);
    left + SEPARATOR + 2.0 * (BUTTON + GAP) + PAD
}

pub fn layout(width: f32, scale: f32) -> Vec<Button> {
    let s = scale;
    let y = (HEIGHT - BUTTON) / 2.0 * s;
    let mut out = Vec::new();
    let mut x = PAD * s;
    let mut push = |hit, x: &mut f32, w: f32| {
        out.push(Button {
            hit,
            x: *x,
            y: if w == SWATCH {
                (HEIGHT - SWATCH) / 2.0 * s
            } else {
                y
            },
            w: w * s,
            h: w * s,
        });
        *x += (w + GAP) * s;
    };
    for t in Tool::ALL {
        push(Hit::Tool(t), &mut x, BUTTON);
    }
    x += SEPARATOR * s;
    for i in 0..COLORS.len() {
        push(Hit::Color(i), &mut x, SWATCH);
    }
    x += SEPARATOR * s;
    push(Hit::Size, &mut x, BUTTON);
    x += SEPARATOR * s;
    push(Hit::Undo, &mut x, BUTTON);
    push(Hit::Redo, &mut x, BUTTON);
    // Copy and Save sit at the right edge, where a dialog's buttons are.
    let mut x = (width - PAD * s - 2.0 * BUTTON * s - GAP * s).max(x + SEPARATOR * s);
    push(Hit::Copy, &mut x, BUTTON);
    push(Hit::Save, &mut x, BUTTON);
    out
}

pub fn draw(pm: &mut Pixmap, scale: f32, state: &State, look: &Palette) {
    let s = scale;
    let width = pm.width() as f32;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(look.bar);
    if let Some(r) = Rect::from_xywh(0.0, 0.0, width, HEIGHT * s) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
    paint.set_color(look.line);
    if let Some(r) = Rect::from_xywh(0.0, HEIGHT * s - s.max(1.0), width, s.max(1.0)) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }

    for b in layout(width, s) {
        let enabled = match b.hit {
            Hit::Undo => state.can_undo,
            Hit::Redo => state.can_redo,
            _ => true,
        };
        if let Hit::Color(i) = b.hit {
            swatch(pm, &b, COLORS[i], i == state.color, s, look);
            continue;
        }
        let selected = b.hit == Hit::Tool(state.tool);
        let bg = if selected || (enabled && state.pressed == Some(b.hit)) {
            Some(look.selected)
        } else if enabled && state.hover == Some(b.hit) {
            Some(look.hover)
        } else {
            None
        };
        if let Some(bg) = bg {
            paint.set_color(bg);
            if let Some(path) = rounded(b.x, b.y, b.w, b.h, 6.0 * s) {
                pm.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
        if selected {
            paint.set_color(look.accent);
            if let Some(path) = rounded(
                b.x + b.w * 0.3,
                b.y + b.h - 3.0 * s,
                b.w * 0.4,
                2.5 * s,
                1.25 * s,
            ) {
                pm.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
        let ink = if enabled { look.fg } else { look.faint };
        let glyph = if state.done == Some(b.hit) {
            Glyph::Tick
        } else {
            Glyph::of(b.hit)
        };
        // The 20x20 grid, centred in the button.
        let t = Transform::from_row(
            s,
            0.0,
            0.0,
            s,
            b.x + (b.w - 20.0 * s) / 2.0,
            b.y + (b.h - 20.0 * s) / 2.0,
        );
        draw_glyph(pm, glyph, t, ink, look, state);
    }
}

fn swatch(pm: &mut Pixmap, b: &Button, rgb: [u8; 3], selected: bool, s: f32, look: &Palette) {
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    if selected {
        paint.set_color(look.accent);
        let ring = Stroke {
            width: 2.0 * s,
            ..Stroke::default()
        };
        if let Some(c) = PathBuilder::from_circle(cx, cy, 11.5 * s) {
            pm.stroke_path(&c, &paint, &ring, Transform::identity(), None);
        }
    }
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    if let Some(c) = PathBuilder::from_circle(cx, cy, 8.0 * s) {
        pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        // A hairline, so white shows on a light bar and black on a dark one.
        let mut edge = look.fg;
        edge.set_alpha(0.25);
        paint.set_color(edge);
        let hair = Stroke {
            width: 1.0 * s,
            ..Stroke::default()
        };
        pm.stroke_path(&c, &paint, &hair, Transform::identity(), None);
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Tool(Tool),
    Size,
    Undo,
    Redo,
    Copy,
    Save,
    Tick,
}

impl Glyph {
    fn of(hit: Hit) -> Self {
        match hit {
            Hit::Tool(t) => Glyph::Tool(t),
            Hit::Size => Glyph::Size,
            Hit::Undo => Glyph::Undo,
            Hit::Redo => Glyph::Redo,
            Hit::Copy => Glyph::Copy,
            Hit::Save => Glyph::Save,
            Hit::Color(_) => Glyph::Tick,
        }
    }
}

fn draw_glyph(
    pm: &mut Pixmap,
    glyph: Glyph,
    t: Transform,
    ink: Color,
    look: &Palette,
    state: &State,
) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(ink);
    let stroke = Stroke {
        // In grid units; the transform scales it with everything else.
        width: 1.6,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let line = |pm: &mut Pixmap, paint: &Paint, pts: &[(f32, f32)]| {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for &(x, y) in &pts[1..] {
            pb.line_to(x, y);
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, paint, &stroke, t, None);
        }
    };
    let shape = |pm: &mut Pixmap, paint: &Paint, pts: &[(f32, f32)], fill: bool| {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for &(x, y) in &pts[1..] {
            pb.line_to(x, y);
        }
        pb.close();
        if let Some(p) = pb.finish() {
            if fill {
                pm.fill_path(&p, paint, FillRule::Winding, t, None);
            } else {
                pm.stroke_path(&p, paint, &stroke, t, None);
            }
        }
    };
    match glyph {
        Glyph::Tool(Tool::Pen) => {
            shape(
                pm,
                &paint,
                &[
                    (3.5, 16.5),
                    (4.5, 12.5),
                    (13.5, 3.5),
                    (16.5, 6.5),
                    (7.5, 15.5),
                ],
                false,
            );
            line(pm, &paint, &[(11.5, 5.5), (14.5, 8.5)]);
        }
        Glyph::Tool(Tool::Marker) => {
            shape(
                pm,
                &paint,
                &[(6.0, 12.0), (12.0, 4.0), (16.0, 7.0), (10.0, 15.0)],
                false,
            );
            line(
                pm,
                &paint,
                &[(6.0, 12.0), (5.0, 15.0), (7.0, 16.0), (10.0, 15.0)],
            );
            let mut ink = paint.clone();
            let [r, g, b] = COLORS[1];
            ink.set_color_rgba8(r, g, b, 255);
            let wide = Stroke {
                width: 2.6,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            let mut pb = PathBuilder::new();
            pb.move_to(3.0, 18.5);
            pb.line_to(17.0, 18.5);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &ink, &wide, t, None);
            }
        }
        Glyph::Tool(Tool::Arrow) => {
            line(pm, &paint, &[(4.0, 16.0), (15.0, 5.0)]);
            line(pm, &paint, &[(8.0, 5.0), (15.0, 5.0), (15.0, 12.0)]);
        }
        Glyph::Tool(Tool::Rect) => {
            shape(
                pm,
                &paint,
                &[(3.0, 5.0), (17.0, 5.0), (17.0, 15.0), (3.0, 15.0)],
                false,
            );
        }
        Glyph::Tool(Tool::Blur) => {
            for row in 0..3 {
                for col in 0..3 {
                    let (x, y) = (3.0 + col as f32 * 5.0, 3.0 + row as f32 * 5.0);
                    let mut c = ink;
                    if (row + col) % 2 == 1 {
                        c.set_alpha(0.35);
                    }
                    paint.set_color(c);
                    if let Some(r) = Rect::from_xywh(x, y, 4.2, 4.2) {
                        pm.fill_rect(r, &paint, t, None);
                    }
                }
            }
        }
        Glyph::Size => {
            let r = [2.0, 3.5, 5.5][state.size.min(SIZES.len() - 1)];
            if let Some(c) = PathBuilder::from_circle(10.0, 10.0, r) {
                pm.fill_path(&c, &paint, FillRule::Winding, t, None);
            }
        }
        Glyph::Undo | Glyph::Redo => {
            let flip = matches!(glyph, Glyph::Redo);
            let x = |v: f32| if flip { 20.0 - v } else { v };
            line(pm, &paint, &[(x(8.0), 4.5), (x(4.0), 8.5), (x(8.0), 12.5)]);
            let mut pb = PathBuilder::new();
            pb.move_to(x(4.0), 8.5);
            pb.line_to(x(12.0), 8.5);
            pb.cubic_to(x(18.0), 8.5, x(18.0), 16.5, x(12.0), 16.5);
            pb.line_to(x(8.0), 16.5);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint, &stroke, t, None);
            }
        }
        Glyph::Copy => {
            if let Some(p) = rounded(7.0, 3.0, 10.0, 11.0, 2.0) {
                pm.stroke_path(&p, &paint, &stroke, t, None);
            }
            if let Some(p) = rounded(3.0, 7.0, 10.0, 11.0, 2.0) {
                let mut bg = paint.clone();
                bg.set_color(look.bar);
                pm.fill_path(&p, &bg, FillRule::Winding, t, None);
                pm.stroke_path(&p, &paint, &stroke, t, None);
            }
        }
        Glyph::Save => {
            line(pm, &paint, &[(10.0, 3.0), (10.0, 12.5)]);
            line(pm, &paint, &[(6.0, 8.5), (10.0, 12.5), (14.0, 8.5)]);
            line(
                pm,
                &paint,
                &[(4.0, 13.0), (4.0, 17.0), (16.0, 17.0), (16.0, 13.0)],
            );
        }
        Glyph::Tick => {
            paint.set_color(look.accent);
            line(pm, &paint, &[(4.5, 10.5), (8.5, 14.5), (15.5, 6.0)]);
        }
    }
}

/// A rounded rectangle, which tiny-skia has no shortcut for.
pub fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    // Control-point distance for a quarter circle drawn as one cubic.
    let k = r * 0.552_285;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_fits_at_the_minimum_width() {
        for scale in [1.0, 1.25, 2.0] {
            let w = min_width() * scale;
            let buttons = layout(w, scale);
            assert_eq!(buttons.len(), 5 + 6 + 1 + 2 + 2);
            for pair in buttons.windows(2) {
                assert!(pair[0].x + pair[0].w <= pair[1].x, "{pair:?} overlap");
            }
            let last = buttons.last().unwrap();
            assert!(last.x + last.w <= w + 0.5);
        }
    }
}
