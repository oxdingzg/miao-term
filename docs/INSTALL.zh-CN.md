# 安装 / 构建 miaotty

[English](INSTALL.md)

miaotty 使用原 miaotty-native 的 winit/wgpu 实现，只提供一个 GUI 主程序和一个 CLI。
命名与迁移见 [APP-IDENTITY.zh-CN.md](APP-IDENTITY.zh-CN.md)。

## 从源码运行

```sh
cargo run --release -p miaotty-app
cargo build --release -p miaotty-app -p miaotty-cli
./target/release/miaotty --version
```

需要 Rust stable 和对应平台的 wgpu/窗口系统库；首次编译 wgpu/glyphon 可能需要数分钟。

## macOS

```sh
scripts/package-macos.sh
python3 scripts/smoke-hosts.py --bundle dist/miaotty.app
bash scripts/install-macos.sh
```

只生成一个 ad-hoc 签名的 `dist/miaotty.app`，包内有 `miaotty`、`miaotty-cli`、图标及 URL schemes。
安装时压缩备份身份匹配的旧应用，安装 miaotty 并移除旧 native 入口，保留用户配置。
`PROFILE=debug scripts/package-macos.sh` 可生成开发包。
安装的 app 使用 macOS 系统菜单栏；裸跑二进制使用窗口内菜单。

## 发布包

[release.yml](../.github/workflows/release.yml) 要求 Apple Silicon macOS、Intel macOS、
Linux、Windows 四个 runner 成功。`v*` 标签触发发布，手动 dispatch 演练打包而不发布。

- macOS：zip 只包含 `miaotty.app`。
- Linux：tar、DEB、AppImage，包含 `miaotty` 和 `miaotty-cli`。
- Windows：zip、MSI，包含 `miaotty.exe` 和 `miaotty-cli.exe`。

Apple Developer ID 签名/公证、Windows MSI 签名和 minisign 产物签名使用
[RELEASE.zh-CN.md](RELEASE.zh-CN.md) 说明的可选 secrets。
`dist-workspace.toml` 仍为 cargo-dist 脚手架，不是当前发布流水线。

## 配置与深链接

配置位于 `~/.config/miaotty/config.toml` 或 `$XDG_CONFIG_HOME/miaotty/config.toml`。
Ghostty/Alacritty 配置导入及 zsh ZDOTDIR 集成保持原有行为，旧 native/eframe 会话格式
通过兼容迁移读取。

macOS/Linux 注册 `miaotty://`、`ssh://`、`x-man-page://`；Windows MSI 只注册 `miaotty://`。
后续启动通过已有的控制 socket/inbox 转发到正在运行的实例，`miaotty://` 身份保持不变。
