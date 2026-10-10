import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("release_ci", Path(__file__).with_name("check-release-ci.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def run(**fields):
    return {"id": 1, "head_sha": "release-sha", "head_branch": "main", "event": "push", "status": "completed", "conclusion": "success", "html_url": "https://example.invalid/run/1", **fields}


class ReleaseCITest(unittest.TestCase):
    def test_exact_main_push_passes(self):
        self.assertEqual(release.require_success([run()], "release-sha", "test.yml"), "https://example.invalid/run/1")

    def test_missing_wrong_sha_branch_or_event_fails(self):
        for runs in [[], [run(head_sha="other")], [run(head_branch="feature")], [run(event="pull_request")]]:
            with self.assertRaisesRegex(ValueError, "no main push run"):
                release.require_success(runs, "release-sha", "test.yml")

    def test_unfinished_or_failed_run_cannot_publish(self):
        for status, conclusion in [("queued", None), ("in_progress", None), ("completed", "failure"), ("completed", "cancelled"), ("completed", "skipped")]:
            with self.assertRaises(ValueError):
                release.require_success([run(status=status, conclusion=conclusion)], "release-sha", "test.yml")

    def test_new_run_supersedes_an_older_success(self):
        with self.assertRaises(ValueError):
            release.require_success([run(), run(id=2, status="in_progress", conclusion=None)], "release-sha", "test.yml")
        self.assertEqual(release.require_success([run(conclusion="failure"), run(id=2)], "release-sha", "test.yml"), "https://example.invalid/run/1")


if __name__ == "__main__":
    unittest.main()
