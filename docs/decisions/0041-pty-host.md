# ADR 0041 — Panes survive an app restart (per-pane PTY host)

> 简体中文版: [`0041-pty-host.zh-CN.md`](0041-pty-host.zh-CN.md)

Status: accepted (2026-10-03, with the proposed defaults: an ordinary quit
ends the panes, `detached-timeout` 24 h, an 8 MiB ring). Revised after a
review against `alacritty_terminal` 0.25 and the current restore code; see
*Review findings* at the end.

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
- does **no screen emulation** and never answers terminal queries. It runs
  only a small scanner over the output (section 4) that finds escape-sequence
  and UTF-8 boundaries and tracks the private modes (DECSET/DECRST, the kitty
  keyboard flags, the alternate screen).

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

1. **The app's state snapshot**, written atomically at detach and by the
   existing once-a-minute save. It must reproduce the screen *exactly*, not
   just its look: Claude Code (Ink) and every full-screen program redraw with
   relative cursor moves, so a reflowed screen or a misplaced cursor would make
   the replayed bytes paint over the wrong rows. It holds:
   - the size it was taken at;
   - history as SGR text (the cursor cannot reach history, so reflowing it
     on a later resize is harmless);
   - the visible rows of the main screen, and of the alternate screen when
     active, as exact cells (`Cell` is `Clone` + `Serialize` with public
     fields: character, colours, flags including `WRAPLINE`, zero-width
     characters, hyperlink);
   - each screen's cursor and saved cursor (position, template cell,
     charsets, pending wrap; all public fields of `Grid`);
   - `TermMode` (cursor keys, keypad, bracketed paste, mouse modes and
     encoding, focus reporting, origin, insert, line wrap, the current kitty
     keyboard flags) and the cursor style, title and OSC 7 directory;
   - **the host offset it covers**, which is always the end of a whole
     `Output` frame and therefore a sequence boundary (section 4). The app
     cannot find boundaries itself: vte keeps its parser state private.

   Reading the main screen under an alternate screen goes through
   `swap_alt`, which clears the alternate screen; the snapshot copies the
   alternate screen's cells first and writes them back, so it no longer
   destroys anything and can run while the program keeps drawing.
2. **The host's ring**, which holds the output after that offset, evicted in
   whole frames.

On attach the app builds a fresh screen **at the snapshot's size**, loads the
snapshot, replays the ring from its offset, and only then resizes the pane to
its current size.

**Replies are muted while loading and replaying.** The screen model answers
queries it parses (DA, cursor position, OSC 10/11 colours) by writing to the
PTY; replayed output contains the program's old queries, and answering them
again would type escape sequences into the running program.

**Some state cannot be read back.** `alacritty_terminal` 0.25 keeps the
scroll region, tab stops, the title stack and the depth of the kitty keyboard
stack private. After every reattach the app therefore toggles the pane size
once, so `SIGWINCH` makes full-screen programs and Claude Code redraw (which
sets their scroll region again). Inline images that are not in the replayed
range are not restored.

If the ring no longer reaches the snapshot's offset (more than 8 MiB while
detached, or a crash with an old snapshot), the app loads the snapshot,
applies the private modes the host reports with `Truncated` (they may have
changed in the lost bytes), prints a dim `[mtty]` note that output was
truncated, replays the ring from its oldest frame, and nudges the size as
above.

While no client is attached, terminal queries (DA, OSC 10/11 colour, cursor
position) get no reply. A program that waits for one sees its own timeout;
this only matters at program start-up, which rarely coincides with a restart.

### 3. Lifecycle per exit path

| Event | Panes |
|---|---|
| Close a pane or tab | The app sends `Kill`; the host hangs up the child (Unix: `SIGHUP` to its process group, then `SIGKILL` after 250 ms; Windows: the ADR 0027 tree kill moves into the host) and exits. |
| Quit (menu, ⌘Q, Dock, logout) | `keep-sessions-on-quit = false` (default): as today, every pane is killed. The app must send `Kill` to every host before `process::exit`, because exiting alone would now leave them running. `true`: every pane detaches (tmux-like). |
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

What to run (program, arguments, environment including `MTTY_PANE_ID`,
working directory, initial size) is given to the host on its command line and
environment when it is launched, not in the protocol, which keeps v1 to the
running session.

The host cuts `Output` frames only at sequence boundaries: its scanner holds
back an incomplete escape sequence or UTF-8 character until it completes, and
flushes it anyway after 5 ms or 4 KiB so a stray `ESC` cannot stall output.
Offsets are `u64`.

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
- `Truncated{oldest_offset, modes}` (the scanner's private-mode state)
- `Exited{status, at_offset}`
- `Answer(...)`
- `Detached` (another client took over)

Hosts outlive app versions, so the compatibility rule is strict:

- v1 is never changed.
- Additions are optional capability bits in `Welcome.caps`.
- Every app version keeps a client for every protocol version it can meet.
- Golden-frame tests pin the encoding.

The foreground-process and cwd probes the app runs today (`tcgetpgrp`,
`process_cwd`) move into the host, which holds the master fd. They become
asynchronous: the app caches the last answer, and the session save at quit
uses the cache instead of blocking on the host.

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
- Starting a host never blocks the UI thread: the pane appears at once,
  input typed before the host's `Welcome` is queued, and the performance
  gate's first-frame budget is unchanged.
- If `Welcome` takes longer than 500 ms, or the host fails, the pane falls
  back to the in-process PTY with a notice. A pane is never lost to the
  feature.
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
| P0 | State snapshot and restore in `term-core` (`ATerm::snapshot_state` / `restore_state`), non-destructive under the alternate screen, replies muted while restoring; the periodic save includes full-screen panes | Equivalence tests: for recorded streams *A* and *B* (shell, vim-like alternate screen, Ink-style redraws, wide characters, wrapped lines), restoring a snapshot taken after *A* and then feeding *B* yields the same cells, cursor and modes as feeding *A + B* without interruption; vim/less/Claude Code captures on a macOS desktop |
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
- With hosting off, a restored pane runs a *new* shell, so only the snapshot's
  content is shown there; its modes are never applied (a new shell is not in
  vim's alternate screen or mouse mode). P0 still helps that path: the
  snapshot no longer destroys the alternate screen, so the periodic save
  covers panes running a full-screen program.
- Packaging gains one binary on four targets (`release.yml`, MSI, AppImage,
  deb, `.app`).
- `docs/ARCHITECTURE.md` gains the host process and its crate once this is
  accepted; D6's platform boundaries extend to `term-ptyhost`.

## Addendum: P1 as built (2026-10-03)

- **Protocol v1** as frozen in `crates/term-ptyhost/src/proto.rs` (golden-frame
  tests): `Hello`, `Attach`, `Input`, `Resize`, `Kill`, `Detach` to the host;
  `Welcome`, `Output{offset, boundary}`, `Truncated{oldest, modes}`,
  `Live{offset}`, `Exited`, `Detached` from it. `Live` marks the end of the
  replay, so the app knows when to answer queries again and resize. There is
  no `Query`/`Answer`: the app reads the foreground process group from the
  kernel (`tpgid` via `proc_pidinfo` on macOS, `/proc/<pid>/stat` on Linux)
  using the child pid from `Welcome`, so no round trip is needed.
- `pty-host = true` in `config.toml` turns it on (default off). The host
  binary `mtty-ptyhost` must sit next to `mtty`; it is copied to
  `<data dir>/ptyhost/<version>/` before use. Packaging it is part of P2.
- A pane whose host never answers within 2 s runs its shell in the app with
  a dim note; a host that cannot be spawned at all falls back silently.
- Session restore reattaches before anything else and keeps the pane id; the
  minute-by-minute save writes `<pane>.host.json` (state + offset) next to
  the ANSI copy. Without a snapshot the host's whole ring is replayed.
- Verified: host tests (attach, offset resume, truncation with modes, exit
  while detached, timeout, takeover, boundaries), a hosted `Terminal` across a
  simulated restart (no duplicated output, replayed queries not answered,
  foreground known), and `kill -9` of the app on a macOS and a Linux desktop
  with output continuing without a gap and the pane id kept.

## Addendum: P2 as built (2026-10-03)

- **Update and Relaunch** quits keeping sessions: each hosted pane's state
  and offset are saved (`<pane>.host.json`), its host is detached, and the
  relaunched app reattaches. Panes without a host end as on any quit.
- A palette command, *Relaunch, Keeping Programs Running* (shown when
  `pty-host` is on), runs the same path without an update. It is how the
  path is tested end to end: an update needs a release-signed artifact.
- `mtty-ptyhost` ships in the macOS bundle, the Linux tarball, AppImage and
  deb (`release.yml`, `scripts/package-macos.sh`, the deb assets).
- Hosts running that no restored pane took back (a crash before the session
  was saved) are announced at startup and listed in the palette as
  *Reattach / End Running Program*.
- `mtty-cli` retries a refused connection for 5 s when it runs inside a pane
  (`MTTY_PANE_ID` is set), so agent hooks reach the relaunched app;
  `--wait SECS` sets the time anywhere.
- Verified on a macOS and a Linux desktop: the relaunch keeps a running loop
  going without a gap, the old app exits, the snapshot is written and used,
  the pane id is kept; a host orphaned by `kill -9` is reattached from the
  palette. A real update from one release to the next is checked when the
  next release ships.

## Review findings (2026-10-03)

The first draft was checked against `alacritty_terminal` 0.25.1, vte 0.15 and
the restore code before any implementation. Changes made above:

1. **Exact screens.** The draft reused the ANSI scrollback copy, which joins
   wrapped rows, drops trailing blank rows and loses the cursor. Replayed
   relative cursor moves would then corrupt the screen. The snapshot now keeps
   visible rows as exact cells and the cursors, and restores at its own size.
2. **Muted replies.** Replaying old output through the screen model would
   answer old queries into the live program.
3. **Sequence boundaries.** A snapshot offset in the middle of an escape
   sequence or UTF-8 character would corrupt the replay, and the app cannot
   see vte's parser state; the host frames output at boundaries.
4. **Modes on a new shell.** The draft claimed the snapshot would restore
   modes for today's (process-less) restore; that would leave a new shell in
   a dead program's modes.
5. **Unreadable state** (scroll region, tab stops, keyboard-stack depth):
   documented, and covered by a size nudge after every reattach rather than
   only after truncation.
6. **Lost modes on truncation:** reported by the host's scanner.
7. **Missing spawn description, blocking start-up, synchronous foreground
   probe at quit, and quit leaving hosts alive:** specified above.

## Settled questions

The defaults were accepted: an ordinary quit ends the panes, `detached-timeout`
is 24 h, and each host keeps an 8 MiB ring.
