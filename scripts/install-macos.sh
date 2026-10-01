#!/usr/bin/env bash
# Install mtty.app and retire the bundles of the former name (ADR 0032).
# Replaced bundles are archived locally; configuration and sessions stay put
# (mtty copies the former configuration directory on first start).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
destination="${1:-/Applications}"
source_app="$root/dist/mtty.app"
python3 "$root/scripts/check-macos-bundle.py" "$source_app"
mkdir -p "$destination"
# entry:expected bundle identifier; anything else at that path is not ours.
entries="mtty:dev.mtty.terminal miaotty:io.miaotty.terminal miaotty-native:io.miaotty.terminal.native"
for pair in $entries; do
  app="$destination/${pair%%:*}.app"
  if [ -e "$app" ]; then
    ident=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")
    if [ "$ident" != "${pair#*:}" ]; then
      echo "unexpected application identity: ${pair%%:*} ($ident)" >&2
      exit 1
    fi
  fi
done
backup="$(mktemp -d "${TMPDIR:-/tmp}/mtty-app-backup.XXXXXX")"
staging="$(mktemp -d "$destination/.mtty-install.XXXXXX")"
rollback() {
  for pair in $entries; do
    entry="${pair%%:*}"
    if [ -d "$staging/old-$entry.app" ] && [ ! -e "$destination/$entry.app" ]; then
      mv "$staging/old-$entry.app" "$destination/$entry.app"
    fi
  done
  rm -rf "$staging"
}
trap rollback EXIT
ditto "$source_app" "$staging/mtty.app"
python3 "$root/scripts/check-macos-bundle.py" "$staging/mtty.app"
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
for pair in $entries; do
  entry="${pair%%:*}"
  app="$destination/$entry.app"
  if [ -d "$app" ]; then
    ditto -c -k --keepParent "$app" "$backup/$entry.app.zip"
    if [ "$entry" != mtty ]; then
      # Drop the retired bundle's URL-scheme registrations.
      "$lsregister" -u "$app" || true
    fi
    mv "$app" "$staging/old-$entry.app"
  fi
done
mv "$staging/mtty.app" "$destination/mtty.app"
"$lsregister" -f "$destination/mtty.app"
rm -rf "$staging"
trap - EXIT
echo "installed: $destination/mtty.app (native)"
echo "previous bundles archived locally: $backup"
