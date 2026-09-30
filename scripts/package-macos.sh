#!/usr/bin/env bash
# Build a minimal macOS .app bundle for miaotty (ad-hoc signed).
#
#   scripts/package-macos.sh            # release build (default)
#   PROFILE=debug scripts/package-macos.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

profile="${PROFILE:-release}"
if [ "$profile" = "release" ]; then
  cargo build --release -p miaotty-app
  cargo build --release -p miao-term-widget --bin miaotty-native
  bin="target/release/miaotty"
  native="target/release/miaotty-native"
else
  cargo build -p miaotty-app
  cargo build -p miao-term-widget --bin miaotty-native
  bin="target/debug/miaotty"
  native="target/debug/miaotty-native"
fi

app="$root/dist/miaotty.app"
rm -rf "$root/dist"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/miaotty"
# The native host ships in the same bundle; the bundle is what registers the URL
# schemes, and either host can be the one that is running when a link arrives.
cp "$native" "$app/Contents/MacOS/miaotty-native"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>miaotty</string>
  <key>CFBundleDisplayName</key><string>miaotty</string>
  <key>CFBundleIdentifier</key><string>io.miaotty.terminal</string>
  <key>CFBundleExecutable</key><string>miaotty</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>io.miaotty.terminal</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>miaotty</string>
        <string>ssh</string>
        <string>x-man-page</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$app" >/dev/null 2>&1 || echo "warning: ad-hoc codesign failed"
fi

echo "built: $app"
