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

    def test_cli_and_build_configuration_require_compilation(self):
        for path in ["mtty-cli/src/main.rs", ".cargo/config.toml"]:
            self.assertTrue(changes.classify([path])["perf"])

    def test_patched_dependencies_require_tests_render_and_performance(self):
        for path in ["vendor/winit/src/event_loop.rs", "vendor/muda/src/lib.rs", "vendor/egui_commonmark/src/lib.rs"]:
            self.assertEqual(changes.classify([path]), {"code": True, "dependencies": False, "render": True, "perf": True})

    def test_every_workspace_member_runs_compilation(self):
        # Discover members from the real tree so a newly added workspace cannot
        # silently inherit a successful gate with all compilation jobs skipped.
        root = Path(__file__).resolve().parent.parent
        for manifest in [*root.glob("crates/*/Cargo.toml"), root / "mtty-app/Cargo.toml", root / "mtty-cli/Cargo.toml"]:
            source = str(manifest.parent.relative_to(root) / "src/lib.rs")
            self.assertTrue(changes.classify([source])["perf"], source)

    def test_release_script_avoids_unrelated_expensive_checks(self):
        self.assertEqual(changes.classify(["scripts/release-notes.py"]), {"code": True, "dependencies": False, "render": False, "perf": False})

    def test_asset_changes_run_render_only(self):
        # Assets compile nothing: `code` is true, but the perf-gated jobs must
        # not be required, so an icon-only PR is checked by render-linux alone.
        self.assertEqual(changes.classify(["assets/icons/mtty.ico"]), {"code": True, "dependencies": False, "render": True, "perf": False})


if __name__ == "__main__":
    unittest.main()
