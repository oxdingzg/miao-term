# ADR 0004 — 自研标签/分屏模型(非系统原生)

> English (default): [`0004-tab-split-model.md`](0004-tab-split-model.md)

状态:已接受。

## 背景

Ghostty 的 macOS 版用系统原生 `NSWindowTabGroup`(每个标签是一个 NSWindow)。这只在 macOS 有,
Windows/Linux 没有——跨平台应用不能依赖它。

## 决定

- 窗口/标签自研:`Window → Tab[] → panes`。一个 tab 含一个或多个 pane;当前分屏为**两 pane**
  (右侧或下方),点击切换焦点。
- tab 在同一个 OS 窗口内(不用系统原生标签);分屏在 tab 内。
- 快捷键:`⌘T` 新建标签、`⌘W` 关闭标签/pane、`⌘D` 右侧分屏、`⇧⌘D` 下方分屏、`⌥⌘D` 开关 details。
- 左侧边栏列标签;每个 pane 有独立 PTY、尺寸与 pane id。

## 后果

- 三平台行为一致;不依赖原生标签 API。
- 标签栏/侧栏渲染与焦点模型自持。
- 首版只支持一层分屏;递归分屏树(N pane)是后续细化,复用同一 `Pane` 抽象。
- Update:递归分屏树现已存在(`crates/term-ui/src/layout.rs` 的 `Layout::Split` 可任意嵌套),
  因此分屏超过一层。
