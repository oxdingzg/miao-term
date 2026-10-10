import importlib.util
from pathlib import Path
import unittest


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gate = load("ci-gate")
changes = load("ci-changes")


class GateTest(unittest.TestCase):
    def needs(self, paths, successful):
        flags = {key: str(value).lower() for key, value in changes.classify(paths).items()}
        flags["packaging"] = str(changes.packaging(paths)).lower()
        needs = {job: {"result": "success" if job in successful else "skipped"} for job in gate.JOBS}
        needs["changes"]["outputs"] = flags
        return needs

    def test_docs_allow_explicit_skips(self):
        gate.enforce(self.needs(["README.md"], {"changes", "privacy"}), "pull_request")

    def test_packaging_only_pr_accepts_successful_rehearsal(self):
        needs = self.needs(["scripts/package-macos.sh"], {"changes", "privacy", "packaging"})
        gate.enforce(needs, "pull_request")
        needs["packaging"]["result"] = "skipped"
        with self.assertRaisesRegex(ValueError, "packaging: expected success"):
            gate.enforce(needs, "pull_request")

    def test_icon_pr_requires_render_and_packaging(self):
        gate.enforce(self.needs(["assets/icons/mtty.ico"], {"changes", "privacy", "render-linux", "packaging"}), "pull_request")

    def test_dependency_pr_requires_all_jobs(self):
        gate.enforce(self.needs(["Cargo.lock"], gate.JOBS), "pull_request")

    def test_push_skips_pr_and_nightly_only_jobs(self):
        gate.enforce(self.needs(["Cargo.lock"], {"changes", "privacy", "lint", "deny", "test-linux", "macos"}), "push")

    def test_failed_cancelled_or_missing_jobs_fail_closed(self):
        for job in gate.JOBS:
            for result in ["failure", "cancelled", "abandoned", "skipped"]:
                needs = self.needs(["Cargo.lock"], gate.JOBS)
                needs[job]["result"] = result
                with self.assertRaises(ValueError):
                    gate.enforce(needs, "pull_request")
            del needs[job]
            with self.assertRaises(ValueError):
                gate.enforce(needs, "pull_request")


if __name__ == "__main__":
    unittest.main()
