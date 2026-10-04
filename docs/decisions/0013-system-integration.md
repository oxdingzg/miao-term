# ADR 0013 — URL schemes, Quick Terminal, i18n, update check

Status: accepted.

## Context

To be a *usable* terminal rather than a demo, miaotty must plug into the OS:
open `ssh://` and `x-man-page://` links, offer a scratch terminal, speak more
than English, and tell you when a newer build exists — all without dragging in
heavy dependencies.

## Decision

**URL schemes** (`crates/mtty-ui/src/launch.rs`). The executable inspects its argv
for `scheme://…` and turns it into a shell command for a new tab:
`ssh://[user@]host[:port][/path]` → `ssh [-p port] host` (IPv6 via `[..]`,
arguments single-quoted), `x-man-page://cmd` → `man cmd`, `miaotty://…` →
activate only. Registration lives in the macOS `Info.plist` (built by
`scripts/package-macos.sh`) and the Linux `.desktop` file shipped by
`cargo-deb`; Windows registration is documented, not automated.

**Quick Terminal.** ⌘⇧T (or the palette verb) toggles a scratch tab: created on
first use, then switching between it and the previously active tab. A true
system-wide hotkey needs an OS-specific API and is deferred.

**i18n** (`crates/mtty-ui/src/i18n.rs`). A string table with English keys: `En`
returns the key, `Zh` maps the visible chrome (Settings, Details tabs, palette
verbs, editor/composer/recipe buttons, section titles, agent-loop messages).
`language` in the config or `$LANG` selects it; anything untranslated degrades
to English by falling back to the key.

**Update check.** With `update-check-url` set, the palette verb or the Settings
button fetches that URL on a thread via `curl` (`--max-time 5`), compares the
first token to `CARGO_PKG_VERSION`, and reports *up to date* / *update
available*. A silent check also runs once on startup (config
`update-auto-check`, on by default; see ADR 0022) and a newer version is shown
in the status line. No TLS/runtime dependency is added.

## Consequences

- Integrations stay shell-level and dependency-free, so they work on all three
  platforms and cannot fail startup.
- Translation is additive: adding a language is one match arm; untranslated keys
  are visibly English rather than blank.
- True global hotkeys, deep-link focus of an existing window, and in-app update
  download/signing remain follow-ups.
- Update: true global hotkeys and deep-link focus landed in ADR 0019 (Wayland in
  ADR 0026), and in-app download, verification and install landed in ADRs
  0022/0025.
