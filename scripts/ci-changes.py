"""Choose expensive CI checks from the paths changed by a PR or push."""
import os
import subprocess


def packaging(paths):
    return any(
        path.endswith(("Cargo.toml", "Cargo.lock"))
        or path.startswith(("assets/", "mtty-app/wix/"))
        or path in (
            ".github/workflows/release.yml", "mtty-app/build.rs", "THIRD-PARTY-LICENSES.md",
            "scripts/check-release-ci.py", "scripts/test_release_ci.py", "scripts/package-macos.sh",
            "scripts/check-macos-bundle.py", "scripts/check-windows-icon.py",
            "scripts/build-update-manifest.py", "scripts/render-third-party-licenses.py", "scripts/make-icon.py",
        )
        for path in paths
    )


def classify(paths):
    code = any(not (path.startswith("docs/") or ("/" not in path and path.endswith((".md", ".txt")))) for path in paths)
    dependencies = any(path.endswith(("Cargo.toml", "Cargo.lock")) or path in ("deny.toml", ".github/workflows/ci.yml", "scripts/ci-changes.py") for path in paths)
    render = dependencies or any(path.startswith(("crates/mtty-render/", "crates/mtty-core/", "crates/mtty-graphics/", "crates/mtty-widget/", "crates/mtty-ui/", "crates/mtty-platform/", "mtty-app/", "vendor/", "assets/")) for path in paths)
    perf = dependencies or any(path.startswith(("crates/", "mtty-app/", "mtty-cli/", "vendor/", "benches/", ".cargo/")) or path == "scripts/check-perf-baseline.py" for path in paths)
    return {"code": code, "dependencies": dependencies, "render": render, "perf": perf}


if __name__ == "__main__":
    event = os.environ["EVENT_NAME"]
    base = os.environ.get("BASE_SHA", "")
    full = event == "schedule" or (event == "workflow_dispatch" and os.environ.get("FULL", "true") == "true") or not base or set(base) == {"0"}
    revisions = [f"{base}...{os.environ['HEAD_SHA']}"] if event == "pull_request" else [base, os.environ.get("HEAD_SHA", "HEAD")]
    paths = [] if full else [path for path in subprocess.check_output(["git", "diff", "--name-only", "-z", *revisions], text=True).split("\0") if path]
    flags = {key: True for key in ("code", "dependencies", "render", "perf")} if full else classify(paths)
    flags["packaging"] = full or packaging(paths)
    for key, value in flags.items():
        print(f"{key}={str(value).lower()}")
