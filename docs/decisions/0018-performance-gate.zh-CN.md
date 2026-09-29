# ADR 0018 — 性能门

> English (default): [`0018-performance-gate.md`](0018-performance-gate.md)

状态:已接受。

## 背景

roadmap 把每个里程碑的退出条件都设为性能预算,并规定在 baseline + CI 门存在之前不落
任何热路径代码。Swift 参考实现用 Instruments 测帧时间
与功耗;我们需要无头 CI 能确定运行的东西。

## 决定

采用 **release 模式、绝对预算的门**,而非统计型基准框架:

- `crates/term-core/tests/perf.rs` 测 VT 解析吞吐(16 MB 重 SGR 负载 ≥ 25 MB/s)与 30 行
  屏幕快照(≤ 2 ms)。
- `miaotty-app` 的 `perf_tests` 测 damage 重建路径 `build_rows`(≤ 4 ms/帧 —— 即 roadmap
  的帧预算)与 10k 条目的 Open Quickly 排名(≤ 100 ms)。
- 所有性能测试标记 `#[ignore]`,故常规 `cargo test` 保持快速;专门的 `perf` CI 作业在
  `ubuntu-latest` 上跑 `cargo test --release … -- --ignored`。`MIAOTTY_PERF_SCALE` 可在慢
  机器上放宽所有预算。
- 预算与实测基线在 `benches/budgets.json`;roadmap 预算的映射,以及**未**入门者(帧时间、
  按键→上屏、相对 Ghostty 吞吐、功耗、内存、冷启动)记录在 `docs/PERFORMANCE.md`。

选择"绝对预算 + 大余量"而非"百分比回归追踪",是因为 CI runner 噪声大且我们没有共享基线
存储;数量级退化仍会失败,而这正是我们关心的失效模式。

## 后果

- 热路径(VT 解析、行重建、面板排名)在每次 push 上都有真实、可复现的门。
- 数值保守:M 系列机器上行构建约 0.12 ms(预算 4 ms)、排名约 2 ms(预算 100 ms),故该门
  抓的是断崖而非漂移。
- GPU 帧时间、端到端延迟与内存仍人工测量;日后加入共享基线存储即可收紧预算,而无需改动
  测试骨架。
