# ADR 0016 — Agent 集成自动化与单实例

> English (default): [`0016-integration-automation.md`](0016-integration-automation.md)

状态:已接受。

## 背景

还差两个 U6/系统级能力:agent 必须手工接线到 miaotty;以及第二次启动(例如再打开一个
`ssh://` 链接)会开第二个实例,而不是复用正在运行的那个。

## 决定

**Agent hooks**(`crates/mtty-ui/src/integration.rs`)。应用知道一小组 agent(claude、codex、
opencode、miao)及其二进制、启动命令与 hook 注册位置。在 *设置 → Agent 集成* 中你可以:
- 查看各 agent 是否在 `PATH` 上;
- **安装 hook**:写出 `~/.config/miaotty/hooks/<agent>.sh`(mode 0755,并以 `sh -n`
  做语法检查),脚本通过 `miaotty-cli state <agent> --state <s>` 并带
  `--pane $MIAOTTY_PANE_ID` 上报状态;
- **复制片段**:复制要粘贴到该 agent 自身 hook 配置里的那一行;
- **启动**:新开标签运行该 agent。

我们**刻意不**替用户改 agent 的配置;只生成脚本并交付片段。

**自上报 agent。** `miao` 内置插件在检测到 `MIAOTTY_PANE_ID` 时,直接用同一套状态词表经
`miaotty-cli` 上报,因此无需安装 hook 或粘贴片段。UI 将其标注为*内置*,只提供**启动**。

**单实例 / 深链接。** 启动时检查 MTP socket;若已有实例在监听,新进程把请求写到
`~/.local/share/miaotty/inbox/` 后退出。运行中的实例每帧排空该目录,为转发的命令开标签
(裸启动即"激活")。无需新的 MTP 方法 —— 因此**不改动共享的 `mtty-mtp` crate**。

## 后果

- "接上一个 agent"变成一次点击加一次粘贴,且绝不会用重写工具配置的方式惊到用户。
- `ssh://` 等链接会落到用户会话所在处,而不是再开一个重复窗口。
- 真正的全局热键、深链接*进入*已有窗口的某个 pane、远端 hook 仍属后续。
- Update:真正的全局热键与深链接到具体 pane 已在 ADR 0019 落地;远端 hook 仍属后续。
