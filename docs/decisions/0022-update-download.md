# ADR 0022 — In-app update download and verification

Status: accepted.

## Context

ADR 0013 only compared a version. A user who is told "update available" should be
able to fetch and check the artifact without leaving the app — and the check must
not trust the download path.

## Decision

`crates/term-ui/src/update.rs` plus a small UI:

- **Manifest.** `update-check-url` may return either a plain document whose first
  line is the version, or JSON:
  `{ "version": "0.2.0", "artifacts": { "macos-aarch64": { "url": …, "sha256": … } } }`.
  `platform_key()` maps OS+arch to that key. Parsing is tested for both forms.
- **Download.** The *Download Update* button (Settings, and a palette verb) is
  enabled only when the manifest offers an artifact for this platform. It fetches
  with `curl -fL --max-time 600 -o <tmp>` on a worker thread.
- **Verification.** SHA-256 is computed **in-crate** (a dependency-free streaming
  implementation, FIPS 180-4) and compared to the manifest's hex digest; on
  mismatch the file is deleted and an error reported. The implementation is
  checked against the standard vectors *and* cross-checked against `shasum` on a
  real file in the test suite.
- **Placement.** A verified file is moved to `~/Downloads` (falling back to the
  temp path), and the status line reports the path and that the checksum matched.

## Addendum (signatures)

When `update-pubkey` (a minisign public key) is set and the artifact carries a
`signature` URL, the detached `.sig` is fetched and checked with `minisign -V`;
a failed check deletes the file, and a missing `minisign` binary downgrades the
status to "signature not checked" rather than failing. After a verified
download the Settings row offers *Open Download* and *Quit and Install* (open
the artifact and quit); true self-replacement is still a follow-up.

## Consequences

- The happy path is fully in-app and the integrity check is independent of curl.
- HTTPS transit plus a pinned digest covers corruption and trivial tampering;
  **signature** verification (minisign/ed25519) is still a follow-up and is the
  remaining gap before auto-install, as is actually replacing the running app.
- Everything is dependency-free (`curl` + our hash), keeping ADR 0006's rule
  that the engine and app pull no copyleft and few new crates.
- Update: the signature check landed (see the addendum) and self-replacement
  landed in ADR 0025, where the button is now *Install and Relaunch*. The
  manifest may also carry `macos-x86_64`, `linux-x86_64` (plus the
  manifest-only `linux-x86_64-deb`), `linux-aarch64` and `windows-x86_64`.
