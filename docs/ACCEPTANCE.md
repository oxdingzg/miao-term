# Acceptance follow-up — 2026-10-04

[简体中文](ACCEPTANCE.zh-CN.md)

This records the source changes following v0.1.3. Earlier UI audits and release
rehearsals remain historical. Host identities, private addresses, local paths,
credentials, screenshots and raw logs stay outside the public repository.

## Changes

- Windows ZIP updates stage and validate all three executables, then replace
  them with backups. A failure restores the original executables and restarts
  the old app. MSI exit codes are checked. See [ADR 0043](decisions/0043-windows-update-transaction.md).
- Idle serial reads retry timeouts and interruptions instead of disconnecting.
- Agent proposals show deleted pre-image rows in red with a strike-through;
  these rows scroll but never enter copied text or the saved document. Whole-file
  proposals retain unchanged context. Acceptance and rejection remain undoable.
- ACP adds authentication, loading sessions, terminal lifecycle, valid permission
  option IDs and streaming UTF-8 handling. Reading a file prefers the editor's
  unsaved buffer. Writing requires review and saving before returning success;
  rejection leaves the file unchanged. Commands need permission, output and
  process counts are bounded, and closing the client cleans up its process tree.
- Clipboard image forwarding covers Windows and X11, with PNG encoding and
  text-paste cleanup; Wayland uses the host's existing data device.
- README, product status and release documentation no longer list completed
  editor and transport work as future milestones.

## Evidence

| Area | Evidence | Limits |
|---|---|---|
| macOS | Release build, bundled-app real-window smoke, workspace tests, clippy and 14 release performance tests | Ad-hoc bundle signing does not provide Apple notarization |
| Linux AppImage | Ubuntu 24.04, native X11 window: signed download, verification, replacement, relaunch and retained PTY host session | No health handshake to detect every failure after launching a replacement |
| Linux failed updates | Wrong SHA-256 or minisign signature refused and download removed; missing download left the old package intact and restarted it | Uses a private test manifest and temporary signing key |
| Linux remote/transport | Real mosh SSH bootstrap and UDP session; TCP and Telnet bidirectional bytes, Telnet negotiation; serial PTY idle and round-trip | Serial PTY coverage does not certify every physical adapter |
| Broadcast | Real keyboard events sent to two Linux panes; matching command output and exit codes | X11 input injection |
| Windows helper | Actual PowerShell execution: good ZIP, corrupt ZIP and a locked second executable; exact original hashes after rollback, automatic relaunch, Unicode/space/quote paths | MSI desktop results are tracked separately |
| Windows IME | Current v0.1.3 desktop build: Microsoft Pinyin `nihao` + Space commits `你好`, read back through the PTY | Not exhaustive across all input methods |
| Clipboard | Windows 3×2 PNG dimensions/pixels; Linux X11 2×2 RGBA including transparency, then text paste and old-image removal | Wayland results are recorded separately |
| ACP provider | Codex ACP adapter 2.1.1: initialize, new session, real prompt, output file check and session/load | This adapter executes its own tools; it does not exercise client file/terminal callbacks |
| ACP client protocol | Official TypeScript SDK 1.6.0 peer: file read/write, permission option ID and all five terminal methods | Tests the protocol bridge separately from provider login |
| Claude ACP | Adapter 0.85.1 initializes and creates a session | A real prompt returns `Authentication required`; this is not complete provider acceptance |

## Recovery verification

The follow-up check found that shell integration could finish its assertions but
hang while destroying the terminal. Child termination and reaping now run off
the UI thread, retaining the PTY master until the child is reaped. Unix native
children are forcefully terminated even if they ignore SIGHUP. Windows tree
termination also runs off-thread.

Verified on the current source snapshot:

- `cargo fmt --all --check`, `git diff --check` and the privacy scan passed.
- Remote macOS `cargo clippy --workspace --all-targets` passed.
- Remote macOS `cargo test --workspace`: 531 passed, zero failed, 19 ignored
  platform/performance checks. Both installed Bash versions passed without
  excluding the previously hanging integration test.
- The new Unix regression verified responsive pane teardown and eventual reaping
  of a shell that ignores SIGHUP.
- The release performance gate passed all 14 tests.
- Native Windows ConPTY tests: all three passed (spawn/echo, resize, environment).

These checks cover the source snapshot; installation and release are separate
steps. The dependency `block` 0.1.6 still emits its existing future-incompatibility
notice.

## Remaining owner dependency

Apple Developer ID signing/notarization and Windows Authenticode signing require
owner credentials. The repository currently has the minisign release secret;
no Apple or Windows signing secret is configured, and the Windows test host has
no usable code-signing certificate. Detached minisign verification is a separate
check and already works.

## v0.1.6 GUI acceptance — 2026-10-05

Three items that need a real desktop were re-checked on the released v0.1.6
packages. PowerShell 5.1 history and image restore passed end to end; the IME
candidate window still needs a visual check with macOS desktop permissions.

| Item | Result | Evidence | Limit |
|---|---|---|---|
| IME candidate placement past CJK | Verified at the anchor | mtty places the candidate window at the cursor cell (`active_cursor` -> `cursor_position`). A new test fixes the cursor's *column* after wide characters: `目录:` ends at column 5 and `中` advances two columns. | The candidate window's on-screen box still needs one interactive desktop look; keystroke automation is blocked by the macOS Accessibility/Automation permission (below) |
| Inline image session restore | Verified end to end | On a real macOS desktop: a Kitty image drawn live in a pane (15,500 red pixels in an app self-capture), saved with the scrollback to `pane0.images.json`, and re-placed after a relaunch at the same width — the restored capture again shows 15,500 red pixels. A Sixel restores the same way (43,200 red pixels). | Only at the same pane width; a changed width drops the placement. Animations restore as a still frame (asserted in `graphics::tests`) |
| Windows PowerShell 5.1 history | Verified end to end | On a logged-on Windows desktop, `scripts/ps51-history-acceptance.ps1` returned PASS with Windows PowerShell 5.1.22621.6133 and the released v0.1.6 Windows x86_64 ZIP (SHA-256 matched the release asset). `history.json` contains the second typed marker in `pane0`; `output.txt` was also captured. | One PowerShell 5.1 patch level; other input methods are not covered |

### Why the two desktop checks cannot run over ssh

- **macOS keystrokes**: driving pinyin through the IME needs `System Events`
  keystroke injection, which requires the Accessibility/Automation grant; an ssh
  session cannot grant it and the call is silently blocked.
- **Windows PowerShell 5.1**: the app must run in the logged-on console session
  to hook `PSConsoleHostReadLine`; `schtasks /run` with an interactive-only task
  fails from session 0. Use RDP or the physical console.

