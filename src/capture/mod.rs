//! A picture of every screen at once, taken before the overlay goes up so
//! the overlay can show it: what you select on is the screen as it was when
//! you pressed the hotkey, menus and tooltips included.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::screen;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::screen;

#[cfg(not(any(windows, target_os = "macos")))]
pub fn screen() -> Result<Shot, String> {
    Err("screen capture is not supported on this system yet".into())
}

use tao::monitor::MonitorHandle;

/// Everything on the screens at one moment. Windows takes the whole
/// virtual desktop in one picture; macOS takes one picture per display,
/// because a Retina screen next to a plain one has two pixel sizes and no
/// single picture in physical pixels can hold both without overlapping.
pub struct Shot {
    parts: Vec<Part>,
}

struct Part {
    /// The display the picture is of: the `CGDirectDisplayID` on macOS,
    /// None for the one picture of the whole desktop.
    display: Option<u32>,
    frame: Frame,
}

impl Shot {
    /// One picture of the whole desktop.
    #[allow(dead_code)]
    pub fn whole(frame: Frame) -> Self {
        Shot {
            parts: vec![Part {
                display: None,
                frame,
            }],
        }
    }

    /// One picture per display.
    #[allow(dead_code)]
    pub fn per_display(parts: Vec<(u32, Frame)>) -> Self {
        Shot {
            parts: parts
                .into_iter()
                .map(|(id, frame)| Part {
                    display: Some(id),
                    frame,
                })
                .collect(),
        }
    }

    /// What `monitor` showed, in the coordinates its overlay window uses:
    /// placed at the monitor's position and exactly the monitor's size.
    pub fn monitor(&self, monitor: &MonitorHandle) -> Frame {
        let pos = monitor.position();
        let size = monitor.size();
        #[cfg(target_os = "macos")]
        {
            use tao::platform::macos::MonitorHandleExtMacOS;
            let id = monitor.native_id();
            if let Some(p) = self.parts.iter().find(|p| p.display == Some(id)) {
                return p
                    .frame
                    .resized(size.width, size.height)
                    .placed(pos.x, pos.y);
            }
        }
        match self.parts.iter().find(|p| p.display.is_none()) {
            Some(p) => p.frame.crop(pos.x, pos.y, size.width, size.height),
            // A display that appeared after the picture: black.
            None => Frame {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
                px: vec![0; (size.width * size.height) as usize],
            },
        }
    }

    /// For the log: `3024x1964 + 1920x1080`.
    pub fn describe(&self) -> String {
        let sizes: Vec<String> = self
            .parts
            .iter()
            .map(|p| format!("{}x{}", p.frame.width, p.frame.height))
            .collect();
        sizes.join(" + ")
    }
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

    /// The same pixels with the top-left at `(x, y)`.
    #[allow(dead_code)]
    fn placed(mut self, x: i32, y: i32) -> Frame {
        self.x = x;
        self.y = y;
        self
    }

    /// Stretched to `width` by `height`, nearest pixel. A copy when the size
    /// is already right, which it nearly always is.
    #[allow(dead_code)]
    fn resized(&self, width: u32, height: u32) -> Frame {
        if (width, height) == (self.width, self.height) || self.width == 0 || self.height == 0 {
            return Frame {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
                px: self.px.clone(),
            };
        }
        let mut px = Vec::with_capacity((width * height) as usize);
        for row in 0..height {
            let sy = (row as u64 * self.height as u64 / height as u64) as u32;
            let line = &self.px[(sy * self.width) as usize..][..self.width as usize];
            for col in 0..width {
                let sx = (col as u64 * self.width as u64 / width as u64) as usize;
                px.push(line[sx]);
            }
        }
        Frame {
            x: self.x,
            y: self.y,
            width,
            height,
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

    #[test]
    fn resizing_doubles_each_pixel() {
        let f = Frame {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            px: vec![1, 2],
        };
        let r = f.resized(4, 2);
        assert_eq!(r.px, vec![1, 1, 2, 2, 1, 1, 2, 2]);
        assert_eq!(f.resized(2, 1).px, vec![1, 2]);
    }
}
