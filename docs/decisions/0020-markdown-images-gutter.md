# ADR 0020 — Markdown images/footnotes and the editing gutter

Status: accepted.

## Context

Two reader gaps remained: images and footnotes in Markdown previews, and line
numbers while *editing* (the read-only view already had them, ADR 0015).

## Decision

**Images.** A line that is exactly `![alt](url)` renders as an image for local
raster files (`png/jpg/jpeg/gif/webp/bmp`) via `egui_extras` image loaders
(`image` + `file` features). Relative paths resolve against the document's
directory and are turned into `file://` URIs; remote `http(s)` URLs and unknown
extensions fall back to a labelled hyperlink. We deliberately do **not** enable
`egui_extras`' `svg` feature, which would pull MPL-licensed `resvg` (ADR 0006).

**Footnotes.** `take_footnotes` hoists `[^id]: text` definitions out of the
flow; inline `[^id]` references are rewritten to `[id]` by `footnote_refs`; the
definitions render under a *Footnotes* separator at the end. All three helpers
are unit-tested.

**Editing gutter.** In edit mode the editor is laid out with a fixed-width
gutter column beside a `TextEdit`; `paint_gutter` reads the
`TextEditOutput.galley` rows and `galley_pos`, so numbers track the text's real
row positions (including scroll) and increment on rows that end with a newline
(logical lines). The numbering is a pure helper (`gutter_numbers`) and is
tested.

## Addendum (gutter)

The editing gutter now sits **outside** the scroll area in its own column, so it
stays put when long lines scroll horizontally (the earlier limitation is gone).

## Consequences

- Preview fidelity improves without a full Markdown engine, and the animated
  raster decoders (`gif`) are off, so only static formats load.
- The gutter is inside the same scroll area as the text, so it scrolls
  horizontally with long lines; that is acceptable for code and can be pinned by
  moving the gutter outside the scroll area later.
- Mermaid/`svg` diagrams and remote images remain follow-ups (the latter need an
  HTTP loader and a network fetch).
- Update: the gutter now sits outside the scroll area (see the addendum), so it
  no longer scrolls horizontally with long lines.
- Update: Mermaid diagrams are drawn inline for `graph`/`flowchart`,
  `sequenceDiagram`, `stateDiagram`, `classDiagram`, `erDiagram` and `pie`; other
  types (gantt, journey, …) and the binary side of rendering still fall back to
  `mermaid-command` or a placeholder.
