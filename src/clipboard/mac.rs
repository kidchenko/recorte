//! The general pasteboard, with the picture as PNG and as TIFF: PNG for
//! browsers, chat apps and anything recent, TIFF for the older Cocoa apps
//! that only look for the type `NSImage` writes.

use objc2::AnyThread;
use objc2_app_kit::{NSImage, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF};
use objc2_foundation::NSData;

pub fn set_image(_width: u32, _height: u32, _rgba: &[u8], png: &[u8]) -> Result<(), String> {
    let data = NSData::with_bytes(png);
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    if !board.setData_forType(Some(&data), unsafe { NSPasteboardTypePNG }) {
        return Err("the pasteboard refused the picture".into());
    }
    // Best effort: the PNG is already there, which is what most apps take.
    if let Some(tiff) =
        NSImage::initWithData(NSImage::alloc(), &data).and_then(|image| image.TIFFRepresentation())
    {
        board.setData_forType(Some(&tiff), unsafe { NSPasteboardTypeTIFF });
    }
    Ok(())
}
