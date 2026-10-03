# ADR 0043 — Transactional Windows update helper

[简体中文](0043-windows-update-transaction.zh-CN.md)

Status: accepted. Supersedes the Windows helper choice in ADR 0025.

## Context

The batch helper continued after extraction/copy failures and could leave an
installation containing executables from different builds. It also ignored MSI
failure exit codes. Real Windows testing exposed quoting and Unicode-path
requirements for the detached helper.

## Decision

Use a UTF-8 BOM PowerShell script launched directly with `CREATE_NO_WINDOW`.
It waits for the app to exit before doing any replacement.

For ZIPs, use a unique staging directory inside the installation directory,
extract and validate `mtty.exe`, `mtty-cli.exe` and `mtty-ptyhost.exe`, then move
the old executables to backups and replace all three. On failure restore every
moved file, restart the original app after a successful rollback, and preserve
backups if rollback itself fails.

For MSI, wait for `msiexec`, check its success/reboot exit codes and relaunch only
an existing installation. Failure must not be reported as a successful upgrade.

## Consequences

Signed download verification remains a prerequisite. The helper handles
replacement failures; it does not add an application-health handshake after
launch. Tests execute the generated PowerShell script on Windows and exercise
corrupt archives, file locks and paths containing spaces, Unicode and quotes.
