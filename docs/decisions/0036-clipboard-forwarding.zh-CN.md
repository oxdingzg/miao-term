# ADR 0036 — 宿主转发剪贴板图片

> English (default): [`0036-clipboard-forwarding.md`](0036-clipboard-forwarding.md)

状态:已接受。

## 背景

运行在 mtty 里的 TUI 无法自己读取 macOS 剪贴板。miao 目前通过 shell out 到
`osascript -l JavaScript` 来实现,这既慢,又因为 `osascript` 属于脚本编辑器,把应用和那个
bundle 绑在一起(ADR 0035)。Linux 走 `wl-paste`/`xclip`,Windows 走 PowerShell。

宿主本来就会为文本粘贴读取剪贴板,而“剪贴板里只有图片”的既有约定是发一个空的
bracketed paste,让应用自己去取图(`crates/mtty-widget/src/lib.rs` 的
`paste_clipboard`)。

## 决定

宿主拥有剪贴板图片,通过一个文件交给 pane:

- 启动时 mtty 把 `MTTY_CLIPBOARD_FILE`(临时目录下、每个进程一个的路径)导出到 pane 的
  环境里,和 `MTTY_CLI`/`MTTY_SOCKET` 并列(`export_pane_environment`)。
- 粘贴时,如果剪贴板里是图片,`paste_clipboard` 读取它
  (`mtty-platform::clipboard_image`,macOS 用 `NSPasteboard`),把 PNG 写进该路径,
  然后仍发送应用已经当作“去读剪贴板”信号的空 bracketed paste。
- 文本粘贴会先删除该文件,这样之后的一次空粘贴不会读到过期图片。
- 还没有后端平台的 `clipboard_image` 返回 `None`,宿主回退到文本粘贴。

应用通过读这个文件来接入:miao 的 `clipboard.read()` 优先尝试
`MTTY_CLIPBOARD_FILE`,并保留 `osascript`/`wl-paste`/PowerShell 作为回退,因此在其他终端
和 ssh 下仍然工作。

考虑过的其他方案:
- bracketed paste 里带 data URI 需要能力协商(私有 DECSET),否则不认识的 TUI 会把
  base64 当文本粘贴;而且几 MB 要过 pty。
- OSC 1337 inline image 的语义是“显示这张图”,不是“把图片数据给你”,通道不对。
- 用 MTP 让应用向宿主要剪贴板是最纯的模型,但会让 miao 变成 MTP 客户端;留作长期方向。

## 后果

- 在 macOS 上,miao 在 mtty 里粘贴不再需要 `osascript`,和脚本编辑器的关联随之消失。
- 该路径被所有 pane 共享(剪贴板是全局的),生命周期到进程结束;每次粘贴都会被覆盖或删除。
- 非 mtty 终端和 ssh pane 保持既有回退。
- Linux/Windows 的宿主后端(`wl-paste`/PowerShell)是后续工作;在那之前这些平台用应用自身的回退。
