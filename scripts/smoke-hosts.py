#!/usr/bin/env python3
"""Real-window/PTY smoke for the native miaotty app on macOS, using isolated state.

Build release binaries first. Results and screenshots stay in a temporary
folder. This exercises MTP-driven features; pointer routing is covered by the
headless overlay-drag regression test, not by this script.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def eventually(check, timeout=12):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError("timed out waiting for the host or PTY")


def smoke(output, bundle=None):
    host = "miaotty"
    case = output / host
    case.mkdir()
    fixture = case / "fixture space 目录"
    fixture.mkdir()
    marker = fixture / "visible-marker.txt"
    marker.write_text("HOST_SMOKE_VIEW_OK\n目录 fixture\n", encoding="utf-8")
    config = case / "config/miaotty"
    config.mkdir(parents=True)
    legacy_session = config / "native-session.json"
    legacy_session.write_text(json.dumps({
        "active_tab": 0, "recent": [str(marker)], "tabs": [{
            "title": "migrated-workspace", "group": "QA", "active": "old-b",
            "panes": [{"id": "old-a", "cwd": str(fixture)}, {"id": "old-b", "cwd": str(fixture)}],
            "layout": {"dir": "right", "ratio": 0.35, "a": {"leaf": "old-a"}, "b": {"leaf": "old-b"}},
        }],
    }))
    socket = case / "miaotty.sock"
    env = os.environ.copy()
    for name in ("MIAOTTY_MTP_TOKEN", "MIAOTTY_MTP_ALLOW", "MIAOTTY_SOCKET",
                 "MIAOTTY_NATIVE_SHOT", "MIAOTTY_NATIVE_SHOT_AFTER", "MIAOTTY_SHOT_AFTER"):
        env.pop(name, None)
    env.update(HOME=str(case), XDG_CONFIG_HOME=str(case / "config"),
               XDG_DATA_HOME=str(case / "data"), XDG_RUNTIME_DIR=str(case),
               SHELL="/bin/sh")
    env["MIAOTTY_SHOT_AFTER"] = "10"
    shot = Path("/tmp/miaotty_shot.ppm")
    launched_at = time.time()
    binaries = bundle / "Contents/MacOS" if bundle else ROOT / "target/release"
    cli_path = binaries / "miaotty-cli"
    identity = subprocess.check_output([str(binaries / "miaotty"), "--version"], text=True).strip()
    assert identity.endswith(" (native)"), "smoke must exercise the native main application"

    def cli(*args, ready=False):
        proc = subprocess.run([str(cli_path), "--socket", str(socket), *map(str, args)],
                              env=env, capture_output=True, text=True, timeout=5)
        if ready and proc.returncode:
            return None
        if proc.returncode:
            raise AssertionError(f"{host}: {' '.join(map(str, args[:2]))}: {proc.stderr.strip()}")
        return json.loads(proc.stdout)

    with (case / "host.log").open("w") as log:
        process = subprocess.Popen([str(binaries / host)],
                                   cwd=fixture, env=env, stdout=log, stderr=log)
        checks = []
        try:
            panes = eventually(lambda: (cli("pane", "list", ready=True) or {}).get("panes"))
            assert len(panes) == 2, "legacy native split session was not restored"
            saved = json.loads((config / "session.json").read_text())
            assert saved["tabs"][0]["group"] == "QA"
            assert abs(saved["tabs"][0]["layout"]["ratio"] - 0.35) < 1e-6
            assert legacy_session.exists(), "migration deleted the legacy session"
            checks.append("native session migration preserving split ratio, group and source file")
            pane = panes[0]["id"]
            # Bring only the test window forward for desktop verification.
            # Activate only this test process; no keyboard/mouse injection.
            subprocess.run(["swift", "-e", "import AppKit; "
                            f"NSRunningApplication(processIdentifier: {process.pid})?"
                            ".activate(options: [.activateIgnoringOtherApps])"],
                           check=True, capture_output=True, timeout=15)
            assert cli("health")["ok"]
            cli("pane", "focus", "--pane", pane)
            checks.append("host startup, health, pane discovery/focus")

            roundtrip = fixture / "roundtrip.txt"
            content = "file roundtrip 中文\n"
            cli("file", "write", "--path", roundtrip, "--data", content)
            assert roundtrip.read_text(encoding="utf-8") == content
            assert cli("file", "read", "--path", roundtrip)["data"] == content
            checks.append("file write/read including Unicode")

            proof = fixture / "pty-proof.txt"
            command = (f"cd {shlex.quote(str(fixture))}; "
                       "printf '\\033[90mgray-path-readable\\033[0m\\n'; "
                       f"printf 'PTY_OK\\n' > {shlex.quote(str(proof))}; "
                       f"pwd >> {shlex.quote(str(proof))}; "
                       f"stty size >> {shlex.quote(str(proof))}")
            cli("pane", "run", "--pane", pane, "--data", command)
            eventually(lambda: proof.exists() and len(proof.read_text().splitlines()) >= 3)
            lines = proof.read_text().splitlines()
            assert lines[0] == "PTY_OK"
            assert Path(lines[1]).resolve() == fixture.resolve()
            assert all(int(n) > 0 for n in lines[2].split()) and len(lines[2].split()) == 2
            checks.append("real shell command, cd with spaces/CJK, PTY resize")

            cli("pane", "close", "--pane", panes[1]["id"])
            eventually(lambda: len(cli("pane", "list")["panes"]) == 1)
            checks.append("close one split pane while retaining the other shell")

            cli("history", "add", "--pane", pane, "--command", "SMOKE_HISTORY_OK")
            assert "SMOKE_HISTORY_OK" in json.dumps(cli("history", "list", "--pane", pane))
            cli("state", "miao", "--pane", pane, "--state", "processing")
            assert "processing" in json.dumps(cli("state", "list"))
            cli("state", "miao", "--pane", pane, "--state", "idle")
            assert "idle" in json.dumps(cli("state", "list"))
            checks.append("command history, agent processing/idle state")

            cli("view", marker)
            time.sleep(0.3)
            cli("edit", marker)
            assert marker.read_text(encoding="utf-8").startswith("HOST_SMOKE_VIEW_OK")
            checks.append("view/edit dispatch without modifying the fixture")

            process.wait(timeout=20)
            assert process.returncode == 0
            assert shot.stat().st_mtime >= launched_at, "capture is stale"
            data = shot.read_bytes()
            magic, dimensions, maximum, pixels = data.split(b"\n", 3)
            width, height = map(int, dimensions.split())
            assert magic == b"P6" and maximum == b"255"
            assert len(pixels) == width * height * 3
            colors = {pixels[i:i + 3] for i in range(0, len(pixels), 3)}
            assert len(colors) > 50, "capture has no rendered content"
            (case / "capture.ppm").write_bytes(data)
            checks.append("fresh GPU screenshot and clean timed exit")
            return {"host": host, "identity": identity, "checks": checks, "capture_size": [width, height]}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, help="test the packaged miaotty.app and its bundled CLI")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this real-window capture smoke currently targets macOS")
    binaries = args.bundle.resolve() / "Contents/MacOS" if args.bundle else ROOT / "target/release"
    for name in ["miaotty", "miaotty-cli"]:
        if not (binaries / name).exists():
            parser.error("build release binaries first; see docs/UI-AUDIT.md")
    output = Path(tempfile.mkdtemp(prefix="miaotty-smoke-"))
    print(f"Local artifacts: {output}", flush=True)
    reports = [smoke(output, args.bundle.resolve() if args.bundle else None)]
    (output / "report.json").write_text(json.dumps(reports, indent=2) + "\n")
    print(json.dumps(reports, indent=2))


if __name__ == "__main__":
    main()
