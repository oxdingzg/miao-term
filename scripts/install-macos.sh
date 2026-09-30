#!/usr/bin/env bash
# Install the unified miaotty.app and retire the former native application.
# Existing app bundles are archived locally; configuration and sessions stay put.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
destination="${1:-/Applications}"
source_app="$root/dist/miaotty.app"
python3 "$root/scripts/check-macos-bundle.py" "$source_app"
mkdir -p "$destination"
for entry in miaotty miaotty-native; do
  app="$destination/$entry.app"
  if [ -e "$app" ]; then
    ident=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")
    case "$entry:$ident" in
      miaotty:io.miaotty.terminal|miaotty-native:io.miaotty.terminal.native) ;;
      *) echo "unexpected application identity: $entry ($ident)" >&2; exit 1 ;;
    esac
  fi
done
backup="$(mktemp -d "${TMPDIR:-/tmp}/miaotty-app-backup.XXXXXX")"
staging="$(mktemp -d "$destination/.miaotty-install.XXXXXX")"
rollback() {
  for entry in miaotty miaotty-native; do
    if [ -d "$staging/old-$entry.app" ] && [ ! -e "$destination/$entry.app" ]; then
      mv "$staging/old-$entry.app" "$destination/$entry.app"
    fi
  done
  rm -rf "$staging"
}
trap rollback EXIT
ditto "$source_app" "$staging/miaotty.app"
python3 "$root/scripts/check-macos-bundle.py" "$staging/miaotty.app"
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
for entry in miaotty miaotty-native; do
  app="$destination/$entry.app"
  if [ -d "$app" ]; then
    ditto -c -k --keepParent "$app" "$backup/$entry.app.zip"
    if [ "$entry" = miaotty-native ]; then
      "$lsregister" -u "$app" || true
    fi
    mv "$app" "$staging/old-$entry.app"
  fi
done
mv "$staging/miaotty.app" "$destination/miaotty.app"
"$lsregister" -f "$destination/miaotty.app"
rm -rf "$staging"
trap - EXIT
echo "installed: $destination/miaotty.app (native)"
echo "previous bundles archived locally: $backup"
