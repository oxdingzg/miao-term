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

> **项目状态 —— 预发布。** 当前版本为 `0.0.0`,API 尚未稳定。macOS 是主要平台;
> Windows 已在真实硬件上构建、测试并经 MTP 驱动(见 [`docs/WINDOWS-DEV.zh-CN.md`](docs/WINDOWS-DEV.zh-CN.md));
> Linux 在 CI 中构建并通过测试。

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
- 内联标签栏(图标、Agent 徽章、`+`、`×`、拖拽重排),以及带右键菜单
  (重命名 / 前缀 / 标记 / 分组 / 复制 / 关闭 / 关闭其他)的 Tabs 侧边栏。
- 递归分屏树:`⌘D` 向右分屏,`⇧⌘D` 向下分屏,分隔条可拖拽调整比例。`⌘⇧T` 切换临时快速终端。
- 侧边栏**文件树**(双击用查看器打开)与 **View 规则**:把 pane 的 cwd/命令/agent/host/文件
  映射为别名、图标、标签标题与徽章 —— 见 [`docs/VIEW-RULES.zh-CN.md`](docs/VIEW-RULES.zh-CN.md)。
- **Open Quickly**(`⌘K`):一个面板覆盖标签、agent、文件、最近项、内容搜索命中与命令。
- 右侧 details 面板含分页:**Info / Agent / Outline / Git / Files / Ports / Queue**
  (git 状态、目录列表、监听端口、提示队列)。
- **Composer**(`⌘⇧E`)与**提示队列**:agent 空闲时自动投递;由 agent 状态驱动的**通知**与**防休眠**。
- **查看器/编辑器**:只读预览带行号与跳转行高亮,编辑态带行号栏与**可选的极简 vim 模式**(`editor-vim`),*Open Externally* /
  *Edit in Tab*,以及无依赖的 Markdown 渲染(标题、列表、引用、表格、代码、链接、图片、脚注)。
- **终端内联图片**:Sixel / Kitty / iTerm2 图片直接画在字符网格上——随内容滚动、裁剪在 pane 内;
  用 `graphics` 开关(默认开)。
- **Recipes**:保存并回放整个工作区;配置导出。
- 设置窗口(`⌘,`):字号/字体族、透明度、行高、光标样式、主题、agent 徽章、通知、防休眠、
  agent 集成、View 规则 —— 写入 `config.toml` / `views.json`。

**配置与集成**
- 配置位于 `~/.config/miaotty/config.toml`:字号、字体族、透明度、行高、光标样式、主题、配色、
  `language`(英文或简体中文)、`editor`、agent 开关、`quick-terminal-hotkey`、
  `update-check-url` / `update-pubkey` —— 见 [`docs/config.example.toml`](docs/config.example.toml)。
- 当不存在 miaotty 配置时,自动导入 ghostty 的 `config` 与 alacritty 的 `alacritty.toml`。
- zsh shell 集成(经 OSC 7 上报 cwd、命令历史),通过 `ZDOTDIR` shim 安装 —— 不修改用户点文件。
- **URL scheme**:`miaotty://`、`ssh://`、`x-man-page://` 会用对应命令新开标签;二次启动会转发给
  正在运行的实例(单实例,含"聚焦 pane""quick"意图)。
- **全局快速终端热键**:macOS/Windows 用 `global-hotkey`,Linux 用 `GlobalShortcuts` 门户
  (另提供 sway/hyprland/GNOME 等 compositor 绑定)。
- **Agent 集成**:检测 claude/codex/opencode/miao,安装状态上报 hook 脚本,复制接入该 agent 自身
  配置的片段,并可启动 agent —— 不替用户修改 agent 配置。`miao` 通过内置集成自动上报状态,
  无需接线 hook。
- **更新**:检查清单、下载本平台产物、校验 SHA-256(配置后另校验 minisign 签名),macOS 上安装
  并重启(带回滚 helper)—— 见 [`docs/decisions`](docs/decisions/README.zh-CN.md)。
- **远端 view/edit**:经 pane 的 ssh ControlMaster 连接读/写远端文件,带零安装 terminfo 引导。

**自动化**
- **MTP 控制面**,经 per-user Unix socket(Windows 为命名管道):
  `core.ping/health`、`agent.state.*`、`history.*`、`pane.list/send/run/focus/close`,
  以及 `app.view/edit`(在应用中打开文件)与 `file.read/write`(上限 2 MB)。
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
miaotty-cli view /path/to/file            # 在应用中以只读方式打开
miaotty-cli edit /path/to/file            # 在编辑器中打开
miaotty-cli file read  --path /etc/hosts  # 单次上限 2 MB
miaotty-cli file read  --path app.bin --base64 --offset 0 --length 65536
miaotty-cli file write --path /tmp/x --data "hello"
miaotty-cli file write --path /tmp/x --data-b64 "AAECAw=="   # 二进制
```

若 host 以 `MIAOTTY_MTP_TOKEN` 启动,请求必须携带该令牌;CLI 会从同一环境变量读取。
`MIAOTTY_MTP_ALLOW`(逗号分隔,如 `core.basic,file.read,history.read`)限定允许的能力,其余返回
`forbidden`;不设=全允许,`core.basic`(ping/health)始终允许以便客户端发现 host,`ping` 会在
`allowed` 里报告生效能力集。socket 可经 ssh 转发(`ssh -R /tmp/fwd.sock:<host socket>`),从而让远端
客户端驱动 host。

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
| `⌘K` | Open Quickly(标签、agent、文件、命令) |
| `⌘F` | 查找 |
| `⌘⇧E` | Composer(向焦点 pane 发送多行提示) |
| `⌘⇧T` | 快速终端(临时标签) |
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
| View 规则(标题/图标/徽章) | [`docs/VIEW-RULES.md`](docs/VIEW-RULES.md) | [`docs/VIEW-RULES.zh-CN.md`](docs/VIEW-RULES.zh-CN.md) |
| 性能预算与门 | [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) | [`docs/PERFORMANCE.zh-CN.md`](docs/PERFORMANCE.zh-CN.md) |
| Windows 开发/验证 | [`docs/WINDOWS-DEV.md`](docs/WINDOWS-DEV.md) | [`docs/WINDOWS-DEV.zh-CN.md`](docs/WINDOWS-DEV.zh-CN.md) |
| 发布与更新 | [`docs/RELEASE.md`](docs/RELEASE.md) | [`docs/RELEASE.zh-CN.md`](docs/RELEASE.zh-CN.md) |
| 工作约定(隐私、检查) | [`AGENTS.md`](AGENTS.md) | —— |
| 示例配置 | [`docs/config.example.toml`](docs/config.example.toml) | —— |

---

## 路线图

近期已完成:Windows 命名管道传输与 ConPTY 路径(真实硬件验证)、会话恢复、View 规则、
Open Quickly、details 面板、agent 闭环(通知、防休眠、提示队列)、Recipes、经 ssh 的远端
view/edit、更新下载/校验/安装、URL scheme、全局快速终端热键、i18n,以及性能门。

仍待完成:

- **发布链**:取得签名凭证并跑通端到端 —— minisign 密钥 + 公钥分发、macOS 公证
  (Developer ID 证书 + App 专用密码)、Windows MSI 签名(CA 证书)。工作流已就绪,缺的是密钥
  (见 `docs/RELEASE.md`)。
- **平台验证**:Linux 的 wgpu 渲染路径现已在 CI 中通过 Mesa 软件 Vulkan(lavapipe)覆盖;
  真实 Linux 桌面、Wayland 门户热键、以及 Windows 的 IME/GUI 仍需交互式会话。
- **更新安装**:Windows 与 Linux(目前仅 macOS)。
- **CI 性能基线**:回归门对比记录的基线,需 CI 侧基线存储才能在 CI 生效。
- **原生版对齐**:native 宿主仍缺窗口透明度、全局快速终端热键、URL scheme、以及 shell/agent
  集成安装;其设置窗口是子集。
- **终端内联图片**:超过回滚容量后的锚定是近似;按整个网格区裁剪而非按 pane;Kitty 按区域
  删除(`d=p/c/r`)与动画未实现;会话恢复不保留图像。
- **Markdown**:Mermaid 支持 `graph`/`flowchart` 子集(或经 `mermaid-command` 全量渲染);
  其它图类型显示占位,远程(http/https)图片不加载。
- **编辑器**:两端编辑器均有可选的 vim 模式(ADR 0029);i18n 覆盖主要界面但未覆盖全部字符串。
- **Open Quickly**:文件夹/打开文件条目与 frecency 排序(ADR 0008)。
- **MTP**:`file.read/write` 仅 UTF-8 文本(无二进制/流式传输);已支持按能力授权(`MIAOTTY_MTP_ALLOW`)。

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
