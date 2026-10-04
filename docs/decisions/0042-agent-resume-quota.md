# ADR 0042 — Agent session resume and quota display

> 简体中文版: [`0042-agent-resume-quota.zh-CN.md`](0042-agent-resume-quota.zh-CN.md)

Status: accepted (2026-10-03). Implementation is tracked as M7/A4 in
[`../PRODUCT.md`](../PRODUCT.md).

## Context

The agent loop (ADR 0010, 0016, 0040) already knows, per pane, which agent
runs, its state, and the session id its hook reports: MTP `agent.state.set`
carries `agent`, `state`, `session_id` and `tty`, the host keeps them in
`agent_states`, and the Agent tab shows them. `docs/PRODUCT.md` lists two
gaps as "Not available — not designed yet":

- **Resume.** After a pane or the app ends, the session id is known but there
  is no way to relaunch the agent into that session. Every CLI spells resume
  differently (`claude --resume <id>`, `codex resume <id>`, …), and miao
  already supports resume natively.
- **Quota.** A user running several agents cannot see how much of an agent's
  usage window is left without leaving mtty to ask the agent.

## Decision

### Resume

Each supported agent ([`integration::Agent`](../../crates/mtty-ui/src/integration.rs))
gains a `resume` command template with `{session}` and `{cwd}` placeholders;
an agent without one is simply not resumable. The exact flags are verified per
agent when wired.

- A new command **Resume Agent Session…** lists the sessions mtty has seen in
  this run — agent, session id, the pane's cwd, and when it was last seen,
  most recent first. Choosing one opens a tab whose cwd is the recorded one and
  types the agent's resume template into it.
- The record lives where the agent state already lives (`agent_states`,
  extended with the session id and cwd). It is in-memory for the run;
  persisting sessions across restarts is a separate, later decision.
- `mtty-cli`/MTP gain `agent.sessions` (list) and `agent.resume` (by pane id or
  session id) so a plugin or script can do the same. Reading needs
  `agent.state.read`; resuming adds a new `agent.resume` capability.
- Resuming types a command into a fresh pane's shell, the same way *Launch
  agent* does; mtty never runs the agent as its own child.

### Quota

The state report gains an **optional** quota object:

```json
"quota": { "used": 42, "limit": 100, "unit": "percent",
           "window": "5h", "resets_at": "2026-10-03T15:00:00Z" }
```

- `mtty-cli state` accepts `--quota FILE|-` (JSON) next to `--state`; the hook
  script forwards it unchanged.
- The Agent tab shows one compact line when a quota is present ("42% of 5h,
  resets in 3h") and nothing when it is absent. A threshold
  (`agent-quota-warn`, default 80%) tints it.
- mtty never calls a model or an account API: the quota is whatever the agent
  (or its hook) reports. miao reports it from its built-in integration over the
  existing `agent.state` channel.

## Consequences

- Resume is correct only while each agent's template matches its CLI; a wrong
  template fails visibly in the pane, like any mistyped command.
- No new network, credentials or background daemon: quota is push-only and
  optional, so agents that do not report it are unaffected.
- `agent_states` grows a session/cwd record; the prompt queue and badges keep
  working on the existing transitions.

## Alternatives considered

- **Shelling out to each agent to query quota on a timer** — rejected: it needs
  credentials, can prompt the user, and duplicates what the agents already know.
- **A single generic resume (`--continue`)** — rejected: it resumes the agent's
  most recent session, not necessarily the one tied to the pane the user picks.
