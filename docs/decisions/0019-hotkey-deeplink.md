# ADR 0019 — Global hotkey and deep-link to a pane

Status: accepted.

## Context

Two system-level gaps: a truly system-wide Quick Terminal hotkey, and getting a
later launch (URL scheme or shortcut) to act on the *running* instance —
including focusing a specific pane — rather than opening a duplicate window.

## Decision

**One intent model** (`crates/term-ui/src/launch.rs`). `Intent` is
`Activate | Quick | Focus(pane_id) | Run(command)`, parsed from argv
(`--quick`, `--focus <id>`, `ssh://…`, `x-man-page://…`, `miaotty://quick`,
`miaotty://focus?pane=…`) and encoded/decoded for the cross-instance inbox
(`quick`, `focus\t<id>`, `run\t<cmd>`). Unit-tested round-trip.

**Global hotkey.** `crates/term-ui/src/hotkey.rs` parses an accelerator
(`cmd+shift+t`, `ctrl+alt+space`, platform-independent and tested everywhere) and
registers it via `global-hotkey` (MIT) on macOS and Windows; the dependency is
declared per-target so Linux CI does not pull X11/Wayland. The handler flips a
flag and calls `egui::Context::request_repaint`; the app polls it each frame and
toggles the Quick Terminal. Registration failure is logged, never fatal,
and the accelerator comes from `quick-terminal-hotkey` in the config (off when
unset). On Linux — and as an alternative anywhere — Settings shows paste-ready
bindings for skhd / Hammerspoon / AutoHotkey / GNOME that call `miaotty --quick`.

**Deep link.** `main` turns argv into an `Intent` and, if an instance already
answers on the MTP socket, forwards the encoded intent through the inbox
directory and exits. The running instance drains it and applies it: `Focus`
switches to the pane's tab and focuses it; `Run` opens a tab; `Quick` toggles
the Quick Terminal. A palette verb *Copy Pane ID* yields the id to target.

## Addendum (Wayland)

A native Wayland global shortcut is implemented separately (ADR 0026) through the
`GlobalShortcuts` xdg-desktop-portal. The Settings row additionally offers **sway** and **hyprland** bindings
alongside skhd/Hammerspoon/AutoHotkey/GNOME, all of which call `miaotty --quick` —
so Wayland users get the Quick Terminal through their compositor's own bind
mechanism, which is how they bind everything else anyway.

## Consequences

- One path handles argv, URL schemes and forwarded requests, so behavior is
  identical however the app is invoked.
- Global-hotkey failures (no display, grab conflict) degrade to the external
  binding snippets rather than breaking startup.
- Focusing a pane by id, toggling Quick Terminal and running a command can all
  be driven from outside the app, which is what editor/launcher integrations
  need. Grabbing keys *inside* an existing window (menu accelerators) and
  Wayland native global shortcuts remain follow-ups.
- Update: Wayland native global shortcuts landed in ADR 0026 (see the
  addendum); in-window menu accelerators remain a follow-up.
- Update: the intent/forwarding helpers moved into `miao-term-ui::launch`, and
  the native host (`miaotty-native`) now parses argv intents, forwards later
  launches through the same inbox (waking the running instance over MTP) and
  implements the Quick Terminal scratch tab on `⌘⇧T`; reopening a closed tab
  moved to `⌘⇧Z` there.
