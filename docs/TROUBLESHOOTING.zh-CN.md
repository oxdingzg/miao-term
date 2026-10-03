# 排障

[English](TROUBLESHOOTING.md)

## 构建失败，或首次构建要好几分钟

| 要求 | 说明 |
|---|---|
| Rust | **stable** 工具链，锁定于 [`rust-toolchain.toml`](../rust-toolchain.toml);MSRV 1.80 |
| GPU | 支持 Metal(macOS)、Vulkan(Linux)或 DX12(Windows)的驱动 |
| Linux | 常见的 `winit`/`wgpu` 系统库(X11 或 Wayland 开发包) |

首次构建会编译 `wgpu` 与 `glyphon`，可能需要几分钟;之后的构建是增量的。

## macOS 提示应用来自未识别的开发者

`scripts/package-macos.sh` 产出的是 **ad-hoc 签名**的 bundle。只有在发布流程里配置了可选的签名
密钥，发布包才带 Developer ID 签名与公证;Windows MSI 签名同理。在此之前，macOS 会在首次打开时
要求确认，标准路径是「系统设置 → 隐私与安全性 → 仍要打开」。

可以确认当前用的是哪个构建:

```sh
mtty --version     # 输出 "mtty <版本号> (native)"
```

## 配置没有被读取

配置就是一个文件:

| 平台 | 路径 |
|---|---|
| Linux / macOS | `~/.config/mtty/config.toml`，或 `$XDG_CONFIG_HOME/mtty/config.toml` |
| Windows | `%APPDATA%\mtty\config.toml` |

两个常见的坑:

- **Git Bash 下的 Windows。** 若 Git Bash 设置的 `HOME` 下已存在 `~/.config/mtty`，会继续使用它，
  而不是 `%APPDATA%`。
- **从 miaotty 升级过来。** 首次启动时，若 `$XDG_CONFIG_HOME/mtty` 不存在，会把
  `$XDG_CONFIG_HOME/miaotty` 复制过去。旧目录会保留，所以改它不会生效 —— 请把改动挪到新路径。

如果完全不存在 mtty 配置，会自动导入 ghostty 的 `config` 与 alacritty 的 `alacritty.toml`，
所以某项设置也可能是从那里来的。见[配置](CONFIG.zh-CN.md)。

## 窗格不上报目录或历史

那是 shell shim 的职责。shim 先加载你自己的启动文件，且绝不修改它们:

| Shell | shim 的加载方式 |
|---|---|
| zsh | 一个 `ZDOTDIR`,其 `.zshenv` 会恢复真实的 `ZDOTDIR` |
| bash | `--rcfile`,先 source `~/.bashrc`;bash 4.4+ 用 `PS0`,更老的 bash(macOS 3.2)用 DEBUG trap |
| fish | 经 `XDG_DATA_DIRS` 找到的 `vendor_conf.d` 脚本 |
| PowerShell | 在 profile 之后用 `-NoExit -Command` 加载;PSReadLine 历史需 PowerShell 7 |

如果某个启动文件在 shim 运行之后重置 `ZDOTDIR`、`XDG_DATA_DIRS` 或 `PS0`，上报就会失效。这些 shim
都在真实 PTY 中做过端到端测试(Linux 上的 zsh、bash 3.2/5.x、fish 3.7、PowerShell 7.5;
Windows 上的 PowerShell 由 CI 运行)。

## 恢复会话后，之前运行的东西没回来

恢复重放的是**布局**，并启动新的 shell;正在运行的进程不会被保留。若要在重启后保留 shell:

```toml
pty-host = true      # Unix，实验性
```

## `mtty-cli` 连不上

| 检查项 | 说明 |
|---|---|
| socket | `$XDG_RUNTIME_DIR/mtty.sock`(回退到 `$TMPDIR`);用 `--socket PATH` 或设置 `MTTY_SOCKET` 覆盖 |
| 更早的客户端 | `miaotty.sock` 链接到 `mtty.sock`;`mtty-cli` 也会回退到更早宿主的 socket 或管道 |
| 远程 TCP | 未设 `MTTY_MTP_TOKEN` 时 `remote-listen` 拒绝启动;设了之后每个请求都必须携带令牌 |
| 能力 | `MTTY_MTP_ALLOW` 会以 `forbidden` 拒绝白名单之外的请求 |

`mtty-cli ping` 会在 `allowed` 里报告宿主实际允许的能力。见[控制面](CLI.zh-CN.md)。

## 再次启动应用没有反应

这是有意的:第二次启动会经已有的控制 socket 转发给正在运行的实例，而不是再起一个进程。要再开一个
终端，请用 `⌘⇧T` / `Ctrl+Shift+Alt+T`(快速终端)或新建标签。

## 链接，以及从 miaotty 升级

| 项目 | 行为 |
|---|---|
| URL scheme | macOS 与 Linux 注册 `mtty://`、`ssh://` 与 `x-man-page://`;Windows MSI 只注册 `mtty://` |
| `mtty://` 与 `miaotty://` | 两者都会打开 mtty |
| agent 钩子 | 已安装的钩子脚本与 miao 的集成读取 `MIAOTTY_PANE_ID` / `MIAOTTY_CLI`，这些变量仍然导出，因此它们会继续上报状态;新安装的钩子使用 `MTTY_*` 名字 |
| macOS 通知 | 更名改变了 bundle ID,macOS 会重新询问通知权限 |

完整对照见[应用身份与迁移](APP-IDENTITY.zh-CN.md)。

## 编辑器没有补全、诊断或悬停提示

编辑器窗格会为**登录 shell 的 `PATH`** 上存在的 `rust-analyzer`、
`typescript-language-server`、`pyright-langserver`、`gopls` 或 `clangd` 启动语言服务器。检查两点:

- 语言服务器未安装，或不在登录 shell 的 `PATH` 上 —— 应用不会读取某个 shell 的交互式别名。
- 文件超过 2 MB，不启动语言服务器。

显式设置 `[lsp] enabled = false` 会全部关闭。见[配置](CONFIG.zh-CN.md)。

## 这里没有我遇到的问题

其余文档在仓库里:[安装](INSTALL.zh-CN.md)、[视图规则](VIEW-RULES.zh-CN.md)，以及带注释的
[`config.example.toml`](config.example.toml)。其他问题请在
[oxdingzg/miao-term](https://github.com/oxdingzg/miao-term/issues) 开 issue，或写信到
<dingzg@mtty.dev>。
