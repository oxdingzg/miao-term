# ADR 0008 — Open Quickly / 命令面板

> English (default): [`0008-open-quickly.md`](0008-open-quickly.md)

状态:已接受。

## 背景

miaotty 中一切可导航的对象 —— 窗口、标签、pane、文件夹、agent —— 以及一切动作,
都应能从同一个"键盘优先"的入口触达。View 规则引擎(ADR 0007)已为每个 pane 给出
标题与图标,因此面板能一致地呈现标签与 agent。

## 决定

一个浮层,用 **⌘K** 打开(Escape 关闭),对单一扁平条目列表做模糊过滤并执行所选条目。

- **条目类型**:`tab`、`agent`、`command`。每条带标签、可选的规则引擎图标、类型标记
  (右对齐显示)与动作。
- **排序**(`palette_score`):大小写不敏感;空查询匹配全部;子串命中按命中位置排序;
  否则子序列命中排在所有子串命中之后;无子序列则不匹配。
- **动作**(`command`):New Tab、Split Right、Split Down、Close Tab、Toggle Details、
  Find、Settings、Next/Previous Tab。(`jump`、文件夹与打开文件随编辑器在 U5 落地。)
- **交互**:↑/↓ 移动选择(循环),Enter 执行,点击或悬停即选中。面板打开期间,窗口快捷键
  与 PTY 输入被抑制,按键只进入查询框。
- **归属**:面板是应用状态(`Option<Palette>`),不是插件;它直接读取标签/agent,并经由
  与快捷键相同的那些方法派发。

## 后果

- 标签与 agent 自动带上规则引擎的标题/图标。
- 新条目只需扩展 `palette_entries`,新动作只需扩展 `Verb` —— 都是局部的少量改动。
- 文件夹/打开文件条目与 frecency(`jump` 排序)推迟到 U5,等文件模型落地后一并处理。
