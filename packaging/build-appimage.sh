#!/usr/bin/env bash
# Build a Linux AppImage for FreeJoyXConfigurator.
#
# Prereqs (on the build host):
# - cargo + rust toolchain
# - appimagetool (https://github.com/AppImage/AppImageKit/releases) on $PATH
# - libudev-dev / libusb-1.0-0-dev installed at build time (hidapi
#   compile-time link)
#
# Output: dist/FreeJoyXConfigurator-x86_64.AppImage
#
# Run from the repo root:
#     ./packaging/build-appimage.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist"
APPDIR="$DIST/FreeJoyXConfigurator.AppDir"
BIN="$ROOT/target/release/freejoyx-app"

cd "$ROOT"

echo "==> cargo build --release -p freejoyx-app"
cargo build --release -p freejoyx-app

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
          "$APPDIR/usr/share/applications"

cp "$BIN" "$APPDIR/usr/bin/freejoyx-app"
cp "$ROOT/crates/freejoyx-ui/assets/icon.svg" \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps/freejoyx-configurator.svg"
cp "$ROOT/crates/freejoyx-ui/assets/icon.svg" "$APPDIR/freejoyx-configurator.svg"

cat > "$APPDIR/freejoyx-configurator.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=FreeJoyXConfigurator
Exec=freejoyx-app
Icon=freejoyx-configurator
Comment=Configurator for FreeJoyX DIY USB HID game controllers
Categories=Utility;HardwareSettings;
Terminal=false
EOF

cp "$APPDIR/freejoyx-configurator.desktop" \
    "$APPDIR/usr/share/applications/freejoyx-configurator.desktop"

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/freejoyx-app" "$@"
EOF
chmod +x "$APPDIR/AppRun"

echo "==> appimagetool"
appimagetool "$APPDIR" "$DIST/FreeJoyXConfigurator-x86_64.AppImage"

echo "==> built $DIST/FreeJoyXConfigurator-x86_64.AppImage"
