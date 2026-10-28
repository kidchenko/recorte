//! A window that is a plain pixel buffer.
//!
//! softbuffer hands over a `u32` per pixel (`0x00RRGGBB`) and copies it to
//! the window with whatever the system has for that (GDI, CoreGraphics,
//! X11 or Wayland shared memory). No GPU, no shaders, nothing to fail to
//! initialise on a remote desktop.

use std::num::NonZeroU32;
use std::rc::Rc;
use tao::window::Window;

pub struct View {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    size: (u32, u32),
}

impl View {
    pub fn new(window: Window) -> Result<Self, String> {
        let window = Rc::new(window);
        let context =
            softbuffer::Context::new(window.clone()).map_err(|e| format!("softbuffer: {e}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|e| format!("softbuffer surface: {e}"))?;
        Ok(Self {
            window,
            surface,
            size: (0, 0),
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// The window's size now, in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// Have `paint` fill every pixel of a `width * height` buffer, row by
    /// row, and put it on screen.
    pub fn present(&mut self, paint: impl FnOnce(&mut [u32], u32, u32)) {
        let (w, h) = self.size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };
        if self.size != (w, h) {
            if self.surface.resize(nw, nh).is_err() {
                return;
            }
            self.size = (w, h);
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return;
        };
        paint(&mut buffer, w, h);
        let _ = buffer.present();
    }
}
