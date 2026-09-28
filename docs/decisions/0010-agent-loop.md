# ADR 0010 — Agent loop: notifications, sleep guard, prompt queue

Status: accepted.

## Context

An agent pane is only useful if the app tells you when it needs you and stays
out of the way while it works. Roadmap U6/M1 asks for system notifications, a
sleep inhibitor and a way to hand prompts to an agent — a *Composer* plus a
*Prompt Queue*.

## Decision

Three effects driven by the agent state published over MTP
(`agent.state.*`, keyed by pane):

**Notifications** (`miaotty-app/src/agentloop.rs`). The app keeps the last
state per pane and fires a system notification when a pane **transitions** into
`awaiting` or `error` while it is not the focused pane. Platform backends:
`osascript` (macOS), `notify-send` (Linux), `powershell`/BurntToast (Windows,
best-effort). Toggle with `notifications`.

**Sleep guard.** While any pane's agent is `processing`, a platform inhibitor is
spawned so the machine will not idle-sleep: `caffeinate -dims` (macOS),
`systemd-inhibit … sleep infinity` (Linux); the process is killed when no agent
is processing and on drop. Toggle with `prevent-sleep`.

**Composer and Prompt Queue.** ⌘⇧E (or the palette verb *Composer*) opens a
multi-line composer addressed to the focused pane: **Send** writes the text
plus Enter now, **Queue** appends it. The Details *Queue* tab lists queued
prompts with Send-now and remove. A queued prompt is delivered automatically
when its target pane's agent becomes `idle`.

## Consequences

- The agent loop is a pure consumer of MTP state, so it works with any agent
  that publishes state (claude, opencode, miao) and stays testable.
- Notifications and the sleep guard degrade to no-ops when a platform tool is
  missing; they can never fail the app.
- The queue is in-memory (session restore covers panes, not prompts); resuming
  a specific agent session and quota display remain follow-ups.
