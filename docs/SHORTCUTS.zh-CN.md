# 快捷键

[English](SHORTCUTS.md)

`⌘` 是 macOS 的主修饰键;Linux / Windows 一列是对应平台上的等价组合。

## 窗口

| macOS | Linux / Windows | 操作 |
|---|---|---|
| `⌘T` | `Ctrl+Shift+T` | 新建标签 |
| `⌘W` | `Ctrl+Shift+W` | 关闭当前 pane(或标签) |
| `⌘D` / `⇧⌘D` | `Ctrl+Shift+D` / `Ctrl+Shift+Alt+D` | 向右 / 向下分屏 |
| `⌘[` / `⌘]` | `Ctrl+Shift+[` / `Ctrl+Shift+]` | 上一个 / 下一个 pane |
| `⇧⌘[` / `⇧⌘]` | `Ctrl+PgUp` / `Ctrl+PgDn`(或 `Ctrl+Tab`) | 上一个 / 下一个标签 |
| `⌘1`…`⌘9` | `Alt+1`…`Alt+9` | 跳到标签 |
| `⇧⌘L` / `⇧⌘R` | `Ctrl+Shift+Alt+L` / `Ctrl+Shift+Alt+R` | 开关侧栏 / details 面板 |
| `⇧⌘T` | `Ctrl+Shift+Alt+T` | 快速终端(临时标签) |
| `⇧⌘Z` | `Ctrl+Shift+Alt+Z` | 重新打开最近关闭的标签 |
| `⌘,` | `Ctrl+,` | 设置 |

## 查找与执行

| macOS | Linux / Windows | 操作 |
|---|---|---|
| `⌘K`(或 `⇧⌘P`) | `Ctrl+Shift+K`(或 `Ctrl+Shift+P`) | 命令面板 |
| `⇧⌘O` | `Ctrl+Shift+Alt+O` | Open Quickly(标签、agent、文件、主机) |
| `⌘F` | `Ctrl+Shift+F` | 查找 |
| `⌘G` / `⇧⌘G` | `Ctrl+Shift+G` / `Ctrl+Shift+Alt+G` | 下一个 / 上一个匹配 |
| `⇧⌘H` | `Ctrl+Shift+Alt+H` | Hints(按标签打开链接或路径) |
| `⌘E` | `Ctrl+Shift+E` | Composer(向焦点 pane 发送多行提示) |

## 终端

| macOS | Linux / Windows | 操作 |
|---|---|---|
| `⌘+` / `⌘-` | `Ctrl+=` / `Ctrl+-` | 增大 / 减小字号 |
| `⌘C` / `⌘V` | `Ctrl+Shift+C` / `Ctrl+Shift+V`(`Ctrl+V` 也可;有选区时 `Ctrl+C` 也可复制) | 复制 / 粘贴 |
| `Shift+PgUp` / `Shift+PgDn` | 滚动视口 |

在 Linux 与 Windows 上，单独的 `Ctrl` 组合键(`Ctrl+C`、`Ctrl+W`、`Ctrl+D`……)始终交给 shell，
`Super`/`Win` 组合留给桌面。

## 编辑器窗格

在编辑器窗格中，下列按键优先于上面的窗口快捷键。语言相关功能需要该语言的语言服务器(见
[`config.example.toml`](config.example.toml) 中的 `[lsp]`);鼠标停在代码上会显示类型、文档与问题。

| macOS | Linux / Windows | 操作 |
|---|---|---|
| `⌘D` | `Ctrl+D` | 选中当前词，再按添加下一个相同项 |
| `⇧⌘L` | `Ctrl+Shift+L` | 选中所有相同项 |
| `⌥⌘↑` / `⌥⌘↓` | `Ctrl+Alt+↑` / `Ctrl+Alt+↓` | 在上方 / 下方添加光标 |
| `⌥` 单击 | `Alt` 单击 | 添加光标 |
| `⇧⌥I` | `Shift+Alt+I` | 在所选各行末尾添加光标 |
| `⌥⌘F` | `Ctrl+H` | 查找替换(`Aa` 区分大小写、`ab` 全字匹配、`.*` 正则) |
| 查找框中 `⌥↩` | 查找框中 `Alt+Enter` | 选中全部匹配 |
| `⌃G` | `Ctrl+G` | 跳转到行(`行` 或 `行:列`) |
| `⌃Space` | `Ctrl+Space` | 补全(输入时也会自动弹出;`↩`/`⇥` 确认) |
| `F12` 或 `⌘` 单击 | `F12` 或 `Ctrl` 单击 | 跳转到定义 |
| `F8` / `⇧F8` | `F8` / `Shift+F8` | 下一个 / 上一个问题 |
| `⌘Z` / `⇧⌘Z` | `Ctrl+Z` / `Ctrl+Y` | 撤销 / 重做 |
