"""Require successful main push workflows for the exact release commit."""
import argparse
import json
import os
import subprocess


def require_success(runs, sha, workflow):
    matching = [run for run in runs if run["head_sha"] == sha and run["head_branch"] == "main" and run["event"] == "push"]
    if not matching:
        raise ValueError(f"{workflow}: no main push run for {sha}")
    latest = max(matching, key=lambda run: run["id"])
    if latest["status"] != "completed" or latest["conclusion"] != "success":
        raise ValueError(f"{workflow}: {latest['status']}/{latest['conclusion']} ({latest['html_url']})")
    return latest["html_url"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("workflows", nargs="+")
    args = parser.parse_args()
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"], check=True)
    repo = os.environ["GITHUB_REPOSITORY"]
    for workflow in args.workflows:
        pages = json.loads(subprocess.check_output([
            "gh", "api", "--paginate", "--slurp",
            f"repos/{repo}/actions/workflows/{workflow}/runs?head_sha={sha}&branch=main&event=push&per_page=100",
        ], text=True))
        runs = [run for page in pages for run in page["workflow_runs"]]
        print(f"{workflow}: {require_success(runs, sha, workflow)}")


if __name__ == "__main__":
    main()
