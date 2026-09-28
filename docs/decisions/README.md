# Architecture Decision Records

Short, append-only decisions (context → decision → consequences). Supersede
rather than rewrite. The overall design lives in [`../ARCHITECTURE.md`](../ARCHITECTURE.md).

Docs are English by default; keep a Simplified Chinese version in sync
(`*.zh-CN.md`).

| ADR | Title | Status |
|-----|-------|--------|
| [0001](./0001-stack.md) | Stack selection (pty/vt/render/window/UI) | accepted |
| [0002](./0002-concurrency.md) | Concurrency & lock discipline | accepted |
| [0003](./0003-co-frame-rendering.md) | Terminal + egui co-frame rendering | accepted |
| [0004](./0004-tab-split-model.md) | Own tab/split model (not OS-native) | accepted |
| [0005](./0005-transport.md) | MTP transport (Unix socket / Windows named pipe) | accepted |
| [0006](./0006-license-policy.md) | License & dependency policy | accepted |
| [0007](./0007-view-rule-engine.md) | View rule engine | accepted |
| [0008](./0008-open-quickly.md) | Open Quickly / command palette | accepted |
