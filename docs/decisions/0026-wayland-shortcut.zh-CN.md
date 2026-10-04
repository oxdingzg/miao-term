# ADR 0026 — 原生 Wayland/X11 全局快捷键(门户)

> English (default): [`0026-wayland-shortcut.md`](0026-wayland-shortcut.md)

状态:已接受。

## 背景

ADR 0019 通过 `global-hotkey` 在 macOS/Windows 上给了系统级热键,但在 Linux 上该 crate 只会
X11;Wayland compositor 会忽略它。`GlobalShortcuts` XDG desktop portal 是与 compositor 无关
的机制,且其触发键由 compositor/用户拥有,而非由我们抢占。

## 决定

Linux 上 `crates/mtty-ui/src/hotkey.rs` 经 `ashpd`(MIT)用门户注册快速终端:

1. 专属线程跑一个 current-thread tokio runtime(`enable_all`)。
2. `GlobalShortcuts::new` → `create_session` → `bind_shortcuts(["quick"])`;门户可能提示用户,
   并由它决定实际按键组合。
3. 用 `poll_fn` + `Pin::new(..).poll_next` 轮询 `receive_activated()`(**不引入 `futures-util`**:
   `Stream` trait 经 `ashpd::zbus::export::futures_core` 再导出),`quick` 被激活时置位并唤醒 UI。

若门户不可用(无 `xdg-desktop-portal`、无头、或用户拒绝),`register` 返回 `None` 并记日志 ——
ADR 0019 的 compositor 绑定片段仍可作为替代。

依赖仅限 Linux 目标(`ashpd` + 小型 `tokio`),故 macOS/Windows 构建与 macOS 依赖集不变。

## 后果

- Wayland 用户通过标准门户获得全局快速终端快捷键,X11 用户也走同一门户(其转发到 X11)。
- 触发键在门户 UI 中选择,故配置的加速键在 Linux 上只是提示;这是门户的性质,不是 bug。
- 该路径仅 Linux,无法在维护者机器上运行;由 CI 的 `ubuntu-latest` 作业以及本地
  `cargo check --target x86_64-unknown-linux-gnu` 探针做编译检查。
