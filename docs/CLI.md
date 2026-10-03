# The mtty-cli control plane

[简体中文](CLI.zh-CN.md)

`mtty-cli` drives a running mtty host from a shell, a script or another
program. It speaks **MTP** (mtty terminal protocol) to the application over a
per-user socket, and it is the reference client for that protocol.

## The socket

The control plane listens on `$XDG_RUNTIME_DIR/mtty.sock`, falling back to
`$TMPDIR`, and the socket is created with owner-only permissions. On Windows it
is a named pipe.

The shell in every pane inherits two variables:

| Variable | Meaning |
|---|---|
| `MTTY_SOCKET` | The socket to talk to |
| `MTTY_PANE_ID` | The pane that shell belongs to |

Pass `--socket PATH`, or set `MTTY_SOCKET`, to target a non-default socket.
`mtty-cli` also falls back to an older host's socket or pipe.

## State revisions and events

Every response carries a state `revision`. Two commands use it instead of
polling:

- `core.wait` blocks until the revision moves past a given value.
- `core.subscribe` upgrades the connection to an event stream, so a client can
  follow changes as they happen.

The event topics are `agent.state`, `panes` and `history`.

## Commands

```sh
mtty-cli ping
mtty-cli wait --since 42               # block until the state revision moves
mtty-cli events                        # stream state changes as JSON lines
mtty-cli events --topic agent.state    # ... filtered to one topic
mtty-cli pane list
mtty-cli pane run --pane ID --data "echo hello"
mtty-cli pane focus --pane ID
mtty-cli pane output --pane ID           # last command's output and exit status
mtty-cli state claude --state processing --pane ID
mtty-cli state list
mtty-cli history add --command "cargo test" --cwd "$PWD"
mtty-cli history list --pane ID
mtty-cli view /path/to/file            # open it read-only in the app
mtty-cli edit /path/to/file            # open it in the editor
mtty-cli file read  --path /etc/hosts  # bounded to 2 MB per call
mtty-cli file read  --path app.bin --base64 --offset 0 --length 65536
mtty-cli file write --path /tmp/x --data "hello"
mtty-cli file write --path /tmp/x --data-b64 "AAECAw=="   # binary
```

| Group | What it covers |
|---|---|
| `ping` | Liveness, and which capabilities the host allows |
| `wait`, `events` | Blocking on, or subscribing to, state changes |
| `pane` | List panes, run a command in one, focus one, read the last command's output and exit status |
| `state` | Report an agent's state for a pane, or list the panes that report one |
| `history` | Add to, or read, a pane's command history |
| `view`, `edit` | Open a file in the app, read-only or in the editor |
| `file` | Read and write files through the host, bounded to 2 MB per call |

File reads and writes are bounded to 2 MB per call; `--base64` and
`--data-b64` carry binary content, with `--offset` and `--length` to page
through a larger file.

## Remote access

Set `remote-listen = "127.0.0.1:7273"` to serve the control plane over TCP, and
connect with:

```sh
mtty-cli --socket tcp://host:7273 pane list
```

Without a token the TCP listener refuses to start.

## Tokens and capabilities

The control plane runs commands in your shell, so treat the token as a secret
and prefer a loopback address you control, or an ssh tunnel.

| Setting | Effect |
|---|---|
| `MTTY_MTP_TOKEN` | When the host is started with it, every request must carry it; the CLI picks it up from the same variable |
| `MTTY_MTP_ALLOW` | A comma-separated capability allowlist, e.g. `core.basic,file.read,history.read`; anything else returns `forbidden` |

Unset `MTTY_MTP_ALLOW` means everything is allowed, and `core.basic`
(ping/health) is always allowed so clients can discover the host. `ping` reports
the effective set in `allowed`.

The socket can be forwarded over ssh, so a remote client drives the host:

```sh
ssh -R /tmp/fwd.sock:<host socket>
```

## Talking to it directly

MTP is newline-delimited JSON over the socket, so a client does not need
`mtty-cli`: write a request on one line, read one response line back. Use
`core.wait` or `core.subscribe` rather than polling in a loop.
