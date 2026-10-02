# ADR 0036 — Host-forwarded clipboard images

Status: accepted.

## Context

A TUI running inside mtty cannot read the macOS pasteboard itself. miao does it
by shelling out to `osascript -l JavaScript`, which is slow and — because
`osascript` belongs to Script Editor — ties the app to that bundle (ADR 0035).
Linux shells out to `wl-paste`/`xclip`, Windows to PowerShell.

The host already reads the pasteboard for text pastes, and the existing
convention for an image-only clipboard is an empty bracketed paste that tells
the application to fetch the image on its own
(`crates/term-widget/src/lib.rs`, `paste_clipboard`).

## Decision

The host owns the clipboard image and hands it to the pane through a file:

- At startup mtty exports `MTTY_CLIPBOARD_FILE` — a per-process path in the temp
  directory — into the panes' environment, next to `MTTY_CLI`/`MTTY_SOCKET`
  (`export_pane_environment`).
- On paste, when the clipboard holds an image, `paste_clipboard` reads it
  (`miao-term-platform::clipboard_image`, macOS `NSPasteboard`), writes the PNG
  to that path, and sends the empty bracketed paste the application already
  treats as "read the clipboard".
- A text paste removes the file first, so a later empty paste cannot read a
  stale image.
- `clipboard_image` returns `None` on platforms without a backend yet, and the
  host then falls back to the text paste.

Applications opt in by reading the file: miao's `clipboard.read()` tries
`MTTY_CLIPBOARD_FILE` first and keeps `osascript`/`wl-paste`/PowerShell as the
fallback, so it still works in other terminals and over ssh.

Alternatives considered:
- A bracketed-paste data URI needs capability negotiation (a private DECSET) or
  a TUI that does not understand it pastes base64 as text; megabytes would also
  cross the pty.
- OSC 1337 inline images mean "display this", not "here is the image data", so
  they are the wrong channel.
- An MTP request would be the purest model (the app asks the host for the
  clipboard) but makes miao an MTP client; left as the long-term direction.

## Consequences

- On macOS, miao no longer needs `osascript` for paste inside mtty, so the
  Script Editor association is gone there.
- The path is shared by every pane (the clipboard is global) and lives for the
  process lifetime; it is overwritten or removed on each paste.
- Non-mtty terminals and ssh panes keep the existing fallbacks.
- Linux/Windows host backends (`wl-paste` / PowerShell) are follow-ups; until
  then those platforms use the application fallbacks.
