//! A low-level keyboard hook, which is the only way to have Win+Shift+S:
//! the shell registered it first, so `RegisterHotKey` fails on it.
//!
//! The hook runs on its own thread with its own message loop, ahead of
//! every other program's input. It must answer fast (Windows drops a hook
//! that keeps it waiting, `LowLevelHooksTimeout`), so on a match it only
//! posts to the event loop and returns.
//!
//! Swallowing the S is not quite enough. The shell watches Win: pressed and
//! released with nothing in between opens the Start menu, and with the S
//! swallowed, nothing came in between. So a key nobody uses (0xE8, which
//! Windows leaves unassigned) is pressed and released in its place, the
//! same trick PowerToys uses.

use super::{Hotkey, Key};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use traykit::{linfo, lwarn};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_F1, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_SNAPSHOT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage, HC_ACTION,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

type Fire = Box<dyn Fn() + Send + Sync>;

static FIRE: OnceLock<Fire> = OnceLock::new();
static HOTKEY: Mutex<Option<Hotkey>> = Mutex::new(None);
/// The key whose release is still to be swallowed, or 0. Also stops a held
/// key's auto-repeat from starting a snip per repeat.
static HELD: AtomicU32 = AtomicU32::new(0);

/// Unassigned (Windows lists 0xE8 as such); see the module comment.
const DUMMY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

pub fn start(hotkey: Hotkey, fire: Fire) {
    set(hotkey);
    if FIRE.set(fire).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("print-hotkey".into())
        .spawn(|| unsafe {
            let module = GetModuleHandleW(None).ok();
            let hook =
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module.map(Into::into), 0) {
                    Ok(h) => h,
                    Err(e) => {
                        lwarn!("could not install the keyboard hook ({e}); the hotkey is off");
                        return;
                    }
                };
            linfo!("keyboard hook installed");
            // A low-level hook is called through this thread's message loop.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
        });
}

pub fn set(hotkey: Hotkey) {
    if let Ok(mut h) = HOTKEY.lock() {
        *h = Some(hotkey);
    }
}

fn vk(key: Key) -> u32 {
    match key {
        // Letters and digits are their own ASCII uppercase code.
        Key::Char(c) => c as u32,
        Key::F(n) => VK_F1.0 as u32 + n as u32 - 1,
        Key::PrintScreen => VK_SNAPSHOT.0 as u32,
    }
}

fn down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetAsyncKeyState(key.0 as i32) } as u16 & 0x8000) != 0
}

/// Is `vk` going down with exactly the hotkey's modifiers held?
fn matches(code: u32) -> Option<Hotkey> {
    let hk = (*HOTKEY.lock().ok()?)?;
    if code != vk(hk.key) {
        return None;
    }
    let meta = down(VK_LWIN) || down(VK_RWIN);
    let exact = meta == hk.meta
        && down(VK_SHIFT) == hk.shift
        && down(VK_CONTROL) == hk.ctrl
        && down(VK_MENU) == hk.alt;
    exact.then_some(hk)
}

/// Press and release [`DUMMY`], so a lone Win or Alt release does nothing.
fn mask_modifier() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: DUMMY,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // Our own DUMMY, and anything another tool typed for the user, is
        // not someone pressing the hotkey.
        if !info.flags.contains(LLKHF_INJECTED) {
            let msg = wparam.0 as u32;
            let code_vk = info.vkCode;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                if HELD.load(Ordering::Relaxed) == code_vk {
                    // Auto-repeat of a hotkey already handled.
                    return LRESULT(1);
                }
                if let Some(hk) = matches(code_vk) {
                    HELD.store(code_vk, Ordering::Relaxed);
                    if hk.meta || hk.alt {
                        mask_modifier();
                    }
                    if let Some(fire) = FIRE.get() {
                        fire();
                    }
                    return LRESULT(1);
                }
            } else if (msg == WM_KEYUP || msg == WM_SYSKEYUP)
                && HELD.load(Ordering::Relaxed) == code_vk
            {
                HELD.store(0, Ordering::Relaxed);
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
