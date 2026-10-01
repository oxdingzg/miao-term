# mtty 产品需求与路线图

[English](PRODUCT.md)

> 基线:`v0.0.5` 加提交 `689cbce`(稳定性修复),日期 2026-10-01。
> 本文是功能需求的唯一出处:README 只描述"现在能用的",本文还记录"要做什么、按什么顺序、怎样算完成"。

## 1. 定位

**mtty** 是一个跨平台(macOS / Linux / Windows)的**终端入口**:日常开发、AI 编码 agent 与远程运维
都从同一个窗口开始。

| 来源 | 吸收什么 | 不吸收什么 |
|---|---|---|
| [Otty](https://otty.sh/) | 现代本地终端体验:标签/分屏、命令面板、文件查看、Agent 徽章、Composer、会话恢复 | — |
| [Termius](https://termius.com/) | 远程运维:主机库与分组、SSH 密钥/身份、SFTP、端口转发与跳板机、Snippets | 账号体系、强制云同步、订阅墙 |
| [mtty](https://mtty.dev/) | 多 agent 并行:每个任务一个 git worktree、统一看状态、diff 审阅后合并 | 只服务 agent 的 IDE 形态 |
| miao | 原生集成:无需 hook 的状态上报、会话恢复、经 MTP 双向控制 | — |

一句话:**本地终端要快而稳,agent 要看得见、管得住,远程主机要像本地目录一样好用。**

### 1.1 命名

- 正式产品名:**mtty**。仓库与可嵌入引擎仍叫 **miao-term**。
- 可执行文件 `mtty` / `mtty-cli`,bundle `mtty.app`(`dev.mtty.terminal`),配置目录
  `~/.config/mtty`,环境变量 `MTTY_*`,URL scheme `mtty://`。v0.0.5 及之前名为 `miaotty`,
  兼容规则见 [ADR 0032](decisions/0032-rename-mtty.zh-CN.md)。

### 1.2 目标用户与核心场景

1. **开发者本地终端**:多项目、多标签、分屏,中文路径和中文输出正常显示与查找。
2. **AI 编码 agent 用户**:同时跑 codex / claude / opencode / miao,知道谁在忙、谁在等我、
   谁出错了,能把提示排队交给它们。
3. **运维/全栈**:管理几十台主机,SSH 登录、传文件、转发端口、批量执行常用命令,
   凭证不离开本机。

## 2. 交互设计原则

这些原则是验收标准的一部分,新功能评审时逐条核对。

1. **终端优先**:终端区域永远是主角;面板、对话框不能抢走终端输入,
   也不能让按键同时作用于两处(文本框有焦点时 ⌘C/⌘V/⌘A 作用于文本框)。
2. **键盘优先、一个入口**:每个功能都能从命令面板(`⌘K`)找到;常用功能有快捷键,
   并且快捷键在菜单、面板、文档三处一致。
3. **失败必须可见**:任何用户发起的操作失败(保存、打开、连接、写配置)都要在界面上看到原因;
   不能只写 stderr,不能假装成功。
4. **不丢数据**:未保存的修改关闭前要确认;保存失败保持"已修改";写配置不覆盖解析失败的文件。
5. **不替用户改配置**:不改 shell 点文件、agent 配置、`~/.ssh/config`;需要接入时给出片段让用户自己粘贴。
6. **远端零安装**:SSH 能力不要求远端安装任何东西。
7. **凭证留在本机**:优先使用 ssh-agent 与系统钥匙串;不明文保存密码;云同步(若做)必须端到端加密且可完全关闭。
8. **UI 线程不阻塞**:网络、外部进程、大目录读取都在后台线程,界面显示进度并可取消。
9. **中英双语**:界面与文档都提供英文和简体中文。

## 3. 功能基线(当前版本)

状态含义:

- **可用**:用户入口可达、实现完整,有自动化测试或真实窗口冒烟证据。
- **部分**:能用,但有明确缺口(列在备注里,并进入 M0)。
- **未提供**:曾被文档宣称或用户期望,但当前 native 应用没有;已从 README 移除,进入路线图。

验证证据:`cargo test --workspace`(183 项)、`cargo clippy -D warnings`、
`scripts/smoke-hosts.py`(真实窗口 + 真实 shell + GPU 截图),以及本次对 native 代码的逐项核对。

### 3.1 终端内核

| 功能 | 状态 | 备注 |
|---|---|---|
| PTY + VT 解析(alacritty_terminal),GPU 字形网格渲染 | 可用 | 性能门见 PERFORMANCE |
| 回滚、选区、复制粘贴、中日韩宽字符与字体回退 | 可用 | 宽字符复制有回归测试 |
| 查找 `⌘F`(高亮 + 计数) | 可用 | 本次修复:中文查询不匹配、含中文行高亮错位 |
| 键盘编码:Ctrl/Alt/修饰键导航、F1–F12、Insert | 可用 | 本次修复:F1–F12 与 Insert 此前完全不发送 |
| kitty keyboard 协议 | 部分 | 只支持 disambiguate 级别 |
| 输入法(IME)内联预编辑 | 部分 | 候选窗未定位到光标(缺 `set_ime_cursor_area`) |
| 终端内联图片 Sixel / Kitty / iTerm2 | 可用 | 不模拟 Kitty z-index;会话恢复不保留图片 |
| 鼠标上报(SGR / X10) | 可用 | |

### 3.2 窗口与工作区

| 功能 | 状态 | 备注 |
|---|---|---|
| 标签栏:拖拽重排、`+`、`×`、右键菜单(重命名/前缀/标记/分组/复制/移动/关闭其他/关闭下方) | 可用 | 复制标签不带标记和分组;关闭其他/下方不进"重开"栈 |
| 分屏树 `⌘D` / `⇧⌘D`、拖动分隔条、每个 pane 的关闭按钮 | 可用 | 新分屏/新标签不继承当前目录 |
| 快速终端 `⌘⇧T`、重开关闭的标签 `⌘⇧Z` | 可用 | 重开只恢复目录 |
| 全局快速终端热键 | 可用 | 本次修复:一次按键触发两个动作(切到 Quick 后又隐藏窗口) |
| 命令面板 `⌘K` | 可用 | |
| Open Quickly `⌘⇧O` | 部分 | 只列标签、目录、最近文件;无文件、agent、内容命中 |
| details 面板:Info / Agent / Outline / Git / Files / Ports / Queue | 部分 | Ports 只看 shell 进程本身;非 git 目录显示 clean;面板隐藏时仍每 2 秒轮询 |
| 会话恢复(布局、目录、标题、分组) | 可用 | 本次修复:`⌘Q` 退出不保存会话。恢复的是布局,不是仍在运行的进程 |
| Recipes 保存/打开工作区 | 部分 | 写入失败不提示;名称未做文件名校验 |
| 画中画、Hint、只读模式 | 可用 | 只读模式不拦截 MTP `pane.send/run` |
| View 规则 | 部分 | 只生效别名和标题;图标、徽章、按命令/host/文件匹配未生效;不热加载 |
| macOS 原生菜单栏 | 可用 | 本次修复:文本框有焦点时菜单的复制/粘贴/全选作用于终端 |
| 设置窗口 `⌘,` | 可用 | 本次修复:此前改动不落盘、主题名被强制为 Nord;配置语法错误时静默回退 |

### 3.3 查看器与编辑器

| 功能 | 状态 | 备注 |
|---|---|---|
| 只读查看、编辑、行号、语法高亮、极简 vim 模式 | 可用 | 本次修复:保存失败被吞掉且显示为已保存;关闭时不确认未保存修改 |
| Markdown 渲染 + Mermaid 子集 | 部分 | 相对路径本地图片不显示;外部 `mermaid-command` 在 UI 线程同步执行 |
| 跳转行高亮、Open Externally、Edit in Tab | 未提供 | `editor` 配置项当前未被读取 |

### 3.4 Agent

| 功能 | 状态 | 备注 |
|---|---|---|
| 检测 claude / codex / opencode / miao,安装状态 hook,显示接入片段 | 可用 | 片段没有"复制"按钮 |
| 状态徽章、需要关注标记、系统通知、防休眠 | 可用 | 本次修复:防休眠进程在应用退出后可能残留 |
| Composer `⌘⇧E`、提示队列(手动发送) | 可用 | |
| agent 空闲时自动投递队列 | 未提供 | 队列没有目标 pane 的数据模型 |
| 从界面启动 agent | 未提供 | |
| Resume(会话 id)、配额显示 | 可用 | 依赖 hook 上报的字段 |

### 3.5 远程与运维

| 功能 | 状态 | 备注 |
|---|---|---|
| 新建 SSH 会话(遵循 `~/.ssh/config`、ControlMaster 复用、零安装 terminfo) | 部分 | `ssh -G` 在 UI 线程同步执行;恢复的 SSH 标签变成本地 shell |
| 远端文件查看/编辑(经 ssh) | 部分 | 主机需手填;读写在 UI 线程同步执行(失败现在会提示) |
| 主机库、分组、密钥管理、SFTP、FTP、端口转发、Snippets | 未提供 | 见 M3 |

### 3.6 系统集成与自动化

| 功能 | 状态 | 备注 |
|---|---|---|
| 配置文件 + ghostty / alacritty 导入 | 可用 | 本次修复:语法错误的 `config.toml` 会在状态栏提示原因 |
| zsh shell 集成(OSC 7、命令历史) | 可用 | bash / fish / PowerShell 未覆盖 |
| URL scheme 与单实例转发 | 部分 | 命令行传入可用;macOS 浏览器/Finder 发来的 URL 事件未处理 |
| MTP 控制面与 `mtty-cli` | 可用 | 真实窗口冒烟覆盖 |
| 版本检查 | 可用 | 只检查,不自动安装;`update-pubkey` 未使用 |
| 中英界面 | 部分 | details 面板部分行仍是英文 |

## 4. 路线图与执行批次

执行顺序:**M1 → M0 → M2 → M3 → M4**。M1(改名)按 2026-10-01 的决定提前,因为越晚改,
需要迁移的用户状态和外部集成越多。每个批次是一个可独立提交、独立验证的单元:

- 每批结束必须通过:`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace`、
  `scripts/smoke-hosts.py`(涉及界面或打包时),以及该批列出的专项验收。
- 每批一个(或少数几个)提交,完成后立即推送,并在本节打勾、注明证据。
- 需要凭证、真机或所有者确认的步骤(签名、公证、发布、生产主机)标记为 **[所有者]**,不在批次内自动执行。

### M1 品牌统一:mtty(设计见 [ADR 0032](decisions/0032-rename-mtty.zh-CN.md))

- [x] **B1.1 运行时改名与兼容层** — `ee12822`:迁移/环境变量/socket 链接/scheme 单测;冒烟覆盖旧配置复制、两套 pane 变量、旧式 hook 上报、旧 socket 路径。
  - `miao-term-config`:`config_dir()`、`migrate_legacy_config()`(复制一次、不覆盖、不删旧目录)、`env()`(`MTTY_*` 优先、回退 `MIAOTTY_*`)。
  - 所有路径改走 `config_dir()`:config、views、hooks、launch inbox、session/queue/window/recipes。
  - MTP:默认 socket `mtty.sock` + `miaotty.sock` 兼容链接;Windows 管道 `mtty`;令牌/能力环境变量双读。
  - pane 环境:同时导出 `MTTY_*` 与 `MIAOTTY_*`,`MTTY_CLI`/`MIAOTTY_CLI` 指向同目录 CLI 的绝对路径。
  - URL scheme `mtty://` 与 `miaotty://` 都接受;hook 脚本、shell shim、热键片段、ControlPath、界面文字改为 mtty。
  - 验收:迁移/环境变量/scheme 单元测试;旧 hook 脚本在新 pane 中上报状态成功(冒烟)。
- [x] **B1.2 构建与打包** — `ee12822`:`package-macos.sh` + `check-macos-bundle.py` + `smoke-hosts.py --bundle` 通过,`test-release-manifest.py` 9 项通过;release 工作流演练(run 36830274914)四个 runner 全部通过,含 MSI 安装/卸载与两个 scheme、deb 兼容别名。
  - 目录与包:`miaotty-app` → `mtty-app`(二进制 `mtty`),`miaotty-cli` → `mtty-cli`。
  - macOS:`mtty.app`、`dev.mtty.terminal`、URL scheme 注册;安装脚本归档并移除旧 `miaotty.app`。
  - Linux:deb 包 `mtty`(Replaces/Conflicts/Provides `miaotty`)、兼容符号链接、`.desktop`、AppImage。
  - Windows:MSI 产品名与目录 `mtty`,UpgradeCode 不变,注册 `mtty://` 与 `miaotty://`。
  - `release.yml`、`ci.yml`、更新清单脚本与其测试、冒烟/性能脚本、`windows-verify.ps1`。
  - 验收:`scripts/package-macos.sh` + `check-macos-bundle.py` + `smoke-hosts.py --bundle` 通过;
    `test-release-manifest.py` 通过;release 工作流手动演练 **[所有者]**。
- [ ] **B1.3 文档与官网**
  - README(中英)、`docs/*`、AGENTS.md、配置示例改为 mtty;APP-IDENTITY 改写为迁移说明;历史 ADR 不改。
  - mtty.dev 站点的安装、配置、CLI 示例改为 mtty(独立仓库,单独提交)。
  - 验收:`check-privacy.sh` 通过;仓库内除兼容说明与历史记录外不再出现 `miaotty`。

### M0 稳定化:把"部分"变成"可用"

- [x] **B0.1 UI 线程不阻塞**(另修:SSH 会话改用用户输入的别名连接,`Host 别名` 下的配置此前不生效):`ssh -G`、远端读写、`mermaid-command`、Files 目录读取改为后台任务 +
  加载状态;details 面板只在可见时轮询;Ports 覆盖子进程;非 git 目录如实显示。
  验收:后台任务单元测试;冒烟中打开远端/大目录时界面帧不停顿(日志计时)。
- [x] **B0.2 工作区正确性**(只读模式下 MTP `pane.send/run` 返回 `read_only` 错误;URL 打开新标签运行命令属用户显式操作,不拦截;新分屏 cwd 由单测覆盖,MTP 无分屏接口):新分屏/标签继承目录;复制标签保留标记与分组;关闭其他/下方可重开;
  只读模式拦截 MTP/URL 输入;Recipes 名称校验与读写失败提示;GPU 初始化失败给出错误而非 panic。
  验收:对应纯函数单测 + 冒烟检查新分屏 cwd。
- [x] **B0.3 系统集成**(`0417c1b` 及后续;`open -a dist/mtty.app mtty://quick` 冷启动与运行中均实测生效;另修:复制 SSH 标签会重新连接;IME 候选窗定位待桌面人工确认):macOS URL Apple Event(浏览器/Finder 打开链接);恢复的 SSH 标签显示
  "已断开 — 回车重连"并可重连;IME 候选窗定位到光标。
  验收:`open mtty://quick` 在已安装应用上生效;重连有单测;IME 需桌面人工核对 **[所有者]**。
- [x] **B0.4 View 规则与入口**(`d52d898` 及后续;真实窗口截图核对图标、徽章、命令匹配与热加载;另修:重命名标签此前不生效、深链接 Focus 未切到 pane、符号链接目录不匹配):规则图标/徽章生效,支持按命令/host 匹配,`views.json` 热加载;
  Open Quickly 加入文件与 agent;details 面板剩余英文串国际化。验收:规则引擎单测、截图核对。
- [x] **B0.5 可自动验收的 UI**(标签栏导出每个标签的矩形作为测试目标;回放测试覆盖标签拖拽重排、分隔条拖动、重命名回车提交/Esc 取消、焦点文本框接收粘贴;回放即发现:8 个输入框按回车无效,已修复;编辑器保存由单测覆盖):语义化控件目标 + 无窗口事件回放,覆盖重命名/取消、标签重排、
  分隔条拖动、编辑器保存、剪贴板焦点。验收:新增回放测试在 CI 中运行。

### M2 Agent 工作台(吸收 mtty / Otty)

- [x] **B2.1 启动与接入**(codex 片段按其 `hooks.json` 格式生成,需 `[features] hooks = true`;钩子脚本端到端测试覆盖 `--stdin` 取 session、pane 过滤、不阻塞;未改动用户真实的 agent 配置):设置与命令面板中"启动 agent"(codex/claude/opencode/miao,可选目录);
  接入片段带"复制"按钮;codex hook 片段与验证脚本。验收:启动命令单测;冒烟用假 agent 上报状态。
- [x] **B2.2 队列**(另修:"立即发送"不移出队列导致重复投递;状态按帧采样会漏掉快速的 processing→idle,改为 MTP 按序记录的状态变化;冒烟背靠背发送状态连续 8 次通过):队列项绑定目标 pane,agent 转为 idle 时自动投递,持久化到 `queue.json`(兼容旧格式)。
  验收:状态转换与投递单测(含重复事件不重复投递)。
- [x] **B2.3 命令边界**(按字节流捕获 C 与 D 之间的输出,不受回滚上限导致的行号平移影响;真实 zsh 端到端测试;冒烟经 MTP `pane.output` 读回):OSC 133(A/B/C/D)解析,zsh shim 发送;"复制/发送上一条命令输出"。
  验收:解析单测;冒烟中运行命令后取回其输出。
- [x] **B2.4 Worktree 任务**(临时仓库集成测试 4 项;任务窗口与新建对话框经 `MTTY_QA_COMMAND` 真实窗口截图核对):新建任务 = `git worktree add` + 分支 + agent pane;任务列表、diff 查看、
  合并或丢弃(破坏性操作需确认)。验收:在临时仓库上的集成测试。
- [x] **B2.5 注意力与 miao**(osascript 通知不支持点击回调,改为:通知后两分钟内激活 mtty 即跳到对应 pane;后台标签 `!`/✓/• 标记经真实窗口截图核对;miao 插件读取 `MTTY_*`,miao 仓库 `f596c592a`):通知点击跳转 pane、后台标签未读/完成标记;miao 插件读取 `MTTY_*`
  (miao 仓库,单独提交)。验收:状态转换单测;miao 插件测试。

### M3 远程运维(吸收 Termius)

- [x] **B3.1 主机库**(一台真实 Linux 主机端到端:导入的别名经 `~/.ssh/config` 的 ProxyJump 连接,经 `mtty://host/<名称>` 打开并在远端执行命令核对;侧栏 HOSTS 截图核对):`hosts.toml`(名称、地址、用户、端口、分组、标签、跳板机)、从 `~/.ssh/config`
  导入、侧栏主机列表、命令面板搜索、双击连接。验收:解析/导入单测,冒烟连接本机 sshd 或容器 **[需测试主机]**。
- [x] **B3.2 安全连接**(交互式 ssh 由 ssh 自身确认主机密钥;主机库提供后台检查:已知/未知(显示指纹,核对后信任)/已变化(拒绝);真实主机验证"已知";密钥生成与 ssh-copy-id 在终端标签中进行,mtty 不经手口令):known_hosts 首次连接/指纹变化确认界面;ssh-agent 状态与密钥生成;不保存明文密码。
- [x] **B3.3 端口转发**:L/R/D 规则随主机保存;每条规则一个由 mtty 管理的 `ssh -N`(`ExitOnForwardFailure`),状态可见、随 mtty 退出而结束(不复用 ControlMaster:其 60 秒 ControlPersist 会让转发随之消失)。真实主机端到端:经本地端口读到远端 sshd 握手,停止后端口关闭。
- [x] **B3.4 SFTP**(用系统 OpenSSH sftp 批处理,保留 ssh 配置/跳板/agent/ControlMaster;真实主机往返测试;Linux Wayland 真实桌面截图核对;下载进度读取本地文件大小,上传为无进度百分比的忙碌状态):双栏文件浏览、上传下载、拖放、进度、重命名/权限;远端编辑复用 pane 连接。
- [x] **B3.5 Snippets 与广播**(片段在多台主机上各开一个标签执行 `ssh -t 主机 '命令'`,引号经 sh 实际拆分验证;广播输入走键盘路径,无法经 MTP 驱动,待桌面人工确认):命令片段库,在当前 pane 或多台主机执行;多 pane 广播输入。
- [ ] **B3.6 FTP/FTPS 与持久会话**:FTP/FTPS 共用文件浏览(明文提示);可选 tmux/mosh 重连。
  验收(M3 整体):对测试主机的自动化冒烟;凭证不进日志、会话文件或 MTP 响应。

### M4 跨平台交付

- [ ] **B4.1** Windows/Linux 真实桌面验收(IME、热键、拖放、菜单)**[所有者 + 真机]**。
- [ ] **B4.2** Apple 公证、Windows MSI 签名 **[所有者 + 凭证]**。
- [ ] **B4.3** 自动更新:下载、`update-pubkey` 签名校验、替换安装。
- [ ] **B4.4** bash / fish / PowerShell 的 shell 集成。
- [ ] **B4.5** 可选的端到端加密同步,默认关闭。

## 5. 不做的事

- 不做账号体系和强制登录;任何云能力都是可选的。
- 不内置 AI 模型调用;mtty 编排外部 agent,不替代它们。
- 不修改用户的 shell、agent、ssh 配置文件。
- 不在 UI 中保存明文密码。

## 6. 维护规则

- 功能状态变化时,在同一提交中更新 §3 的表格和 README(中英两份)。
- 路线图条目完成后打勾,并附验证证据(测试名、冒烟输出或截图说明)。
- 改变已锁定的设计决策时,在 `docs/decisions/` 新增 ADR。
