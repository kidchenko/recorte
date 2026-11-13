//! The tray application: the icon and its menu, and a snip's way through
//! the app: hotkey, capture, overlay, clipboard, editor. A recording goes
//! the same way as far as the overlay, then to the frame and bar in `record`.
//!
//! Everything here runs on the main thread, driven by the event loop in
//! `main`. The hotkey and the single-instance listener run on their own
//! threads and only post a [`UserEvent`].
//!
//! The loop sleeps between events. Idle, it wakes every two seconds to
//! notice a hand-edited config; there is nothing else to poll. While a
//! recording runs it also wakes each second, for the clock on the bar.

use crate::capture;
use crate::config::Config;
use crate::editor::{self, Editor};
use crate::hotkey::{self, Action, Hotkey};
use crate::icon;
use crate::overlay::{self, Outcome, Overlay, Region};
use crate::record::{self, Recording};
use crate::snip::{self, Snip};

use std::time::{Duration, Instant, SystemTime};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopProxy, EventLoopWindowTarget};
use tao::keyboard::ModifiersState;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon as TrayImage, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use traykit::{ldebug, lerror, linfo, lwarn};

const ID_SNIP: &str = "snip";
const ID_SNIP_LATER: &str = "snip-later";
const ID_RECORD: &str = "record";
const ID_FOLDER: &str = "folder";
const ID_VIDEOS: &str = "videos";
const ID_AUTOSTART: &str = "autostart";
const ID_CONFIG: &str = "config";
const ID_LOG: &str = "log";
const ID_QUIT: &str = "quit";

const IDLE_EVERY: Duration = Duration::from_secs(2);
/// A click on the tray or the menu: let the menu fade out before the screen
/// is captured, or it is in the picture.
const AFTER_CLICK: Duration = Duration::from_millis(250);
const LATER: Duration = Duration::from_secs(5);

pub enum UserEvent {
    /// The hotkey, or a second launch.
    Snip,
    /// The recording hotkey: choose an area, then start, then stop.
    Record,
    Menu(String),
    TrayClick,
}

pub struct App {
    tray: TrayIcon,
    snip_item: MenuItem,
    record_item: MenuItem,
    autostart_item: CheckMenuItem,
    cfg: Config,
    cfg_seen: Option<SystemTime>,
    overlays: Vec<Overlay>,
    /// What the overlays are up for: a snip, or the area of a recording.
    picking: Action,
    editors: Vec<Editor>,
    /// The recording, from its area being chosen until it is saved.
    recording: Option<Recording>,
    /// A snip or a recording asked for, waiting for a menu to fade or a
    /// countdown.
    pending: Option<(Instant, Action)>,
    next_idle: Instant,
    dark_taskbar: bool,
    /// The hotkeys, to recognise them in our own windows (see `handle`).
    hotkey: Hotkey,
    record_hotkey: Hotkey,
    /// Modifiers held, as our windows last reported them.
    mods: ModifiersState,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        let cfg = Config::load();

        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = p.send_event(UserEvent::Menu(e.id.0));
        }));
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = p.send_event(UserEvent::TrayClick);
            }
        }));

        let hk = parse_hotkey(&cfg.hotkey, &Config::default().hotkey);
        let record_hk = parse_hotkey(&cfg.record_hotkey, &Config::default().record_hotkey);
        let p = proxy.clone();
        hotkey::start(hk, record_hk, move |action| {
            let _ = p.send_event(match action {
                Action::Snip => UserEvent::Snip,
                Action::Record => UserEvent::Record,
            });
        });
        hotkey::own_windows_first();

        let snip_item = MenuItem::with_id(ID_SNIP, snip_label(&hk), true, None);
        let record_item = MenuItem::with_id(ID_RECORD, record_label(&record_hk, None), true, None);
        let autostart_item = CheckMenuItem::with_id(
            ID_AUTOSTART,
            "Start at login",
            true,
            traykit::autostart::is_enabled(),
            None,
        );
        let menu = Menu::new();
        menu.append_items(&[
            &snip_item,
            &MenuItem::with_id(ID_SNIP_LATER, "Snip in 5 seconds", true, None),
            &record_item,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_FOLDER, "Open screenshots folder", true, None),
            &MenuItem::with_id(ID_VIDEOS, "Open recordings folder", true, None),
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &MenuItem::with_id(ID_CONFIG, "Edit settings", true, None),
            &MenuItem::with_id(ID_LOG, "Open log", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_QUIT, "Quit Recorte", true, None),
        ])
        .map_err(|e| format!("could not build the menu: {e}"))?;

        let dark_taskbar = traykit::theme::tray_is_dark();
        let tray = TrayIconBuilder::new()
            .with_tooltip(format!("Recorte ({})", hk.label()))
            .with_icon(tray_image(dark_taskbar)?)
            .with_menu(Box::new(menu))
            // Left click snips; the menu is the right button's.
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| format!("could not create the tray icon: {e}"))?;

        Ok(Self {
            tray,
            snip_item,
            record_item,
            autostart_item,
            cfg_seen: traykit::config::modified(),
            cfg,
            overlays: Vec::new(),
            picking: Action::Snip,
            editors: Vec::new(),
            recording: None,
            pending: None,
            next_idle: Instant::now() + IDLE_EVERY,
            dark_taskbar,
            hotkey: hk,
            record_hotkey: record_hk,
            mods: ModifiersState::empty(),
        })
    }

    pub fn handle(
        &mut self,
        event: Event<UserEvent>,
        target: &EventLoopWindowTarget<UserEvent>,
    ) -> ControlFlow {
        match event {
            Event::UserEvent(UserEvent::Snip) => self.act(Action::Snip, target),
            Event::UserEvent(UserEvent::Record) => self.act(Action::Record, target),
            Event::UserEvent(UserEvent::TrayClick) => self.act_in(AFTER_CLICK, Action::Snip),
            Event::UserEvent(UserEvent::Menu(id)) => match id.as_str() {
                ID_SNIP => self.act_in(AFTER_CLICK, Action::Snip),
                ID_SNIP_LATER => self.act_in(LATER, Action::Snip),
                ID_RECORD => self.act_in(AFTER_CLICK, Action::Record),
                ID_FOLDER => open_folder(self.cfg.save_dir()),
                ID_VIDEOS => open_folder(self.cfg.record_dir()),
                ID_AUTOSTART => self.toggle_autostart(),
                ID_CONFIG => traykit::config::reveal::<Config>(),
                ID_LOG => traykit::log::reveal(),
                ID_QUIT => {
                    linfo!("quit from the menu");
                    // Dropped, a recording under way is closed and kept. It
                    // has to be done here: the process ends without
                    // dropping anything.
                    self.recording = None;
                    return ControlFlow::Exit;
                }
                _ => {}
            },
            Event::WindowEvent {
                window_id, event, ..
            } => {
                if let WindowEvent::ModifiersChanged(m) = &event {
                    self.mods = *m;
                }
                // A hotkey in one of our own windows: Windows hands it to
                // the window there rather than to the hook (see
                // `hotkey::own_windows_first`), so it is caught here, before
                // the editor reads Win+Shift+S as a shortcut of its own.
                if let WindowEvent::KeyboardInput { event: key, .. } = &event {
                    let pressed = if self.hotkey.pressed_in(key, self.mods) {
                        Some(Action::Snip)
                    } else if self.record_hotkey.pressed_in(key, self.mods) {
                        Some(Action::Record)
                    } else {
                        None
                    };
                    if let Some(action) = pressed {
                        linfo!("hotkey pressed in our own window ({action:?})");
                        self.act(action, target);
                        return ControlFlow::WaitUntil(Instant::now());
                    }
                }
                if let Some(i) = self.overlays.iter().position(|o| o.id() == window_id) {
                    match self.overlays[i].handle(&event) {
                        Outcome::Nothing => {}
                        Outcome::Cancel(why) => {
                            linfo!("selection cancelled ({why})");
                            self.overlays.clear();
                        }
                        Outcome::Mode(action) => self.choose(action),
                        Outcome::Done(region) => match self.picking {
                            Action::Snip => {
                                let snip = self.overlays[i].cut(&region);
                                self.overlays.clear();
                                self.taken(snip, target);
                            }
                            Action::Record => {
                                self.overlays.clear();
                                self.frame(region, target);
                            }
                        },
                    }
                } else if let Some(i) = self.editors.iter().position(|e| e.id() == window_id) {
                    match self.editors[i].handle(&event) {
                        editor::Request::Nothing => {}
                        editor::Request::Close => {
                            self.editors.remove(i);
                        }
                        editor::Request::NewSnip => self.act(Action::Snip, target),
                    }
                } else if let Some(r) = self.recording.as_mut().filter(|r| r.owns(window_id)) {
                    let request = r.handle(window_id, &event);
                    self.recorded(request);
                }
            }
            // The loop is ending some other way than the menu's Quit.
            Event::LoopDestroyed => self.recording = None,
            Event::RedrawRequested(id) => {
                if let Some(o) = self.overlays.iter_mut().find(|o| o.id() == id) {
                    o.redraw();
                } else if let Some(e) = self.editors.iter_mut().find(|e| e.id() == id) {
                    e.redraw();
                } else if let Some(r) = self.recording.as_mut().filter(|r| r.owns(id)) {
                    r.redraw(id);
                }
            }
            _ => {}
        }

        let now = Instant::now();
        if let Some((_, action)) = self.pending.filter(|(at, _)| now >= *at) {
            self.pending = None;
            self.act(action, target);
        }
        for e in &mut self.editors {
            e.tick(now);
        }
        if let Some(r) = &mut self.recording {
            let request = r.tick(now);
            self.recorded(request);
        }
        if now >= self.next_idle {
            self.next_idle = now + IDLE_EVERY;
            self.idle();
        }

        let wake = [
            Some(self.next_idle),
            self.pending.map(|(at, _)| at),
            self.recording.as_ref().and_then(Recording::deadline),
        ]
        .into_iter()
        .chain(self.editors.iter().map(Editor::deadline))
        .flatten()
        .min()
        .unwrap_or(self.next_idle);
        ControlFlow::WaitUntil(wake)
    }

    fn act_in(&mut self, delay: Duration, action: Action) {
        self.pending = Some((Instant::now() + delay, action));
    }

    /// A hotkey, or the menu standing in for one.
    fn act(&mut self, action: Action, target: &EventLoopWindowTarget<UserEvent>) {
        // With a recording's frame up, its hotkey is the record button:
        // start, then stop. Not while selecting, though: there a hotkey is
        // a button of the overlay's menu.
        if self.overlays.is_empty() {
            if let (Action::Record, Some(r)) = (action, &mut self.recording) {
                let request = r.toggle();
                self.recorded(request);
                return;
            }
        }
        self.pick(action, target);
    }

    /// Freeze the screen and put the overlay over every monitor, to choose
    /// what to snip or the area to record.
    fn pick(&mut self, action: Action, target: &EventLoopWindowTarget<UserEvent>) {
        if !self.overlays.is_empty() {
            // Pressing a hotkey again while selecting brings the overlay
            // back to the front, in case something took the focus, and the
            // other hotkey is the other button of the overlay's menu.
            self.choose(action);
            for o in &self.overlays {
                o.focus();
            }
            return;
        }
        self.picking = action;
        let started = Instant::now();
        let frame = match capture::screen() {
            Ok(f) => f,
            Err(e) => {
                lerror!("capture failed: {e}");
                return;
            }
        };
        let under_pointer = overlay::monitor_at(target);
        let filming = self.recording.as_ref().is_some_and(Recording::is_filming);
        for m in target.available_monitors() {
            ldebug!(
                "monitor {:?} at {:?} size {:?} scale {}",
                m.name(),
                m.position(),
                m.size(),
                m.scale_factor()
            );
        }
        #[cfg(windows)]
        unsafe {
            use windows::Win32::UI::HiDpi::*;
            ldebug!(
                "dpi awareness {:?}, system dpi {}",
                GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()),
                GetDpiForSystem()
            );
        }
        for monitor in target.available_monitors() {
            let focus = under_pointer
                .as_ref()
                .is_some_and(|m| m.position() == monitor.position());
            match Overlay::new(target, monitor, &frame, action, filming) {
                Ok(o) => {
                    if focus {
                        o.focus();
                    }
                    self.overlays.push(o);
                }
                Err(e) => lwarn!("{e}"),
            }
        }
        linfo!(
            "overlay up on {} monitor(s) in {} ms ({}x{} captured)",
            self.overlays.len(),
            started.elapsed().as_millis(),
            frame.width,
            frame.height
        );
    }

    /// The overlay's menu, or a hotkey while the overlay is up: what is
    /// selected next is for `action`.
    fn choose(&mut self, action: Action) {
        if action != self.picking {
            linfo!("selecting for {action:?}");
            self.picking = action;
            for o in &mut self.overlays {
                o.set_mode(action);
            }
        }
    }

    fn taken(&mut self, snip: Snip, target: &EventLoopWindowTarget<UserEvent>) {
        linfo!("snip taken: {}x{}", snip.width, snip.height);
        if self.cfg.copy_on_snip {
            if let Err(e) = snip.copy() {
                lwarn!("copy failed: {e}");
            }
        }
        if self.cfg.auto_save {
            match self.cfg.save_dir() {
                Some(dir) => {
                    let name = snip::file_name(traykit::time::LocalTime::now(), "png");
                    let path = snip::fresh_path(&dir, &name);
                    match snip.save(&path) {
                        Ok(()) => linfo!("saved {}", path.display()),
                        Err(e) => lwarn!("auto-save failed: {e}"),
                    }
                }
                None => lwarn!("auto-save is on but there is no folder to save to"),
            }
        }
        if self.cfg.open_editor {
            let monitor = overlay::monitor_at(target);
            match Editor::new(target, snip, monitor, self.cfg.save_dir()) {
                Ok(e) => self.editors.push(e),
                Err(e) => lerror!("{e}"),
            }
        }
    }

    /// The area is chosen: put the frame and the bar around it, ready to
    /// start.
    fn frame(&mut self, region: Region, target: &EventLoopWindowTarget<UserEvent>) {
        let Some(dir) = self.cfg.record_dir() else {
            alert("There is no folder to save a recording to.");
            return;
        };
        // One recording at a time. One that is filming is stopped and kept,
        // as its stop button would; one that only waited is put away.
        let filming = self.recording.as_mut().filter(|r| r.is_filming());
        if let Some(request) = filming.map(Recording::toggle) {
            self.recorded(request);
        }
        self.put_away();
        let (mic, system) = (self.cfg.record_mic, self.cfg.record_system_audio);
        match Recording::new(target, region, dir, mic, system) {
            Ok(r) => {
                linfo!("ready to record {}x{}", region.width, region.height);
                self.recording = Some(r);
                self.label_record_item();
            }
            Err(e) => alert(&format!("Recorte could not record this.\n\n{e}")),
        }
    }

    /// What a press on the recording's bar, or its hotkey, came to.
    fn recorded(&mut self, request: record::Request) {
        match request {
            // Most of what the bar is told changes nothing here.
            record::Request::Nothing => return,
            record::Request::Started => {}
            record::Request::Cancelled => {
                linfo!("recording cancelled");
                self.put_away();
            }
            record::Request::Saved(path) => {
                linfo!("recording saved to {}", path.display());
                self.put_away();
                if self.cfg.open_recording {
                    traykit::util::shell_open(&path.to_string_lossy());
                }
            }
            record::Request::Failed(e) => {
                self.put_away();
                // The frame and the bar are gone: without this it would
                // look like a recording that was saved.
                alert(&format!("Recorte could not record.\n\n{e}"));
            }
        }
        // It has started or ended: the menu says what comes next.
        self.label_record_item();
    }

    /// Close the recording's frame and bar, and keep how its sound was
    /// switched: the next recording starts the same way.
    fn put_away(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let (mic, system) = recording.sound();
        if (mic, system) != (self.cfg.record_mic, self.cfg.record_system_audio) {
            self.cfg.record_mic = mic;
            self.cfg.record_system_audio = system;
            match traykit::config::save(&self.cfg) {
                // Our own change to the file is not one to read back.
                Ok(()) => self.cfg_seen = traykit::config::modified(),
                Err(e) => lwarn!("could not save the settings: {e}"),
            }
        }
    }

    fn label_record_item(&self) {
        self.record_item
            .set_text(record_label(&self.record_hotkey, self.recording.as_ref()));
    }

    fn toggle_autostart(&mut self) {
        let want = !traykit::autostart::is_enabled();
        if let Err(e) = traykit::autostart::set(want) {
            lerror!("{e}");
        }
        self.autostart_item
            .set_checked(traykit::autostart::is_enabled());
    }

    /// A hand-edited config, and the taskbar changing between light and dark.
    fn idle(&mut self) {
        let seen = traykit::config::modified();
        if seen != self.cfg_seen {
            self.cfg_seen = seen;
            if let Some(cfg) = Config::reload() {
                if cfg.record_hotkey != self.cfg.record_hotkey {
                    let hk = parse_hotkey(&cfg.record_hotkey, &Config::default().record_hotkey);
                    hotkey::set(Action::Record, hk);
                    self.record_hotkey = hk;
                    self.label_record_item();
                    linfo!("recording hotkey is now {}", hk.label());
                }
                if cfg.hotkey != self.cfg.hotkey {
                    let hk = parse_hotkey(&cfg.hotkey, &Config::default().hotkey);
                    hotkey::set(Action::Snip, hk);
                    self.hotkey = hk;
                    self.snip_item.set_text(snip_label(&hk));
                    let _ = self
                        .tray
                        .set_tooltip(Some(format!("Recorte ({})", hk.label())));
                    linfo!("hotkey is now {}", hk.label());
                }
                self.cfg = cfg;
            }
        }
        let dark = traykit::theme::tray_is_dark();
        if dark != self.dark_taskbar {
            self.dark_taskbar = dark;
            if let Ok(img) = tray_image(dark) {
                let _ = self.tray.set_icon(Some(img));
            }
        }
    }
}

fn parse_hotkey(text: &str, fallback: &str) -> Hotkey {
    Hotkey::parse(text).unwrap_or_else(|e| {
        lwarn!("hotkey {e}; using {fallback}");
        Hotkey::parse(fallback).expect("the default parses")
    })
}

fn snip_label(hk: &Hotkey) -> String {
    format!("New snip\t{}", hk.label())
}

/// Say what went wrong where it will be seen. The log has it too, but
/// nobody looks there when a recording just does not happen.
fn alert(text: &str) {
    lerror!("{}", text.replace("\n\n", " "));
    #[cfg(windows)]
    unsafe {
        use windows::core::{w, HSTRING};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST,
        };
        let style = MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST;
        MessageBoxW(None, &HSTRING::from(text), w!("Recorte"), style);
    }
}

/// The recording's menu item, saying what the hotkey would do now.
fn record_label(hk: &Hotkey, recording: Option<&Recording>) -> String {
    let what = match recording {
        None => "New recording",
        Some(r) if r.is_filming() => "Stop recording",
        Some(_) => "Start recording",
    };
    format!("{what}\t{}", hk.label())
}

fn open_folder(dir: Option<std::path::PathBuf>) {
    let Some(dir) = dir else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    traykit::util::shell_open(&dir.to_string_lossy());
}

fn tray_image(dark: bool) -> Result<TrayImage, String> {
    TrayImage::from_rgba(icon::tray(dark), icon::TRAY_EDGE, icon::TRAY_EDGE)
        .map_err(|e| format!("could not build the tray image: {e}"))
}
