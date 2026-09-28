# Windows dev / verification machine

How to build, test and smoke-test miaotty on Windows, and the machine we use for
it. English default; keep [`WINDOWS-DEV.zh-CN.md`](WINDOWS-DEV.zh-CN.md) in sync.

## The machine

- Reachable as `ssh <windows-host>` (Windows 11, your Windows user).
- **MSVC is already present**: VS 2022 Community
  (`…\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\…\cl.exe`) and a
  2022 BuildTools install. No Visual Studio install is needed.
- rustup lives at `%USERPROFILE%\.cargo\bin`, with **`stable-x86_64-pc-windows-msvc`
  as the default** toolchain (`rustc 1.98.1`).
- The source copy is `C:\Users\<you>\miao-term` (~3 GB with `target/`).

## Use the MSVC toolchain, not `-gnu`

Installing the `stable-x86_64-pc-windows-gnu` toolchain *succeeds*, and its
bundled `dlltool.exe` runs by hand, but rustc's call to it while linking
`windows-sys` fails:

```
error: dlltool could not create import library with …\self-contained\dlltool.exe -d … -D kernel32.dll …
       …\dlltool.exe: CreateProcess…
```

So use the MSVC toolchain (which is also what CI's `windows-latest` runner uses).
`rust-toolchain.toml` must therefore pin `channel = "stable"` (the host default),
not a `-gnu` triple.

## Sync the source from macOS

There are no GitHub credentials on that host, so push a snapshot instead of
cloning:

```sh
cd <the directory containing the checkout>
tar -czf /tmp/miao-term-src.tgz --exclude target --exclude .git --exclude dist miao-term
scp /tmp/miao-term-src.tgz <windows-host>:C:/Users/<you>/miao-term-src.tgz
ssh <windows-host> 'powershell -NoProfile -Command "Remove-Item -Recurse -Force C:\Users\<you>\miao-term -ErrorAction SilentlyContinue"'
ssh <windows-host> 'powershell -NoProfile -Command "& tar.exe -xzf C:\Users\<you>\miao-term-src.tgz -C C:\Users\<you>"'
```

## Verify

On the host (or over ssh):

```powershell
powershell -ExecutionPolicy Bypass -File C:\Users\<you>\miao-term\scripts\windows-verify.ps1
```

It builds `miaotty` + `miaotty-cli`, runs the engine/MTP tests, then starts the
app and drives it over MTP (`ping`, `file write`, `file read`, `view`).

Verified this way on 2026-09-29: build OK (5m47s), `conpty_spawns_shell_and_echoes`
passes, `ping` advertises `app.view.write` / `file.read` / `file.write`, and the
file round-trip works.

## Over ssh, without a desktop session

ssh runs in a non-interactive session, so:

- Long builds: a **`schtasks` task run as `SYSTEM`** with explicit
  `RUSTUP_HOME` / `CARGO_HOME` survives the ssh disconnect; a plain
  `Start-Process` may be killed when the session ends. `/tr` is capped at 261
  characters, so put the command in a `.cmd` and pass that path.
- The GUI cannot be seen and **IME cannot be exercised** from such a session —
  that needs an interactive desktop (RDP or a task in the logged-on session).
- Known issue to watch: `conpty_resize_updates_screen_and_pty` was still running
  after 60 s (and the process later disappeared) on this machine; the echo test
  passes. Worth investigating with its author.

## CI

`.github/workflows/ci.yml` runs `cargo check` + `cargo test` on
`windows-latest` (so the `#[cfg(windows)]` ConPTY test runs there too), and a
`perf` job on `ubuntu-latest`. Use this machine when you need a *real* Windows
host or a GUI/IME check, not as a replacement for CI.
