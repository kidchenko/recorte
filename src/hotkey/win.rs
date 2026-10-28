//! A low-level keyboard hook, which is the only way to have Win+Shift+S:
//! the shell registered it first, so `RegisterHotKey` fails on it.
//!
//! The hook runs on its own thread with its own message loop, ahead of
//! every other program's input. It must answer fast: past
//! `LowLevelHooksTimeout` Windows stops waiting, lets the keys through to
//! Snipping Tool anyway, and eventually drops the hook. So on a match it only
//! posts to its own thread and returns; the rest happens after.
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
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_F1, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_SNAPSHOT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
};

type Fire = Box<dyn Fn() + Send + Sync>;

static FIRE: OnceLock<Fire> = OnceLock::new();
static HOTKEY: Mutex<Option<Hotkey>> = Mutex::new(None);
/// The key whose release is still to be swallowed, or 0. Also stops a held
/// key's auto-repeat from starting a snip per repeat.
static HELD: AtomicU32 = AtomicU32::new(0);
/// The hook thread, which the hook posts to.
static THREAD: AtomicU32 = AtomicU32::new(0);

/// Unassigned (Windows lists 0xE8 as such); see the module comment.
const DUMMY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
/// Posted by the hook to its own thread: "the hotkey was pressed". `wParam`
/// is 1 when a modifier needs masking.
const WM_FIRE: u32 = WM_APP + 1;
/// Put the hook back at the front of the chain this often. Windows removes
/// a low-level hook that times out too often, without saying so, and a hook
/// installed later by another program runs before ours.
const REHOOK_EVERY_MS: u32 = 30_000;

pub fn start(hotkey: Hotkey, fire: Fire) {
    set(hotkey);
    if FIRE.set(fire).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("recorte-hotkey".into())
        .spawn(|| unsafe {
            THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
            let Some(mut hook) = install() else { return };
            linfo!("keyboard hook installed");
            let _ = SetTimer(None, 0, REHOOK_EVERY_MS, None);
            // The hook is called through this thread's message loop, and
            // posts here what it cannot do inside the call.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    WM_FIRE => {
                        if msg.wParam.0 == 1 {
                            mask_modifier();
                        }
                        linfo!("hotkey pressed");
                        if let Some(fire) = FIRE.get() {
                            fire();
                        }
                    }
                    WM_TIMER => {
                        if let Some(fresh) = install() {
                            let _ = UnhookWindowsHookEx(hook);
                            hook = fresh;
                        }
                    }
                    _ => {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            let _ = UnhookWindowsHookEx(hook);
        });
}

unsafe fn install() -> Option<HHOOK> {
    let module = GetModuleHandleW(None).ok();
    match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module.map(Into::into), 0) {
        Ok(h) => Some(h),
        Err(e) => {
            lwarn!("could not install the keyboard hook ({e}); the hotkey is off");
            None
        }
    }
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
/// Never from inside the hook: injected input goes through the same hook
/// chain, so sending it while the hook is still deciding can stall the hook
/// past its timeout, and then Windows lets the original keys through.
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

/// Decide and return: swallow the hotkey and post the rest to the thread.
/// Anything slower here and Windows stops waiting for the answer.
unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // Our own DUMMY, and anything another tool typed for the user, is
        // not someone pressing the hotkey.
        if !info.flags.contains(LLKHF_INJECTED) {
            let msg = wparam.0 as u32;
            let code_vk = info.vkCode;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                if let Some(hk) = matches(code_vk) {
                    // Auto-repeat of a hotkey already handled: swallow, but
                    // do not snip again.
                    if HELD.swap(code_vk, Ordering::Relaxed) != code_vk {
                        let mask = (hk.meta || hk.alt) as usize;
                        let _ = PostThreadMessageW(
                            THREAD.load(Ordering::Relaxed),
                            WM_FIRE,
                            WPARAM(mask),
                            LPARAM(0),
                        );
                    }
                    return LRESULT(1);
                }
                // The key without the modifiers is just typing, even if a
                // release was missed.
                if HELD.load(Ordering::Relaxed) == code_vk {
                    HELD.store(0, Ordering::Relaxed);
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
