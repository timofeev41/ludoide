#!/usr/bin/env bash
# Build Ludo.app (universal arm64 + x86_64) into dist/.
# Usage: scripts/bundle_macos.sh [--native]   (--native: only the host architecture)
# LUDO_VERSION overrides the version from Cargo.toml (CI sets it from the release tag).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=${LUDO_VERSION:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}
VERSION=${VERSION#v}
APP=dist/Ludo.app
rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [[ "${1:-}" == "--native" ]]; then
  cargo build --release
  cp target/release/ludo "$APP/Contents/MacOS/ludo"
else
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  lipo -create -output "$APP/Contents/MacOS/ludo" \
    target/aarch64-apple-darwin/release/ludo target/x86_64-apple-darwin/release/ludo
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Ludo</string>
  <key>CFBundleDisplayName</key><string>Ludo</string>
  <key>CFBundleIdentifier</key><string>dev.ludo.ide</string>
  <key>CFBundleExecutable</key><string>ludo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Ad-hoc signature so Apple Silicon will run it (not notarized).
codesign --force --deep --sign - "$APP"
(cd dist && ditto -c -k --keepParent Ludo.app "Ludo-${VERSION}-macos.zip")
echo "Built $APP and dist/Ludo-${VERSION}-macos.zip"
