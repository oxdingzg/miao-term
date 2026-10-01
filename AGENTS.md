# AGENTS.md — working agreement for this repo

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
- Performance gate (release): `cargo test --release -p miao-term-core -p mtty-app -- --ignored`
- Windows real-host checks: `docs/WINDOWS-DEV.md` (IME needs an interactive desktop).
- Packaging/release: `.github/workflows/release.yml` (four runner builds and AppImage/MSI are required; manual dispatch rehearses without publishing).

## Shared working tree

Another session may share this directory.

- Stage only the paths you changed (`git add <paths>`), never `git add -A`.
- Do not rewrite or discard someone else's uncommitted work.
- History rewrites require the owner's explicit approval; afterwards every clone
  must re-sync (`git fetch && git reset --hard origin/main`).

## Screenshots for verification

The application can capture itself without screen-recording permission — useful
for QA and visual diffs when the fix is a rendering change:

- `MTTY_SHOT_AFTER=<secs> ./target/release/mtty` writes
  `/tmp/mtty_shot.ppm` and exits. The same flag works with the executable
  inside `dist/mtty.app` to verify the packaged application.

PPM is written as binary RGB (the native host swaps channels from its BGRA
target), so a plain reader can sample pixels directly. Drive the running app with
`mtty-cli pane run --pane <id> --data ...` while the capture deadline runs.
