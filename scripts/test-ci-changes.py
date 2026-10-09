import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("ci_changes", Path(__file__).with_name("ci-changes.py"))
changes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(changes)


class ChangesTest(unittest.TestCase):
    def test_docs_and_empty_diff(self):
        for paths in [[], ["README.md", "docs/guide.md"]]:
            self.assertFalse(any(changes.classify(paths).values()))

    def test_dependency_changes_require_all_checks(self):
        for path in ["Cargo.lock", "crates/mtty-core/Cargo.toml", ".github/workflows/ci.yml"]:
            self.assertTrue(all(changes.classify([path]).values()))

    def test_terminal_changes_require_tests_and_performance(self):
        self.assertEqual(changes.classify(["crates/mtty-ptyhost/src/lib.rs"]), {"code": True, "dependencies": False, "render": False, "perf": True})

    def test_render_changes_require_render_and_performance(self):
        self.assertEqual(changes.classify(["crates/mtty-render/src/lib.rs"]), {"code": True, "dependencies": False, "render": True, "perf": True})

    def test_release_script_avoids_unrelated_expensive_checks(self):
        self.assertEqual(changes.classify(["scripts/release-notes.py"]), {"code": True, "dependencies": False, "render": False, "perf": False})

    def test_asset_changes_run_render_only(self):
        # Assets compile nothing: `code` is true, but the perf-gated jobs must
        # not be required, so an icon-only PR is checked by render-linux alone.
        self.assertEqual(changes.classify(["assets/icons/mtty.ico"]), {"code": True, "dependencies": False, "render": True, "perf": False})


if __name__ == "__main__":
    unittest.main()
