# ADR 0007 — View rule engine

Status: accepted.

## Context

A pane's identity in miaotty is derived from its context, not typed by hand.
Given a working directory, a running command, an agent, a host or a file, the
app should know the pane's short alias, icon, tab title and badge, and which
Details components to show. This mapping is the *view rule engine* and is the
data source for the tab bar, the Details panel and Open Quickly (U3), so it
lands before them (roadmap U2).

## Decision

A single **rule set** is loaded from `~/.config/miaotty/views.json` (JSON, so it
is user-editable and round-trippable). It lives in `miao-term-config::view`
(pure data + matching, no dependency on the terminal core) and is:

```jsonc
{
  "rules": [
    {
      "name": "work repo",                 // editor-only label
      "match": { "path": "~/work/**" },    // path|command|agent|host|file, glob
      "alias": "work",                     // short name for {alias}
      "icon": { "name": "git-branch", "color": "#81a1c1" }, // or { "emoji": "🌿" }
      "title": "{alias} · {folder}",       // template, see variables
      "badge": "repo"                      // tab badge key (optional)
    },
    { "match": { "agent": "claude" }, "icon": { "name": "claude" } },
    { "match": {}, "title": "{osc_title}" } // `any` catch-all
  ],
  "projects": [ { "path": "~/work/miao", "alias": "miao" } ],
  "worktree_suffix": true
}
```

- **Match kinds**: `path` (glob over cwd, `~` expanded), `command` (glob over
  the foreground command), `agent` (exact name), `host` (glob), `file` (glob
  over the pane's current file). A rule with an empty `match` matches anything
  (`any` catch-all).
- **Evaluation**: rules are an ordered list; the **first** rule whose *every*
  present matcher matches wins (`move_up`/`move_down` reorder). Iterate once,
  O(rules × matchers).
- **Title template**: `{alias} {cwd} {folder} {user} {host} {agent} {branch}
  {command} {title} {shell} {index} {file}`, plus `{osc_title}` for the raw
  program title. Unknown variables render empty. `folder` is the last path
  component; `cwd` is abbreviated (`~` for home).
- **Icon**: either a built-in icon name (resolved by the UI from a hand-authored,
  in-repo painter-primitive set — no SVG), an emoji, or a plain color. The
  engine only carries the descriptor.
- **Projects**: `path → alias` with longest-prefix match; when
  `worktree_suffix` is on and cwd is a `.worktrees/<name>` checkout, the alias
  gets that suffix.
- **Persistence**: the app reads on startup and writes on change; a missing or
  malformed file falls back to an empty rule set (the plain terminal title).

## Consequences

- Tabs/Details/Open Quickly all consume one `View` value per evaluation, so they
  stay consistent and are testable without a renderer.
- A malformed `views.json` degrades to OSC titles rather than breaking startup.
- The icon set itself (29 hand-authored painter primitives in
  `miaotty-app/src/icons.rs`) and the rule *editor* UI are follow-ups; this
  ADR covers the model and the matching semantics they build on.
