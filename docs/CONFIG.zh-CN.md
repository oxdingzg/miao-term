# 配置

[English](CONFIG.md)

mtty 的配置项全部可选。只有一行的文件就是一份合法配置，文件不存在等同于空配置。

## 配置文件的位置

| 平台 | 路径 |
|---|---|
| Linux / macOS | `~/.config/mtty/config.toml`，或 `$XDG_CONFIG_HOME/mtty/config.toml` |
| Windows | `%APPDATA%\mtty\config.toml` |

保存状态(会话、队列、窗口位置)放在同一目录;Windows 上是 `%LOCALAPPDATA%\mtty`。文档其他地方提到的
其他 `~/.config/mtty/...` 文件也都在这个目录里。

v0.0.5 及之前应用名为 `miaotty`。首次启动时，若 `$XDG_CONFIG_HOME/mtty` 不存在，会把
`$XDG_CONFIG_HOME/miaotty` 复制过去，并保留旧目录，使更早的版本仍可使用。见
[应用身份与迁移](APP-IDENTITY.zh-CN.md)。

## 最小配置

```toml
font-size   = 13                 # 默认 13
font-family = "JetBrains Mono"   # 默认;回退到系统等宽字体
theme       = "nord"             # nord | dracula | gruvbox | solarized | tokyo-night

[colors]                          # 显式配色会覆盖命名主题
background = "#2e3440"
foreground = "#d8dee9"
palette    = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
              "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
```

若不存在 mtty 配置，会自动导入 ghostty 的 `config` 与 alacritty 的 `alacritty.toml`。

## 配置项

| 键 | 默认值 | 作用 |
|---|---|---|
| `font-size` | `13` | 字号 |
| `font-family` | `JetBrains Mono` | 任意已安装字体;回退到系统等宽字体 |
| `line-height` | `1.25` | 字号的倍数 |
| `cursor-style` | `block` | `block`、`bar` 或 `underline` |
| `background-opacity` | `1.0` | `0.1`–`1.0`;小于 1 需要支持合成的窗口管理器 |
| `notifications` | `true` | agent 需要你时发系统通知 |
| `prevent-sleep` | `true` | agent 工作时保持系统不休眠 |
| `restore-scrollback` | `true` | 退出时保存终端内容，重启后显示 |
| `pty-host` | `false` | 重启后保留 shell 进程(实验性) |
| `quick-terminal-hotkey` | — | 全局快速终端热键，如 `cmd+shift+t` |
| `editor-vim` | `false` | 内置编辑器启用极简 vim 模式 |
| `editor` | — | 「在标签中编辑」执行的命令，如 `code --wait` |
| `mermaid-command` | — | 用 mermaid-cli 渲染 ` ```mermaid ` 块;不设则用内置子集 |
| `graphics` | `true` | 内联终端图像(Sixel / Kitty / iTerm2) |
| `remote-listen` | — | 用 TCP 暴露 MTP 控制面，如 `127.0.0.1:7273`(需 `MTTY_MTP_TOKEN`) |
| `language` | — | 界面语言，`en` 或 `zh`;也会读取 `$LANG` |
| `update-pubkey` | — | minisign 公钥;启用签名校验 |
| `update-check-url` | — | 更新清单地址，见下文 |
| `theme` | — | 内置命名主题，会被显式的 `[colors]` 覆盖 |

### 标签徽章

`[badges]` 决定哪些 agent 状态在标签上显示徽章。四项默认全开:

```toml
[badges]
processing = true
idle = true
awaiting = true
error = true
```

### 语言服务器

编辑器窗格会为登录 shell 的 `PATH` 上已安装的 `rust-analyzer`、
`typescript-language-server`、`pyright-langserver`、`gopls` 或 `clangd` 启动语言服务器;超过
2 MB 的文件不启动。每项的命令可以是用空格切分的字符串，也可以是数组;`root-markers` 用于识别
工作区根目录。

```toml
# [lsp]
# enabled = false                  # 全部关闭
# [lsp.rust]                       # rust | typescript | python | go | c
# command = "rust-analyzer"
# root-markers = ["Cargo.toml"]    # 最近的含该文件的目录即工作区
# [lsp.python]
# command = ["pylsp"]
# [lsp.go]
# enabled = false
```

鼠标停在代码上会显示类型、文档与问题。

### ACP agent

每个 `[acp]` 条目都可以从命令面板的「ACP Agent…」启动，并打开一个会话窗口。`command` 可以是用
空格切分的字符串，也可以是数组。

```toml
# [acp]
# [[acp.agent]]
# name = "codex"
# command = "codex acp"
# [[acp.agent]]
# name = "gemini"
# command = ["gemini", "--experimental-acp"]
```

### 更新检查

```toml
# update-pubkey = "RW…"            # minisign 公钥;启用签名校验
# update-check-url = "https://example.com/mtty/latest.json"
#   JSON 清单: {"version":"0.2.0","artifacts":{"macos-aarch64":{"url":"…","sha256":"…"}}}
#   首行为版本号的纯文本文件同样可用。
```

## Shell 集成

新 pane 中的 shell 会上报工作目录(OSC 7)、每条命令输出的起止与退出码(OSC 133)以及命令历史，
无需手动配置。shim 写入仅当前用户可访问的私有目录，并先加载用户自己的启动文件:

| Shell | shim 的加载方式 |
|---|---|
| zsh | 一个 `ZDOTDIR`,其 `.zshenv` 会恢复真实的 `ZDOTDIR` |
| bash | `--rcfile`,先 source `~/.bashrc`;bash 4.4+ 用 `PS0`,更老的 bash(macOS 3.2)用 DEBUG trap |
| fish | 经 `XDG_DATA_DIRS` 找到的 `vendor_conf.d` 脚本，并恢复原值 |
| PowerShell | 在 profile 之后用 `-NoExit -Command` 加载;包装 `prompt` 与 PSReadLine(历史需 PowerShell 7) |

你自己的启动文件绝不修改。每种 shim 都在真实 PTY 中做了端到端测试(Linux 上的 zsh、bash 3.2/5.x、
fish 3.7、PowerShell 7.5;Windows 上的 PowerShell 由 CI 运行)。

## 视图规则

窗格标题、图标与徽章来自同一目录下的 `views.json`。见[视图规则](VIEW-RULES.zh-CN.md)。

## 完整参考

[`config.example.toml`](config.example.toml) 是带注释的完整参考:上面每个键及其默认值都在一个文件里。
