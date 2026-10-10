"""Choose expensive CI checks from the paths changed by a PR or push."""
import os
import subprocess


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
    for key, value in ({key: True for key in ("code", "dependencies", "render", "perf")} if full else classify(paths)).items():
        print(f"{key}={str(value).lower()}")
