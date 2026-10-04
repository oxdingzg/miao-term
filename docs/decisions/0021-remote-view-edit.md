# ADR 0021 — Remote view/edit over ssh

Status: accepted.

## Context

M4's acceptance is "a remote pane can view/edit files". miaotty's usual answer is
an MTP method the host answers, but the `mtty-mtp` crate currently carries
another author's uncommitted changes, so editing its dispatcher would mix work.

## Decision

Implement the capability **through the existing ControlMaster ssh connection**,
with no MTP change and nothing installed on the host:

- `crates/mtty-ui/src/ssh.rs` gains `read_remote(dest, path)`
  (`ssh … dest 'cat -- <path>'`) and `write_remote(dest, path, data)`
  (`ssh … dest 'cat > <path>'` fed on stdin), both reusing `ControlMaster=auto` /
  `ControlPersist=60s` and `BatchMode=yes`, and capped at 2 MB per read. The argv
  builders are pure and unit-tested (`read_args`, `write_args`, quoting).
- Open Quickly gains *View Remote File…* and *Edit Remote File…*. They open a
  two-field dialog (ssh destination, remote path); the read runs on a thread so
  the UI never blocks, and the result opens in the reader (view) or the editor
  (edit).
- The editor remembers a `RemoteRef`; Save then writes back over ssh and Reload
  re-reads, so a remote file behaves like a local one from the user's side.

## Consequences

- Remote view/edit works today, riding the same connection the pane already
  uses, with zero remote install — consistent with M4's zero-install fallback.
- The read/write path is only as available as ssh (ControlMaster established,
  key-based or agent auth for `BatchMode`); failures surface in the status line
  rather than as a hang.
- Exposing `app.view` / `app.edit` (and `file.read` / `file.write`) as first-class
  MTP methods — so a *tunnelled* client can trigger this without running our
  binary locally — remains a follow-up, gated on landing the pending `mtty-mtp`
  changes first.
- Update: the `mtty-mtp` pending changes turned out to be `cargo fmt` fallout,
  not another author's work; `app.view` / `app.edit` and `file.read` /
  `file.write` landed in ADR 0024, so this route is no longer the only one.
