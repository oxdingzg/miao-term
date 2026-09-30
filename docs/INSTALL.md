# Installing / building miaotty

[简体中文](INSTALL.zh-CN.md)

miaotty is the native winit/wgpu application formerly named miaotty-native.
There is one GUI executable and one CLI. See [identity and migration](APP-IDENTITY.md).

## From source

```sh
cargo run --release -p miaotty-app
cargo build --release -p miaotty-app -p miaotty-cli
./target/release/miaotty --version
```

Rust stable and the platform's wgpu/window-system libraries are required.
The first build compiles wgpu/glyphon and may take a few minutes.

## macOS

```sh
scripts/package-macos.sh
python3 scripts/smoke-hosts.py --bundle dist/miaotty.app
bash scripts/install-macos.sh
```

Produces one ad-hoc-signed `dist/miaotty.app` with `miaotty`, `miaotty-cli`, the
icon and URL schemes. Installation archives old known bundles, installs miaotty
and removes the old native launcher without removing user configuration.
`PROFILE=debug scripts/package-macos.sh` builds a development bundle.
The installed app uses the macOS system menu bar; a bare binary uses an in-window menu.

## Release packages

[release.yml](../.github/workflows/release.yml) requires Apple Silicon macOS,
Intel macOS, Linux and Windows runner builds. A `v*` tag publishes a release;
manual dispatch rehearses packaging without publishing.

- macOS: a zip containing only `miaotty.app`.
- Linux: tar, DEB and AppImage, containing `miaotty` and `miaotty-cli`.
- Windows: zip and MSI, containing `miaotty.exe` and `miaotty-cli.exe`.

Apple Developer ID signing/notarization, Windows MSI signing and minisign
artifact signatures use the optional secrets described in [RELEASE.md](RELEASE.md).
`dist-workspace.toml` remains a cargo-dist scaffold, not the active release pipeline.

## Configuration and links

Configuration is `~/.config/miaotty/config.toml`, or
`$XDG_CONFIG_HOME/miaotty/config.toml`. Ghostty/Alacritty config import and the
zsh ZDOTDIR integration retain their existing behavior. Existing native and
eframe session formats are read through the compatibility migration.

macOS and Linux register `miaotty://`, `ssh://` and `x-man-page://`.
Windows MSI registers only `miaotty://`. Subsequent launches forward to the
running instance through the existing control socket/inbox. The `miaotty://`
identity remains unchanged.
