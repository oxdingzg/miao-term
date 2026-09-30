# 性能

预算与 CI 门(ADR 0018)。本文件定义可在 CI 中确定测量的预算。
机器可读的数值与实测基线见
[`../benches/budgets.json`](../benches/budgets.json)。

## 门

```sh
cargo test --release -p miao-term-core -p miaotty-app -- --ignored
```

性能测试标记为 `#[ignore]`,故常规测试保持快速;`perf` CI 作业在 `ubuntu-latest` 上以
release 运行它们。预算是绝对值且留出宽裕余量(数量级退化会失败,runner 抖动不会)。
`MIAOTTY_PERF_SCALE`(默认 `1.0`)可在慢机器上按倍数放宽所有预算。每个指标还会与
`budgets.json` 里记录的**基线**按 `regression_pct` 比较(ADR 0023),故漂移也会失败,而不只是断崖。`perf` 作业还会与仓库内持久的 **CI 基线** [`benches/perf-baseline.json`](../benches/perf-baseline.json)
比较(ADR 0028):`main` 上的 nightly/手动运行会用本次测量重写该文件并提交,因此不受缓存淘汰
与 runner 镜像变更影响。比较仅**报告**,因为 runner 抖动(同一份代码 1.8x)远大于信号——
真正的门是绝对预算。

## 预算

| 指标 | 预算 | 基线(M 系列,release) | 位置 |
|------|------|------------------------|------|
| VT 解析吞吐 | ≥ 25 MB/s | 73 MB/s | `crates/term-core/tests/perf.rs` |
| 屏幕快照(30 行) | ≤ 2 ms | 0.012 ms | `crates/term-core/tests/perf.rs` |
| 每帧行构建 | ≤ 4 ms | 0.12 ms | `miaotty-app` `perf_tests` |
| 面板排名(10k 条目) | ≤ 100 ms | 2.0 ms | `miaotty-app` `perf_tests` |
| IPC 空闲开销 | ≈ 0(无轮询) | — | 设计如此 |
| agent 突发 | 100 事件 → 1 重绘 | — | 设计如此 |

行构建预算即 roadmap 的帧时间预算(`p99 < 4 ms`)作用于 damage 重建路径,这是无需 GPU
即可压测的部分。冷启动(`< 300 ms`)、新建 pane(`< 50 ms`)与内存不是 CI 门(需要窗口/
活进程),仅人工测量,在 `budgets.json` 中标记 `hard: false`。

## 未入门(原因)

- **帧时间 / 按键→上屏延迟**需要真实窗口与 GPU;行构建门以无头方式近似帧成本。
- **相对 Ghostty 的吞吐(≥ 95%)**需要同机 Ghostty;我们改为门控绝对解析速率。
- **功耗**需要 `powermetrics`;人工复核。
