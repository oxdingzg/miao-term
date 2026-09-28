# ADR 0025 — Install and relaunch (self-replacement)

Status: accepted.

## Context

ADR 0022 verified a downloaded update but stopped at "open the artifact and
quit". Completing the loop means replacing the running app and restarting it,
which cannot be done from inside the running process (its executable/bundle is
in use).

## Decision

On macOS, `install_update` performs the swap with a detached helper:

1. Require a **verified** artifact (`update_ready`, set only after the checksum
   and optional signature check pass).
2. If the process is not inside an `.app` (i.e. a dev build), refuse and just
   open the artifact; `bundle_root()` detects `…/X.app/Contents/MacOS/x`.
3. Unpack the artifact (`unzip`) into a staging dir and locate the new `.app`
   (`find_app`).
4. Write `install.sh`, which waits for our PID to exit, moves the current bundle
   aside to `<bundle>.old`, moves the new bundle into place, relaunches with
   `open`, and removes the backup — restoring the old bundle if the move fails.
5. Launch it detached (`nohup sh … &`) and quit, so the swap happens after we
   exit.

Elsewhere the button degrades to *open the download* and a status message, since
per-platform in-place replacement (Windows locked `.exe`, Linux AppImage/deb) is
different work.

## Consequences

- The update path is end-to-end on macOS: check → download → verify → install →
  relaunch, with a rollback if the swap fails.
- The helper text and `bundle_root` are pure functions and unit-tested; the
  actual swap can only be exercised on a real installed bundle.
- Windows/Linux self-replacement and a signed-release channel remain follow-ups.
