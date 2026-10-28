//! The global hotkey.
//!
//! Windows registers Win+Shift+S for itself, so `RegisterHotKey` cannot have
//! it. A low-level keyboard hook sees the keys before the shell does and
//! swallows the combination instead; that is the Windows side. The parsing
//! and the label are shared.

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as imp;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as imp;

/// Start listening, calling `fire` each time the hotkey is pressed. `fire`
/// runs on the listener's thread and must return quickly.
pub fn start(hotkey: Hotkey, fire: impl Fn() + Send + Sync + 'static) {
    imp::start(hotkey, Box::new(fire));
}

/// Change the combination being listened for.
pub fn set(hotkey: Hotkey) {
    imp::set(hotkey);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Win on Windows, Cmd on macOS, Super on Linux.
    pub meta: bool,
    pub key: Key,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// `A` to `Z` or `0` to `9`, uppercase ASCII.
    Char(u8),
    /// F1 to F24.
    F(u8),
    PrintScreen,
}

impl Hotkey {
    /// `Win+Shift+S`, `ctrl + alt + p`, `PrintScreen`. Case and spaces do
    /// not matter.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut hk = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key: Key::PrintScreen,
        };
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" | "option" | "opt" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "windows" | "cmd" | "command" | "super" | "meta" => hk.meta = true,
                "printscreen" | "prtsc" | "print" | "prtscn" => key = Some(Key::PrintScreen),
                _ if lower.len() == 1 && lower.as_bytes()[0].is_ascii_alphanumeric() => {
                    key = Some(Key::Char(lower.as_bytes()[0].to_ascii_uppercase()))
                }
                _ if lower.starts_with('f') => match lower[1..].parse::<u8>() {
                    Ok(n @ 1..=24) => key = Some(Key::F(n)),
                    _ => return Err(format!("`{part}` is not a key")),
                },
                "" => return Err(format!("`{text}` has an empty part")),
                _ => return Err(format!("`{part}` is not a key")),
            }
        }
        hk.key = key.ok_or_else(|| format!("`{text}` has no key, only modifiers"))?;
        Ok(hk)
    }

    /// The combination as the menu shows it, in the platform's words.
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let meta = if cfg!(windows) {
            "Win"
        } else if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Super"
        };
        if self.meta {
            parts.push(meta.into());
        }
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        parts.push(match self.key {
            Key::Char(c) => (c as char).to_string(),
            Key::F(n) => format!("F{n}"),
            Key::PrintScreen => "PrintScreen".into(),
        });
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_parses() {
        let hk = Hotkey::parse("Win+Shift+S").unwrap();
        assert!(hk.meta && hk.shift && !hk.ctrl && !hk.alt);
        assert_eq!(hk.key, Key::Char(b'S'));
    }

    #[test]
    fn case_and_spaces_do_not_matter() {
        assert_eq!(
            Hotkey::parse(" ctrl + ALT + p ").unwrap(),
            Hotkey::parse("Ctrl+Alt+P").unwrap()
        );
        assert_eq!(Hotkey::parse("f12").unwrap().key, Key::F(12));
        assert_eq!(Hotkey::parse("PrintScreen").unwrap().key, Key::PrintScreen);
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(Hotkey::parse("Ctrl+Shift").is_err());
        assert!(Hotkey::parse("Ctrl+Banana").is_err());
        assert!(Hotkey::parse("F25").is_err());
        assert!(Hotkey::parse("Ctrl++S").is_err());
    }

    #[test]
    fn the_label_round_trips() {
        let hk = Hotkey::parse("shift+ctrl+4").unwrap();
        assert_eq!(Hotkey::parse(&hk.label()).unwrap(), hk);
    }
}
