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
