# ADR 0002 — Concurrency and lock discipline

Status: accepted.

## Context

The terminal hot path (`pty → vt → grid → renderer`) must never block on I/O or
GPU work, and must not add per-frame allocations or polling.

## Decision

- **Bootstrap (current)**: one **PTY reader thread** per pane does blocking
  `read()` and sends chunks over an `mpsc` channel; the **UI thread** drains the
  channel and feeds the VT parser (no lock shared with the UI). Redraw is
  requested only when output arrived (event-driven; no polling).
- **Target (R1+)**: adopt Alacritty's model — a `FairMutex<Term>` plus an
  `EventListener`; the lock covers **only** "parse one chunk" or "build render
  instances", never `read`/`write`/GPU submit. Reads and writes use separate
  handles so they cannot block each other (especially ConPTY).

## Invariants

1. The VT/screen state is owned by one thread at a time; no lock spans I/O or GPU.
2. Resize is computed on the UI thread and applied to both the model and the PTY.
3. Idle CPU ≈ 0: nothing polls; the reader thread blocks on `read`.
4. Exit status comes from OSC 133;D, not the process exit code.

## Consequences

- Simple and correct now; the channel adds one copy per chunk (acceptable).
- The R1 swap to `FairMutex` keeps the same external API (`term-core`), so the
  app is unaffected.
