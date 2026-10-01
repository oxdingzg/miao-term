# miao-term

**一个用 Rust 编写的、快速且可嵌入的跨平台终端模拟器与引擎。**

[![CI](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml/badge.svg)](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](Cargo.toml)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#环境要求)

**官网:** [mtty.dev/mtty](https://mtty.dev/zh/mtty) · **文档:** [mtty.dev/docs/mtty](https://mtty.dev/zh/docs/mtty) · [版本发布](https://github.com/oxdingzg/miao-term/releases) · [相关项目](#相关项目)

[English](README.md) · **简体中文**

---

## 概述

`miao-term` 在一个仓库里包含两件事:

- **终端引擎** —— `miao-term-core` 负责从 PTY 到屏幕的热路径,不依赖任何窗口或 GPU 代码;
- **`mtty`** —— 构建在该引擎之上的、开箱即用的终端应用,具备标签、分屏、侧边面板、
  设置窗口、shell 集成以及可脚本化的控制面。

**应用统一为一个 `mtty`**，使用 `miao-term-widget` 中的原生 `winit` + `wgpu`
窗口与渲染循环，在同一帧合成 egui 界面。`mtty-app` 提供主程序及安装包元数据。
包含画中画、Hint、只读模式和每个 pane 的关闭按钮。

> **由 miaotty 更名而来。** v0.0.5 及之前应用名为 `miaotty`。mtty 首次启动会复制
> `~/.config/miaotty`,读取 `MIAOTTY_*` 环境变量,继续响应 `miaotty://` 链接与旧 socket 路径,
> 并在 pane 中导出旧变量,已安装的 hook 与 miao 无需修改 —— 见
> [ADR 0032](docs/decisions/0032-rename-mtty.zh-CN.md) 与 [APP-IDENTITY.zh-CN.md](docs/APP-IDENTITY.zh-CN.md)。

引擎与应用保持解耦，供第三方嵌入。
完整设计见 [`docs/ARCHITECTURE.zh-CN.md`](docs/ARCHITECTURE.zh-CN.md)。

> **项目状态 —— 预发布。** 当前版本为 `0.0.6`,API 尚未稳定。macOS 是主要平台;
> Windows 已在真实硬件上构建、测试并经 MTP 驱动(见 [`docs/WINDOWS-DEV.zh-CN.md`](docs/WINDOWS-DEV.zh-CN.md));
> Linux 在 CI 中构建并通过测试,并已在真实的 GNOME/Wayland 桌面上验收(输入法、菜单、文件拖放、剪贴板、快捷键)。

---


## 相关项目

| 项目 | 是什么 | 链接 |
|---|---|---|
| **mtty**(本仓库) | 终端应用,以及其背后可嵌入的终端引擎 | [mtty.dev/mtty](https://mtty.dev/zh/mtty) · [oxdingzg/miao-term](https://github.com/oxdingzg/miao-term) |
| **miao** | 在终端里运行的开源 AI 编程代理 | [mtty.dev/miao](https://mtty.dev/zh/miao) · [oxdingzg/miao](https://github.com/oxdingzg/miao) |
| **mtty.dev** | 两者的官网与文档站 | [mtty.dev](https://mtty.dev/zh/) |

mtty 与 miao 是两个独立项目,任意一个都可以单独使用。在 mtty 的窗格里,miao 通过控制面上报自己的状态(工作中、
等待你、已完成、出错);mtty 据此给窗格加徽章、在代理需要你时通知你、在代理工作时阻止电脑休眠,并在它空闲时发出你
排队的提示。mtty 也为其他代理 CLI(Claude Code、Codex、OpenCode)提供状态钩子。

有两个都叫 *miaotty* 的东西:本仓库的应用在 v0.0.5 及之前名为 `miaotty`(见上文的更名说明);而
[oxdingzg/miaotty](https://github.com/oxdingzg/miaotty) 是另一个更早的个人 macOS 原型(基于 Ghostty 分叉),
已不再开发,请改用 mtty。

## 功能

**终端内核**
- 基于 PTY 的 shell,经 `alacritty_terminal` 做 VT 解析。
- GPU 字形网格渲染(`wgpu` + `glyphon`),复用 egui 的 device、queue 与 surface。
- 回滚缓冲与滚动条指示、拖拽/双击选区、复制粘贴,以及宽字符 / 中日韩排版与系统 CJK 字体回退。
- 查找(`⌘F`),带匹配高亮与结果计数,支持中文。
- 当程序请求时,支持
  [kitty keyboard 协议](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)(CSI-u)
  (disambiguate 级别)与带修饰键的光标移动;F1–F12 / Insert 按 xterm 序列发送。

**窗口与工作区**
- 内联标签栏(图标、Agent 徽章、`+`、`×`、拖拽重排)与 Tabs 侧边栏。右键标签或会话行弹出
  同一套菜单:重命名标签…、前缀…、标记…、分组…、移出分组(仅在分组内时出现)、复制标签、
  上移/下移、新建标签、关闭标签、关闭其他标签、关闭下方标签。标记会附加在标题后,分组随会话
  一起持久化,重启后仍在。应用提供这套菜单,标签栏在分组变化处显示分隔线。
  “关闭标签”、标签的 `×` 与会话行中键关闭整个标签(含全部分屏),但保留最后一个标签。
  `⌘W` / `Ctrl+W` 在分屏标签中仍只关闭当前窗格。
- 左右两侧面板可拖动边缘调整宽度,宽度随窗口尺寸一起保存。
- 递归分屏树:`⌘D` 向右分屏,`⇧⌘D` 向下分屏,分隔条可拖拽调整比例,每个 pane 都有
  关闭按钮。`⌘⇧T` 切换临时快速终端。
- details 面板 Files 分页中的**文件列表**(单击用查看器打开)与 **View 规则**:把 pane 的目录、
  前台命令、agent 或 SSH 主机映射为别名、图标、标签标题与徽章,`views.json` 修改后自动生效 ——
  见 [`docs/VIEW-RULES.zh-CN.md`](docs/VIEW-RULES.zh-CN.md)。
- **命令面板**(`⌘K`)与 **Open Quickly**(`⌘⇧O`):覆盖标签、agent、当前目录的文件与子目录,以及最近文件。
- 右侧 details 面板含分页:**Info / Agent / Outline / Git / Files / Ports / Queue**
  (git 状态、目录列表、监听端口、提示队列)。
- **Composer**(`⌘E`;Linux/Windows 上 `Ctrl+Shift+E`)与**提示队列**:队列项绑定入队时的 pane,该 pane 的 agent 每次转为空闲时
  投递一条(重启后保留;"立即发送"会将其移出队列);由 agent 状态驱动的**通知**与**防休眠**。后台标签会
  标出 agent 需要输入或出错(`!`)、已完成(✓)或有新输出(•);收到通知后不久切回 mtty,会直接显示该
  通知对应的 pane。
- **查看器/编辑器**:只读预览带行号,编辑态带行号栏与**可选的极简 vim 模式**(`editor-vim`)。
  保存失败会提示且保持"已修改";有未保存修改时需再次关闭才会丢弃。CommonMark 渲染
  (`egui_commonmark`):标题、列表、引用、表格、代码、链接、远程图片,外加 `graph`/`flowchart`、`sequenceDiagram`、`stateDiagram`、`classDiagram`、
  `erDiagram` 与 `pie` 的 Mermaid 子集(或经 `mermaid-command` 全量渲染)。
- **终端内联图片**:Sixel / Kitty / iTerm2 图片由 mtty 直接画在字符网格上——随内容滚动、
  裁剪在 pane 内;用 `graphics` 开关(默认开)。
- **Agent 任务**:*新建 Agent 任务…* 创建 git worktree 与分支(`<仓库>/.worktrees/<名称>`、
  `mtty/<名称>`),在独立标签中打开并可直接启动 agent;*Agent 任务…* 列出任务,可打开、查看改动
  (含未提交内容)、合并回基线分支或丢弃(二者均需确认)。git 操作在后台执行,不改动仓库中被跟踪的文件。
- **Recipes**:保存并回放整个工作区。
- 设置窗口(`⌘,`):字号/字体族、透明度、行高、光标样式、主题、内联图片、通知、防休眠与
  agent 钩子安装。关闭窗口时把改动过的值写回 `config.toml`,保留注释与其他键;无法解析的
  `config.toml` 会在状态栏提示,且不会被覆盖。

**配置与集成**
- 配置位于 `~/.config/mtty/config.toml`:字号、字体族、透明度、行高、光标样式、主题、配色、
  `language`(英文或简体中文)、`editor`、agent 开关、`quick-terminal-hotkey`、
  `update-check-url` / `update-pubkey` —— 见 [`docs/config.example.toml`](docs/config.example.toml)。
- 当不存在 mtty 配置时,自动导入 ghostty 的 `config` 与 alacritty 的 `alacritty.toml`。
- zsh、bash、fish 与 PowerShell 的 shell 集成(经 OSC 7 上报 cwd、命令历史、OSC 133 命令边界),
  每种 shell 各用一个 shim 安装 —— 不修改用户点文件。有了它,*复制上一条命令的输出* 与 *把上一条命令的输出发到 Composer* 只取上一条
  命令的输出(含退出码)。
- **URL scheme**:`mtty://`、`ssh://`、`x-man-page://` 会用对应命令新开标签;二次启动会转发给
  正在运行的实例(单实例,含"聚焦 pane""quick"意图),无参数的二次启动会把窗口带到前台。
  URL 目前经命令行参数传入;macOS 从浏览器/Finder 发来的 URL 事件尚未处理。
- **macOS 原生菜单栏**:安装的 `.app` 会把 文件/编辑/视图/终端/Agent/帮助 放进系统菜单栏
  (含 About/Services/Hide/Quit),窗口内不再有菜单条,与其它 macOS 终端一致(ADR 0031);
  裸跑 `mtty` 仍用窗口内菜单。窗口本身请求深色外观,标题栏与界面一致,不再是一条
  浅色条。
- **全局快速终端热键**:macOS/Windows 用 `global-hotkey`,Linux 使用 sway/hyprland/GNOME 等 compositor 绑定。
- **Agent 集成**:检测 claude/codex/opencode/miao,并可在新标签中启动(设置或命令面板);
  安装状态上报 hook 脚本,并复制可直接合并的配置 —— Claude Code(`~/.claude/settings.json`)
  与 codex(`~/.codex/hooks.json`)的 `hooks` JSON、opencode 的插件文件。不替用户修改 agent 配置;
  hook 只为运行在 mtty pane 内的 agent 上报。`miao` 通过内置集成自动上报状态,无需接线 hook。
- **更新**:*检查更新* 读取版本清单;*下载更新* 获取本平台安装包,在程序内校验 SHA-256 与 minisign 签名
  (使用内置的发布公钥,或 `update-pubkey`);未签名或校验不符的下载会被删除,绝不安装。*安装并重启*(或命令面板
  的 *更新并重启*,一步完成)在 macOS 上替换 app 并在失败时回滚,替换正在运行的 AppImage,或运行 Windows MSI;
  deb 与 tarball 安装则打开已校验的下载交给包管理器。
- **主机库**:保存在 `~/.config/mtty/hosts.toml` 的主机(名称、地址、用户、端口、分组、标签、跳板机)
  显示在侧栏与 Open Quickly 中;*主机…* 可搜索、添加、删除(需确认),并导入 `~/.ssh/config` 中的具体
  `Host` 条目 —— 导入的主机按别名连接,ssh 对该条目的所有配置都会生效。`mtty://host/<名称>` 可从脚本或
  启动器直接连接。不保存密码或密钥。*主机…* 还显示 ssh-agent 中的密钥,检查主机密钥是否与 `known_hosts`
  一致(未知密钥显示指纹供核对后信任;已变化的密钥会被拒绝),并在终端标签中运行 `ssh-keygen` / `ssh-copy-id`。每台主机可保存端口转发(`-L`、`-R`、`-D` SOCKS),
  在窗口中启停,显示运行状态或 ssh 放弃的原因;状态栏显示数量。
- **SSH 会话与远端 view/edit**:*新建 SSH 会话…* 遵循 `~/.ssh/config`,复用 ControlMaster 连接,
  远端零安装引导 terminfo;*查看/编辑远端文件…* 经该连接读写(自动带入当前 SSH 标签的主机)。
- **FTP/FTPS**:*连接 FTP/FTPS…* 经系统 `curl` 打开同一个双栏文件浏览器(默认显式 TLS;明文 FTP 会标注未加密)。
  口令只保存在内存中;留空则使用 `~/.netrc` 或匿名登录。
- **持久会话**:在已保存主机上勾选 *在 tmux 中保持 shell*,重连后回到原会话;*使用 mosh 连接* 可跨休眠和网络切换
  (两端都需安装 mosh;本机没有 mosh 时退回 ssh)。
- **加密同步(可选,默认关闭)**:*同步主机与片段…* 把主机与片段加密后写入你已在同步的文件夹(iCloud Drive、
  Dropbox、Syncthing)。无需账号、没有服务器;密钥保存在 `~/.config/mtty/sync.key`,第二台设备用配对码加入
  (ADR 0033)。
- **命令片段与广播**:*命令片段…* 把命令保存在 `~/.config/mtty/snippets.toml`(名称、命令、标签),
  可在当前 pane 执行,或在多台已保存主机上执行(各开一个标签);Open Quickly 也能搜到。*向本标签所有分屏
  广播输入* 让输入同时进入所有分屏(状态栏显示 BROADCAST)。
- **SFTP**:双栏文件浏览(本机 | 主机),可从"主机…"、命令面板(当前 SSH 标签)或 `mtty://sftp/<名称>` 打开:
  浏览、上传(也可把文件拖到窗口上)、带进度的下载、重命名、chmod、新建文件夹与删除(需确认)。底层调用系统
  `sftp`,ssh 配置、跳板机、agent 与共享连接均照常生效。

**自动化**
- **MTP 控制面**,经 per-user Unix socket(Windows 为命名管道):
  `core.ping/health`、`agent.state.*`、`history.*`、`pane.list/send/run/focus/close`、
  `pane.output`(上一条命令的输出,能力 `pane.read`),以及 `app.view/edit`(在应用中打开文件)与
  `file.read/write`(上限 2 MB)。
- **`mtty-cli`**,跨平台的控制面客户端。

---

## 架构

引擎按层组织,依赖只向内指向(`host → render → core`),引擎之上提供可复用的 UI helper。

| Crate | 职责 |
|-------|------|
| [`miao-term-core`](crates/term-core) | PTY、VT 解析、网格/回滚、选区、查找、OSC、输入编码。不含 GPU 与窗口。 |
| [`miao-term-graphics`](crates/term-graphics) | 内联图片流扫描器与解码器(Sixel、Kitty、iTerm2)。 |
| [`miao-term-render`](crates/term-render) | `wgpu` + `glyphon` 字形网格渲染器,含 quad 与图像管线。 |
| [`miao-term-ui`](crates/term-ui) | 与 host 无关的 UI:主题、输入编码、选区、分屏布局、egui 外壳、命令面板、hint、vim、markdown、ssh、更新与 agent 集成等 helper。 |
| [`miao-term-config`](crates/term-config) | 配置与主题,ghostty/alacritty 导入,以及 View 规则引擎。 |
| [`miao-term-mtp`](crates/term-mtp) | MTP 协议、host/client 与传输(Unix socket、Windows 命名管道、TCP)。 |
| [`miao-term-widget`](crates/term-widget) | mtty 原生 host 库:`winit` + `wgpu` 渲染循环,直接绘制网格并合成 egui 外壳。 |
| [`mtty-app`](mtty-app) | `mtty` 原生主程序及平台安装包元数据。 |
| [`mtty-cli`](mtty-cli) | `mtty-cli` 控制客户端。 |

热路径 —— `pty → vt → grid → renderer` —— 不跨锁,且每帧不做分配。平台差异只出现在负责
相应关注点的 crate 中少量 `#[cfg]` 守卫的代码块里:`term-core` 的 PTY 派生
(`src/term.rs`)、`term-widget` 的窗口/事件循环(`src/lib.rs`),以及 `term-mtp` 的
socket/命名管道传输(`src/lib.rs`)。

### 原则

1. 引擎与 host 分离;hosts 是引擎的第一批消费者。
2. 热路径不跨锁、不做分配。
3. 先做应用、后抽库,API 由真实需求驱动。
4. 平台差异只出现在 `term-core`、`term-widget` 与 `term-mtp` 中 `#[cfg]` 守卫的代码块里。
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

# 构建并运行统一的原生终端
cargo run --release -p mtty-app
```

首次构建会编译 `wgpu`/`glyphon`,可能需要几分钟。

### 测试与静态检查

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

CI([`.github/workflows/ci.yml`](.github/workflows/ci.yml))在每次 push、PR 与 nightly 上运行。
push 时执行 `cargo fmt --check`、`cargo clippy --workspace --all-targets -D warnings`、
Linux 与 macOS 的 `cargo check` 以及隐私扫描;完整的三平台
`cargo test --workspace`、Linux 软件 Vulkan 渲染测试、Windows job 与 release 模式性能门,
则在 pull request、nightly 计划与手动触发时运行。仅文档变更的 push 会跳过编译与 lint job。

### 打包

```sh
scripts/package-macos.sh          # -> dist/mtty.app(ad-hoc 签名)
```

发布构建由 [`.github/workflows/release.yml`](.github/workflows/release.yml) 在
`v*` 标签上生成:`mtty` 与 `mtty-cli`、macOS app bundle、
Linux `.deb`/AppImage 以及 Windows MSI。[`dist-workspace.toml`](dist-workspace.toml) 是一份
[cargo-dist](https://opensource.axo.dev/cargo-dist/) 脚手架。见
[`docs/INSTALL.zh-CN.md`](docs/INSTALL.zh-CN.md) 与 [`docs/RELEASE.zh-CN.md`](docs/RELEASE.zh-CN.md)。

---

## 配置

mtty 读取 `~/.config/mtty/config.toml`(或
`$XDG_CONFIG_HOME/mtty/config.toml`)。所有键均可选;完整参考见
[`docs/config.example.toml`](docs/config.example.toml)。

```toml
font-size   = 13                 # 默认 13
font-family = "JetBrains Mono"   # 默认;回退到系统等宽字体
theme       = "nord"             # nord | dracula | gruvbox | solarized | tokyo-night

[colors]                          # 显式配色会覆盖命名主题
background = "#2e3440"
foreground = "#d8dee9"
palette    = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
              "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
```

若不存在 mtty 配置,会自动导入 ghostty 的 `config` 与 alacritty 的
`alacritty.toml`。

### Shell 集成

新 pane 中的 shell 会上报工作目录(OSC 7)、每条命令输出的起止与退出码(OSC 133)以及命令历史,无需手动配置。
shim 写入仅当前用户可访问的私有目录,并先加载用户自己的启动文件:

| Shell | shim 的加载方式 |
|---|---|
| zsh | 一个 `ZDOTDIR`,其 `.zshenv` 会恢复真实的 `ZDOTDIR` |
| bash | `--rcfile`,先 source `~/.bashrc`;bash 4.4+ 用 `PS0`,更老的 bash(macOS 3.2)用 DEBUG trap |
| fish | 经 `XDG_DATA_DIRS` 找到的 `vendor_conf.d` 脚本,并恢复原值 |
| PowerShell | 在 profile 之后用 `-NoExit -Command` 加载;包装 `prompt` 与 PSReadLine(历史需 PowerShell 7) |

每种都在真实 PTY 中做端到端测试(Linux 上的 zsh、bash 3.2/5.x、fish 3.7、PowerShell 7.5;Windows 上的
PowerShell 由 CI 运行)。

---

## 控制面

**MTP**(mtty terminal protocol)控制面在 `$XDG_RUNTIME_DIR/mtty.sock`
(回退到 `$TMPDIR`)上使用换行分隔的 JSON,且 socket 以仅属主可访问的权限创建。
shell 会继承 `MTTY_SOCKET` 与 `MTTY_PANE_ID`。每个响应都带状态 `revision`;
`core.wait` 会阻塞到该值超过给定值后再返回;`core.subscribe` 则把连接升级为事件流
(`agent.state`、`panes`、`history`),客户端据此跟踪 agent 状态、pane 或历史,无需轮询。

`mtty-cli` 是参考客户端:

```sh
mtty-cli ping
mtty-cli wait --since 42               # 阻塞直到状态 revision 变化
mtty-cli events                        # 以 JSON 行流式输出状态变化
mtty-cli events --topic agent.state    # ...仅订阅某个 topic
mtty-cli pane list
mtty-cli pane run --pane ID --data "echo hello"
mtty-cli pane focus --pane ID
mtty-cli pane output --pane ID           # 上一条命令的输出与退出码
mtty-cli state claude --state processing --pane ID
mtty-cli state list
mtty-cli history add --command "cargo test" --cwd "$PWD"
mtty-cli history list --pane ID
mtty-cli view /path/to/file            # 在应用中以只读方式打开
mtty-cli edit /path/to/file            # 在编辑器中打开
mtty-cli file read  --path /etc/hosts  # 单次上限 2 MB
mtty-cli file read  --path app.bin --base64 --offset 0 --length 65536
mtty-cli file write --path /tmp/x --data "hello"
mtty-cli file write --path /tmp/x --data-b64 "AAECAw=="   # 二进制
```

**远程访问**:设 `remote-listen = "127.0.0.1:7273"`(并设 `MTTY_MTP_TOKEN`)即可用 TCP 暴露控制
平面,客户端 `mtty-cli --socket tcp://host:7273` 连接;**没令牌时 TCP 监听会拒绝启动**。注意控制平面
能在你的 shell 里执行命令,令牌务必保密(并尽量只监听 loopback 或走 ssh 隧道)。

若 host 以 `MTTY_MTP_TOKEN` 启动,请求必须携带该令牌;CLI 会从同一环境变量读取。
`MTTY_MTP_ALLOW`(逗号分隔,如 `core.basic,file.read,history.read`)限定允许的能力,其余返回
`forbidden`;不设=全允许,`core.basic`(ping/health)始终允许以便客户端发现 host,`ping` 会在
`allowed` 里报告生效能力集。socket 可经 ssh 转发(`ssh -R /tmp/fwd.sock:<host socket>`),从而让远端
客户端驱动 host。

使用 `--socket PATH` 或设置 `MTTY_SOCKET` 可指定非默认 socket。

---

## 快捷键

| macOS | Linux / Windows | 操作 |
|---|---|---|
| `⌘T` | `Ctrl+Shift+T` | 新建标签 |
| `⌘W` | `Ctrl+Shift+W` | 关闭当前 pane(或标签) |
| `⌘D` / `⇧⌘D` | `Ctrl+Shift+D` / `Ctrl+Shift+Alt+D` | 向右 / 向下分屏 |
| `⌘[` / `⌘]` | `Ctrl+Shift+[` / `Ctrl+Shift+]` | 上一个 / 下一个 pane |
| `⇧⌘[` / `⇧⌘]` | `Ctrl+PgUp` / `Ctrl+PgDn`(或 `Ctrl+Tab`) | 上一个 / 下一个标签 |
| `⌘1`…`⌘9` | `Alt+1`…`Alt+9` | 跳到标签 |
| `⌘K`(或 `⇧⌘P`) | `Ctrl+Shift+K`(或 `Ctrl+Shift+P`) | 命令面板 |
| `⇧⌘O` | `Ctrl+Shift+Alt+O` | Open Quickly(标签、agent、文件、主机) |
| `⌘F` | `Ctrl+Shift+F` | 查找 |
| `⌘G` / `⇧⌘G` | `Ctrl+Shift+G` / `Ctrl+Shift+Alt+G` | 下一个 / 上一个匹配 |
| `⇧⌘H` | `Ctrl+Shift+Alt+H` | Hints(按标签打开链接或路径) |
| `⌘E` | `Ctrl+Shift+E` | Composer(向焦点 pane 发送多行提示) |
| `⇧⌘T` | `Ctrl+Shift+Alt+T` | 快速终端(临时标签) |
| `⇧⌘Z` | `Ctrl+Shift+Alt+Z` | 重新打开最近关闭的标签 |
| `⇧⌘L` / `⇧⌘R` | `Ctrl+Shift+Alt+L` / `Ctrl+Shift+Alt+R` | 开关侧栏 / details 面板 |
| `⌘,` | `Ctrl+,` | 设置 |
| `⌘+` / `⌘-` | `Ctrl+=` / `Ctrl+-` | 增大 / 减小字号 |
| `⌘C` / `⌘V` | `Ctrl+Shift+C` / `Ctrl+Shift+V`(`Ctrl+V` 也可;有选区时 `Ctrl+C` 也可复制) | 复制 / 粘贴 |
| `Shift+PgUp` / `Shift+PgDn` | 滚动视口 |

在 Linux 与 Windows 上,单独的 `Ctrl` 组合键(`Ctrl+C`、`Ctrl+W`、`Ctrl+D`……)始终交给 shell,`Super`/`Win` 组合留给桌面。

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
| 产品需求与路线图 | [`docs/PRODUCT.md`](docs/PRODUCT.md) | [`docs/PRODUCT.zh-CN.md`](docs/PRODUCT.zh-CN.md) |
| 发布与更新 | [`docs/RELEASE.md`](docs/RELEASE.md) | [`docs/RELEASE.zh-CN.md`](docs/RELEASE.zh-CN.md) |
| 工作约定(隐私、检查) | [`AGENTS.md`](AGENTS.md) | —— |
| 示例配置 | [`docs/config.example.toml`](docs/config.example.toml) | —— |

---

## 路线图

**mtty** 的产品方向、
已验证的功能基线与里程碑计划见 [`docs/PRODUCT.zh-CN.md`](docs/PRODUCT.zh-CN.md)。

近期已完成:Windows 命名管道传输与 ConPTY 路径(真实硬件验证)、会话恢复、View 规则、
Open Quickly、details 面板、agent 闭环(通知、防休眠、提示队列)、Recipes、经 ssh 的远端
view/edit、版本检查、URL scheme、全局快速终端热键、应用的内联 IME 拼写与
终端内联图片、Mermaid 时序/状态/类/ER/饼图、MTP 事件流、i18n,以及性能门。
CI 性能基线已持久化于 `benches/perf-baseline.json`,由 nightly/手动运行刷新;
比较仅报告,绝对预算作为门控。

仍待完成:

- **发布验收**:手动演练覆盖四个 runner 构建、包检查、minisign 校验与更新清单汇总。
  仍需正式发布及下载/安装验收;**Apple 公证**与 **Windows MSI 签名**需要凭证。
  见 `docs/RELEASE.zh-CN.md`。
- **平台验证(需要硬件)**:Linux 的 wgpu 渲染路径已在 CI 中通过 Mesa 软件 Vulkan(lavapipe)
  覆盖,Windows 也在真机上经 MTP 驱动;真实 Linux 桌面会话、Wayland 门户热键、Windows 的
  IME/GUI 路径仍需一台交互机器。
- **更新安装**:macOS 替换流程已用打包后的 app 与测试签名的发布包做过端到端验证(篡改的包会被拒绝);
  AppImage 与 Windows 辅助脚本有单元测试,尚未在这两类桌面上实际运行。
- **单一原生应用**:原 native 实现统一以 mtty 发布；配置和会话保留迁移兼容，
  见 [APP-IDENTITY.zh-CN.md](docs/APP-IDENTITY.zh-CN.md)。
- **终端内联图片**:不模拟 Kitty 的 z-index(图片绘制在网格之上);回滚容量内锚定精确,超出后
  为近似(alacritty 不暴露滚动计数,除非打补丁);会话恢复不保留图像(会与恢复的内容不一致)。
- **Mermaid**:内置子集覆盖 `graph`/`flowchart`、`sequenceDiagram`、`stateDiagram`、
  `classDiagram`、`erDiagram` 与 `pie`;gantt、journey、git graph 等仍回退到
  `mermaid-command` 或占位符。
- **i18n**:外壳、命令面板、设置与对话框已覆盖;少量示例/agent 串刻意保留英文。

---

## 参与贡献

欢迎贡献。提交 pull request 前:

1. 运行 `cargo fmt --all`、`cargo clippy --workspace --all-targets` 与
   `cargo test --workspace`。
2. 保持文档双语:修改某个文档时,请在同一改动中更新其 `*.zh-CN.md` 对应版本(例如
   `README.md` / `README.zh-CN.md`,或 `docs/RELEASE.md` / `docs/RELEASE.zh-CN.md`)。
3. 若要变更某条**锁定**的设计决策,请在 [`docs/decisions/`](docs/decisions/README.md)
   下新增一份 ADR,而不是就地改写记录。

参与贡献即表示你同意你的贡献按本项目的许可(Apache-2.0)授权。

---

## 许可

基于 [Apache License, Version 2.0](LICENSE) 授权。
