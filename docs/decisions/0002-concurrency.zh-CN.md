# ADR 0002 — 并发与锁纪律

> English (default): [`0002-concurrency.md`](0002-concurrency.md)

状态:已接受。

## 背景

终端热路径(`pty → vt → grid → renderer`)绝不能阻塞在 I/O 或 GPU 上,也不能每帧分配或轮询。

## 决定

- **引导阶段(当前)**:每个 pane 一个 **PTY 读线程**做阻塞 `read()`,通过 `mpsc` 通道把数据块发给
  **UI 线程**;UI 线程排空通道并喂给 VT 解析器(不与 UI 共享锁)。仅在**有输出时**请求重绘(事件驱动,不轮询)。
- **目标(R1+)**:**Alacritty 同款**——`FairMutex<Term>` + `EventListener`;锁**只**覆盖"解析一段字节"
  或"构建渲染实例",绝不跨越 `read`/`write`/GPU 提交。读写使用各自句柄,避免互相阻塞(ConPTY 尤其)。

## 不变量

1. VT/屏幕状态同一时刻只被一个线程持有;锁不跨 I/O 与 GPU。
2. resize 在 UI 线程计算,同时应用到模型与 PTY。
3. 空闲 CPU ≈ 0:无轮询;读线程阻塞在 `read`。
4. 退出码以 OSC 133;D 为准,不信进程退出码。

## 后果

- 当前实现简单正确;通道带来每块一次拷贝(可接受)。
- R1 换成 `FairMutex` 时对外 API(`mtty-core`)不变,app 无感。
