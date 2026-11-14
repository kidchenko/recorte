#!/usr/bin/env bash
# Builds Recorte.app for macOS, the counterpart of build.ps1 on Windows.
#
#   ./build.sh                 # test + release binary + dist/Recorte.app
#   ./build.sh --dmg           # ... then wrap it in dist/Recorte-<version>.dmg
#   ./build.sh --run           # ... then open it
#   ./build.sh --install       # ... then copy it to /Applications and open it
#   ./build.sh --universal     # one binary for Apple silicon and Intel
#   ./build.sh --skip-tests
#
# The app is signed ad hoc unless SIGN_IDENTITY names a certificate in the
# keychain ("Developer ID Application: ..." or "Apple Development: ...").
# macOS keys the Screen Recording permission to the signature, and an ad hoc
# one changes with every build, so with it the permission has to be given
# again after each rebuild; a real identity keeps it.

set -euo pipefail
cd "$(dirname "$0")"

run=0 install=0 universal=0 tests=1 dmg=0
for arg in "$@"; do
  case "$arg" in
    --run) run=1 ;;
    --install) install=1 ;;
    --universal) universal=1 ;;
    --dmg) dmg=1 ;;
    --skip-tests) tests=0 ;;
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done

step() { printf '\n\033[36m==> %s\033[0m\n' "$1"; }
ok()   { printf '    \033[32m%s\033[0m\n' "$1"; }
warn() { printf '    \033[33m%s\033[0m\n' "$1"; }

if [[ "$(uname)" != Darwin ]]; then
  echo "build.sh makes the macOS app; on Windows use build.ps1" >&2
  exit 1
fi
command -v cargo >/dev/null || { echo "cargo not found. Install Rust: https://rustup.rs" >&2; exit 1; }

# [package].version specifically, not the first `version =` in the file.
version=$(awk '
  /^\[/ { in_pkg = ($0 == "[package]") }
  in_pkg && /^[[:space:]]*version[[:space:]]*=/ { gsub(/.*=[[:space:]]*"|".*/, ""); print; exit }
' Cargo.toml)
[[ -n "$version" ]] || { echo "could not read [package].version from Cargo.toml" >&2; exit 1; }

if pgrep -x recorte >/dev/null; then
  warn "stopping the running Recorte first"
  pkill -x recorte || true
  sleep 0.5
fi

if (( tests )); then
  step "Running tests"
  cargo test --quiet
  ok "tests passed"
fi

step "Building release binary"
if (( universal )); then
  targets=(aarch64-apple-darwin x86_64-apple-darwin)
  for t in "${targets[@]}"; do
    rustup target list --installed | grep -qx "$t" || rustup target add "$t"
    cargo build --release --target "$t"
  done
  mkdir -p target/release
  lipo -create -output target/release/recorte-universal \
    "target/${targets[0]}/release/recorte" "target/${targets[1]}/release/recorte"
  bin=target/release/recorte-universal
  arch=universal
else
  cargo build --release
  bin=target/release/recorte
  arch=$(uname -m)
fi
ok "recorte   $(du -h "$bin" | cut -f1 | tr -d ' ')   ($(lipo -archs "$bin"))"

step "Assembling Recorte.app"
app=dist/Recorte.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/recorte"

# The icon, from the 1024 px PNG the app draws of itself (--dump-icons).
iconset=$(mktemp -d)/Recorte.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size assets/icon-1024.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double assets/icon-1024.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Recorte.icns"
rm -rf "$(dirname "$iconset")"

# LSUIElement: a menu-bar app, with no Dock icon and no app menu. The
# bundle id is the one main.rs gives traykit.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>     <string>en</string>
  <key>CFBundleDisplayName</key>           <string>Recorte</string>
  <key>CFBundleExecutable</key>            <string>recorte</string>
  <key>CFBundleIconFile</key>              <string>Recorte</string>
  <key>CFBundleIdentifier</key>            <string>com.kidchenko.recorte</string>
  <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
  <key>CFBundleName</key>                  <string>Recorte</string>
  <key>CFBundlePackageType</key>           <string>APPL</string>
  <key>CFBundleShortVersionString</key>    <string>$version</string>
  <key>CFBundleVersion</key>               <string>$version</string>
  <key>LSApplicationCategoryType</key>     <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>        <string>11.0</string>
  <key>LSUIElement</key>                   <true/>
  <key>NSHighResolutionCapable</key>       <true/>
  <key>NSHumanReadableCopyright</key>      <string>MIT License, kidchenko</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

identity=${SIGN_IDENTITY:--}
if [[ "$identity" == - ]]; then
  codesign --force --sign - "$app"
  ok "signed ad hoc (set SIGN_IDENTITY to keep permissions across builds)"
else
  codesign --force --options runtime --timestamp --sign "$identity" "$app"
  ok "signed as $identity"
fi
codesign --verify --strict "$app"
ok "$app"

if (( dmg )); then
  step "Building the disk image"
  dmg_path="dist/Recorte-$version.dmg"
  stage=$(mktemp -d)
  cp -R "$app" "$stage/"
  # Drag to Applications, the way a Mac app's disk image usually opens.
  ln -s /Applications "$stage/Applications"
  rm -f "$dmg_path"
  hdiutil create -quiet -volname Recorte -srcfolder "$stage" -ov -format UDZO "$dmg_path"
  rm -rf "$stage"
  ok "$dmg_path   $(du -h "$dmg_path" | cut -f1 | tr -d ' ')   ($arch)"
  ok "sha256  $(shasum -a 256 "$dmg_path" | cut -d' ' -f1)"
  # The same name every release, so the website's button can point at
  # github.com/kidchenko/recorte/releases/latest/download/Recorte.dmg.
  cp -f "$dmg_path" dist/Recorte.dmg
fi

if (( install )); then
  step "Installing to /Applications"
  rm -rf /Applications/Recorte.app
  cp -R "$app" /Applications/
  ok "/Applications/Recorte.app"
  app=/Applications/Recorte.app
  run=1
fi

if (( run )); then
  step "Launching"
  open "$app"
  ok "running in the menu bar"
fi
