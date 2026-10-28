//! GDI: one `BitBlt` of the whole virtual desktop.
//!
//! The Graphics Capture API is the modern route, but it is a stream of
//! frames per monitor behind a D3D device. For one still of everything, GDI
//! is a single call, a few tens of milliseconds even across 4K screens, and
//! it sees what DWM composed, so it is the same picture.
//!
//! The process is per-monitor DPI aware (the manifest says so), so every
//! coordinate here is a physical pixel.

use super::Frame;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub fn screen() -> Result<Frame, String> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return Err("no screen to capture".into());
        }

        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: top-down rows, the order everything else uses.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(dib) => {
                let old = SelectObject(mem, dib.into());
                // CAPTUREBLT includes layered windows: tooltips, menus with
                // shadows, the notification flyouts.
                let blit = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
                let frame = blit.map_err(|e| format!("BitBlt failed: {e}")).map(|()| {
                    let src = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
                    Frame {
                        x,
                        y,
                        width: w as u32,
                        height: h as u32,
                        // BGRA in memory is 0xAARRGGBB as a u32; the alpha
                        // byte is undefined for a screen blit, so drop it.
                        px: src.iter().map(|p| p & 0x00ff_ffff).collect(),
                    }
                });
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
                frame
            }
            Err(e) => Err(format!("could not allocate the capture ({e})")),
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}
