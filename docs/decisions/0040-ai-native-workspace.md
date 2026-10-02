# ADR 0040 — AI-native workspace: agent edits, ACP, and context hand-off

> 中文: [`0040-ai-native-workspace.zh-CN.md`](0040-ai-native-workspace.zh-CN.md)

Status: accepted.

## Context

M7. Today an agent runs in a pane and mtty coordinates it through hooks and
OSC 133 (B2): it can see a pane's state, queue a prompt, and read a command's
output. It cannot (A1) turn an agent's edits into something the user reviews
and undoes, (A2) talk to any agent through the Agent Client Protocol, or (A3)
hand an editor selection, diagnostics or a command's output to an agent, or
let an agent open a file at a line.

The pieces already exist: MTP (ADR 0005) is the control channel, the editor
pane (ADR 0034) has a transaction/undo model, and terminal output is published
per command (B2.3).

## Decision

1. **A1 — agent edits are reviewable transactions.** MTP gains
   `editor.propose` (params: `pane_id`, an ordered list of `{start,end,text}`
   ranges in characters, or whole-file `text`, plus an optional label). The
   host applies them to the pane's `Document` as **one transaction** (one undo
   step) and marks each changed line as a pending proposal. The pane draws the
   proposal inline — added lines with a green gutter and background, removed
   text struck through — and the status bar offers **Accept** (drop the
   marking; the text is already applied) and **Reject** (`undo`, which restores
   the proposal's pre-image as one step). Typing anything, saving, closing or
   another proposal is treated as an accept. A proposal is per pane and is
   discarded if the pane is closed; nothing is written to disk until the user
   saves. MTP returns the undo depth so a client can tell accepted from
   rejected.
2. **A2 — an ACP client.** A GPU-free crate, `miao-term-acp`, speaks the Agent
   Client Protocol (JSON-RPC 2.0 over the agent subprocess's stdio, ADR 0006
   licenses). It implements `initialize`, `authenticate`, `session/new`,
   `session/load`, `session/prompt`, `session/cancel` and the streamed
   `session/update` notifications (agent message chunks, tool calls, plans,
   and diffs). Client-side `fs/read_text_file` / `fs/write_text_file` and
   `terminal/*` requests are answered from the editor panes and the active
   terminal, and `session/request_permission` opens a small Allow/Deny dialog.
   Agent diffs are applied through A1. Agents are listed in `config.toml` under
   `[acp]` (`command`, `args`, `env`, one entry per agent); *Launch Agent*
   starts one in a pane, and the ACP session drives that pane.
3. **A3 — hand context to an agent, and open at a line.** MTP `app.edit` and
   `app.view` gain optional `line` and `column`, so an agent can open a file
   at a position. The palette and shortcuts gain *Send Selection to Agent*,
   *Send Diagnostics to Agent* and *Send Last Command Output to Agent* (the
   existing Composer target is joined by the active agent pane); each types a
   short, quoted prompt into that pane (or ACP session), with no copying
   through the clipboard.
4. **Non-goals.** mtty never calls a model itself and stores no agent
   credentials; ACP agents are external programs, as the pane agents are.

## Phases

| Phase | Content | Acceptance |
|---|---|---|
| A3 | MTP `line`/`column`; send selection/diagnostics/output to an agent | unit tests for the prompt text; an MTP test for `app.edit` `line` |
| A1 | `editor.propose`, one-transaction apply, inline diff, accept/reject | editor tests for the transaction and undo; a replay/screenshot check |
| A2 | `miao-term-acp`: JSON-RPC client, sessions, streaming, fs/terminal/permission mapping | tests against a fake ACP agent; a real agent if installed |

## Consequences

- Agents stay external and interchangeable; ACP is additive, and the MTP path
  keeps working for agents that do not speak it.
- Reviewability is the default: an agent's change is one undo step and is never
  written without a save.
- The editor pane grows a proposal layer, and the app grows one dependency-free
  protocol crate; no model or key material enters the tree.
