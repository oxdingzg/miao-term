#!/usr/bin/env bash
# Build one native miaotty.app, carrying miaotty and miaotty-cli.
# PROFILE=debug scripts/package-macos.sh also supports a development bundle.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
profile="${PROFILE:-release}"
target="${CARGO_TARGET_DIR:-target}"
if [ "$profile" = release ]; then
  cargo build --release -p miaotty-app -p miaotty-cli
else
  cargo build -p miaotty-app -p miaotty-cli
fi
built="$target/$profile"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
png="assets/icons/miaotty.png"
[ -f "$png" ] || python3 scripts/make-icon.py "$png" 2e3440
iconset="$staging/miaotty.iconset"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size" "$png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$staging/miaotty.icns"
app="$staging/miaotty.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$built/miaotty" "$built/miaotty-cli" "$app/Contents/MacOS/"
cp "$staging/miaotty.icns" "$app/Contents/Resources/"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>miaotty</string>
  <key>CFBundleDisplayName</key><string>miaotty</string>
  <key>CFBundleIdentifier</key><string>io.miaotty.terminal</string>
  <key>CFBundleExecutable</key><string>miaotty</string>
  <key>CFBundleIconFile</key><string>miaotty.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key><array><dict>
    <key>CFBundleURLName</key><string>io.miaotty.terminal</string>
    <key>CFBundleURLSchemes</key><array>
      <string>miaotty</string><string>ssh</string><string>x-man-page</string>
    </array>
  </dict></array>
</dict></plist>
PLIST
codesign --force --deep --sign - "$app"
mkdir -p "$root/dist"
# Replace only our generated bundles, preserving unrelated dist artifacts.
rm -rf "$root/dist/miaotty.app"
mv "$app" "$root/dist/miaotty.app"
legacy="$root/dist/miaotty-native.app"
if [ -f "$legacy/Contents/Info.plist" ] && \
   [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$legacy/Contents/Info.plist")" = io.miaotty.terminal.native ]; then
  rm -rf "$legacy"
fi
python3 scripts/check-macos-bundle.py "$root/dist/miaotty.app"
echo "built: dist/miaotty.app (native miaotty)"
