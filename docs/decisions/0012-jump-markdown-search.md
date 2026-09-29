# ADR 0012 — Recent files, Markdown preview, content search

Status: accepted.

## Context

U5's remaining polish: Open Quickly should reach files you were just in
(`jump`), the viewer should render Markdown instead of only raw text, and there
should be a way to search file *contents* (not just names) from the keyboard.

## Decision

**Recent files (`jump`).** Every file opened in the reader/editor is pushed to
the front of `recent_files` (deduped, capped at 50) and persisted in
`session.json` (`#[serde(default)]`). Open Quickly lists them as `recent`
entries *before* tabs, so a recency-ordered tie-break falls out of the existing
stable sort by `(score, index)`.

**Markdown preview.** The reader renders a dependency-free subset of Markdown
when the file is `.md`/`.markdown` and read-only: ATX headings, fenced code,
bullet lists, block quotes, blank-line spacing; inline `*`/`_`/backtick markers
are stripped by `strip_inline`. A *Raw* / *Markdown* toggle is available, and
editing still uses the plain text editor.

**Content search.** The panels worker (ADR 0009) gains a `search` field on
`Request`; when set it scans files under the pane's cwd (case-insensitive,
depth ≤5, 600 files, 256 KiB per file, 40 hits) and publishes `Hit { path,
line, text }`. Open Quickly shows hits as `symbol` entries (kind shows
`file:line`) and opens the file. Typing `#term` in the palette drives the
search; requests are debounced to 200 ms so keystrokes do not flood the worker.

## Consequences

- Search and opening share one code path (`OpenFile`) and one reader.
- The work stays off the UI thread and is bounded, so a large tree degrades by
  finding fewer hits rather than by stalling.
- Line-accurate jumps into the editor, richer Markdown (tables/links/Mermaid)
  and a fuzzy-ranked `jump` are follow-ups.
- Update: line jumps and links landed in ADR 0015, tables in ADR 0017, images in
  ADR 0020, and a Mermaid subset later; a fuzzy-ranked `jump` remains a
  follow-up.
