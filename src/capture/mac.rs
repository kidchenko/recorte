//! CoreGraphics: one `CGDisplayCreateImage` per display.
//!
//! ScreenCaptureKit is where Apple is taking this, but for one still of each
//! display it is an async stream set-up for what CoreGraphics does in one
//! call, and both are behind the same Screen Recording permission.
//!
//! Without that permission macOS still hands back a picture, but of the
//! wallpaper alone, every window left out. So it is checked first: the
//! first time, macOS asks; after that, System Settings is opened at the
//! right page. macOS applies a new grant only to a fresh process, so
//! Recorte has to be restarted once it is given.

use super::{Frame, Shot};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use traykit::lwarn;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type Ref = *mut c_void;

/// Four bytes a pixel, B G R and an unused one in memory: `0x00RRGGBB` as a
/// little-endian u32, softbuffer's format.
const NONE_SKIP_FIRST: u32 = 6; // kCGImageAlphaNoneSkipFirst
const BYTE_ORDER_32_LITTLE: u32 = 2 << 12; // kCGBitmapByteOrder32Little

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayCreateImage(display: u32) -> Ref;
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGImageGetColorSpace(image: Ref) -> Ref;
    fn CGImageRelease(image: Ref);
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGColorSpaceRelease(space: Ref);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Ref,
        info: u32,
    ) -> Ref;
    fn CGContextDrawImage(context: Ref, rect: CGRect, image: Ref);
    fn CGContextRelease(context: Ref);
}

/// Whether macOS has been asked in this run already: it shows its own
/// question once, ever, so the second time it is System Settings instead.
static ASKED: AtomicBool = AtomicBool::new(false);

pub fn screen() -> Result<Shot, String> {
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        ask_for_permission();
        return Err(
            "Recorte needs the Screen Recording permission: allow it in System Settings \
                    > Privacy & Security > Screen & System Audio Recording, then quit and \
                    start Recorte again"
                .into(),
        );
    }

    let mut ids = [0u32; 32];
    let mut count = 0u32;
    let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Err(format!("could not list the displays (CGError {err})"));
    }
    let mut parts = Vec::new();
    for &id in &ids[..count as usize] {
        match display(id) {
            Ok(frame) => parts.push((id, frame)),
            // One display failing (being switched off as it is taken) should
            // not lose the others; its overlay shows black.
            Err(e) => lwarn!("display {id}: {e}"),
        }
    }
    if parts.is_empty() {
        return Err("no screen to capture".into());
    }
    Ok(Shot::per_display(parts))
}

fn ask_for_permission() {
    if !ASKED.swap(true, Ordering::Relaxed) && !unsafe { CGRequestScreenCaptureAccess() } {
        // The first call shows macOS's own question, unless it was answered
        // in an earlier run; then it is no help, and the settings page is.
        return;
    }
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn();
}

/// One display at its full pixel size: 2x its size in points on a Retina
/// screen.
fn display(id: u32) -> Result<Frame, String> {
    unsafe {
        let image = CGDisplayCreateImage(id);
        if image.is_null() {
            return Err("CGDisplayCreateImage returned nothing".into());
        }
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        let mut px = vec![0u32; width * height];
        // Drawn into a bitmap of known layout rather than read as it comes,
        // because the image's own layout is the window server's to choose.
        // The image's colour space is kept, so the pixels are not converted
        // and the frozen screen looks exactly like the live one.
        let mut space = CGImageGetColorSpace(image);
        let own_space = space.is_null();
        if own_space {
            space = CGColorSpaceCreateDeviceRGB();
        }
        let context = CGBitmapContextCreate(
            px.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space,
            NONE_SKIP_FIRST | BYTE_ORDER_32_LITTLE,
        );
        let result = if context.is_null() {
            Err(format!("could not make a {width}x{height} bitmap"))
        } else {
            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: width as f64,
                    height: height as f64,
                },
            };
            CGContextDrawImage(context, rect, image);
            CGContextRelease(context);
            for p in &mut px {
                *p &= 0x00ff_ffff;
            }
            Ok(Frame {
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
                px,
            })
        };
        if own_space {
            CGColorSpaceRelease(space);
        }
        CGImageRelease(image);
        result
    }
}
