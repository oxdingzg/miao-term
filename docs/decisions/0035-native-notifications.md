# ADR 0035 — Native system notifications

Status: accepted.

## Context

ADR 0010 fires a system notification when an agent pane transitions into
`awaiting` or `error` while it is not focused. The backends chosen then were
command-line tools: `osascript` on macOS, `notify-send` on Linux and BurntToast
on Windows, all in `crates/term-ui/src/agentloop.rs`.

On macOS, `osascript … display notification` is attributed to **Script Editor**,
because `/usr/bin/osascript` belongs to that app's bundle. The banner shows the
wrong icon and name, cannot be clicked, and activating it can bring Script
Editor's *open file* panel forward. Windows BurntToast needs a PowerShell module
that may be absent. Linux `notify-send` is fine.

## Decision

Add `crates/term-platform` (`mtty-platform`) for native system
integration, starting with notifications. `crates/term-ui/src/agentloop.rs`
re-exports `notify`/`alert` from it, so hosts keep calling `agentloop::notify`
(ADR 0010) while the platform code leaves the host-agnostic crate.

On macOS, when the process runs from an app bundle, `notify` posts through
`UNUserNotificationCenter` (typed objc2 bindings). A
`UNUserNotificationCenterDelegate` shows the banner even while mtty is
frontmost, and authorization is requested on each post (the system only prompts
the first time, and the banner is posted from the completion handler so a
first-run prompt cannot swallow it). When there is no bundle identifier — the
raw binary, `cargo run`, or a headless host — `notify` reports failure and the
caller keeps the `osascript` fallback, so behaviour is unchanged outside a
bundle.

Linux posts through the freedesktop D-Bus service (`zbus`) and Windows through
WinRT toasts under mtty's AppUserModelID; both keep their command-line fallback.
`alert` (the blocking startup-failure dialog) still stays on
`osascript`/zenity/PowerShell: it runs before a window exists, and
`display alert` does not produce the Script Editor open panel.

## Consequences

- macOS notifications belong to mtty, can be clicked, and no longer activate
  Script Editor.
- `term-ui` stays host-agnostic: `objc2`, `block2` and
  `objc2-user-notifications` are confined to `mtty-platform`'s macOS
  target.
- The native path needs an app bundle. `scripts/package-macos.sh` already builds
  and ad-hoc signs one, so no certificate is required to test or to run a local
  build; a Developer ID + notarization is only needed for distribution.
- The notification center's `delegate` property is weak, so one delegate object
  is intentionally leaked for the process lifetime.
- If the user denies notification permission, banners are dropped silently; the
  in-app attention marks (ADR 0010 addendum) still work.
