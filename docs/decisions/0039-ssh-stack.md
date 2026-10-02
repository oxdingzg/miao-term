# ADR 0039 — Keep the system OpenSSH client

> 中文: [`0039-ssh-stack.zh-CN.md`](0039-ssh-stack.zh-CN.md)

Status: accepted.

## Context

M6 R3 asks whether SSH should move from the system `ssh` binary to a
Rust-native stack. mtty has always shelled out to `ssh` (ADR 0014): a pane runs
the command, so `~/.ssh/config`, `ProxyJump`, `ssh-agent`, host-key prompts and
ControlMaster are all OpenSSH's own, and `ssh -G` is how mtty reads the
resolved options (host library, view rules).

A native client (the permissive candidate is `russh`, Apache-2.0) would remove
the external dependency, allow in-process SFTP and port forwarding, and give
Windows a path that does not need OpenSSH. The cost is re-implementing a large,
subtle surface: `~/.ssh/config` parsing with all its option semantics,
ProxyJump chains, agent forwarding, host-key verification, connection reuse,
and every key type and algorithm — and then keeping it equal to whatever
OpenSSH a user already has.

## Decision

**Keep the system OpenSSH client.** It is present on macOS and Linux, and
Windows 10/11 ship it too; the ADR 0037 byte-pipe terminal means a native
backend could later slot into the same pane path without touching the rest of
the app, so nothing is lost by waiting.

Revisit only when one of these becomes true:

- a supported platform without OpenSSH must connect (not just install keys);
- an in-process SFTP or forwarding feature is required that the `ssh` binary
  cannot provide;
- a security or maintenance reason favours a native implementation.

If revisited, `russh` is the candidate; `ssh -G` and the saved-host fields
stay the source of truth so both backends read the same configuration.

## Consequences

- mtty keeps requiring the `ssh` binary; the installer already treats it as a
  runtime dependency, and the UI reports it when missing.
- PPK import (ADR 0038) is unaffected: it only converts key files on disk.
- The transport abstraction (ADR 0037) is where a native SSH backend would go,
  not the pane or the session model.
- No new dependency now; the decision is cheap to reverse because it is
  isolated behind the transport layer.
