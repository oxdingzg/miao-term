# 性能

预算与 CI 门(ADR 0018)。本文件定义可在 CI 中确定测量的预算。
机器可读的数值与实测基线见
[`../benches/budgets.json`](../benches/budgets.json)。

## 门

```sh
cargo test --release -p mtty-core -p mtty-graphics -p mtty-ptyhost -p mtty-app -- --ignored
```

性能测试标记为 `#[ignore]`,故常规测试保持快速;`perf` CI 作业在 `ubuntu-latest` 上以
release 运行它们。预算是绝对值且留出宽裕余量(数量级退化会失败,runner 抖动不会)。
`MTTY_PERF_SCALE`(默认 `1.0`)可在慢机器上按倍数放宽所有预算。每个指标还会与
`budgets.json` 里记录的**基线**按 `regression_pct` 比较(ADR 0023),故漂移也会失败,而不只是断崖。`perf` 作业还会与仓库内持久的 **CI 基线** [`benches/perf-baseline.json`](../benches/perf-baseline.json)
比较(ADR 0028):`main` 上的 nightly/手动运行会用本次测量重写该文件并提交,因此不受缓存淘汰
与 runner 镜像变更影响。比较仅**报告**,因为 runner 抖动(同一份代码 1.8x)远大于信号——
真正的门是绝对预算。

## 预算

| 指标 | 预算 | 基线(M 系列,release) | 位置 |
|------|------|------------------------|------|
| VT 解析吞吐 | ≥ 25 MB/s | 73 MB/s | `crates/term-core/tests/perf.rs` |
| 屏幕快照(30 行) | ≤ 2 ms | 0.012 ms | `crates/term-core/tests/perf.rs` |
| 每帧行构建 | ≤ 4 ms | 0.12 ms | `mtty-app/tests/perf.rs` |
| 面板排名(10k 条目) | ≤ 100 ms | 2.0 ms | `mtty-app/tests/perf.rs` |
| 托管回显往返(p95) | ≤ 4 ms | 0.04 ms | `crates/term-ptyhost/tests/perf.rs` |
| 托管输出吞吐 | ≥ 25 MB/s | 180 MB/s | `crates/term-ptyhost/tests/perf.rs` |
| IPC 空闲开销 | ≈ 0(无轮询) | — | 设计如此 |
| agent 突发 | 100 事件 → 1 重绘 | — | 设计如此 |

行构建预算即 roadmap 的帧时间预算(`p99 < 4 ms`)作用于 damage 重建路径,这是无需 GPU
即可压测的部分。冷启动(`< 300 ms`)、新建 pane(`< 50 ms`)与内存不是 CI 门(需要窗口/
活进程),仅人工测量,在 `budgets.json` 中标记 `hard: false`。

## 未入门(原因)

- **帧时间 / 按键→上屏延迟**需要真实窗口与 GPU;行构建门以无头方式近似帧成本。
- **相对 Ghostty 的吞吐(≥ 95%)**需要同机 Ghostty;我们改为门控绝对解析速率。
- **功耗**需要 `powermetrics`;人工复核。


## CPU / 内存专项优化（2026-10）

本轮治理的热点与约束：

- **行构建**：单宽字符直接追加到 span 的字符串，消除逐字符临时字符串。
- **面板评分**：ASCII 标签、类型和查询通过虚拟拼接迭代器匹配，避免拼接与转小写
  的堆分配；非 ASCII 保留 Unicode 路径，匹配偏移与子序列规则保持一致。
- **PTY 输出**：每 pane 最多排队 32 个 8 KiB chunk（256 KiB 数据，另有一个 reader
  chunk 和通道元数据），队列满后对生产者施加背压。一次处理到 64 KiB 或 4 ms
  就在 chunk 边界让出 UI，并主动安排续调度。4 ms 是协作式预算；单个 chunk 的
  图像解码可能超过它，多 pane 的时间也会叠加。
- **图片绘制**：仅为需要上传的可见图片克隆当前帧的 `Arc`，不再每帧复制 placement
  与整个动画帧列表；视区外的新图片不上传 GPU。
- **图片动画**：按实际 100 ms 换帧边界安排重绘，替代持续请求重绘；视区外动画
  不安排定时重绘。输入与 PTY 输出仍可独立触发重绘。

### 同进程 release 对照

使用相同的 100×30 彩色文本屏幕与 10,000 个文件条目，预热后分别执行 1,000 次
行构建与 100 次排序。旧实现取自优化前源码；下列为本地一次微基准结果，不是整机
CPU 使用率、进程 RSS 或端到端帧耗时。分配统计包含 realloc；字节数为累计申请量，
不是峰值驻留内存。时间不作为跨机器承诺。

| 指标 | 优化前 | 优化后 |
|------|--------|--------|
| 行构建 | 0.1196 ms/次 | 0.0516 ms/次 |
| 行构建分配 | 3,151 次 / 35,760 B | 269 次 / 12,704 B |
| 面板排序（10k） | 1.4334 ms/次 | 0.4261 ms/次 |
| 面板排序分配（含结果向量与排序） | 30,012 次 / 711,592 B | 12 次 / 186,032 B |
| ASCII 评分本身（10k） | — | 0 次分配 |

新增 `mtty-app/tests/perf_allocations.rs`，纳入上面的 release gate：100×30 行构建
须少于 600 次分配/realloc，10k ASCII 评分须零分配。线程局部计数隔离其他线程的
分配噪声。普通测试覆盖队列满时的背压、分批处理后的续调度/EOF/顺序，以及动画
换帧的定时边界。整进程 RSS、功耗与 Windows 交互延迟仍需真实宿主测量。


## 第二轮：完整输入路径与图像内存生命周期

### 根因与结构性改动

真实 macOS 宿主的 `sample` 栈采样将分片输出的热点定位到 `Scanner::feed`。
原实现收到每个 chunk 后，从头查找已积累 payload 的终止符：固定大小分片传入 N
字节时，扫描工作为 O(N²)。此外，同一份 PTY 输出还被 OSC、DSR、图形扫描器分别
复制；普通文字被复制到 scanner buffer 后，又复制到 `Segment::Text`。

改为 `Scanner::feed_with` 的借用事件流：

- 普通输出直接借用原 chunk；只缓存跨 chunk 的未完成控制序列。
- 待完成序列只检查新增字节，并处理跨 chunk 的 ESC/ST/DSR；累计扫描为 O(N)。
- 同一事件流按顺序通知 OSC 元数据与 DSR，移除 Terminal 的另外两份原始字节缓存。
  DSR 在前面的文字更新光标之后回复；不再用整个 chunk 之前的光标位置。
- 缓冲容量增长受 32 MiB 上限约束；超限按借用切片返还文字。序列完成/放弃后，
  超过 64 KiB 的高水位 buffer 被释放。兼容的 owned `feed` API 仍可供嵌入者使用。
- 无图片的 pane 不再扫描每个文字字节来统计图像 anchor 的行推进。

图像内存不能用“最多 256 张”来充分约束：一张图本来可达约 64 MB，动画帧还可累积。
现在每 terminal 默认约束为：

| 保留资源 | 上限 / 策略 |
|----------|-------------|
| 所有 placement 和动画帧的像素 buffer **实际容量** | 128 MiB；超额淘汰最旧的完整 placement |
| 所有未完成 Kitty 传输的 buffer **实际容量** | 合计 32 MiB，最多 16 个 ID；超额丢弃当前传输 |
| 单 placement 的动画帧 | 最多 256 帧；后续帧不追加，避免小图片绕过元数据内存约束 |
| 单图像素 | 16,000,000，保持原约束；PNG 在像素解码前检查尺寸 |

预算包含 frame 0 与 image 共享的像素只一次；截短后的 Vec 空余容量也计入。超额动画
整张淘汰，避免删掉中间帧后悄悄重编号。预算是每 terminal 的保留资源上限，**不是整
进程 RSS 上限**：解码临时内存、GPU texture、字体缓存、scrollback 和多 pane 另计。

解码路径同时消除正常 base64 的清洗副本、未压缩 bitmap 的输入副本、RGBA 的二次
逐像素复制、以及 RGBA PNG 的 `to_rgba8` 副本。仅有空白的 base64 才生成清洗副本。
还修复了连续三个以上 Kitty chunk 时，每个 `m=1` 都清空前文的错误。

### 对照测量

下列“优化前”均指第一轮优化完成后的实现；为本机 release 单次对照结果。
分片扫描测试只提取协议 payload，不含图片解码；完整输入基准在 20×5 / 100 行
history 的 Terminal 上，以 8 KiB chunk 喂入 16 MiB 彩色文字，包含元数据扫描与 VT。

| 指标 | 第二轮前 | 第二轮后 |
|------|----------|----------|
| 1 MiB 序列，512 B 分片 | 967.2 ms | 1.4 ms |
| 2 MiB 序列，512 B 分片 | 4,010.6 ms | 2.7 ms |
| 4 MiB 序列，512 B 分片 | 16,246.5 ms | 6.2 ms |
| 4 MiB 序列结束后 scanner 保留容量 | 8 MiB | 0 |
| 完整文字输入路径吞吐 | 74.4 MB/s | 129.8 MB/s |
| 4 MiB 原始 RGBA 解码分配/realloc | 23 次 / 25,165,818 B | 1 次 / 4,194,306 B |

真实宿主对照使用隔离配置、相同 shell 和隐藏测试窗口，避免桌面键入扰动；计数仅
包含应用进程，不包含生成数据的 Python。完成标记是**生产者写完**，随后等待 4 秒
再取 CPU/RSS，故不当作端到端 UI 延迟。RSS 采样不是精确内存峰值。

| 真实宿主场景 | 第二轮前 | 第二轮后 |
|--------------|----------|----------|
| 4 MiB 分片输出：应用工作及收尾 CPU 时间 | 5.90 s | 0.27 s |
| 同场景生产者完成时间（含每片 0.2 ms sleep） | 5.89 s | 2.30 s |
| 40 张 1024×1024 PNG：稳定 RSS | 427.91 MiB | 395.70 MiB |
| 同场景相对空闲 RSS 增量 | 165.14 MiB | 133.03 MiB |

CPU 场景工作时间约减少 95%；大量图片的 RSS 增量减少约 32 MiB，符合 128 MiB
像素容量预算。空闲 CPU 两者都约为 0–0.33%，本轮不声称空闲功耗有明显改善。
释放 buffer 不保证分配器立刻归还 RSS，因此高水位容量与 RSS 分开报告。

### 复现与门禁

```sh
cargo test --release -p mtty-core -p mtty-graphics -p mtty-ptyhost -p mtty-app -- --ignored --nocapture
cargo build --release -p mtty-app -p mtty-cli
python3 scripts/profile-input.py --mode fragments
python3 scripts/profile-input.py --mode images
# 比较保留的旧二进制时，指定 --binary <old-binary>。
```

真实宿主脚本当前针对 macOS，原始 sample、配置和 JSON 报告只写到临时目录。
CI perf 作业已纳入图形 crate：门禁覆盖线性扫描工作量、序列结束后的 buffer 释放、
普通文字流零分配，以及 4 MiB RGBA 解码不产生第二份完整像素 buffer。
普通测试覆盖所有 chunk 边界的事件一致性、DSR 回复位置、三段 Kitty 重组、总像素
容量淘汰、未完成传输的总容量与 ID 上限、动画帧元数据上限和 PNG/压缩路径。
Windows 的 ConPTY 真实宿主和 IME 检查仍按 `WINDOWS-DEV.md` 执行。
