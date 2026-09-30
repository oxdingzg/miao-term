# Installing / building

> 简体中文: [`INSTALL.zh-CN.md`](INSTALL.zh-CN.md)

## Run from source

```sh
cargo run -p miaotty-app      # or: cargo build && ./target/debug/miaotty
```

Requires the Rust stable toolchain (see `rust-toolchain.toml`). First build
compiles wgpu/glyphon and may take a few minutes.

## macOS app bundle

```sh
scripts/package-macos.sh              # release build -> dist/miaotty.app
PROFILE=debug scripts/package-macos.sh
```

Produces an ad-hoc-signed `dist/miaotty.app`. For distribution it is codesigned
and notarized automatically when the Apple secrets are configured (see
[Releases](#releases)).

## Releases

Pushing a `v*` tag runs [`.github/workflows/release.yml`](../.github/workflows/release.yml):
it builds `miaotty` + `miaotty-cli` + `miaotty-native` on macOS/Linux/Windows and
attaches a `miaotty.app` zip / Linux tarball / Windows zip to a GitHub Release.

macOS builds are ad-hoc signed by default; if the repo has `APPLE_CERT_P12` +
`APPLE_CERT_PASSWORD` + `APPLE_ID` + `APPLE_TEAM_ID` + `APPLE_APP_PASSWORD`
secrets, the workflow signs with a Developer ID, notarizes and staples instead.
Linux also builds a `.deb` (via `cargo-deb`, metadata in `miaotty-app/Cargo.toml`)
and, best-effort, an AppImage (`appimagetool`). Windows ships a zip and,
best-effort, an MSI (`cargo-wix` / WiX). The AppImage/MSI steps are
`continue-on-error` so a failure there does not fail the release. If
`WINDOWS_CERT_PFX` + `WINDOWS_CERT_PASSWORD` secrets exist, the MSI is signed
with `signtool`.

## Cross-platform installers

`dist-workspace.toml` is a [cargo-dist](https://opensource.axo.dev/cargo-dist/)
scaffold (shell/PowerShell installers + MSI). Run `dist init` then `dist build`
on a release to generate per-platform artifacts.

## Configuration

- `~/.config/miaotty/config.toml` (or `$XDG_CONFIG_HOME/miaotty/config.toml`) —
  see [`config.example.toml`](config.example.toml).
- If there is no miaotty config, ghostty `config` and alacritty
  `alacritty.toml` are imported automatically.

## Deep links (URL schemes)

`miaotty.app` registers `miaotty://`, `ssh://` and `x-man-page://` with the OS,
and the Linux `.desktop` file registers the same three. A link is translated into
a tab running the matching command.

Because the hosts share one control socket and an inbox beside it, the bundle only
has to be *installed*: when a link arrives while any host is already running — the
eframe app or `miaotty-native` — the launching process forwards the intent to it
and exits, so the running window handles the deep link. That is also how a
`miaotty-native` session receives OS links without registering anything itself;
the bundle ships both binaries (`Contents/MacOS/miaotty` and `miaotty-native`).

The Windows MSI registers `miaotty://` only, and deliberately leaves `ssh://` /
`x-man-page://` alone rather than hijacking them machine-wide.

## Shell integration

miaotty installs a zsh `ZDOTDIR` shim so the shell reports its cwd (OSC 7) and
command history; the user's dotfiles are untouched. No manual setup needed.
