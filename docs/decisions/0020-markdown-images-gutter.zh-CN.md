# ADR 0020 — Markdown 图片/脚注与编辑态行号栏

> English (default): [`0020-markdown-images-gutter.md`](0020-markdown-images-gutter.md)

状态:已接受。

## 背景

查看器还剩两个缺口:Markdown 预览中的图片与脚注;以及*编辑*时的行号(只读视图早已有,
见 ADR 0015)。

## 决定

**图片。** 形如 `![alt](url)` 的整行,对本地位图(`png/jpg/jpeg/gif/webp/bmp`)经
`egui_extras` 的图像加载器(`image` + `file` 特性)渲染为图片。相对路径以文档所在目录解析,
并转为 `file://` URI;远程 `http(s)` 与未知扩展名回退为带标签的超链接。我们**刻意不**启用
`egui_extras` 的 `svg` 特性,那会引入 MPL 许可的 `resvg`(违反 ADR 0006)。

**脚注。** `take_footnotes` 把 `[^id]: text` 定义从正文中抽出;行内 `[^id]` 引用由
`footnote_refs` 改写为 `[id]`;定义在末尾的 *Footnotes* 分隔线下列出。三个辅助函数均有单测。

**编辑态行号栏。** edit 模式下,编辑器与一个固定宽度的行号列并排布局;`paint_gutter` 读取
`TextEditOutput.galley` 的行与 `galley_pos`,故行号跟随文本真实行位置(含滚动),并在以换行结尾
的行(逻辑行)递增。编号是纯函数 `gutter_numbers`,有单测。

## 附记(行号栏)

编辑态行号栏现在位于滚动区**之外**的独立列,故长行横向滚动时它保持不动(此前的局限已消除)。

## 后果

- 预览保真度提升,而无须完整 Markdown 引擎;且关闭了动图解码器(`gif`),只加载静态格式。
- 行号栏与文本处于同一滚动区,故长行横向滚动时会一起移动;对代码尚可接受,日后可把行号栏移出
  滚动区以固定。
- Mermaid/`svg` 图与远程图片仍属后续(后者需要 HTTP 加载器与网络请求)。
- Update:行号栏现已位于滚动区之外(见附记),故不再随长行横向滚动。
- Update:`graph`/`flowchart`、`sequenceDiagram` 与 `pie` 已在应用内直接绘制;其余类型
  (以及渲染的完整能力)仍回退到 `mermaid-command` 或占位符。
