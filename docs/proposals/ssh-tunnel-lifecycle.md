# App-managed SSH tunnels

Status: proposal for requirements review; no runtime behavior changes in this PR.

## Problem and intended experience

A Windows user currently starts an SSH tunnel through Task Scheduler to obtain
access to a proxy or remote service. mtty should offer an opt-in replacement:
configure or import the tunnel once, start it when mtty starts, show its status,
and stop it when mtty exits. This should also work on macOS and Linux.

The existing scheduled task's forwarding mode and the applications consuming
its local port must be confirmed before implementation. A tunnel does not
configure a system-wide proxy by itself.

## Existing foundation

- `crates/term-config/src/hosts.rs` saves local (`-L`), remote (`-R`) and
  dynamic SOCKS (`-D`) forwarding rules under SSH hosts.
- `crates/term-ui/src/forward.rs` launches a dedicated `ssh -N` child with
  batch authentication, forwarding failure detection and keepalives. Its drop
  handler kills and waits for that child.
- The host manager in `crates/term-widget/src/lib.rs` provides manual
  start/stop and error display. Each window currently owns its tunnel map.
- Host aliases can be imported from SSH config; importing forwarding rules,
  app-wide ownership, automatic startup and reconnect still need design/work.

Reuse this implementation and system OpenSSH, consistent with
[ADR 0039](../decisions/0039-ssh-stack.md). Normal child cleanup is not yet a
promise that tunnels disappear after a crash or forced app termination.

## User-facing entry points

1. **Hosts → Port forwards** remains the source of host and rule configuration.
   Add a friendly name, “Start with mtty” and “Reconnect automatically” per rule.
2. **Command palette → SSH Tunnels…** opens one overview across hosts, with
   Add, Import, Start, Stop and error details.
3. **Status bar** shows aggregate tunnel health and opens the overview.
4. A later **Settings → Network** feature can select a tunnel for new local
   terminal sessions. It references existing rules instead of duplicating host
   credentials or connection settings.

Automatic startup is off by default. Saving or importing a rule never starts
it without an explicit user action or enabled startup preference.

## Required configuration

| Field | Requirement |
| --- | --- |
| Display name | A recognizable label for the tunnel |
| SSH destination | Existing SSH alias, or hostname, username and SSH port |
| Authentication | ssh-agent or a local identity file; no stored plaintext password |
| Jump host | Optional; preserve SSH config semantics |
| Forwarding mode | Local, remote or dynamic SOCKS |
| Listen address and port | Explicit address; local listeners default to `127.0.0.1` |
| Target address and port | Required for local/remote forwarding, absent for SOCKS |
| Startup and reconnect | Separate opt-in preferences |

For remote forwarding, clearly label that the listener is on the server and
that the target is reached from the client. A remote wildcard listener also
depends on the server's SSH policy. Binding beyond loopback requires explicit
confirmation in the configuration UI.

Credentials and machine-specific settings stay in private local configuration.
Encrypted keys may need the user's agent to be unlocked before unattended
startup. First-use host-key verification must be an explicit trust flow;
changed keys block connection. Do not disable host-key checking or rewrite the
user's SSH config.

## Import and sharing

The preferred migration path is **Paste SSH command**: copy the executable and
arguments from the scheduled task and preview the resulting form before saving.

- Parse supported SSH arguments such as `-L`, `-R`, `-D`, `-p`, `-i`, `-J` and
  approved `-o` options as data; never execute the pasted command or a shell.
- Support quoted Windows paths. Reject shell operators, unsupported executables
  and unrecognized options with an explanation instead of silently dropping
  behavior. Do not import background/detach options as runtime behavior.
- If a task calls a script or a different SSH client, explain how to extract
  its SSH settings; arbitrary script execution is outside the import feature.
- **Import SSH config** should let users select hosts and forwarding rules,
  preserve alias semantics and preview duplicates/conflicts. Existing host
  import must not be presented as already importing all forwarding settings.
- A versioned tunnel profile file can later support export/import with a
  preview. Exclude passwords, private keys and machine-specific identity paths;
  choose the local identity or alias mapping on the receiving computer.

Any actual scheduled-task command or server details used for review must be
redacted before posting to this public repository.

## Lifecycle and failure behavior

- “Start with mtty” means application startup, independently of terminal tabs.
  Closing a tab does not stop a tunnel. Closing a window stops tunnels only
  when it actually exits the app; background/tray residency keeps them alive.
- Use application-level ownership across windows. For multiple app processes,
  ensure one owner per rule/local endpoint or report an existing owner; do not
  silently start a second tunnel or attach to an unrelated listener.
- Connect asynchronously with a bounded timeout so startup never blocks the UI.
  Status distinguishes Stopped, Connecting, Connected, Reconnecting and Failed.
- A running SSH process alone is not proof of readiness. Confirm forwarding
  establishment; distinguish it from end-to-end proxy/service health.
- Retry transient network failures with bounded exponential backoff; recover
  after sleep/network changes. Authentication, trust and port-conflict errors
  require user action. Manual Stop cancels retries for this app session;
  changing the startup preference controls future launches.
- On normal exit, cancel retries and terminate owned SSH processes and jump
  children. On Windows, use a Job Object with kill-on-close and appropriate
  handle ownership to cover crashes and forced termination. Other platforms
  need their own cleanup strategy and verification before promising the same.
- Port conflicts show the endpoint and a useful error. Never kill or reuse the
  old scheduled task's process automatically. Let the user disable the task
  after verifying the replacement.
- If OpenSSH is unavailable, report how to install/enable it; do not silently
  install software or fall back to an insecure connection.

## Proxy use is a separate requirement

`-D` creates a SOCKS proxy. `-L` can expose an existing HTTP proxy or another
remote service. The server must support the intended forwarding and egress.

The first increment manages the tunnel only. If reviewers require terminal
commands or agents to use it automatically, include a separate, explicit proxy
integration design: protocol-aware environment variables for new local panes,
optional readiness gating, DNS behavior, and useful handling of tunnel failure.
Applications differ in proxy support, so environment variables alone cannot
promise that every tool uses the proxy. Existing panes, external applications
and Windows system proxy settings are not changed by the initial proposal.

## Delivery and acceptance

First increment: command import with preview, per-rule automatic startup,
central status/errors, reconnect and reliable Windows process cleanup. Profile
sharing and proxy integration follow unless review identifies them as essential
to replacing the current task.

Verify on a real interactive Windows host before claiming implementation done:

- Import representative `-D` and `-L` commands, including quoted identity paths;
  unsupported commands never execute and do not silently lose options.
- Establish a verified connection, save a startup rule, restart mtty and reach
  the intended service through its local endpoint.
- Exercise absent/locked credentials, unknown/changed host keys, unavailable
  SSH, unreachable server and occupied port; errors remain visible and useful.
- Disconnect the network and sleep/resume; observe reconnect without duplicates.
  Manual Stop prevents retries until explicitly started again in that session.
- Multiple windows/processes do not create conflicting duplicate listeners.
- Closing a tab preserves the tunnel. App exit and forced termination release
  the listener and owned jump processes; background residency follows the
  agreed product behavior.
- Coexistence with the old task reports conflicts without terminating it;
  migrate and disable that task manually only after a successful smoke test.

Implementation must also pass the repository's remote Rust checks and applicable
platform checks. This proposal is documentation only and makes no claim that
these runtime acceptance checks have passed.

## Questions for the scenario reviewer

- Is the existing tunnel dynamic SOCKS (`-D`), a local forward to an HTTP proxy
  (`-L`), or something else? A redacted command is sufficient.
- Which applications need it: commands/agents inside mtty, browsers, or other
  Windows applications? Must proxy configuration be part of the first release?
- Does authentication already work unattended through a key/agent, or does it
  require a password or interactive key unlock?
- Should closing the final window always exit mtty and stop the tunnel, or
  should background/tray operation keep it alive?
- Are command paste and SSH-config import sufficient, or is shareable profile
  import required for the first release?
- Are several simultaneous tunnels, non-loopback listeners or multiple mtty
  instances needed? Does any consumer require the old fixed local port?

## Technical references

- [OpenSSH forwarding, failure detection and keepalives](https://man.openbsd.org/ssh_config.5)
- [Windows Job Objects and process lifetime](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
