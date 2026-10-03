# ADR 0042 —— Agent 会话恢复与配额显示

> English: [`0042-agent-resume-quota.md`](0042-agent-resume-quota.md)

状态:已接受(2026-10-03)。实现作为 M7/A4 跟踪,见
[`../PRODUCT.zh-CN.md`](../PRODUCT.zh-CN.md)。

## 背景

agent 闭环(ADR 0010、0016、0040)已经知道每个 pane 里跑的是哪个 agent、它的状态,
以及 hook 上报的会话 id:MTP `agent.state.set` 带 `agent`、`state`、`session_id`、`tty`,
宿主把它们存在 `agent_states` 里,Agent 面板会显示。`docs/PRODUCT.md` 把两个缺口列为
“暂不可用 —— 尚未设计”:

- **恢复(Resume)。** pane 或应用结束后,会话 id 还在,但无法把 agent 重新拉回该会话。
  每个 CLI 的写法都不同(`claude --resume <id>`、`codex resume <id>`……),而 miao 已原生支持。
- **配额(Quota)。** 同时跑多个 agent 时,用户无法在不离开 mtty 去问 agent 的情况下,
  看到某个 agent 的用量窗口还剩多少。

## 决策

### 恢复

每个受支持的 agent([`integration::Agent`](../../crates/term-ui/src/integration.rs))
新增一个带 `{session}`、`{cwd}` 占位符的 `resume` 命令模板;没有模板的 agent 即不可恢复。
具体参数在接线时逐个核对。

- 新增命令 **恢复 Agent 会话…**,列出本次运行中见过的会话 —— agent、会话 id、所在 pane 的
  cwd、最后出现时间,最近的在前。选中后打开一个 cwd 为记录值的新标签,并把该 agent 的
  恢复模板敲进它的 shell。
- 记录放在 agent 状态原本所在的地方(`agent_states`,扩展出会话 id 与 cwd)。本次运行内存态;
  跨重启持久化会话是另一个、更晚的决定。
- `mtty-cli`/MTP 新增 `agent.sessions`(列表)与 `agent.resume`(按 pane id 或会话 id),
  插件或脚本可同样操作。读取需要 `agent.state.read`;恢复新增一个 `agent.resume` 能力位。
- 恢复是把命令敲进一个新 pane 的 shell,和 *启动 agent* 一样;mtty 绝不把 agent 作为自己的子进程运行。

### 配额

状态上报新增一个**可选**的 quota 对象:

```json
"quota": { "used": 42, "limit": 100, "unit": "percent",
           "window": "5h", "resets_at": "2026-10-03T15:00:00Z" }
```

- `mtty-cli state` 在 `--state` 之外接受 `--quota FILE|-`(JSON);hook 脚本原样转发。
- 有配额时,Agent 面板显示一行紧凑文本(“42% of 5h, resets in 3h”);没有则不显示。
  阈值(`agent-quota-warn`,默认 80%)会把它染色。
- mtty 从不调用模型或账号 API:配额就是 agent(或其 hook)报上来的数字。miao 通过内置集成
  经既有的 `agent.state` 通道上报。

## 影响

- 只有当每个 agent 的模板与其 CLI 保持一致时,恢复才正确;模板写错会在 pane 里以
  “命令敲错”的方式可见地失败。
- 不新增网络、凭证或后台常驻:配额是可选、只推的,不上报的 agent 不受影响。
- `agent_states` 多出会话/cwd 记录;提示队列与徽标仍基于既有的状态迁移工作。

## 备选方案

- **定时调用各 agent 查询配额** —— 否决:需要凭证、可能弹交互,且与 agent 已知信息重复。
- **统一的通用恢复(`--continue`)** —— 否决:它恢复的是 agent 最近的会话,未必是用户所选 pane 对应的那个。
