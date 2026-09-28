# 安装 / 构建

> English (default): [`INSTALL.md`](INSTALL.md)

## 从源码运行

```sh
cargo run -p miaotty-app      # 或:cargo build && ./target/debug/miaotty
```

需要 Rust stable(见 `rust-toolchain.toml`)。首次构建会编译 wgpu/glyphon,可能几分钟。

## macOS .app

```sh
scripts/package-macos.sh              # release 构建 → dist/miaotty.app
PROFILE=debug scripts/package-macos.sh
```

产出 ad-hoc 签名的 `dist/miaotty.app`。要分发还需用 Developer ID 签名并公证(尚未做)。

## 发布

推 `v*` tag 会触发 [`.github/workflows/release.yml`](../.github/workflows/release.yml):
在 macOS/Linux/Windows 构建 `miaotty` + `miaotty-cli`,并把 `miaotty.app` zip / Linux tar / Windows zip
挂到 GitHub Release。

macOS 为 ad-hoc 签名;公证需要 Developer ID 证书,尚未接入。三平台均为免安装产物。

## 跨平台安装包

`dist-workspace.toml` 是 [cargo-dist](https://opensource.axo.dev/cargo-dist/) 脚手架
(shell/PowerShell 安装器 + MSI)。发布时跑 `dist init` 再 `dist build` 生成各平台产物。

## 配置

- `~/.config/miaotty/config.toml`(或 `$XDG_CONFIG_HOME/miaotty/config.toml`)——
  见 [`config.example.toml`](config.example.toml)。
- 若没有 miaotty 配置,会自动导入 ghostty `config` 与 alacritty `alacritty.toml`。

## Shell 集成

miaotty 会安装 zsh 的 `ZDOTDIR` shim,让 shell 上报 cwd(OSC 7)与命令历史;用户 dotfiles 不受影响,
无需手动配置。
