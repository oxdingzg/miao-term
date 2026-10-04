# Performance

Budgets and the CI gate (ADR 0018). This file defines the budgets we can measure
deterministically in CI. Machine-readable values and measured baselines are in
[`../benches/budgets.json`](../benches/budgets.json).

## The gate

```sh
cargo test --release -p mtty-core -p mtty-graphics -p mtty-ptyhost -p mtty-app -- --ignored
```

Perf tests are `#[ignore]`d so the normal test run stays fast; the `perf` CI job
runs them in release on `ubuntu-latest`. Budgets are absolute with generous
headroom (an order-of-magnitude regression fails, runner jitter does not).
`MTTY_PERF_SCALE` (default `1.0`) relaxes every budget by a factor on slow
machines. Each metric is also checked against the recorded **baseline** in
`budgets.json` with a `regression_pct` (ADR 0023), so drift fails, not just
cliffs. The `perf` job additionally compares against a **durable CI baseline** tracked at
[`benches/perf-baseline.json`](../benches/perf-baseline.json) (ADR 0028): the
nightly/manual run on `main` rewrites it from the measurements and commits the
change, so it survives cache eviction and runner-image changes. The comparison is
**reported only**, since runner variance (1.8x on the same code) dwarfs the
signal — the absolute budgets are the gate.

## Budgets

| Metric | Budget | Baseline (M-series, release) | Where |
|--------|--------|------------------------------|-------|
| VT parse throughput | ≥ 25 MB/s | 73 MB/s | `crates/term-core/tests/perf.rs` |
| Screen snapshot (30 rows) | ≤ 2 ms | 0.012 ms | `crates/term-core/tests/perf.rs` |
| Row build per frame | ≤ 4 ms | 0.12 ms | `mtty-app/tests/perf.rs` |
| Palette rank (10k entries) | ≤ 100 ms | 2.0 ms | `mtty-app/tests/perf.rs` |
| Hosted echo round trip (p95) | ≤ 4 ms | 0.04 ms | `crates/term-ptyhost/tests/perf.rs` |
| Hosted output | ≥ 25 MB/s | 180 MB/s | `crates/term-ptyhost/tests/perf.rs` |
| IPC idle cost | ≈ 0 (no polling) | — | by design |
| Agent burst | 100 events → 1 repaint | — | by design |

The row-build budget is the roadmap frame-time budget (`p99 < 4 ms`) applied to
the damage-rebuild path, which is the part a benchmark can exercise without a
GPU. Cold start (`< 300 ms`), new-pane (`< 50 ms`) and memory are not CI gates
(they need a window / a live process) and are measured manually; they stay in
`budgets.json` marked `hard: false`.

## Not gated (and why)

- **Frame time / key-to-glyph latency** need a real window and GPU; the row-build
  gate approximates frame cost headlessly.
- **Throughput vs Ghostty (≥ 95%)** needs Ghostty on the same machine; we gate an
  absolute parse rate instead.
- **Power** needs `powermetrics`; reviewed manually.


## CPU / memory optimization pass (2026-10)

- Row building appends single-cell characters directly to their span strings,
  removing per-character temporary allocations.
- ASCII palette scoring searches a virtual lowercase `label kind` byte stream
  without allocating. Non-ASCII input retains Unicode matching semantics.
- PTY output uses a bounded queue of 32 × 8 KiB chunks per pane (256 KiB of
  queued payload, plus a reader chunk and channel metadata). A full queue applies
  producer backpressure. Processing yields at a chunk boundary after 64 KiB or
  4 ms and explicitly schedules continuation. This is a cooperative budget:
  decoding one graphics chunk can exceed 4 ms, and multiple panes add up.
- Image rendering clones only the current frame's `Arc` when a visible image
  needs uploading, avoiding per-frame placement/frame-list copies. New offscreen
  images are not uploaded.
- Visible image animations schedule redraws at their actual 100 ms frame
  boundaries instead of requesting continuous redraws. Offscreen animations do
  not schedule timer-driven redraws. Input/output can still trigger redraws.

### Same-process release comparison

A warmed 100×30 colored screen was built 1,000 times, and 10,000 file entries
were ranked 100 times. The old implementation was taken from the pre-optimization
source. These are one local microbenchmark's results, not process CPU utilization,
RSS, or end-to-end frame time. Allocation counts include reallocations; requested
bytes are cumulative allocation traffic, not peak resident memory. Timings are
not cross-machine guarantees.

| Metric | Before | After |
|--------|--------|-------|
| Row build | 0.1196 ms/call | 0.0516 ms/call |
| Row allocations | 3,151 / 35,760 B | 269 / 12,704 B |
| Palette ranking (10k) | 1.4334 ms/call | 0.4261 ms/call |
| Ranking allocations (including result vector/sort) | 30,012 / 711,592 B | 12 / 186,032 B |
| ASCII scoring alone (10k) | — | zero allocations |

`mtty-app/tests/perf_allocations.rs` adds release gates to the command above:
row building must stay below 600 allocations/reallocations for this 100×30
workload, and 10k ASCII scores must allocate nothing. Thread-local counters
exclude other threads' allocation noise. Normal tests cover queue backpressure,
yield/continuation/EOF/order, and animation frame deadlines. Process RSS, power,
and Windows interactive latency still require real-host measurements.


## Second pass: full input path and image lifetime

A real macOS `sample` profile identified `Scanner::feed` as the fragmented-output
hotspot. Each arriving chunk rescanned the accumulated payload for its terminator,
producing O(N²) work. OSC, DSR, and graphics also independently copied the same
PTY chunks, and ordinary text was copied into and back out of the scanner.

`Scanner::feed_with` now visits borrowed text and ordered control notifications.
Only incomplete controls are buffered; only new fragments are scanned, including
split ESC/ST/DSR boundaries. Terminal no longer needs separate raw OSC/DSR buffers.
DSR replies use the cursor after preceding text has been parsed. Scanner capacity
is bounded at 32 MiB; completed/abandoned buffers above 64 KiB are released.
Oversized sequences are returned as borrowed text slices. The owned `feed` API
remains available for embedders. Text-only panes skip image-anchor byte accounting.

### Retained image budgets

A count-only limit of 256 placements allowed approximately 64 MB per image and
unbounded animation frames. Default per-terminal limits now cover:

- **128 MiB of actual pixel-buffer capacity**, across placements and frames;
  evict oldest complete placements, preserving retained animation frame numbering.
- **32 MiB of actual incomplete-transfer capacity**, across at most 16 Kitty IDs;
  discard the current transfer when it exceeds the aggregate budget.
- **256 frames per animation**, also bounding metadata for tiny frames.
- The existing **16,000,000 pixels per image**; PNG geometry is checked before
  allocating/decoding pixels.

Frame 0 shares its image allocation and counts once. Unused Vec capacity counts.
These are retained-resource limits, not process RSS limits: transient decode memory,
GPU textures, fonts, scrollback, and additional panes are separate costs.

Normal base64 no longer needs a sanitized copy; uncompressed bitmap input is
borrowed; owned RGBA and RGBA PNG buffers are reused. Base64 containing whitespace
still gets a sanitized copy. Kitty transfers with three or more fragments now
append intermediate `m=1` payloads instead of clearing preceding fragments.

### Measurements (release, versus the first-pass implementation)

Scanner tests extract protocol payloads without image decoding. The full input
benchmark feeds 16 MiB of colored output in 8 KiB chunks through a 20×5 terminal
with 100 history lines, including control scanning and VT parsing.

| Metric | Before | After |
|--------|--------|-------|
| 1 MiB sequence / 512 B fragments | 967.2 ms | 1.4 ms |
| 2 MiB sequence / 512 B fragments | 4,010.6 ms | 2.7 ms |
| 4 MiB sequence / 512 B fragments | 16,246.5 ms | 6.2 ms |
| Scanner capacity after 4 MiB sequence | 8 MiB | 0 |
| Full text input throughput | 74.4 MB/s | 129.8 MB/s |
| 4 MiB raw RGBA allocation traffic | 23 calls / 25,165,818 B | 1 call / 4,194,306 B |

Real-host comparisons use isolated state, the same shell, and a hidden test window
to prevent desktop typing from corrupting commands. CPU covers the app, not the
Python producer. The marker means producer write completion, followed by a 4-second
settle period; it does not measure end-to-end UI latency. Sampled RSS is not a
precise memory peak.

| Real-host workload | Before | After |
|--------------------|--------|-------|
| 4 MiB fragmented output: app work/settle CPU | 5.90 s | 0.27 s |
| Producer completion (including 0.2 ms sleep per fragment) | 5.89 s | 2.30 s |
| 40 × 1024×1024 PNG: settled RSS | 427.91 MiB | 395.70 MiB |
| Same images: RSS growth over idle | 165.14 MiB | 133.03 MiB |

Workload CPU drops about 95%; image RSS growth drops about 32 MiB, consistent with
the retained-pixel budget. Idle CPU is approximately 0–0.33% for both versions;
this pass does not claim an idle-power improvement. Allocators may retain freed
memory, so buffer capacity and RSS are reported separately.

### Reproduce

```sh
cargo test --release -p mtty-core -p mtty-graphics -p mtty-ptyhost -p mtty-app -- --ignored --nocapture
cargo build --release -p mtty-app -p mtty-cli
python3 scripts/profile-input.py --mode fragments
python3 scripts/profile-input.py --mode images
# Use --binary <old-binary> to compare a saved pre-change executable.
```

The real-host script currently targets macOS. Raw sample reports, configuration,
and JSON measurements stay in a temporary directory. CI perf now includes the
graphics crate, gating linear scan work, buffer release, zero-allocation text
streaming, and the absence of a second full-size RGBA allocation. Normal tests
cover all chunk boundaries, ordered DSR replies, multi-fragment Kitty reassembly,
aggregate image/transfer capacities, ID/frame limits, and PNG/compressed paths.
ConPTY and IME still require the real Windows-host checks in `WINDOWS-DEV.md`.
