# ADR 0012 — 最近文件、Markdown 预览、内容搜索

> English (default): [`0012-jump-markdown-search.md`](0012-jump-markdown-search.md)

状态:已接受。

## 背景

U5 剩下的打磨项:Open Quickly 应能到达你刚看过的文件(`jump`),查看器应渲染 Markdown
而非只有原始文本,还应能从键盘搜索文件*内容*(不只是文件名)。

## 决定

**最近文件(`jump`)。** 每次在查看/编辑器中打开文件,都把它前插到 `recent_files`
(去重、上限 50)并随 `session.json` 持久化(`#[serde(default)]`)。Open Quickly 把它们作为
`recent` 条目排在标签*之前*,故按 `(score, index)` 的既有稳定排序自然得到"按最近"的
平局裁决。

**Markdown 预览。** 当文件是 `.md`/`.markdown` 且只读时,查看器渲染一个无依赖的
Markdown 子集:ATX 标题、围栏代码、无序列表、引用、空行间距;行内 `*`/`_`/反引号标记由
`strip_inline` 去除。提供 *Raw* / *Markdown* 切换,编辑仍用纯文本编辑器。

**内容搜索。** 面板 worker(ADR 0009)在 `Request` 上新增 `search` 字段;设置后扫描该 pane
cwd 下的文件(大小写不敏感,深度 ≤5、600 个文件、每文件 256 KiB、40 条命中)并发布
`Hit { path, line, text }`。Open Quickly 把命中显示为 `symbol` 条目(类型显示 `file:line`)
并打开该文件。在面板输入 `#term` 触发搜索;请求去抖 200 ms,避免按键冲刷 worker。

## 后果

- 搜索与打开共用一条代码路径(`OpenFile`)与一个查看器。
- 工作留在 UI 线程之外且有上界,故大目录树的降级表现是"命中更少",而非卡顿。
- 精确跳转到编辑器某行、更丰富的 Markdown(表格/链接/Mermaid)与模糊排序的 `jump`
  属后续。
- Update:按行跳转与链接已在 ADR 0015 落地,表格在 ADR 0017,图片在 ADR 0020,Mermaid
  子集随后落地;模糊排序的 `jump` 仍属后续。
