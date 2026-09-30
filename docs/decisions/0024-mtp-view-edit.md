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

## Addendum (auth, binary transfer, tunnelled clients)

- **Token**: when the host is started with `MIAOTTY_MTP_TOKEN`, every request
  must carry a matching `token` field; otherwise it is answered with
  `unauthorized`. Clients read the same variable and add the field themselves.
- **Offset/length + base64**: `file.read` accepts `offset`, `length` (capped at
  `MAX_FILE_BYTES`) and `encoding = "base64"`, and reports
  `bytes/offset/returned/eof/truncated`; `file.write` accepts `data_b64`. That
  makes binary files transferable in bounded chunks. base64 is implemented in
  the crate (no new dependency).
- **Tunnelled client**: verified end to end (Mac host, Linux client) by
  forwarding the socket with `ssh -R /tmp/fwd.sock:<host socket>` and running
  `miaotty-cli --socket /tmp/fwd.sock ping` on the remote: without the token it
  is refused, with it `ping` reports the caps, and a base64 write+read round-trip
  returns the same bytes.

## Consequences

- A tunnelled or local client can now open a file in the running app and read or
  write files through the host, closing M4's "remote pane can view/edit".
- `file.read` is bounded and returns `truncated`, so a client cannot make the
  host read an unbounded file; `file.write` is UTF-8 text, which is enough for
  the reader/editor flow.
- Auth remains "whoever can reach the socket" (the socket is user-private,
  0600); finer-grained capability gating and binary/streaming transfers are
  follow-ups.
- Update: a `remote-listen` TCP listener (requiring `MIAOTTY_MTP_TOKEN`) and an
  `MIAOTTY_MTP_ALLOW` capability allowlist were added later, so auth is no longer
  only "whoever can reach the socket"; the local socket stays user-private
  (0600).

- Update: the protocol also gained `core.wait` (long-poll on the state
  revision) and `core.subscribe`, which upgrades a connection to a pushed event
  stream for the `agent.state`, `panes` and `history` topics
  (`miaotty-cli events [--topic T]`); a client drops the subscription by closing
  the connection.
