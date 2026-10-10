"""Enforce the same conditions that launch each CI job, including packaging."""
import json
import os

JOBS = {"changes", "privacy", "lint", "deny", "test-linux", "macos", "windows", "render-linux", "perf", "packaging"}


def enforce(needs, event):
    if needs.keys() != JOBS:
        raise ValueError(f"Unexpected gate jobs: {sorted(needs.keys() ^ JOBS)}")
    flags = needs["changes"]["outputs"]
    required = {"changes", "privacy"}
    if flags.get("perf") == "true":
        required.update(("lint", "test-linux", "macos"))
        if event != "push":
            required.add("windows")
    if flags.get("dependencies") == "true":
        required.add("deny")
    if event != "push":
        if flags.get("render") == "true":
            required.add("render-linux")
        if flags.get("perf") == "true":
            required.add("perf")
        if flags.get("packaging") == "true":
            required.add("packaging")
    for job, info in needs.items():
        expected = "success" if job in required else "skipped"
        if info["result"] != expected:
            raise ValueError(f"{job}: expected {expected}, got {info['result']}")


if __name__ == "__main__":
    enforce(json.loads(os.environ["NEEDS"]), os.environ["EVENT_NAME"])
    print("Required checks passed; unrelated checks were explicitly skipped.")
