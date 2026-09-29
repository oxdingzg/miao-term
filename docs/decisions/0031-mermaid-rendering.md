# ADR 0031 — Mermaid in the Markdown preview

Status: accepted.

## Context

` ```mermaid ` blocks in the built-in Markdown preview used to show a
"not rendered" placeholder. Real Mermaid is a large JS library that relies on
browser DOM measurement, so it cannot run in-process; and a half-compatible
renderer that silently draws the wrong diagram is worse than an honest
placeholder (the same principle as ADR 0029).

## Decision

Two layers, both in `miao_term_ui::mermaid`:

1. **A built-in subset** (`parse` + `show`) for the common case: `graph` /
   `flowchart` with `TD|TB|BT|LR|RL`, `id[Label]` / `id(Label)` / `id{Label}`
   (plus bare ids), and `-->` / `---` / `-.->` / `==>` edges, optionally
   labelled (`-->|text|` or `-- text -->`) and chained (`A --> B --> C`). Nodes
   are laid out by longest-path layering and drawn with `egui` shapes. Anything
   it cannot parse returns `None` and the caller keeps the placeholder.
2. **An optional external renderer** (`render_external`), used only when
   `mermaid-command` is configured (e.g. `mmdc`). It runs the tool once per
   diagram, caches the PNG next to the session file, and falls back to (1) if the
   tool is missing or fails. Default: unset (no process spawn).

Both hosts render local images in the preview via the shared
`miao_term_ui::markdown` helpers; the native preview also gains an image loader.

## Consequences

- Flowcharts render offline with no new dependencies and no license risk.
- Other diagram types (sequence, class, gantt, …) are **not** drawn: they show
  the placeholder unless `mermaid-command` is set, in which case the external
  tool renders them at full fidelity.
- The subset can differ from real Mermaid in layout/styling; this is documented
  rather than hidden. The external renderer is the escape hatch for anything
  important.
