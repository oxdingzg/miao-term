# ADR 0024 — MTP view/edit and file read/write

Status: accepted.

## Context

ADR 0021 gave remote view/edit through ssh, but a *tunnelled* client (a remote
shell with `MIAOTTY_SOCKET` pointed at the app's socket) still had no way to ask
the app to open a file, or to read/write one, because those verbs did not exist
on the control plane. The `term-mtp` crate blocked this only because the working
tree had uncommitted changes — which turned out to be `cargo fmt` fallout from
this session, not another author's work, so it is safe to extend.

## Decision

Extend the MTP surface (`crates/term-mtp`):

- **`app.view` / `app.edit`** — `{ path }`; queue `Command::View/Edit(path)`. The
  app opens the file read-only in the reader (`view`) or in the editor (`edit`),
  so a client can drive the UI.
- **`file.read`** — `{ path }` → `{ data, bytes, truncated }`, capped at
  `MAX_FILE_BYTES` (2 MB) with a `truncated` flag; IO errors become an
  `io_error` response.
- **`file.write`** — `{ path, data }` → `{ ok, bytes }`.
- Capabilities `app.view.write`, `file.read`, `file.write` are advertised in
  `core.ping`.

The CLI gains matching subcommands: `miaotty-cli view <path>`,
`miaotty-cli edit <path>`, `miaotty-cli file read|write --path P [--data D]`.

## Consequences

- A tunnelled or local client can now open a file in the running app and read or
  write files through the host, closing M4's "remote pane can view/edit".
- `file.read` is bounded and returns `truncated`, so a client cannot make the
  host read an unbounded file; `file.write` is UTF-8 text, which is enough for
  the reader/editor flow.
- Auth remains "whoever can reach the socket" (the socket is user-private,
  0600); finer-grained capability gating and binary/streaming transfers are
  follow-ups.
