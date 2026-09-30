#!/usr/bin/env bash
# Build the macOS .app bundles for miaotty (ad-hoc signed).
#
#   scripts/package-macos.sh            # release build (default)
#   PROFILE=debug scripts/package-macos.sh
#
# Produces dist/miaotty.app (the eframe host) and dist/miaotty-native.app (the
# native host). Each bundle carries both binaries and the URL schemes; the bundle
# is also what gives the native host a real application icon, which the OS menu
# bar needs (ADR 0031).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

profile="${PROFILE:-release}"
target="${CARGO_TARGET_DIR:-target}"
if [ "$profile" = "release" ]; then
  cargo build --release -p miaotty-app
  cargo build --release -p miaotty-cli
  cargo build --release -p miao-term-widget --bin miaotty-native
else
  cargo build -p miaotty-app
  cargo build -p miaotty-cli
  cargo build -p miao-term-widget --bin miaotty-native
fi
built="$target/$profile"

# App icon: committed as a PNG, converted to .icns with the system tools.
png="assets/icons/miaotty.png"
[ -f "$png" ] || python3 scripts/make-icon.py "$png" 2e3440
iconset="$(mktemp -d)/miaotty.iconset"
icns="$(mktemp -d)/miaotty.icns"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size" "$png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$icns"

# write_bundle <app dir> <executable> <bundle id> <display name>
write_bundle() {
  app="$1" exe="$2" ident="$3" display="$4"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$built/miaotty" "$built/miaotty-cli" "$built/miaotty-native" "$app/Contents/MacOS/"
  cp "$icns" "$app/Contents/Resources/miaotty.icns"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${display}</string>
  <key>CFBundleDisplayName</key><string>${display}</string>
  <key>CFBundleIdentifier</key><string>${ident}</string>
  <key>CFBundleExecutable</key><string>${exe}</string>
  <key>CFBundleIconFile</key><string>miaotty.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>${ident}</string>
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
    codesign --force --deep --sign - "$app" >/dev/null 2>&1 || \
      echo "warning: ad-hoc codesign failed for $app"
  fi
  echo "built: $app"
}

rm -rf "$root/dist"
mkdir -p "$root/dist"
write_bundle "$root/dist/miaotty.app" miaotty io.miaotty.terminal miaotty
write_bundle "$root/dist/miaotty-native.app" miaotty-native io.miaotty.terminal.native "miaotty native"
