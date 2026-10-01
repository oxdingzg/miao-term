# ADR 0033 — Optional end-to-end-encrypted sync (B4.5)

Status: accepted.

## Context

Hosts and snippets (M3) are per machine. Termius syncs them through its own
cloud and an account; mtty's principles rule that out: no account, no mtty
server, nothing readable leaves the machine, and the feature is off by default.

## Decision

- **Transport is a folder the user already syncs** (iCloud Drive, Dropbox,
  Syncthing, a network share). `sync-dir` in `config.toml` turns sync on; mtty
  never talks to a server.
- **Key.** Turning sync on creates a random 256-bit key in
  `~/.config/mtty/sync.key` (mode 0600), which never goes into the folder.
  Another device joins by pasting the *pairing code* (`mtty-sync:` + base64url
  of the key) once. No passphrase, so no KDF to tune.
- **Encryption.** XChaCha20-Poly1305 (`chacha20poly1305`, RustCrypto,
  MIT/Apache-2.0) with a random 24-byte nonce per write. A file is
  `MTSYNC1\n` + nonce + ciphertext; a file made with another key fails
  authentication and is reported, never merged.
- **One file per device.** Each device writes only
  `<sync-dir>/mtty-sync/<device-id>.mtsync`, so cloud clients never see two
  writers on one file (no "conflicted copy" files). Reading merges every
  device's file.
- **Per-entry merge.** Entries are hosts and snippets keyed by name, each with
  an `updated` time; deletions are tombstones. A device detects its own
  changes by comparing the files with the state it last synced, then merges all
  devices' entries, newest wins (ties by device id). So two devices adding
  different hosts both keep both; editing one host on two devices keeps the
  later edit.
- **Safety.** Before mtty rewrites `hosts.toml` or `snippets.toml` from a merge,
  it copies the old file to `*.before-sync`. A file that does not parse is
  never overwritten (as for saves).
- **When.** At startup, every 60 s, after a save and on *Sync Now*, as a
  background job.

## Consequences

- Nothing about the user's hosts is readable in the cloud folder; losing the
  key loses only the ability to read the folder, not local data.
- Clock skew between devices can make an older edit win; acceptable for small
  manual edits, and the backup keeps the previous file.
- Only hosts and snippets sync for now; `config.toml` stays per machine.
