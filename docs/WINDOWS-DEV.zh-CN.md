# Windows 开发 / 验证机

如何在 Windows 上构建、测试与冒烟验证 miaotty,以及我们用于此的机器。
英文为默认;请保持 [`WINDOWS-DEV.md`](WINDOWS-DEV.md) 同步。

## 机器

- 以 `ssh <windows-host>` 访问(Windows 11,你的 Windows 用户)。
- **MSVC 已存在**:VS 2022 Community
  (`…\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\…\cl.exe`)以及一个
  2022 BuildTools。**无需**再装 Visual Studio。
- rustup 位于 `%USERPROFILE%\.cargo\bin`,**默认工具链为
  `stable-x86_64-pc-windows-msvc`**(`rustc 1.98.1`)。
- 源码副本在 `C:\Users\<you>\miao-term`(含 `target/` 约 3 GB)。

## 用 MSVC,不要用 `-gnu`

安装 `stable-x86_64-pc-windows-gnu` 会*成功*,其自带的 `dlltool.exe` 手动也能运行,但 rustc
在链接 `windows-sys` 时调用它会失败:

```
error: dlltool could not create import library with …\self-contained\dlltool.exe -d … -D kernel32.dll …
       …\dlltool.exe: CreateProcess…
```

因此请使用 MSVC 工具链(CI 的 `windows-latest` 跑的也是它)。
故 `rust-toolchain.toml` 必须 pin `channel = "stable"`(宿主默认),而非 `-gnu` 三元组。

## 从 macOS 同步源码

该主机上没有 GitHub 凭据,所以推快照而不是 clone:

```sh
cd <the directory containing the checkout>
tar -czf /tmp/miao-term-src.tgz --exclude target --exclude .git --exclude dist miao-term
scp /tmp/miao-term-src.tgz <windows-host>:C:/Users/<you>/miao-term-src.tgz
ssh <windows-host> 'powershell -NoProfile -Command "Remove-Item -Recurse -Force C:\Users\<you>\miao-term -ErrorAction SilentlyContinue"'
ssh <windows-host> 'powershell -NoProfile -Command "& tar.exe -xzf C:\Users\<you>\miao-term-src.tgz -C C:\Users\<you>"'
```

## 验证

在主机上(或经 ssh):

```powershell
powershell -ExecutionPolicy Bypass -File C:\Users\<you>\miao-term\scripts\windows-verify.ps1
```

它会构建 `miaotty` + `miaotty-cli`,跑引擎/MTP 测试,然后启动应用并经 MTP 驱动它
(`ping`、`file write`、`file read`、`view`)。

2026-09-29 以此验证:构建成功(5m47s)、`conpty_spawns_shell_and_echoes` 通过、`ping` 广告
`app.view.write` / `file.read` / `file.write`,文件往返正常。

## 经 ssh 且无桌面会话时

ssh 运行在非交互会话,故:

- 长构建:用 **`schtasks` 以 `SYSTEM` 运行**并显式设置 `RUSTUP_HOME` / `CARGO_HOME` 的任务,
  可在 ssh 断开后存活;直接 `Start-Process` 可能在会话结束时被杀。`/tr` 上限 261 字符,
  故把命令写进 `.cmd` 再传其路径。
- 看不到 GUI,也**无法验证 IME** —— 那需要交互式桌面(RDP,或在已登录会话中运行任务)。
- 需留意的问题:本机上 `conpty_resize_updates_screen_and_pty` 运行 60 秒后仍在跑(随后进程消失);
  echo 测试通过。值得与其作者一起排查。

## CI

`.github/workflows/ci.yml` 在 `windows-latest` 上跑 `cargo check` + `cargo test`
(故 `#[cfg(windows)]` 的 ConPTY 测试也会跑),并在 `ubuntu-latest` 跑 `perf` 作业。需要**真实**
Windows 主机或 GUI/IME 检查时用这台机器,但不要以它替代 CI。
