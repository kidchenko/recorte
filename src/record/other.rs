//! macOS and Linux: not written yet. Starting a recording says so.

use super::Area;
use std::path::Path;

pub struct Recorder;

impl Recorder {
    pub fn start(_area: Area, _path: &Path, _mic: bool, _system: bool) -> Result<Self, String> {
        Err("screen recording is not supported on this system yet".into())
    }

    pub fn set_paused(&self, _paused: bool) {}

    pub fn set_mic(&self, _on: bool) {}

    pub fn set_system(&self, _on: bool) {}

    pub fn is_over(&self) -> bool {
        false
    }

    pub fn stop(self) -> Result<(), String> {
        Ok(())
    }
}

/// Cutting a window to a shape: the frame is not drawn here yet.
pub fn clip(_window: &tao::window::Window, _at: (i32, i32), _shape: &[(i32, i32, i32, i32)]) {}
