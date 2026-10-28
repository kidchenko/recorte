// No console window: this is a tray app. No-op off Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! Recorte is a snipping tool in the tray: a hotkey freezes the screen, you
//! drag out a part of it, and it is on the clipboard. An editor opens with
//! it for a pen, a highlighter, arrows, boxes and blur, then copy or save.
//!
//! | module      | responsibility                                            |
//! |-------------|-----------------------------------------------------------|
//! | `app`       | the tray icon and menu, and moving a snip along           |
//! | `hotkey`    | the global hotkey (Win+Shift+S taken over from Windows)   |
//! | `capture`   | a picture of every screen at once                         |
//! | `overlay`   | the frozen, dimmed screen you drag a selection on         |
//! | `editor`    | the markup window: tools, undo, copy, save                |
//! | `snip`      | the cropped picture, as PNG, and where it is saved        |
//! | `clipboard` | putting a picture on the clipboard                        |
//! | `view`      | a window that is a plain pixel buffer                     |
//! | `icon`      | the tray icon and app icon, drawn rather than shipped     |
//! | `config`    | the settings in `config.json`                             |
//!
//! The windows are pixel buffers drawn with tiny-skia and shown with
//! softbuffer, so the overlay and the editor are written once for all three
//! systems. Capture, the clipboard and the hotkey are per system.

mod app;
mod capture;
mod clipboard;
mod config;
mod editor;
mod hotkey;
mod icon;
mod overlay;
mod snip;
mod view;

use app::{App, UserEvent};
use tao::event_loop::EventLoopBuilder;
use traykit::{lerror, linfo};

pub const APP: traykit::App = traykit::App {
    name: "Recorte",
    id: "recorte",
    bundle_id: "com.kidchenko.recorte",
    summary: "Snip a part of the screen, mark it up, copy it",
};

const USAGE: &str = "\
Recorte: snip a part of the screen, mark it up, copy it.

usage:
  recorte                     run in the tray, or start a snip in the running copy
  recorte --dump-icons [dir]  write the app icon as PNGs and icon.ico
  recorte --render-docs [dir] draw the landing page's images (default docs/img)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => tray(),
        Some("--dump-icons") => {
            traykit::util::attach_console();
            let dir = args.get(1).map(String::as_str).unwrap_or("assets");
            if let Err(e) = icon::dump(std::path::Path::new(dir)) {
                eprintln!("recorte: {e}");
                std::process::exit(1);
            }
        }
        Some("--render-docs") => {
            traykit::util::attach_console();
            let dir =
                std::path::PathBuf::from(args.get(1).map(String::as_str).unwrap_or("docs/img"));
            if let Err(e) = render_docs(&dir) {
                eprintln!("recorte: {e}");
                std::process::exit(1);
            }
        }
        Some("-h" | "--help" | "help") => {
            traykit::util::attach_console();
            println!("{USAGE}");
        }
        Some(other) => {
            traykit::util::attach_console();
            eprintln!("recorte: unknown argument `{other}`\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

fn tray() {
    traykit::init(APP);
    linfo!(
        "--- Recorte {} starting (pid {}) ---",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );

    // A second launch asks the first for a snip, which makes the Start menu,
    // Spotlight or a launcher one more way to start one.
    let Some(signal) = traykit::single_instance::acquire() else {
        linfo!("another instance owns the tray; asked it for a snip, exiting");
        return;
    };

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Menu-bar agent, no Dock icon. tao resets the activation policy as it
    // starts, so it is asked for here and not only in Info.plist.
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();

    let summon = proxy.clone();
    signal.watch(move || {
        let _ = summon.send_event(UserEvent::Snip);
    });

    let mut app = match App::new(proxy) {
        Ok(app) => app,
        Err(e) => {
            lerror!("{e}");
            return;
        }
    };
    linfo!("tray icon created");

    event_loop.run(move |event, target, control_flow| {
        *control_flow = app.handle(event, target);
    });
}

/// The landing page's pictures, drawn by the app itself: the editor in both
/// themes around the sample snip, the sample on its own for the page's demo
/// to draw on, and the icons. At 2x, for sharp screens.
fn render_docs(dir: &std::path::Path) -> Result<(), String> {
    icon::dump_site(dir)?;
    let save = |name: &str, pm: Option<tiny_skia::Pixmap>| {
        let pm = pm.ok_or("could not render")?;
        let path = dir.join(name);
        snip::Snip {
            width: pm.width(),
            height: pm.height(),
            rgba: pm.take(),
            scale: 2.0,
        }
        .save(&path)?;
        println!("{}", path.display());
        Ok::<(), String>(())
    };
    save("editor-dark.png", editor::render_sample(true, 2.0))?;
    save("editor-light.png", editor::render_sample(false, 2.0))?;
    save("sample.png", editor::sample_snip(2.0))?;
    Ok(())
}
