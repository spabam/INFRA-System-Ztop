#!/usr/bin/env python3
"""Exercise the built binary; no GPU workload or system configuration changes."""

import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import termios
import time
import unittest


BIN = os.path.abspath(os.environ.get("ZTOP_TEST_BIN", "target/release/ztop"))
BASE = [BIN, "--no-config", "--no-gpu", "--no-temp"]


class RuntimeTests(unittest.TestCase):
    def test_stream_is_ndjson_and_count_is_respected(self):
        result = subprocess.run(
            BASE + ["--json-stream", "--count", "3", "--refresh", "0.25", "--top", "2"],
            capture_output=True, text=True, check=True, timeout=15,
        )
        samples = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(samples), 3)
        for sample in samples:
            self.assertEqual(sample["schema_version"], 2)
            self.assertEqual(sample["gpus"], [])
            self.assertLessEqual(len(sample["procs"]), 2)
        times = [sample["sampled_at_unix_ms"] for sample in samples]
        self.assertTrue(all(a < b for a, b in zip(times, times[1:])))

    def test_closed_stream_pipe_exits_cleanly(self):
        with subprocess.Popen(
            BASE + ["--json-stream", "--refresh", "0.25"], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        ) as proc:
            json.loads(proc.stdout.readline())
            proc.stdout.close()
            self.assertEqual(proc.wait(timeout=10), 0)
            self.assertEqual(proc.stderr.read(), b"")

    def test_process_rows_exclude_threads(self):
        result = subprocess.run(BASE + ["--json", "--sort", "ram", "--top", "200"],
                                capture_output=True, text=True, check=True, timeout=15)
        checked = 0
        for row in json.loads(result.stdout)["procs"]:
            try:
                status = (Path("/proc") / str(row["pid"]) / "status").read_text()
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                continue
            tgid = next(line.split()[1] for line in status.splitlines() if line.startswith("Tgid:"))
            self.assertEqual(row["pid"], int(tgid))
            checked += 1
        self.assertGreater(checked, 0)

    def test_disk_temperatures_are_whole_degrees_or_null_and_follow_no_temp(self):
        def disks(args):
            result = subprocess.run([BIN, "--no-config", "--no-gpu", "--json", "--top", "1"] + args,
                                    capture_output=True, text=True, check=True, timeout=15)
            return json.loads(result.stdout)["disks"]

        measured = disks([])
        for disk in measured:
            # What the drive is: text as the kernel gives it, or null.
            for field in ("model", "kind"):
                self.assertIn(field, disk)
                if disk[field] is not None:
                    self.assertTrue(disk[field] and disk[field] == disk[field].strip(), disk)
            reported = Path("/sys/block", disk["name"], "device/model")
            if reported.is_file() and reported.read_text().split():
                self.assertIn(" ".join(reported.read_text().split()), disk["model"])
            self.assertIn("temp_c", disk)
            if disk["temp_c"] is not None:
                self.assertIsInstance(disk["temp_c"], int)
                self.assertTrue(-50 <= disk["temp_c"] <= 150, disk)
            # The kernel's own sensor for the drive is what the monitor shows.
            sensors = sorted(Path("/sys/block", disk["name"], "device").glob("hwmon*/temp1_input"))
            sensors += sorted(Path("/sys/block", disk["name"], "device").glob("hwmon/hwmon*/temp1_input"))
            if len(sensors) == 1:
                self.assertIsNotNone(disk["temp_c"], disk)
                self.assertLessEqual(abs(disk["temp_c"] - int(sensors[0].read_text()) / 1000), 3, disk)
        unasked = disks(["--no-temp"])
        self.assertEqual([disk["name"] for disk in unasked], [disk["name"] for disk in measured])
        self.assertTrue(all(disk["temp_c"] is None for disk in unasked))

    def test_filter_matches_exported_processes(self):
        query = "ztop"
        result = subprocess.run(BASE + ["--json", "--filter", query],
                                capture_output=True, text=True, check=True, timeout=15)
        rows = json.loads(result.stdout)["procs"]
        self.assertTrue(rows)
        for row in rows:
            self.assertTrue(any(query in str(row[field]).lower() for field in ("command", "user", "pid")))

    def test_ctrl_c_quits_during_search_and_restores_terminal(self):
        master, slave = pty.openpty()
        before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        proc = subprocess.Popen(BASE, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        try:
            output = b""
            deadline = time.monotonic() + 10
            while b"ZTOP" not in output and time.monotonic() < deadline:
                if select.select([master], [], [], 0.1)[0]:
                    output += os.read(master, 65536)
                if proc.poll() is not None:
                    self.fail(f"TUI exited before rendering: {output!r}")
            self.assertIn(b"ZTOP", output)
            # Enter an unfinished query, resize while active, then quit via Ctrl-C.
            os.write(master, b"/python")
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 40, 0, 0))
            os.write(master, b"\x03")
            self.assertEqual(proc.wait(timeout=10), 0)
            self.assertEqual(termios.tcgetattr(slave), before)
        finally:
            if proc.poll() is None:
                proc.kill()
            proc.wait()
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    unittest.main()
