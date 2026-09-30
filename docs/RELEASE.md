# Releasing

How the release pipeline works, what to configure, and how the update chain fits
together. English default; keep [`RELEASE.zh-CN.md`](RELEASE.zh-CN.md) in sync.

## Trigger

Push a tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds on
macOS, Linux and Windows, packages each platform, optionally signs, uploads the
artifacts, builds an **update manifest**, and creates a GitHub Release.

For a rehearsal without publishing a release:

```sh
gh workflow run release.yml --ref main -f tag=v0.0.1
```

Manual runs execute the same packaging, signing, and manifest checks, and upload
the combined `release-assembled` artifact. The manifest uses the supplied tag;
its download URLs become live only after a release with that tag is published.

## Artifacts

| Platform | Artifact | Notes |
|----------|----------|-------|
| macOS | `miaotty-macos-arm64.zip`, `miaotty-macos-x86_64.zip` (each contains `miaotty.app` and `miaotty-native.app`) | ad-hoc signed; notarized when the Apple secrets are set. Apple Silicon uses `macos-latest`, Intel uses `macos-15-intel` |
| Linux | `miaotty-linux-x86_64.tar.gz`, `miaotty-linux-x86_64.AppImage`, `dist/*.deb` | AppImage has an explicit `AppRun` entry point |
| Windows | `miaotty-windows-x86_64.zip`, `miaotty-app-<ver>-x86_64.msi` | MSI installation/uninstallation is exercised on the runner |

Every artifact gets a `.sig` next to it when signing is configured.
Detached signatures are generated once in the Linux assembly job, after platform
codesigning, and verified there against the committed public key.
All app bundles, main application archives, and installers contain the two hosts
and the CLI; the additional `miaotty-native-macos-*.zip` is a standalone host.
All four runner builds and AppImage/MSI packaging are required to succeed.

## Secrets (all optional)

| Secret | Purpose |
|--------|---------|
| `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | codesign + notarize + staple the macOS build |
| `WINDOWS_CERT_PFX` (base64), `WINDOWS_CERT_PASSWORD` | `signtool` the MSI |
| `MINISIGN_SECRET_KEY` | sign every artifact with `minisign` (passwordless key) |

Without them the pipeline still produces usable (unsigned) artifacts and prints
what it skipped.

### Generating the minisign key

```sh
minisign -G -W -p minisign.pub -s minisign.key     # -W: no password, for CI
gh secret set MINISIGN_SECRET_KEY < minisign.key   # raw contents of the secret key
```

The repository keeps `minisign.pub` at its root, and the release job attaches it,
so `…/releases/download/<tag>/minisign.pub` always resolves. Tell users to put
its single line into their config:

```toml
update-pubkey = "RWQ…"
```

## Backing up and rotating the minisign key

GitHub Secrets are **write-only**: once `MINISIGN_SECRET_KEY` is set you cannot
read it back from the UI or API. Losing the local copy does **not** break
releases (CI keeps signing), but you cannot recover the raw key either — so keep
an offline backup and treat the key as long-lived:

- The public key is in the repo (`minisign.pub`) and attached to every release.
- Back up the private key somewhere durable (password manager, encrypted copy,
  macOS Keychain). To rotate: generate a new pair, `gh secret set
  MINISIGN_SECRET_KEY`, replace `minisign.pub` and tell users to update
  `update-pubkey`. Old releases stay verifiable with the archived old public key.

## Update manifest

The release job writes `latest.json` and attaches it to the release:

```json
{
  "version": "0.1.0",
  "artifacts": {
    "macos-aarch64": {
      "url": "https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip",
      "sha256": "…",
      "signature": "https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip.sig"
    },
    "linux-x86_64": { "url": "…AppImage", "sha256": "…" },
    "windows-x86_64": { "url": "…msi", "sha256": "…" }
  }
}
```

Point the app at it:

```toml
update-check-url = "https://github.com/oxdingzg/miao-term/releases/latest/download/latest.json"
```

Then *Settings → Check for Updates* reports the version, *Download Update* fetches
the artifact for the running platform, verifies its SHA-256 (and the minisign
signature when `update-pubkey` is set), and on macOS *Install and Relaunch*
swaps the bundle with a rollback helper (ADR 0025).

Platform keys match the app's `platform_key()`: `macos-aarch64`,
`macos-x86_64`, `linux-x86_64`, `linux-aarch64`, `windows-x86_64`.
`linux-x86_64-deb` is a manifest-only key for the `.deb` artifact (the app never
asks for it), and `release.yml` does not currently emit a `linux-aarch64`
manifest entry.

`scripts/build-update-manifest.py` requires both macOS architectures, Linux and
Windows, computes SHA-256 from the actual files, and prefers AppImage/MSI over
archive fallbacks. When minisign is configured, missing signatures fail the
assembly job; all detached signatures are verified against `minisign.pub`.
Run `python3 scripts/test-release-manifest.py` to check selection and failure cases.

## MSI (Windows) — verified

The WiX template is committed at [`miaotty-app/wix/main.wxs`](../miaotty-app/wix/main.wxs)
(it installs `miaotty.exe`, `miaotty-cli.exe`, and `miaotty-native.exe` into
`%ProgramFiles%\miaotty\bin`, adds that directory to the machine `PATH`, and
registers an uninstall entry). `cargo wix --package miaotty-app` builds it; it
needs WiX 3.x (chocolatey `wixtoolset`), and must run **from `miaotty-app/`**
because the template references `wix\License.rtf` relatively.

Verified end to end on a real Windows 11 host (2026-09-29): build → `msiexec /i`
→ both binaries on disk + PATH entry + "miaotty 0.0.0" uninstall entry → the
installed app launches → `msiexec /x` removes the directory.


## Verifying installers

Both installers were checked on real hardware (2026-09-29).

**Windows MSI** — see the section above (build → install → PATH → run → uninstall).

**Linux `.deb` and AppImage** (Ubuntu 24.04):

```sh
cargo build --release -p miaotty-app -p miaotty-cli -p miao-term-widget
cargo install cargo-deb --locked && cargo deb -p miaotty-app --no-build
sudo dpkg -i target/debian/miaotty_*_amd64.deb     # /usr/bin/miaotty{,-cli,-native}
miaotty-cli ping                                   # runs; errors only because no host
```

The AppImage needs a **real 256x256 icon** — `appimagetool` refuses a 1x1
placeholder, and the desktop file must carry an `Icon=` key — so the pipeline
generates one with `scripts/make-icon.py` (standard library only). Result:
`miaotty-linux-x86_64.AppImage` containing `miaotty` and `miaotty-cli`, and it
extracts cleanly (`--appimage-extract`).

**Not verified**: the GUI itself. The machine's X displays belong to the login
screen (no auth cookie), and under `xvfb` + software Vulkan (lavapipe) `wgpu`
fails with `Invalid surface` (a limitation of offscreen software Vulkan, not the
app). A real desktop session or a GPU is needed for that.

## Verifying a signed release (users)

Fetch the artifact, its `.sig`, and the published public key (`minisign.pub`):

```sh
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip.sig
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/minisign.pub
minisign -Vm miaotty-macos-arm64.zip -p minisign.pub      # -> "Signature and comment signature verified"
```

The manifest ([`latest.json`](#update-manifest)) carries both a `sha256` and a
`signature` URL per artifact, so a downloader can check either or both:

```sh
shasum -a 256 miaotty-macos-arm64.zip   # compare with the manifest's "sha256"
```

Nothing is signed until `MINISIGN_SECRET_KEY` is configured, so unsigned releases
simply have no `.sig` — verification is optional, not required to install.

## Acceptance coverage and remaining work

The release workflow checks Linux package contents and CLI execution, and Windows
MSI install → three binaries + URL handler → CLI execution → uninstall. These
checks do not establish interactive desktop behavior. The real-host checks above
describe the earlier two-binary packages, not the new native-host payload.

Local macOS verification (2026-09-30, `9e817a9`): both bundles pass
`codesign --verify --deep --strict`; fmt, strict clippy, workspace tests and all
four release performance budgets pass. The bundled binaries were driven over
MTP for Kitty inline graphics and Mermaid sequence/pie previews in both hosts,
with screenshots confirming the output. The eframe chrome now stays dark even
when the OS uses a light theme, keeping preview text legible.

The macOS update-helper test executes the swap and missing-download rollback
against temporary bundles with spaces in their paths, using a test relaunch
command. Archive selection rejects a native-only download and selects
`miaotty.app` from a dual-bundle archive. These are component checks, not a real
download/install/relaunch acceptance. Windows zip updates also replace the native
binary along with the eframe host and CLI.

| Check | Acceptance criterion | Environment still needed |
|-------|----------------------|--------------------------|
| AppImage desktop launch | Execute the AppImage, open a pane, use MTP, then exit cleanly | Linux desktop |
| Wayland shortcut | Grant the portal request; invoke the shortcut with the app unfocused | Wayland desktop |
| Windows IME, both hosts | Compose CJK text, check candidate position, commit/cancel, switch splits | Interactive Windows desktop |
| Update self-replace | Download/verify a newer version, install/relaunch, retain workspace; exercise failure recovery | Installed macOS app, Windows MSI/zip, Linux AppImage |
| Visual acceptance | Capture inline graphics, IME composition and Mermaid previews in both hosts | Desktop session |
| Platform signing | Verify Developer ID/notarization and MSI Authenticode on shipped artifacts | Apple/Windows signing credentials |

Record the commit, package hash, commands, result and screenshots for each desktop
check. A manual rehearsal does not create live update-download URLs; a published
release (or a dedicated test manifest server) is needed for download acceptance.
