# Local patches to muda 0.15.3

Only the macOS icon path is touched (`src/platform_impl/macos/icon.rs`),
because its `unwrap`s run inside an AppKit menu callback that cannot unwind:

1. `PlatformIcon::to_png` — a zero-sized icon (what a process without an
   application icon produces) made `png` reject the image and the `unwrap`
   abort the process. It now encodes a transparent image of at least 1x1 and
   returns an empty buffer instead of panicking if encoding still fails.
2. `PlatformIcon::to_nsimage` — the same zero-size case divided by zero, and a
   decode failure panicked. The size math now clamps to 1 and an undecodable
   image falls back to an empty `NSImage`.

Symptom without the patch: clicking the standard *About* (or *Quit and Keep
Windows*) menu item aborted `miaotty-native` with
`FormatError { inner: ZeroWidth }` from `muda/src/platform_impl/macos/icon.rs`.

Additionally two elided lifetimes in `src/platform_impl/mod.rs` (`child`,
`child_mut`) are spelled out so the vendored copy lints clean under the
workspace's `clippy -D warnings`.

Upstream is MIT / Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`); only `src/`,
`Cargo.toml` and those licence files are vendored.
