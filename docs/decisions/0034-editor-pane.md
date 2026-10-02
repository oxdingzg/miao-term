# ADR 0034 — A native editor pane

> 中文: [`0034-editor-pane.zh-CN.md`](0034-editor-pane.zh-CN.md)

Status: accepted.

## Context

mtty edits files in a floating egui window: one `TextEdit` over a `String`,
a hand-written highlighter for six languages, no multi-cursor, no LSP, and
every keystroke re-lays out the whole file. The goal is for mtty to be a
first-class text editor — fast on large files, highlighting most languages,
keyboard-first — sitting in the same tabs and splits as its terminals, where
agents and the shell already are.

The deciding parts of an editor are its core (text storage, edits, undo,
syntax analysis) and its renderer, not the GUI toolkit. mtty already draws
terminal panes itself (wgpu + glyphon/cosmic-text, ADR 0030) under a
performance gate; egui only draws the chrome. Switching toolkits (GPUI,
iced, Xilem) would mean rewriting the chrome and still writing the editor
core.

## Decision

**An editor is a pane, peer of the terminal**, rendered by mtty's own glyph
renderer and laid out by the existing split tree.

1. **Pane kinds.** A tab holds terminal panes and editor panes side by side
   (`Tab::panes`, `Tab::editors`); the layout tree names either by id and is
   unchanged, so the terminal code paths stay as they are. (Implementation
   note, E2: this replaced the planned `PaneContent` enum, which would have
   touched every one of the 66 `pane.term` call sites.) Session entries gain
   `editors` with path, cursor and scroll; MTP `pane.list` reports each pane's
   `kind`. `pane.send`/`pane.run` to an editor insert the text at its carets
   (one undo step); `pane.close` leaves an editor with unsaved changes open.
2. **A GPU-free core crate, `term-editor`.** A rope (`ropey`) buffer;
   transactions with an undo/redo history; multiple selections; search
   (`regex`) over the rope; line-ending and UTF-8/BOM detection; reload on
   external change. Pure functions with unit tests and its own perf bench.
3. **Syntax with tree-sitter.** Incremental reparse after each edit and
   highlighting of the visible range only; a compiled-in set of grammars
   (about 20 to start), highlight queries mapped to the mtty theme, plain text
   for unknown files. The same trees later give folding, outline and
   structural selection. Chosen over syntect, which re-highlights from the top
   and has no tree. (Implementation note, E3: 80 grammars are compiled in,
   each with the highlight query its crate ships; the macOS binary grew from
   39.8 MB with 20 grammars to 87.8 MB. Files no grammar claims fall back to Sublime syntaxes through
   syntect: its default set plus permissively licensed syntaxes vendored from
   bat, parsed line by line from checkpoints every 64 lines. Above 512 KB
   the parse runs on a background thread, with edits made meanwhile replayed
   on its result; tree-sitter stops at 8 MB, because a tree takes 25-35
   times the file in memory, and the Sublime fallback at 1 MB.
   (Implementation note, large files: above 64 MB a file opens in view
   mode. It is read in place: a background thread indexes every 1024th
   line start, and the pane's document is a window of about 3000 lines that
   moves as the view scrolls, so selection, copying and drawing are the
   editor's own. Edits ask first and load the whole file, stating the memory
   it takes, measured at about 2.2 times the file.)
   Licences are listed in `docs/third-party/SYNTAXES.md`.)
4. **Rendering through the terminal pipeline.** The editor builds the visible
   rows as styled spans on the monospace cell grid (tabs expanded, wide
   characters as two cells) plus quads for the gutter, selections and
   cursors. `term-render` gains per-span style (bold, italic, underline) and
   clips glyphs to the pane (today only to the window). Only visible lines are
   built, so a 100 MB file costs what a screen costs; a perf-gate entry holds
   that under the 4 ms frame budget.
5. **Input.** Keys go to the active pane's kind. Default keymap follows macOS
   text conventions (arrows, ⌥ word, ⌘ line, ⇧ select, ⌘Z/⇧⌘Z, ⌘F, ⌘D next
   occurrence, ⌥-click add cursor) with Ctrl equivalents elsewhere; IME
   preedit draws inline at the caret, as in terminals; the vim mode moves over
   in a later phase.
6. **Files.** Files panel, Open Quickly, `app.edit`, drops and hint mode open
   an editor pane (new tab, or a split with a modifier). Unsaved state shows on
   the tab; closing asks. Remote files keep the ssh read/write path.
7. **LSP later**, per workspace root over stdio: diagnostics, hover,
   completion, go-to-definition, configured per language in `config.toml`.
8. The floating editor window is retired once the pane covers it; Markdown
   preview becomes a preview pane beside the editor.

## Phases

| Phase | Content | Acceptance |
|---|---|---|
| E1 | `term-editor` core: rope, transactions, undo, selections, search | unit tests; edit/search bench on a 100 MB file |
| E2 | Editor pane MVP: pane kinds, styled spans and clipping, keys, mouse, IME, clipboard, open/save/close, session restore | replay tests; real-window capture; perf gate for the row build |
| E3 | tree-sitter highlighting (80 languages, a Sublime-syntax fallback for more), large-file mode, remote files | highlight snapshot tests; keystroke latency on a large file |
| E4 | Multi-cursor, find/replace, go to line, Markdown preview pane | replay tests |
| E5 | LSP: diagnostics, hover, completion, definition | against rust-analyzer and a TypeScript server |
| E6 | vim mode in the pane, folding and outline from the syntax tree, external-change reload | replay tests |

## Consequences

- New dependencies: `ropey`, `regex`, `tree-sitter` and grammar crates (C,
  built with `cc` on every platform's toolchain), later an LSP client.
  Grammar and query licences are checked per ADR 0006.
- The renderer stays monospace; proportional fonts and rich text are out of
  scope.
- Panes are no longer all terminals: broadcast input, MTP `pane.send` and
  read-only mode must say what they do for an editor pane (E2).
