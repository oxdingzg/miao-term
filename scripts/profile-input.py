#!/usr/bin/env python3
"""Profile mtty's real PTY input on macOS with isolated state.

Build release binaries first. The test app hides its own window before sending
input, so desktop typing cannot corrupt the shell command. Artifacts (including
sample's machine-specific report) stay in a temporary directory, outside the repo.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import shlex
import struct
import subprocess
import sys
import tempfile
import time
import zlib

ROOT = Path(__file__).resolve().parent.parent


def usage(pid):
    words = subprocess.check_output(
        ["ps", "-p", str(pid), "-o", "time=,rss="], text=True
    ).split()
    minutes, seconds = words[0].split(":")
    return int(minutes) * 60 + float(seconds), int(words[1]) / 1024


def png_chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", zlib.crc32(kind + data)))


def fixture(case, mode):
    if mode == "fragments":
        # Deliberately fragmented, invalid image payload: isolates framing/scans,
        # not PNG decode cost. The sender's sleep gives a throughput lower bound.
        return r"""import os,time
os.write(1,b'\x1b]1337;File=:')
for _ in range(8192):
    os.write(1,b'A'*512)
    time.sleep(.0002)
os.write(1,b'\x1b\\\r\nPROFILE_DONE\r\n')
open('done','w').write('ok')
"""
    # Small encoded PNGs with 4 MiB decoded RGBA each: exercises retained pixels
    # and byte-based eviction without making the scanner dominate the workload.
    raw = (b"\x00" + b"\xff\x00\x00\xff" * 1024) * 1024
    png = (b"\x89PNG\r\n\x1a\n"
           + png_chunk(b"IHDR", struct.pack(">2I5B", 1024, 1024, 8, 6, 0, 0, 0))
           + png_chunk(b"IDAT", zlib.compress(raw)) + png_chunk(b"IEND", b""))
    (case / "image.b64").write_bytes(base64.b64encode(png))
    return r"""import os
data=open('image.b64','rb').read()
for i in range(40):
    os.write(1,b'\x1b_Ga=T,f=100,i='+str(i).encode()+b';'+data+b'\x1b\\')
open('done','w').write('ok')
"""


def profile(binary, cli, mode):
    case = Path(tempfile.mkdtemp(prefix="mtty-input-profile-"))
    print(f"Local artifacts: {case}", flush=True)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("MTTY_", "MIAOTTY_"))}
    env.update(HOME=str(case), XDG_CONFIG_HOME=str(case / "config"),
               XDG_DATA_HOME=str(case / "data"), XDG_RUNTIME_DIR=str(case),
               SHELL="/bin/sh")
    (case / "workload.py").write_text(fixture(case, mode))

    def call(*args):
        result = subprocess.run(
            [str(cli), "--socket", str(case / "mtty.sock"), *args],
            env=env, capture_output=True, text=True, timeout=5)
        return json.loads(result.stdout) if result.returncode == 0 else None

    sampler = None
    with (case / "host.log").open("w") as log:
        app = subprocess.Popen([str(binary)], env=env, cwd=case, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 15
            panes = None
            while not panes and time.monotonic() < deadline:
                if app.poll() is not None:
                    raise RuntimeError("host exited; see host.log")
                panes = (call("pane", "list") or {}).get("panes")
                time.sleep(.1)
            if not panes:
                raise TimeoutError("host did not advertise a pane")
            subprocess.run(
                ["swift", "-e", "import AppKit; "
                 f"NSRunningApplication(processIdentifier: {app.pid})?.hide()"],
                check=True, capture_output=True, timeout=20)
            time.sleep(2)
            cpu0, _ = usage(app.pid)
            time.sleep(3)
            cpu1, rss1 = usage(app.pid)
            command = "\x15" + shlex.quote(sys.executable) + " workload.py"
            if not call("pane", "run", "--pane", panes[0]["id"], "--data", command):
                raise RuntimeError("pane run failed")
            start = time.monotonic()
            peak = rss1
            sampler = subprocess.Popen(
                ["sample", str(app.pid), "1", "-file", str(case / "sample.txt")],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            while not (case / "done").exists():
                if time.monotonic() - start > 90:
                    raise TimeoutError("workload did not finish; see host.log")
                _, rss = usage(app.pid)
                peak = max(peak, rss)
                time.sleep(.05)
            elapsed = time.monotonic() - start
            # The producer marker means write completion, not UI/GPU completion.
            # Allow pending chunks, image uploads, and deferred frees to settle.
            time.sleep(4)
            cpu2, rss2 = usage(app.pid)
            sampler.wait(timeout=10)
            report = {
                "mode": mode,
                "idle_cpu_pct": round((cpu1 - cpu0) / 3 * 100, 2),
                "producer_completion_sec": round(elapsed, 3),
                "work_and_settle_cpu_sec": round(cpu2 - cpu1, 3),
                "initial_rss_mib": round(rss1, 2),
                "sampled_peak_rss_mib": round(max(peak, rss2), 2),
                "settled_rss_mib": round(rss2, 2),
                "rss_growth_mib": round(rss2 - rss1, 2),
            }
            (case / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            return report
        finally:
            if sampler and sampler.poll() is None:
                sampler.terminate()
                sampler.wait(timeout=5)
            if app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mtty")
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/mtty-cli")
    parser.add_argument("--mode", choices=["fragments", "images"], default="fragments")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this real-host CPU/RSS profile currently targets macOS")
    for binary in (args.binary, args.cli):
        if not binary.is_file():
            parser.error("build release binaries first")
    print(json.dumps(profile(args.binary.resolve(), args.cli.resolve(), args.mode), indent=2))


if __name__ == "__main__":
    main()
