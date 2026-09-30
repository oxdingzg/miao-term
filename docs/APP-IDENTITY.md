# One application: miaotty

[简体中文](APP-IDENTITY.zh-CN.md)

## Name and implementation

The application is **miaotty**, the command is `miaotty`, and the companion CLI
is `miaotty-cli`. The repository and embeddable engine keep the name **miao-term**.
`miaoterm` would describe a terminal more literally, but adds no implementation
benefit and would rename the CLI, URL scheme, configuration and installer
identities. We retain the established miaotty identity.

`miaotty-app/src/main.rs` now launches the native `miao-term-widget` library.
The former eframe application and separate `miaotty-native` binary are retired.
This supersedes the dual-host phase described in ADRs 0030/0031.
`miaotty --version` prints `miaotty <version> (native)` so checks can verify the
actual implementation instead of inferring it from the filename.

Every release contains one GUI application and its CLI:

- macOS: one `miaotty.app`, executable `miaotty`, bundle ID `io.miaotty.terminal`;
- Linux: `miaotty` and `miaotty-cli` in tar/DEB/AppImage;
- Windows: `miaotty.exe` and `miaotty-cli.exe` in zip/MSI.

No `miaotty-native.app`, standalone native archive, or second GUI executable is
included in current release recipes. The four-runner release workflow remains
the release gate; a local macOS build does not replace the Linux/Windows gates.

## Existing state

The existing `miaotty` configuration directory, CLI protocol, socket and URL
scheme remain compatible. The unified native app writes `session.json`,
`queue.json` and `window` under `$XDG_CONFIG_HOME/miaotty` (default
`~/.config/miaotty`). Reads use this priority:

1. canonical state in the configuration directory;
2. former native `native-session.json`, `native-queue.json`, `native-window`;
3. if no native session exists, former eframe
   `$XDG_DATA_HOME/miaotty/session.json` (default `~/.local/share/miaotty`).

Legacy source files remain intact. The indexed eframe layout and pane focus are
converted to native IDs, including split directions/ratios, cwd, tab title,
prefix, mark, group, active tab and recent files. Recipes use the same conversion.
If both former apps have saved sessions, native state takes precedence; the
former eframe session is retained rather than silently merged.

This restores layouts and starts new shells; it does not preserve running
processes. Configuration imports and recipes retain their existing locations.

## Build, install and verify

```sh
cargo build --release -p miaotty-app -p miaotty-cli
scripts/package-macos.sh
python3 scripts/check-macos-bundle.py dist/miaotty.app
python3 scripts/smoke-hosts.py --bundle dist/miaotty.app
bash scripts/install-macos.sh
```

The macOS installer replaces the known `miaotty.app`, retires the known former
`miaotty-native.app`, and archives old bundles in a local temporary directory.
It does not remove configuration or stop running sessions. Fully quit the old
application and reopen miaotty to switch a currently running process.

The desktop smoke checks the **packaged** executable and bundled CLI, including
legacy native split-session migration, pane focus/close, shell commands, file
read/write, history, agent state, view/edit dispatch and a fresh GPU screenshot.
`MIAOTTY_SHOT_AFTER=<seconds>` writes `/tmp/miaotty_shot.ppm` and exits. Former
native screenshot environment variables remain compatibility aliases.

The original two-host audit in `UI-AUDIT.md` is historical evidence; the current
smoke targets only the unified native miaotty application. The native app keeps
its version-check UI. The removed eframe app's automatic update-download/install
UI is not advertised as a feature of this application.
