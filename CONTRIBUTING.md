# Contributing to mtty

Thanks for your interest. mtty is in early development; the most useful
contributions right now are precise bug reports, focused fixes, and terminal
rendering or PTY fixes with a reproduction.

## Ground rules

- `main` is protected and always releasable. Every change arrives by pull
  request with an approving review from a code owner and passing CI. Direct
  pushes to `main` are rejected, including for maintainers.
- Keep one pull request to one concern.

## Branches and merging

Trunk-based with short-lived branches.

| | |
|---|---|
| **Naming** | `feat/`, `fix/`, `docs/`, `chore/`, `refactor/`, or `test/` plus a short subject, e.g. `fix/pty-resize` |
| **Lifetime** | Merge within a day or two |
| **Merging** | Squash, so one pull request becomes one commit on `main` |
| **After merge** | The branch is deleted |

If a change is too large to land in a couple of days, land it in pieces behind
whatever makes each piece safe on its own.

## Pull request standards

- **Title**: conventional commit, `type(scope): summary` — `feat`, `fix`,
  `docs`, `chore`, `refactor`, or `test`. Scope is optional, e.g. `fix(pty):`.
- **Issue**: reference an existing issue with `Closes #<number>` (not required
  for `docs`, `refactor`, or `feat` PRs).
- **What and why**: describe the issue and why your change fixes it, briefly
  and in your own words.
- **Verification**: explain how you tested it and how a reviewer can confirm
  the fix. For UI changes, include a screenshot or recording.

## Development

Rust 2024 edition. Rust compilation is offloaded to an approved remote build
host; see `AGENTS.md`. Local checks that do not compile Rust are fine.

Before claiming done, the CI gates are also the local gates:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run these on a build host, not the local machine. `scripts/check-privacy.sh`
scans tracked files for machine-specific details and must stay green.
