#!/usr/bin/env python3
"""Verify the single mtty app bundle and its native executable identity."""
import argparse
from pathlib import Path
import plistlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("bundle", type=Path)
args = parser.parse_args()
bundle = args.bundle.resolve()
assert bundle.name == "mtty.app", "the public application must be mtty.app"
info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
assert info["CFBundleIdentifier"] == "dev.mtty.terminal"
assert info["CFBundleExecutable"] == "mtty"
assert info["CFBundleDisplayName"] == "mtty"
schemes = {s for t in info.get("CFBundleURLTypes", []) for s in t.get("CFBundleURLSchemes", [])}
# The former scheme stays registered during the rename transition (ADR 0032).
assert {"mtty", "miaotty", "ssh", "x-man-page"} <= schemes, schemes
executables = bundle / "Contents/MacOS"
assert {p.name for p in executables.iterdir()} == {"mtty", "mtty-cli", "mtty-ptyhost"}
version = subprocess.check_output([str(executables / "mtty"), "--version"], text=True).strip()
assert version == f"mtty {info['CFBundleShortVersionString']} (native)", version
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
print(f"verified single application: {version}")
