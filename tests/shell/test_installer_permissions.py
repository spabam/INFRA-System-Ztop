"""Exercise installer permission helpers against temporary build artifacts only.

Run: python3 -m unittest discover -s tests/shell -p test_installer_permissions.py -v
The installer is sourced; its installation entrypoints are never called.
"""

import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[2] / "installer.sh"
SHELL_SETUP = 'source "$1"\nHERE="$2"\ncd -- "$HERE"\nshift 2\n'


class InstallerPermissions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ztop permissions ")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.checkout = self.base / "fixture checkout"
        self.checkout.mkdir()
        # The installer builds outside the checkout by default, under
        # $XDG_CACHE_HOME/ztop-target; the fixture points that at a temp dir.
        self.cache = self.base / "fixture cache"
        self.target = self.cache / "ztop-target"
        self.executed = self.base / "artifact executed"

    def artifact(self, path, mode=0o664):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#!/bin/sh\nprintf executed > "$FIXTURE_EXECUTED"\n')
        path.chmod(mode)
        return path

    def run_helpers(self, body, *args, target_dir=None):
        env = os.environ.copy()
        env.pop("CARGO_TARGET_DIR", None)
        env["HOME"] = str(self.base / "fixture home")
        env["XDG_CACHE_HOME"] = str(self.cache)
        env["FIXTURE_EXECUTED"] = str(self.executed)
        if target_dir is not None:
            env["CARGO_TARGET_DIR"] = str(target_dir)
        return subprocess.run(
            ["bash", "-c", SHELL_SETUP + body, "test", str(INSTALLER),
             str(self.checkout), *(str(arg) for arg in args)],
            env=env, text=True, capture_output=True, timeout=10,
        )

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(self.executed.exists(), "permission repair executed an artifact")

    def mode(self, path):
        return stat.S_IMODE(path.stat().st_mode)

    def test_restores_cached_script_and_hardlink_without_changing_other_files(self):
        build_dir = self.target / "release/build/quote cached"
        script = self.artifact(build_dir / "build-script-build")
        alias = build_dir / "build_script_build-a1b2c3"
        os.link(script, alias)
        untouched = [
            self.artifact(build_dir / "build_script_build-a1b2c3.d"),
            self.artifact(build_dir / "output"),
            self.artifact(build_dir / "out/build-script-build"),
            self.artifact(self.target / "debug/build/quote/build-script-build"),
        ]
        original_contents = {path: path.read_bytes() for path in [script, *untouched]}

        result = self.run_helpers("ensure_executable_build_scripts")

        self.assert_success(result)
        self.assertIn("restoring execute permission", result.stdout)
        self.assertEqual(self.mode(script), 0o764)
        self.assertEqual(self.mode(alias), 0o764)
        self.assertEqual(script.stat().st_ino, alias.stat().st_ino)
        for path in untouched:
            self.assertEqual(self.mode(path), 0o664, str(path))
        for path, content in original_contents.items():
            self.assertEqual(path.read_bytes(), content, str(path))

    def test_default_build_directory_is_outside_the_checkout(self):
        def resolve(**overrides):
            env = {key: value for key, value in os.environ.items()
                   if key not in ("CARGO_TARGET_DIR", "HOME", "XDG_CACHE_HOME")}
            env.update(overrides)
            result = subprocess.run(
                ["bash", "-c", SHELL_SETUP + "cargo_target_dir", "test",
                 str(INSTALLER), str(self.checkout)],
                env=env, text=True, capture_output=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stderr, "")
            return result.stdout.rstrip("\n")

        home = self.base / "fixture home"
        self.assertEqual(resolve(HOME=str(home), XDG_CACHE_HOME=str(self.cache)), str(self.target))
        self.assertEqual(resolve(HOME=str(home)), str(home / ".cache/ztop-target"))
        # XDG_CACHE_HOME must be absolute; a relative value is ignored.
        self.assertEqual(resolve(HOME=str(home), XDG_CACHE_HOME="relative cache"),
                         str(home / ".cache/ztop-target"))
        self.assertEqual(resolve(HOME=str(home), CARGO_TARGET_DIR="custom target"), "custom target")
        # Without any home directory, fall back to cargo's own default.
        self.assertEqual(resolve(), str(self.checkout / "target"))
        self.assertFalse((self.checkout / "target").exists())
        self.assertFalse(self.cache.exists())

    def test_missing_or_empty_build_directories_are_noops(self):
        for layout in (None, "", "release", "release/build"):
            with self.subTest(layout=layout):
                if layout is not None:
                    (self.target / layout).mkdir(parents=True, exist_ok=True)
                result = self.run_helpers("ensure_executable_build_scripts")
                self.assert_success(result)
                self.assertEqual(result.stdout, "")
                self.assertEqual(result.stderr, "")

    def test_executable_cached_script_needs_no_chmod(self):
        script = self.artifact(self.target / "release/build/quote/build-script-build", 0o751)
        result = self.run_helpers(
            "chmod() { printf UNEXPECTED_CHMOD; return 42; }\n"
            "ensure_executable_build_scripts"
        )
        self.assert_success(result)
        self.assertEqual(result.stdout, "")
        self.assertEqual(self.mode(script), 0o751)

    def test_absolute_and_relative_custom_target_directories(self):
        default_script = self.artifact(self.target / "release/build/quote/build-script-build")
        for target_dir in (self.base / "absolute target", Path("relative target")):
            with self.subTest(target_dir=target_dir):
                resolved = target_dir if target_dir.is_absolute() else self.checkout / target_dir
                script = self.artifact(resolved / "release/build/quote/build-script-build")
                result = self.run_helpers("ensure_executable_build_scripts", target_dir=target_dir)
                self.assert_success(result)
                self.assertEqual(self.mode(script), 0o764)
                self.assertEqual(self.mode(default_script), 0o664)

    def test_chmod_failure_stops_before_subsequent_work(self):
        script = self.artifact(self.target / "release/build/quote/build-script-build")
        result = self.run_helpers(
            "chmod() { return 42; }\n"
            "ensure_executable_build_scripts\n"
            "printf UNEXPECTED_CONTINUATION"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("UNEXPECTED_CONTINUATION", result.stdout)
        self.assertIn(str(script), result.stderr)
        self.assertIn("cannot restore execute permission", result.stderr)
        self.assertEqual(self.mode(script), 0o664)
        self.assertFalse(self.executed.exists())

    def test_permissions_are_rechecked_after_chmod(self):
        script = self.artifact(self.target / "release/build/quote/build-script-build")
        result = self.run_helpers(
            "chmod() { return 0; }\n"
            'ensure_executable_artifact "$1"\n'
            "printf UNEXPECTED_CONTINUATION", script,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("UNEXPECTED_CONTINUATION", result.stdout)
        self.assertIn("noexec", result.stderr)
        self.assertFalse(self.executed.exists())

    def test_release_binary_permission_is_restored_without_execution(self):
        binary = self.artifact(self.target / "release/ztop")
        result = self.run_helpers('ensure_executable_artifact "$1"', binary)
        self.assert_success(result)
        self.assertEqual(self.mode(binary), 0o764)

    def test_missing_artifact_is_a_noop(self):
        result = self.run_helpers('ensure_executable_artifact "$1"', self.target / "release/ztop")
        self.assert_success(result)
        self.assertEqual(result.stdout, "")
        self.assertFalse(self.target.exists())

    def test_nonregular_artifacts_and_symlinks_are_refused(self):
        regular = self.artifact(self.base / "regular artifact")
        directory = self.base / "directory artifact"
        directory.mkdir()
        fifo = self.base / "fifo artifact"
        os.mkfifo(fifo)
        symlink = self.base / "symlink artifact"
        symlink.symlink_to(regular)
        dangling = self.base / "dangling artifact"
        dangling.symlink_to(self.base / "missing artifact")
        for path in (directory, fifo, symlink, dangling):
            with self.subTest(path=path):
                result = self.run_helpers(
                    'ensure_executable_artifact "$1"\nprintf UNEXPECTED_CONTINUATION', path,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("UNEXPECTED_CONTINUATION", result.stdout)
                self.assertIn("regular build artifact", result.stderr)
                self.assertIn(str(path), result.stderr)
        self.assertEqual(self.mode(regular), 0o664)
        self.assertFalse(self.executed.exists())


if __name__ == "__main__":
    unittest.main()
