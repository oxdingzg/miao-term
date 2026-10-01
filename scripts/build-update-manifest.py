#!/usr/bin/env python3
"""Build and validate the update manifest from the assembled release artifacts."""

import argparse
import hashlib
import json
from pathlib import Path
import re


# Preference is explicit: installable artifacts precede archive fallbacks.
PATTERNS = {
    "macos-aarch64": [r"mtty-macos-arm64\.zip"],
    "macos-x86_64": [r"mtty-macos-x86_64\.zip"],
    "linux-x86_64": [r"mtty-linux-x86_64\.AppImage", r"mtty-linux-x86_64\.tar\.gz"],
    "linux-x86_64-deb": [r"mtty_.*_amd64\.deb"],
    "windows-x86_64": [r"mtty-.*\.msi", r"mtty-windows-x86_64\.zip"],
}
# Names that must never be published again: the retired second app and the
# pre-rename application name (ADR 0032).
RETIRED = ("miaotty-native-", "miaotty-", "miaotty_")
REQUIRED = ("macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-x86_64-deb", "windows-x86_64")


def collect_artifacts(directory):
    """Flatten upload-artifact's preserved dist/ and target/wix/ directories."""
    packages = {}
    for path in sorted(directory.rglob("*")):
        if not path.is_file() or not path.name.endswith((".zip", ".tar.gz", ".AppImage", ".deb", ".msi", ".sig")):
            continue
        if path.name.startswith(RETIRED):
            raise ValueError(f"retired application artifact: {path.name}")
        if path.name in packages:
            raise ValueError(f"duplicate artifact filename: {path.name}")
        packages[path.name] = path
    # Validate the entire plan before moving any files.
    for name, source in packages.items():
        target = directory / name
        if source != target:
            source.rename(target)
    for path in sorted(directory.rglob("*"), key=lambda p: len(p.parts), reverse=True):
        if path.is_dir() and not any(path.iterdir()):
            path.rmdir()
    print(f"collected {len(packages)} artifact files")


def build_manifest(directory, tag, repository, require_signatures=False):
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", tag):
        raise ValueError("tag must be a version such as v0.0.1")
    files = sorted(path for path in directory.iterdir() if path.is_file())
    if any(path.name.startswith(RETIRED) for path in files):
        raise ValueError("retired application artifact in release")
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
    parser.add_argument("--tag")
    parser.add_argument("--repository")
    parser.add_argument("--collect-only", action="store_true")
    parser.add_argument("--require-signatures", action="store_true")
    args = parser.parse_args()
    if args.collect_only:
        collect_artifacts(args.directory)
        return
    if not args.tag or not args.repository:
        parser.error("--tag and --repository are required when building a manifest")
    manifest = build_manifest(args.directory, args.tag, args.repository, args.require_signatures)
    text = json.dumps(manifest, indent=2) + "\n"
    (args.directory / "latest.json").write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
