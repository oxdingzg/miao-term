# mtty product requirements and roadmap

[简体中文](PRODUCT.zh-CN.md)

> Baseline: `v0.0.5` plus commit `689cbce` (reliability fixes), 2026-10-01.
> This is the single source for feature requirements. The README describes what
> works today; this document also records what comes next, in which order, and
> what counts as done.

## 1. Positioning

**mtty** is a cross-platform (macOS / Linux / Windows) **terminal front door**:
everyday development, AI coding agents and remote operations all start from one
window.

| Source | What we take | What we leave |
|---|---|---|
| [Otty](https://otty.sh/) | A modern local terminal: tabs/splits, command palette, file reader, agent badges, Composer, session recovery | — |
| [Termius](https://termius.com/) | Remote operations: host library and groups, SSH keys/identities, SFTP, port forwarding and jump hosts, snippets | Accounts, mandatory cloud sync, paywalled basics |
| [mtty](https://mtty.dev/) | Parallel agents: one git worktree per task, one place to watch them, diff review before merge | An agent-only IDE shape |
| miao | Native integration: state reporting without hooks, session resume, two-way control over MTP | — |

In one line: **the local terminal must be fast and dependable, agents must be
visible and steerable, and remote hosts should feel as easy as local folders.**

### 1.1 Naming

- Official product name: **mtty**. The repository and embeddable engine stay
  **miao-term**.
- Binaries, bundle, config directory, socket, environment variables and URL
  scheme are still `miaotty`. The rename is its own task in milestone M1 (see
  §4), with migration — never a drive-by change inside feature work.

### 1.2 Users and core scenarios

1. **Developers' local terminal**: many projects, tabs and splits; CJK paths and
   output display and search correctly.
2. **AI coding agent users**: run codex / claude / opencode / miao side by side,
   see who is busy, who is waiting for me, who failed, and queue prompts for them.
3. **Ops / full-stack**: manage dozens of hosts — log in over SSH, move files,
   forward ports, run common commands in bulk — with credentials kept on this
   machine.

## 2. Interaction design principles

These are part of the acceptance criteria; feature reviews check each one.

1. **Terminal first**: the terminal area is always the main surface. Panels and
   dialogs must not steal terminal input, and no key may act in two places at
   once (with a text field focused, ⌘C/⌘V/⌘A act on the field).
2. **Keyboard first, one entry point**: every feature is reachable from the
   command palette (`⌘K`); frequent ones have shortcuts that agree across the
   menu, the palette and the docs.
3. **Failures are visible**: any user-initiated action that fails (save, open,
   connect, write config) shows why in the UI — never only on stderr, never
   pretending to succeed.
4. **No data loss**: closing unsaved changes asks first; a failed save stays
   modified; writing config never overwrites a file that fails to parse.
5. **The user's config is theirs**: no edits to shell dotfiles, agent config or
   `~/.ssh/config`; when wiring is needed, show a snippet to paste.
6. **Zero remote install**: SSH features require nothing installed remotely.
7. **Credentials stay local**: prefer ssh-agent and the system keychain; never
   store plaintext passwords; any cloud sync is end-to-end encrypted and fully
   optional.
8. **The UI thread never blocks**: network, external processes and large
   directory reads run in the background with visible, cancellable progress.
9. **Bilingual**: UI and docs in English and Simplified Chinese.

## 3. Feature baseline (current version)

Status meanings:

- **Works**: reachable by the user, complete, with automated tests or
  real-window smoke evidence.
- **Partial**: usable with known gaps (listed in notes and scheduled in M0).
- **Not available**: previously claimed or commonly expected, but absent from
  the native app; removed from the README and scheduled on the roadmap.

Evidence: `cargo test --workspace` (183 tests), `cargo clippy -D warnings`,
`scripts/smoke-hosts.py` (real window + real shell + GPU capture), and a
feature-by-feature review of the native host code for this baseline.

### 3.1 Terminal core

| Feature | Status | Notes |
|---|---|---|
| PTY + VT parsing (alacritty_terminal), GPU glyph grid | Works | See PERFORMANCE for the gate |
| Scrollback, selection, copy/paste, CJK wide characters and font fallback | Works | Wide-character copy has regressions |
| Find `⌘F` (highlights + count) | Works | Fixed now: CJK queries never matched; highlights were misplaced on CJK lines |
| Key encoding: Ctrl/Alt/modified navigation, F1–F12, Insert | Works | Fixed now: F1–F12 and Insert were not sent at all |
| kitty keyboard protocol | Partial | Disambiguate level only |
| IME inline preedit | Partial | Candidate window is not placed at the cursor (no `set_ime_cursor_area`) |
| Inline images: Sixel / Kitty / iTerm2 | Works | No Kitty z-index; session restore keeps no images |
| Mouse reporting (SGR / X10) | Works | |

### 3.2 Window and workspace

| Feature | Status | Notes |
|---|---|---|
| Tab bar: drag reorder, `+`, `×`, row menu (rename/prefix/mark/group/duplicate/move/close others/close below) | Works | Duplicate drops mark and group; Close Others/Below are not reopenable |
| Split tree `⌘D` / `⇧⌘D`, divider drag, per-pane close | Works | New splits/tabs do not inherit the current directory |
| Quick Terminal `⌘⇧T`, reopen closed tab `⌘⇧Z` | Works | Reopen restores the directory only |
| Global Quick Terminal hotkey | Works | Fixed now: one press did two things (switch to Quick, then hide the window) |
| Command palette `⌘K` | Works | |
| Open Quickly `⌘⇧O` | Partial | Tabs, directories and recent files only; no files, agents or content hits |
| Details panel: Info / Agent / Outline / Git / Files / Ports / Queue | Partial | Ports sees the shell process only; non-git dirs show "clean"; polls every 2 s even when hidden |
| Session restore (layout, directories, titles, groups) | Works | Fixed now: `⌘Q` skipped the save. Restores layout, not running processes |
| Recipes save/open | Partial | Write failures are silent; names are not validated as file names |
| Picture-in-picture, hints, read-only mode | Works | Read-only does not block MTP `pane.send/run` |
| View rules | Partial | Alias and title only; icons, badges and command/host/file matching are not applied; no hot reload |
| macOS menu bar | Works | Fixed now: menu Copy/Paste/Select All acted on the terminal while a text field had focus |
| Settings window `⌘,` | Works | Fixed now: changes were never saved, the theme name was forced to Nord, and a broken config fell back silently |

### 3.3 Reader and editor

| Feature | Status | Notes |
|---|---|---|
| Read-only view, edit, line numbers, syntax colouring, minimal vim mode | Works | Fixed now: failed saves were swallowed and shown as saved; closing did not confirm unsaved changes |
| Markdown + Mermaid subset | Partial | Relative local images do not render; external `mermaid-command` runs on the UI thread |
| Jump-to-line highlight, Open Externally, Edit in Tab | Not available | The `editor` setting is not read |

### 3.4 Agents

| Feature | Status | Notes |
|---|---|---|
| Detect claude / codex / opencode / miao, install the state hook, show the wiring snippet | Works | The snippet has no Copy button |
| Badges, needs-attention marks, system notifications, sleep guard | Works | Fixed now: the sleep inhibitor could outlive the app |
| Composer `⌘⇧E`, prompt queue (sent by hand) | Works | |
| Automatic queue delivery when an agent turns idle | Not available | The queue has no target-pane model |
| Launch an agent from the UI | Not available | |
| Resume (session id), quota display | Works | Depends on fields the hook reports |

### 3.5 Remote and operations

| Feature | Status | Notes |
|---|---|---|
| New SSH session (`~/.ssh/config`, ControlMaster reuse, zero-install terminfo) | Partial | `ssh -G` runs on the UI thread; restored SSH tabs come back as local shells |
| Remote file view/edit over ssh | Partial | Host typed by hand; read/write block the UI thread (failures are now shown) |
| Host library, groups, key management, SFTP, FTP, port forwarding, snippets | Not available | See M3 |

### 3.6 System integration and automation

| Feature | Status | Notes |
|---|---|---|
| Config file + ghostty / alacritty import | Works | Fixed now: a `config.toml` with syntax errors is reported with the reason |
| zsh shell integration (OSC 7, history) | Works | bash / fish / PowerShell not covered |
| URL schemes and single-instance forwarding | Partial | Command-line URLs work; macOS URL events from a browser/Finder are not handled |
| MTP control plane and `miaotty-cli` | Works | Covered by the real-window smoke |
| Version check | Works | Check only, no install; `update-pubkey` unused |
| English / Chinese UI | Partial | Some details-panel rows are still English |

## 4. Roadmap

Order is priority. **A milestone's acceptance must pass before the next starts.**

### M0 Stabilize (v0.0.6): turn "Partial" into "Works"

Goal: every "Partial" row in §3 is fixed or its limit is stated in the README.

- [ ] Nothing blocks the UI thread: `ssh -G`, remote read/write,
      `mermaid-command` and Files-panel directory reads move to the background
      with a loading state.
- [ ] The details panel polls only while visible; Ports includes child
      processes; Git says "not a git repository" outside one.
- [ ] macOS URL events (`miaotty://`, `ssh://` opened from a browser/Finder).
- [ ] Restored SSH tabs reconnect, or clearly show "disconnected — press Enter
      to reconnect".
- [ ] New splits/tabs inherit the pane's directory; Duplicate keeps mark and
      group; Close Others/Below are reopenable.
- [ ] Read-only mode also blocks input from MTP and URLs.
- [ ] The IME candidate window follows the cursor.
- [ ] Recipe names validated, read/write failures shown; GPU init failure at
      startup shows an error instead of panicking.
- [ ] View rules apply icons and badges, match command/host, and hot-reload.
- [ ] Automatable UI acceptance (the P0 from UI-AUDIT): semantic control
      targets plus event replay for rename/cancel, tab reorder, divider drag,
      editor save, clipboard focus and IME.
- [ ] Real-desktop acceptance for menu shortcuts vs text-field focus (fixed at
      code level now; no OS-level key replay yet).

**Acceptance**: fmt / clippy / tests / perf gate / desktop smoke all pass; no
"Partial" left in §3 (or its limit is in the README); one Windows and one Linux
desktop pass following WINDOWS-DEV and RELEASE.

### M1 One brand: mtty

- [ ] Binaries `mtty` / `mtty-cli`, keeping `miaotty` / `miaotty-cli` aliases
      for at least two releases.
- [ ] Config directory `~/.config/mtty`, migrated from `~/.config/miaotty` on
      first start (copy, never delete).
- [ ] `MTTY_*` environment variables, still reading `MIAOTTY_*`; socket name and
      a new `mtty://` scheme, keeping `miaotty://`.
- [ ] Packages, bundle name, icon and docs updated together; a bundle ID change
      needs its own ADR (it affects OS permissions and updates).

**Acceptance**: existing config, sessions, recipes and hook scripts keep
working; the old CLI drives the new app; migration has automated tests.

### M2 Agent workbench (from mtty / Otty)

- [ ] Launch an agent (codex / claude / opencode / miao) from the UI, with an
      optional working directory.
- [ ] Queue items target a pane and are delivered when its agent turns idle;
      the queue persists.
- [ ] OSC 133 command boundaries: select the last command's output and send it
      to the Composer / an agent in one step.
- [ ] Task = git worktree + branch + agent pane: create, list, review the diff,
      merge or discard.
- [ ] Clicking a notification jumps to its pane; unread/finished marks on
      background tabs.
- [ ] Deeper miao integration: event subscription over MTP, session resume,
      opening files miao produces in mtty.
- [ ] codex wiring: a ready-to-use hook snippet and a verification script, with
      an end-to-end state-reporting test.

**Acceptance**: three agents in three worktrees run in parallel; state,
notifications, queue and diff review are covered by a real-window smoke.

### M3 Remote operations (from Termius)

- [ ] **Host library**: `hosts.toml` (name, address, user, port, group, tags,
      jump host), one-step import from `~/.ssh/config`; a sidebar host list,
      double-click to connect, hosts searchable from `⌘K`.
- [ ] **Identities and keys**: ssh-agent and the system keychain; optional key
      generation; no plaintext passwords.
- [ ] **known_hosts**: clear confirmation on first connect and on fingerprint
      change.
- [ ] **SFTP**: two-pane browser (local/remote), upload/download, drag and
      drop, progress, resume, permissions and rename.
- [ ] **FTP / FTPS** in the same browser (plain FTP is flagged as unencrypted).
- [ ] **Port forwarding**: local / remote / dynamic (SOCKS) rules stored with
      the host, started and stopped individually, with visible state.
- [ ] **Snippets**: a command library to run in the current pane or on several
      selected hosts.
- [ ] **Broadcast input** to several panes.
- [ ] **Persistent sessions**: optional tmux / mosh to keep remote work alive
      and reconnect.
- [ ] Remote view/edit uses the pane's own connection; no typing the host.

**Acceptance**: an automated smoke against a test host (a container is fine)
covers host import, login, SFTP transfer, port forwarding and running a snippet;
credentials never appear in logs, session files or MTP responses.

### M4 Cross-platform delivery

- [ ] Real Windows / Linux desktop acceptance (IME, hotkey, drag and drop, menus).
- [ ] Apple notarization and Windows MSI signing (credentials; run by the owner).
- [ ] Auto-update: download, signature check (`update-pubkey`), replace.
- [ ] Shell integration for bash / fish / PowerShell.
- [ ] Optional end-to-end-encrypted sync (hosts, snippets, settings), off by
      default.

## 5. Non-goals

- No accounts or mandatory sign-in; every cloud feature is optional.
- No built-in model calls; mtty orchestrates external agents, it does not
  replace them.
- No edits to the user's shell, agent or ssh config files.
- No plaintext passwords stored by the UI.

## 6. Keeping this current

- When a feature's status changes, update the §3 tables and both READMEs in the
  same commit.
- Tick roadmap items when done and cite the evidence (test names, smoke output
  or a screenshot note).
- Changing a locked design decision needs a new ADR under `docs/decisions/`.
