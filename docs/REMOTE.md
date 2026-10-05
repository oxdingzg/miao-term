# Remote hosts and SSH

[简体中文](REMOTE.zh-CN.md)

mtty runs remote work over the system **OpenSSH** client. There is nothing to
install on the machine you connect to, and credentials stay where OpenSSH and
your operating system already keep them — `ssh-agent`, the system keychain and
the key files under `~/.ssh`. mtty never stores a plaintext password, and none
of this needs an account: the host library and every operation below work
offline and stay on this machine.

This covers the terminal side of remote work. To drive a running mtty from a
script or another program, see [the `mtty-cli` control plane](CLI.md).

## The host library

Saved hosts live in `~/.config/mtty/hosts.toml` (next to
[`config.toml`](CONFIG.md); on Windows, `%APPDATA%\mtty\hosts.toml`). A file
with one `[[host]]` is enough:

```toml
[[host]]
name    = "prod-web"
address = "prod-web.example.com"
user    = "deploy"
port    = 22
group   = "production"
tags    = ["web", "eu"]
```

| Key | What it is |
|---|---|
| `name` | The label shown in the sidebar, the palette and `views.json` |
| `alias` | Connect with `ssh <name>` and let `~/.ssh/config` supply address, user, port and options |
| `address` / `user` / `port` | The connection target when no alias is used |
| `group` / `tags` | Grouping and search in the sidebar |
| `jump` | A jump host (`ssh -J`) for a host not described in `~/.ssh/config` |
| `mosh` | Connect with `mosh` instead of `ssh` (needs `mosh` at both ends; survives roaming and sleep) |
| `tmux` | Keep the remote shell in a named tmux session, so reconnecting returns to it |
| `forward` | Saved port forwards (see below) |
| `kind` | `ssh` (default), `serial`, `telnet` or `tcp` (see below) |

Hosts imported from `~/.ssh/config` are kept as `alias` entries, so options
you already set there — `ProxyJump`, `IdentityFile`, `ControlMaster` — keep
applying. The library is read when it changes; a file that fails to parse is
reported and never overwritten.

Open a saved host from the **HOSTS** section of the sidebar, from the command
palette, or through the `mtty://host/<name>` URL scheme.

## Connecting

*New SSH Session…* (Shell menu and palette) opens a login in a new tab. The
connection uses your `~/.ssh/config`, reuses a `ControlMaster` connection when
one exists, and installs no `terminfo` on the remote — mtty ships the entry it
needs through the connection.

Host keys are checked the way `ssh` checks them: a host that is **known** is
verified quietly; an **unknown** host shows its fingerprints and is trusted
only after you compare them; a **changed** key is refused. Generating a key and
copying it with `ssh-copy-id` run in a normal terminal tab, so a passphrase is
typed into `ssh` and never passes through mtty.

A restored SSH tab says it is disconnected and reconnects when you press Enter.
Set `ssh-auto-reconnect = true` to connect on startup instead.

## Port forwarding

Forwards are saved with the host and use ssh's own notation. Each runs as its
own connection and is listed in the **Ports** panel.

```toml
[[host]]
name = "prod-web"
[[host.forward]]
kind = "local"                       # local (-L) | remote (-R) | dynamic (-D)
spec = "8080:127.0.0.1:80"           # [bind:]port:host:hostport, or [bind:]port for -D
```

## File transfer

The two-pane **SFTP** browser connects with the OpenSSH `sftp` client in batch
mode, so it keeps your `~/.ssh/config`, jump hosts, agent and `ControlMaster`.
Upload, download and drag-and-drop run in the background with progress;
uploads show a busy state rather than a percentage while the remote confirms
the write.

A single remote file can also be viewed and edited in place: open it over ssh
and `⌘S` writes it back the same way. A remote pane polls the file and reloads
when it changes underneath you, as long as you have no unsaved edits.

## Snippets and broadcast

Reusable commands live in `~/.config/mtty/snippets.toml`:

```toml
[[snippet]]
name    = "disk usage"
command = "df -h"
tags    = ["ops"]
```

Run a snippet in the current pane, or pick several hosts and run it on each —
one tab each, quoting handled by `ssh -t` as usual. **Broadcast input** types
what you type into several panes at once, which is useful for keeping a group
of hosts on the same page.

## Serial, Telnet and raw TCP

`kind` selects a transport. *New Serial/Telnet/TCP Session…* is in the Shell
menu and the palette, and saved entries of every kind open from the sidebar and
the `mtty://host/<name>` scheme like SSH hosts.

```toml
[[host]]
name = "console"
kind = "serial"
[host.serial]
device = "/dev/ttyUSB0"
baud   = 115200
```

Telnet and raw TCP are unencrypted and marked as such; serial is not a network
transport at all. These sessions end when the connection drops, and shell
integration (working directory, command history) does not apply to them.

## PuTTY keys

*Hosts… → Import PuTTY Key…* reads a PuTTY `.ppk` (v2 and v3, Ed25519, RSA and
ECDSA), verifies it and re-encodes it as an OpenSSH key under a passphrase you
choose. There is no unencrypted output path.

## Security

mtty reads private keys only where OpenSSH would: it does not hold a decrypted
key of its own and does not forward them anywhere. The system OpenSSH client is
used on purpose — macOS, Linux and Windows all ship it, and moving to a
Rust-native stack is a recorded, revisitable decision, not an accident.

To report a vulnerability, see [Security](SECURITY.md). For the control plane's
own boundary, see [`mtty-cli`](CLI.md).
