# Windows PowerShell 5.1 历史 —— 验收 runbook

Windows PowerShell 5.1 的历史修复已在 v0.1.6 随版本发布（`crates/mtty-core/src/shell.rs`，
PR #48）。其单元测试与独立的参数引用检查均已通过，但**完整的应用内抓取**仍需一台真实、
已登录的 Windows 桌面——应用必须在控制台会话中运行才能挂上 `PSConsoleHostReadLine`。
本 runbook 用于完成这项检查。

请在 Windows 桌面（RDP 或物理控制台）中运行，**不要**经 ssh：
ssh 会话落在 session 0，`[Environment]::UserInteractive` 为 `False`，
交互式登录的计划任务无法在那里启动。

## 步骤

1. 取得 v0.1.6 应用。从发布页下载 `mtty-windows-x86_64.zip` 并解压
   （内含 `mtty.exe`、`mtty-cli.exe`、`mtty-ptyhost.exe`）。
2. 先保存工作并关闭所有已运行的 mtty 窗口。Windows MTP 使用固定命名管道，
   已运行实例会使目录隔离失效；脚本检测到已有实例时会拒绝运行。
3. 在普通桌面 PowerShell 窗口中运行：

   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass `
     -File ps51-history-acceptance.ps1 -App <mtty.exe 的路径>
   ```

4. 脚本以隔离状态启动 mtty，窗格 shell 设为 **Windows PowerShell 5.1**，
   经 shell 集成向窗格键入两条带标记的命令，并经 MTP 读回已记录的历史。
   它会打印 `PASS`/`FAIL`，并在运行目录旁写出 `history.json` 与 `output.txt`。

`-WorkDir` 应使用短路径：虽然 Windows MTP 使用命名管道，PTY host 仍使用 AF_UNIX
socket 路径。脚本在启动应用前拒绝达到 108 字节的 host socket 路径。过长路径可能导致
host 失败并出现 PowerShell 启动错误，而回退的 shell 仍然能记录历史。
检查结束后，脚本也会结束隔离目录中的 PTY host 及其 shell。

## “PASS”的含义

第二条键入的命令出现在 `history list` 中——即 5.1 钩子通过 PowerShell 7 所用的同一
`PSConsoleHostReadLine` 路径记录了它。这就完成了验收记录。

## 记录结果

结果行位于 `docs/ACCEPTANCE.md` / `docs/ACCEPTANCE.zh-CN.md`
（“v0.1.6 桌面验收”）。PASS 之后，把 Windows PowerShell 5.1 行从*阻塞*改为*通过*，
附上 PowerShell 版本与应用构建，并把脚本名作为证据。不要粘贴主机名或路径。

## 未覆盖范围

- 不穷举所有输入法或所有 5.1 补丁级别。
- CI 的 `windows` 任务只列 PowerShell 7，因此 5.1 保持为人工桌面检查。
