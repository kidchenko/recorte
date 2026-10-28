//! The picture goes on as two formats at once: `CF_DIB`, which every
//! Windows program reads (Paint, Word, Teams), and `PNG`, the registered
//! format browsers and Office prefer because it keeps the pixels exact.

use traykit::ldebug;
use traykit::util::{pcwstr, wide};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;

pub fn set_image(width: u32, height: u32, rgba: &[u8], png: &[u8]) -> Result<(), String> {
    let dib = dib(width, height, rgba);
    open()?;
    let result = unsafe {
        let _ = EmptyClipboard();
        put(CF_DIB.0 as u32, &dib).and_then(|()| {
            let name = wide("PNG");
            match RegisterClipboardFormatW(pcwstr(&name)) {
                0 => Ok(()),
                png_format => put(png_format, png),
            }
        })
    };
    unsafe {
        let _ = CloseClipboard();
    }
    result
}

/// Another program can hold the clipboard for a moment (a clipboard
/// manager reading what was just copied), so try for about half a second.
fn open() -> Result<(), String> {
    for attempt in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        ldebug!("clipboard busy, attempt {attempt}");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("the clipboard is in use by another program".into())
}

/// Copy `bytes` into a movable global block and hand it over. Once
/// `SetClipboardData` succeeds the block belongs to the clipboard.
unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| format!("out of memory: {e}"))?;
    let dst = GlobalLock(mem) as *mut u8;
    if dst.is_null() {
        let _ = GlobalFree(Some(mem));
        return Err("could not lock clipboard memory".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
    let _ = GlobalUnlock(mem);
    match SetClipboardData(format, Some(HANDLE(mem.0))) {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = GlobalFree(Some(HGLOBAL(mem.0)));
            Err(format!("could not set clipboard data: {e}"))
        }
    }
}

/// A packed DIB: the header, then 32-bit BGRA rows bottom-up, which is the
/// orientation every reader of `CF_DIB` handles. Alpha stays 255: a snip is
/// opaque, and a reader that honours alpha would otherwise show nothing.
fn dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: width * height * 4,
        ..Default::default()
    };
    let size = std::mem::size_of::<BITMAPINFOHEADER>();
    let mut out = Vec::with_capacity(size + rgba.len());
    out.extend_from_slice(unsafe {
        std::slice::from_raw_parts(&header as *const _ as *const u8, size)
    });
    let stride = width as usize * 4;
    for row in rgba.chunks_exact(stride).rev() {
        for px in row.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    out
}
