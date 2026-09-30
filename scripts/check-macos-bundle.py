#!/usr/bin/env python3
"""Verify the single miaotty app bundle and its native executable identity."""
import argparse
from pathlib import Path
import plistlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("bundle", type=Path)
args = parser.parse_args()
bundle = args.bundle.resolve()
assert bundle.name == "miaotty.app", "the public application must be miaotty.app"
info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
assert info["CFBundleIdentifier"] == "io.miaotty.terminal"
assert info["CFBundleExecutable"] == "miaotty"
assert info["CFBundleDisplayName"] == "miaotty"
executables = bundle / "Contents/MacOS"
assert {p.name for p in executables.iterdir()} == {"miaotty", "miaotty-cli"}
version = subprocess.check_output([str(executables / "miaotty"), "--version"], text=True).strip()
assert version == f"miaotty {info['CFBundleShortVersionString']} (native)", version
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
print(f"verified single application: {version}")
