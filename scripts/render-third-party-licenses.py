#!/usr/bin/env python3
"""Render cargo-bundle-licenses output as a readable THIRD-PARTY-LICENSES.md.

cargo-bundle-licenses 4.x writes machine-readable TOML (or JSON/YAML); cargo-dist
normally renders that into a licence file for a release. mtty ships a rendered
file with every artifact, and this is the renderer.

    cargo bundle-licenses --format toml --output - | \\
        python3 scripts/render-third-party-licenses.py > THIRD-PARTY-LICENSES.md

Arguments: an input path, or nothing to read stdin.
"""

import sys
import tomllib
from collections import defaultdict

INPUT = sys.argv[1] if len(sys.argv) > 1 else "-"

with open(INPUT, "rb") as handle:
    data = tomllib.load(handle)

libraries = data.get("third_party_libraries", [])
if not libraries:
    sys.exit("no third_party_libraries in the input")

# licence id -> {text: [crate, ...]}, so a text shared by several crates is
# written once, and a licence used with different copyright lines keeps each.
by_licence: dict[str, dict[str, list[str]]] = defaultdict(lambda: defaultdict(list))
for library in libraries:
    name = f"{library['package_name']} {library['package_version']}"
    entries = library.get("licenses") or [{"license": library.get("license", "?"), "text": ""}]
    for entry in entries:
        # Several crates ship a licence file with CRLF endings. Normalising here
        # keeps the output byte-identical to a fresh generation after git has
        # checked it out, which is what the release comparison depends on.
        text = (entry.get("text") or "").replace("\r\n", "\n").replace("\r", "\n").strip()
        by_licence[entry.get("license") or "?"][text].append(name)

libraries.sort(key=lambda item: (item["package_name"], item["package_version"]))

out = [
    "# Third-party licences",
    "",
    "mtty itself is Apache-2.0 — see [`LICENSE`](../LICENSE). It is built on the",
    f"crates below, {len(libraries)} of them, each under its own licence. The licence",
    "texts follow the table, once per distinct text.",
    "",
    "*Generated from `Cargo.lock` by [`scripts/render-third-party-licenses.py`](../scripts/render-third-party-licenses.py),*",
    "*which runs `cargo bundle-licenses`. Do not edit by hand: a release compares*",
    "*a fresh generation against this file and stops if they differ.*",
    "",
    "## Crates",
    "",
    "| Crate | Licence | Upstream |",
    "|---|---|---|",
]
for library in libraries:
    name = f"{library['package_name']} {library['package_version']}"
    licence = (library.get("license") or "?").replace("|", "\\|")
    repository = library.get("repository") or ""
    link = f"[{repository.split('//')[-1]}]({repository})" if repository.startswith("http") else ""
    out.append(f"| {name} | {licence} | {link} |")

def crate_list(crates: list[str]) -> str:
    """A readable list: named while it is short, counted once it is not."""
    names = sorted(crates)
    if len(names) <= 8:
        return "This exact text is filed by " + ", ".join(names) + "."
    shown = ", ".join(names[:4])
    return f"This exact text is filed by {len(names)} crates, among them {shown}, … {names[-1]}."


out += ["", "## Licence texts", ""]
for licence in sorted(by_licence):
    texts = by_licence[licence]
    out += [f"### {licence}", ""]
    if len(texts) == 1:
        # One text for every crate that names this licence — Apache-2.0 is the
        # usual case. Listing the crates would say nothing the table does not.
        text = next(iter(texts))
        out.append("Every crate above that names this licence is covered by this text.")
        out.append("")
        out.append("```text")
        out.append(text or "(no licence file found in the crate; see its repository)")
        out.append("```")
        out.append("")
        continue
    # Several texts under one identifier: in practice MIT, where each crate
    # carries its own copyright line.
    for text, crates in sorted(texts.items(), key=lambda kv: sorted(kv[1])[0]):
        out.append(crate_list(crates))
        out.append("")
        out.append("```text")
        out.append(text or "(no licence file found in the crate; see its repository)")
        out.append("```")
        out.append("")

sys.stdout.write("\n".join(out).rstrip() + "\n")
