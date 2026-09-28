# Architecture Decision Records

Short, append-only decisions (context → decision → consequences). Supersede
rather than rewrite. The overall design lives in [`../ARCHITECTURE.md`](../ARCHITECTURE.md).

Docs are English by default; keep a Simplified Chinese version in sync
(`*.zh-CN.md`).

| ADR | Title | Status |
|-----|-------|--------|
| [0001](./0001-stack.md) | Stack selection (pty/vt/render/window/UI) | accepted |
| 0002 | Concurrency & lock discipline | planned |
| 0003 | Terminal + egui co-frame rendering | planned |
| 0004 | Own tab/split model (not OS-native) | planned |
| 0005 | MTP transport (Unix socket / Windows named pipe) | planned |
| 0006 | License & dependency policy | planned |
