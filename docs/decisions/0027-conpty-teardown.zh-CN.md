# ADR 0027 — ConPTY 的收尾不得阻塞

状态:已接受。

## 背景

在真实 Windows 上,`conpty_resize_updates_screen_and_pty` 看似挂起:测试永不结束。实测表明
测试主体本身很快(~10 ms),时间花在**终端收尾**上。Windows 的 `ClosePseudoConsole`
(a) 会等待客户端进程退出,(b) 当 shell 仍存活、或其输出从未被读取时,可能阻塞数分钟。也就是
说,在开着 shell 的情况下关闭标签或退出应用可能卡住数分钟 —— 不只是测试慢。

## 决定

`Drop for Terminal`(区分 Windows)让收尾有界:

1. `taskkill /T /F /PID <child>` 且带 `CREATE_NO_WINDOW`,杀掉整个客户端进程树(ConPTY 客户端
   可能是包装进程,其子进程才是 shell),并且不闪出控制台窗口。
2. `child.kill()` + `child.wait()`(树杀掉后很快)。
3. 最多排空 50 ms 的读取通道。
4. 在**分离线程**上关闭 `MasterPty`,使 `ClosePseudoConsole` 永不阻塞调用方。它所等待的
   conhost 会在其后不久自行退出(已观测);这段时间里只多一个线程。

`master` 改为 `Option` 以便 `Drop` 取用。Windows 测试现在也会发送 `exit\r\n`(与 echo 测试
一致),因此收尾时 shell 已退出。

## 后果

- Windows 上关闭 pane/标签与退出应用变为即时;引擎测试在真实硬件上从 ~240 s(或无界挂起)
  降到 ~1 s。
- Windows 路径每次关 pane 会短暂启一个 `taskkill` 进程;用 Job Object 更干净,但需要直接依赖
  `windows` crate。
- 非 Windows 行为除"分离关闭"外不变,后者也保护 Linux/macOS 免受未来某个阻塞式 close 的影响。
