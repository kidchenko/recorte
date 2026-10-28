//! A finished picture: what the overlay cropped, or what the editor
//! exported. Encoding, file names and the folder it is saved to.

use std::path::{Path, PathBuf};
use traykit::time::LocalTime;

pub struct Snip {
    pub width: u32,
    pub height: u32,
    /// Opaque RGBA.
    pub rgba: Vec<u8>,
    /// The scale factor of the monitor it was taken on, so a pen stroke is
    /// as thick on a 200% screen as on a 100% one.
    pub scale: f64,
}

impl Snip {
    pub fn png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        // Screenshots are flat colour and text; the fast end of deflate is
        // most of the gain and keeps saving instant.
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        // RGB rather than RGBA: the alpha is always 255, and a quarter of
        // the file for nothing.
        let rgb: Vec<u8> = self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }

    pub fn copy(&self) -> Result<(), String> {
        crate::clipboard::set_image(self.width, self.height, &self.rgba, &self.png()?)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.png()?).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// `Print 2026-10-28 14-03-11.png`: sorts by time, and has no characters a
/// file system or a chat upload would object to.
pub fn file_name(t: LocalTime) -> String {
    format!(
        "Print {:04}-{:02}-{:02} {:02}-{:02}-{:02}.png",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// A path in `dir` that does not exist yet: two snips in one second get
/// `(2)`, `(3)` rather than overwriting each other.
pub fn fresh_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = name.trim_end_matches(".png");
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).png")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// The user's Pictures folder, wherever they moved it.
#[cfg(windows)]
pub fn pictures_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}

#[cfg(not(windows))]
pub fn pictures_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_by_time() {
        let t = LocalTime {
            year: 2026,
            month: 3,
            day: 9,
            hour: 7,
            minute: 5,
            second: 1,
            millis: 0,
        };
        assert_eq!(file_name(t), "Print 2026-03-09 07-05-01.png");
    }

    #[test]
    fn png_round_trips() {
        let s = Snip {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
            scale: 1.0,
        };
        let bytes = s.png().unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        r.next_frame(&mut buf).unwrap();
        assert_eq!(&buf[..6], &[255, 0, 0, 0, 0, 255]);
    }
}
