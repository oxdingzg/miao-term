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
- Binaries `mtty` / `mtty-cli`, bundle `mtty.app` (`dev.mtty.terminal`),
  config directory `~/.config/mtty`, environment `MTTY_*`, URL scheme
  `mtty://`. Up to v0.0.5 the name was `miaotty`; compatibility rules are in
  [ADR 0032](decisions/0032-rename-mtty.md).

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
| MTP control plane and `mtty-cli` | Works | Covered by the real-window smoke |
| Version check | Works | Check only, no install; `update-pubkey` unused |
| English / Chinese UI | Partial | Some details-panel rows are still English |

## 4. Roadmap and execution batches

Order: **M1 → M0 → M2 → M3 → M4**. M1 (the rename) moved first by the
2026-10-01 decision: the later it happens, the more user state and external
integrations need migrating. Each batch is a unit that is committed and
verified on its own:

- Every batch must pass `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`, `scripts/smoke-hosts.py` (when UI or packaging is
  touched), and the acceptance listed for it.
- One (or a few) commits per batch, pushed right away; tick the box here and
  cite the evidence.
- Steps that need credentials, real hardware or the owner's confirmation
  (signing, notarization, publishing, production hosts) are marked **[owner]**
  and are not run automatically inside a batch.

### M1 One brand: mtty (design: [ADR 0032](decisions/0032-rename-mtty.md))

- [x] **B1.1 Runtime rename and compatibility layer** — `ee12822`: unit tests for migration, environment, socket link and schemes; the smoke covers the config copy, both pane environment sets, a pre-rename hook and the former socket path.
  - `miao-term-config`: `config_dir()`, `migrate_legacy_config()` (copy once,
    never overwrite, never delete the old directory), `env()` (`MTTY_*` first,
    then `MIAOTTY_*`).
  - Every path goes through `config_dir()`: config, views, hooks, launch inbox,
    session/queue/window/recipes.
  - MTP: default socket `mtty.sock` plus a `miaotty.sock` compatibility link;
    Windows pipe `mtty`; token/capability variables read under both names.
  - Pane environment exports `MTTY_*` and `MIAOTTY_*`; `MTTY_CLI`/`MIAOTTY_CLI`
    hold the absolute path of the sibling CLI.
  - Both `mtty://` and `miaotty://` are accepted; hook scripts, the shell shim,
    hotkey snippets, ControlPath and UI text say mtty.
  - Acceptance: unit tests for migration, environment and schemes; an old hook
    script reports state from a new pane (smoke).
- [x] **B1.2 Build and packaging** — `ee12822`: `package-macos.sh` + `check-macos-bundle.py` + `smoke-hosts.py --bundle` pass; `test-release-manifest.py` passes 9 tests; the release-workflow rehearsal (run 36830274914) passes on all four runners, including MSI install/uninstall with both schemes and the deb aliases.
  - Dirs and packages: `miaotty-app` → `mtty-app` (binary `mtty`),
    `miaotty-cli` → `mtty-cli`.
  - macOS: `mtty.app`, `dev.mtty.terminal`, URL scheme registration; the
    installer archives and removes the old `miaotty.app`.
  - Linux: deb package `mtty` (Replaces/Conflicts/Provides `miaotty`),
    compatibility symlinks, `.desktop`, AppImage.
  - Windows: MSI product and folder `mtty`, same UpgradeCode, `mtty://` and
    `miaotty://` registered.
  - `release.yml`, `ci.yml`, the update-manifest script and its tests,
    smoke/profiling scripts, `windows-verify.ps1`.
  - Acceptance: `scripts/package-macos.sh` + `check-macos-bundle.py` +
    `smoke-hosts.py --bundle` pass; `test-release-manifest.py` passes; a manual
    release-workflow rehearsal **[owner]**.
- [ ] **B1.3 Docs and website**
  - READMEs, `docs/*`, AGENTS.md and the example config say mtty; APP-IDENTITY
    becomes the migration guide; historical ADRs stay as written.
  - The mtty.dev site's install, config and CLI examples say mtty (separate
    repository, separate commit).
  - Acceptance: `check-privacy.sh` passes; `miaotty` remains only in
    compatibility notes and historical records.

### M0 Stabilize: turn "Partial" into "Works"

- [x] **B0.1 Nothing blocks the UI thread** (also fixed: SSH sessions connect with the typed alias; options under `Host <alias>` were not applied before): `ssh -G`, remote read/write,
  `mermaid-command` and Files directory reads become background tasks with a
  loading state; the details panel polls only while visible; Ports includes
  child processes; non-git directories are reported as such. Acceptance:
  background-task unit tests; no frame stalls in the smoke while opening a
  remote file or a large directory (timed in the log).
- [x] **B0.2 Workspace correctness** (read-only makes MTP `pane.send/run` fail with `read_only`; a URL that opens a new tab to run a command is an explicit user action and is not blocked; a new split's cwd is unit-tested, as MTP has no split method): new splits/tabs inherit the directory;
  Duplicate keeps mark and group; Close Others/Below are reopenable; read-only
  blocks MTP/URL input; recipe names validated and read/write failures shown;
  GPU init failure shows an error instead of panicking. Acceptance: unit tests
  for the pure parts plus a smoke check of a new split's cwd.
- [x] **B0.3 System integration** (`0417c1b` and after; `open -a dist/mtty.app mtty://quick` verified on a cold start and on the running app; also: duplicating an ssh tab connects again; IME candidate placement awaits a desktop check): macOS URL Apple Events (links opened from a
  browser/Finder); restored SSH tabs show "disconnected — press Enter to
  reconnect" and reconnect; the IME candidate window follows the cursor.
  Acceptance: `open mtty://quick` works on the installed app; reconnect has unit
  tests; IME needs a manual desktop check **[owner]**.
- [x] **B0.4 View rules and entry points** (`d52d898` and after; icons, badges, command matching and hot reload checked in real-window captures; also fixed: Rename Tab had no effect, deep-link Focus did not select the pane, symlinked folders did not match): rule icons/badges applied,
  command/host matching, `views.json` hot reload; Open Quickly lists files and
  agents; the remaining English strings in the details panel are translated.
  Acceptance: rule-engine unit tests and a screenshot check.
- [x] **B0.5 Automatable UI acceptance** (the tab bar exports each tab's rect as a test target; replay tests cover tab drag-reorder, divider drag, rename commit on Enter / cancel on Escape and a paste landing in a focused field; the replay found that Enter did nothing in 8 input fields, now fixed; editor save is unit-tested): semantic control targets plus
  windowless event replay for rename/cancel, tab reorder, divider drag, editor
  save and clipboard focus. Acceptance: the new replay tests run in CI.

### M2 Agent workbench (from mtty / Otty)

- [x] **B2.1 Launch and wiring** (the codex snippet follows its `hooks.json` format and needs `[features] hooks = true`; an end-to-end hook-script test covers `--stdin` session parsing, the pane filter and never blocking; the user's real agent configs were not touched): "Launch agent" in settings and the palette
  (codex/claude/opencode/miao, optional directory); a Copy button for wiring
  snippets; a codex hook snippet and verification script. Acceptance: launch
  command unit tests; a fake agent reports state in the smoke.
- [x] **B2.2 Queue** (also fixed: Send now left the prompt queued, so it was delivered again; per-frame state sampling missed a quick processing -> idle, now the control plane records transitions in order; the smoke with back-to-back states passed 8 runs in a row): items target a pane and are delivered when its agent turns
  idle; persisted in `queue.json` (old format still read). Acceptance: unit
  tests for transitions and delivery, including no double delivery on repeated
  events.
- [x] **B2.3 Command boundaries** (output is captured from the byte stream between C and D, so the scrollback cap shifting line numbers does not matter; real zsh end-to-end test; the smoke reads it back over MTP `pane.output`): OSC 133 (A/B/C/D) parsing, emitted by the zsh
  shim; "copy/send the last command's output". Acceptance: parser unit tests;
  the smoke runs a command and retrieves its output.
- [x] **B2.4 Worktree tasks** (4 integration tests on temporary repositories; the tasks window and the new-task dialog checked in real-window captures via `MTTY_QA_COMMAND`): a task = `git worktree add` + branch + agent pane;
  task list, diff view, merge or discard (destructive steps confirm).
  Acceptance: integration tests against a temporary repository.
- [x] **B2.5 Attention and miao** (osascript notifications cannot report clicks, so activating mtty within two minutes of a notification jumps to its pane; the background `!`/✓/• marks were checked in a real-window capture; the miao plugin reads `MTTY_*`, miao commit `f596c592a`): clicking a notification jumps to its pane;
  unread/finished marks on background tabs; the miao plugin reads `MTTY_*`
  (miao repository, separate commit). Acceptance: transition unit tests; miao
  plugin tests.

### M3 Remote operations (from Termius)

- [x] **B3.1 Host library** (end to end against a real Linux host: an imported alias connected through its `~/.ssh/config` ProxyJump, opened via `mtty://host/<name>`, a remote command checked; sidebar HOSTS checked in a capture): `hosts.toml` (name, address, user, port, group,
  tags, jump host), import from `~/.ssh/config`, sidebar host list, palette
  search, double-click to connect. Acceptance: parser/import unit tests; a smoke
  against a local sshd or container **[needs a test host]**.
- [x] **B3.2 Safe connections** (an interactive ssh asks about host keys itself; the host library checks in the background: known / unknown (fingerprints, trust after comparing) / changed (refused); "known" checked against a real host; key generation and ssh-copy-id run in a terminal tab, so mtty never handles a passphrase): known_hosts confirmation on first connect and
  fingerprint change; ssh-agent status and key generation; no plaintext
  passwords.
- [x] **B3.3 Port forwarding**: L/R/D rules stored with the host; each runs as
  an `ssh -N` owned by mtty (`ExitOnForwardFailure`), with visible state, ending
  with mtty (not the shared ControlMaster, whose 60 s ControlPersist would drop
  forwards). End to end against a real host: the remote sshd banner read
  through the local port, the port closed after Stop.
- [x] **B3.4 SFTP** (OpenSSH sftp in batch mode, keeping ssh config, jump hosts, the agent and ControlMaster; real round-trip test; checked on a real Linux Wayland desktop; download progress is read off the local file, uploads show a busy state without a percentage): two-pane browser, upload/download, drag and drop, progress,
  rename/permissions; remote editing reuses the pane's connection.
- [x] **B3.5 Snippets and broadcast** (on several hosts a snippet runs as `ssh -t host 'command'` in a tab each, quoting checked by letting sh split it; broadcast goes through the keyboard path, which MTP cannot drive, so it awaits a desktop check): a command library run in the current pane
  or on several hosts; broadcast input to several panes.
- [ ] **B3.6 FTP/FTPS and persistent sessions**: FTP/FTPS in the same browser
  (plaintext flagged); optional tmux/mosh reconnect.
  Acceptance (M3 overall): an automated smoke against a test host; credentials
  never reach logs, session files or MTP responses.

### M4 Cross-platform delivery

- [ ] **B4.1** Real Windows/Linux desktop acceptance (IME, hotkey, drag and
  drop, menus) **[owner + hardware]**.
- [ ] **B4.2** Apple notarization and Windows MSI signing **[owner +
  credentials]**.
- [ ] **B4.3** Auto-update: download, `update-pubkey` signature check, replace.
- [ ] **B4.4** Shell integration for bash / fish / PowerShell.
- [ ] **B4.5** Optional end-to-end-encrypted sync, off by default.

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
