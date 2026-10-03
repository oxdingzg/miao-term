# ADR 0041 — Panes survive an app restart (per-pane PTY host)

> 简体中文版: [`0041-pty-host.zh-CN.md`](0041-pty-host.zh-CN.md)

Status: proposed.

## Context

Updating mtty kills whatever runs in its panes. A long Claude Code, Codex or
miao session started in mtty ends when *Update and Relaunch* quits the app;
the relaunched app only restores the layout, the working directory, an ANSI
copy of the screen and an offer to run the last command again
(`restore_pane_contents`). `docs/PRODUCT.md` already says live processes "do
not survive a quit (that needs a separate PTY daemon)".

What ties a child to the app today:

- `Terminal` (`crates/term-core/src/term.rs`) owns the `portable-pty` master,
  the writer and the child. portable-pty starts each child in its own session
  with the PTY as controlling terminal.
- `Cmd::Quit` (menu, Dock, logout, and the update path via `install_update`)
  ends with `std::process::exit(0)`. The kernel then closes the master, the
  PTY hangs up and the shell gets `SIGHUP`. Closing the window or a pane runs
  `Drop for Terminal`, which kills the child explicitly (`taskkill /T /F` on
  Windows, ADR 0027).
- The MTP control socket (`term-mtp`) is a thread in the app. Pane ids are a
  per-run counter (`pane0`, `pane1`, …), exported to the child as
  `MTTY_PANE_ID`.

Not keeping the master open is enough to lose the child, so the master has to
live in a process that outlives the app.

## Decision

### 1. One small host process per pane

Each local terminal pane is served by its own **PTY host**, `mtty-ptyhost`, in
the manner of `dtach`. The app is the host's client. The host:

- spawns the child on a PTY it owns (portable-pty, as today) and keeps the
  master open while the child runs;
- accepts one client at a time over a private socket and relays output, input,
  resize and signals;
- keeps an **output ring** (default 8 MiB) addressed by the absolute byte
  offset since the child started;
- does **no terminal emulation**. It never parses output or answers terminal
  queries.

Why per pane rather than one daemon for all panes:

- **Upgrades.** A host never needs to be replaced. Old hosts end with their
  children; new panes start new hosts. A single daemon would either keep the
  old version alive indefinitely or need its own hand-off.
- **Isolation.** A crash in one host takes one pane, not all of them.
- **Size.** The protocol is small enough to freeze (section 4).

The cost is one extra process per pane (a few MB resident; no GPU, font or UI
code is linked).

`mtty-ptyhost` is a separate small binary from a new leaf crate
`term-ptyhost` (protocol, host, client; depends on `portable-pty`, not on the
engine). It ships in every package next to `mtty-cli`.

Before spawning, the app copies the host binary into a versioned per-user
directory (`<data dir>/mtty/ptyhost/<version>/`) once, and runs it from there.
Three cases make running it from the install location unsafe:

- An AppImage's mount disappears when the app exits.
- An MSI cannot replace an executable that is running.
- The macOS update moves the old `.app` aside and deletes it.

Versions that no host uses any more are removed at startup.

### 2. The app keeps the screen; reattach replays from an offset

The terminal model stays in the app (`ATerm`, unchanged rendering, selection
and search). A hosted `Terminal` uses a host client as its byte stream instead
of a local PTY: a third backend next to the PTY and `from_pipe`.

The state survives in two halves:

1. **The app's snapshot.** The existing scrollback snapshot is extended to a
   full **state snapshot**:
   - both screens, with the alternate screen marked;
   - cursor position, style and visibility, and the scroll region;
   - the modes that change input or output: application cursor and keypad,
     bracketed paste, mouse tracking and its encoding, focus reporting, the
     kitty keyboard flags and the origin mode;
   - the current SGR, the title and the OSC 7 working directory;
   - **the host offset it covers.**

   It is written atomically at detach and by the existing once-a-minute save.
2. **The host's ring.** It holds everything after that offset.

On attach the app loads the snapshot and asks the host to replay from its
offset, then streams live output.

If the ring no longer reaches that offset (more than 8 MiB while detached, or
a crash with an old snapshot), the app:

- loads the snapshot;
- prints a dim `[mtty]` note that output was truncated;
- replays what the ring still holds after the next newline;
- toggles the size so `SIGWINCH` makes full-screen programs, Claude Code
  included, redraw.

While no client is attached, terminal queries (DA, OSC 10/11 colour, cursor
position) get no reply. A program that waits for one sees its own timeout;
this only matters at program start-up, which rarely coincides with a restart.

### 3. Lifecycle per exit path

| Event | Panes |
|---|---|
| Close a pane or tab | The app sends `Kill`; the host hangs up the child (Unix: `SIGHUP` to its process group, then `SIGKILL` after 250 ms; Windows: the ADR 0027 tree kill moves into the host) and exits. |
| Quit (menu, ⌘Q, Dock, logout) | `keep-sessions-on-quit = false` (default): as today, every pane is killed. `true`: every pane detaches (tmux-like). |
| **Update and Relaunch** | Always detaches. The app saves state snapshots, sends `Detach` to every host, then runs the install helper; the relaunched app restores the session and reattaches. |
| App crash | Hosts see the connection close and keep running detached; the next launch reattaches using the periodic snapshot (section 2's fallback covers the gap). |
| Child exits while detached | The host keeps the exit status and the ring until a client collects them or the timeout expires; the app shows the final output and `[exited]`. |
| No client for `detached-timeout` (default 24 h) | The host hangs up the child and exits, so a failed update or a crash cannot leak processes forever. |

On startup, hosts the session does not name (left over from a crash after the
session file was last written) are listed under *Recovered sessions* to
attach or end, rather than adopted silently.

### 4. Protocol (frozen v1)

Length-prefixed binary frames `type: u8, len: u32 LE, payload` over a Unix
socket or a Windows named pipe.

Client → host:

- `Hello{proto, client_version}`
- `Attach{from_offset}`
- `Input(bytes)`
- `Resize{cols, rows, px_w, px_h}`
- `Kill`
- `Detach`
- `Query(Foreground | Cwd)`

Host → client:

- `Welcome{proto, host_version, child_pid, started_at, caps}`
- `Output{offset, bytes}`
- `Truncated{oldest_offset}`
- `Exited{status, at_offset}`
- `Answer(...)`
- `Detached` (another client took over)

Hosts outlive app versions, so the compatibility rule is strict:

- v1 is never changed.
- Additions are optional capability bits in `Welcome.caps`.
- Every app version keeps a client for every protocol version it can meet.
- Golden-frame tests pin the encoding.

The foreground-process and cwd probes the app runs today (`tcgetpgrp`,
`process_cwd`) move into the host, which holds the master fd.

### 5. Identity, discovery and the environment

- The session file records, per hosted pane, `host: {id, socket}`. `id` is a
  random 128-bit name.
- Sockets live in a `0700` directory:
  - Linux: `$XDG_RUNTIME_DIR/mtty/hosts/<id>.sock`
  - macOS: `$TMPDIR`, kept short for the ~100-byte limit.
- The host checks the peer's uid (`getpeereid`/`SO_PEERCRED`).
- On Windows the pipe is `\\.\pipe\mtty-host-<user sid>-<id>`, with a DACL
  that grants only the current user, and it rejects remote clients.
- Each host also writes `<id>.json` (pids, version, start time) for the
  recovered-sessions list.
- A reattached pane **keeps its pane id**, because the child's environment
  still says `MTTY_PANE_ID=pane3`. The id counter starts above the largest
  restored id.
- `MTTY_SOCKET` is the fixed per-user path, so agent hooks reach the new app
  once it is up.

### 6. Starting a host

- The app spawns the host detached from itself:
  - Unix: `setsid`, stdio on `/dev/null`, `SIGHUP` ignored.
  - Windows: `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP |
    CREATE_BREAKAWAY_FROM_JOB`, so it leaves the app's job object.
- The host signals ready by accepting the first `Hello`. If that takes longer
  than 500 ms, or the host fails, the pane falls back to the in-process PTY
  with a notice. A pane is never lost to the feature.
- `pty-host = false` turns hosting off entirely: today's behaviour.
- Platform behaviour still to be confirmed in P1 on real desktops:
  - that a GNOME/KDE app scope does not take the host down when the app
    exits;
  - that launchd does not, after a LaunchServices relaunch;
  - that a Windows host survives the MSI's `RestartManager`.

### 7. Control plane during a restart

The MTP socket stays in the app, so it is absent for the second or two of a
relaunch.

- `mtty-cli` and the shell or agent hooks treat a refused connection as
  *app restarting*. They retry for up to 5 s (`--wait`), and the hooks drop the
  event after that instead of failing the agent.
- Events that happen while the app is down (an agent finishing, say) are not
  replayed. The pane's own output still holds them.

### 8. Performance

Hosting adds one socket hop: one more copy and wake-up on output, and one on
input.

- The performance gate (ADR 0018/0023) gets hosted variants of the
  input-latency and throughput tests, under the same budgets.
- Output frames are batched by the host; the 16 ms input-latency target is the
  limit.
- If hosted throughput cannot meet the budget, hosting stays opt-in.

## Phases

| Phase | Content | Acceptance |
|---|---|---|
| P0 | State snapshot (modes, both screens, offset) replaces the ANSI copy for local restore | Round-trip tests: snapshot → fresh `ATerm` → identical grid and modes, vim/less/Claude Code captures on a macOS desktop |
| P1 | `term-ptyhost` crate, host binary, Unix client backend, `pty-host` setting (default off), versioned copy | Host integration tests (spawn, detach, reattach from offset, truncation, kill, timeout, peer check); kill -9 the app and reattach on a macOS and a Linux desktop |
| P2 | Update path detaches and reattaches; pane ids preserved; recovered-sessions list; `mtty-cli --wait` | Real update from the previous release on a macOS desktop (.app) and a Linux desktop (AppImage) with Claude Code mid-turn: it keeps running and the pane shows its output after relaunch |
| P3 | Windows host (ConPTY, named pipe, job breakaway, versioned copy outside `Program Files`) | Same update test with the MSI on a Windows desktop |
| P4 | `keep-sessions-on-quit`, `detached-timeout`, hosted performance-gate tests | Gate green with hosting on |
| P5 | `pty-host` on by default after a release of soak | — |

SSH, serial, Telnet and TCP panes are out of scope. Their state is remote and
the SSH *keep the shell in tmux* option already covers it. Surviving logout or
reboot is also out of scope.

## Consequences

- Updating no longer interrupts running agents. A GUI crash no longer kills
  the panes either.
- One extra process per local pane and a frozen wire protocol that must stay
  supported as long as an old host can be running.
- The state snapshot (P0) improves today's restore even with hosting off:
  full-screen programs and input modes come back correctly.
- Packaging gains one binary on four targets (`release.yml`, MSI, AppImage,
  deb, `.app`).
- `docs/ARCHITECTURE.md` gains the host process and its crate once this is
  accepted; D6's platform boundaries extend to `term-ptyhost`.

## Open questions

1. Default for an ordinary quit: kill (as today) or keep (tmux-like)?
2. `detached-timeout` default (24 h proposed).
3. Ring size per pane (8 MiB proposed; total memory grows with pane count).
