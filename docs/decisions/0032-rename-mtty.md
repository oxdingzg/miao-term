# ADR 0032 — The application is renamed mtty

> 简体中文: [`0032-rename-mtty.zh-CN.md`](0032-rename-mtty.zh-CN.md)

Status: accepted. Supersedes the "keep the miaotty name" conclusion in
[APP-IDENTITY](../APP-IDENTITY.md).

## Context

The product name is **mtty** (site: `mtty.dev`). Until now the application,
CLI, config directory, socket, environment variables, URL scheme and packages
were all `miaotty`. Existing user state and external integrations depend on the
old name:

- The config directory `~/.config/miaotty` (`config.toml`, `views.json`,
  session, queue, recipes, hooks).
- Installed agent hook scripts call `${MIAOTTY_CLI:-miaotty-cli}` and read
  `MIAOTTY_PANE_ID`.
- miao's built-in integration reads `MIAOTTY_PANE_ID` and `MIAOTTY_CLI` (falling
  back to `miaotty-cli` on `PATH`).
- Hotkey snippets users pasted into skhd / Hammerspoon / sway call
  `miaotty --quick`.
- Older CLIs connect to `$XDG_RUNTIME_DIR/miaotty.sock` by default.

## Decision

**The new name is the only primary name; the old one is a read-only
compatibility input, kept for at least two releases.**

| Area | New | Compatibility |
|---|---|---|
| Executables | `mtty`, `mtty-cli` | `miaotty`, `miaotty-cli` symlinks in the Linux packages |
| Cargo packages / dirs | `mtty-app`, `mtty-cli` | — |
| macOS bundle | `mtty.app`, bundle ID `dev.mtty.terminal` | The installer archives and removes `miaotty.app` (`io.miaotty.terminal`) by identity |
| Config directory | `$XDG_CONFIG_HOME/mtty` | When it does not exist, the `miaotty` directory is **copied** once and kept |
| Environment | `MTTY_*` | Reads prefer `MTTY_*`, then `MIAOTTY_*`; panes **export both** |
| CLI path | In panes, `MTTY_CLI` and `MIAOTTY_CLI` both hold the absolute path of the sibling `mtty-cli` | Old hooks and miao find the CLI unchanged |
| Socket | `$XDG_RUNTIME_DIR/mtty.sock`, Windows pipe `mtty` | On Unix a `miaotty.sock` symlink is created (when absent or stale); a new CLI falls back to the old path when `mtty.sock` is missing |
| URL scheme | `mtty://` | `miaotty://` is still accepted and registered |
| ssh ControlPath | `~/.ssh/mtty-cm-%r@%h:%p` | None needed (just a new shared connection) |
| Windows MSI | Product `mtty`, install folder `mtty` | **Same UpgradeCode**, so older versions upgrade in place |
| Debian package | `mtty` | `Replaces/Conflicts/Provides: miaotty` |

The bundle ID changes now: the project is a 0.0.x pre-release, `mtty.dev` is a
domain we own, and the change only gets more expensive later. The cost is that
macOS treats it as a new application, so permissions such as notifications must
be granted again; this ADR and the release notes say so.

Migration lives in `miao-term-config` as `config_dir()` and
`migrate_legacy_config()`; every config/state path goes through it instead of
joining `"miaotty"` on its own. Environment reads go through
`miao_term_config::env()` (`term-mtp` has no such dependency and keeps an
equivalent small function).

The retired eframe app's `$XDG_DATA_HOME/miaotty/session.json` stays the last
session source and is not renamed. Historical ADRs are records and are not
rewritten.

## Consequences

- Upgraded users keep their config, sessions, recipes and hooks; the old
  directory stays, so going back to an older build works.
- miao, old hooks, old hotkey snippets (Linux) and old CLIs keep working during
  the transition.
- After two releases (around v0.0.8) dropping the `MIAOTTY_*` exports and the
  old symlinks is reconsidered in a new ADR.
- Migration has unit tests (copy, never overwrite, never delete the old
  directory, do nothing when the new one exists).
