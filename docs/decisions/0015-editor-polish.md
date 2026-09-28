# ADR 0015 — Editor polish: line jumps, source view, external open

Status: accepted.

## Context

The reader opened a file but ignored *where* the user came from (a search hit or
a changed file), had no line numbers, and offered no way out to another editor.
U5's remaining polish.

## Decision

- **Line jumps.** `open_editor_at(path, line)` records a 1-based target line.
  Open Quickly's content-search hits pass their hit line; recent files and
  details pass `None`. The read-only view highlights the target line and scrolls
  to it once on open.
- **Source view.** Read-only, non-Markdown content renders as a line-numbered
  monospace view (`source_ui`), with the target line framed in a highlight
  colour and `scroll_to_rect` centering it. Editing still uses the plain text
  editor.
- **External open.** *Open Externally* launches the OS default app
  (`open` / `xdg-open` / `cmd /C start`); *Edit in Tab* opens a terminal tab
  running `${EDITOR:-vi} <path>`.
- **Markdown extras.** Horizontal rules (`---`/`***`/`___`) render as
  separators, and inline `[text](url)` links render as `hyperlink_to` segments
  (the rest of the line is plain text).

## Consequences

- Search results now land on the matching line, closing the loop between
  content search (ADR 0012) and the reader.
- All of this is presentation over strings and stays unit-tested
  (`link_segments`, `is_rule`, `heading`).
- In-editor line-number gutters while *editing*, richer Markdown (tables,
  images, Mermaid) and an `open_with` preference remain follow-ups.
