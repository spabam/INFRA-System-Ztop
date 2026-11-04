"""Fixture-only checks. Kernel knobs, /proc, state, and swap commands are fake.

Run: python3 -m unittest discover -s tests/shell -v
No test invokes the installer mutation entrypoints or host memory management.
"""
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RC = ROOT / "ramcleaner/ztop-ramcleaner"
INSTALLER = ROOT / "installer.sh"

STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ["FIXTURE_ROOT"])
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name in ("sudo", "apt-get", "dnf", "yum", "pacman", "zypper", "apk", "curl", "rustup"):
    with (root / "commands").open("a") as f: f.write(json.dumps([name] + args) + "\n")
    sys.exit(93)
if name in ("renice", "ionice"): sys.exit(0)
if name == "id": print("0"); sys.exit(0)
if name == "nproc": print("4"); sys.exit(0)
if name == "nvidia-smi": sys.exit(1)
if name in ("rustc", "cargo"): print(name + " 1.88.0"); sys.exit(0)
if name == "findmnt":
    if os.environ.get("DROP_MEMORY_DURING_PREPARE"):
        p = root / "proc/meminfo"
        p.write_text(p.read_text().replace("MemAvailable: 4194304 kB", "MemAvailable: 262144 kB"))
    print("defaults,discard=once,pri=10")
    sys.exit(0)
if name in ("swapon", "swapoff"):
    with (root / "commands").open("a") as f: f.write(json.dumps([name] + args) + "\n")
    table = root / "proc/swaps"
    if name == "swapoff":
        if os.environ.get("SWAPOFF_FAIL"): sys.exit(1)
        lines = table.read_text().splitlines()
        table.write_text("\n".join(x for x in lines if not x.startswith("/dev/mock ")) + "\n")
        sys.exit(0)
    if os.environ.get("SWAPON_FAIL"): sys.exit(1)
    with table.open("a") as f: f.write("/dev/mock partition 1048576 0 10\n")
    sys.exit(0)
if name == "timeout":
    if os.environ.get("TIMEOUT_AFTER_OFF"):
        subprocess = __import__("subprocess")
        subprocess.run(args[2:], check=False)
        sys.exit(124)
    os.execvp(args[2], args[2:])
raise SystemExit("Unexpected stub: " + name)
'''


class ShellBehavior(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ztop-shell-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        for p in ("proc/pressure", "proc/sys/vm", "cgroup/system.slice", "cgroup/user.slice", "bin", "home", "prefix"):
            (self.base / p).mkdir(parents=True)
        self.write_mem(4096)
        (self.base / "proc/loadavg").write_text("0.01 0.00 0.00 1/10 1\n")
        (self.base / "proc/pressure/memory").write_text("some avg10=0.00 avg60=0.00 avg300=0.00 total=1\n")
        (self.base / "proc/swaps").write_text("Filename Type Size Used Priority\n/dev/mock partition 1048576 943718 10\n/dev/zram0 partition 2097152 0 100\n")
        (self.base / "proc/sys/vm/compact_memory").write_text("original\n")
        for name in ("system.slice", "user.slice"):
            (self.base / f"cgroup/{name}/memory.current").write_text("1073741824\n")
            (self.base / f"cgroup/{name}/memory.reclaim").write_text("original\n")
        for name in ("sudo", "apt-get", "dnf", "yum", "pacman", "zypper", "apk", "curl", "rustup", "renice", "ionice", "id", "nproc", "nvidia-smi", "rustc", "cargo", "findmnt", "swapon", "swapoff", "timeout"):
            stub = self.base / "bin" / name
            stub.write_text(STUB)
            stub.chmod(0o755)
        self.config = self.base / "config"
        self.settings = {
            "PROC_ROOT": str(self.base / "proc"),
            "CGROUP_ROOT": str(self.base / "cgroup"),
            "STATE_DIR": str(self.base / "state"),
            "LOG": str(self.base / "log"),
            "LOCK": str(self.base / "lock"),
            "DISABLE_FLAG": str(self.base / "disabled"),
            "FSTAB": str(self.base / "fstab"),
        }
        self.env = os.environ.copy()
        self.env.update({
            "FIXTURE_ROOT": str(self.base),
            "ZTOP_RC_CONFIG": str(self.config),
            "ZTOP_RC_STATE_DIR": str(self.base / "state"),
            "ZTOP_RC_LOG": str(self.base / "log"),
            "PATH": str(self.base / "bin") + ":" + self.env["PATH"],
            "HOME": str(self.base / "home"),
            "RUSTUP_HOME": str(self.base / "home/.rustup"),
            "PREFIX": str(self.base / "prefix"),
        })
        self.update_config()

    def update_config(self, **values):
        self.settings.update(values)
        self.config.write_text("\n".join(f"{key}={shlex.quote(str(value))}" for key, value in self.settings.items()) + "\n")

    def write_mem(self, available):
        (self.base / "proc/meminfo").write_text(f"MemTotal: 8388608 kB\nMemAvailable: {available * 1024} kB\nSwapTotal: 3145728 kB\nSwapFree: 2202010 kB\n")

    def run_rc(self, *args, expect=0):
        result = subprocess.run(["bash", str(RC), *args], env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, expect, result.stdout + result.stderr + self.log())
        return result

    def commands(self):
        file = self.base / "commands"
        return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []

    def log(self):
        file = self.base / "log"
        return file.read_text() if file.exists() else ""

    def state(self):
        file = self.base / "state/state"
        return dict(line.split("=", 1) for line in file.read_text().splitlines()) if file.exists() else {}

    def test_installer_inspection_has_no_mutations(self):
        for args, code in ((["--help"], 0), (["--check"], 1), (["--invalid"], 1), (["--check", "extra"], 1)):
            result = subprocess.run(["bash", str(INSTALLER), *args], env=self.env, text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        self.assertEqual(self.commands(), [])
        self.assertEqual(list((self.base / "prefix").iterdir()), [])
        self.assertFalse((self.base / "lock").exists())

    def test_installer_lock_failure_prevents_progress(self):
        flock = self.base / "bin/flock"
        flock.write_text("#!/bin/sh\nexit 42\n")
        flock.chmod(0o755)
        result = subprocess.run(["bash", "-c", 'source "$1"; acquire_install_lock; printf UNEXPECTED', "test", str(INSTALLER)], env=self.env, text=True, capture_output=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("UNEXPECTED", result.stdout)
        self.assertEqual(self.commands(), [])

    def test_help_version_do_not_source_config(self):
        self.config.write_text("exit 99\n")
        self.run_rc("--help")
        self.run_rc("--version")
        self.assertEqual(self.commands(), [])

    def test_default_forced_reclaim_only_previews(self):
        self.run_rc("--force-tier", "1")
        self.assertIn("DRY would write", self.log())
        self.assertEqual((self.base / "cgroup/system.slice/memory.reclaim").read_text(), "original\n")
        self.assertFalse((self.base / "state").exists())
        self.assertFalse((self.base / "lock").exists())
        self.assertEqual(self.commands(), [])

    def test_dry_run_overrides_active_config(self):
        self.update_config(DRY_RUN=0)
        self.run_rc("--dry-run", "--force-tier", "2")
        self.assertEqual((self.base / "proc/sys/vm/compact_memory").read_text(), "original\n")
        self.assertFalse((self.base / "state").exists())

    def test_status_has_no_mutations(self):
        result = self.run_rc("--status")
        self.assertIn("compaction: available", result.stdout)
        self.assertFalse((self.base / "log").exists())
        self.assertFalse((self.base / "state").exists())

    def test_disabled_swap_tier_cannot_be_forced(self):
        self.run_rc("--apply", "--force-tier", "3", expect=1)
        self.assertEqual(self.commands(), [])

    def test_tier4_always_unavailable(self):
        self.run_rc("--apply", "--force-tier", "4", expect=1)
        self.update_config(ENABLE_T4_PARK_SERVICES=1)
        self.run_rc("--apply", expect=1)
        self.assertEqual(self.commands(), [])

    def test_success_advances_cooldown_and_second_pass_skips(self):
        self.write_mem(2000)
        self.run_rc("--apply")
        self.assertEqual(self.state()["t1_outcome"], "succeeded")
        self.assertIn("t1_last", self.state())
        knob = self.base / "cgroup/system.slice/memory.reclaim"
        knob.write_text("sentinel\n")
        self.run_rc("--apply")
        self.assertEqual(knob.read_text(), "sentinel\n")
        self.assertEqual(self.state()["pass_outcome"], "skipped")

    def test_failed_write_returns_nonzero_without_success_cooldown(self):
        self.update_config(RECLAIM_TARGETS="unused")
        # Replace scalar assignment with a proper single-target Bash array.
        with self.config.open("a") as f: f.write('RECLAIM_TARGETS=("system.slice:10:256")\n')
        knob = self.base / "cgroup/system.slice/memory.reclaim"
        knob.unlink()
        knob.mkdir()
        self.run_rc("--apply", "--force-tier", "1", expect=1)
        self.assertEqual(self.state()["t1_outcome"], "failed")
        self.assertNotIn("t1_last", self.state())
        result = self.run_rc("--status")
        self.assertIn("t1 outcome: failed", result.stdout)

    def test_mixed_reclaim_is_partial_and_nonzero(self):
        knob = self.base / "cgroup/user.slice/memory.reclaim"
        knob.unlink()
        knob.mkdir()
        self.run_rc("--apply", "--force-tier", "1", expect=1)
        self.assertEqual(self.state()["t1_outcome"], "partial")
        self.assertNotIn("t1_last", self.state())

    def test_missing_kernel_capability_is_visible_skip(self):
        (self.base / "proc/sys/vm/compact_memory").unlink()
        self.run_rc("--apply", "--force-tier", "2")
        self.assertEqual(self.state()["t2_outcome"], "skipped")
        self.assertNotIn("t2_last", self.state())

    def test_t3_refreshes_metrics_after_preparation(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.env["DROP_MEMORY_DURING_PREPARE"] = "1"
        self.run_rc("--apply", "--force-tier", "3")
        self.assertEqual(self.commands(), [])
        self.assertEqual(self.state()["t3_outcome"], "skipped")

    def test_other_swap_capacity_does_not_reduce_required_ram(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.write_mem(1024)
        self.run_rc("--apply", "--force-tier", "3")
        self.assertEqual(self.commands(), [])
        self.assertIn("need >", self.log())

    def test_swap_configuration_and_priority_restored(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.run_rc("--apply", "--force-tier", "3")
        self.assertEqual(self.commands(), [["swapoff", "--", "/dev/mock"], ["swapon", "--options", "defaults,discard=once,pri=10", "--priority", "10", "--", "/dev/mock"]])
        self.assertEqual(self.state()["t3_outcome"], "succeeded")
        self.assertEqual(self.state()["recovery_device"], "")

    def test_swapoff_failure_checks_state_and_reports_failure(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.env["SWAPOFF_FAIL"] = "1"
        self.run_rc("--apply", "--force-tier", "3", expect=1)
        self.assertEqual(self.commands(), [["swapoff", "--", "/dev/mock"]])
        self.assertEqual(self.state()["t3_outcome"], "failed")
        self.assertEqual(self.state()["recovery_device"], "")
        self.assertNotIn("t3_last", self.state())

    def test_timeout_after_disable_still_restores_swap(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.env["TIMEOUT_AFTER_OFF"] = "1"
        self.run_rc("--apply", "--force-tier", "3", expect=1)
        self.assertEqual([c[0] for c in self.commands()], ["swapoff", "swapon"])
        self.assertEqual(self.state()["recovery_device"], "")
        self.assertEqual(self.state()["t3_outcome"], "failed")

    def test_failed_reenable_is_visible_and_next_pass_recovers(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.env["SWAPON_FAIL"] = "1"
        self.run_rc("--apply", "--force-tier", "3", expect=1)
        self.assertEqual(self.state()["recovery_device"], "/dev/mock")
        self.assertIn("RECOVERY REQUIRED", self.run_rc("--status").stdout)
        del self.env["SWAPON_FAIL"]
        self.update_config(ENABLE_T3_SWAP_RECYCLE=0)
        self.run_rc("--apply")
        self.assertEqual(self.state()["recovery_device"], "")
        self.assertEqual(self.commands()[-1][0], "swapon")

    def test_dry_run_never_recovers_disabled_swap(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        self.env["SWAPON_FAIL"] = "1"
        self.run_rc("--apply", "--force-tier", "3", expect=1)
        previous = self.commands()
        self.run_rc("--dry-run")
        self.assertEqual(self.commands(), previous)
        self.assertEqual(self.state()["recovery_device"], "/dev/mock")

    def pending_recovery(self, device="/dev/mock", priority="10", options="defaults,pri=10"):
        state = self.base / "state/state"
        state.parent.mkdir(exist_ok=True)
        state.write_text(f"recovery_device={device}\nrecovery_priority={priority}\nrecovery_options={options}\n")
        swaps = self.base / "proc/swaps"
        swaps.write_text("\n".join(line for line in swaps.read_text().splitlines() if not line.startswith("/dev/mock ")) + "\n")
        return state

    def test_invalid_recovery_state_never_arms_exit_handler(self):
        for mode in ("--apply", "--recover"):
            for device, priority, options in (("/dev/mock", "invalid", "defaults"),
                                              ("/dev/mock", "32768", "defaults"),
                                              ("/dev/mock", "00010", "defaults"),
                                              ("/tmp/not-a-device", "10", "defaults"),
                                              ("-a", "10", "defaults"),
                                              ("/dev/../tmp/mock", "10", "defaults"),
                                              ("/dev/mock", "10", "bad option")):
                with self.subTest(mode=mode, device=device, priority=priority, options=options):
                    state = self.pending_recovery(device, priority, options)
                    before = state.read_bytes()
                    self.run_rc(mode, expect=1)
                    self.assertEqual(self.commands(), [])
                    self.assertEqual(state.read_bytes(), before)

    def test_explicit_recovery_works_while_suspended_without_new_actions(self):
        self.pending_recovery()
        self.write_mem(256)  # Would otherwise trigger both reclaim and compaction.
        self.update_config(ENABLE_T1_RECLAIM=0, ENABLE_T2_COMPACT=0, ENABLE_T3_SWAP_RECYCLE=0)
        (self.base / "disabled").touch()
        self.run_rc("--recover")
        self.assertEqual([command[0] for command in self.commands()], ["swapon"])
        self.assertEqual(self.state()["recovery_device"], "")
        self.assertEqual(self.state()["recovery_outcome"], "succeeded")
        self.assertEqual((self.base / "cgroup/system.slice/memory.reclaim").read_text(), "original\n")
        self.assertEqual((self.base / "proc/sys/vm/compact_memory").read_text(), "original\n")
        self.assertTrue((self.base / "disabled").exists())

    def test_suspended_scheduled_pass_preserves_pending_recovery(self):
        state = self.pending_recovery()
        before = state.read_bytes()
        self.update_config(DRY_RUN=0)
        (self.base / "disabled").touch()
        self.run_rc()
        self.assertEqual(self.commands(), [])
        self.assertEqual(state.read_bytes(), before)
        self.assertIn("run --recover", self.run_rc("--status").stdout)

    def test_recovery_mode_conflicts_do_not_dispatch_actions(self):
        state = self.pending_recovery()
        before = state.read_bytes()
        for args in (("--recover", "--apply"), ("--status", "--recover"),
                     ("--recover", "--dry-run"), ("--recover", "--force-tier", "1")):
            self.run_rc(*args, expect=1)
        self.assertEqual(self.commands(), [])
        self.assertEqual(state.read_bytes(), before)

    def test_recovery_requires_root_without_arming_exit_handler(self):
        state = self.pending_recovery()
        before = state.read_bytes()
        (self.base / "bin/id").write_text("#!/bin/sh\nprintf '1000\\n'\n")
        self.run_rc("--recover", expect=1)
        self.assertEqual(self.commands(), [])
        self.assertEqual(state.read_bytes(), before)

    def test_recovery_lock_failure_does_not_arm_exit_handler(self):
        state = self.pending_recovery()
        before = state.read_bytes()
        flock = self.base / "bin/flock"
        flock.write_text("#!/bin/sh\nexit 75\n")
        flock.chmod(0o755)
        self.run_rc("--recover", expect=1)
        self.assertEqual(self.commands(), [])
        self.assertEqual(state.read_bytes(), before)

    def test_negative_priority_is_skipped(self):
        self.update_config(ENABLE_T3_SWAP_RECYCLE=1)
        swaps = self.base / "proc/swaps"
        swaps.write_text(swaps.read_text().replace("943718 10", "943718 -2"))
        self.run_rc("--apply", "--force-tier", "3")
        self.assertEqual(self.commands(), [])
        self.assertEqual(self.state()["t3_outcome"], "skipped")

    def test_invalid_metrics_fail_before_actions(self):
        (self.base / "proc/meminfo").write_text("MemTotal: invalid\n")
        self.run_rc("--apply", expect=1)
        self.assertEqual(self.commands(), [])

    def test_disable_flag_skips_actions(self):
        (self.base / "disabled").touch()
        self.run_rc("--apply", "--force-tier", "1")
        self.assertFalse((self.base / "state").exists())
        self.assertEqual(self.commands(), [])


if __name__ == "__main__":
    unittest.main()
