# Windows dev / verification

How to build, test and smoke-test mtty on a Windows machine. English default;
keep [`WINDOWS-DEV.zh-CN.md`](WINDOWS-DEV.zh-CN.md) in sync.

Nothing here is host-specific: use your own ssh alias and paths. Which machine we
use, and how to reach it, is kept in private notes, not in this repository.

## What the machine needs

- Windows 10/11 x64.
- **MSVC build tools already present** — either Visual Studio 2022 (any edition)
  or the standalone Build Tools install. `cl.exe` / `link.exe` must exist; rustc
  finds them automatically. No Visual Studio install is required otherwise.
- Rust via [rustup](https://rustup.rs) with the default host toolchain
  (`stable-x86_64-pc-windows-msvc`). `rustfmt` + `clippy` components are handy.
- A checkout somewhere under `%USERPROFILE%` (e.g. `%USERPROFILE%\miao-term`).

## Use the MSVC toolchain, not `-gnu`

Installing `stable-x86_64-pc-windows-gnu` *succeeds*, and its bundled
`dlltool.exe` runs by hand, but rustc's call to it while linking `windows-sys`
fails:

```
error: dlltool could not create import library with …\self-contained\dlltool.exe -d … -D kernel32.dll …
       …\dlltool.exe: CreateProcess…
```

So keep `rust-toolchain.toml` pinning `channel = "stable"` (the host default),
not a `-gnu` triple. CI's `windows-latest` runner uses MSVC for the same reason.

## Get the source onto the machine

If the machine has no GitHub credentials, push a snapshot instead of cloning:

```sh
# from the machine that has the checkout
tar -czf /tmp/miao-term-src.tgz --exclude target --exclude .git --exclude dist miao-term
scp /tmp/miao-term-src.tgz <windows-host>:miao-term-src.tgz
```

```powershell
# on the Windows host
Remove-Item -Recurse -Force "$env:USERPROFILE\miao-term" -ErrorAction SilentlyContinue
& tar.exe -xzf "$env:USERPROFILE\miao-term-src.tgz" -C "$env:USERPROFILE"
```

## Verify

```powershell
powershell -ExecutionPolicy Bypass -File "$env:USERPROFILE\miao-term\scripts\windows-verify.ps1"
```

It builds `mtty` + `mtty-cli`, runs the engine/MTP tests, then starts the
app and drives it over MTP (`ping`, `file write`, `file read`, `view`).

Last run (this project's Windows box, 2026-09-29): build OK, the engine/MTP
tests pass, the app starts, `ping` advertises `app.view.write` / `file.read` /
`file.write` (plus the agent/history caps), and the `file write` → `file read` →
`view` round-trip works. This runs over ssh without a desktop session; IME/GUI
still need an interactive desktop.

## Over ssh, without a desktop session

ssh runs in a non-interactive session, so:

- Long builds: a **`schtasks` task run as `SYSTEM`** with explicit
  `RUSTUP_HOME` / `CARGO_HOME` survives the ssh disconnect; a plain
  `Start-Process` may be killed when the session ends. `/tr` is capped at 261
  characters, so put the command in a `.cmd` and pass that path.
- The GUI cannot be seen and **IME cannot be exercised** from such a session —
  that needs an interactive desktop (RDP, or a task in the logged-on session).
- `conpty_resize_updates_screen_and_pty` used to appear to hang (a full run took
  ~4 minutes); the cause was teardown, not the test — `ClosePseudoConsole` blocks
  while the shell is alive. Fixed in ADR 0027: the engine kills the tree, drains
  and closes off-thread, and the test exits its shell. Both ConPTY tests now
  finish in ~1 s on real hardware.

## CI

`.github/workflows/ci.yml` runs `cargo test --workspace` on `windows-latest` (so
the `#[cfg(windows)]` ConPTY test runs there too), but only on non-push events
(PRs, the nightly run, manual dispatch); `cargo check --workspace` runs in the
Linux and macOS jobs. A `perf` job also runs on `ubuntu-latest`. Use a real
Windows host when you need a GUI or an IME check; it does not replace CI.
