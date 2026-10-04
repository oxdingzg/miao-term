# ADR 0006 — License and dependency policy

Status: accepted.

## Context

The engine (`mtty-*`) must be embeddable by third parties, and the app
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

## Amendment — the gate exists, and what it found

`cargo-deny` now runs in CI (`deny.toml`, the `deny` job in `ci.yml`) rather
than during review, which is the difference between a policy and a habit.

Its first run rejected four crates. Three are permissive licences that were
simply absent from the list above, and are added to it:

| Licence | Crate | Why it is allowed |
|---|---|---|
| `CDLA-Permissive-2.0` | `webpki-roots` | The root certificate list behind `egui_extras`' http feature. Permissive, no copyleft. |
| `LicenseRef-UFL-1.0` | `epaint_default_fonts` | The Ubuntu font egui bundles. Permissive, and the crate is already covered by `OFL-1.1` and `MIT OR Apache-2.0` besides. |
| `Apache-2.0 WITH LLVM-exception` | `target-lexicon` | The exception is strictly more permissive than `Apache-2.0`; the expression is not the bare `Apache-2.0` already listed, which is why it was rejected. |

The fourth was a genuine violation: **`serialport`, a direct dependency of
`mtty-ui`, is `MPL-2.0`** — weak copyleft, which this ADR excludes. It had
been recorded as MIT since the day it was added, and the wrong claim had been
copied into ADR 0037 and the manifest comment without anyone checking.

**Resolution: replace the crate, do not exempt it.** The dependency is now
`serial2` (BSD-2-Clause OR Apache-2.0), a fork of the same project. Exempting
MPL-2.0 was the alternative and was rejected: the engine is meant to be
embeddable, and "no copyleft inside the engine" is worth more than one crate's
convenience. ADR 0037 carries the details.

### Advisories

The same first run reported one vulnerability and seven unmaintained crates.
Both were assessed before either was written down as acceptable.

**The vulnerability is accepted as unreachable.** RUSTSEC-2023-0071, the Marvin
Attack, is a timing side channel in `rsa`'s *decryption*, and no fixed release
exists. mtty reaches `rsa` only through `ssh-key`, and only to parse and
re-encode a private key: the workspace calls `PrivateKey::from_openssh` and
`to_openssh` and nothing else, and contains no `sign`, `verify` or RSA-decrypt
call at all — the single `decrypt` in `term-keys` is the PuTTY `.ppk`'s own
AES-256-CBC. The crate is linked into the binary; the vulnerable operation is
not on any path from this code. `deny.toml` records that reasoning beside the
exception, with the date, and a note to re-examine it when `ssh-key` is
upgraded.

**The unmaintained advisories are warnings, not exceptions.** "Unmaintained"
means upstream stopped developing a crate; it does not mean the crate is faulty.
Silencing them would hide them — which is how an MPL-2.0 dependency went
unnoticed in this very tree — so the gate demotes them with `-W unmaintained`
and prints them on every run while staying green. Two of the six, `rustybuzz`
and `ttf-parser`, do ship and do read font files, so they are the ones worth
watching; `skrifa`, `ttf-parser`'s successor in the same stack, is already in
the dependency tree, so that migration belongs to upstream. The rest are
compile-time only, or a version its maintainers have said is complete.

Hand-rolling any of these to silence an advisory was considered and rejected.
Reimplementing text shaping or font parsing is where vulnerabilities get
*introduced*, not removed, and writing our own RSA to avoid a crate that is not
itself wrong would be worse still.
