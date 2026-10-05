# 验收收尾 — 2026-10-04

[English](ACCEPTANCE.md)

本文记录 v0.1.3 之后的源码修复。早期 UI 审计和发布演练保留为历史记录。
主机身份、私有地址、本地路径、凭证、截图与原始日志均留在公开仓库之外。

## 改动

- Windows ZIP 更新先暂存并验证三个可执行文件，再保留备份并替换。失败时恢复原文件并重启旧应用；
  MSI 返回码也会检查。见 [ADR 0043](decisions/0043-windows-update-transaction.zh-CN.md)。
- 串口空闲读取超时或被中断时重试，避免把空闲当成断线。
- Agent 提案以红色删除线显示删除前的内容。这些行可以滚动，但不会进入复制内容或保存的文档。
  整文件提案保留未变动的上下文，接受和拒绝仍可以撤销。
- ACP 补齐认证、加载会话、终端生命周期、权限选项 ID 和 UTF-8 流式读取。读文件优先使用编辑器的未保存内容；
  写文件要经用户审阅、实际保存后才报告成功，拒绝则保留磁盘原文。命令需要授权，输出和进程数量有上限，
  关闭客户端会清理其进程树。
- 剪贴板图片转发支持 Windows 和 X11，包括 PNG 编码及文本粘贴清理；Wayland 使用宿主已有的 data device。
- README、产品状态和发布说明不再把已完成的编辑器与传输功能列为未来里程碑。

## 验证证据

| 项目 | 证据 | 边界 |
|---|---|---|
| macOS | Release 构建、打包应用的真实窗口冒烟、workspace 测试、clippy、14 项 release 性能测试 | Ad-hoc 签名不代表 Apple 公证 |
| Linux AppImage | Ubuntu 24.04 原生 X11 窗口：签名下载、验证、替换、重启及保留 PTY host 会话 | 没有启动健康握手，无法识别所有替换后的启动故障 |
| Linux 更新失败 | 错误 SHA-256 或 minisign 签名被拒且下载删除；缺失下载时原包不变并重新启动 | 使用私有测试清单与临时签名密钥 |
| Linux 远程与传输 | 真实 mosh SSH 引导及 UDP 会话；TCP/Telnet 双向字节、Telnet 协商；串口 PTY 空闲及双向收发 | PTY 验收不代表所有实体串口适配器 |
| 广播 | 实际键盘事件输入两个 Linux pane，命令输出及退出码一致 | X11 输入注入 |
| Windows 更新辅助程序 | 实际执行 PowerShell：正常 ZIP、坏 ZIP、第二个程序被占用；回滚后原哈希完全一致，自动重启，支持中文、空格及引号路径 | MSI 桌面结果单独记录 |
| Windows 输入法 | 当前 v0.1.3 桌面构建：微软拼音 `nihao` + 空格提交“你好”，经 PTY 读回 | 未穷举所有输入法 |
| 剪贴板 | Windows 3×2 PNG 尺寸/像素；Linux X11 2×2 RGBA 含透明度，然后文本粘贴并清除旧图 | Wayland 结果单独记录 |
| ACP 实际 Agent | Codex ACP adapter 2.1.1：初始化、新会话、实际提示、输出文件校验、session/load | 此 adapter 自行执行工具，未经过客户端文件/终端回调 |
| ACP 客户端协议 | 官方 TypeScript SDK 1.6.0 对端：文件读写、权限选项 ID、全部五个终端方法 | 协议桥接与模型提供商登录分别验收 |
| Claude ACP | Adapter 0.85.1 初始化和创建会话通过 | 实际提示返回 `Authentication required`，不算完整提供商验收 |

## 中断后的恢复检查

复查发现，shell 集成测试虽然完成断言，却可能在销毁终端时卡住。
现在子进程终止与回收均在后台线程执行，PTY master 保留到子进程回收后再关闭。
Unix 原生子进程即使忽略 SIGHUP 也会被强制终止；Windows 进程树终止也移到后台。

当前源码快照的验证结果：

- `cargo fmt --all --check`、`git diff --check` 和隐私扫描通过。
- 远程 macOS `cargo clippy --workspace --all-targets` 通过。
- 远程 macOS `cargo test --workspace`：531 项通过、零失败，19 项平台/性能检查忽略。
  两种已安装 Bash 均通过，未排除此前卡住的集成测试。
- 新增 Unix 回归测试验证：忽略 SIGHUP 的 shell 关闭时不阻塞界面，并最终被回收。
- Release 性能门禁全部 14 项通过。
- Windows 实机 ConPTY 测试全部三项通过（启动/回显、尺寸调整、环境变量）。

这些检查对应源码快照，安装和发版是后续独立步骤。
依赖 `block` 0.1.6 仍有原有的 future-incompatibility 提示。

## 仍需所有者提供的材料

Apple Developer ID 签名/公证和 Windows Authenticode 签名需要所有者凭证。
仓库当前有 minisign 发布密钥，未配置 Apple/Windows 签名密钥，Windows 验收主机也没有可用的代码签名证书。
minisign 分离签名校验是另一项检查，已经可用。

## v0.1.6 桌面验收 — 2026-10-05

三项需要真实桌面的检查已在发布的 v0.1.6 包上复验。PowerShell 5.1 历史和图片恢复已端到端通过；输入法候选框仍需获得 macOS 桌面权限后目视确认。

| 项目 | 结果 | 证据 | 边界 |
|---|---|---|---|
| 中日韩之后的输入法候选框定位 | 锚点已验证 | mtty 把候选框定位到光标所在单元格（`active_cursor` -> `cursor_position`）。新增测试固定宽字符后的光标**列**：`目录:` 结束于第 5 列，`中` 前进两列。 | 候选框在屏幕上的实际位置仍需一次交互式桌面确认；键盘注入被 macOS 辅助功能/自动化权限阻塞（见下） |
| 内联图片的会话恢复 | 端到端通过 | 真实 macOS 桌面：在窗格中现场绘制 Kitty 图片（应用自截图中有 15,500 个红色像素），随回滚保存到 `pane0.images.json`，以相同宽度重启后重新放置——恢复后的截图同样有 15,500 个红色像素。Sixel 同样恢复（43,200 个红色像素）。 | 仅限窗格宽度不变时；宽度变化会放弃放置。动画以静帧恢复（见 `graphics::tests` 断言） |
| Windows PowerShell 5.1 历史 | 端到端通过 | 在已登录的 Windows 桌面上，`scripts/ps51-history-acceptance.ps1` 使用 Windows PowerShell 5.1.22621.6133 和已发布的 v0.1.6 Windows x86_64 ZIP 返回 PASS（SHA-256 与发布资产一致）。`history.json` 包含 `pane0` 中输入的第二条标记命令，同时保存了 `output.txt`。 | 仅验证一个 PowerShell 5.1 补丁版本；未覆盖其他输入方式 |

### 为什么这两项桌面检查无法经 ssh 完成

- **macOS 按键**：通过输入法驱动拼音需要 `System Events` 注入按键，而这需要辅助功能/自动化授权；
  ssh 会话无法授予，调用会被静默拦截。
- **Windows PowerShell 5.1**：应用必须在登录的控制台会话中运行才能挂上 `PSConsoleHostReadLine`；
  从 session 0 用 `schtasks /run` 启动交互式登录任务会失败。请用 RDP 或物理控制台。

