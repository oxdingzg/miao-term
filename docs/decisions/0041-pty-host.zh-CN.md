# ADR 0041 — pane 跨应用重启存活(每 pane 一个 PTY 宿主进程)

> English (default): [`0041-pty-host.md`](0041-pty-host.md)

状态:提议中。

## 背景

升级 mtty 会杀掉 pane 里运行的一切。在 mtty 里启动的长时间 Claude Code、Codex 或 miao 会话,会在 *Update and Relaunch* 退出应用时结束。重启后的应用只恢复布局、工作目录、一份屏幕的 ANSI 副本,以及"重新运行上次命令"的提示(`restore_pane_contents`)。`docs/PRODUCT.md` 已写明活进程"不会在退出后存活(需要独立的 PTY 守护进程)"。

今天子进程与应用绑在一起的方式:

- `Terminal`(`crates/term-core/src/term.rs`)持有 `portable-pty` 的 master、writer 和 child。portable-pty 让每个子进程自成一个会话,以该 PTY 为控制终端。
- `Cmd::Quit`(菜单、Dock、注销,以及经 `install_update` 的更新路径)以 `std::process::exit(0)` 结束。随后内核关闭 master,PTY 挂断,shell 收到 `SIGHUP`。关窗口或关 pane 会走 `Drop for Terminal`,主动杀子进程(Windows 上是 `taskkill /T /F`,ADR 0027)。
- MTP 控制 socket(`term-mtp`)是应用内的一个线程。pane id 是每次运行重新计数的(`pane0`、`pane1`……),并以 `MTTY_PANE_ID` 导出给子进程。

只要 master 没被持有,子进程就会丢,所以 master 必须放在一个比应用活得更久的进程里。

## 决策

### 1. 每个 pane 一个小宿主进程

每个本地终端 pane 由自己的 **PTY 宿主** `mtty-ptyhost` 服务,类似 `dtach`。应用是宿主的客户端。宿主:

- 在自己持有的 PTY 上启动子进程(仍用 portable-pty),子进程运行期间一直保持 master 打开;
- 通过私有 socket 一次只接受一个客户端,转发输出、输入、尺寸变化和信号;
- 保留一个**输出环形缓冲**(默认 8 MiB),按"自子进程启动以来的绝对字节偏移"寻址;
- **不做终端模拟**:从不解析输出,也不回答终端查询。

为什么每 pane 一个,而不是一个守护进程管所有 pane:

- **升级**:宿主永远不需要被替换。老宿主随子进程结束而退出,新 pane 启动新宿主。单一守护进程要么让旧版本无限期活着,要么需要它自己的交接机制。
- **隔离**:一个宿主崩溃只影响一个 pane,而不是全部。
- **体积**:协议足够小,可以冻结(第 4 节)。

代价是每个 pane 多一个进程(常驻几 MB;不链接 GPU、字体或 UI 代码)。

`mtty-ptyhost` 是一个独立的小二进制,来自新的叶子 crate `term-ptyhost`(协议、宿主、客户端;依赖 `portable-pty`,不依赖引擎)。它和 `mtty-cli` 一起进入每个安装包。

启动宿主前,应用先把宿主二进制复制到按版本区分的用户目录(`<data dir>/mtty/ptyhost/<version>/`,只复制一次),从那里运行。以下三种情况让"从安装位置直接运行"不安全:

- AppImage 的挂载点会在应用退出时消失。
- MSI 无法替换正在运行的可执行文件。
- macOS 更新会把旧 `.app` 挪开并删除。

启动时会清理已无宿主使用的旧版本。

### 2. 屏幕由应用保存;重连时从偏移量回放

终端模型仍在应用里(`ATerm`;渲染、选择、搜索都不变)。托管的 `Terminal` 用宿主客户端作为字节流,而不是本地 PTY:这是 PTY 和 `from_pipe` 之外的第三种后端。

状态分两半保存:

1. **应用的快照**。现有的回滚快照扩展为完整的**状态快照**:
   - 两块屏幕,并标明当前是否在备用屏;
   - 光标位置、样式和可见性,以及滚动区域;
   - 改变输入或输出的模式:应用光标键和小键盘、括号粘贴、鼠标跟踪及其编码、焦点报告、kitty 键盘标志、原点模式;
   - 当前 SGR、标题、OSC 7 工作目录;
   - **该快照覆盖到的宿主偏移量**。

   快照在断开时原子写入,也随现有的每分钟一次保存写入。
2. **宿主的环形缓冲**,保存该偏移量之后的全部输出。

重连时,应用加载快照,请宿主从该偏移量开始回放,然后接上实时输出。

如果环形缓冲已经够不到那个偏移量(断开期间输出超过 8 MiB,或崩溃时快照较旧),应用会:

- 加载快照;
- 打印一行暗色的 `[mtty]` 提示,说明中间输出被截断;
- 从下一个换行处开始回放缓冲里还有的内容;
- 改一下尺寸再改回,让 `SIGWINCH` 促使全屏程序(包括 Claude Code)重绘。

没有客户端连着时,终端查询(DA、OSC 10/11 颜色、光标位置)得不到回复。等待回复的程序会走到自己的超时。这只影响程序启动阶段,而程序启动很少恰好碰上重启。

### 3. 各退出路径的行为

| 事件 | pane |
|---|---|
| 关闭 pane 或标签 | 应用发送 `Kill`;宿主挂断子进程(Unix:向其进程组发 `SIGHUP`,250 ms 后 `SIGKILL`;Windows:ADR 0027 的进程树强杀移入宿主),然后退出。 |
| 退出(菜单、⌘Q、Dock、注销) | `keep-sessions-on-quit = false`(默认):与今天一样,杀掉所有 pane。`true`:所有 pane 断开保留(类似 tmux)。 |
| **Update and Relaunch** | 一律断开保留。应用保存状态快照,向每个宿主发送 `Detach`,然后运行安装辅助脚本;重启后的应用恢复会话并重连。 |
| 应用崩溃 | 宿主发现连接关闭,继续以断开状态运行;下次启动用周期性快照重连(第 2 节的兜底方案覆盖中间的空缺)。 |
| 断开期间子进程退出 | 宿主保留退出状态和环形缓冲,直到客户端取走或超时;应用显示最后的输出和 `[exited]`。 |
| 超过 `detached-timeout`(默认 24 小时)无客户端 | 宿主挂断子进程并退出,避免更新失败或崩溃导致进程永久泄漏。 |

启动时,会话文件里没有记录的宿主(会话文件最后一次写入之后发生的崩溃留下的)会列在*已恢复的会话*里,由用户选择重连或结束,而不是被悄悄接管。

### 4. 协议(冻结的 v1)

在 Unix socket 或 Windows 命名管道上传输带长度前缀的二进制帧:`type: u8, len: u32 LE, payload`。

客户端 → 宿主:

- `Hello{proto, client_version}`
- `Attach{from_offset}`
- `Input(bytes)`
- `Resize{cols, rows, px_w, px_h}`
- `Kill`
- `Detach`
- `Query(Foreground | Cwd)`

宿主 → 客户端:

- `Welcome{proto, host_version, child_pid, started_at, caps}`
- `Output{offset, bytes}`
- `Truncated{oldest_offset}`
- `Exited{status, at_offset}`
- `Answer(...)`
- `Detached`(被另一个客户端接管)

宿主的寿命长于应用版本,所以兼容规则很严格:

- v1 永不修改。
- 新增内容只作为 `Welcome.caps` 里的可选能力位。
- 每个应用版本都保留它可能遇到的所有协议版本的客户端实现。
- 用固定帧(golden frame)测试锁定编码。

应用今天做的前台进程和 cwd 探测(`tcgetpgrp`、`process_cwd`)移入持有 master fd 的宿主。

### 5. 标识、发现与环境变量

- 会话文件为每个托管 pane 记录 `host: {id, socket}`。`id` 是随机的 128 位名字。
- socket 放在权限为 `0700` 的目录里:
  - Linux:`$XDG_RUNTIME_DIR/mtty/hosts/<id>.sock`
  - macOS:`$TMPDIR`,路径保持较短以满足约 100 字节的限制。
- 宿主检查对端 uid(`getpeereid`/`SO_PEERCRED`)。
- Windows 上的管道是 `\\.\pipe\mtty-host-<用户 SID>-<id>`,DACL 只授权当前用户,并拒绝远程客户端。
- 每个宿主还写一个 `<id>.json`(各 pid、版本、启动时间),供"已恢复的会话"列表使用。
- 重连的 pane **保留原来的 pane id**,因为子进程的环境里仍然是 `MTTY_PANE_ID=pane3`。id 计数器从已恢复的最大 id 之后开始。
- `MTTY_SOCKET` 是固定的每用户路径,新应用启动后 agent 钩子就能连上。

### 6. 启动宿主

- 应用以脱离自身的方式启动宿主:
  - Unix:`setsid`,标准输入输出接 `/dev/null`,忽略 `SIGHUP`。
  - Windows:`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB`,以脱离应用所在的 job object。
- 宿主以接受第一个 `Hello` 表示就绪。如果超过 500 ms,或宿主启动失败,该 pane 回退到进程内 PTY 并给出提示。这个功能永远不会让 pane 丢失。
- `pty-host = false` 完全关闭托管,即今天的行为。
- 以下平台行为待 P1 在真实桌面上确认:
  - GNOME/KDE 的应用 scope 在应用退出时不会带走宿主;
  - 经 LaunchServices 重启后,launchd 不会带走宿主;
  - Windows 上的宿主能挺过 MSI 的 `RestartManager`。

### 7. 重启期间的控制面

MTP socket 仍在应用内,所以在重启的一两秒内不可用。

- `mtty-cli` 和 shell/agent 钩子把"连接被拒绝"视为*应用正在重启*,最多重试 5 秒(`--wait`)。之后钩子丢弃该事件,而不是让 agent 失败。
- 应用不在线期间发生的事件(比如某个 agent 完成了)不会补发。pane 自己的输出里仍然保留着这些信息。

### 8. 性能

托管多了一跳 socket:输出多一次拷贝和唤醒,输入也多一次。

- 性能门禁(ADR 0018/0023)增加输入延迟和吞吐测试的托管版本,预算不变。
- 宿主批量发送输出帧,以 16 ms 输入延迟目标为上限。
- 如果托管模式的吞吐达不到预算,托管保持为可选项。

## 分阶段

| 阶段 | 内容 | 验收 |
|---|---|---|
| P0 | 状态快照(模式、两块屏幕、偏移量)取代 ANSI 副本,用于本地恢复 | 往返测试:快照 → 新 `ATerm` → 网格与模式一致;在 macOS 桌面上截图核对 vim、less、Claude Code |
| P1 | `term-ptyhost` crate、宿主二进制、Unix 客户端后端、`pty-host` 设置(默认关闭)、版本化复制 | 宿主集成测试(启动、断开、从偏移量重连、截断、kill、超时、对端检查);在 macOS 和 Linux 桌面上 kill -9 应用后重连 |
| P2 | 更新路径断开并重连;保留 pane id;已恢复会话列表;`mtty-cli --wait` | 在 macOS 桌面(.app)和 Linux 桌面(AppImage)上从上一个正式版真实更新,更新时 Claude Code 正在执行:它继续运行,重启后 pane 显示其输出 |
| P3 | Windows 宿主(ConPTY、命名管道、job 脱离、复制到 `Program Files` 之外的版本化目录) | 在 Windows 桌面上用 MSI 做同样的更新测试 |
| P4 | `keep-sessions-on-quit`、`detached-timeout`、托管模式的性能门禁测试 | 开启托管时门禁通过 |
| P5 | 经过一个版本的试用后,`pty-host` 默认开启 | — |

SSH、串口、Telnet、TCP pane 不在范围内。它们的状态在远端,SSH 的 *keep the shell in tmux* 选项已经覆盖。挺过注销或重启电脑也不在范围内。

## 后果

- 升级不再打断正在运行的 agent;GUI 崩溃也不再杀掉 pane。
- 每个本地 pane 多一个进程,外加一个冻结的线协议:只要还可能有老宿主在运行,就必须继续支持它。
- 状态快照(P0)即使在关闭托管时也能改进今天的恢复:全屏程序和输入模式能正确恢复。
- 打包在四个目标上多一个二进制(`release.yml`、MSI、AppImage、deb、`.app`)。
- 本 ADR 接受后,`docs/ARCHITECTURE.md` 补充宿主进程及其 crate;D6 的平台边界扩展到 `term-ptyhost`。

## 待定问题

1. 普通退出的默认行为:像今天一样杀掉,还是保留(类似 tmux)?
2. `detached-timeout` 的默认值(提议 24 小时)。
3. 每 pane 的环形缓冲大小(提议 8 MiB;总内存随 pane 数增长)。
