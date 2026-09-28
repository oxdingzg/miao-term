# miao-term

**一个用 Rust 编写的、快速且可嵌入的跨平台终端模拟器与引擎。**

[![CI](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml/badge.svg)](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](rust-toolchain.toml)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#环境要求)

[English](README.md) · **简体中文**

---

## 概述

`miao-term` 在一个仓库里包含两件事:

- **终端引擎** —— `miao-term-core` 负责从 PTY 到屏幕的热路径,不依赖任何窗口或 GPU 代码;
- **`miaotty`** —— 构建在该引擎之上的、开箱即用的终端应用,具备标签、分屏、侧边面板、
  设置窗口、shell 集成以及可脚本化的控制面。

引擎与应用被刻意解耦:`miaotty` 是引擎的第一个消费者,而引擎本身设计为可被第三方嵌入。
完整设计见 [`docs/ARCHITECTURE.zh-CN.md`](docs/ARCHITECTURE.zh-CN.md)。

> **项目状态 —— 预发布。** 当前版本为 `0.0.0`,API 尚未稳定。macOS 是主要且已验证的平台;
> Linux 与 Windows 能在 CI 中构建,但尚未经过充分实测。

---

## 功能

**终端内核**
- 基于 PTY 的 shell,经 `alacritty_terminal` 做 VT 解析。
- GPU 字形网格渲染(`wgpu` + `glyphon`),复用 egui 的 device、queue 与 surface。
- 回滚缓冲与滚动条指示、拖拽/双击选区、复制粘贴,以及宽字符 / 中日韩排版与系统 CJK 字体回退。
- 查找(`⌘F`),带匹配高亮与结果计数。
- 当程序请求时,支持
  [kitty keyboard 协议](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)(CSI-u)
  与带修饰键的光标移动。

**窗口与工作区**
- 内联标签栏(`+`、`×`),以及带右键菜单(重命名 / 复制 / 关闭 / 关闭其他)的 Tabs 侧边栏。
- 递归分屏树:`⌘D` 向右分屏,`⇧⌘D` 向下分屏,分隔条可拖拽调整比例。
- 右侧 details 面板:工作目录、Agent 状态、每个 pane 的命令历史。
- 设置窗口(`⌘,`):字号、字体族与主题,可写入 `config.toml`。

**配置与集成**
- 配置位于 `~/.config/miaotty/config.toml`:字号、字体族、内置命名主题或显式配色。
- 当不存在 miaotty 配置时,自动导入 ghostty 的 `config` 与 alacritty 的 `alacritty.toml`。
- zsh shell 集成(经 OSC 7 上报 cwd、命令历史),通过 `ZDOTDIR` shim 安装 —— 不修改用户点文件。

**自动化**
- **MTP 控制面**,经 per-user Unix socket(Windows 命名管道为规划中的传输方式):
  `core.ping/health`、`agent.state.*`、`history.*`、`pane.list/send/run/focus/close`。
- **`miaotty-cli`**,跨平台的控制面客户端。

---

## 架构

引擎按层组织,依赖只向内指向(`widget → render → core`),平台相关代码被收敛在少数模块中。

| Crate | 职责 |
|-------|------|
| [`miao-term-core`](crates/term-core) | PTY、VT 解析、网格/回滚、选区、查找、OSC、输入编码。不含 GPU 与窗口。 |
| [`miao-term-render`](crates/term-render) | `wgpu` + `glyphon` 字形网格渲染器。 |
| [`miao-term-widget`](crates/term-widget) | `winit` 集成、输入/IME/剪贴板、egui 组装。 |
| [`miao-term-config`](crates/term-config) | 配置与主题,以及 ghostty/alacritty 导入。 |
| [`miao-term-mtp`](crates/term-mtp) | MTP 协议、host/client 与传输。 |
| [`miaotty-app`](miaotty-app) | `miaotty` 二进制:标签、分屏、面板、设置。 |
| [`miaotty-cli`](miaotty-cli) | `miaotty-cli` 控制客户端。 |

热路径 —— `pty → vt → grid → renderer` —— 不跨锁,且每帧不做分配。平台差异只出现在
`core::pty`、`widget::platform` 与 `mtp::transport`。

### 原则

1. 引擎与应用分离;`miaotty-app` 是引擎的第一个消费者。
2. 热路径不跨锁、不做分配。
3. 先做应用、后抽库,API 由真实需求驱动。
4. 平台差异只出现在 `core::pty`、`widget::platform`、`mtp::transport`。
5. 控制面(MTP / CLI)与引擎解耦。

---

## 环境要求

- Rust **stable** 工具链(锁定于 [`rust-toolchain.toml`](rust-toolchain.toml);MSRV 1.80)。
- 支持 Metal(macOS)、Vulkan(Linux)或 DX12(Windows)的 GPU 与驱动。
- 在 Linux 上,需要常见的 `winit`/`wgpu` 系统库(X11 或 Wayland 开发包)。

---

## 快速开始

```sh
git clone https://github.com/oxdingzg/miao-term.git
cd miao-term

# 构建并运行终端
cargo run -p miaotty-app          # 或:cargo build --release && ./target/release/miaotty
```

首次构建会编译 `wgpu`/`glyphon`,可能需要几分钟。

### 测试与静态检查

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

CI 在 macOS、Linux 与 Windows 上运行 `cargo check` 与 `cargo test`;见
[`.github/workflows/ci.yml`](.github/workflows/ci.yml)。

### 打包

```sh
scripts/package-macos.sh          # -> dist/miaotty.app(ad-hoc 签名)
```

发布构建由 [`.github/workflows/release.yml`](.github/workflows/release.yml) 在
`v*` 标签上生成。[`dist-workspace.toml`](dist-workspace.toml) 是一份
[cargo-dist](https://opensource.axo.dev/cargo-dist/) 脚手架。见
[`docs/INSTALL.zh-CN.md`](docs/INSTALL.zh-CN.md)。

---

## 配置

miaotty 读取 `~/.config/miaotty/config.toml`(或
`$XDG_CONFIG_HOME/miaotty/config.toml`)。所有键均可选;完整参考见
[`docs/config.example.toml`](docs/config.example.toml)。

```toml
font-size   = 14
font-family = "JetBrains Mono"   # 默认:系统等宽字体
theme       = "nord"             # nord | dracula | gruvbox | solarized | tokyo-night

[colors]                          # 显式配色会覆盖命名主题
background = "#2e3440"
foreground = "#d8dee9"
palette    = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
              "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
```

若不存在 miaotty 配置,会自动导入 ghostty 的 `config` 与 alacritty 的
`alacritty.toml`。

### Shell 集成

启动时,miaotty 会安装一个 zsh `ZDOTDIR` shim,用于上报工作目录(OSC 7)与命令历史。
shim 写入一个仅当前用户可访问的私有目录,并会恢复用户真实的 `ZDOTDIR`,因此现有
点文件不受影响,无需手动配置。

---

## 控制面

**MTP**(miaotty terminal protocol)控制面在 `$XDG_RUNTIME_DIR/miaotty.sock`
(回退到 `$TMPDIR`)上使用换行分隔的 JSON,且 socket 以仅属主可访问的权限创建。
shell 会继承 `MIAOTTY_SOCKET` 与 `MIAOTTY_PANE_ID`。

`miaotty-cli` 是参考客户端:

```sh
miaotty-cli ping
miaotty-cli pane list
miaotty-cli pane run --pane ID --data "echo hello"
miaotty-cli pane focus --pane ID
miaotty-cli state claude --state processing --pane ID
miaotty-cli state list
miaotty-cli history add --command "cargo test" --cwd "$PWD"
miaotty-cli history list --pane ID
```

使用 `--socket PATH` 或设置 `MIAOTTY_SOCKET` 可指定非默认 socket。

---

## 快捷键

| 快捷键 | 操作 |
|--------|------|
| `⌘T` | 新建标签 |
| `⌘W` | 关闭当前 pane(或标签) |
| `⌘D` / `⇧⌘D` | 向右 / 向下分屏 |
| `⌥⌘→` / `⌥⌘←`(或 `⌘⇧]` / `⌘⇧[`) | 轮换 pane 焦点 |
| `⌥⌘D` | 开关 details 面板 |
| `⌘F` | 查找 |
| `⌘,` | 设置 |
| `⌘+` / `⌘-` / `⌘0` | 增大 / 减小 / 重置字号 |
| `Shift+PgUp` / `Shift+PgDn` | 滚动视口 |

在 macOS 上 `⌘` 为命令修饰键;其他平台上使用对应的主修饰键。

---

## 文档

文档默认英文,并配套同步维护的简体中文版(`*.zh-CN.md`)。

| 文档 | English | 简体中文 |
|------|---------|---------|
| 架构与设计 | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | [`docs/ARCHITECTURE.zh-CN.md`](docs/ARCHITECTURE.zh-CN.md) |
| 架构决策记录 | [`docs/decisions/`](docs/decisions/README.md) | [`docs/decisions/README.zh-CN.md`](docs/decisions/README.zh-CN.md) |
| 安装 | [`docs/INSTALL.md`](docs/INSTALL.md) | [`docs/INSTALL.zh-CN.md`](docs/INSTALL.zh-CN.md) |
| 示例配置 | [`docs/config.example.toml`](docs/config.example.toml) | —— |

---

## 路线图

尚未实现:

- Windows 命名管道传输与 ConPTY 实测。
- 打包:公证的 macOS 构建、Linux AppImage/Flatpak/`.deb`、Windows MSI。
- 渲染器打磨:damage 驱动的图集上传与图集 trim。
- 更完整的配置导入,以及窗口/分屏状态的持久化。

---

## 参与贡献

欢迎贡献。提交 pull request 前:

1. 运行 `cargo fmt --all`、`cargo clippy --workspace --all-targets` 与
   `cargo test --workspace`。
2. 保持文档双语:修改 `doc.md` 时,请在同一改动中更新 `doc.zh-CN.md`。
3. 若要变更某条**锁定**的设计决策,请在 [`docs/decisions/`](docs/decisions/README.md)
   下新增一份 ADR,而不是就地改写记录。

参与贡献即表示你同意你的贡献按本项目的许可(Apache-2.0)授权。

---

## 许可

基于 [Apache License, Version 2.0](LICENSE) 授权。
