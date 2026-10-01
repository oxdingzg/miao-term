# UI and basic-function audit — 2026-10-01

> Historical audit before application consolidation. Current identity and
> the single-app packaged smoke are documented in [APP-IDENTITY.md](APP-IDENTITY.md).
> The current smoke script targets mtty only; commands below use the
> current package names (`mtty-app`, `mtty-cli`; formerly `miaotty-*`).

[简体中文](UI-AUDIT.zh-CN.md)

## Reproduce

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo test --release -p miao-term-core -p mtty-app -- --ignored
cargo build --release -p miao-term-widget -p mtty-app -p mtty-cli
python3 scripts/smoke-hosts.py
```

The last command requires a macOS desktop and Swift. It starts both release
hosts sequentially with isolated HOME/config/data/runtime directories, runs a
real `/bin/sh` on each PTY, exercises the CLI, and captures a fresh GPU image.
It activates each test window without injecting mouse or keyboard events.
Screenshots, logs, fixtures and a JSON report stay in a temporary directory;
keep them out of the public repository. Do not run two capture smokes together:
the hosts' built-in screenshot destinations are fixed.

For stable-machine baseline comparison, run performance tests **separately from
builds, GPU tests and desktop smoke**:

```sh
PERF_ENFORCE=1 cargo test --release -p miao-term-core -p mtty-app -- --ignored --nocapture --test-threads=1
```

## Fixes and regression coverage

- Native pointer routing now respects egui's consumed events and pane bounds.
  Overlay/chrome presses cannot start terminal selection, scrolling or mouse
  reports. A terminal-started gesture retains its release outside the pane.
  The headless regression replays hover, press, move and release on a movable
  Rename Tab window and checks both window movement and routing isolation.
- Clipboard copy/paste from native text widgets is no longer also sent to the
  PTY. The eframe host blocks terminal input while overlays own interaction;
  terminal wheel scrolling requires pointer hover on the pane.
- Native split-pane clicks and file drops focus the target pane; wheel scrolling
  addresses the hovered pane. Read-only mode also blocks paste and application
  mouse reporting, rather than just typed keys. These guards were code-reviewed;
  full OS-level acceptance coverage remains follow-up work.
- The initial shell directory is available immediately. On macOS/Linux, shells
  without OSC 7 get a bounded, direct process-cwd fallback (no subprocess).
  Once OSC 7 is received it remains authoritative, including remote contexts.
  URI-encoded spaces/Unicode are decoded; invalid escapes/NUL are rejected.
- Dark ANSI black/bright-black/grayscale foregrounds are lifted for readability.
  Explicit RGB and background colors retain their values.
- Selection copying honors its inclusive end on both single and multiple lines;
  wide-character spacer cells are omitted. Word selection recognizes CJK
  spacer cells. Both plain and ANSI copy have regressions.
- Reverse-video glyphs use the cell's actual background color.
- Native background waiting no longer overwrites the screenshot deadline; idle
  cwd checks advance without requiring new shell output.
- Long paths no longer share the editor toolbar row or overlap status chips.

The real-window smoke also caught a nested egui context-lock deadlock introduced
while adding the eframe input guard. The context query was moved outside the
`input()` closure, and both hosts subsequently completed the smoke. This is why
unit-test success alone is not the completion criterion.

## What was verified

| Area | Evidence | Coverage limit |
|---|---|---|
| VT, scrollback, input encoding, graphics, configuration | Workspace tests and offscreen GPU smoke | Does not certify every VT sequence or text style |
| Split/resize/close, tab/session decorations | Layout lifecycle and existing session/menu tests | No exhaustive OS drag/reorder sequence |
| Selection and copying | Exact substring, multi-line, backwards, CJK and ANSI regressions | OS clipboard/IME interaction still needs desktop coverage |
| Overlay pointer ownership | Deterministic egui window-drag replay | Not a full OS-level mouse replay suite |
| Directory tracking | Live plain-shell `cd` with spaces/CJK, plus OSC URI tests | Windows fallback and remote file browsing not certified |
| Both release hosts | Startup/health, pane discovery/focus, real command execution, PTY size, Unicode file read/write, history, agent state, view/edit dispatch | Editor save via UI, undo/redo, SSH/network and external agent executables not exercised |
| Actual rendering | Fresh screenshots from both hosts; editor fixture and directory visually inspected; native Files panel inspected separately | Screenshot smoke asserts image freshness/content, not pixel-perfect UI correctness |
| Performance | Four absolute release budgets passed; serialized enforced baseline run passed | No key-to-glyph latency, power or cold-start guarantee |

At this audit stage the workspace run passed **179 tests**, with four performance
tests excluded from the ordinary run and executed separately. Serialized release
final measurements were approximately 120.6 MB/s VT parsing, 0.0074 ms screen snapshot,
0.133 ms row construction, and 1.429 ms ranking 10k palette entries. Concurrent
build/GPU activity initially exceeded two baseline tolerances; serialized
remeasurement passed without changing budgets or applying a scale factor.

Windows interactive IME/ConPTY, Linux real-window behavior, installation/update
flows, genuine agent integrations, large-directory responsiveness and long-run
input/rendering latency remain separate acceptance work. Use
[WINDOWS-DEV.md](WINDOWS-DEV.md) and [RELEASE.md](RELEASE.md) for their procedures.

## Otty comparison and priorities

Reference: **Otty by appmakes.io**, using its [official site](https://otty.sh/)
and [official documentation](https://docs.otty.sh/agents/agents-overview), accessed
2026-10-01. This is a documentation/code comparison, not a measured head-to-head
benchmark. The similarly named `otty-shell/otty` project is a different product.

Existing overlap includes tabs/splits, command palette, file viewer/editor,
recipes, agent badges, composer/queue, hint/read-only modes, and inline images.
The most valuable next work is:

1. **P0 — interaction acceptance infrastructure.** Add semantic UI targets and
   observable interaction state, then cover rename/cancel, tab reorder, divider
   drag, editor save, clipboard focus and IME. Otty exposes accessibility and
   automation references; adopting that testability is more valuable than
   adding another panel before existing interactions have acceptance coverage.
2. **P1 — command/output context.** Otty's
   [Send to Chat](https://docs.otty.sh/agents/send-to-chat) and command-aware
   selection suggest OSC 133 command boundaries, last-output extraction and
   sending selected context to Composer. Current history is not a substitute
   for precise output boundaries.
3. **P1 — completion and unread state.** Add generic job completion/progress and
   background unread indicators alongside existing agent badges, with state
   transition and duplicate-notification tests.
4. **P1 — durable sessions.** Otty documents
   [session recovery](https://docs.otty.sh/workflows/session-recovery) and tmux
   reattachment. Current layout/cwd restore recreates shells; preserving live
   jobs requires a multiplexer/session strategy and disconnect/restart tests.
5. **P2 — configurable bindings and pane drag/snap.** Existing fixed shortcuts
   and divider resize are a foundation; test conflicts and ownership before
   adding drag-to-reparent layouts.
6. **P2 — inline suggestions and richer Unicode/styles.** Otty documents
   [autocomplete](https://docs.otty.sh/terminal-features/autocomplete) and
   [Unicode/text styles](https://docs.otty.sh/terminal-features/unicode-and-text-styles).
   Grapheme clusters, bold/italic/underline/strike-through and smooth scrolling
   need explicit visual/latency tests; a VT parser supporting a flag does not
   prove the renderer displays it correctly.

These are prioritized follow-ups, not claims that these features were added or
that Otty's implementation was tested locally during this audit.
