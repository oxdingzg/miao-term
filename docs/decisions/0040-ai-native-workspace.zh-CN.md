# ADR 0040 — AI 原生工作区:agent 编辑、ACP 与上下文交接

> English: [`0040-ai-native-workspace.md`](0040-ai-native-workspace.md)

状态:已接受。

## 背景

M7。今天 agent 运行在 pane 里,mtty 通过 hooks 与 OSC 133(B2)协调它:能看到
pane 状态、排队 prompt、读取命令输出。但还做不到:(A1)把 agent 的修改变成用户可
审阅、可撤销的东西;(A2)通过 Agent Client Protocol 与任意 agent 对话;(A3)把编辑器
选区、诊断或命令输出交给 agent,或让 agent 在指定行打开文件。

所需的基础已经存在:MTP(ADR 0005)是控制通道,编辑器 pane(ADR 0034)有事务/撤销
模型,命令输出按条发布(B2.3)。

## 决定

1. **A1 —— agent 的修改是可审阅的事务。** MTP 增加 `editor.propose`(参数:
   `pane_id`,按字符的 `{start,end,text}` 有序范围列表,或整文件 `text`,以及可选
   的 label)。宿主把它们作为**一个事务**应用到 pane 的 `Document`(一个撤销步骤),
   并把每条被改动的行标记为待处理的提案。pane 行内绘制提案——新增行带绿色行号栏与
   背景,删除的文字加删除线——状态栏提供 **接受**(去掉标记;文本已应用)与
   **拒绝**(`undo`,一步恢复提案前的原文)。输入、保存、关闭或出现新提案都视为接受。
   提案按 pane 保存;pane 关闭即丢弃;在用户保存之前不写磁盘。MTP 返回撤销深度,使
   客户端能区分接受与拒绝。
2. **A2 —— ACP 客户端。** 新增不依赖 GPU 的 `mtty-acp` crate,实现 Agent Client
   Protocol(agent 子进程 stdio 上的 JSON-RPC 2.0,许可证遵循 ADR 0006):`initialize`、
   `authenticate`、`session/new`、`session/load`、`session/prompt`、`session/cancel`
   以及流式的 `session/update`(agent 消息分片、工具调用、计划、diff)。客户端的
   `fs/read_text_file`/`fs/write_text_file` 与 `terminal/*` 请求由编辑器 pane 与当前
   终端应答,`session/request_permission` 弹出一个小的“允许/拒绝”对话框。agent 的 diff
   经 A1 应用。agent 在 `config.toml` 的 `[acp]` 中配置(`command`、`args`、`env`,
   每个 agent 一项);*启动 Agent* 在 pane 中启动一个,ACP 会话驱动该 pane。
3. **A3 —— 把上下文交给 agent,并按行打开。** MTP 的 `app.edit` 与 `app.view` 增加
   可选 `line` 与 `column`,agent 可在指定位置打开文件。命令面板与快捷键增加
   *把选区发给 Agent*、*把诊断发给 Agent*、*把上一条命令输出发给 Agent*(原来的
   Composer 目标之外,再加入当前 agent pane);每条都把简短的带引号 prompt 输入到该
   pane(或 ACP 会话),不经过剪贴板。
4. **非目标。** mtty 自身绝不调用模型,也不保存任何 agent 凭据;ACP agent 与 pane
   agent 一样,都是外部程序。

## 分期

| 阶段 | 内容 | 验收 |
|---|---|---|
| A3 | MTP `line`/`column`;把选区/诊断/输出发给 agent | prompt 文本的单元测试;`app.edit` `line` 的 MTP 测试 |
| A1 | `editor.propose`、单事务应用、行内 diff、接受/拒绝 | 事务与撤销的编辑器测试;回放/截图检查 |
| A2 | `mtty-acp`:JSON-RPC 客户端、会话、流式、fs/终端/权限映射 | 对假 ACP agent 的测试;若装有真实 agent 则对其测试 |

## 影响

- agent 保持外部、可替换;ACP 是增量能力,不支持 ACP 的 agent 仍走 MTP。
- 默认可审阅:agent 的修改是一个撤销步骤,且未经保存不会写入。
- 编辑器 pane 增加一层提案;应用增加一个无依赖的协议 crate;仓库中不引入模型或密钥。
