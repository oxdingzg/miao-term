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
2. **A GPU-free core crate, `mtty-editor`.** A rope (`ropey`) buffer;
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
   (Implementation note, E6: folding follows the tree — any named node
   spanning more than one line, outermost per start line, with indentation as
   the fallback when there is no tree or it has no such node; the gutter shows
   ▸/▾, a collapsed header ends in ⋯, up/down skip hidden lines, ⌥⌘[ / ⌥⌘]
   fold/unfold, and the palette has Fold All / Unfold All / Toggle Fold. The
   outline lists definition nodes (functions, classes, structs, enums,
   traits/interfaces, impls, modules, constants, types, variables, macros)
   labelled by the node's `name` field and nested by depth; ⌘R opens a
   filterable picker.)
4. **Rendering through the terminal pipeline.** The editor builds the visible
   rows as styled spans on the monospace cell grid (tabs expanded, wide
   characters as two cells) plus quads for the gutter, selections and
   cursors. `mtty-render` gains per-span style (bold, italic, underline) and
   clips glyphs to the pane (today only to the window). Only visible lines are
   built, so a 100 MB file costs what a screen costs; a perf-gate entry holds
   that under the 4 ms frame budget.
5. **Input.** Keys go to the active pane's kind. Default keymap follows macOS
   text conventions (arrows, ⌥ word, ⌘ line, ⇧ select, ⌘Z/⇧⌘Z, ⌘F, ⌘D next
   occurrence, ⌥-click add cursor) with Ctrl equivalents elsewhere; IME
   preedit draws inline at the caret, as in terminals; the vim mode moves over
   in a later phase. (Implementation note, E6: vim mode is a `mtty-editor`
   state machine over the document — NORMAL/INSERT/VISUAL/VISUAL LINE, counts,
   `h j k l w b e 0 ^ $ gg G`, `i a I A o O`, `x`, `d`/`c`/`y` with motions
   (`dd`/`cc`/`yy`, `dw`, `d$`), `p`/`P` with an internal register, `u` and
   Ctrl-r, `J`; `/` and `:` go to the host's Find bar and a small `:` command
   line (`:w`, `:q`, `:wq`, `:<line>`), and `za`/`zo`/`zc`/`zR`/`zM` drive
   folds. The pane shows its mode in the status bar. It is on with the
   `editor-vim` config key.)
6. **Files.** Files panel, Open Quickly, `app.edit`, drops and hint mode open
   an editor pane (new tab, or a split with a modifier). Unsaved state shows on
   the tab; closing asks. Remote files keep the ssh read/write path.
   (Implementation note, E3 remote files: the remote-file dialog reads the
   bytes over ssh on a background job and opens them in an editor pane; `⌘S`
   and vim `:w` write them back the same way, and the pane stays modified if
   it changed while the write ran (`:wq` closes once the write lands). A
   remote pane keeps no local disk stamp, so external-change polling is
   skipped; it gets no language server (its path is on the host, not the local
   disk) and no view mode; its title is `host:name`. The session stores the
   destination and path and re-reads the host on restore.)
   (Implementation note, E6: an open pane remembers the file's length and
   modification time and re-checks it about once a second. A change with no
   unsaved edits reloads in place as one undoable transaction that replaces
   only the span that differs, with the cursor mapped through it, and leaves
   the pane clean; with unsaved edits mtty asks whether to reload from disk or
   keep the local version. A file deleted underneath the pane is reported once.
   View-mode panes, over 64 MB, are left alone.)
7. **LSP later**, per workspace root over stdio: diagnostics, hover,
   completion, go-to-definition, configured per language in `config.toml`. (Implementation note, E5:
   a GPU-free crate, `mtty-lsp`, speaks JSON-RPC over the server's stdio
   with serde_json alone. One server runs per language group and workspace
   root (the nearest folder with a project file, else the `.git` folder);
   it is started, initialized and read on background threads, and found on
   the login shell's `PATH`, which an app launched from the Dock lacks.
   Documents are synced whole on every change, as `textDocument/didChange`
   without ranges, which keeps the client simple and correct; files over
   2 MB get no server. Positions are negotiated as UTF-8 and fall back to
   UTF-16. Diagnostics underline their cells and count in the status bar;
   hover shows after the pointer rests 450 ms; completion opens on a
   trigger character, as a word is typed, or with Ctrl+Space, filtered on
   the client, and applies snippets as plain text with the caret on the
   first placeholder plus any extra edits (imports); F12 or ⌘-click goes to
   the definition, F8 to the next problem. Defaults cover rust-analyzer,
   typescript-language-server, pyright, gopls and clangd; `[lsp]` in
   `config.toml` overrides them. Accepted against rust-analyzer 1.94 and
   typescript-language-server 5.3 with TypeScript 5.9.)
8. The floating editor window is retired once the pane covers it; Markdown
   preview becomes a preview pane beside the editor. (Implementation note, E4: a
   tab holds `previews` beside its terminals and editors; the layout names
   them by id like the others. A preview shows its editor's text as it is
   typed, rendered by egui inside the pane's card in the background layer,
   so dialogs stay above it; closing the editor closes its previews, and
   sessions keep them. Local Markdown files open in an editor pane with a
   preview to the right; the floating window remains for read-only views
   (`app.view`) until those move to panes. Find in an
   editor gained case, whole-word and regex options with replace; in the
   macOS app, menu shortcuts that are also editor chords (⌘D, ⇧⌘Z, ⇧⌘L)
   go to a focused editor when pressed, told apart from clicks by the event
   AppKit is handling.)

## Phases

| Phase | Content | Acceptance |
|---|---|---|
| E1 | `mtty-editor` core: rope, transactions, undo, selections, search | unit tests; edit/search bench on a 100 MB file |
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
