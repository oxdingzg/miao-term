# ADR 0006 — License and dependency policy

Status: accepted.

## Context

The engine (`miao-term-*`) must be embeddable by third parties, and the app
(`miaotty-app`) is its first consumer. The Rust ecosystem we build on is
overwhelmingly permissively licensed, but some transitive crates are
dual-licensed with a copyleft *option* (e.g. `Apache-2.0 OR GPL-2.0-only`).
We need one clear outbound license and a rule for what we may depend on.

## Decision

- **Outbound**: the engine crates and `miaotty-app` are licensed
  **Apache-2.0** (root [`LICENSE`](../../LICENSE)). Apache-2.0 is permissive
  and embeddable, and adds an explicit patent grant and trademark clause.
- **Inbound**: dependencies must be permissive. Allowed: `Apache-2.0`, `MIT`,
  `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `0BSD`, `CC0-1.0`,
  `Unicode-3.0`, `OFL-1.1`, `BSL-1.0`, and any expression that offers one of
  these as a choice.
- **No copyleft inside the engine**: GPL/AGPL/SSPL and similar are excluded.
  A copyleft-*optional* crate is acceptable only when a permissive option is
  selected.
- **Review**: `cargo-deny` (licenses + advisories) is the intended CI gate; it
  currently runs during review, and no new dependency lands without it passing.

## Consequences

- One license, no "dual-licensed, choose one" ambiguity for consumers.
- A dependency that drops its permissive option must be replaced or pinned.
- Bundling third-party Apache-2.0 code that ships a `NOTICE` file obliges us to
  propagate that notice.
