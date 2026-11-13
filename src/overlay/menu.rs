//! The menu at the top of the overlay: a camera and a film camera, to choose
//! between a snip and a recording of what is selected next, and a cross to
//! close. The hotkey that brought the overlay up has already chosen one, so
//! the menu is there to change your mind, and to show which it will be.
//!
//! It is a picture laid over the frozen screen, not a window of its own:
//! drawn like the editor's toolbar, with round corners, then blended in.

use crate::editor::{rounded_rect, Palette};
use crate::hotkey::Action;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Logical pixels.
const WIDTH: f32 = PAD + 2.0 * (BUTTON + GAP) + SEPARATOR + BUTTON + PAD;
const HEIGHT: f32 = 48.0;
/// From the top of the monitor. A recording's bar goes in the same place.
pub const TOP: f32 = 20.0;
const PAD: f32 = 6.0;
const BUTTON: f32 = 40.0;
const BUTTON_HEIGHT: f32 = 36.0;
const GAP: f32 = 2.0;
const SEPARATOR: f32 = 13.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Mode(Action),
    Close,
}

pub struct State {
    pub mode: Action,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
}

/// The menu's size, in physical pixels.
pub fn size(scale: f32) -> (u32, u32) {
    (
        (WIDTH * scale).round() as u32,
        (HEIGHT * scale).round() as u32,
    )
}

/// Where its top-left corner goes on a monitor `width` pixels wide.
pub fn origin(width: u32, scale: f32) -> (i32, i32) {
    (
        (width as i32 - size(scale).0 as i32) / 2,
        (TOP * scale).round() as i32,
    )
}

/// The buttons as `(hit, x, y, width, height)`, in physical pixels from the
/// menu's corner.
fn layout(scale: f32) -> [(Hit, f32, f32, f32, f32); 3] {
    let s = scale;
    let y = (HEIGHT - BUTTON_HEIGHT) / 2.0 * s;
    let at = |hit, x: f32| (hit, x * s, y, BUTTON * s, BUTTON_HEIGHT * s);
    [
        at(Hit::Mode(Action::Snip), PAD),
        at(Hit::Mode(Action::Record), PAD + BUTTON + GAP),
        at(Hit::Close, WIDTH - PAD - BUTTON),
    ]
}

/// What is at `(x, y)`, in pixels from the menu's corner.
pub fn hit(x: f32, y: f32, scale: f32) -> Option<Hit> {
    layout(scale)
        .into_iter()
        .find(|&(_, bx, by, w, h)| x >= bx && x < bx + w && y >= by && y < by + h)
        .map(|(hit, ..)| hit)
}

/// The menu as a picture, see-through outside its round corners.
pub fn draw(scale: f32, state: &State, look: &Palette) -> Option<Pixmap> {
    let s = scale;
    let (w, h) = size(s);
    let mut pm = Pixmap::new(w, h)?;
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    let fill = |pm: &mut Pixmap, paint: &Paint, (x, y, w, h): (f32, f32, f32, f32), r: f32| {
        if let Some(path) = rounded_rect(x, y, w, h, r) {
            pm.fill_path(&path, paint, FillRule::Winding, Transform::identity(), None);
        }
    };
    // A hairline around it, then the bar inside that.
    let edge = s.round().max(1.0);
    let (wf, hf) = (w as f32, h as f32);
    paint.set_color(look.line);
    fill(&mut pm, &paint, (0.0, 0.0, wf, hf), 10.0 * s);
    paint.set_color(look.bar);
    fill(
        &mut pm,
        &paint,
        (edge, edge, wf - 2.0 * edge, hf - 2.0 * edge),
        10.0 * s - edge,
    );
    let x = (WIDTH - PAD - BUTTON - SEPARATOR / 2.0) * s;
    paint.set_color(look.line);
    fill(
        &mut pm,
        &paint,
        (x - edge / 2.0, 12.0 * s, edge, hf - 24.0 * s),
        0.0,
    );

    for (hit, x, y, bw, bh) in layout(s) {
        let selected = hit == Hit::Mode(state.mode);
        let bg = if selected || state.pressed == Some(hit) {
            Some(look.selected)
        } else if state.hover == Some(hit) {
            Some(look.hover)
        } else {
            None
        };
        if let Some(bg) = bg {
            paint.set_color(bg);
            fill(&mut pm, &paint, (x, y, bw, bh), 6.0 * s);
        }
        if selected {
            paint.set_color(look.accent);
            fill(
                &mut pm,
                &paint,
                (x + bw * 0.3, y + bh - 3.0 * s, bw * 0.4, 2.5 * s),
                1.25 * s,
            );
        }
        // The same 20x20 grid as the editor's glyphs, centred in the button.
        let t = Transform::from_row(
            s,
            0.0,
            0.0,
            s,
            x + (bw - 20.0 * s) / 2.0,
            y + (bh - 20.0 * s) / 2.0,
        );
        paint.set_color(look.fg);
        if let Some(path) = glyph(hit) {
            let stroke = Stroke {
                // In grid units; the transform scales it with the rest.
                width: 1.6,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pm.stroke_path(&path, &paint, &stroke, t, None);
        }
    }
    Some(pm)
}

fn glyph(hit: Hit) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    match hit {
        // A camera: the body, the bump the viewfinder is in, the lens.
        Hit::Mode(Action::Snip) => {
            if let Some(body) = rounded_rect(2.5, 6.0, 15.0, 10.5, 2.0) {
                pb.push_path(&body);
            }
            pb.move_to(7.0, 6.0);
            pb.line_to(8.2, 3.8);
            pb.line_to(11.8, 3.8);
            pb.line_to(13.0, 6.0);
            pb.push_circle(10.0, 11.2, 2.9);
        }
        // A film camera: the body, and the lens hood widening from it.
        Hit::Mode(Action::Record) => {
            if let Some(body) = rounded_rect(2.0, 5.5, 11.0, 9.0, 2.0) {
                pb.push_path(&body);
            }
            pb.move_to(13.0, 8.8);
            pb.line_to(17.8, 6.2);
            pb.line_to(17.8, 13.8);
            pb.line_to(13.0, 11.2);
        }
        Hit::Close => {
            pb.move_to(5.5, 5.5);
            pb.line_to(14.5, 14.5);
            pb.move_to(14.5, 5.5);
            pb.line_to(5.5, 14.5);
        }
    }
    pb.finish()
}

/// Lay `menu` over `buf`, a window's pixels `stride` wide, with its corner
/// at `(x, y)`. What falls outside the window is left out.
pub fn blend(buf: &mut [u32], stride: usize, menu: &Pixmap, (x, y): (i32, i32)) {
    let rows = buf.len() / stride.max(1);
    let (mw, mh) = (menu.width() as usize, menu.height() as usize);
    let px = menu.data().as_chunks::<4>().0;
    for row in 0..mh {
        let dy = y + row as i32;
        if dy < 0 || dy as usize >= rows {
            continue;
        }
        for col in 0..mw {
            let dx = x + col as i32;
            if dx < 0 || dx as usize >= stride {
                continue;
            }
            // tiny-skia's colours are already multiplied by their alpha.
            let [r, g, b, a] = px[row * mw + col];
            let dst = &mut buf[dy as usize * stride + dx as usize];
            let keep = 255 - a as u32;
            let mix = |over: u8, shift: u32| over as u32 + ((*dst >> shift) & 0xff) * keep / 255;
            *dst = mix(r, 16) << 16 | mix(g, 8) << 8 | mix(b, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_are_found_where_they_are_drawn() {
        for scale in [1.0, 1.25, 2.0] {
            let mid = HEIGHT / 2.0 * scale;
            let at = |x: f32| hit(x * scale, mid, scale);
            assert_eq!(at(PAD + 1.0), Some(Hit::Mode(Action::Snip)));
            assert_eq!(
                at(PAD + BUTTON + GAP + 1.0),
                Some(Hit::Mode(Action::Record))
            );
            assert_eq!(at(WIDTH - PAD - 1.0), Some(Hit::Close));
            // The separator, and the padding around the buttons.
            assert_eq!(at(WIDTH - PAD - BUTTON - 2.0), None);
            assert_eq!(hit((PAD + 1.0) * scale, 1.0, scale), None);
            let (w, _) = size(scale);
            let last = layout(scale)[2];
            assert!(last.1 + last.3 <= w as f32);
        }
    }

    #[test]
    fn it_is_centred_at_the_top() {
        assert_eq!(size(1.0), (149, 48));
        assert_eq!(origin(1920, 1.0), (885, 20));
        assert_eq!(origin(3840, 2.0), (1771, 40));
    }

    #[test]
    fn blending_covers_where_it_is_solid_and_clips_at_the_edges() {
        let mut menu = Pixmap::new(2, 2).unwrap();
        menu.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let mut buf = vec![0x00ff_ffff_u32; 9];
        // One pixel of it hangs off the right and the bottom.
        blend(&mut buf, 3, &menu, (2, 2));
        assert_eq!(buf[8], 0x000a_141e);
        assert!(buf[..8].iter().all(|p| *p == 0x00ff_ffff));
        // See-through leaves the screen as it was.
        let clear = Pixmap::new(3, 3).unwrap();
        blend(&mut buf, 3, &clear, (0, 0));
        assert_eq!(buf[0], 0x00ff_ffff);
        assert_eq!(buf[8], 0x000a_141e);
    }
}
