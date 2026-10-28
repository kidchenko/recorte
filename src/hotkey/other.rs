//! macOS and Linux: not written yet. The tray menu and a relaunch still
//! start a snip.

use super::Hotkey;
use traykit::lwarn;

pub fn start(hotkey: Hotkey, _fire: Box<dyn Fn() + Send + Sync>) {
    lwarn!(
        "global hotkeys are not supported on this system yet; {} is off",
        hotkey.label()
    );
}

pub fn set(_hotkey: Hotkey) {}

pub fn own_windows_first() {}
