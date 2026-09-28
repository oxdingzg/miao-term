# 架构决策记录(ADR)

> English (default): [`README.md`](README.md)

简短、只追加的决策(背景 → 决定 → 后果)。变更用新记录取代,而非改写旧记录。
整体设计见 [`../ARCHITECTURE.zh-CN.md`](../ARCHITECTURE.zh-CN.md)。

文档默认英文,简体中文版(`*.zh-CN.md`)需保持同步。

| ADR | 标题 | 状态 |
|-----|------|------|
| [0001](./0001-stack.zh-CN.md) | 技术栈选型(pty/vt/渲染/窗口/UI) | 已接受 |
| 0002 | 并发与锁纪律 | 待定 |
| 0003 | 终端 + egui 共帧渲染 | 待定 |
| 0004 | 自研 tab/split 模型(非 OS 原生) | 待定 |
| 0005 | MTP 传输(Unix socket / Windows 命名管道) | 待定 |
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
