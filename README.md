<div align="center">

<img src="assets/icon-256.png" width="112" height="112" alt="Print">

# Print

A snipping tool in the tray.<br>
For Windows today; macOS and Linux next.

**[josebarbosa.dev/print](https://josebarbosa.dev/print/)**

</div>

Press **Win+Shift+S**, drag across any part of the screen, and it's on the
clipboard. A small editor opens with it for a pen, a highlighter, arrows,
boxes and blur, then copy it again or save it. It's the same keys as
Windows' own Snipping Tool, but faster to come up and with better markup.

It's one file of under 1 MB with nothing else to install. Idle in the tray,
it uses a few MB of memory and no CPU.

## Taking a snip

- **Win+Shift+S**, a left click on the tray icon, **New snip** in its menu,
  or launching Print again while it's running all freeze the screen and dim
  it. **Snip in 5 seconds** waits first, for a menu or a hover state.
- Drag across what you want. When you let go it's on the clipboard, as an
  image any app can paste.
- **Enter** without dragging takes the whole monitor. **Esc** or a right
  click cancels.

Print takes Win+Shift+S over from Windows: while Print is running, the key
opens Print and not Snipping Tool. Quit Print and Windows has it back.

## The editor

| | tool | key |
|---|---|---|
| pen | freehand | P |
| highlighter | wide, see-through, like ink on paper | H |
| arrow | | A |
| box | | R |
| blur | pixelates what's under it, for an email address or a token | B |

Pick a colour (keys 1 to 6) and a size (the dot, or **[** and **]**). Each
tool remembers its own colour, so the highlighter stays yellow.

**Ctrl+Z** undo, **Ctrl+Y** redo, **Ctrl+C** copy, **Ctrl+S** save,
**Ctrl+N** a new snip, **Esc** close. Copy and save show a tick when they're
done. Saving suggests `Pictures\Screenshots\Print 2026-10-28 14-03-11.png`.

## Settings

**Edit settings** in the menu opens `%APPDATA%\Print\config.json`. Changes
apply within two seconds, the hotkey included.

| setting | default | |
|---|---|---|
| `hotkey` | `Win+Shift+S` | Ctrl, Alt, Shift, Win, plus a letter, digit, F1-F24 or PrintScreen |
| `copyOnSnip` | `true` | copy the moment the snip is taken |
| `openEditor` | `true` | open the editor with each snip |
| `autoSave` | `false` | also save every snip, as taken |
| `saveFolder` | `""` | where to save; empty is `Pictures\Screenshots` |

The menu also has **Start at login**, **Open screenshots folder** and
**Open log** (`%APPDATA%\Print\print.log`; `PRINT_LOG=debug` for more).

## Building

```sh
cargo build --release          # target/release/print.exe
print --dump-icons assets      # redraw assets/icon.ico and the PNGs
print --render-docs            # redraw docs/img: the site's icons and editor pictures
```

The shared tray-app pieces (log, single instance, autostart, config, theme)
come from [traykit](https://github.com/kidchenko/traykit). To build against a
local checkout next to this one, see traykit's README.

## How it works

The overlay and the editor are plain pixel buffers drawn with tiny-skia and
shown with softbuffer: no webview, no GPU, and the same drawing code on
every system. The screen is captured once, with GDI, before the overlay goes
up, so you select on a still picture, menus and tooltips included. The
hotkey is a low-level keyboard hook, the only way to have Win+Shift+S, which
Windows registers for itself.

## License

MIT
