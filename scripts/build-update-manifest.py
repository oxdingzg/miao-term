#!/usr/bin/env python3
"""Build and validate the update manifest from the assembled release artifacts."""

import argparse
import hashlib
import json
from pathlib import Path
import re


# Preference is explicit: installable artifacts precede archive fallbacks.
PATTERNS = {
    "macos-aarch64": [r"miaotty-macos-arm64\.zip"],
    "macos-x86_64": [r"miaotty-macos-x86_64\.zip"],
    "linux-x86_64": [r"miaotty-linux-x86_64\.AppImage", r"miaotty-linux-x86_64\.tar\.gz"],
    "linux-x86_64-deb": [r"miaotty_.*_amd64\.deb"],
    "windows-x86_64": [r"miaotty-.*\.msi", r"miaotty-windows-x86_64\.zip"],
}
REQUIRED = ("macos-aarch64", "macos-x86_64", "linux-x86_64", "windows-x86_64")


def build_manifest(directory, tag, repository, require_signatures=False):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", tag):
        raise ValueError("tag must be a version such as v0.0.1")
    files = sorted(path for path in directory.iterdir() if path.is_file())
    base = f"https://github.com/{repository}/releases/download/{tag}"
    artifacts = {}
    for platform, patterns in PATTERNS.items():
        for pattern in patterns:
            matches = [path for path in files if re.fullmatch(pattern, path.name)]
            if len(matches) > 1:
                raise ValueError(f"ambiguous artifacts for {platform}: {matches}")
            if not matches:
                continue
            path = matches[0]
            with path.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
            entry = {"url": f"{base}/{path.name}", "sha256": digest}
            signature = path.with_name(path.name + ".sig")
            if signature.is_file():
                entry["signature"] = f"{base}/{signature.name}"
            elif require_signatures:
                raise ValueError(f"missing signature for {path.name}")
            artifacts[platform] = entry
            break
    missing = set(REQUIRED) - artifacts.keys()
    if missing:
        raise ValueError(f"missing platform artifacts: {', '.join(sorted(missing))}")
    return {"version": tag.removeprefix("v"), "artifacts": artifacts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--require-signatures", action="store_true")
    args = parser.parse_args()
    manifest = build_manifest(args.directory, args.tag, args.repository, args.require_signatures)
    text = json.dumps(manifest, indent=2) + "\n"
    (args.directory / "latest.json").write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
