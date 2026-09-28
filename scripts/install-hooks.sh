#!/bin/sh
# Point git at the versioned hooks so the privacy check runs before every commit.
#
#   sh scripts/install-hooks.sh
#
# To also block your own identifiers, write them (one per line) to
# ~/.config/miao-term/privacy-denylist — it stays outside the repository.
set -eu

root=$(git rev-parse --show-toplevel)
git -C "$root" config core.hooksPath .githooks
chmod +x "$root/.githooks/pre-commit" "$root/scripts/check-privacy.sh"
echo "installed: core.hooksPath=.githooks"
echo "denylist file (optional): $HOME/.config/miao-term/privacy-denylist"
