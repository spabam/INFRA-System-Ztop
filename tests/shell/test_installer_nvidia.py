"""Read-only NVIDIA installer checks with fixture command responses only.

Run: python3 -m unittest discover -s tests/shell -p test_installer_nvidia.py -v
No test queries hardware, installs packages, or invokes a real monitor.
"""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[2] / "installer.sh"
SETUP = r'''
source "$1"
pkg_install() { printf UNEXPECTED_PACKAGE_INSTALL >&2; exit 93; }
need_sudo() { printf UNEXPECTED_SUDO >&2; exit 94; }
nvidia_runtime_present() { return "${FIXTURE_RUNTIME:-1}"; }
nvidia-smi() { printf '%s\n' "$FIXTURE_SMI"; return "$FIXTURE_SMI_STATUS"; }
'''


class InstallerNvidia(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ztop gpu checks ")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.binary = self.base / "prefix/bin/ztop"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text(
            '#!/bin/bash\n'
            'if [ "$1" = --version ]; then printf "fixture ztop\\nfixture copyright\\n"; exit 0; fi\n'
            'printf "%s\\n" "$*" > "$FIXTURE_ARGS"\n'
            'printf "%s\\n" "$FIXTURE_DIAGNOSTIC" >&2\n'
            'printf "%s\\n" "$FIXTURE_SNAPSHOT"\n'
            'exit "${FIXTURE_ZTOP_STATUS:-0}"\n'
        )
        self.binary.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update({
            "PREFIX": str(self.base / "prefix"),
            "HOME": str(self.base / "home"),
            "FIXTURE_SMI": "NVIDIA fixture GPU",
            "FIXTURE_SMI_STATUS": "0",
            "FIXTURE_ARGS": str(self.base / "args"),
            "FIXTURE_DIAGNOSTIC": "",
        })
        (self.base / "home").mkdir()
        self.snapshot("fresh")

    def snapshot(self, *statuses):
        self.env["FIXTURE_SNAPSHOT"] = json.dumps({
            "schema_version": 2,
            "gpus": [
                {"index": i, "name": f"NVIDIA fixture {i}", "status": status}
                for i, status in enumerate(statuses)
            ],
            "procs": [],
        }, indent=2)

    def run_helpers(self, body, **env):
        return subprocess.run(
            ["bash", "-c", SETUP + body, "test", str(INSTALLER)],
            env={**self.env, **env}, text=True, capture_output=True, timeout=10,
        )

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("UNEXPECTED_", result.stdout + result.stderr)

    def no_smi(self):
        return r'''
command() {
    if [ "$*" = "-v nvidia-smi" ]; then return 1; fi
    builtin command "$@"
}
'''

    def test_all_nvidia_devices_reported_and_verified_without_packages(self):
        self.env["FIXTURE_SMI"] = "NVIDIA fixture 0\nNVIDIA fixture 1"
        self.snapshot("fresh", "fresh")
        result = self.run_helpers('check_nvidia\nverify_ztop_gpu "$TARGET"')
        self.assert_success(result)
        self.assertIn("NVIDIA detected: NVIDIA fixture 0", result.stdout)
        self.assertIn("NVIDIA detected: NVIDIA fixture 1", result.stdout)
        self.assertIn("2 device(s) with fresh telemetry", result.stdout)
        self.assertEqual((self.base / "args").read_text().strip(),
                         "--no-config --gpu --no-pid-gpu --no-temp --json --top 1")

    def test_mismatch_preserves_error_and_returns_failure(self):
        result = self.run_helpers("check_nvidia", FIXTURE_SMI_STATUS="18",
                                  FIXTURE_SMI="Failed to initialize NVML: Driver/library version mismatch\nNVML library version: 615.71")
        self.assertEqual(result.returncode, 1)
        self.assertIn("NVML library version: 615.71", result.stderr)
        self.assertIn("loaded NVIDIA kernel module", result.stderr)
        self.assertIn("rebooting the container cannot reload the host module", result.stderr)
        self.assertNotIn("UNEXPECTED_", result.stderr)

    def test_generic_query_error_is_not_hidden(self):
        result = self.run_helpers("check_nvidia", FIXTURE_SMI_STATUS="1",
                                  FIXTURE_SMI="Insufficient Permissions")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Insufficient Permissions", result.stderr)
        self.assertIn("device permissions", result.stderr)

    def test_empty_successful_query_is_failure(self):
        result = self.run_helpers("check_nvidia", FIXTURE_SMI="")
        self.assertEqual(result.returncode, 1)
        self.assertIn("returned no GPU names", result.stderr)

    def test_nvml_without_nvidia_smi_needs_monitor_verification(self):
        result = self.run_helpers(self.no_smi() + 'check_nvidia\nverify_ztop_gpu "$TARGET"',
                                  FIXTURE_RUNTIME="0")
        self.assert_success(result)
        self.assertIn("GPU access still needs verification", result.stdout)
        self.assertIn("GPU verification passed", result.stdout)

    def test_runtime_files_without_working_nvml_fail_verification(self):
        self.snapshot()
        result = self.run_helpers(self.no_smi() + 'check_nvidia\nverify_ztop_gpu "$TARGET"',
                                  FIXTURE_RUNTIME="0",
                                  FIXTURE_DIAGNOSTIC="ztop: GPU init failed (NVML init: mismatch)")
        self.assertEqual(result.returncode, 1)
        self.assertIn("GPU verification failed", result.stderr)
        self.assertIn("NVML init: mismatch", result.stderr)

    def test_cpu_only_install_is_valid_without_nvidia_runtime(self):
        self.snapshot()
        result = self.run_helpers(self.no_smi() + 'check_nvidia\nverify_ztop_gpu "$TARGET"')
        self.assert_success(result)
        self.assertIn("GPU monitoring unavailable", result.stdout)

    def test_cpu_only_host_reports_missing_nvml_as_information(self):
        self.snapshot()
        result = self.run_helpers(self.no_smi() + 'check_nvidia\nverify_ztop_gpu "$TARGET"',
                                  FIXTURE_DIAGNOSTIC="ztop: GPU init failed (NVML init: libnvidia-ml.so.1: cannot open shared object file)")
        self.assert_success(result)
        self.assertIn("no NVIDIA runtime detected", result.stdout)
        self.assertIn("libnvidia-ml.so.1: cannot open", result.stdout)
        self.assertNotIn("GPU init failed", result.stderr)
        self.assertIn("GPU monitoring unavailable", result.stdout)

    def test_missing_or_stale_gpu_cannot_pass(self):
        for statuses in (("fresh",), ("fresh", "stale"), ("unavailable", "fresh"), ()):
            with self.subTest(statuses=statuses):
                self.env["FIXTURE_SMI"] = "NVIDIA fixture 0\nNVIDIA fixture 1"
                self.snapshot(*statuses)
                result = self.run_helpers('check_nvidia\nverify_ztop_gpu "$TARGET"')
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("GPU verification failed", result.stderr)

    def test_snapshot_command_failure_and_invalid_output_cannot_pass(self):
        result = self.run_helpers('check_nvidia\nverify_ztop_gpu "$TARGET"',
                                  FIXTURE_ZTOP_STATUS="2")
        self.assertEqual(result.returncode, 1)
        self.assertIn("could not produce a verification snapshot", result.stderr)
        result = self.run_helpers('check_nvidia\nverify_ztop_gpu "$TARGET"',
                                  FIXTURE_SNAPSHOT="not a snapshot")
        self.assertEqual(result.returncode, 1)
        self.assertIn("did not contain a GPU snapshot", result.stderr)

    def test_check_fails_with_broken_monitor_but_accepts_working_private_nvml(self):
        self.snapshot()
        result = self.run_helpers("do_check", FIXTURE_SMI_STATUS="18",
                                  FIXTURE_SMI="Driver/library version mismatch")
        self.assertEqual(result.returncode, 1)
        self.snapshot("fresh")
        result = self.run_helpers("do_check", FIXTURE_SMI_STATUS="18",
                                  FIXTURE_SMI="Driver/library version mismatch")
        self.assert_success(result)
        self.assertIn("Driver/library version mismatch", result.stderr)
        self.assertIn("GPU verification passed", result.stdout)
        # Only the first line of `--version` belongs in the audit line.
        self.assertIn("exists (fixture ztop)", result.stdout)
        self.assertNotIn("fixture copyright", result.stdout)

    def test_writable_user_prefix_needs_no_sudo_authentication(self):
        # Fixture is writable even when the test suite itself runs as root.
        result = self.run_helpers(r'''
id() { printf '1000\n'; }
sudo() { printf UNEXPECTED_SUDO >&2; return 93; }
sudo_preauth
if monitor_install_needs_sudo; then printf UNEXPECTED_PRIVILEGES; exit 1; fi
''')
        self.assert_success(result)

        self.binary.unlink()
        self.binary.parent.rmdir()
        result = self.run_helpers(r'''
id() { printf '1000\n'; }
sudo() { printf UNEXPECTED_SUDO >&2; return 93; }
sudo_preauth
if monitor_install_needs_sudo; then printf UNEXPECTED_PRIVILEGES; exit 1; fi
''')
        self.assert_success(result)

    def test_system_prefix_pre_authenticates_before_building(self):
        result = self.run_helpers(r'''
id() { printf '1000\n'; }
monitor_install_needs_sudo() { return 0; }
sudo() { return 0; }
sudo_authenticate() { printf AUTHENTICATED; }
sudo_preauth
''')
        self.assert_success(result)
        self.assertIn("AUTHENTICATED", result.stdout)


if __name__ == "__main__":
    unittest.main()
