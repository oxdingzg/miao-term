# ADR 0035 — 原生系统通知

> English (default): [`0035-native-notifications.md`](0035-native-notifications.md)

状态:已接受。

## 背景

ADR 0010 规定:某 agent pane 转换到 `awaiting` 或 `error` 且它不是焦点 pane 时,发一条
系统通知。当时选的平台后端是命令行工具:`osascript`(macOS)、`notify-send`(Linux)、
BurntToast(Windows),都写在 `crates/term-ui/src/agentloop.rs`。

在 macOS 上,`osascript … display notification` 会归属于 **脚本编辑器**,因为
`/usr/bin/osascript` 属于那个 app 的 bundle:横幅显示错误的图标和名字、不能点击,
激活它还可能把脚本编辑器的"打开文件"面板带到前台。Windows 的 BurntToast 依赖一个
可能没装的 PowerShell 模块。Linux 的 `notify-send` 没问题。

## 决定

新增 `crates/term-platform`(`miao-term-platform`)承载原生系统集成,先从通知开始。
`crates/term-ui/src/agentloop.rs` 从它 re-export `notify`/`alert`,这样各 host 仍然调用
`agentloop::notify`(ADR 0010),而平台代码离开 host-agnostic 的 crate。

在 macOS 上,当进程从 app bundle 运行时,`notify` 通过 `UNUserNotificationCenter`
(objc2 类型化绑定)发送。一个 `UNUserNotificationCenterDelegate` 让 mtty 在前台时也显示
横幅;每次发送都请求授权(系统只会在第一次弹框,且横幅在 completion 回调里发送,首次
授权框不会把它吞掉)。没有 bundle identifier 时(直接跑裸二进制、`cargo run`,或 headless
host),`notify` 返回失败,调用方保留 `osascript` 回退,所以 bundle 之外的行为不变。

Linux 通过 freedesktop D-Bus 服务(`zbus`)发送,Windows 通过 mtty 的 AppUserModelID 下的
WinRT toast 发送,两者都保留命令行回退。`alert`(启动失败时的阻塞对话框)仍用
`osascript`/zenity/PowerShell:它在窗口出现前运行,而 `display alert` 不会产生脚本编辑器
的打开面板。

## 后果

- macOS 通知归属于 mtty、可点击,不再激活脚本编辑器。
- `term-ui` 保持 host-agnostic:`objc2`、`block2`、`objc2-user-notifications` 都限制在
  `miao-term-platform` 的 macOS target 内。
- 原生路径需要 app bundle。`scripts/package-macos.sh` 已经会构建并 ad-hoc 签名一个,
  测试和本地运行都不需要证书;只有对外分发才需要 Developer ID + 公证。
- 通知中心的 `delegate` 是弱引用,所以有一个 delegate 对象会故意泄漏到进程结束。
- 若用户拒绝通知权限,横幅会被静默丢弃;应用内的 attention 标记(ADR 0010 补遗)仍然有效。
