# Changelog

## [Unreleased]

- First version, for Windows. Win+Shift+S (taken over from Snipping Tool)
  freezes every monitor; drag a region and it's on the clipboard as a DIB
  and a PNG. An editor opens with it: pen, highlighter, arrow, box and
  pixelate blur, six colours, three sizes, undo and redo, copy and save.
- Tray menu: New snip, Snip in 5 seconds, Open screenshots folder, Start at
  login, Edit settings, Open log. A left click on the icon, or launching
  Print again, starts a snip.
- `config.json`: the hotkey, copy on snip, open the editor, auto-save and
  the folder, all picked up without a restart.
- A landing page in `docs/` with a working copy of the editor to try in the
  browser. Its pictures and icons are drawn by `print --render-docs`.
