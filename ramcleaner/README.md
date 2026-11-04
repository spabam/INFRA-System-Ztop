# ztop-ramcleaner

An optional memory-management companion to ZTOP. The normal monitor install does
not install it. `../installer.sh --install-ramcleaner` installs a separate cron
job every 10 minutes with **preview enabled** and **swap recycling disabled**.
Existing operator configuration is preserved on reinstall and can already be
active; the install verification pass always uses `--dry-run`.

## Behavior and tradeoffs

| Tier | Action | Default |
|---|---|---|
| T1 | Proactive `memory.reclaim` for configured cgroup v2 slices | Preview |
| T2 | `vm.compact_memory` to compact free pages | Preview |
| T3 | Temporarily disable and re-enable one disk swap partition | Disabled |
| T4 | Service parking | Unavailable until service-specific idle probes exist |

These operations can introduce latency and memory pressure. The script does not
kill processes, but it cannot guarantee applications survive unrelated or
concurrent memory pressure. Reclaim can evict useful pages; compaction consumes
CPU. Swap occupancy alone does not establish that recycling benefits a workload.
Measure pressure, latency, and application behavior before enabling actions.
Neither `--apply` nor `--force-tier` enables a disabled tier. A forced tier runs
only that tier and retains its safety and capability checks. T4 always fails
closed; service uptime is not proof of idleness.

## Preview and opt in

```sh
./installer.sh --install-ramcleaner       # from the repository root
sudo ztop-ramcleaner --status
sudo ztop-ramcleaner --dry-run --verbose
sudo ztop-ramcleaner --apply              # one active pass of enabled tiers
sudo ztop-ramcleaner --recover            # restore pending swap only, even suspended
# Enable scheduled T1/T2 actions after reviewing the preview:
sudo sed -i 's/^DRY_RUN=1/DRY_RUN=0/' /etc/ztop/ramcleaner.conf
sudo touch /run/ztop-ramcleaner.disable   # suspend passes
sudo rm /run/ztop-ramcleaner.disable      # resume
```

Enable T3 separately with `ENABLE_T3_SWAP_RECYCLE=1` in the configuration only
after assessing its behavior on the host. The installer requires a running
cron daemon and checks executables before scheduling. Missing optional kernel
capabilities are reported as skipped, so a successful preview is not a guarantee
that every tier is supported or that active writes will succeed.

`--status`, `--help`, and `--version` make no changes. A dry run writes its log
only; it does not create a lock, update cooldowns, modify kernel settings, or
re-enable swap. Active passes use `flock` to prevent overlap, with lower CPU/I/O
priority where supported. These priorities do not eliminate workload interference.

## Swap guard and recovery

T3 selects the first non-zram partition in `/proc/swaps`; swap files and zram
are left alone. It requires a single matching `/etc/fstab` entry and an explicit
nonnegative current swap priority. Kernel-assigned negative priorities cannot
be restored exactly and are skipped. Configured swap options and the current
priority are preserved when re-enabling the device.

Metrics are refreshed after earlier tiers and directly before swapoff. The
headroom gate requires `MemAvailable > selected-device-used × 1.5 + 512 MiB`
with low load by default. Free capacity on other swaps is not counted as RAM
headroom. The sample does not reserve memory: concurrent allocations can still
change conditions during swapoff. A timeout bounds the command, but a failure is
not assumed to be a no-op; the actual device state is checked afterward.

Before swapoff, recovery intent is written to persistent state. The device is
re-enabled after success or failure if it is found inactive. An exit/signal
handler retries recovery, and a subsequent active pass retries valid recorded
pending recovery before doing other work. Scheduled passes remain suspended when
the disable flag exists, including their automatic recovery. Use the dedicated
`sudo ztop-ramcleaner --recover` command to restore pending swap while suspended:
it requires root and the same lock, ignores tier enablement and pressure
thresholds, and performs no reclaim, compaction, or new swap recycling. It cannot
be combined with another action mode or `--force-tier`.

Recovery metadata is validated before the exit handler is armed. Invalid device
paths, priorities outside 0..32767, or invalid options cause an error without
invoking swapon or clearing the recovery marker. Persistent failure returns
nonzero, is reported by `--status`, and prints `--recover` and manual recovery hints. No handler can
guarantee recovery after abrupt termination or hardware failure; inspect the
reported state and host swap configuration before resuming a workload.

## Outcomes and files

Each attempted tier records `succeeded`, `partial`, `failed`, or `skipped`, its
last attempt, and its last successful action. A failed reclaim write with observed
memory reduction is reported as partial; concurrent activity means the amount
cannot be attributed exclusively to the request. Partial/failed actions return
nonzero and do not advance the success cooldown. Preview records decisions in
the log without changing persistent state. The pass outcome and pending swap
recovery are included in `--status`.

| File | Purpose |
|---|---|
| `$PREFIX/sbin/ztop-ramcleaner` | Executable |
| `/etc/ztop/ramcleaner.conf` | Operator configuration; preserved on reinstall |
| `/etc/cron.d/ztop-ramcleaner` | Separate optional schedule |
| `/var/lib/ztop-ramcleaner/state` | Outcomes, cooldowns, and swap recovery |
| `/var/log/ztop-ramcleaner.log` | Decisions and action results |
| `/etc/logrotate.d/ztop-ramcleaner` | Weekly log rotation |

`--uninstall-ramcleaner` removes the script, schedule, and logrotate entry while
preserving configuration, state, and logs. Resolve pending recovery before
uninstalling. Optional `--install-ramcleaner-tuning` applies only its own sysctl
file and requires active zram; review the profile in that file first. The zram example is a
configuration starting point, not a prediction of compression or workload benefit.

The script does not schedule `drop_caches`, manage the OOM killer, or install
zram. CPU count, workload, memory capacity, and kernel capabilities vary; tune
thresholds deliberately instead of assuming the bundled values suit every host.
