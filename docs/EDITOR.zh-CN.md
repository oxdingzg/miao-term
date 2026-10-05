# 编辑器

[English](EDITOR.md)

mtty 把真正的编辑器放在终端旁边，共用同一批标签页和分屏，文件与其构建它的 shell 始终不离开窗口。编辑器构建在 `mtty-editor` 之上——一个基于 rope、不含 UI 代码的内核，因此同一个引擎也能嵌入别处。编辑通过终端渲染器由 GPU 绘制，这正是超大文件依然流畅的原因。

## 打开文件

详情面板的 **Files** 标签列出当前目录，点击即在阅读器中打开。此外：

- **Open Quickly**（`⌘⇧O`）模糊匹配文件、文件夹、最近文件及其中的文本，也包括标签页、agent 和已保存的主机。
- **Edit in Tab** 在当前编辑器窗格打开文件。依次使用 `editor` 设置、`$EDITOR`、`vi`（Windows 下是 Notepad）。
- **Open Externally** 从命令面板把文件交给外部编辑器。
- 跳转到行导航和 `file:line` 参数会在对应行打开窗格。

本地和远程文件都能在窗格中打开；远程文件通过 ssh 读写（见[远程主机与 SSH](REMOTE.zh-CN.md)）。

## 语法与超大文件

高亮使用 80 个内置的 **tree-sitter** 语法，支持增量重解析和可见范围查询。没有对应语法的文件回退到 syntect 的 Sublime 语法，以及从 `bat` 引入的宽松许可语法（Julia、nginx、VHDL、Org 等）；状态条上会显示识别出的语言。

体积分级处理，而不是拒绝打开：

| 文件大小 | 行为 |
|---|---|
| ≤ 512 KB | 在 UI 线程解析；按键增量重解析 |
| > 512 KB | 解析移到后台线程，颜色稍后到达 |
| > 8 MB | tree-sitter 停止（语法树约占文件的 25–35 倍内存）；Sublime 回退到 1 MB 为止 |
| > 64 MB | 以**视图模式**打开：按窗口从磁盘读取若干行，后台建立稀疏行索引，任意大小下内存都很小 |

视图模式下，输入、粘贴或 **Switch to Editing…** 会提示载入以进行编辑，并说明将占用的内存（约文件的 2.2 倍）。查找在线程中扫描整个文件。

## 编辑

- **多光标**：`⇧⌘L` 选中全部匹配（从裸光标起按整词），`⇧⌥I` 在每一选中行末加光标，`⌥⌘↑` / `⌥⌘↓` 增加光标（macOS 之外为 `Ctrl+Alt+↑/↓`）。
- **查找替换**：区分大小写、整词、正则；屏幕上每个匹配都会高亮；*Replace*、*Replace All*（一步撤销）和 *Select All Matches*（`⌥↩`）。
- **Go to Line**（`⌃G`）接受 `line:column`。
- 常规撤销、重做、缩进、按字素与按词的移动，以及为编辑器保留 `⌘D`、`⇧⌘Z`、`⇧⌘L` 的 macOS 风格键位。

### Vim 模式

设 `editor-vim = true` 可启用作用于文档的 vim 状态机：`NORMAL` / `INSERT` / `VISUAL` / `VISUAL LINE`、计数、`h j k l w b e 0 ^ $ gg G`、`i a I A o O`、`x`、`d`/`c`/`y` 配动作（`dd`、`cc`、`yy`、`dw`、`d$`）、带内部寄存器的 `p`/`P`、`u` 与 `Ctrl-r`、`J`。`/` 打开查找，`:` 执行 `w`、`q`、`wq` 或跳行；`za` 系列控制折叠。状态条显示当前模式。

## 折叠与大纲

折叠来自语法树（任何跨多行的具名节点），没有语法时按缩进。装订线显示 `▸`/`▾`，折叠的标题以 `⋯` 结尾，上下移动会跳过隐藏行。`⌥⌘[` / `⌥⌘]` 折叠与展开；*Fold All*、*Unfold All*、*Toggle Fold* 在命令面板里。`⌘R` 打开可过滤的**大纲**，按深度嵌套显示文件中的定义。

## Markdown 预览

本地 Markdown 文件会在右侧打开一个跟随输入的预览窗格。渲染器覆盖标题、列表、引用、表格、代码和链接，相对图片按文档所在目录解析，并渲染 Mermaid 的 `graph`/`flowchart`、`sequenceDiagram`、`stateDiagram`、`classDiagram`、`erDiagram` 和 `pie`。设 `mermaid-command` 可调用 `mermaid-cli` 获得完整 Mermaid。用命令面板和 View 菜单里的 *Toggle Markdown Preview* 开关预览；预览随编辑器一起关闭，并保留在保存的会话中。

## 语言服务器

配置 `[lsp]` 后，编辑器会在登录 shell 的 `PATH` 上寻找 `rust-analyzer`、`typescript-language-server`、`pyright-langserver`、`gopls` 或 `clangd` 并启动——每种语言分组和工作区根目录一个服务。超过 2 MB 的文件不启用。

窗格中提供：诊断下划线与状态栏的 ✖/⚠ 计数、指针停留后的悬停、触发字符与词及 `Ctrl+Space` 的补全（客户端过滤，含片段与 import 编辑）、`F12` 或 `⌘`-点击跳转定义、`F8` 跳到下一个问题。服务器与根目录标记在 [`config.toml`](CONFIG.zh-CN.md) 的 `[lsp]` 下配置。

## 保存，不丢工作

保存是原子的，并保留文件权限。保存失败会被报告，缓冲区保持**已修改**；关闭未保存的改动、Close Others/Below 或退出都会先询问。打开的窗格会监视文件：没有未保存修改时，作为一次可撤销事务就地重载；有未保存修改时，询问是重载还是保留你的版本；文件在下方被删除会报告一次。远程窗格以同样方式通过 ssh 重载。

## 配置与快捷键

| 键 | 作用 |
|---|---|
| `editor` | *Edit in Tab* 运行的命令（如 `code --wait`） |
| `editor-vim` | 启用最简 vim 模式 |
| `mermaid-command` | 用 `mermaid-cli` 渲染完整 Mermaid |
| `[lsp]` | 语言服务器、命令与工作区根目录标记 |

[快捷键](SHORTCUTS.zh-CN.md)列出了窗口、终端和编辑器的按键。
