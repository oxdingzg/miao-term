# ADR 0017 — Markdown tables and the external editor

Status: accepted.

## Context

The reader's Markdown subset lacked tables, and "Edit in Tab" always used
`$EDITOR`, ignoring a user's preferred editor.

## Decision

**Tables.** `markdown_ui` recognises a GFM pipe table: a header row containing
`|` followed by a separator row (`---|:--:|---`), then contiguous `|` rows. It
renders them in an `egui::Grid` (striped, per-column alignment from the
separator's `:` markers) and consumes the block in one pass. Cells run through
the same inline stripper as the rest of the renderer. Helpers `is_table_sep`,
`split_row` and `table_align` are unit-tested.

**External editor.** A new config key `editor` sets the command *Edit in Tab*
runs; when unset it falls back to `${EDITOR:-vi}`. The path is single-quoted, so
spaces and quotes are safe.

## Consequences

- Tables are read-only-correct without pulling in a Markdown library; inline
  formatting inside cells is limited to the stripper (no nested emphasis).
- Editors like `code --wait`, `nvim` or `emacsclient` can be pinned per user.
- Images, footnotes and Mermaid remain follow-ups; the separator/pipe heuristics
  cover the common GFM table shape.
- Update: images and footnotes landed in ADR 0020.
