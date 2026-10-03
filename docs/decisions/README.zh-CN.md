# 架构决策记录(ADR)

> English (default): [`README.md`](README.md)

简短、只追加的决策(背景 → 决定 → 后果)。变更用新记录取代,而非改写旧记录。
整体设计见 [`../ARCHITECTURE.zh-CN.md`](../ARCHITECTURE.zh-CN.md)。

文档默认英文,简体中文版(`*.zh-CN.md`)需保持同步。

| ADR | 标题 | 状态 |
|-----|------|------|
| [0001](./0001-stack.zh-CN.md) | 技术栈选型(pty/vt/渲染/窗口/UI) | 已接受 |
| [0002](./0002-concurrency.zh-CN.md) | 并发与锁纪律 | 已接受 |
| [0003](./0003-co-frame-rendering.zh-CN.md) | 终端 + egui 共帧渲染 | 已接受 |
| [0004](./0004-tab-split-model.zh-CN.md) | 自研 tab/split 模型(非 OS 原生) | 已接受 |
| [0005](./0005-transport.zh-CN.md) | MTP 传输(Unix socket / Windows 命名管道) | 已接受 |
| [0006](./0006-license-policy.zh-CN.md) | 许可与依赖策略 | 已接受 |
| [0007](./0007-view-rule-engine.zh-CN.md) | View 规则引擎 | 已接受 |
| [0008](./0008-open-quickly.zh-CN.md) | Open Quickly / 命令面板 | 已接受 |
| [0009](./0009-details-panels-editor.zh-CN.md) | Details 面板与文件编辑器 | 已接受 |
| [0010](./0010-agent-loop.zh-CN.md) | Agent 闭环：通知、防休眠、提示队列 | 已接受 |
| [0011](./0011-tab-groups-tree-recipes.zh-CN.md) | 标签分组、文件树与 Recipes | 已接受 |
| [0012](./0012-jump-markdown-search.zh-CN.md) | 最近文件、Markdown 预览、内容搜索 | 已接受 |
| [0013](./0013-system-integration.zh-CN.md) | URL scheme、快速终端、i18n、更新检查 | 已接受 |
| [0014](./0014-ssh.zh-CN.md) | SSH 会话与远端 terminfo | 已接受 |
| [0015](./0015-editor-polish.zh-CN.md) | 编辑器打磨：按行跳转、源码视图、外部打开 | 已接受 |
| [0016](./0016-integration-automation.zh-CN.md) | Agent 集成自动化与单实例 | 已接受 |
| [0017](./0017-markdown-tables-editor-config.zh-CN.md) | Markdown 表格与外部编辑器 | 已接受 |
| [0018](./0018-performance-gate.zh-CN.md) | 性能门 | 已接受 |
| [0019](./0019-hotkey-deeplink.zh-CN.md) | 全局热键与深链接到 pane | 已接受 |
| [0020](./0020-markdown-images-gutter.zh-CN.md) | Markdown 图片、脚注与编辑态行号栏 | 已接受 |
| [0021](./0021-remote-view-edit.zh-CN.md) | 经 ssh 的远端 view/edit | 已接受 |
| [0022](./0022-update-download.zh-CN.md) | 应用内更新下载与校验 | 已接受 |
| [0023](./0023-perf-regression-gate.zh-CN.md) | 性能回归门（共享基线） | 已接受 |
| [0024](./0024-mtp-view-edit.zh-CN.md) | MTP view/edit 与 file read/write | 已接受 |
| [0025](./0025-self-install.zh-CN.md) | 安装并重启（自我替换） | 已接受 |
| [0026](./0026-wayland-shortcut.zh-CN.md) | 原生 Wayland/X11 全局快捷键（门户） | 已接受 |
| [0027](./0027-conpty-teardown.zh-CN.md) | ConPTY 的收尾不得阻塞 | 已接受 |
| [0028](./0028-ci-perf-baseline.zh-CN.md) | CI 侧性能基线 | 已接受 |
| [0029](./0029-vim-mode.zh-CN.md) | 极小、可选的 vim 模式 | 已接受 |
| [0030](./0030-native-render-loop.zh-CN.md) | 原生渲染循环（自绘网格） | 已接受 |
| [0031](./0031-native-app-menu.zh-CN.md) | 把应用菜单放进 macOS 菜单栏 | 已接受 |
| [0032](./0032-rename-mtty.zh-CN.md) | 应用正式更名为 mtty | 已接受 |
| [0033](./0033-encrypted-sync.zh-CN.md) | 可选的端到端加密同步 | 已接受 |
| [0034](./0034-editor-pane.zh-CN.md) | 原生编辑器 pane | 已接受 |
| [0035](./0035-native-notifications.zh-CN.md) | 原生系统通知 | 已接受 |
| [0036](./0036-clipboard-forwarding.zh-CN.md) | 宿主转发剪贴板图片 | 已接受 |
| [0041](./0041-pty-host.zh-CN.md) | pane 跨应用重启存活(每 pane 一个 PTY 宿主进程) | 已接受 |
| [0042](./0042-agent-resume-quota.zh-CN.md) | Agent 会话恢复与配额显示 | 已接受 |
| [0043](./0043-windows-update-transaction.zh-CN.md) | Windows 更新辅助程序的事务替换 | 已接受 |
