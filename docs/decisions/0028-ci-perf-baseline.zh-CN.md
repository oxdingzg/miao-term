# ADR 0028 — CI 侧性能基线

> English (default): [`0028-ci-perf-baseline.md`](0028-ci-perf-baseline.md)

状态:已接受。

## 背景

ADR 0023 把性能指标与 `benches/budgets.json` 中记录的基线(在维护者机器上测得)比较。在 CI 中
真正能生效的是**绝对预算**(ADR 0018),因为 M 系列基线对 `ubuntu` runner 没有意义 —— 于是
CI 只能抓断崖,抓不到漂移。

## 决定

性能测试现在还会**写出** `target/perf-measured.json`(四个门控指标;app 与 core 合并写入同一
文件)。`perf` 作业:

1. 从 actions 缓存恢复 `.perf/baseline.json`
   (`perf-baseline-<os>-…`,`restore-keys` 回退到该 OS 的最新一份);
2. 跑性能测试;
3. 运行 `scripts/check-perf-baseline.py`:若某指标相对基线退化超过 `regression_pct`
   (取自 `budgets.json`,CI 中覆盖为 40% 以免共享 runner 噪声导致抖动)—— 会**方向感知**,即
   吞吐不得下降、耗时不得增长 —— 则失败;
4. 在 `main` 上把本次测量拷回 `.perf/baseline.json`,由缓存在作业结束时保存。

首次运行没有基线:只报告并通过,随后播种缓存。

### CI 中只报告

runner 抖动远大于信号:同一版本在 `ubuntu-latest` 上先后测得 139 MB/s 与 79 MB/s 的 VT 解析
吞吐(1.8×)。故该比较**默认只报告**;CI 的门是测试内部断言的**绝对预算**。在稳定机器上可用
`PERF_ENFORCE=1` 将其变为门(配合每指标下限)。

### 执行下限

亚毫秒计时在共享 runner 上由抖动主导(同一份代码在 `ubuntu-latest` 上先后测得 0.06 ms 与
0.10 ms),故低于每指标下限(行构建/屏幕快照 0.5 ms、面板排名 10 ms)时**只报告、不判失败**,
那里的门是绝对预算。吞吐这类持续测量则始终参与比较。

## 后果

- CI 让同一 runner 与**自身**的历史比较,能抓到宽泛绝对预算会掩盖的漂移,且不依赖维护者硬件。
- 基线按 runner OS 缓存,可能过期(7 天未访问);那时作业改为"只报告、不失败"。
- `benches/budgets.json` 仍是经过评审、与机器无关的预算与参考基线记录;CI 基线是缓存,不是源码。
