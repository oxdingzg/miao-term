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
