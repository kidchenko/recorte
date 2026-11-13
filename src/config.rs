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
    /// `Win+Shift+R`, written like `hotkey`: choose an area to record, then
    /// start, then stop.
    pub record_hotkey: String,
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
    /// Where recordings are saved. Empty means `Videos/Screen Recordings`,
    /// where Snipping Tool puts its own.
    pub record_folder: String,
    /// Open a recording in the system's player when it is done.
    pub open_recording: bool,
    /// Whether a recording starts with the microphone in it, and with what
    /// the computer plays. Both can be changed on the recording's bar.
    pub record_mic: bool,
    pub record_system_audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey('S'),
            record_hotkey: default_hotkey('R'),
            copy_on_snip: true,
            open_editor: true,
            auto_save: true,
            save_folder: String::new(),
            record_folder: String::new(),
            open_recording: true,
            record_mic: false,
            record_system_audio: true,
        }
    }
}

/// Windows: the keys Snipping Tool taught everyone, S to snip and R to
/// record. macOS keeps Cmd+Shift+3 to 5 for its own screenshots, and on
/// Linux Super+Shift+S and R are free on the common desktops.
fn default_hotkey(key: char) -> String {
    let modifiers = if cfg!(windows) {
        "Win+Shift"
    } else if cfg!(target_os = "macos") {
        "Ctrl+Shift"
    } else {
        "Super+Shift"
    };
    format!("{modifiers}+{key}")
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

    pub fn record_dir(&self) -> Option<PathBuf> {
        let custom = self.record_folder.trim();
        if !custom.is_empty() {
            return Some(PathBuf::from(custom));
        }
        crate::snip::videos_dir().map(|p| p.join("Screen Recordings"))
    }
}
