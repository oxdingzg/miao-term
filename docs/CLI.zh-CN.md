# mtty-cli 控制面

[English](CLI.md)

`mtty-cli` 让脚本或其他程序驱动正在运行的 mtty 宿主。它通过每用户套接字与应用程序通话，说的是
**MTP**(mtty terminal protocol)，并且是该协议的参考客户端。

## 套接字

控制面监听 `$XDG_RUNTIME_DIR/mtty.sock`(回退到 `$TMPDIR`)，且 socket 以仅属主可访问的权限创建。
Windows 上为命名管道。

每个窗格中的 shell 都会继承两个变量:

| 变量 | 含义 |
|---|---|
| `MTTY_SOCKET` | 要通信的 socket |
| `MTTY_PANE_ID` | 该 shell 所属的窗格 |

用 `--socket PATH` 或设置 `MTTY_SOCKET` 可以指定非默认 socket;`mtty-cli` 也会回退到更早版本的宿主
所用的 socket 或管道。

## 状态版本号与事件

每个响应都带状态 `revision`。有两个命令据此替代轮询:

- `core.wait` 会阻塞到该值超过给定值后再返回。
- `core.subscribe` 把连接升级为事件流，客户端据此跟踪变化。

事件 topic 有 `agent.state`、`panes` 与 `history`。

## 命令

```sh
mtty-cli ping
mtty-cli wait --since 42               # 阻塞直到状态 revision 变化
mtty-cli events                        # 以 JSON 行流式输出状态变化
mtty-cli events --topic agent.state    # ...仅订阅某个 topic
mtty-cli pane list
mtty-cli pane run --pane ID --data "echo hello"
mtty-cli pane focus --pane ID
mtty-cli pane output --pane ID           # 上一条命令的输出与退出码
mtty-cli state claude --state processing --pane ID
mtty-cli state list
mtty-cli agent sessions                 # 列出可恢复的代理会话
mtty-cli agent resume --pane ID         # 恢复所选窗格的代理
mtty-cli agent resume --session ID      # 恢复发现的会话
mtty-cli state miao --state processing --pane ID --runtime-context context.json
mtty-cli history add --command "cargo test" --cwd "$PWD"
mtty-cli history list --pane ID
mtty-cli view /path/to/file            # 在应用中以只读方式打开
mtty-cli edit /path/to/file            # 在编辑器中打开
mtty-cli file read  --path /etc/hosts  # 单次上限 2 MB
mtty-cli file read  --path app.bin --base64 --offset 0 --length 65536
mtty-cli file write --path /tmp/x --data "hello"
mtty-cli file write --path /tmp/x --data-b64 "AAECAw=="   # 二进制
```

| 命令组 | 覆盖什么 |
|---|---|
| `ping` | 存活探测，以及宿主允许哪些能力 |
| `wait`、`events` | 阻塞等待或订阅状态变化 |
| `pane` | 列出窗格、在其中执行命令、聚焦窗格、读取上一条命令的输出与退出码 |
| `state` | 上报窗格状态；`--runtime-context FILE`（或 `-` 从 stdin 读取）提供结构化 miao Runtime 上下文 |
| `agent` | 发现可恢复的代理会话，或在窗格中恢复 |
| `history` | 写入或读取窗格的命令历史 |
| `view`、`edit` | 在应用中打开文件，只读或在编辑器中 |
| `file` | 经由宿主读写文件，单次上限 2 MB |

文件读写单次上限为 2 MB;`--base64` 与 `--data-b64` 用于二进制内容，`--offset` 与 `--length` 用于
分页读取更大的文件。

## 远程访问

设 `remote-listen = "127.0.0.1:7273"` 即可用 TCP 暴露控制面，然后连接:

```sh
mtty-cli --socket tcp://host:7273 pane list
```

没有令牌时，TCP 监听会拒绝启动。

## 令牌与能力

控制面能在你的 shell 里执行命令，因此令牌务必保密，并尽量只监听你自己控制的 loopback 地址，或走
ssh 隧道。

| 设置 | 作用 |
|---|---|
| `MTTY_MTP_TOKEN` | 宿主以此启动时，每个请求都必须携带该令牌;CLI 会从同一环境变量读取 |
| `MTTY_MTP_ALLOW` | 逗号分隔的能力白名单，如 `core.basic,file.read,history.read`;其余返回 `forbidden` |

不设 `MTTY_MTP_ALLOW` 表示全部允许;`core.basic`(ping/health)始终允许，以便客户端发现宿主。
`ping` 会在 `allowed` 里报告生效的能力集。

socket 可经 ssh 转发，从而让远端客户端驱动宿主:

```sh
ssh -R /tmp/fwd.sock:<宿主 socket>
```

## 直接对接协议

MTP 是套接字上的换行分隔 JSON，客户端不必依赖 `mtty-cli`:写一行请求，读一行响应即可。请使用
`core.wait` 或 `core.subscribe`，不要循环轮询。
