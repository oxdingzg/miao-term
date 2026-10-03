# Installing / building mtty

[简体中文](INSTALL.zh-CN.md)

mtty (formerly miaotty) is the native winit/wgpu application.
There is one GUI executable and one CLI. See [identity and migration](APP-IDENTITY.md).

## From source

```sh
cargo run --release -p mtty-app
cargo build --release -p mtty-app -p mtty-cli
./target/release/mtty --version
```

Rust stable and the platform's wgpu/window-system libraries are required.
The first build compiles wgpu/glyphon and may take a few minutes.

## macOS

```sh
scripts/package-macos.sh
python3 scripts/smoke-hosts.py --bundle dist/mtty.app
bash scripts/install-macos.sh
```

Produces one ad-hoc-signed `dist/mtty.app` with `mtty`, `mtty-cli`, the
icon and URL schemes. Installation archives old known bundles, installs mtty
and removes the old native launcher without removing user configuration.
`PROFILE=debug scripts/package-macos.sh` builds a development bundle.
The installed app uses the macOS system menu bar; a bare binary uses an in-window menu.

## Release packages

[release.yml](../.github/workflows/release.yml) requires Apple Silicon macOS,
Intel macOS, Linux and Windows runner builds. A `v*` tag publishes a release;
manual dispatch rehearses packaging without publishing.

- macOS: a zip containing only `mtty.app`.
- Linux: tar, DEB and AppImage, containing `mtty` and `mtty-cli`.
- Windows: zip and MSI, containing `mtty.exe` and `mtty-cli.exe`.

Apple Developer ID signing/notarization, Windows MSI signing and minisign
artifact signatures use the optional secrets described in [RELEASE.md](RELEASE.md).
`dist-workspace.toml` remains a cargo-dist scaffold, not the active release pipeline.

Windows packages are code-signed under the [SignPath Foundation](https://signpath.org)
program: free code signing provided by [SignPath.io](https://signpath.io), certificate
by [SignPath Foundation](https://signpath.org).

## Configuration and links

Configuration is `~/.config/mtty/config.toml`, or
`$XDG_CONFIG_HOME/mtty/config.toml`; on Windows `%APPDATA%\mtty\config.toml`
(saved state goes to `%LOCALAPPDATA%\mtty`; a `~/.config/mtty` that already
exists under a `HOME` set by Git Bash keeps being used). Ghostty/Alacritty config import and the
zsh ZDOTDIR integration retain their existing behavior. Existing native and
eframe session formats are read through the compatibility migration.

macOS and Linux register `mtty://`, `ssh://` and `x-man-page://`.
Windows MSI registers only `mtty://`. Subsequent launches forward to the
running instance through the existing control socket/inbox. The `mtty://`
identity remains unchanged.
