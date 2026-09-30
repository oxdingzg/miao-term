#!/usr/bin/env python3
"""Exercise update selection and incomplete/signed release failure modes."""

import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("manifest", Path(__file__).with_name("build-update-manifest.py"))
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        for name in ("miaotty-macos-arm64.zip", "miaotty-macos-x86_64.zip",
                     "miaotty-linux-x86_64.tar.gz", "miaotty-windows-x86_64.zip"):
            (self.directory / name).write_bytes(b"artifact")

    def build(self, signed=False):
        return module.build_manifest(self.directory, "v0.0.1", "example/project", signed)

    def test_archive_fallback_and_digest(self):
        manifest = self.build()
        self.assertEqual(manifest["version"], "0.0.1")
        linux = manifest["artifacts"]["linux-x86_64"]
        self.assertTrue(linux["url"].endswith(".tar.gz"))
        self.assertEqual(linux["sha256"], hashlib.sha256(b"artifact").hexdigest())

    def test_installers_preferred_and_native_zip_not_selected(self):
        for name in ("miaotty-linux-x86_64.AppImage", "miaotty-app-0.0.1-x86_64.msi",
                     "miaotty-native-macos-arm64.zip"):
            (self.directory / name).write_bytes(b"installer")
        artifacts = self.build()["artifacts"]
        self.assertTrue(artifacts["linux-x86_64"]["url"].endswith(".AppImage"))
        self.assertTrue(artifacts["windows-x86_64"]["url"].endswith(".msi"))
        self.assertTrue(artifacts["macos-aarch64"]["url"].endswith("miaotty-macos-arm64.zip"))

    def test_missing_platform_rejected(self):
        (self.directory / "miaotty-macos-x86_64.zip").unlink()
        with self.assertRaisesRegex(ValueError, "missing platform artifacts"):
            self.build()

    def test_signatures_required_and_attached(self):
        with self.assertRaisesRegex(ValueError, "missing signature"):
            self.build(signed=True)
        for artifact in list(self.directory.iterdir()):
            artifact.with_name(artifact.name + ".sig").write_text("signature")
        for entry in self.build(signed=True)["artifacts"].values():
            self.assertEqual(entry["signature"], entry["url"] + ".sig")


if __name__ == "__main__":
    unittest.main()
