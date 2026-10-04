# ADR 0010 — Agent 闭环:通知、防休眠、提示队列

> English (default): [`0010-agent-loop.md`](0010-agent-loop.md)

状态:已接受。

## 背景

agent pane 只有在应用能"需要你时提醒你、工作时别打扰你"时才有用。roadmap 的
U6/M1 要求系统通知、防休眠,以及把提示交给 agent 的方式 —— *Composer* 加 *提示队列*。

## 决定

三个副作用,均由 MTP 发布的 agent 状态(按 pane,`agent.state.*`)驱动:

**通知**(`crates/mtty-ui/src/agentloop.rs`)。应用记录每个 pane 的上一次状态,当某 pane
**转换**到 `awaiting` 或 `error` 且它不是焦点 pane 时,发系统通知。平台后端:
`osascript`(macOS)、`notify-send`(Linux)、`powershell`/BurntToast(Windows,尽力而为)。
开关:`notifications`。

**防休眠。** 当任一 pane 的 agent 处于 `processing` 时,启动平台抑制器,使机器不进入
空闲休眠:`caffeinate -dims`(macOS)、`systemd-inhibit … sleep infinity`(Linux);
无 agent 处理时及 drop 时杀掉该进程。开关:`prevent-sleep`。

**Composer 与提示队列。** ⌘⇧E(或面板动词 *Composer*)打开发给焦点 pane 的多行编辑器:
**Send** 立即写入文本并回车,**Queue** 入队。Details 的 *Queue* 标签列出队列提示,可
Send now / 删除。队列中的提示会在其目标 pane 的 agent 变为 `idle` 时自动投递。

## 附记(U6 尾巴)

- **注意力**:agent 为 `awaiting`/`error` 且该 pane 非焦点时标记之;顶栏与侧栏显示 `!`,
  面板列出 *needs attention* 条目,聚焦该 pane 即清除。(系统通知不做深链接到 pane;
  这是应用内的等价物。)
- **恢复(resume)**:Agent 面板显示 `session_id`(可复制)与 *Resume* 动作 —— 若 hook 提供了
  显式 `resume` 字段则用之,否则 `claude --resume <id>` / `codex resume <id>` /
  `<agent> --resume <id>`。
- **配额**:agent 状态里的 `usage` 原样显示在 Agent 面板。
- **徽章**:逐状态开关(`[badges]`)控制标签/侧栏圆点。

## 后果

- agent 闭环只是 MTP 状态的纯消费者,故对任何发布状态的 agent(claude、opencode、miao)
  都有效,且依然可测。
- 平台工具缺失时,通知与防休眠退化为 no-op;它们绝不会导致应用失败。
- 队列在内存中(会话恢复覆盖 pane,不含提示);恢复特定 agent 会话与配额显示仍是后续。
- Update:会话恢复与配额显示已随上文的 U6 尾巴附记落地;队列仍在内存中。
