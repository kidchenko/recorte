# Changelog

## [Unreleased]

- macOS: snipping. Ctrl+Shift+S (Carbon hotkeys, no Accessibility
  permission) freezes every display, each at its own pixel size, so a
  Retina screen next to a plain one lines up; drag a region and it's on the
  pasteboard as PNG and TIFF, and the editor opens. The overlay covers the
  menu bar and the Dock, and appears over full-screen apps. The Screen
  Recording permission is asked for on the first snip. Recording is not
  there yet on macOS.
- macOS packaging: `build.sh` makes Recorte.app and, with `--dmg`, a disk
  image. Releases carry a universal Recorte-<version>.dmg, and the Homebrew
  tap has a `recorte` cask. The website offers both downloads and shows the
  visitor's platform first.

- First version, for Windows. Win+Shift+S (taken over from Snipping Tool)
  freezes every monitor; drag a region and it's on the clipboard as a DIB
  and a PNG. An editor opens with it: pen, highlighter, arrow, box and
  pixelate blur, six colours, three sizes, undo and redo, copy and save.
- Tray menu: New snip, Snip in 5 seconds, Open screenshots folder, Start at
  login, Edit settings, Open log. A left click on the icon, or launching
  Recorte again, starts a snip.
- `config.json`: the hotkey, copy on snip, open the editor, auto-save and
  the folder, all picked up without a restart.
- Screen recording. Win+Shift+R (taken over from Snipping Tool too) selects
  an area; a dashed frame goes around it and a small bar at the top of the
  screen; Win+Shift+R again or the red button starts, and once more stops.
  The video is an H.264 MP4 at 30 frames a second, with the pointer, saved
  to `Videos\Screen Recordings` and opened. The bar can pause, and has the
  microphone and the computer's sound, each in the video or not. New
  settings: `recordHotkey`, `recordFolder`, `openRecording`, `recordMic`,
  `recordSystemAudio`. The tray menu has New recording and Open recordings
  folder.
- A menu at the top of the selection overlay, to change between a snip and
  a recording, or close.
- A landing page in `docs/` with a working copy of the editor to try in the
  browser. Its pictures and icons are drawn by `recorte --render-docs`.
