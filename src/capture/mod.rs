//! A picture of every screen at once, taken before the overlay goes up so
//! the overlay can show it: what you select on is the screen as it was when
//! you pressed the hotkey, menus and tooltips included.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::screen;

#[cfg(not(windows))]
pub fn screen() -> Result<Frame, String> {
    Err("screen capture is not supported on this system yet".into())
}

/// Pixels in physical screen coordinates. `x` and `y` are where the top-left
/// pixel sits on the virtual desktop, which is negative for a monitor left
/// of or above the primary one.
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// `0x00RRGGBB`, row by row: softbuffer's format, so the overlay copies
    /// rows without converting them.
    pub px: Vec<u32>,
}

impl Frame {
    /// The part at `(x, y, width, height)` in desktop coordinates, clipped to
    /// what was captured.
    pub fn crop(&self, x: i32, y: i32, width: u32, height: u32) -> Frame {
        let left = x.max(self.x);
        let top = y.max(self.y);
        let right = (x + width as i32).min(self.x + self.width as i32);
        let bottom = (y + height as i32).min(self.y + self.height as i32);
        let w = (right - left).max(0) as u32;
        let h = (bottom - top).max(0) as u32;
        let mut px = Vec::with_capacity((w * h) as usize);
        for row in 0..h {
            let start = ((top - self.y) as u32 + row) * self.width + (left - self.x) as u32;
            px.extend_from_slice(&self.px[start as usize..(start + w) as usize]);
        }
        Frame {
            x: left,
            y: top,
            width: w,
            height: h,
            px,
        }
    }

    /// Opaque RGBA, for tiny-skia, PNG and the clipboard.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_is_in_desktop_coordinates_and_clipped() {
        // A 4x3 desktop starting left of the primary monitor.
        let f = Frame {
            x: -2,
            y: 0,
            width: 4,
            height: 3,
            px: (0..12).collect(),
        };
        let c = f.crop(-1, 1, 2, 2);
        assert_eq!((c.x, c.y, c.width, c.height), (-1, 1, 2, 2));
        assert_eq!(c.px, vec![5, 6, 9, 10]);
        let c = f.crop(1, 2, 10, 10);
        assert_eq!((c.width, c.height), (1, 1));
        assert_eq!(c.px, vec![11]);
    }
}
