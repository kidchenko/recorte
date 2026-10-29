//! The settings, in `config.json` in the app's folder (see `traykit::dirs`).
//!
//! There is no settings window: the tray menu carries the switches and
//! "Edit settings" opens this file. A hand edit is picked up within two
//! seconds, hotkey included.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// `Win+Shift+S`. Modifiers are Ctrl, Alt, Shift and Win (Cmd and Super
    /// name the same key); the key is a letter, a digit, F1 to F24 or
    /// PrintScreen.
    pub hotkey: String,
    /// Put the snip on the clipboard the moment it is taken, before any
    /// markup, as Snipping Tool does.
    pub copy_on_snip: bool,
    /// Open the editor with the snip. Off, a snip only goes to the
    /// clipboard (and the folder, with `autoSave`).
    pub open_editor: bool,
    /// Also save every snip, as taken, to `saveFolder`.
    pub auto_save: bool,
    /// Where snips are saved. Empty means `Pictures/Screenshots`.
    pub save_folder: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey().into(),
            copy_on_snip: true,
            open_editor: true,
            auto_save: true,
            save_folder: String::new(),
        }
    }
}

/// Windows: the key Snipping Tool taught everyone. macOS keeps Cmd+Shift+3
/// to 5 for its own screenshots, and on Linux Super+Shift+S is free on the
/// common desktops.
fn default_hotkey() -> &'static str {
    if cfg!(windows) {
        "Win+Shift+S"
    } else if cfg!(target_os = "macos") {
        "Ctrl+Shift+S"
    } else {
        "Super+Shift+S"
    }
}

impl Config {
    pub fn load() -> Self {
        traykit::config::load()
    }

    pub fn reload() -> Option<Self> {
        traykit::config::reload()
    }

    pub fn save_dir(&self) -> Option<PathBuf> {
        let custom = self.save_folder.trim();
        if !custom.is_empty() {
            return Some(PathBuf::from(custom));
        }
        crate::snip::pictures_dir().map(|p| p.join("Screenshots"))
    }
}
