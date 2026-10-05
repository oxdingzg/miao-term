# The editor

[简体中文](EDITOR.zh-CN.md)

mtty puts a real editor next to the terminal, in the same tabs and splits, so a
file and the shell that builds it never leave the window. It is built on
`mtty-editor`, a rope-backed core with no UI code, so the same engine can be
embedded elsewhere. Editing is GPU-rendered through the terminal renderer,
which is what keeps very large files fast.

## Opening a file

The **Files** tab of the details panel lists the active directory; a click
opens the reader. Beyond that:

- **Open Quickly** (`⌘⇧O`) fuzzy-matches files, folders, recent files and text
  inside them, plus tabs, agents and saved hosts.
- **Edit in Tab** opens the current file in a local editor pane. It uses the
  `editor` setting, then `$EDITOR`, then `vi` (Notepad on Windows).
- **Open Externally** hands the file to an external editor from the palette.
- Jump-to-line navigation and `file:line` arguments open the pane at the line.

Local and remote files both open in a pane; remote files are read and written
over ssh (see [Remote hosts and SSH](REMOTE.md)).

## Syntax and large files

Highlighting uses 80 compiled-in **tree-sitter** grammars with incremental
reparse and visible-range queries. A file no grammar claims falls back to
syntect's Sublime syntaxes plus permissively licensed ones vendored from `bat`
(Julia, nginx, VHDL, Org and others); the language shows in the status chip.

Size is handled in stages rather than refused:

| File size | What happens |
|---|---|
| Up to 512 KB | Parsed on the UI thread; a keystroke reparses incrementally |
| Above 512 KB | The parse moves to a background thread; colour arrives shortly after |
| Above 8 MB | Tree-sitter stops (a tree costs 25–35× the file in memory); the Sublime fallback continues to 1 MB |
| Above 64 MB | The file opens in **view mode**: a window of lines is read from disk with a sparse index built in the background, so memory stays small at any size |

In view mode, typing, pasting or **Switch to Editing…** offers to load the file
for editing and states the memory it will take (about 2.2× the file). Find
scans the whole file on a thread.

## Editing

- **Multiple cursors**: `⇧⌘L` selects every occurrence (whole words from a bare
  caret), `⇧⌥I` puts a caret at each selected line's end, `⌥⌘↑` / `⌥⌘↓` add
  carets (`Ctrl+Alt+↑/↓` outside macOS).
- **Find and replace**: match case, whole word and regex; every match on screen
  is highlighted; *Replace*, *Replace All* (one undo step) and *Select All
  Matches* (`⌥↩`).
- **Go to Line** (`⌃G`) accepts `line:column`.
- Ordinary undo, redo, indent, grapheme- and word-wise motions, and a macOS-style
  keymap that keeps `⌘D`, `⇧⌘Z` and `⇧⌘L` for the editor.

### Vim mode

Set `editor-vim = true` for a vim state machine over the document:
`NORMAL` / `INSERT` / `VISUAL` / `VISUAL LINE`, counts, `h j k l w b e 0 ^ $ gg
G`, `i a I A o O`, `x`, `d`/`c`/`y` with motions (`dd`, `cc`, `yy`, `dw`, `d$`),
`p`/`P` with an internal register, `u` and `Ctrl-r`, `J`. `/` opens Find and `:`
runs `w`, `q`, `wq` or a line number; the `za` family drives folds. The status
bar shows the mode.

## Folding and outline

Folds come from the syntax tree (any named node spanning more than one line),
with indentation as the fallback. The gutter shows `▸`/`▾`, a collapsed header
ends in `⋯`, and up/down skip hidden lines. `⌥⌘[` / `⌥⌘]` fold and unfold;
*Fold All*, *Unfold All* and *Toggle Fold* are in the palette. `⌘R` opens a
filterable **outline** of the file's definitions, nested by depth.

## Markdown preview

A local Markdown file opens with a preview pane to its right that follows
typing. The renderer covers headings, lists, quotes, tables, code and links,
resolves relative images against the document's folder, and renders Mermaid
`graph`/`flowchart`, `sequenceDiagram`, `stateDiagram`, `classDiagram`,
`erDiagram` and `pie`. Set `mermaid-command` to shell out to `mermaid-cli` for
full Mermaid. Toggle the preview with *Toggle Markdown Preview* in the palette
and the View menu; it closes with its editor and is kept in a saved session.

## Language servers

With an `[lsp]` entry, the editor starts a language server for `rust-analyzer`,
`typescript-language-server`, `pyright-langserver`, `gopls` or `clangd` found on
the login shell's `PATH` — one server per language group and workspace root.
Files over 2 MB get none.

In the pane you get diagnostic underlines with ✖/⚠ counts in the status bar,
hover after the pointer rests, completion on trigger characters, words and
`Ctrl+Space` with client-side filtering (including snippets and import edits),
`F12` or `⌘`-click to a definition, and `F8` for the next problem. Servers and
root markers are configured under `[lsp]` in [`config.toml`](CONFIG.md).

## Saving without losing work

Saves are atomic and keep the file's permissions. A failed save is reported and
leaves the buffer **modified**; closing unsaved changes, Close Others/Below or
quitting asks first. An open pane watches its file: with no unsaved edits it
reloads in place as one undoable transaction; with unsaved edits it asks whether
to reload or keep your version; a file deleted underneath the pane is reported
once. Remote panes reload the same way over ssh.

## Configuration and keys

| Key | What it does |
|---|---|
| `editor` | The command *Edit in Tab* runs (for example `code --wait`) |
| `editor-vim` | Enable minimal vim mode |
| `mermaid-command` | Use `mermaid-cli` for full Mermaid rendering |
| `[lsp]` | Language servers, commands and workspace root markers |

[Keyboard shortcuts](SHORTCUTS.md) lists the window, terminal and editor keys.
