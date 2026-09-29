# ADR 0015 — 编辑器打磨:按行跳转、源码视图、外部打开

> English (default): [`0015-editor-polish.md`](0015-editor-polish.md)

状态:已接受。

## 背景

查看器能打开文件,但忽略了用户*从哪来*(搜索命中或变更文件),没有行号,也没有通往其他
编辑器的出口。这正是 U5 剩下的打磨。

## 决定

- **按行跳转。** `open_editor_at(path, line)` 记录 1 起的目标行。Open Quickly 的内容搜索
  命中会带上命中行;最近文件与 details 传 `None`。只读视图会高亮目标行并在打开时滚动到它。
- **源码视图。** 只读且非 Markdown 的内容以带行号的等宽视图渲染(`source_ui`),目标行用
  高亮色描框并用 `scroll_to_rect` 居中。编辑仍用纯文本编辑器。
- **外部打开。** *Open Externally* 调用系统默认应用
  (`open` / `xdg-open` / `cmd /C start`);*Edit in Tab* 打开一个运行
  `${EDITOR:-vi} <path>` 的终端标签。
- **Markdown 增补。** 分隔线(`---`/`***`/`___`)渲染为分隔符;行内 `[text](url)` 链接渲染为
  `hyperlink_to` 片段(该行其余部分为纯文本)。

## 后果

- 搜索结果现在会落到匹配行,闭合了内容搜索(ADR 0012)与查看器之间的回路。
- 这些都是在字符串之上的展示逻辑,并保持有单测
  (`link_segments`、`is_rule`、`heading`)。
- *编辑态*的行号栏、更丰富的 Markdown(表格、图片、Mermaid)与 `open_with` 偏好仍属后续。
- Update:编辑态行号栏已在 ADR 0020 落地,表格/图片/Mermaid 已在 ADR 0017/0020 落地;
  `open_with` 偏好仍属后续。
