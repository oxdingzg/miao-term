# 远程主机与 SSH

[English](REMOTE.md)

mtty 的远程操作建立在系统自带的 **OpenSSH** 客户端之上：你连接的目标机器**无需安装任何东西**，凭证始终留在 OpenSSH 和操作系统本来管理它们的地方——`ssh-agent`、系统钥匙串，以及 `~/.ssh` 下的密钥文件。mtty 从不保存明文密码，这一切也不需要账号：主机库和下面每一项操作都可离线使用，并只留在本机。

这里讲的是远程操作的终端侧。若要用脚本或另一个程序驱动正在运行的 mtty，见 [mtty-cli 控制面](CLI.zh-CN.md)。

## 主机库

保存的主机放在 `~/.config/mtty/hosts.toml`（与 [`config.toml`](CONFIG.zh-CN.md) 同目录；Windows 下是 `%APPDATA%\mtty\hosts.toml`）。只写一个 `[[host]]` 也是合法文件：

```toml
[[host]]
name    = "prod-web"
address = "prod-web.example.com"
user    = "deploy"
port    = 22
group   = "production"
tags    = ["web", "eu"]
```

| 键 | 含义 |
|---|---|
| `name` | 侧边栏、命令面板和 `views.json` 中显示的名称 |
| `alias` | 用 `ssh <name>` 连接，地址、用户、端口和选项都交给 `~/.ssh/config` |
| `address` / `user` / `port` | 不使用别名时的连接目标 |
| `group` / `tags` | 侧边栏的分组与搜索 |
| `jump` | 跳板机（`ssh -J`），用于没有写进 `~/.ssh/config` 的主机 |
| `mosh` | 用 `mosh` 而非 `ssh` 连接（两端都需安装；可跨越网络切换与休眠） |
| `tmux` | 把远程 shell 保持在指定 tmux 会话里，重连即回到原处 |
| `forward` | 保存的端口转发（见下） |
| `kind` | `ssh`（默认）、`serial`、`telnet` 或 `tcp`（见下） |

从 `~/.ssh/config` 导入的主机会保留为 `alias` 条目，因此你在那里已经设好的选项——`ProxyJump`、`IdentityFile`、`ControlMaster`——继续生效。主机库在文件变化时重新读取；解析失败的文件会被报告，且绝不会被覆盖写回。

可从侧边栏的 **HOSTS** 区域、命令面板，或通过 `mtty://host/<name>` URL scheme 打开已保存的主机。

## 连接

*New SSH Session…*（Shell 菜单和命令面板）在新标签页里打开一个登录。连接使用你的 `~/.ssh/config`，存在 `ControlMaster` 连接时复用它，并且不在远端安装任何 `terminfo`——mtty 通过连接把它需要的条目送过去。

主机密钥按 `ssh` 的方式校验：**已知**主机静默通过；**未知**主机显示指纹，只有你比对确认后才信任；**变更**的密钥会被拒绝。生成密钥和 `ssh-copy-id` 都在普通终端标签页里运行，口令输入给 `ssh`，不经过 mtty。

恢复的 SSH 标签页会显示为已断开，按 Enter 重连。设 `ssh-auto-reconnect = true` 可在启动时直接连接。

## 端口转发

转发随主机一起保存，沿用 ssh 自己的写法。每条转发是独立连接，并显示在 **Ports** 面板里。

```toml
[[host]]
name = "prod-web"
[[host.forward]]
kind = "local"                       # local (-L) | remote (-R) | dynamic (-D)
spec = "8080:127.0.0.1:80"           # [bind:]port:host:hostport；-D 时为 [bind:]port
```

## 文件传输

双栏 **SFTP** 浏览器以批处理模式调用 OpenSSH 的 `sftp` 客户端，因此保留你的 `~/.ssh/config`、跳板机、agent 和 `ControlMaster`。上传、下载和拖放都在后台进行并显示进度；远端确认写入前，上传显示为忙碌状态而不是百分比。

单个远程文件也可以就地查看和编辑：通过 ssh 打开它，`⌘S` 以同样方式写回。远程窗格会轮询该文件，在没有未保存修改时随其变化就地重新加载。

## 片段与广播

可复用的命令放在 `~/.config/mtty/snippets.toml`：

```toml
[[snippet]]
name    = "disk usage"
command = "df -h"
tags    = ["ops"]
```

片段可以在当前窗格运行，也可以选多台主机、每台各开一个标签页运行——引号处理照旧交给 `ssh -t`。**广播输入（Broadcast input）** 会把你输入的内容同时送进多个窗格，适合让一组主机保持同样的操作。

## 串口、Telnet 与裸 TCP

`kind` 选择传输方式。*New Serial/Telnet/TCP Session…* 在 Shell 菜单和命令面板里；各类型的已保存条目都能像 SSH 主机一样从侧边栏和 `mtty://host/<name>` 打开。

```toml
[[host]]
name = "console"
kind = "serial"
[host.serial]
device = "/dev/ttyUSB0"
baud   = 115200
```

Telnet 和裸 TCP 不加密，并会被明确标注；串口则根本不是网络传输。这些会话在连接断开时结束，shell 集成（工作目录、命令历史）对它们不适用。

## PuTTY 密钥

*Hosts… → Import PuTTY Key…* 读取 PuTTY 的 `.ppk`（v2 与 v3，Ed25519、RSA、ECDSA），校验后用你选择的口令重新编码为 OpenSSH 密钥。不存在未加密的输出路径。

## 安全

mtty 只在 OpenSSH 会读取的地方读取私钥：它自己不持有解密后的密钥，也不会把它们转发到任何地方。刻意使用系统 OpenSSH——macOS、Linux 和 Windows 都自带它；转向 Rust 原生栈是一个有记录、可重新评估的决定，而非偶然。

报告漏洞见[安全](SECURITY.zh-CN.md)。控制面自身的边界见 [mtty-cli](CLI.zh-CN.md)。
