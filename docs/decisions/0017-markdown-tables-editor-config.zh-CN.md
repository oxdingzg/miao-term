# ADR 0017 — Markdown 表格与外部编辑器

> English (default): [`0017-markdown-tables-editor-config.md`](0017-markdown-tables-editor-config.md)

状态:已接受。

## 背景

查看器的 Markdown 子集缺少表格;而 "Edit in Tab" 总是用 `$EDITOR`,忽略了用户偏好的
编辑器。

## 决定

**表格。** `markdown_ui` 识别 GFM 管道表格:含 `|` 的表头行、其后是分隔行
(`---|:--:|---`),再是连续的 `|` 行。用 `egui::Grid` 渲染(斑马纹,列对齐来自分隔行的 `:`
标记)并一次消费整块。单元格走与其余渲染相同的行内清理器。辅助函数 `is_table_sep`、
`split_row`、`table_align` 均有单测。

**外部编辑器。** 新增配置键 `editor` 决定 *Edit in Tab* 运行的命令;未设置时回退
`${EDITOR:-vi}`。路径用单引号包裹,故空格与引号安全。

## 后果

- 表格在只读场景下正确,且无需引入 Markdown 库;单元格内联格式仅限清理器(不支持嵌套强调)。
- 用户可固定 `code --wait`、`nvim`、`emacsclient` 等编辑器。
- 图片、脚注与 Mermaid 仍属后续;分隔/管道启发式覆盖常见 GFM 表格形态。
