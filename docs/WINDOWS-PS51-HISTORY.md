# Windows PowerShell 5.1 history — acceptance runbook

The Windows PowerShell 5.1 history fix shipped in v0.1.6 (`crates/mtty-core/src/shell.rs`,
PR #48). Its unit tests and a standalone quoting check pass, but a **full in-app
capture** still needs a real, logged-on Windows desktop — the app must run in the
console session to hook `PSConsoleHostReadLine`. This runbook finishes that check.

Run it from the Windows desktop (RDP or the physical console), **not** over ssh:
an ssh session lands in session 0, `[Environment]::UserInteractive` is `False`,
and an interactive-only scheduled task cannot start there.

## Steps

1. Get the v0.1.6 app. From the release page download `mtty-windows-x86_64.zip`
   and extract it (it contains `mtty.exe`, `mtty-cli.exe`, `mtty-ptyhost.exe`).
2. Save your work and close all existing mtty windows first. Windows MTP uses a
   fixed named pipe, so an already-running instance defeats directory isolation;
   the script refuses to run while one exists.
3. In a normal desktop PowerShell window, run:

   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass `
     -File ps51-history-acceptance.ps1 -App <path to>\mtty.exe
   ```

4. The script launches mtty in an isolated state with **Windows PowerShell 5.1**
   as the pane shell, types two marked commands into the pane through the shell
   integration, and reads the recorded history back over MTP. It prints
   `PASS`/`FAIL` and writes `history.json` and `output.txt` next to the run.

Keep `-WorkDir` short: PTY hosts use AF_UNIX socket paths even though Windows
MTP uses a named pipe. The script rejects host socket paths of 108 bytes or
more before launching the app. A long path can make the host fail and show a
PowerShell startup error while the fallback shell still records history.
The script also ends the isolated PTY hosts and their shells after the check.

## What "PASS" means

The second typed command appears in `history list` — i.e. the 5.1 hook recorded
it through the same `PSConsoleHostReadLine` path PowerShell 7 uses. That closes
the acceptance record.

## Recording the result

The result row lives in `docs/ACCEPTANCE.md` / `docs/ACCEPTANCE.zh-CN.md`
("v0.1.6 GUI acceptance"). After a PASS, update the Windows PowerShell 5.1 row
from *Blocked* to *Verified* with the PowerShell version and the app build, and
keep the script name as the evidence. Do not paste host names or paths.

## What it does not cover

- It does not exercise every input method or every 5.1 patch level.
- The CI `windows` job lists only PowerShell 7, so 5.1 stays a manual desktop check.
