# ADR 0008 — Open Quickly / command palette

Status: accepted.

## Context

Every navigable thing in miaotty — windows, tabs, panes, folders, agents — and
every verb should be reachable from one keyboard-first surface. The view rule
engine (ADR 0007) already gives each pane a title and icon, so a palette can
present tabs and agents consistently.

## Decision

One overlay, opened with **⌘K** (Escape closes), that fuzzy-filters a single
flat list of entries and runs the chosen one.

- **Entry kinds**: `tab`, `agent`, `command`. Each entry carries a label, an
  optional view-engine icon, a kind tag (shown right-aligned) and an action.
- **Ranking** (`palette_score`): case-insensitive; an empty query matches all;
  a substring hit ranks by match position; otherwise a subsequence hit ranks
  after every substring hit; no subsequence means no match.
- **Verbs** (`command`): New Tab, Split Right, Split Down, Close Tab, Toggle
  Details, Find, Settings, Next/Previous Tab. (`jump`, folders and open-file
  land with the editor in U5.)
- **Interaction**: ↑/↓ move the selection (wrapping), Enter runs it, click or
  hover selects. While the palette is open, window shortcuts and PTY input are
  suppressed so keystrokes go to the query.
- **Ownership**: the palette is app state (`Option<Palette>`), not a plugin; it
  reads tabs/agents directly and dispatches through the same methods the
  shortcuts use.

## Consequences

- Tabs and agents appear with their rule-engine title/icon for free.
- New entries are added by extending `palette_entries`; new verbs by extending
  `Verb` — both are small, local changes.
- Folder/open-file entries and frecency (the `jump` ranking) are deferred to
  U5, where the file model lands.
