# ADR 0005 — MTP transport

> 简体中文: [`0005-transport.zh-CN.md`](0005-transport.zh-CN.md)

Status: accepted.

## Context

The control plane (MTP) must be reachable by `miaotty-cli`, plugins and agent
hooks, on all three platforms, without coupling to the engine.

## Decision

- **Wire**: newline-delimited JSON (NDJSON), the existing `miaotty` MTP envelope
  (`v/id/kind/ns/method/params` → `v/id/kind/ok/result/error/revision`).
- **Transport**: Unix domain socket at `$TMPDIR/miaotty.sock` (current);
  Windows named pipe `\\.\pipe\miaotty` (planned). The path is exported as
  `MIAOTTY_SOCKET` so children (and the CLI) find it.
- **In-process UI** reads the registries directly (no socket round-trip);
  external callers go over the socket/pipe.
- Requests are bounded (`MAX_LINE`) so one client cannot exhaust memory.

## Consequences

- The existing `miaotty-cli` talks to the Rust host unchanged (verified).
- Windows needs a small transport shim behind the same interface.
- A crash in the socket server cannot take down the UI (separate thread).
