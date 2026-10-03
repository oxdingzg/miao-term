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
