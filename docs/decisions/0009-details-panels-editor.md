# ADR 0009 — Details panels and the file editor

Status: accepted.

## Context

The right-hand Details column must show more than the working directory. U5 of
the roadmap calls for the git, files and ports panels, plus a file viewer that
opens by double-click. All of this reads the *focused pane's* context (cwd and
child pid) and must not block the UI thread.

## Decision

**Panels.** Details becomes a tab strip: Info, Agent, Outline, Git, Files,
Ports. Info/Agent/Outline read existing state; Git/Files/Ports read a snapshot
produced by a background worker.

**Background worker** (`miaotty-app/src/panels.rs`). A thread owns a request
channel and a shared `Arc<Mutex<Panels>>` slot. The UI sends a `Request { cwd,
pid }` when the focused pane's context changes or every 2.5 s; the worker
coalesces to the newest request, computes, and publishes. The UI only ever
reads a clone, so it never blocks.
- Git: `git -C <cwd> rev-parse --abbrev-ref HEAD` and `status --porcelain`.
- Files: `read_dir`, directories first then case-insensitive name order.
- Ports: `lsof -nP -iTCP -sTCP:LISTEN -a -p <pid>` (empty where `lsof` is
  unavailable, e.g. Windows).

**Editor / Quick Look.** Double-clicking a file in the Files or Git panel opens
it in a window that is **read-only by default** (Preview), with an Edit toggle,
Reload and Save. Files over 2 MB are refused, and non-UTF-8 bytes are decoded
lossily, so the viewer never crashes on binary or huge inputs. Markdown/Mermaid
rendering is deferred.

**Palette.** Open Quickly gains `file` entries for the focused directory, and
its `OpenFile` action routes to the same editor.

## Consequences

- Panels stay responsive even when `git status` or `lsof` is slow; the UI shows
  the previous snapshot until the next one lands.
- The worker is a plain thread, not an async runtime — consistent with the
  engine's threading model (ADR 0002).
- Symbol search, `jump` frecency and rich previews (Markdown/Mermaid) are
  follow-ups; this ADR fixes the plumbing they build on.
- Update: content search landed in ADR 0012, and rich previews (Markdown,
  Mermaid, images) landed in ADRs 0015/0017/0020; `jump` frecency remains a
  follow-up.
