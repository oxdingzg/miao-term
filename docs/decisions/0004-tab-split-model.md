# ADR 0004 — Own tab/split model (not OS-native)

> 简体中文: [`0004-tab-split-model.zh-CN.md`](0004-tab-split-model.zh-CN.md)

Status: accepted.

## Context

Ghostty's macOS app uses native `NSWindowTabGroup` (each tab is an NSWindow).
That is macOS-only and does not exist on Windows/Linux, so a cross-platform app
cannot rely on it.

## Decision

- Model windows ourselves: `Window → Tab[] → panes`. A tab holds one or more
  panes; the current split is **two panes** (Right or Down), focused by click.
- Tabs live inside one OS window (not OS-native tabs); splits live inside a tab.
- Shortcuts: `⌘T` new tab, `⌘W` close tab/pane, `⌘D` split right, `⇧⌘D` split
  down, `⌥⌘D` toggle details.
- The left sidebar lists tabs; each pane has its own PTY, size, and pane id.

## Consequences

- One behaviour on all platforms; no dependence on native tab APIs.
- We own the tab bar/sidebar rendering and focus model.
- The first cut supports one split level; a recursive split tree (N panes) is a
  later refinement with the same `Pane` abstraction.
- Update: the recursive split tree now exists (`crates/term-ui/src/layout.rs` —
  `Layout::Split` nests arbitrarily), so splits go beyond one level.
