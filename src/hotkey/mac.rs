//! Carbon's `RegisterEventHotKey`, which every macOS hotkey utility still
//! uses: the combination is the app's alone, wherever the focus is, and it
//! needs no Accessibility permission, unlike a keyboard event tap.
//!
//! The keys are matched by position, as on Windows: `S` is the key where S
//! is on a US keyboard. Everything here runs on the main thread, where the
//! event loop runs and the handler is called.

use super::{Action, Hotkey, Key};
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use traykit::{linfo, lwarn};

type Ref = *mut c_void;
type Fire = Box<dyn Fn(Action) + Send + Sync>;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

type Handler = extern "C" fn(call: Ref, event: Ref, user: *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> Ref;
    fn InstallEventHandler(
        target: Ref,
        handler: Handler,
        count: u32,
        types: *const EventTypeSpec,
        user: *mut c_void,
        out: *mut Ref,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: Ref,
        options: u32,
        out: *mut Ref,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: Ref) -> i32;
    fn GetEventParameter(
        event: Ref,
        name: u32,
        desired: u32,
        actual: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
}

const fn four(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}

const K_EVENT_CLASS_KEYBOARD: u32 = four(b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = four(b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = four(b"hkid");
const EXISTS: i32 = -9878; // eventHotKeyExistsErr
const SIGNATURE: u32 = four(b"Rcrt");

const CMD: u32 = 1 << 8;
const SHIFT: u32 = 1 << 9;
const OPTION: u32 = 1 << 11;
const CONTROL: u32 = 1 << 12;

/// In the order a key is matched against them, so a combination given to
/// both snips. The index plus one is the Carbon hotkey id.
const ACTIONS: [Action; 2] = [Action::Snip, Action::Record];

static FIRE: OnceLock<Fire> = OnceLock::new();

/// What is asked for, and what Carbon handed back for it.
struct Slot {
    hotkey: Option<Hotkey>,
    registered: usize, // EventHotKeyRef, as an address so the static is Send
}

static SLOTS: Mutex<[Slot; 2]> = Mutex::new([
    Slot {
        hotkey: None,
        registered: 0,
    },
    Slot {
        hotkey: None,
        registered: 0,
    },
]);

pub fn start(snip: Hotkey, record: Hotkey, fire: Fire) {
    if FIRE.set(fire).is_err() {
        return;
    }
    let spec = EventTypeSpec {
        class: K_EVENT_CLASS_KEYBOARD,
        kind: K_EVENT_HOT_KEY_PRESSED,
    };
    let err = unsafe {
        InstallEventHandler(
            GetApplicationEventTarget(),
            on_hotkey,
            1,
            &spec,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if err != 0 {
        lwarn!("could not listen for hotkeys (OSStatus {err}); they are off");
        return;
    }
    let Ok(mut slots) = SLOTS.lock() else { return };
    slots[0].hotkey = Some(snip);
    slots[1].hotkey = Some(record);
    apply(&mut slots);
}

pub fn set(action: Action, hotkey: Hotkey) {
    let Ok(mut slots) = SLOTS.lock() else { return };
    let index = ACTIONS.iter().position(|a| *a == action).unwrap_or(0);
    slots[index].hotkey = Some(hotkey);
    // Both are registered again: the record hotkey may have been left out
    // for having the snip's combination, which may have just changed.
    apply(&mut slots);
}

/// Register what `slots` asks for, in place of what was registered.
fn apply(slots: &mut [Slot; 2]) {
    for slot in slots.iter_mut() {
        if slot.registered != 0 {
            unsafe { UnregisterEventHotKey(slot.registered as Ref) };
            slot.registered = 0;
        }
    }
    let snip = slots[0].hotkey;
    for (i, slot) in slots.iter_mut().enumerate() {
        let Some(hk) = slot.hotkey else { continue };
        if i > 0 && Some(hk) == snip {
            lwarn!("{} is the snip hotkey too; it snips", hk.label());
            continue;
        }
        slot.registered = register(hk, i as u32 + 1);
    }
}

pub fn own_windows_first() {}

fn register(hk: Hotkey, id: u32) -> usize {
    let mut modifiers = 0;
    if hk.meta {
        modifiers |= CMD;
    }
    if hk.shift {
        modifiers |= SHIFT;
    }
    if hk.alt {
        modifiers |= OPTION;
    }
    if hk.ctrl {
        modifiers |= CONTROL;
    }
    let mut out: Ref = std::ptr::null_mut();
    let err = unsafe {
        RegisterEventHotKey(
            key_code(hk.key),
            modifiers,
            EventHotKeyID {
                signature: SIGNATURE,
                id,
            },
            GetApplicationEventTarget(),
            0,
            &mut out,
        )
    };
    match err {
        0 => {
            linfo!("listening for {}", hk.label());
            out as usize
        }
        EXISTS => {
            lwarn!("{} is taken by another app; it is off", hk.label());
            0
        }
        _ => {
            lwarn!("could not register {} (OSStatus {err})", hk.label());
            0
        }
    }
}

extern "C" fn on_hotkey(_call: Ref, event: Ref, _user: *mut c_void) -> i32 {
    let mut id = EventHotKeyID::default();
    let err = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            std::ptr::null_mut(),
            (&mut id as *mut EventHotKeyID).cast(),
        )
    };
    if err != 0 || id.signature != SIGNATURE {
        return -9874; // eventNotHandledErr: someone else's
    }
    if let (Some(action), Some(fire)) = (ACTIONS.get(id.id as usize - 1), FIRE.get()) {
        fire(*action);
    }
    0
}

/// The macOS virtual key code: the key's position on an ANSI keyboard.
fn key_code(key: Key) -> u32 {
    const LETTERS: [u32; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    const DIGITS: [u32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    const FS: [u32; 20] = [
        122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90,
    ];
    match key {
        Key::Char(c @ b'A'..=b'Z') => LETTERS[(c - b'A') as usize],
        Key::Char(c @ b'0'..=b'9') => DIGITS[(c - b'0') as usize],
        Key::Char(_) => u32::MAX,
        // Macs stop at F20; past that it cannot be pressed.
        Key::F(n @ 1..=20) => FS[n as usize - 1],
        Key::F(_) => u32::MAX,
        // A PC keyboard's Print Screen reaches a Mac as F13.
        Key::PrintScreen => 105,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s_and_r_are_where_they_are_on_a_mac() {
        assert_eq!(key_code(Key::Char(b'S')), 1);
        assert_eq!(key_code(Key::Char(b'R')), 15);
        assert_eq!(key_code(Key::Char(b'Z')), 6);
        assert_eq!(key_code(Key::Char(b'0')), 29);
        assert_eq!(key_code(Key::F(1)), 122);
    }
}
