//! The little bar of a recording: start, pause and stop, how long it has
//! been, whether the microphone and the computer's sound are in it, and a
//! cross to close it, which once it films is a bin for the recording.
//!
//! It is the size of the overlay's menu, whose place it takes.
//!
//! Drawn like the editor's toolbar, from a few path commands at the screen's
//! scale. The clock's digits are seven strokes each, as on a stopwatch, so
//! there is still no font in the binary.

use crate::editor::{rounded_rect, Palette};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

/// Logical pixels.
pub const WIDTH: f32 = DISCARD_X + BUTTON + PAD;
pub const HEIGHT: f32 = 48.0;
const PAD: f32 = 6.0;
const BUTTON: f32 = 36.0;
/// Between two buttons, around the clock, and where the line before the
/// cross is.
const GAP: f32 = 2.0;
const SPACE: f32 = 6.0;
const SEPARATOR: f32 = 13.0;
/// Room for `000:00`: a recording can pass a hundred minutes.
const CLOCK: f32 = 64.0;

/// Where each thing starts, from the left.
const STOP_X: f32 = PAD + BUTTON + GAP;
const CLOCK_X: f32 = STOP_X + BUTTON + SPACE;
const MIC_X: f32 = CLOCK_X + CLOCK + SPACE;
const SYSTEM_X: f32 = MIC_X + BUTTON + GAP;
const DISCARD_X: f32 = SYSTEM_X + BUTTON + SEPARATOR;

/// A digit's box, the space between two, and the colon's width.
const DIGIT: (f32, f32) = (8.0, 14.0);
const SPACING: f32 = 3.5;
const COLON: f32 = 2.0;

/// The red of a record button, in both themes, and of the start button
/// under the pointer and held down.
pub const RED: [u8; 3] = [0xe5, 0x3b, 0x3b];
const RED_OVER: [u8; 3] = [0xee, 0x55, 0x55];
const RED_DOWN: [u8; 3] = [0xc9, 0x30, 0x30];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    /// Start; then pause, and carry on.
    Main,
    /// Stop and keep.
    Stop,
    /// The microphone, in the recording or not.
    Mic,
    /// What the computer plays, the same.
    System,
    /// Close without a recording: asked twice once there is one to lose.
    Discard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The area is chosen and nothing is filmed yet.
    Ready,
    /// Start was pressed and the encoder is being set up: a moment.
    Starting,
    Filming,
    Paused,
}

pub struct State {
    pub phase: Phase,
    pub seconds: u64,
    pub mic: bool,
    pub system: bool,
    /// The off half of the paused clock's blink.
    pub blink: bool,
    /// The bin was pressed once: once more throws the recording away.
    pub armed: bool,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
}

/// The buttons as `(hit, x, y, edge)`, in physical pixels.
fn layout(scale: f32) -> [(Hit, f32, f32, f32); 5] {
    let s = scale;
    let y = (HEIGHT - BUTTON) / 2.0 * s;
    let at = |hit, x: f32| (hit, x * s, y, BUTTON * s);
    [
        at(Hit::Main, PAD),
        at(Hit::Stop, STOP_X),
        at(Hit::Mic, MIC_X),
        at(Hit::System, SYSTEM_X),
        at(Hit::Discard, DISCARD_X),
    ]
}

pub fn hit(x: f32, y: f32, scale: f32) -> Option<Hit> {
    layout(scale)
        .into_iter()
        .find(|&(_, bx, by, edge)| x >= bx && x < bx + edge && y >= by && y < by + edge)
        .map(|(hit, ..)| hit)
}

/// `75` is `01:15`. Minutes go on past 99 rather than turn into hours.
pub fn clock(seconds: u64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

pub fn draw(pm: &mut Pixmap, scale: f32, state: &State, look: &Palette) {
    let s = scale;
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    // A hairline around the bar: the window has no frame of its own, and
    // over a window of the same grey it would have no edge at all.
    pm.fill(look.line);
    paint.set_color(look.bar);
    let edge = s.round().max(1.0);
    if let Some(r) = Rect::from_xywh(edge, edge, w - 2.0 * edge, h - 2.0 * edge) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
    // And one before the cross, which is not like the others: it ends it.
    paint.set_color(look.line);
    let x = ((DISCARD_X - SEPARATOR / 2.0) * s - edge / 2.0).round();
    if let Some(r) = Rect::from_xywh(x, 12.0 * s, edge, h - 24.0 * s) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }

    let rgb = |[r, g, b]: [u8; 3]| Color::from_rgba8(r, g, b, 255);
    let red = rgb(RED);
    let waiting = matches!(state.phase, Phase::Ready | Phase::Starting);
    for (hit, x, y, side) in layout(s) {
        // There is nothing to stop before it starts, and nothing to press
        // while it is starting.
        let dead = match hit {
            Hit::Stop => waiting,
            Hit::Main => state.phase == Phase::Starting,
            _ => false,
        };
        let (held, over) = (state.pressed == Some(hit), state.hover == Some(hit));
        let bg = if dead {
            None
        } else if hit == Hit::Main && state.phase == Phase::Ready {
            // The one thing to do next: a button that looks like one, where
            // a bare red dot would look like a light that is already on.
            Some(rgb(if held {
                RED_DOWN
            } else if over {
                RED_OVER
            } else {
                RED
            }))
        } else if held || (hit == Hit::Discard && state.armed) {
            Some(look.selected)
        } else if over {
            Some(look.hover)
        } else {
            None
        };
        if let Some(bg) = bg {
            paint.set_color(bg);
            if let Some(path) = rounded_rect(x, y, side, side, 6.0 * s) {
                pm.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
        // The same 20x20 grid as the editor's glyphs, centred in the button.
        let t = Transform::from_row(
            s,
            0.0,
            0.0,
            s,
            x + (side - 20.0 * s) / 2.0,
            y + (side - 20.0 * s) / 2.0,
        );
        let mut fill = |pm: &mut Pixmap, path: Option<Path>, color: Color| {
            paint.set_color(color);
            if let Some(path) = path {
                pm.fill_path(&path, &paint, FillRule::Winding, t, None);
            }
        };
        match hit {
            // A dot in a ring to start or carry on, two bars to pause, a
            // square to stop, as on any recorder.
            Hit::Main if matches!(state.phase, Phase::Filming | Phase::Starting) => {
                let mut pb = PathBuilder::new();
                for x in [5.0, 11.5] {
                    if let Some(bar) = rounded_rect(x, 4.0, 3.5, 12.0, 1.2) {
                        pb.push_path(&bar);
                    }
                }
                fill(pm, pb.finish(), if dead { look.faint } else { look.fg });
            }
            Hit::Main => {
                // White on the red button; paused, a red dot in the bar's ink.
                let (ring, dot) = if state.phase == Phase::Ready {
                    (Color::WHITE, Color::WHITE)
                } else {
                    (look.fg, red)
                };
                fill(pm, PathBuilder::from_circle(10.0, 10.0, 3.4), dot);
                draw_stroke(pm, PathBuilder::from_circle(10.0, 10.0, 6.8), ring, t);
            }
            Hit::Stop => {
                let color = if dead { look.faint } else { red };
                fill(pm, rounded_rect(4.0, 4.0, 12.0, 12.0, 2.5), color);
            }
            Hit::Mic | Hit::System => {
                let on = if hit == Hit::Mic {
                    state.mic
                } else {
                    state.system
                };
                let glyph = if hit == Hit::Mic { mic() } else { speaker() };
                let ink = if on { look.fg } else { look.faint };
                draw_stroke(pm, glyph, ink, t);
                if on {
                    // Like the selected tool in the editor.
                    fill(pm, rounded_rect(6.0, 19.5, 8.0, 2.0, 1.0), look.accent);
                } else {
                    draw_stroke(pm, line((3.5, 3.5), (16.5, 16.5)), look.fg, t);
                }
            }
            // With nothing filmed it only closes. After that it is a bin,
            // red once pressed: the next press empties it.
            Hit::Discard if waiting => draw_stroke(pm, cross(), look.fg, t),
            Hit::Discard => {
                let ink = if state.armed { red } else { look.fg };
                draw_stroke(pm, bin(), ink, t);
            }
        }
    }

    // Faint before it starts: it is not counting. Paused, it blinks.
    let ink = if waiting || (state.phase == Phase::Paused && state.blink) {
        look.faint
    } else {
        look.fg
    };
    let text = clock(state.seconds);
    let x = (CLOCK_X + (CLOCK - text_width(&text)) / 2.0) * s;
    let y = (HEIGHT - DIGIT.1) / 2.0 * s;
    draw_clock(pm, &text, Transform::from_row(s, 0.0, 0.0, s, x, y), ink);
}

fn draw_stroke(pm: &mut Pixmap, path: Option<Path>, ink: Color, t: Transform) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(ink);
    if let Some(path) = path {
        pm.stroke_path(&path, &paint, &stroke(), t, None);
    }
}

fn stroke() -> Stroke {
    Stroke {
        // In grid units; the transform scales it with everything else.
        width: 1.6,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn line(from: (f32, f32), to: (f32, f32)) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(from.0, from.1);
    pb.line_to(to.0, to.1);
    pb.finish()
}

fn cross() -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(5.5, 5.5);
    pb.line_to(14.5, 14.5);
    pb.move_to(14.5, 5.5);
    pb.line_to(5.5, 14.5);
    pb.finish()
}

/// A bin: the lid and its handle, and the body narrowing down.
fn bin() -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(4.5, 6.0);
    pb.line_to(15.5, 6.0);
    pb.move_to(8.0, 6.0);
    pb.line_to(8.6, 4.0);
    pb.line_to(11.4, 4.0);
    pb.line_to(12.0, 6.0);
    pb.move_to(6.0, 6.0);
    pb.line_to(6.8, 16.5);
    pb.line_to(13.2, 16.5);
    pb.line_to(14.0, 6.0);
    pb.finish()
}

/// A microphone: the capsule, the cradle under it, and its stand.
fn mic() -> Option<Path> {
    let mut pb = PathBuilder::new();
    if let Some(capsule) = rounded_rect(7.5, 2.0, 5.0, 9.5, 2.5) {
        pb.push_path(&capsule);
    }
    pb.move_to(4.8, 9.0);
    pb.quad_to(4.8, 14.2, 10.0, 14.2);
    pb.quad_to(15.2, 14.2, 15.2, 9.0);
    pb.move_to(10.0, 14.2);
    pb.line_to(10.0, 17.5);
    pb.move_to(7.5, 17.5);
    pb.line_to(12.5, 17.5);
    pb.finish()
}

/// A loudspeaker, and two waves coming from it.
fn speaker() -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(2.5, 7.8);
    pb.line_to(5.5, 7.8);
    pb.line_to(9.5, 4.3);
    pb.line_to(9.5, 15.7);
    pb.line_to(5.5, 12.2);
    pb.line_to(2.5, 12.2);
    pb.close();
    pb.move_to(12.3, 7.6);
    pb.quad_to(14.3, 10.0, 12.3, 12.4);
    pb.move_to(14.6, 5.2);
    pb.quad_to(18.6, 10.0, 14.6, 14.8);
    pb.finish()
}

fn advance(c: char) -> f32 {
    if c == ':' {
        COLON
    } else {
        DIGIT.0
    }
}

fn text_width(text: &str) -> f32 {
    let glyphs: f32 = text.chars().map(advance).sum();
    glyphs + SPACING * text.chars().count().saturating_sub(1) as f32
}

/// Which of the seven strokes a digit lights, as bits 0 to 6 for the
/// segments in [`SEGMENTS`].
const DIGITS: [u8; 10] = [
    0b011_1111, // 0
    0b000_0110, // 1
    0b101_1011, // 2
    0b100_1111, // 3
    0b110_0110, // 4
    0b110_1101, // 5
    0b111_1101, // 6
    0b000_0111, // 7
    0b111_1111, // 8
    0b110_1111, // 9
];

/// The strokes in a digit's box, in the order a display names them, a to g:
/// top, the two on the right going down, bottom, the two on the left going
/// up, middle. Each stops short of the corners so that the round ends of two
/// neighbours meet without making a blob.
const SEGMENTS: [[(f32, f32); 2]; 7] = [
    [(1.4, 0.0), (6.6, 0.0)],
    [(8.0, 1.4), (8.0, 5.6)],
    [(8.0, 8.4), (8.0, 12.6)],
    [(1.4, 14.0), (6.6, 14.0)],
    [(0.0, 8.4), (0.0, 12.6)],
    [(0.0, 1.4), (0.0, 5.6)],
    [(1.4, 7.0), (6.6, 7.0)],
];

fn draw_clock(pm: &mut Pixmap, text: &str, t: Transform, ink: Color) {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color(ink);
    let mut x = 0.0;
    for c in text.chars() {
        if let Some(d) = c.to_digit(10) {
            let mut pb = PathBuilder::new();
            for (i, [from, to]) in SEGMENTS.iter().enumerate() {
                if DIGITS[d as usize] & (1 << i) != 0 {
                    pb.move_to(x + from.0, from.1);
                    pb.line_to(x + to.0, to.1);
                }
            }
            draw_stroke(pm, pb.finish(), ink, t);
        } else {
            for y in [4.0, 10.0] {
                if let Some(dot) = PathBuilder::from_circle(x + COLON / 2.0, y, 1.0) {
                    pm.fill_path(&dot, &paint, FillRule::Winding, t, None);
                }
            }
        }
        x += advance(c) + SPACING;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_counts_minutes_and_seconds() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(75), "01:15");
        assert_eq!(clock(100 * 60 + 9), "100:09");
    }

    #[test]
    fn the_longest_clock_fits() {
        assert!(text_width("888:88") <= CLOCK);
    }

    #[test]
    fn digits_light_the_right_strokes() {
        let lit = |d: usize| DIGITS[d].count_ones();
        assert_eq!(lit(1), 2);
        assert_eq!(lit(7), 3);
        assert_eq!(lit(8), 7);
        // A 6 is an 8 without the top right, a 9 without the bottom left.
        assert_eq!(DIGITS[6], DIGITS[8] & !(1 << 1));
        assert_eq!(DIGITS[9], DIGITS[8] & !(1 << 4));
    }

    #[test]
    fn the_buttons_are_found_where_they_are_drawn() {
        assert_eq!(WIDTH, 285.0);
        for scale in [1.0, 1.5, 2.0] {
            let mid = HEIGHT / 2.0 * scale;
            let at = |x: f32| hit(x * scale, mid, scale);
            assert_eq!(at(PAD + 1.0), Some(Hit::Main));
            assert_eq!(at(STOP_X + 1.0), Some(Hit::Stop));
            assert_eq!(at(MIC_X + 1.0), Some(Hit::Mic));
            assert_eq!(at(SYSTEM_X + 1.0), Some(Hit::System));
            assert_eq!(at(WIDTH - PAD - 1.0), Some(Hit::Discard));
            // The clock, the line before the cross, and above the buttons.
            assert_eq!(at(CLOCK_X + CLOCK / 2.0), None);
            assert_eq!(at(DISCARD_X - 2.0), None);
            assert_eq!(hit((PAD + 1.0) * scale, 1.0, scale), None);
        }
    }
}
