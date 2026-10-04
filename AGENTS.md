# AGENTS.md — working agreement for this repo

## Highest-priority project rule: every change lands through a pull request

`main` is protected: it must stay releasable, and every change reaches it
through a pull request with an approving review. Direct pushes to `main` are
rejected, including for maintainers. This mirrors the workflow used across the
miao projects.

- Work on a short-lived branch, open a pull request, and let CI pass before
  merging. Never commit or push directly to `main`.
- Branch names use a conventional-commit type prefix — `feat/`, `fix/`,
  `docs/`, `chore/`, `refactor/`, or `test/` — plus a few hyphen-separated
  words, e.g. `fix/pty-resize`. Keep the branch short-lived and delete it after
  merge.
- Open one pull request per concern, with a conventional title
  (`type(scope): summary`) and a linked issue.
- Merge by squash, so one pull request becomes one commit on `main`.
- A branch's cost grows faster than its age; land it within a day or two, or
  split it into pieces that are each safe on their own.

## Mandatory: never compile Rust on the local development machine

**This is a mandatory execution constraint, not a recommendation. Before running
any command that may compile Rust, ensure it runs on an approved remote build
host. Never compile Rust on the local development machine.**

- This applies to `rustc`, `cargo build`, `cargo check`, `cargo test`,
  `cargo clippy`, `cargo bench`, `cargo run`, `cargo install`, and any script,
  IDE task, packaging step, or temporary test harness that invokes Rust
  compilation, including dependency compilation and debug builds.
- Use the approved macOS, Windows, or Linux build hosts identified in the
  private global agent instructions. Host addresses, aliases, and credentials
  must stay in private configuration, outside this public repository.
- Sync the required source to a remote build host, execute compilation and
  compilation-dependent checks there over SSH, and copy artifacts back when
  local runtime or UI verification is needed.
- If no approved build host is reachable, report the blocker. Do not fall back
  to local compilation, even for a small crate or a quick check.
- Formatting (`cargo fmt --all --check`), source inspection, and running
  already-built binaries may be performed locally because they do not compile
  Rust.
- The verification requirements below must respect this constraint: Clippy,
  tests, performance gates, and Rust packaging builds run remotely.

## Privacy: keep machine specifics out of the repository

This repository is public. **Never** commit host aliases, user names, absolute
home paths, private IPs, internal hostnames, or credentials — in files, docs,
commit messages, or branches. Use placeholders instead:

- `<windows-host>`, `%USERPROFILE%`, `C:\Users\<you>` for Windows
- `$HOME`, `/home/<you>`, `/Users/<you>` for Unix
- `<you>`, `<dir containing the checkout>` for anything else

Machine-specific runbooks (which box, how to reach it, paths) belong in private
notes, **not** in `docs/`.

Guardrails:

- `sh scripts/check-privacy.sh` scans tracked files (generic patterns + an
  optional denylist). CI runs it as the `privacy` job.
- `sh scripts/install-hooks.sh` enables a pre-commit hook that runs the same
  check on staged files (via `core.hooksPath=.githooks`).
- Your own identifiers go in `~/.config/miao-term/privacy-denylist` (one string
  per line) or the CI secret `PRIVACY_DENYLIST` — never in the tree.

  Do **not** denylist your public GitHub handle (`oxdingzg` is fine in repo
  URLs) or other strings that legitimately appear in the tree — the scanner
  matches substrings, so `dingzg` hits every `github.com/oxdingzg` link. Local
  home paths are already covered by the generic pattern.

If something sensitive is committed, treat it as an incident: rewrite history
(`git filter-repo --replace-text`), force-push with owner approval, tell
everyone to re-sync, and rotate any credentials involved.

## Verify before claiming done

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`
- Performance gate (release): `cargo test --release -p miao-term-core -p miao-term-editor -p miao-term-widget -p miao-term-ptyhost -p mtty-app -- --ignored`
- Windows real-host checks: `docs/WINDOWS-DEV.md` (IME needs an interactive desktop).
- Packaging/release: `.github/workflows/release.yml` (four runner builds and AppImage/MSI are required; manual dispatch rehearses without publishing).

## Shared working tree

Another session may share this directory.

- Stage only the paths you changed (`git add <paths>`), never `git add -A`.
- Do not rewrite or discard someone else's uncommitted work.
- History rewrites require the owner's explicit approval; afterwards every clone
  must re-sync (`git fetch && git reset --hard origin/main`).

### Isolate a session in its own worktree

A pull request only protects what is already committed. Uncommitted work in the
shared checkout can still be wiped by another session's `checkout`/`reset
--hard` (this has happened). For any task that edits the tree, give the session
its own worktree and commit early:

```sh
sh scripts/session-worktree.sh start feat/short-name   # prints the path
cd .worktrees/feat-short-name                          # work and commit there
sh scripts/session-worktree.sh finish feat/short-name  # remove + prune
```

- `start` reuses an existing worktree for the same branch, so re-running it
  cannot stack duplicates under `.worktrees/` (git-ignored).
- `finish` removes the working copy and prunes git's metadata; it refuses when
  the worktree has uncommitted changes, so it cannot discard work. Pass
  `--delete` to also drop the local branch once it is merged.
- `list` shows what is left; `prune` clears entries whose directory is gone.
- **Always `finish` when a task ends** — nothing removes worktrees
  automatically, and a leftover `.worktrees/*` is a bug, not a feature.

## Screenshots for verification

The application can capture itself without screen-recording permission — useful
for QA and visual diffs when the fix is a rendering change:

- `MTTY_SHOT_AFTER=<secs> ./target/release/mtty` writes
  `/tmp/mtty_shot.ppm` and exits. The same flag works with the executable
  inside `dist/mtty.app` to verify the packaged application.

PPM is written as binary RGB (the native host swaps channels from its BGRA
target), so a plain reader can sample pixels directly. Drive the running app with
`mtty-cli pane run --pane <id> --data ...` while the capture deadline runs.

- `MTTY_QA_COMMAND="<palette label>"` runs one command-palette command at
  startup by its English label (for example `Agent Tasks…`), so windows that
  only open from the palette can be captured too.
- `MTTY_QA_SCROLL=<lines>` scrolls the active pane back by that many lines,
  like the wheel.
- `MTTY_QA_DRAG=<x>,<y>[,alt]:<path>[;<path>…]` holds a file drag at a window
  point (logical pixels) so the drop targets can be captured;
  `MTTY_QA_DROP=1` also drops the files there. All of these run at startup, or
  `MTTY_QA_AFTER=<secs>` after startup.
- Use isolated state (`HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
  `XDG_RUNTIME_DIR` pointing at a temporary directory) so a capture never reads
  or writes your real configuration; keep the runtime path short (Unix sockets
  are limited to ~100 bytes).
