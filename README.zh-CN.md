# miao-term

跨平台(macOS / Linux / Windows)终端**引擎**，用 Rust 编写；`miaotty` 应用构建在其之上。

> English (default): [`README.md`](README.md)
> 状态:**可用的 R0/R1 引导版**(macOS 已验证;Linux/Windows 未测)。
>
> 设计文档:[`docs/ARCHITECTURE.zh-CN.md`](docs/ARCHITECTURE.zh-CN.md) ·
> 决策记录:[`docs/decisions/`](docs/decisions/)。文档默认英文,配套 `*.zh-CN.md` 简体中文版。

## 目前能做什么

- 窗口、PTY 上跑 shell、VT 解析(`alacritty_terminal`)、GPU 网格渲染、键盘输入、resize。
- 多标签 + 左侧边栏(`+` 新建、点击切换、中键关闭、右键 重命名/复制/关闭/关闭其他)。
- 分屏:递归分屏树(任意数量 pane),`⌘D`(右侧)/`⇧⌘D`(下方),点击切换焦点,`⌘W` 关闭当前 pane。
- 右侧 details 面板:Info(工作目录 + 复制路径 / 在访达中显示)、Agent 状态、Outline(每 pane 命令历史)。
- 选区(拖拽 + 双击选词)、复制粘贴、回滚(滚轮 + Shift+PgUp/PgDn)。
- 查找(`⌘F`,匹配高亮 + 计数)。
- 宽字符/中日韩排版 + 系统 CJK 字体回退。
- Unix socket 上的 MTP 控制面:`core.ping/health`、`agent.state.*`、`history.*`、`pane.list`、
  `pane.send/run`、`pane.focus/close` —— 与现有 `miaotty-cli` 互通。
- zsh shell 集成(OSC 7 上报 cwd、命令历史),通过 `ZDOTDIR` shim 自动安装。
- 配置 `~/.config/miaotty/config.toml`(字号 + 配色)—— 见
  [`docs/config.example.toml`](docs/config.example.toml)。
- **GPU 字形渲染**:终端网格由 `term-render`(wgpu + glyphon)经 egui `PaintCallback` 绘制,
  与 egui 共享同一 device/queue/surface。

配置导入:当没有 miaotty 配置时,会自动读取 ghostty `config` 与 alacritty `alacritty.toml`。

打包:`scripts/package-macos.sh` 生成 ad-hoc 签名的 `dist/miaotty.app`;
`dist-workspace.toml` 是 cargo-dist 脚手架。见 [`docs/INSTALL.zh-CN.md`](docs/INSTALL.zh-CN.md)。

快捷键:`⌘T` 新建标签、`⌘W` 关 pane/标签、`⌘D`/`⇧⌘D` 分屏、`⌥⌘→`/`⌥⌘←`(或 `⌘⇧[`/`⌘⇧]`)轮换 pane、
`⌥⌘D` 开关 details、`⌘F` 查找、`⌘+`/`⌘-`/`⌘0` 字号。

尚未完成:Windows 命名管道传输 + ConPTY 实测、打包(MSI/AppImage/公证),
以及渲染器进一步打磨(damage 上传、图集 trim)。

## 运行

```sh
cargo run -p miaotty-app      # 或:./target/debug/miaotty
```

## 目录

```
miao-term/
  crates/
    term-core     PTY + vte + grid + term + 选区/查找 + OSC + 输入编码(无 GPU)
    term-render   wgpu 字形网格渲染器
    term-widget   winit 集成 + 输入/IME/剪贴板 + egui 组装
    term-config   配置 + 主题(+ ghostty/alacritty 导入)
    term-mtp      MTP 协议 + host/client + 传输(Unix socket / Windows 命名管道)
  miaotty-app     产品:窗口/标签/分屏 chrome、面板、徽章、设置
```

## 原则

1. 引擎与应用分离;`miaotty-app` 是引擎的第一个消费者。
2. 热路径(`pty → vt → grid → renderer`)不跨锁、不分配。
3. 先做 app、后抽库,API 由真实需求驱动。
4. 平台差异只出现在 `core::pty`、`widget::platform`、`mtp::transport`。
5. 控制面(MTP/CLI)与引擎解耦。

## 目标技术栈

`portable-pty` · `alacritty_terminal` · `vte` · `winit` · `wgpu` · `glyphon`/`cosmic-text`/`swash` · `egui`。

## 构建

```sh
cargo check
```

## 许可

Apache-2.0,见 [`LICENSE`](LICENSE)。
