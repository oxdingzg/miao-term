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
| [0009](./0009-details-panels-editor.md) | Details panels and the file editor | accepted |
| [0010](./0010-agent-loop.md) | Agent loop: notifications, sleep guard, prompt queue | accepted |
| [0011](./0011-tab-groups-tree-recipes.md) | Tab groups, file tree, recipes | accepted |
| [0012](./0012-jump-markdown-search.md) | Recent files, Markdown preview, content search | accepted |
| [0013](./0013-system-integration.md) | URL schemes, Quick Terminal, i18n, update check | accepted |
| [0014](./0014-ssh.md) | SSH sessions and remote terminfo | accepted |
| [0015](./0015-editor-polish.md) | Editor polish: line jumps, source view, external open | accepted |
| [0016](./0016-integration-automation.md) | Agent integration automation and single instance | accepted |
| [0017](./0017-markdown-tables-editor-config.md) | Markdown tables and the external editor | accepted |
| [0018](./0018-performance-gate.md) | Performance gate | accepted |
| [0019](./0019-hotkey-deeplink.md) | Global hotkey and deep-link to a pane | accepted |
| [0020](./0020-markdown-images-gutter.md) | Markdown images/footnotes and the editing gutter | accepted |
| [0021](./0021-remote-view-edit.md) | Remote view/edit over ssh | accepted |
| [0022](./0022-update-download.md) | In-app update download and verification | accepted |
| [0023](./0023-perf-regression-gate.md) | Performance regression gate (shared baseline) | accepted |
| [0024](./0024-mtp-view-edit.md) | MTP view/edit and file read/write | accepted |
| [0025](./0025-self-install.md) | Install and relaunch (self-replacement) | accepted |
| [0026](./0026-wayland-shortcut.md) | Native Wayland/X11 global shortcut (portal) | accepted |
| [0027](./0027-conpty-teardown.md) | ConPTY teardown must not block | accepted |
| [0028](./0028-ci-perf-baseline.md) | CI-side performance baseline | accepted |
| [0029](./0029-vim-mode.md) | A minimal, opt-in vim mode | accepted |
| [0030](./0030-native-render-loop.md) | Native render loop (self-drawn grid) | accepted |
| [0031](./0031-native-app-menu.md) | Application menu in the macOS menu bar | accepted |
| [0032](./0032-rename-mtty.md) | The application is renamed mtty | Accepted |
