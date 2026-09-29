# ADR 0016 — Agent integration automation and single instance

Status: accepted.

## Context

Two U6/system-level gaps remained: agents had to be wired to miaotty by hand,
and a second launch (e.g. opening another `ssh://` link) started a second app
instance instead of reusing the running one.

## Decision

**Agent hooks** (`miaotty-app/src/integration.rs`). The app knows a small set of
agents (claude, codex, opencode, miao) with their binary, launch command and
where the hook is registered. From *Settings → Agent integrations* you can:
- see whether each agent is on `PATH`;
- **Install hook**, which writes `~/.config/miaotty/hooks/<agent>.sh` (mode
  0755, syntax-checked with `sh -n`) that reports state via
  `miaotty-cli state <agent> --state <s>` with `--pane $MIAOTTY_PANE_ID`;
- **Copy snippet**, which copies the exact line to wire into that agent's own
  hook config;
- **Launch**, which opens a tab running the agent.

We deliberately do **not** edit an agent's config for the user; we generate the
script and hand over the snippet.

**Self-reporting agents.** `miao` ships a built-in plugin that reports the same
state vocabulary through `miaotty-cli` whenever `MIAOTTY_PANE_ID` is set, so it
needs no hook install or snippet. The UI marks it *built-in* and only offers
**Launch**.

**Single instance / deep link.** A launch checks the MTP socket; if an instance
is already listening, the new process writes a request under
`~/.local/share/miaotty/inbox/` and exits. The running instance drains that
directory each frame and opens a tab for any forwarded command (a bare launch
is just an activation). No new MTP method — and therefore no change to the
shared `term-mtp` crate — was needed.

## Consequences

- "Wire up an agent" is one button plus one paste, and never surprises a user by
  rewriting their tool config.
- `ssh://` and other links open where the user's session lives instead of
  spawning a duplicate window.
- True global hotkeys, deep-linking *into* an existing window's specific pane,
  and remote hooks remain follow-ups.
