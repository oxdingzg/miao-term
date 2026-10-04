# Windows 开发 / 验证

如何在 Windows 机器上构建、测试与冒烟验证 mtty。英文为默认;请保持
[`WINDOWS-DEV.md`](WINDOWS-DEV.md) 同步。

本文不含任何主机专有信息:使用你自己的 ssh 别名与路径。我们具体用哪台机器、怎么连,
记在私有笔记里,不放在本仓库。

## 机器需要什么

- Windows 10/11 x64。
- **MSVC 构建工具已存在** —— Visual Studio 2022(任意版本)或独立的 Build Tools 安装皆可。
  需有 `cl.exe` / `link.exe`;rustc 会自动找到它们。除此之外无需再装 Visual Studio。
- 通过 [rustup](https://rustup.rs) 安装 Rust,使用宿主默认工具链
  (`stable-x86_64-pc-windows-msvc`)。建议加上 `rustfmt` + `clippy` 组件。
- 一份放在 `%USERPROFILE%` 下的检出(例如 `%USERPROFILE%\mtty`)。

## 用 MSVC,不要用 `-gnu`

安装 `stable-x86_64-pc-windows-gnu` 会*成功*,其自带的 `dlltool.exe` 手动也能跑,但 rustc
在链接 `windows-sys` 时调用它会失败:

```
error: dlltool could not create import library with …\self-contained\dlltool.exe -d … -D kernel32.dll …
       …\dlltool.exe: CreateProcess…
```

故让 `rust-toolchain.toml` pin `channel = "stable"`(宿主默认),而非 `-gnu` 三元组。CI 的
`windows-latest` 也是同样理由用 MSVC。

## 把源码弄到机器上

若该机器没有 GitHub 凭据,推快照而非 clone:

```sh
# 在有检出的机器上
tar -czf /tmp/mtty-src.tgz --exclude target --exclude .git --exclude dist mtty
scp /tmp/mtty-src.tgz <windows-host>:mtty-src.tgz
```

在 macOS 上执行该 `tar` 时请设置 `COPYFILE_DISABLE=1`:否则会加入 `._*` AppleDouble 文件,`term-editor` 的构建脚本读取其中的 `._*.sublime-syntax` 时会失败。

```powershell
# 在 Windows 主机上
Remove-Item -Recurse -Force "$env:USERPROFILE\mtty" -ErrorAction SilentlyContinue
& tar.exe -xzf "$env:USERPROFILE\mtty-src.tgz" -C "$env:USERPROFILE"
```

## 验证

```powershell
powershell -ExecutionPolicy Bypass -File "$env:USERPROFILE\mtty\scripts\windows-verify.ps1"
```

它会构建 `mtty` + `mtty-cli`,跑引擎/MTP 测试,然后启动应用并经 MTP 驱动它
(`ping`、`file write`、`file read`、`view`)。

本项目 Windows 机器最近一次(2026-09-29):构建成功、引擎/MTP 测试通过、应用能启动,`ping`
广告 `app.view.write` / `file.read` / `file.write`(以及 agent/history 能力),`file write` →
`file read` → `view` 往返正常。这是在无桌面会话的 ssh 下完成的;IME/GUI 仍需交互式桌面。

## 经 ssh 且无桌面会话时

ssh 运行在非交互会话,故:

- 长构建:用 **`schtasks` 以 `SYSTEM` 运行**并显式设置 `RUSTUP_HOME` / `CARGO_HOME` 的任务,
  可在 ssh 断开后存活;直接 `Start-Process` 可能在会话结束时被杀。`/tr` 上限 261 字符,
  故把命令写进 `.cmd` 再传其路径。
- 看不到 GUI,也**无法验证 IME** —— 那需要交互式桌面(RDP,或在已登录会话中运行任务)。
- `conpty_resize_updates_screen_and_pty` 过去看似挂起(整轮约 4 分钟);根因是收尾而非测试 ——
  shell 存活时 `ClosePseudoConsole` 会阻塞。已在 ADR 0027 修复:引擎杀掉进程树、排空并在分离线程
  中关闭,测试自身也会退出 shell。现在两个 ConPTY 测试在真机上约 1 秒完成。

## CI

`.github/workflows/ci.yml` 在 `windows-latest` 上跑 `cargo test --workspace`(故
`#[cfg(windows)]` 的 ConPTY 测试也会跑),但仅在非 push 事件(PR、夜间定时、手动触发)运行;
`cargo check --workspace` 则在 Linux 与 macOS 作业中运行。另有在 `ubuntu-latest` 上的 `perf`
作业。需要 GUI 或 IME 检查时用真实 Windows 主机;它不替代 CI。
