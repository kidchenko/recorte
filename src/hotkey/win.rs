//! A low-level keyboard hook, which is the only way to have Win+Shift+S and
//! Win+Shift+R: the shell registered them first, so `RegisterHotKey` fails
//! on them.
//!
//! The hook runs on its own thread with its own message loop, ahead of
//! every other program's input. It must answer fast: past
//! `LowLevelHooksTimeout` Windows stops waiting, lets the keys through to
//! Snipping Tool anyway, and eventually drops the hook. So on a match it only
//! posts to its own thread and returns; the rest happens after.
//!
//! Swallowing the letter is not quite enough. The shell watches Win: pressed
//! and released with nothing in between opens the Start menu, and with the
//! letter swallowed, nothing came in between. So a key nobody uses (0xE8, which
//! Windows leaves unassigned) is pressed and released in its place, the
//! same trick PowerToys uses.

use super::{Action, Hotkey, Key};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

type Fire = Box<dyn Fn(Action) + Send + Sync>;

/// In the order a key is matched against them, so a combination given to
/// both snips.
const ACTIONS: [Action; 2] = [Action::Snip, Action::Record];

static FIRE: OnceLock<Fire> = OnceLock::new();
/// The combination for each of [`ACTIONS`].
static HOTKEYS: Mutex<[Option<Hotkey>; 2]> = Mutex::new([None; 2]);
/// The key whose release is still to be swallowed, or 0. Also stops a held
/// key's auto-repeat from starting a snip per repeat.
static HELD: AtomicU32 = AtomicU32::new(0);
/// The hook thread, which the hook posts to.
static THREAD: AtomicU32 = AtomicU32::new(0);
/// `RECORTE_TEST_KEYS=1`: treat typed-by-software keys as real, so the hotkey
/// can be driven by SendInput in a test. Off, a tool that types for the user
/// cannot start a snip.
static TAKE_INJECTED: AtomicBool = AtomicBool::new(false);

/// Unassigned (Windows lists 0xE8 as such); see the module comment.
const DUMMY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
/// Posted by the hook to its own thread: "a hotkey was pressed". `wParam` is
/// 1 when a modifier needs masking, `lParam` the index in [`ACTIONS`].
const WM_FIRE: u32 = WM_APP + 1;
/// Put the hook back at the front of the chain this often. Windows removes
/// a low-level hook that times out too often, without saying so, and a hook
/// installed later by another program runs before ours.
const REHOOK_EVERY_MS: u32 = 30_000;

pub fn start(snip: Hotkey, record: Hotkey, fire: Fire) {
    TAKE_INJECTED.store(
        std::env::var("RECORTE_TEST_KEYS").is_ok_and(|v| v == "1"),
        Ordering::Relaxed,
    );
    set(Action::Snip, snip);
    set(Action::Record, record);
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
                        let action = ACTIONS[(msg.lParam.0 as usize).min(ACTIONS.len() - 1)];
                        linfo!("hotkey pressed ({action:?})");
                        if let Some(fire) = FIRE.get() {
                            fire(action);
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

pub fn set(action: Action, hotkey: Hotkey) {
    let index = ACTIONS.iter().position(|a| *a == action).unwrap_or(0);
    if let Ok(mut h) = HOTKEYS.lock() {
        h[index] = Some(hotkey);
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

/// Is `code` going down with exactly the modifiers of one of the hotkeys
/// held? Which one, as an index in [`ACTIONS`].
fn matches(code: u32) -> Option<(usize, Hotkey)> {
    let hotkeys = *HOTKEYS.lock().ok()?;
    // This runs for every key typed anywhere, so the modifiers are only
    // asked about for a key that is one of the hotkeys'.
    if !hotkeys.iter().flatten().any(|hk| code == vk(hk.key)) {
        return None;
    }
    let meta = down(VK_LWIN) || down(VK_RWIN);
    let (shift, ctrl, alt) = (down(VK_SHIFT), down(VK_CONTROL), down(VK_MENU));
    hotkeys.iter().enumerate().find_map(|(i, hk)| {
        let hk = (*hk)?;
        let exact = code == vk(hk.key)
            && meta == hk.meta
            && shift == hk.shift
            && ctrl == hk.ctrl
            && alt == hk.alt;
        exact.then_some((i, hk))
    })
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
        let ours = info.vkCode == DUMMY.0 as u32;
        let typed = !info.flags.contains(LLKHF_INJECTED) || TAKE_INJECTED.load(Ordering::Relaxed);
        if typed && !ours {
            let msg = wparam.0 as u32;
            let code_vk = info.vkCode;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                if let Some((action, hk)) = matches(code_vk) {
                    // Auto-repeat of a hotkey already handled: swallow, but
                    // do not fire again.
                    if HELD.swap(code_vk, Ordering::Relaxed) != code_vk {
                        let mask = (hk.meta || hk.alt) as usize;
                        let _ = PostThreadMessageW(
                            THREAD.load(Ordering::Relaxed),
                            WM_FIRE,
                            WPARAM(mask),
                            LPARAM(action as isize),
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

/// Register for raw keyboard input with `RIDEV_NOHOTKEYS`: while one of our
/// windows has the focus, Windows does not act on hotkeys other programs
/// registered, Win+Shift+S and Win+Shift+R among them, and the keys reach
/// the window.
///
/// The hook alone is not enough there. With our own editor in front,
/// Windows has been seen to pass Win+Shift+S to Snipping Tool without
/// calling the hook at all; this makes the editor see the keys instead.
/// tao registers the keyboard once, when the event loop starts, for device
/// events this app does not use, so this replaces that registration.
pub fn own_windows_first() {
    use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_NOHOTKEYS};
    let keyboard = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop
        usUsage: 0x06,     // keyboard
        dwFlags: RIDEV_NOHOTKEYS,
        // No target window: the input follows the keyboard focus.
        hwndTarget: Default::default(),
    };
    let ok = unsafe {
        RegisterRawInputDevices(&[keyboard], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
    };
    if let Err(e) = ok {
        lwarn!("could not keep hotkeys away from our windows ({e})");
    }
}
