# ADR 0026 — Native Wayland/X11 global shortcut (portal)

Status: accepted.

## Context

ADR 0019 gave a system-wide hotkey on macOS/Windows via `global-hotkey`, but on
Linux that crate only speaks X11; Wayland compositors ignore it. The
`GlobalShortcuts` XDG desktop portal is the compositor-agnostic mechanism, and
its trigger is owned by the compositor/user rather than grabbed by us.

## Decision

On Linux, `crates/mtty-ui/src/hotkey.rs` registers the Quick Terminal through the
portal using `ashpd` (MIT):

1. A dedicated thread runs a current-thread tokio runtime (`enable_all`).
2. `GlobalShortcuts::new` → `create_session` → `bind_shortcuts(["quick"])`; the
   portal may prompt the user, and it decides the actual key combo.
3. `receive_activated()` is polled with `poll_fn` + `Pin::new(..).poll_next`
   (no `futures-util` dependency: the `Stream` trait is re-exported through
   `ashpd::zbus::export::futures_core`), and an activation of `quick` flips the
   pending flag and wakes the UI.

If the portal is unavailable (no `xdg-desktop-portal`, headless, or the user
declines), `register` returns `None` and the app logs it — the compositor
binding snippets from ADR 0019 remain as an alternative.

Dependencies are Linux-target-only (`ashpd` + a small `tokio`), so macOS and
Windows builds and the macOS dependency set are unchanged.

## Consequences

- Wayland users get a global Quick Terminal shortcut through the standard
  portal, and X11 users go through the same portal (which forwards to X11).
- The trigger is chosen in the portal UI, so the configured accelerator is only
  a hint on Linux; this is a portal property, not a bug.
- The code path is Linux-only and cannot be exercised on the maintainer's
  machine; it is compile-checked by CI's `ubuntu-latest` job and by a local
  cross-`cargo check --target x86_64-unknown-linux-gnu` probe.
