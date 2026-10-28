//! The mark: the four corners of a selection around a filled square, as
//! the overlay draws it. Drawn in code at every size, so the tray icon is
//! sharp at any DPI and the app icon comes from the same lines.
//!
//! `print --dump-icons assets` writes the app icon as PNGs and the
//! `icon.ico` the build embeds in the exe.

use std::path::Path;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PathBuilder, Pixmap,
    Point, SpreadMode, Stroke, Transform,
};

/// Windows draws tray icons at 16 logical pixels; 32 covers 200%.
pub const TRAY_EDGE: u32 = 32;

const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// The tray icon, white on a dark taskbar and near-black on a light one.
pub fn tray(dark_taskbar: bool) -> Vec<u8> {
    let ink = if dark_taskbar {
        Color::WHITE
    } else {
        Color::from_rgba8(0x1f, 0x1f, 0x1f, 255)
    };
    let mut pm = Pixmap::new(TRAY_EDGE, TRAY_EDGE).expect("non-zero");
    mark(&mut pm, ink, ink, 0.0);
    unpremultiplied(pm)
}

/// The app icon: the mark in white on a blue-to-violet tile.
pub fn app(edge: u32) -> Vec<u8> {
    let mut pm = Pixmap::new(edge, edge).expect("non-zero");
    let e = edge as f32;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.shader = LinearGradient::new(
        Point::from_xy(0.0, 0.0),
        Point::from_xy(e, e),
        vec![
            GradientStop::new(0.0, Color::from_rgba8(0x25, 0x63, 0xeb, 255)),
            GradientStop::new(1.0, Color::from_rgba8(0x7c, 0x3a, 0xed, 255)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .expect("two stops");
    // A little inset, as the system's own icons have.
    let inset = e * 0.04;
    if let Some(tile) =
        crate::editor::rounded_rect(inset, inset, e - 2.0 * inset, e - 2.0 * inset, e * 0.22)
    {
        pm.fill_path(
            &tile,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let mut inner = Color::WHITE;
    inner.set_alpha(0.85);
    mark(&mut pm, Color::WHITE, inner, e * 0.14);
    unpremultiplied(pm)
}

pub fn window_icon() -> Option<tao::window::Icon> {
    tao::window::Icon::from_rgba(app(64), 64, 64).ok()
}

/// The corners and the square, in `pm`, `margin` in from its edges.
fn mark(pm: &mut Pixmap, corners: Color, square: Color, margin: f32) {
    let e = pm.width() as f32;
    // On a 32 grid, then scaled.
    let k = (e - 2.0 * margin) / 32.0;
    let t = Transform::from_row(k, 0.0, 0.0, k, margin, margin);
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(corners);
    let stroke = Stroke {
        width: 3.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let (lo, hi, arm) = (3.5, 28.5, 8.0);
    let mut pb = PathBuilder::new();
    for (x, y, dx, dy) in [
        (lo, lo, 1.0, 1.0),
        (hi, lo, -1.0, 1.0),
        (lo, hi, 1.0, -1.0),
        (hi, hi, -1.0, -1.0),
    ] {
        pb.move_to(x, y + dy * arm);
        pb.line_to(x, y);
        pb.line_to(x + dx * arm, y);
    }
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint, &stroke, t, None);
    }
    paint.set_color(square);
    if let Some(r) = crate::editor::rounded_rect(10.0, 10.0, 12.0, 12.0, 2.0) {
        pm.fill_path(&r, &paint, FillRule::Winding, t, None);
    }
}

/// tiny-skia keeps premultiplied alpha; everything outside it wants it
/// straight.
fn unpremultiplied(pm: Pixmap) -> Vec<u8> {
    let mut out = pm.take();
    for px in out.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    out
}

fn png(edge: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, edge, edge);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// An .ico of PNG images, which every Windows since Vista reads.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (edge, data) in images {
        // 0 means 256.
        let side = if *edge >= 256 { 0 } else { *edge as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}

pub fn dump(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut images = Vec::new();
    for edge in ICO_SIZES {
        images.push((edge, png(edge, &app(edge))?));
    }
    let write = |name: &str, bytes: &[u8]| {
        let p = dir.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        println!("{}", p.display());
        Ok::<(), String>(())
    };
    write("icon.ico", &ico(&images))?;
    write("icon-256.png", &png(256, &app(256))?)?;
    write("icon-1024.png", &png(1024, &app(1024))?)?;
    Ok(())
}
