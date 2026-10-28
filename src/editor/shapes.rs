//! The marks, kept as shapes rather than painted into the picture, so undo
//! is a `pop` and the window can be resized without losing quality. They
//! are painted into the pixels only when the picture is copied or saved.
//!
//! Coordinates are the snip's own pixels.

use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapMut,
    Point, Stroke, Transform,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Marker,
    Arrow,
    Rect,
    Blur,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::Pen, Tool::Marker, Tool::Arrow, Tool::Rect, Tool::Blur];

    /// Stroke width at the middle size, in logical pixels of the screen the
    /// snip came from.
    fn width(self) -> f32 {
        match self {
            Tool::Pen => 3.0,
            Tool::Marker => 16.0,
            Tool::Arrow => 4.0,
            Tool::Rect => 3.0,
            Tool::Blur => 0.0,
        }
    }
}

/// The swatches: a red that reads on anything, the highlighter yellow, and
/// the rest of a small set.
pub const COLORS: [[u8; 3]; 6] = [
    [0xe5, 0x39, 0x35],
    [0xff, 0xd6, 0x00],
    [0x43, 0xa0, 0x47],
    [0x1e, 0x88, 0xe5],
    [0x21, 0x21, 0x21],
    [0xff, 0xff, 0xff],
];

/// Small, medium, large.
pub const SIZES: [f32; 3] = [0.6, 1.0, 1.8];

#[derive(Clone, Debug)]
pub struct Shape {
    pub tool: Tool,
    pub color: [u8; 3],
    /// In snip pixels.
    pub width: f32,
    /// Every point for the pen and the marker; start and end for the rest.
    pub points: Vec<Point>,
}

impl Shape {
    pub fn new(tool: Tool, color: [u8; 3], size: f32, scale: f32, at: Point) -> Self {
        Self {
            tool,
            color,
            width: tool.width() * size * scale,
            points: vec![at],
        }
    }

    pub fn extend(&mut self, to: Point) {
        match self.tool {
            Tool::Pen | Tool::Marker => {
                // Skip sub-pixel moves: they add points, not ink.
                if self.points.last().is_some_and(|p| p.distance(to) < 0.75) {
                    return;
                }
                self.points.push(to);
            }
            _ => {
                self.points.truncate(1);
                self.points.push(to);
            }
        }
    }

    /// Worth keeping: a dot from the pen is, a zero-size box is not.
    pub fn is_visible(&self) -> bool {
        match self.tool {
            Tool::Pen | Tool::Marker => true,
            _ => self.rect().is_some_and(|(_, _, w, h)| w >= 2.0 || h >= 2.0),
        }
    }

    /// Start and end as `(x, y, w, h)`, normalised.
    pub fn rect(&self) -> Option<(f32, f32, f32, f32)> {
        let a = *self.points.first()?;
        let b = *self.points.get(1)?;
        Some((
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        ))
    }

    /// Paint everything but blur (which is applied to the picture itself,
    /// see [`pixelate`]).
    pub fn draw(&self, pm: &mut PixmapMut, t: Transform, clip: Option<&Mask>) {
        let [r, g, b] = self.color;
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color_rgba8(r, g, b, 255);
        let mut stroke = Stroke {
            width: self.width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        match self.tool {
            Tool::Pen => {
                if self.points.len() == 1 {
                    let p = self.points[0];
                    if let Some(dot) = PathBuilder::from_circle(p.x, p.y, self.width / 2.0) {
                        pm.fill_path(&dot, &paint, FillRule::Winding, t, clip);
                    }
                } else if let Some(path) = polyline(&self.points) {
                    pm.stroke_path(&path, &paint, &stroke, t, clip);
                }
            }
            Tool::Marker => {
                // Multiply, like ink on paper: text under it stays black,
                // and crossing a stroke never darkens it, because the whole
                // stroke is one path.
                paint.set_color(Color::from_rgba8(r, g, b, 255));
                paint.blend_mode = BlendMode::Multiply;
                stroke.line_cap = LineCap::Square;
                let mut pts = self.points.clone();
                if pts.len() == 1 {
                    pts.push(Point::from_xy(pts[0].x + 0.1, pts[0].y));
                }
                if let Some(path) = polyline(&pts) {
                    pm.stroke_path(&path, &paint, &stroke, t, clip);
                }
            }
            Tool::Arrow => {
                let (Some(&a), Some(&b)) = (self.points.first(), self.points.get(1)) else {
                    return;
                };
                let len = a.distance(b);
                if len < 1.0 {
                    return;
                }
                let head = (self.width * 3.2 + 6.0).min(len * 0.6);
                let half = head * 0.6;
                let (dx, dy) = ((b.x - a.x) / len, (b.y - a.y) / len);
                // The shaft stops inside the head, so its round cap does not
                // poke through the point.
                let base = Point::from_xy(b.x - dx * head, b.y - dy * head);
                let shaft_end = Point::from_xy(b.x - dx * head * 0.5, b.y - dy * head * 0.5);
                if let Some(path) = polyline(&[a, shaft_end]) {
                    pm.stroke_path(&path, &paint, &stroke, t, clip);
                }
                let mut pb = PathBuilder::new();
                pb.move_to(b.x, b.y);
                pb.line_to(base.x - dy * half, base.y + dx * half);
                pb.line_to(base.x + dy * half, base.y - dx * half);
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint, FillRule::Winding, t, clip);
                }
            }
            Tool::Rect => {
                let Some((x, y, w, h)) = self.rect() else {
                    return;
                };
                let Some(r) = tiny_skia::Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)) else {
                    return;
                };
                let path = PathBuilder::from_rect(r);
                stroke.line_join = LineJoin::Miter;
                pm.stroke_path(&path, &paint, &stroke, t, clip);
            }
            Tool::Blur => {}
        }
    }
}

fn polyline(points: &[Point]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    pb.move_to(first.x, first.y);
    if rest.is_empty() {
        pb.line_to(first.x + 0.01, first.y);
    }
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    pb.finish()
}

/// Mosaic `(x, y, w, h)` of `pm` in blocks of `block` pixels.
///
/// Pixelating rather than blurring: a blur of a few pixels can be read back
/// (or sharpened back) and one wide enough to hide text smears into what it
/// does not cover. Big flat blocks hide text completely.
pub fn pixelate(pm: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32), block: u32) {
    let (pw, ph) = (pm.width() as i64, pm.height() as i64);
    let x0 = (x.floor() as i64).clamp(0, pw);
    let y0 = (y.floor() as i64).clamp(0, ph);
    let x1 = ((x + w).ceil() as i64).clamp(0, pw);
    let y1 = ((y + h).ceil() as i64).clamp(0, ph);
    let block = block.max(1) as i64;
    let data = pm.data_mut();
    let mut by = y0;
    while by < y1 {
        let bh = block.min(y1 - by);
        let mut bx = x0;
        while bx < x1 {
            let bw = block.min(x1 - bx);
            let mut sum = [0u64; 3];
            for row in by..by + bh {
                for col in bx..bx + bw {
                    let i = ((row * pw + col) * 4) as usize;
                    sum[0] += data[i] as u64;
                    sum[1] += data[i + 1] as u64;
                    sum[2] += data[i + 2] as u64;
                }
            }
            let n = (bw * bh) as u64;
            let avg = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];
            for row in by..by + bh {
                for col in bx..bx + bw {
                    let i = ((row * pw + col) * 4) as usize;
                    data[i..i + 3].copy_from_slice(&avg);
                    data[i + 3] = 255;
                }
            }
            bx += bw;
        }
        by += bh;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixelating_averages_each_block() {
        let mut pm = Pixmap::new(4, 2).unwrap();
        // Left half black, right half white.
        for (i, px) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = if i % 4 >= 2 { 255 } else { 0 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        pixelate(&mut pm, (0.0, 0.0, 4.0, 2.0), 4);
        assert!(pm
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [127, 127, 127, 255]));
    }

    #[test]
    fn a_click_with_the_box_tool_draws_nothing() {
        let mut s = Shape::new(Tool::Rect, COLORS[0], 1.0, 1.0, Point::from_xy(5.0, 5.0));
        assert!(!s.is_visible());
        s.extend(Point::from_xy(20.0, 9.0));
        assert!(s.is_visible());
        assert_eq!(s.rect(), Some((5.0, 5.0, 15.0, 4.0)));
    }
}
