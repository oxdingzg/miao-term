# Local patches to egui_commonmark 0.19.0

Only table layout is touched (`src/parsers/pulldown.rs`, `table` and the new
`wrapped_cell`):

1. Table cells were laid out with `ui.horizontal`, and egui truncates text in a
   horizontal layout. A long cell lost everything past the edge of the window,
   and scrolling the preview could not reveal it. Each cell now wraps its text
   at what is left of the row up to the table's right edge, keeping 60 pt for
   every column after it, so short columns keep their natural width and long
   ones fold inside the window.

Symptom without the patch: a README table's description column ends mid-word
at the editor's right edge in mtty's Markdown preview.

Upstream is MIT / Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`); only `src/`,
`Cargo.toml` and those licence files are vendored.
