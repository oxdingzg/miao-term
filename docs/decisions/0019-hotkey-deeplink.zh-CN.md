# ADR 0019 — 全局热键与深链接到 pane

> English (default): [`0019-hotkey-deeplink.md`](0019-hotkey-deeplink.md)

状态:已接受。

## 背景

两个系统级缺口:真正系统级的快速终端热键;以及让后续启动(URL scheme 或快捷键)作用于
*正在运行*的实例 —— 包括聚焦到具体 pane —— 而不是再开一个重复窗口。

## 决定

**统一意图模型**(`crates/term-ui/src/launch.rs`)。`Intent` 为
`Activate | Quick | Focus(pane_id) | Run(command)`,从 argv 解析
(`--quick`、`--focus <id>`、`ssh://…`、`x-man-page://…`、`miaotty://quick`、
`miaotty://focus?pane=…`),并为跨实例 inbox 编码/解码
(`quick`、`focus\t<id>`、`run\t<cmd>`)。往返有单测。

**全局热键。** `crates/term-ui/src/hotkey.rs` 解析加速键(`cmd+shift+t`、
`ctrl+alt+space`,与平台无关且各处可测),并在 macOS 与 Windows 上通过
`global-hotkey`(MIT)注册;该依赖按目标平台声明,故 Linux CI 不会引入 X11/Wayland。
处理器翻转标志并调用 `egui::Context::request_repaint`;应用每帧轮询并切换快速终端。
注册失败只记日志、绝不致命;加速键来自配置 `quick-terminal-hotkey`(未设置即关闭)。
在 Linux 上(以及任何地方作为替代),设置在 *设置* 中展示 skhd / Hammerspoon /
AutoHotkey / GNOME 的开箱可粘贴绑定,调用 `miaotty --quick`。

**深链接。** `main` 把 argv 变成 `Intent`;若 MTP socket 上已有实例应答,则把编码后的意图
经 inbox 目录转发并退出。运行实例排空它并应用:`Focus` 切到该 pane 所在标签并聚焦;
`Run` 开标签;`Quick` 切换快速终端。面板动词 *Copy Pane ID* 给出可用的 id。

## 附记(Wayland)

原生 Wayland 全局快捷键已另行实现(见 ADR 0026),经 `GlobalShortcuts` xdg-desktop-portal。
设置行另在 skhd/Hammerspoon/AutoHotkey/GNOME 之外
提供 **sway** 与 **hyprland** 绑定,它们都调用 `miaotty --quick` —— 于是 Wayland 用户通过
compositor 自身的绑定机制获得快速终端,而这本来也是他们绑定其他一切的方式。

## 后果

- 一条路径同时处理 argv、URL scheme 与转发请求,故无论怎么唤起行为一致。
- 全局热键失败(无显示、抢占冲突)会退化为外部绑定片段,而不是让启动失败。
- 按 id 聚焦 pane、切换快速终端与运行命令都可从应用外部驱动,这正是编辑器/启动器集成所需。
  在已有窗口*内部*抓键(菜单加速键)与 Wayland 原生全局快捷键仍属后续。
