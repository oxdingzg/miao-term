#!/usr/bin/env python3
"""Exercise update selection and incomplete/signed release failure modes."""

import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import sys

sys.dont_write_bytecode = True

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
                     "miaotty-linux-x86_64.tar.gz", "miaotty_0.0.1-1_amd64.deb",
                     "miaotty-windows-x86_64.zip"):
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

    def test_nested_uploads_are_collected_before_signing_and_selection(self):
        nested = self.directory / "dist"
        nested.mkdir()
        deb = self.directory / "miaotty_0.0.1-1_amd64.deb"
        deb.rename(nested / deb.name)
        wix = self.directory / "target/wix"
        wix.mkdir(parents=True)
        msi = wix / "miaotty-app-0.0.1-x86_64.msi"
        msi.write_bytes(b"MSI payload")
        module.collect_artifacts(self.directory)
        artifacts = self.build()["artifacts"]
        self.assertTrue(artifacts["windows-x86_64"]["url"].endswith(".msi"))
        self.assertTrue(artifacts["linux-x86_64-deb"]["url"].endswith(".deb"))
        self.assertEqual((self.directory / msi.name).read_bytes(), b"MSI payload")
        self.assertFalse(nested.exists())
        self.assertFalse(wix.parent.exists())

    def test_collect_rejects_collisions_without_moving_payloads(self):
        nested = self.directory / "dist"
        nested.mkdir()
        duplicate = nested / "miaotty-macos-arm64.zip"
        duplicate.write_bytes(b"different payload")
        with self.assertRaisesRegex(ValueError, "duplicate artifact filename"):
            module.collect_artifacts(self.directory)
        self.assertEqual(duplicate.read_bytes(), b"different payload")
        self.assertEqual((self.directory / duplicate.name).read_bytes(), b"artifact")


class AppRunTests(unittest.TestCase):
    def test_launch_preserves_arguments_directory_path_and_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "bundle with spaces"
            binaries = bundle / "usr/bin"
            binaries.mkdir(parents=True)
            launcher = bundle / "AppRun"
            shutil.copyfile(Path(__file__).parent.parent / "assets/AppRun", launcher)
            host = binaries / "miaotty"
            host.write_text('#!/bin/sh\nprintf "%s\\n" "$PWD" "$@"\ncommand -v miaotty-cli\nexit 7\n')
            host.chmod(0o755)
            cli = binaries / "miaotty-cli"
            cli.write_text("#!/bin/sh\nexit 0\n")
            cli.chmod(0o755)
            working_directory = root / "working directory"
            working_directory.mkdir()
            arguments = ["miaotty://quick", "argument with spaces"]
            result = subprocess.run(
                ["sh", str(launcher), *arguments], cwd=working_directory,
                env=dict(os.environ, PATH="/usr/bin:/bin"), capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 7, result.stderr)
            self.assertEqual(result.stdout.splitlines(), [str(working_directory.resolve()), *arguments, str(cli)])


if __name__ == "__main__":
    unittest.main()
