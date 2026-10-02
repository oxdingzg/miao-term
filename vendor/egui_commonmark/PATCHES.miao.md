# Local patches to egui_commonmark 0.19.0

Two spots in `src/parsers/pulldown.rs` are touched: table layout (`table` and
the new `wrapped_cell`) and image paths (the `Tag::Image` start and the new
`absolute_file_uri`):

1. Table cells were laid out with `ui.horizontal`, and egui truncates text in a
   horizontal layout. A long cell lost everything past the edge of the window,
   and scrolling the preview could not reveal it. Each cell now wraps its text
   at what is left of the row up to the table's right edge, keeping 60 pt for
   every column after it, so short columns keep their natural width and long
   ones fold inside the window.

2. An image path without a scheme is prefixed with the implicit URI scheme.
   mtty sets that scheme to the document's directory (`file:///dir/`) so
   relative images resolve, which would have turned an absolute path into
   `file:///dir//abs/x.png`. Absolute paths now become a `file://` URL of
   their own before that prefix is applied.

Symptom without patch 1: a README table's description column ends mid-word
at the editor's right edge in mtty's Markdown preview.

Upstream is MIT / Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`); only `src/`,
`Cargo.toml` and those licence files are vendored.
