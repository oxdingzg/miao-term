# ADR 0029 — A minimal, opt-in vim mode

Status: accepted.

## Context

The built-in editor is an `egui` text field. Users who live in vim asked for
modal editing, but a half-compatible vim is worse than none, and the editor must
stay a normal text field for everyone else.

## Decision

A deliberately **small** vim mode, **off by default** (`editor-vim = true`
enables it), implemented as a pure key-to-state function so it is unit-tested:

- Modes: `Normal` / `Insert`. `Escape` returns to Normal; `i a I A o O` enter
  Insert (with the usual cursor/line effects).
- Normal-mode commands: `h j k l`, `0 $`, `w b`, `gg G`, `x`, `dd`, `u` (undo),
  and `:w` / `:q` for save/quit.
- In Normal mode the app owns the buffer: `egui`'s own edits are discarded each
  frame and the caret is placed from our state, so typing cannot corrupt the
  file. Insert mode is ordinary editing.
- Positions are **character indices**; edits rebuild the text (O(n) per edit),
  which is fine for the 2 MB cap the reader already enforces.

Explicitly **not** supported (and not pretended): registers, macros, counts,
marks, search motions, text objects, `p`, or ex commands beyond `:w`/`:q`.

## Consequences

- Nobody who does not enable it can be affected: the field behaves exactly as
  before when `editor-vim` is false.
- The engine (movements, edits, mode changes, `:w`/`:q`) is covered by unit
  tests; the `egui` wiring (buffer ownership, caret placement) is the part that
  needs a human at the keyboard, and it is opt-in so a rough edge there cannot
  break normal editing.
- A fuller vim would need a real editor widget rather than a text field; this
  ADR does not commit to that.

## Update (2026-09-29)

The model (and the `egui` glue) now live in `miao-term-ui::vim`, shared by both
hosts; the native editor honours `editor-vim` as well. No behaviour change.
