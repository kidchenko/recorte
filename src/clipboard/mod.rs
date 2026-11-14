//! Putting a picture on the clipboard.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::set_image;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::set_image;

/// Not written yet on Linux.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_image(_width: u32, _height: u32, _rgba: &[u8], _png: &[u8]) -> Result<(), String> {
    Err("the clipboard is not supported on this system yet".into())
}
