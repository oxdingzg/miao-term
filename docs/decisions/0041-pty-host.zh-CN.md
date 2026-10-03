# ADR 0041 — pane 跨应用重启存活(每 pane 一个 PTY 宿主进程)

> English (default): [`0041-pty-host.md`](0041-pty-host.md)

状态:已接受(2026-10-03,采用提议的默认值:普通退出结束 pane,`detached-timeout` 24 小时,环形缓冲 8 MiB)。已对照 `alacritty_terminal` 0.25 和现有恢复代码评审并修订,见文末*评审发现*。

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
- **不做屏幕模拟**,也从不回答终端查询。它只对输出运行一个小扫描器(第 4 节),找出转义序列和 UTF-8 字符的边界,并跟踪私有模式(DECSET/DECRST、kitty 键盘标志、备用屏)。

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

1. **应用的状态快照**,在断开时原子写入,也随现有的每分钟一次保存写入。它必须*精确*复原屏幕,而不只是看起来一样:Claude Code(Ink)和所有全屏程序都用相对光标移动来重绘,屏幕一旦被重排或光标错位,回放的字节就会画到错误的行上。快照包含:
   - 拍摄时的尺寸;
   - 历史行,以带 SGR 的文本保存(光标到不了历史区,之后调整尺寸时重排也无妨);
   - 主屏的可见行,以及备用屏激活时备用屏的可见行,以精确的单元格保存(`Cell` 实现了 `Clone` 和 `Serialize`,字段公开:字符、颜色、包括 `WRAPLINE` 在内的标志、零宽字符、超链接);
   - 每块屏幕的光标和保存的光标(位置、模板单元格、字符集、待换行标志;都是 `Grid` 的公开字段);
   - `TermMode`(光标键、小键盘、括号粘贴、鼠标模式及编码、焦点报告、原点、插入、自动换行、当前 kitty 键盘标志),以及光标样式、标题、OSC 7 目录;
   - **它覆盖到的宿主偏移量**。这个偏移量总是某个完整 `Output` 帧的末尾,因而一定是序列边界(第 4 节)。应用自己无法找到边界:vte 的解析器状态是私有的。

   在备用屏下读取主屏要经过 `swap_alt`,而它会清空备用屏。快照会先复制备用屏的单元格再写回,所以不再破坏任何内容,程序还在绘制时也能运行。
2. **宿主的环形缓冲**,保存该偏移量之后的输出,按整帧淘汰。

重连时,应用**按快照的尺寸**新建屏幕,加载快照,从该偏移量回放环形缓冲,最后才把 pane 调整到当前尺寸。

**加载和回放期间不发送查询回复。** 屏幕模型会对解析到的查询(DA、光标位置、OSC 10/11 颜色)写 PTY 作答;回放的输出里有程序以前发过的查询,再答一遍就等于往正在运行的程序里敲入转义序列。

**有些状态读不回来。** `alacritty_terminal` 0.25 把滚动区域、tab stop、标题栈和 kitty 键盘栈的深度设为私有。因此每次重连后,应用都把 pane 尺寸来回调一次,让 `SIGWINCH` 促使全屏程序和 Claude Code 重绘(重绘时它们会重新设置滚动区域)。不在回放范围内的内联图片不会恢复。

如果环形缓冲已经够不到快照的偏移量(断开期间输出超过 8 MiB,或崩溃时快照较旧),应用会加载快照,应用宿主随 `Truncated` 报告的私有模式(丢失的字节里模式可能已经变了),打印一行暗色的 `[mtty]` 提示说明输出被截断,从最老的一帧开始回放环形缓冲,并像上面那样调一次尺寸。

没有客户端连着时,终端查询(DA、OSC 10/11 颜色、光标位置)得不到回复。等待回复的程序会走到自己的超时。这只影响程序启动阶段,而程序启动很少恰好碰上重启。

### 3. 各退出路径的行为

| 事件 | pane |
|---|---|
| 关闭 pane 或标签 | 应用发送 `Kill`;宿主挂断子进程(Unix:向其进程组发 `SIGHUP`,250 ms 后 `SIGKILL`;Windows:ADR 0027 的进程树强杀移入宿主),然后退出。 |
| 退出(菜单、⌘Q、Dock、注销) | `keep-sessions-on-quit = false`(默认):与今天一样,杀掉所有 pane。应用必须在 `process::exit` 之前向每个宿主发送 `Kill`,因为单纯退出现在会让它们继续运行。`true`:所有 pane 断开保留(类似 tmux)。 |
| **Update and Relaunch** | 一律断开保留。应用保存状态快照,向每个宿主发送 `Detach`,然后运行安装辅助脚本;重启后的应用恢复会话并重连。 |
| 应用崩溃 | 宿主发现连接关闭,继续以断开状态运行;下次启动用周期性快照重连(第 2 节的兜底方案覆盖中间的空缺)。 |
| 断开期间子进程退出 | 宿主保留退出状态和环形缓冲,直到客户端取走或超时;应用显示最后的输出和 `[exited]`。 |
| 超过 `detached-timeout`(默认 24 小时)无客户端 | 宿主挂断子进程并退出,避免更新失败或崩溃导致进程永久泄漏。 |

启动时,会话文件里没有记录的宿主(会话文件最后一次写入之后发生的崩溃留下的)会列在*已恢复的会话*里,由用户选择重连或结束,而不是被悄悄接管。

### 4. 协议(冻结的 v1)

在 Unix socket 或 Windows 命名管道上传输带长度前缀的二进制帧:`type: u8, len: u32 LE, payload`。

要运行什么(程序、参数、包括 `MTTY_PANE_ID` 在内的环境变量、工作目录、初始尺寸)在启动宿主时通过命令行和环境变量交给它,不放进协议,这样 v1 只涉及运行中的会话。

宿主只在序列边界处切分 `Output` 帧:扫描器会暂存不完整的转义序列或 UTF-8 字符,直到它完整;超过 5 ms 或 4 KiB 时无论如何都会发出,避免一个孤立的 `ESC` 卡住输出。偏移量为 `u64`。

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
- `Truncated{oldest_offset, modes}`(扫描器跟踪的私有模式状态)
- `Exited{status, at_offset}`
- `Answer(...)`
- `Detached`(被另一个客户端接管)

宿主的寿命长于应用版本,所以兼容规则很严格:

- v1 永不修改。
- 新增内容只作为 `Welcome.caps` 里的可选能力位。
- 每个应用版本都保留它可能遇到的所有协议版本的客户端实现。
- 用固定帧(golden frame)测试锁定编码。

应用今天做的前台进程和 cwd 探测(`tcgetpgrp`、`process_cwd`)移入持有 master fd 的宿主,并改为异步:应用缓存最近一次回答,退出时保存会话用缓存,而不是阻塞等待宿主。

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
- 启动宿主永远不阻塞 UI 线程:pane 立即出现,在宿主回 `Welcome` 之前输入的内容排队等待,性能门禁的首帧预算不变。
- 如果 `Welcome` 超过 500 ms 才到,或宿主启动失败,该 pane 回退到进程内 PTY 并给出提示。这个功能永远不会让 pane 丢失。
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
| P0 | 在 `term-core` 中实现状态快照与恢复(`ATerm::snapshot_state` / `restore_state`):备用屏下不破坏内容,恢复时不发查询回复;周期保存覆盖运行全屏程序的 pane | 等价测试:对录制的字节流 *A* 和 *B*(shell、类 vim 备用屏、Ink 式重绘、宽字符、软换行),在 *A* 之后拍快照、恢复、再喂 *B*,得到的单元格、光标和模式,与不间断地喂 *A + B* 完全一致;在 macOS 桌面上截图核对 vim、less、Claude Code |
| P1 | `term-ptyhost` crate、宿主二进制、Unix 客户端后端、`pty-host` 设置(默认关闭)、版本化复制 | 宿主集成测试(启动、断开、从偏移量重连、截断、kill、超时、对端检查);在 macOS 和 Linux 桌面上 kill -9 应用后重连 |
| P2 | 更新路径断开并重连;保留 pane id;已恢复会话列表;`mtty-cli --wait` | 在 macOS 桌面(.app)和 Linux 桌面(AppImage)上从上一个正式版真实更新,更新时 Claude Code 正在执行:它继续运行,重启后 pane 显示其输出 |
| P3 | Windows 宿主(ConPTY、命名管道、job 脱离、复制到 `Program Files` 之外的版本化目录) | 在 Windows 桌面上用 MSI 做同样的更新测试 |
| P4 | `keep-sessions-on-quit`、`detached-timeout`、托管模式的性能门禁测试 | 开启托管时门禁通过 |
| P5 | 经过一个版本的试用后,`pty-host` 默认开启 | — |

SSH、串口、Telnet、TCP pane 不在范围内。它们的状态在远端,SSH 的 *keep the shell in tmux* 选项已经覆盖。挺过注销或重启电脑也不在范围内。

## 后果

- 升级不再打断正在运行的 agent;GUI 崩溃也不再杀掉 pane。
- 每个本地 pane 多一个进程,外加一个冻结的线协议:只要还可能有老宿主在运行,就必须继续支持它。
- 关闭托管时,恢复的 pane 运行的是*新的* shell,所以只显示快照的内容,从不应用其中的模式(新 shell 并不处于 vim 的备用屏或鼠标模式)。P0 对这条路径仍有帮助:快照不再破坏备用屏,周期保存也能覆盖运行全屏程序的 pane。
- 打包在四个目标上多一个二进制(`release.yml`、MSI、AppImage、deb、`.app`)。
- 本 ADR 接受后,`docs/ARCHITECTURE.md` 补充宿主进程及其 crate;D6 的平台边界扩展到 `term-ptyhost`。

## 补充:P1 的实际实现(2026-10-03)

- **协议 v1** 以 `crates/term-ptyhost/src/proto.rs` 为准(有固定帧测试):发往宿主的有 `Hello`、`Attach`、`Input`、`Resize`、`Kill`、`Detach`;宿主发出的有 `Welcome`、`Output{offset, boundary}`、`Truncated{oldest, modes}`、`Live{offset}`、`Exited`、`Detached`。`Live` 标记回放结束,应用据此恢复回复查询并调整尺寸。没有 `Query`/`Answer`:应用用 `Welcome` 里的子进程 pid 直接向内核读取前台进程组(macOS 用 `proc_pidinfo` 的 `tpgid`,Linux 读 `/proc/<pid>/stat`),不需要往返。
- 在 `config.toml` 里写 `pty-host = true` 开启(默认关闭)。宿主二进制 `mtty-ptyhost` 需放在 `mtty` 旁边,使用前复制到 `<data dir>/ptyhost/<version>/`。打进安装包属于 P2。
- 宿主 2 秒内无响应时,该 pane 在应用内运行 shell 并显示一行暗色提示;宿主根本无法启动时静默回退。
- 恢复会话时先尝试重连,并保留 pane id;每分钟一次的保存会在 ANSI 副本旁写出 `<pane>.host.json`(状态加偏移量)。没有快照时回放宿主的整个环形缓冲。
- 已验证:宿主测试(连接、从偏移量续接、带模式的截断、断开期间退出、超时、接管、序列边界);托管 `Terminal` 跨模拟重启(输出不重复、回放的查询不被回答、能识别前台进程);在 macOS 和 Linux 桌面上 `kill -9` 应用后,输出无缝衔接,pane id 不变。

## 补充:P2 的实际实现(2026-10-03)

- **Update and Relaunch** 以保留会话的方式退出:为每个托管 pane 保存状态和偏移量(`<pane>.host.json`),断开宿主,重启后的应用重新接上。没有宿主的 pane 和普通退出一样结束。
- 命令面板新增 *重启 mtty(保留运行中的程序)*(开启 `pty-host` 时显示),不经过更新走同一条路径。端到端测试就是靠它完成的:真实更新需要带发布签名的安装包。
- `mtty-ptyhost` 随 macOS bundle、Linux tar 包、AppImage 和 deb 发布(`release.yml`、`scripts/package-macos.sh`、deb assets)。
- 仍在运行、但恢复的 pane 没有接回的宿主(会话保存前发生的崩溃留下的),启动时会提示,并在命令面板中以 *接回 / 结束运行中的程序* 列出。
- `mtty-cli` 在 pane 内运行时(设置了 `MTTY_PANE_ID`)遇到连接被拒会重试 5 秒,让 agent 钩子能连上重启后的应用;`--wait SECS` 可在任何地方指定等待时间。
- 已在 macOS 和 Linux 桌面上验证:重启期间运行中的循环无缝继续,旧应用退出,快照被写出并使用,pane id 保持不变;被 `kill -9` 留下的孤立宿主可以从命令面板接回。从一个正式版到下一个正式版的真实更新,在下次发版时检查。

## 补充:P3 的实际实现(2026-10-03)

- **传输。** Windows 宿主使用 AF_UNIX socket(Windows 10 1803 起支持,低于 ConPTY 自身要求的 1809),而不是第 5 节计划的命名管道:AF_UNIX 是全双工的,读线程和写线程可以共用一个连接,同步命名管道的单个句柄做不到。socket 放在 `%TEMP%\mtty-hosts`,由用户配置目录的 ACL 保证私有;Windows 上不做对端检查。`uds_windows`(MIT)本来就在依赖树里。
- **启动。** ConPTY 在输出任何内容之前会先查询光标位置(`ESC[6n`)。宿主自己回答第一个这样的查询(`1;1`:pane 是新的),并把它从输出流中去掉,这样宿主从不需要等客户端,回放时也不会再回答一次。
- **进程。** 宿主以脱离控制台的方式创建,在应用所在 job 允许时脱离该 job。它*不*使用独立进程组:`CREATE_NEW_PROCESS_GROUP` 会在新进程中关闭 Ctrl+C,shell 会继承这一点,导致在 pane 里按 Ctrl+C 无法送达程序(在桌面验收时发现,现已有测试覆盖)。宿主启动时还会重新开启 Ctrl+C 处理。结束程序时对其进程树执行 `taskkill /T /F`(ADR 0027)。宿主二进制从 `%LOCALAPPDATA%\mtty\ptyhost\<version>\` 运行,MSI 更新时不会遇到正在使用的文件;正在使用的副本保持不变,不会被替换。
- **打包。** `mtty-ptyhost.exe` 随 MSI 和 zip 发布,zip 方式的更新也会复制它。*重启 mtty(保留运行中的程序)* 使用 `.cmd` 辅助脚本。
- 已在 Windows 11 上验证:基于 ConPTY 的宿主测试(连接、从偏移量续接、Ctrl+C 送达程序、断开期间退出的退出码、断开超时结束整个进程树、接管),以及用 `cmd.exe` 的托管 `Terminal` 跨模拟重启;在桌面上,运行中的循环挺过了 *重启 mtty(保留运行中的程序)*;对应用执行 `taskkill /F` 后,运行中的 `ping` 被重新接上、能用 Ctrl+C 打断,pane 继续接受输入且 id 不变。

## 补充:P4 的实际实现(2026-10-03)

- `keep-sessions-on-quit`(默认 `false`):普通退出(菜单、⌘Q、关闭窗口)时,像更新一样保存每个托管 pane 的状态并断开宿主,而不是结束它。`detached-timeout`(默认 `24h`;支持 `90s`、`30m`、`24h`、`7d` 或秒数)会传给每个新宿主。
- 性能门禁:`crates/term-ptyhost/tests/perf.rs` 测量按键经宿主的回显往返(`hosted_echo_p95_ms`,预算 4 ms)和经宿主的输出吞吐(`hosted_output_mbps`,预算 25 MB/s,即屏幕解析的预算),各自与测试直接持有的 PTY 上的同样工作对比。
- 第一次测量发现,Linux 上经宿主的输出只有直连 PTY 的 37%–79%:Linux 的 PTY 每次读取只给几百字节,而宿主每读一次就发一帧。现在宿主在一个线程里把读到的内容放进有界队列,在另一个线程里把队列中已有的全部内容一次组成一帧(最多 256 KiB):输出密集时自然攒成大帧,空闲时的输出仍然立即发出。现在 Linux 上经宿主的输出是直连的 89%–112%,macOS 上超过直连;回显的 p95 从约 0.025 ms 变为约 0.07 ms,相对 16 ms 的按键到字形预算可以忽略。

## 评审发现(2026-10-03)

初稿在动手实现前,对照 `alacritty_terminal` 0.25.1、vte 0.15 和恢复代码做了核对。上文已做的修改:

1. **精确的屏幕。** 初稿沿用 ANSI 回滚副本,它会合并软换行、丢掉末尾空行、丢失光标位置,回放的相对光标移动会因此画乱屏幕。现在快照以精确单元格保存可见行和光标,并按自身尺寸恢复。
2. **不发回复。** 通过屏幕模型回放旧输出,会把旧查询的回答写进正在运行的程序。
3. **序列边界。** 快照偏移量如果落在转义序列或 UTF-8 字符中间,回放就会出错,而应用看不到 vte 的解析器状态;改由宿主在边界处切帧。
4. **新 shell 上的模式。** 初稿声称快照能为今天(进程已不在)的恢复还原模式,那样会让新 shell 处于已退出程序的模式里。
5. **读不回的状态**(滚动区域、tab stop、键盘栈深度):已写明,并改为每次重连后都调一次尺寸,而不只是在截断后。
6. **截断时丢失的模式:** 由宿主的扫描器报告。
7. **缺少启动描述、启动阻塞、退出时同步探测前台进程、退出后宿主仍存活:** 已在上文写明。

## 已定问题

已采用默认值:普通退出结束 pane,`detached-timeout` 为 24 小时,每个宿主保留 8 MiB 环形缓冲。
