//! A made-up screen to show the editor on: a settings page with a title, a
//! secret key, a chart and a Save button. Text is drawn as grey bars, the
//! way a design mockup does, so the landing page shows no real screen and
//! needs no font.
//!
//! Laid out on a 560x400 grid; [`super::render_sample`] places its marks on
//! the same grid.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

pub const WIDTH: f32 = 560.0;
pub const HEIGHT: f32 = 400.0;

pub fn snip(scale: f32) -> Option<Pixmap> {
    let mut pm = Pixmap::new((WIDTH * scale) as u32, (HEIGHT * scale) as u32)?;
    pm.fill(Color::WHITE);
    let t = Transform::from_scale(scale, scale);
    let mut d = Draw { pm: &mut pm, t };

    // Title bar and its three window buttons.
    d.rect(0.0, 0.0, WIDTH, 32.0, 0xf3f4f6);
    for i in 0..3 {
        d.circle(18.0 + i as f32 * 16.0, 16.0, 5.0, 0xd1d5db);
    }
    d.rect(0.0, 32.0, WIDTH, 1.0, 0xe5e7eb);

    // Sidebar, with the current page picked out.
    d.rect(0.0, 33.0, 160.0, HEIGHT - 33.0, 0xf8fafc);
    d.rect(160.0, 33.0, 1.0, HEIGHT - 33.0, 0xe5e7eb);
    d.rounded(10.0, 46.0, 140.0, 26.0, 6.0, 0xe0e7ff);
    for (i, w) in [70.0, 96.0, 82.0, 104.0, 64.0, 88.0]
        .into_iter()
        .enumerate()
    {
        let y = 55.0 + i as f32 * 30.0;
        d.circle(28.0, y + 4.0, 5.0, if i == 0 { 0x4f46e5 } else { 0xcbd5e1 });
        d.rounded(
            42.0,
            y,
            w,
            8.0,
            4.0,
            if i == 0 { 0x4f46e5 } else { 0xcbd5e1 },
        );
    }

    // Heading and two lines of text.
    d.rounded(186.0, 60.0, 140.0, 12.0, 3.0, 0x1f2937);
    d.rounded(186.0, 88.0, 300.0, 7.0, 3.5, 0x9ca3af);
    d.rounded(186.0, 103.0, 236.0, 7.0, 3.5, 0x9ca3af);

    // "API key", its field, and a Copy button.
    d.rounded(186.0, 124.0, 52.0, 7.0, 3.5, 0x6b7280);
    d.rounded(186.0, 140.0, 264.0, 40.0, 6.0, 0xf9fafb);
    d.outline(186.5, 140.5, 263.0, 39.0, 6.0, 0xd1d5db);
    for i in 0..22 {
        let x = 200.0 + i as f32 * 10.5;
        let h = if i % 3 == 1 { 11.0 } else { 9.0 };
        d.rounded(x, 160.0 - h / 2.0, 7.0, h, 1.5, 0x374151);
    }
    d.rounded(460.0, 140.0, 76.0, 40.0, 6.0, 0x2563eb);
    d.rounded(480.0, 157.0, 36.0, 7.0, 3.5, 0xffffff);

    // A card with a bar chart.
    d.rounded(186.0, 220.0, 344.0, 96.0, 8.0, 0xffffff);
    d.outline(186.5, 220.5, 343.0, 95.0, 8.0, 0xe5e7eb);
    d.rounded(202.0, 234.0, 90.0, 7.0, 3.5, 0x6b7280);
    for (i, h) in [28.0, 44.0, 36.0, 58.0, 50.0, 30.0, 46.0, 40.0, 54.0]
        .into_iter()
        .enumerate()
    {
        let x = 214.0 + i as f32 * 34.0;
        let color = if i == 3 { 0x0d9488 } else { 0x5eead4 };
        d.rounded(x, 304.0 - h, 20.0, h, 3.0, color);
    }

    // Save.
    d.rounded(466.0, 350.0, 70.0, 30.0, 6.0, 0x2563eb);
    d.rounded(484.0, 362.0, 34.0, 7.0, 3.5, 0xffffff);

    Some(pm)
}

struct Draw<'a> {
    pm: &'a mut Pixmap,
    t: Transform,
}

fn paint(rgb: u32) -> Paint<'static> {
    let mut p = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    p.set_color_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255);
    p
}

impl Draw<'_> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: u32) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            self.pm.fill_rect(r, &paint(rgb), self.t, None);
        }
    }

    fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: u32) {
        if let Some(p) = super::toolbar::rounded(x, y, w, h, r) {
            self.pm
                .fill_path(&p, &paint(rgb), FillRule::Winding, self.t, None);
        }
    }

    fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: u32) {
        if let Some(p) = super::toolbar::rounded(x, y, w, h, r) {
            let stroke = Stroke {
                width: 1.0,
                ..Stroke::default()
            };
            self.pm.stroke_path(&p, &paint(rgb), &stroke, self.t, None);
        }
    }

    fn circle(&mut self, x: f32, y: f32, r: f32, rgb: u32) {
        if let Some(p) = PathBuilder::from_circle(x, y, r) {
            self.pm
                .fill_path(&p, &paint(rgb), FillRule::Winding, self.t, None);
        }
    }
}
