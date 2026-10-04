# ADR 0038 — Importing PuTTY `.ppk` private keys

> 中文: [`0038-ppk-import.zh-CN.md`](0038-ppk-import.zh-CN.md)

Status: accepted.

## Context

M6 R2. Windows users keep their SSH keys in PuTTY's PPK format; OpenSSH —
which mtty uses for every connection — cannot read it. Two versions are in the
wild: **v2** (PuTTY 0.52–0.74) derives the AES key and IV from the passphrase
with SHA-1 and protects the file with HMAC-SHA-1; **v3** (0.75+) uses Argon2id
and HMAC-SHA-256. Keys are commonly RSA or Ed25519, sometimes ECDSA.

The requirement is one-way import: read a `.ppk`, verify it, and write an
OpenSSH private key. The OpenSSH key must **never** be stored unencrypted.

## Decision

1. **A new GPU-free crate, `mtty-keys`.** It parses PPK v2 and v3,
   verifies the `Private-MAC` before using any key material, decrypts the
   private blob, and returns the key as `ssh-key`'s `PrivateKey`. It has unit
   tests and no windowing code, so it can be fuzzed and audited separately.
2. **Supported key types**: Ed25519, RSA and ECDSA (NIST P-256/P-384/P-521).
   DSA is rejected with a clear message (obsolete; OpenSSH 9.8 disables it).
3. **Supported encryption**: v2 `none` and `aes256-cbc` (SHA-1 KDF, a zero
   IV, and the decrypted blob in the MAC preimage); v3 `none` and
   `aes256-cbc` (Argon2id, 80 bytes out, HMAC-SHA-256 over the decrypted
   blob). `chacha20-poly1305` is recognised and rejected with a clear message
   — `puttygen` writes `aes256-cbc` for v3, so there is no vector to test it
   against yet. The MAC is always checked; a wrong passphrase fails before the
   key is exposed. Argon2 parameters come from the file's `Argon2-*` headers.
4. **Output is always encrypted.** The imported key is re-encoded through the
   `ssh-key` crate's OpenSSH writer with bcrypt-pbkdf + `aes256-ctr` under a
   passphrase the user chooses at import time. An empty new passphrase is
   refused; there is no "save unencrypted" path.
5. **UI.** *Hosts…* gains **Import PuTTY Key…**: pick the `.ppk`, enter its
   passphrase if it is encrypted, then a new passphrase (twice). The key is
   written to `~/.ssh/<name>` (0600) with `<name>.pub` beside it (0644),
   refusing to overwrite an existing file unless the user confirms. The
   passphrases and the decoded key live in `zeroize`-ing buffers and are never
   logged or put on a command line.
6. **Dependencies** (all MIT or Apache-2.0, per ADR 0006): `aes`, `cbc`,
   `sha1`, `sha2`, `hmac`, `argon2`, `base64`, `zeroize`, and `ssh-key` with
   the `encryption`, `getrandom`, `alloc`, `ed25519`, `rsa`, `p256`, `p384`,
   `p521` features. `ssh-key` supplies the OpenSSH parser and encoder and the
   bcrypt-pbkdf + aes256-ctr output encryption.

## Consequences

- Import is pure Rust and in-process: no `puttygen` dependency on any platform,
  and it works on Windows where OpenSSH's own tools may be absent.
- PPK v1 (a brief pre-0.52 format) is not supported; the file is reported as
  such rather than misparsed.
- The conversion is one-way by design. Exporting OpenSSH keys back to PPK is
  not planned; PuTTY can read OpenSSH keys itself.
- Any future change to the on-disk key handling must keep the "never
  unencrypted" rule; the tests assert that unencrypted output is impossible
  (an empty passphrase is rejected).
