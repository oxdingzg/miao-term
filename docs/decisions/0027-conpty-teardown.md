# ADR 0027 — ConPTY teardown must not block

Status: accepted.

## Context

On real Windows, `conpty_resize_updates_screen_and_pty` appeared to hang: the
test never finished. Measurement showed the test body itself was fast (~10 ms);
the time was spent **tearing the terminal down**. Windows' `ClosePseudoConsole`
(a) waits for the client process to exit and (b) can stay blocked for minutes
when the shell is still alive or its output was never drained. That means
closing a tab, or quitting the app, could stall for minutes with a shell open —
not just a slow test.

## Decision

`Drop for Terminal` (Windows-aware) makes teardown bounded:

1. `taskkill /T /F /PID <child>` with `CREATE_NO_WINDOW`, so the whole client
   tree dies (the ConPTY client may be a wrapper whose child is the shell) and no
   console window flashes.
2. `child.kill()` + `child.wait()` (fast once the tree is gone).
3. Drain the reader channel for at most 50 ms.
4. Close the `MasterPty` **on a detached thread**, so `ClosePseudoConsole` can
   never stall the caller. The conhost it waits on exits on its own shortly
   after (observed); this leaks a thread only until then.

`master` became `Option` so `Drop` can take it. The Windows test now also sends
`exit\r\n` (like the echo test) so it tears down a shell that is already gone.

## Consequences

- Closing a pane/tab and quitting are immediate on Windows; the engine's tests
  dropped from ~240 s (or an unbounded hang) to ~1 s on real hardware.
- The Windows path spawns one short-lived `taskkill` process per pane close;
  a Job Object would be cleaner but needs a direct `windows` dependency.
- Non-Windows behaviour is unchanged apart from the detached close, which also
  protects Linux/macOS from any future blocking `close`.
