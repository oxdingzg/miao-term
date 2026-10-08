# View rules

A pane's tab title, icon and badge are derived from its context by the *view
rule engine* (design: [ADR 0007](./decisions/0007-view-rule-engine.md)). Rules
live in `~/.config/mtty/views.json` (JSON). mtty reloads the file when it
changes, so an edit shows within a couple of seconds of the next activity; an
in-app rule editor is not available yet. A fresh install seeds this file with
an empty rule set on first start, so there is always something to edit.

```jsonc
{
  "rules": [
    {
      "name": "work repos",              // editor-only label
      "match": { "path": "~/work/**" },  // path|command|agent|host|file
      "alias": "work",                   // short name for {alias}
      "icon": { "name": "git-branch", "color": "#81a1c1" },
      "title": "{alias}:{folder}",
      "badge": "repo"                    // optional
    },
    { "match": { "agent": "claude" }, "icon": { "name": "claude" } },
    { "match": {}, "title": "{folder} · {osc_title}" }  // catch-all
  ],
  "projects": [ { "path": "~/work/miao", "alias": "miao" } ],
  "worktree_suffix": true
}
```

## Matching
- Rules are an **ordered list**; the **first** rule whose every present clause
  matches wins. Reorder them in the file.
- Clauses: `path` (glob over the working directory, `~` expanded; a pattern
  through a symlinked folder also matches the resolved directory), `command`
  (the name of the program in the foreground, e.g. `vim`, `cargo`; empty while
  the shell waits for input), `agent` (exact name), `host` (an ssh tab's host),
  `file`.
- Glob: `*` matches within a path segment, `**` crosses `/`, `?` matches one
  character. Matching is case-sensitive.
- An empty `match` (`{}`) matches anything — use it as the catch-all.

## Title template
Variables: `{alias} {cwd} {folder} {user} {host} {agent} {branch} {command}
{title} {osc_title} {shell} {index} {file}`. Unknown variables render empty.

## Projects
`projects` maps a path to an alias (longest prefix wins) and is used as the
`{alias}` fallback when no rule sets one. With `worktree_suffix: true`, a
`.worktrees/<name>` checkout gets `<alias>-<name>`.

## Icons
`icon` takes a built-in `name`, an `emoji`, and/or a `color` (`#rrggbb`). A
name with no match falls back to the emoji, then to a colored dot. Built-in
names:

`folder`, `file`, `file-text`, `terminal`, `code`, `git-branch`, `git-commit`,
`github`, `globe`, `bug`, `flame`, `cpu`, `cloud`, `database`, `package`,
`box`, `coffee`, `heart`, `star`, `flag`, `zap`, `lock`, `search`, `settings`,
`user`, `home`, `bell`, `layers`, `claude`.

## Fallback
With no matching rule and no project, the tab shows the program's OSC title,
then the working-directory folder name. A missing or malformed
`views.json` degrades to that same fallback rather than failing startup.
