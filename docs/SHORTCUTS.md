# Keyboard shortcuts

[简体中文](SHORTCUTS.zh-CN.md)

`⌘` is the macOS primary modifier; the Linux and Windows column is the
equivalent chord on those platforms.

## The window

| macOS | Linux / Windows | Action |
|---|---|---|
| `⌘T` | `Ctrl+Shift+T` | New tab |
| `⌘W` | `Ctrl+Shift+W` | Close the focused pane (or tab) |
| `⌘D` / `⇧⌘D` | `Ctrl+Shift+D` / `Ctrl+Shift+Alt+D` | Split right / split down |
| `⌘[` / `⌘]` | `Ctrl+Shift+[` / `Ctrl+Shift+]` | Previous / next pane |
| `⇧⌘[` / `⇧⌘]` | `Ctrl+PgUp` / `Ctrl+PgDn` (or `Ctrl+Tab`) | Previous / next tab |
| `⌘1`…`⌘9` | `Alt+1`…`Alt+9` | Go to tab |
| `⇧⌘L` / `⇧⌘R` | `Ctrl+Shift+Alt+L` / `Ctrl+Shift+Alt+R` | Toggle the sidebar / details panel |
| `⇧⌘T` | `Ctrl+Shift+Alt+T` | Quick Terminal (scratch tab) |
| `⇧⌘Z` | `Ctrl+Shift+Alt+Z` | Reopen the last closed tab |
| `⌘,` | `Ctrl+,` | Settings |

## Finding and running things

| macOS | Linux / Windows | Action |
|---|---|---|
| `⌘K` (or `⇧⌘P`) | `Ctrl+Shift+K` (or `Ctrl+Shift+P`) | Command palette |
| `⇧⌘O` | `Ctrl+Shift+Alt+O` | Open Quickly (tabs, agents, files, hosts) |
| `⌘F` | `Ctrl+Shift+F` | Find |
| `⌘G` / `⇧⌘G` | `Ctrl+Shift+G` / `Ctrl+Shift+Alt+G` | Next / previous match |
| `⇧⌘H` | `Ctrl+Shift+Alt+H` | Hints (open a link or path by its label) |
| `⌘E` | `Ctrl+Shift+E` | Composer (multi-line prompt to the focused pane) |

## The terminal

| macOS | Linux / Windows | Action |
|---|---|---|
| `⌘+` / `⌘-` | `Ctrl+=` / `Ctrl+-` | Larger / smaller font |
| `⌘C` / `⌘V` | `Ctrl+Shift+C` / `Ctrl+Shift+V` (also `Ctrl+V`, and `Ctrl+C` with a selection) | Copy / paste |
| `Shift+PgUp` / `Shift+PgDn` | Scroll the viewport |

Plain `Ctrl` chords (`Ctrl+C`, `Ctrl+W`, `Ctrl+D`, …) always reach the shell on
Linux and Windows, and `Super`/`Win` combinations are left to the desktop.

## The editor pane

In an editor pane these take precedence over the window shortcuts above. The
language features need a server for the file's language (see `[lsp]` in
[`config.example.toml`](config.example.toml)); hovering over code shows its
type, docs and problems.

| macOS | Linux / Windows | Action |
|---|---|---|
| `⌘D` | `Ctrl+D` | Select the word, then add the next occurrence |
| `⇧⌘L` | `Ctrl+Shift+L` | Select every occurrence |
| `⌥⌘↑` / `⌥⌘↓` | `Ctrl+Alt+↑` / `Ctrl+Alt+↓` | Add a caret above / below |
| `⌥`-click | `Alt`-click | Add a caret |
| `⇧⌥I` | `Shift+Alt+I` | A caret at the end of each selected line |
| `⌥⌘F` | `Ctrl+H` | Find and replace (`Aa` case, `ab` whole word, `.*` regex) |
| `⌥↩` in Find | `Alt+Enter` in Find | Select all matches |
| `⌃G` | `Ctrl+G` | Go to line (`line` or `line:column`) |
| `⌃Space` | `Ctrl+Space` | Completions (they also open as you type; `↩`/`⇥` accept) |
| `F12` or `⌘`-click | `F12` or `Ctrl`-click | Go to definition |
| `F8` / `⇧F8` | `F8` / `Shift+F8` | Next / previous problem |
| `⌘Z` / `⇧⌘Z` | `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
