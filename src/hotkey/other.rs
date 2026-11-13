//! macOS and Linux: not written yet. The tray menu and a relaunch still
//! start a snip, and the menu a recording.

use super::{Action, Hotkey};
use traykit::lwarn;

pub fn start(snip: Hotkey, record: Hotkey, _fire: Box<dyn Fn(Action) + Send + Sync>) {
    lwarn!(
        "global hotkeys are not supported on this system yet; {} and {} are off",
        snip.label(),
        record.label()
    );
}

pub fn set(_action: Action, _hotkey: Hotkey) {}

pub fn own_windows_first() {}
