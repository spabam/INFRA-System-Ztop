"""Exercise the installer's checkout-ownership restore against a temporary checkout.

Run: python3 -B -m unittest discover -s tests/shell -p test_installer_ownership.py -v
The installer is sourced; sudo is a recording stub, so no real ownership changes.
"""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[2] / "installer.sh"
SHELL_SETUP = 'source "$1"\nHERE="$2"\ncd -- "$HERE"\nshift 2\n'
SUDO_STUB = '#!/bin/sh\nprintf \'%s\\n\' "$*" >> "$FIXTURE_COMMANDS"\nexit "${FIXTURE_SUDO_EXIT:-0}"\n'
FOREIGN = "nobody:nogroup"


def current_owner():
    user = subprocess.run(["id", "-un"], text=True, capture_output=True, check=True).stdout.strip()
    group = subprocess.run(["id", "-gn"], text=True, capture_output=True, check=True).stdout.strip()
    return f"{user}:{group}"


# The checkout is returned to the account that installs. Started as root there
# is no such account to return it to, so these run for an ordinary login only.
as_login = unittest.skipIf(os.geteuid() == 0, 'describes an installation started by an ordinary login; run the tests as that account')


class CheckoutOwnership(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ztop ownership ")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.checkout = self.base / "fixture checkout"
        (self.checkout / ".git").mkdir(parents=True)
        (self.checkout / ".git" / "index").write_text("")
        self.commands = self.base / "commands"
        bin_dir = self.base / "bin"
        bin_dir.mkdir()
        sudo = bin_dir / "sudo"
        sudo.write_text(SUDO_STUB)
        sudo.chmod(0o755)
        self.env = os.environ.copy()
        self.env["PATH"] = f"{bin_dir}:{self.env['PATH']}"
        self.env["FIXTURE_COMMANDS"] = str(self.commands)
        for name in ("SUDO_ASKPASS", "SUDO_USER", "ZTOP_OWNER"):
            self.env.pop(name, None)

    def run_shell(self, body, **env):
        return subprocess.run(
            ["bash", "-c", SHELL_SETUP + body, "test", str(INSTALLER), str(self.checkout)],
            env={**self.env, **env}, text=True, capture_output=True, timeout=10,
            stdin=subprocess.DEVNULL,
        )

    def recorded(self):
        return self.commands.read_text().splitlines() if self.commands.exists() else []

    def chown_call(self):
        return f"chown -Rf {FOREIGN} {self.checkout}"

    def test_owner_is_override_then_invoking_user(self):
        result = self.run_shell("checkout_owner", ZTOP_OWNER="alice:staff")
        self.assertEqual(result.stdout.strip(), "alice:staff", result.stderr)
        result = self.run_shell("checkout_owner")
        self.assertEqual(result.stdout.strip(), current_owner(), result.stderr)
        # sudo from a root shell must not hand the checkout to root.
        result = self.run_shell("checkout_owner", SUDO_USER="root")
        self.assertEqual(result.stdout.strip(), current_owner(), result.stderr)
        result = self.run_shell("checkout_owner", ZTOP_OWNER="no-such-ztop-user")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ZTOP_OWNER", result.stderr)

    def test_clean_checkout_is_left_alone_without_sudo(self):
        result = self.run_shell("restore_checkout_ownership start; restore_checkout_ownership end; printf DONE")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "DONE")
        self.assertEqual(self.recorded(), [])

    @as_login
    def test_start_stage_authenticates_chowns_and_verifies(self):
        # The stub sudo succeeds without changing anything, so verification must fail loudly.
        result = self.run_shell("restore_checkout_ownership start; printf UNEXPECTED", ZTOP_OWNER=FOREIGN)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("UNEXPECTED", result.stdout)
        self.assertIn("cannot restore ownership", result.stderr)
        self.assertIn(f'sudo chown -Rf {FOREIGN} "{self.checkout}"', result.stderr)
        self.assertEqual(self.recorded(), ["-n true", self.chown_call()])

    @as_login
    def test_start_stage_without_cached_sudo_or_terminal_explains(self):
        result = self.run_shell("restore_checkout_ownership start; printf UNEXPECTED",
                                ZTOP_OWNER=FOREIGN, FIXTURE_SUDO_EXIT="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("UNEXPECTED", result.stdout)
        self.assertIn("no terminal", result.stderr)
        self.assertEqual(self.recorded(), ["-n true"])

    @as_login
    def test_end_stage_only_warns(self):
        body = "if restore_checkout_ownership end; then printf RESTORED; else printf WARNED; fi"
        result = self.run_shell(body, ZTOP_OWNER=FOREIGN)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("WARNED"), result.stdout)
        self.assertIn("was not fully restored", result.stderr)
        self.assertEqual(self.recorded(), ["-n true", self.chown_call()])
        self.commands.unlink()
        result = self.run_shell(body, ZTOP_OWNER=FOREIGN, FIXTURE_SUDO_EXIT="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("WARNED"), result.stdout)
        self.assertIn("no longer cached", result.stderr)
        self.assertEqual(self.recorded(), ["-n true"])

    @as_login
    def test_exit_trap_keeps_the_exit_status(self):
        result = self.run_shell("trap finish_checkout_ownership EXIT; exit 7")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(self.recorded(), [])
        result = self.run_shell("trap finish_checkout_ownership EXIT; exit 7", ZTOP_OWNER=FOREIGN)
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertIn("was not fully restored", result.stderr)
        self.assertEqual(self.recorded(), ["-n true", self.chown_call()])


if __name__ == "__main__":
    unittest.main()
