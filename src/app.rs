//! The tray application: the icon and its menu, and a snip's way through
//! the app: hotkey, capture, overlay, clipboard, editor.
//!
//! Everything here runs on the main thread, driven by the event loop in
//! `main`. The hotkey and the single-instance listener run on their own
//! threads and only post a [`UserEvent`].
//!
//! The loop sleeps between events. Idle, it wakes every two seconds to
//! notice a hand-edited config; there is nothing else to poll.

use crate::capture;
use crate::config::Config;
use crate::editor::{self, Editor};
use crate::hotkey::{self, Hotkey};
use crate::icon;
use crate::overlay::{self, Outcome, Overlay};
use crate::snip::{self, Snip};

use std::time::{Duration, Instant, SystemTime};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopProxy, EventLoopWindowTarget};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon as TrayImage, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use traykit::{ldebug, lerror, linfo, lwarn};

const ID_SNIP: &str = "snip";
const ID_SNIP_LATER: &str = "snip-later";
const ID_FOLDER: &str = "folder";
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
    Menu(String),
    TrayClick,
}

pub struct App {
    tray: TrayIcon,
    snip_item: MenuItem,
    autostart_item: CheckMenuItem,
    cfg: Config,
    cfg_seen: Option<SystemTime>,
    overlays: Vec<Overlay>,
    editors: Vec<Editor>,
    /// A snip asked for, waiting for a menu to fade or a countdown.
    snip_at: Option<Instant>,
    next_idle: Instant,
    dark_taskbar: bool,
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

        let hk = parse_hotkey(&cfg);
        let p = proxy.clone();
        hotkey::start(hk, move || {
            let _ = p.send_event(UserEvent::Snip);
        });

        let snip_item = MenuItem::with_id(ID_SNIP, snip_label(&hk), true, None);
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
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_FOLDER, "Open screenshots folder", true, None),
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &MenuItem::with_id(ID_CONFIG, "Edit settings", true, None),
            &MenuItem::with_id(ID_LOG, "Open log", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_QUIT, "Quit Print", true, None),
        ])
        .map_err(|e| format!("could not build the menu: {e}"))?;

        let dark_taskbar = traykit::theme::tray_is_dark();
        let tray = TrayIconBuilder::new()
            .with_tooltip(format!("Print ({})", hk.label()))
            .with_icon(tray_image(dark_taskbar)?)
            .with_menu(Box::new(menu))
            // Left click snips; the menu is the right button's.
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| format!("could not create the tray icon: {e}"))?;

        Ok(Self {
            tray,
            snip_item,
            autostart_item,
            cfg_seen: traykit::config::modified(),
            cfg,
            overlays: Vec::new(),
            editors: Vec::new(),
            snip_at: None,
            next_idle: Instant::now() + IDLE_EVERY,
            dark_taskbar,
        })
    }

    pub fn handle(
        &mut self,
        event: Event<UserEvent>,
        target: &EventLoopWindowTarget<UserEvent>,
    ) -> ControlFlow {
        match event {
            Event::UserEvent(UserEvent::Snip) => self.snip(target),
            Event::UserEvent(UserEvent::TrayClick) => self.snip_in(AFTER_CLICK),
            Event::UserEvent(UserEvent::Menu(id)) => match id.as_str() {
                ID_SNIP => self.snip_in(AFTER_CLICK),
                ID_SNIP_LATER => self.snip_in(LATER),
                ID_FOLDER => self.open_folder(),
                ID_AUTOSTART => self.toggle_autostart(),
                ID_CONFIG => traykit::config::reveal::<Config>(),
                ID_LOG => traykit::log::reveal(),
                ID_QUIT => {
                    linfo!("quit from the menu");
                    return ControlFlow::Exit;
                }
                _ => {}
            },
            Event::WindowEvent {
                window_id, event, ..
            } => {
                if let Some(i) = self.overlays.iter().position(|o| o.id() == window_id) {
                    match self.overlays[i].handle(&event) {
                        Outcome::Nothing => {}
                        Outcome::Cancel(why) => {
                            linfo!("snip cancelled ({why})");
                            self.overlays.clear();
                        }
                        Outcome::Done(snip) => {
                            self.overlays.clear();
                            self.taken(snip, target);
                        }
                    }
                } else if let Some(i) = self.editors.iter().position(|e| e.id() == window_id) {
                    match self.editors[i].handle(&event) {
                        editor::Request::Nothing => {}
                        editor::Request::Close => {
                            self.editors.remove(i);
                        }
                        editor::Request::NewSnip => self.snip(target),
                    }
                }
            }
            Event::RedrawRequested(id) => {
                if let Some(o) = self.overlays.iter_mut().find(|o| o.id() == id) {
                    o.redraw();
                } else if let Some(e) = self.editors.iter_mut().find(|e| e.id() == id) {
                    e.redraw();
                }
            }
            _ => {}
        }

        let now = Instant::now();
        if self.snip_at.is_some_and(|at| now >= at) {
            self.snip_at = None;
            self.snip(target);
        }
        for e in &mut self.editors {
            e.tick(now);
        }
        if now >= self.next_idle {
            self.next_idle = now + IDLE_EVERY;
            self.idle();
        }

        let wake = [Some(self.next_idle), self.snip_at]
            .into_iter()
            .chain(self.editors.iter().map(Editor::deadline))
            .flatten()
            .min()
            .unwrap_or(self.next_idle);
        ControlFlow::WaitUntil(wake)
    }

    fn snip_in(&mut self, delay: Duration) {
        self.snip_at = Some(Instant::now() + delay);
    }

    /// Freeze the screen and put the overlay over every monitor.
    fn snip(&mut self, target: &EventLoopWindowTarget<UserEvent>) {
        if !self.overlays.is_empty() {
            // Pressing the hotkey again while selecting brings the overlay
            // back to the front, in case something took the focus.
            for o in &self.overlays {
                o.focus();
            }
            return;
        }
        let started = Instant::now();
        let frame = match capture::screen() {
            Ok(f) => f,
            Err(e) => {
                lerror!("capture failed: {e}");
                return;
            }
        };
        let under_pointer = overlay::monitor_at(target);
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
            match Overlay::new(target, monitor, &frame) {
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
                    let path =
                        snip::fresh_path(&dir, &snip::file_name(traykit::time::LocalTime::now()));
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

    fn open_folder(&self) {
        let Some(dir) = self.cfg.save_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        traykit::util::shell_open(&dir.to_string_lossy());
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
                if cfg.hotkey != self.cfg.hotkey {
                    let hk = parse_hotkey(&cfg);
                    hotkey::set(hk);
                    self.snip_item.set_text(snip_label(&hk));
                    let _ = self
                        .tray
                        .set_tooltip(Some(format!("Print ({})", hk.label())));
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

fn parse_hotkey(cfg: &Config) -> Hotkey {
    Hotkey::parse(&cfg.hotkey).unwrap_or_else(|e| {
        let fallback = Config::default().hotkey;
        lwarn!("hotkey {e}; using {fallback}");
        Hotkey::parse(&fallback).expect("the default parses")
    })
}

fn snip_label(hk: &Hotkey) -> String {
    format!("New snip\t{}", hk.label())
}

fn tray_image(dark: bool) -> Result<TrayImage, String> {
    TrayImage::from_rgba(icon::tray(dark), icon::TRAY_EDGE, icon::TRAY_EDGE)
        .map_err(|e| format!("could not build the tray image: {e}"))
}
