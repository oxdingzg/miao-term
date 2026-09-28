#!/bin/sh
# check-privacy.sh — fail if tracked files leak machine-specific details or secrets.
#
#   sh scripts/check-privacy.sh [files...]      # default: every tracked file
#
# Two layers:
#   1. generic patterns (personal home paths, private IPs, common token shapes)
#   2. an optional denylist of your own identifiers (host aliases, usernames,
#      internal hostnames). Supply it out-of-band so it never lands in the repo:
#        - $PRIVACY_DENYLIST        newline-separated strings (e.g. a CI secret)
#        - $PRIVACY_DENYLIST_FILE   file with one string per line
#          (default: $HOME/.config/miao-term/privacy-denylist if it exists)
#
# Docs are expected to use placeholders such as `<windows-host>`, `%USERPROFILE%`,
# `$HOME`, so real values appearing in the tree are a finding.
#
# Exits 0 when clean, 1 when something was found. It never aborts midway: grep
# and xargs return non-zero for "no match" / "some child failed", which must not
# be mistaken for an error.

set -u

if [ "$#" -gt 0 ]; then
    files=$(printf '%s\n' "$@")
else
    files=$(git ls-files)
fi

if [ -z "$files" ]; then
    echo "check-privacy: ok (no files)"
    exit 0
fi

# Example names that are fine in documentation.
allow_names='me|you|user|someone|yourname|example'

found=0

# Run a grep across the file list, printing matches. Always succeeds so callers
# can collect output safely.
scan_files() {
    printf '%s\n' "$files" | xargs grep "$@" -- 2>/dev/null || true
}

report() {
    if [ -n "$1" ]; then
        printf '%s\n' "$1"
        found=1
    fi
}

# macOS/Linux home paths, e.g. /Users/<you>/… or /home/<you>/… (example names ok)
report "$(scan_files -nIE "/(Users|home)/[A-Za-z0-9_.-]+/" | grep -vE "/(Users|home)/($allow_names)/" || true)"

# Windows home paths, e.g. C:\Users\<you> (example names ok)
report "$(scan_files -nIE '[Cc]:\\Users\\[A-Za-z0-9_.-]+' | grep -vE "[Cc]:\\\\Users\\\\($allow_names)" || true)"

# Private / RFC1918 addresses (all four octets, and not preceded by a digit or dot
# so version strings like 0.10.1 and floats like 10.0 are ignored).
report "$(scan_files -nIE '(^|[^0-9.])(10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|192\.168\.[0-9]{1,3}\.[0-9]{1,3}|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3})' || true)"

# Common credential shapes.
report "$(scan_files -nIE 'glpat-[A-Za-z0-9_-]{10,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-|-----BEGIN [A-Z ]*PRIVATE KEY-----' || true)"

# Your own identifiers, out-of-band.
denylist=${PRIVACY_DENYLIST:-}
denylist_file=${PRIVACY_DENYLIST_FILE:-"$HOME/.config/miao-term/privacy-denylist"}
if [ -z "$denylist" ] && [ -f "$denylist_file" ]; then
    denylist=$(cat "$denylist_file")
fi
if [ -n "$denylist" ]; then
    hits=$(printf '%s\n' "$denylist" | while IFS= read -r term; do
        [ -n "$term" ] || continue
        printf '%s\n' "$files" | xargs grep -nIF -e "$term" -- 2>/dev/null || true
    done)
    report "$hits"
    [ -n "$hits" ] && echo "   ^ matches a denylisted identifier" >&2
fi

if [ "$found" -ne 0 ]; then
    echo "" >&2
    echo "check-privacy: FAILED — remove machine-specific details or secrets above." >&2
    echo "Use placeholders in docs (<windows-host>, %USERPROFILE%, \$HOME) and keep" >&2
    echo "real host aliases/paths out of the repository (see AGENTS.md)." >&2
    exit 1
fi

echo "check-privacy: ok"
exit 0
