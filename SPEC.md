# ZTOP Current Engineering Contract

This document is the implementation contract for the current unreleased ZTOP
development line. The README remains the public overview; this file pins the
runtime behavior, layout rules, data contracts, and release checks that should
stay true as the code evolves.

The historical v0.1 MVP is complete. The current implementation includes
multi-GPU panels, config-file defaults, headless JSON output, live help, live
row-count adjustment, GPU peak hold, process search and scrolling, a per-disk
usage panel, JSON streaming, and monotonically increasing release versions.

---

## 1. Scope

ZTOP is a compact Linux terminal monitor focused on:

- CPU brand with physical core and thread counts, total usage, per-core usage,
  and package temperature
- RAM and swap used / total / percentage
- NVIDIA GPU name, utilization, temperature, VRAM used / total / percentage
- Multi-GPU display, one panel per NVIDIA device
- Physical disks: model and kind, raw capacity, used / total / percentage
  of the filesystems mounted on each, and the drive's temperature
- Per-process PID, user, CPU%, RAM%, GPU%, VRAM%, and command
- Top-N process sorting by CPU, RAM, GPU, VRAM, or normalized total score
- TUI mode with keyboard control and width-responsive layout
- JSON snapshot mode and newline-delimited streaming with an optional count
- Case-insensitive command/user/PID filtering in both TUI and JSON
- Height-aware layouts, GPU paging, and process scrolling
- Single release binary with no `libsensors` or `nvidia-smi` runtime dependency

Out of current scope: AMD/Intel GPU telemetry, Prometheus export, container
attribution, themes, and detailed per-process drill-down. Memory management is
an optional companion with its own contract in `ramcleaner/README.md`; normal
monitor installation does not enable it.

---

## 2. GPU Behavior

### 2.1 NVML loading

Runtime loading tries `libnvidia-ml.so` first and falls back to
`libnvidia-ml.so.1`, because many server installs ship only the versioned
runtime soname. If NVML is unavailable, ZTOP continues with an `n/a` GPU panel.
`--no-gpu` skips NVML loading and initialization entirely.

Only when system NVML initialization reports a driver/library version mismatch
may ZTOP try an administrator-provided private runtime. For an executable in
`$PREFIX/bin`, its path is
`$PREFIX/lib/ztop/nvml/<loaded-driver-version>/libnvidia-ml.so.1`. The version
comes from `/sys/module/nvidia/version` and must have three numeric dotted
components. Healthy system NVML always wins; permission and missing-driver
errors do not trigger private loading. Both the wrapper and raw v2 memory
session load the same selected library. Private fallback use prints a warning
that the host driver/library mismatch still requires repair. ZTOP does not
download runtimes or change kernel modules.

### 2.2 Per-process GPU / VRAM

| Field             | NVML source                                                                          | Privilege                                      |
| ----------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------- |
| Per-process VRAM  | `running_compute_processes` + `running_graphics_processes`                           | Usually unprivileged                           |
| Per-process GPU % | `process_utilization_stats` / `nvmlDeviceGetProcessUtilization`                      | Driver-dependent; often root or `CAP_SYS_ADMIN`|

If per-process GPU utilization is unavailable, the process-table GPU% column
renders `—`. Device-level GPU utilization still renders when NVML provides it.

`--no-pid-gpu` keeps device-level GPU panels but disables per-process NVML
inventory and accounting calls. Per-process GPU% and VRAM are unavailable and
render `—`. The driver may still open `/dev/nvidia-uvm` during NVML initialization
or device telemetry; this flag makes no guarantee about UVM descriptors.
Use `--no-gpu` to avoid NVML and GPU device access entirely.

### 2.3 Multi-GPU aggregation

Every NVIDIA device gets its own panel. Per-process VRAM is summed across
devices. Per-process GPU% reports the busiest device for that PID so the value
stays on a 0..100 scale.

GPU identity is an NVIDIA UUID, with a stable index fallback when unavailable.
The physical NVML index is also exposed; labels and order do not depend on which
device first produces a successful sample.
Peak histories and failure grace are keyed by identity. Devices remain in the
snapshot during partial failure and recovery. For failed polls 1–7 the device
status is `stale`; at 8 or more it is `unavailable`. The last observation time
stops advancing, and the UI labels stale data or displays `n/a` after expiry.
Per-process data is cleared immediately when its device sample fails.

Unsupported or failed utilization/temperature readings are `null`, never an
invented zero. No-sample and temporary per-process utilization errors remain
retryable; a successful device utilization reading is used without replacing it
with a sum of process samples.

### 2.4 Diagnostics

Default startup should be quiet except for actionable failures. Detailed GPU
initialization diagnostics are available through `--verbose`; per-poll GPU
diagnostics are written to `ZTOP_DEBUG_GPU`'s file target when that environment
variable is set.

---

## 3. Data Sources

### 3.1 CPU and memory

CPU, RAM, swap, process CPU, process memory, process user, and process command
come from `sysinfo`. The process refresh must stay minimal:

```text
ProcessRefreshKind::nothing().without_tasks()
  + cpu
  + memory
  + user OnlyIfNotSet
  + cmd OnlyIfNotSet
```

Use the same minimal settings during initialization, and refresh CPU usage
without unused frequency counters. Only process leaders belong in process rows.

The physical core count comes from `System::physical_core_count()`. sysinfo
re-parses `/proc/cpuinfo` on every call, so the collector reads it once at
startup and copies the cached value into every snapshot. It is `None` when the
platform hides the topology; the thread count is the number of logical CPUs
(`sys.cpus().len()`), which is also the length of the per-core usage list.

Do not re-enable sysinfo's default per-thread process walk, disk I/O counters,
or `exe` symlink refresh in the hot path without a benchmark showing the cost
is acceptable.

### 3.2 CPU temperature

Order of attempts:

1. Scan `/sys/class/hwmon/*/name`. Prefer `coretemp`, `k10temp`, or `zenpower`.
2. Within the selected hwmon, prefer labels `Package id 0`, `Tctl`, or `Tdie`.
3. If no matching label exists, use `temp1_input`.
4. Fallback to `/sys/class/thermal/thermal_zone*`, preferring `x86_pkg_temp`.
5. Values are millidegrees Celsius and are divided by 1000.

No `libsensors` dependency is allowed in the current design.

### 3.3 Disks

Disk data comes from the kernel only; no `lsblk`, `df`, or udev at run time.

**Physical disks** are the entries of `/sys/block` that have a `device` link
(hardware-backed; loop, zram, device-mapper, and MD devices have none) and a
non-zero `size`. Optical drives (`sr*`), eMMC boot and RPMB areas, and hidden
NVMe multipath paths are excluded. Disks are ordered by name with digit runs
compared as numbers (`nvme2n1` before `nvme10n1`).

**Filesystems** come from `/proc/self/mountinfo`. Only filesystems on local
block devices count:

1. A non-zero `major:minor` is the filesystem's block device; its kernel name
   is the target of `/sys/dev/block/<major>:<minor>`.
2. btrfs numbers itself (major 0): its mount source is resolved to one member
   device, and `/sys/fs/btrfs/<fsid>/devices/` lists the others.
3. Everything else with major 0 is skipped: ZFS, network and FUSE filesystems,
   overlay, tmpfs. ZFS pools are therefore not attributed to their disks.

**Attribution** walks sysfs from the block device to the disks under it: a
partition belongs to its parent device, and a device with entries in `slaves`
(LVM, LUKS, MD RAID) rests on those, to at most eight levels. Devices that end
on no physical disk (loop images, zram) are not counted. Every member of an
MD RAID 1 array (`md/level` reads `raid1`) holds all of what the array holds,
so each disk of a mirror reports the whole filesystem, as `df` does. Anything
else on several devices (LVM, a stripe, a parity array, btrfs) is divided
between them in proportion to their sizes, so those per-disk numbers are
shares that add up to the filesystem's totals rather than physical occupancy.

**Usage** is `statfs(2)` on one mount point per filesystem, however often it
is mounted: capacity `f_blocks * f_frsize` and used
`(f_blocks - f_bfree) * f_frsize`, the Size and Used columns of `df`, and
available `f_bavail * f_frsize`, its Avail column, which leaves out the blocks
reserved for root. The percentage shown is `df`'s Use%,
`used / (used + available)`. A mount point is used only if `stat(2)` still
reports the filesystem's device number there (not checked for btrfs, whose
subvolumes have numbers of their own); otherwise the next mount point of the
same filesystem is tried. A disk with no measurable filesystem reports
`filesystems = 0` and zero usage.

The disk and mount layout is rescanned at most every 5 seconds, because
reading the mount table costs time in proportion to the number of mounts.
Usage is read on every collection.

**Model and kind** are read at each layout scan:

1. The model is `/sys/block/<disk>/device/model` (`name` for a memory card),
   with runs of spaces collapsed. An NVMe controller reports the whole model
   name, a SCSI device 16 characters. The SCSI `vendor` is put in front unless
   it is `ATA`, the placeholder of every SATA drive, or the model already
   starts with it. No udev database is read, so a SATA model stays cut short.
2. The kind is the bus and the medium. `nvme*` is `NVMe SSD`; `mmcblk*` is
   `SD card`, `eMMC` or `memory card` by `device/type`. For any other disk the
   directories between the root of sysfs and the disk tell the bus: a
   `usb<N>` directory makes it `USB disk`, a `virtio<N>` or `vbd-*` directory
   `virtual disk`, an `ata<N>` directory `SATA`, an `end_device-*` directory
   `SAS`, and any other `sd*` disk `SCSI`. SATA, SAS and SCSI are followed by
   `SSD`, `HDD` or `disk` as `queue/rotational` reads `0`, `1` or nothing.
3. Either is `null` where the kernel says nothing.

**Temperature** is the hwmon device sysfs lists under the disk's own
hardware, `/sys/block/<disk>/device`; no `smartctl`, `libsensors`, or SMART
ioctl, and no privilege:

1. Directly under an NVMe controller: `hwmon<N>/temp1_input`, the Composite
   temperature. Where `device` is an NVMe subsystem (a namespace reached
   through several controllers), every `nvme<N>` entry in it is a controller
   and is looked into the same way.
2. In the `hwmon` directory of a SCSI device: `hwmon/hwmon<N>/temp1_input`,
   present for SATA and SAS disks while the `drivetemp` module is loaded.
   ZTOP does not load modules.
3. Values are millidegrees Celsius, rounded to whole degrees. A value outside
   -50..=150 is no reading (a drive that measures nothing reports zero
   kelvin). Several sensors for one disk yield the highest.

A disk with no such sensor reports `temp_c = null`. Reading the file makes the
kernel query the drive, so temperatures are read once per layout scan, at most
every 5 seconds, and repeated in between. `--no-temp` (`show_temperatures =
false`) skips the readings entirely, as it does for the CPU sensor.

---

## 4. Sorting

Default sort is `cpu`.

The `total` mode normalizes CPU to a device-wide percentage before comparing it
with RAM/GPU/VRAM:

```text
score = max(cpu_pct / n_logical_cpus, ram_pct, gpu_pct, vram_pct)
```

The process table displays VRAM% with one decimal place, computed against the
sum of VRAM totals across all detected GPUs, including panels on other pages.
Missing process VRAM or total capacity renders `—`. The same denominator is
used for normalized total sorting.
Missing per-process GPU percentages sort as zero.
Non-finite readings also sort as zero. Ties are broken by ascending PID, so
equal rows (typically idle processes) keep a stable order between refreshes.
The table header marks the active sort column with `▼` (`total` has no column
of its own and is shown only in the footer).

---

## 5. CLI

```text
USAGE:
    ztop [OPTIONS]

OPTIONS:
    -r, --refresh <SECONDS>       Refresh interval (0.25..=5.0) [default: 1.0]
    -s, --sort <MODE>             Initial sort: cpu|ram|gpu|vram|total
    -n, --top <N>                 Number of processes shown (1..=200)
        --gpu-peak-hold <SECONDS> Rolling GPU peak hold (0..=30, 0 disables)
        --no-gpu                  Skip GPU initialization entirely
        --no-pid-gpu              Skip per-process GPU/VRAM accounting
        --no-temp                 Skip temperature probing
        --filter <TEXT>           Match command, user, or PID before top-N
        --gpu / --pid-gpu / --temp Enable a feature disabled by config
        --config <PATH>           Select an explicit config file
        --no-config               Ignore the default config file
        --json                    Print one JSON snapshot and exit
        --json-stream             Emit one compact JSON object per line
        --count <N>               Stop after N snapshots (requires stream)
        --verbose                 Print detailed startup diagnostics
    -h, --help                    Print help
    -V, --version                 Print version and copyright
```

Command-line flags override config-file values. Positive and negative feature
flags conflict; JSON modes are mutually exclusive. `--count` must be positive.
An explicitly selected missing config file is an error.

---

## 6. Configuration

Optional config path:

```text
~/.config/ztop/config.toml
```

`$XDG_CONFIG_HOME` is honored. All keys are optional; unknown keys and
out-of-range values are errors.

```toml
refresh_interval = 1.0    # 0.25..=5.0 seconds
sort_by = "cpu"           # cpu | ram | gpu | vram | total
show_gpu = true           # false = --no-gpu
show_temperatures = true  # false = --no-temp
pid_gpu = true            # false = --no-pid-gpu
top_processes = 10        # 1..=200
gpu_peak_hold = 3.0       # 0..=30 seconds
process_filter = ""       # case-insensitive command/user/PID substring
```

---

## 7. JSON Snapshot Contract

`ztop --json` collects one snapshot, truncates process rows to `--top`, writes
pretty-printed JSON to stdout, and exits without entering alternate-screen TUI
mode.

`--json-stream` emits one compact JSON object per line, flushes each snapshot,
and reuses the collector at the selected refresh interval. GPU sampling retains
its independent cadence between outputs; output backpressure delays collection. `--count N` stops after N snapshots;
a closed output pipe is a normal exit. Both modes filter before truncating rows.

Current schema version: `2`.

Top-level fields:

| Field                | Type        | Unit / meaning                          |
| -------------------- | ----------- | --------------------------------------- |
| `schema_version`     | integer     | JSON schema version, currently `2`      |
| `sampled_at_unix_ms` | integer     | UTC Unix timestamp in milliseconds      |
| `hostname`           | string      | Hostname or `unknown`                   |
| `uptime_secs`        | integer     | System uptime in seconds                |
| `cpu_brand`          | string      | First CPU brand string or `unknown CPU` |
| `cpu_physical_cores` | integer/null | Physical cores; `null` when unknown    |
| `cpu_total_pct`      | number      | Percent                                 |
| `cpu_per_core_pct`   | number list | Percent per logical CPU                 |
| `cpu_temp_c`         | number/null | Degrees Celsius                         |
| `ram_*_bytes`        | integer     | Bytes                                   |
| `swap_*_bytes`       | integer     | Bytes                                   |
| `gpus`               | array       | Device snapshots                        |
| `disks`              | array       | Physical disks, in name order           |
| `procs`              | array       | Top process rows                        |

GPU snapshots contain `id`, `index`, `name`, `sampled_at_unix_ms`, `status`, `util_pct`,
`util_peak_pct`, `temp_c`, `vram_used_bytes`, `vram_peak_used_bytes`, and
`vram_total_bytes`. `util_pct`, `util_peak_pct`, and `temp_c` are nullable.
`status` is `fresh`, `stale`, or `unavailable`; fresh means a device memory sample
succeeded, while individual sensor fields may still be null. Memory fields on
stale/unavailable devices are the last observed values, not current measurements.
The device timestamp identifies that observation. Per-PID GPU maps remain internal.
Schema 2 changes utilization and temperature to nullable fields; schema 1 clients
must update their readers before consuming these outputs. `cpu_physical_cores`
was added in 0.9.6 as an additive nullable field without a schema bump.

Disk records contain `name` (kernel device name), `model` and `kind` (the
drive's model and what sort of drive it is, section 3.3, each `null` when
unknown), `size_bytes` (raw device
capacity), `filesystems` (mounted filesystems measured on the disk),
`fs_used_bytes` / `fs_total_bytes` (the disk's share of their used space and
capacity, section 3.3), `fs_available_bytes`, and `temp_c` (the drive's
temperature in whole degrees Celsius, `null` without a sensor or with
`--no-temp`). The byte fields are `0` when `filesystems` is `0`. The
array is empty where no physical disk can be found. `disks` was added in
0.9.14, `fs_available_bytes` in 0.9.15, `temp_c` in 0.9.16, and `model` and
`kind` in 0.9.17, each as an additive field without a schema bump.

Process rows contain `pid`, `user`, `cpu_pct`, `ram_pct`, `gpu_pct`,
`vram_bytes`, and `command`.

Backward-incompatible JSON changes require incrementing `schema_version`.

---

## 8. Keyboard Interaction

| Key       | Action                                          |
| --------- | ----------------------------------------------- |
| `q` / Ctrl-C | Quit (Ctrl-C also quits during search)         |
| `c`       | Sort by CPU                                     |
| `r`       | Sort by RAM                                     |
| `g`       | Sort by GPU                                     |
| `v`       | Sort by VRAM                                    |
| `t`       | Sort by total                                   |
| Space     | Pause / resume refresh and GPU sampling         |
| `+` / `-` | Refresh rate +/- 0.25 s (clamped 0.25..=5.0)    |
| `[` / `]` | Process rows shown +/- 1 (clamped 1..=200)      |
| `?`       | Toggle key-binding help overlay                 |
| `/`       | Edit a command/user/PID filter                   |
| Enter     | Accept the edited filter                        |
| Esc       | Cancel search, close help, clear filter, then quit |
| Ctrl-U    | Clear the filter while editing                  |
| Up/Down   | Scroll through matching top-N processes         |
| PageUp/PageDown | Scroll by the visible process page        |
| Home/End  | Jump to the start/end of matching top-N rows     |
| `C`       | Toggle per-core details (subject to available height) |
| `{` / `}` | Page through GPUs                               |

---

## 9. Layout and Width Rules

Minimum render width is 40 columns and height is 10 rows. Below the width minimum, render only:

```text
terminal too narrow (need ≥ 40 cols)
```

Process-table shrinkage:

1. `< 76` columns: drop VRAM column
2. `< 70` columns: drop GPU% column
3. `< 60` columns: drop USER column

Metric bars are hidden naturally when the bar width would be smaller than 4
columns. Per-core CPU rows auto-wrap based on terminal width and the core index
width, so high-core-count hosts remain readable on wide terminals.

The CPU panel title shows the core/thread counts only:
` CPU · 12 cores / 14 threads ` (`14 threads` alone when the physical count is
unknown), falling back to ` CPU ` when the counts do not fit the panel width.
It carries no key hints or auto-fit notes: `C` toggles the per-core rows and
auto-fit limits them silently; both are documented in the help overlay. The
first panel line is the brand string. Compact CPU summaries show the same counts.

The header shows the full `vMAJOR.MINOR.PATCH+YYYYMMDD.HHMM` version at widths
`>= 100` and the shorter release version below that.

Height >= 22 reserves at least eight rows for the process table (five data rows)
and keeps a two-row footer. Smaller terminals use compact CPU/RAM/GPU summaries;
heights below 10 show a minimum-height message. Core details collapse when space
is limited. GPU panels compact/page rather than displacing the process viewport.
The table reports visible range and match count, and clamps scrolling after
filter, top-N, process-list, or terminal-size changes. Numeric columns expand for
long PIDs and large CPU values; truncation respects grapheme/display width.
Memory units shown in the TUI are GiB/MiB (powers of 1024).

RAM, swap, and VRAM rows share one shape,
`LABEL  used / total UNIT  pct%  [bar]`. Both numbers are five columns wide:
one decimal below 1000 GiB, whole GiB below 100000 GiB, and TiB with one
decimal beyond.

CPU, GPU, and disk share another: a line in the brand colour that says what
the device is, and under it `LABEL  pct%  temp: reading  [bar]`, where the
label is the generic `CPU`, `GPU`, or `Disk`. Temperatures have one form on
all three: the label `temp` in the label colour, `: `, and the reading in the
temperature colour, after the percentage and before the bar. An unavailable
reading is `n/a`. The CPU reading has one decimal; GPU and disk readings are
whole degrees.

Every bar starts in the same column, whichever of the two shapes its row has.
No row may carry so much text that its bar starts further right.

The Disks panel sits between the GPU panels and the process table. It has two
rows per physical disk:

```text
WD_BLACK SN850X 2000GB · NVMe SSD · nvme0n1 | 441 of 1740 GiB used
Disk   25.3%   temp: 52°C         [████████░░░░░░░░░░░░░░░░░░░░░░░░]
```

The first is the model, the kind, and the kernel device name, joined by ` · `
with whatever is unknown left out, then ` | ` and the used space and capacity
of the filesystems mounted on the disk (section 3.3) in the unit and
precision of the capacity. It is cut at the panel's width. The second is
`Disk`, the percentage (`df`'s Use%), the temperature, and the bar; it never
carries the device name or the sizes. A disk with `filesystems = 0` reads
`| no mounted filesystem` in its first row and has `—` for the percentage and
no bar. A disk without a sensor, and every disk with `--no-temp`, reads
`temp: n/a`. The title is ` Disks · <n> physical · <total raw capacity> `,
falling back to ` Disks ` when it does not fit.

The panel ranks last among the panels: after per-core rows, GPU panel count,
and GPU panel height. It lists at most four disks, two rows each, and appears
only while the
process table keeps ten rows, two more than its minimum; compact layouts
never show it. When there are more disks than fit, the fullest by used
fraction are listed, still in name order, disks without a mounted filesystem
give way first, and the title adds `· <k> fullest shown`.

Rows left over by the panels go to the process table. The TUI keeps no
per-metric sample history: apart from the GPU peak hold, each frame shows only
the current snapshot.

---

## 10. Module Layout

```text
src/
  main.rs            entrypoint, terminal setup/teardown, event loop
  output.rs          one-shot and streaming JSON, filtering, pipe handling
  input.rs           tested key-to-state transitions
  cli.rs             clap definition, help text, value validation
  config.rs          optional TOML config file and CLI precedence
  app.rs             state, filtering, navigation
  metrics/
    mod.rs           Snapshot, Collector, GPU sample cadence, peak history
    cpu.rs           CPU total/per-core/brand helpers
    disk/
      mod.rs         DiskSnapshot, platform dispatch
      linux.rs       sysfs + mountinfo attribution, statfs, layout cache
    memory.rs        RAM and swap helpers
    temp.rs          hwmon + thermal_zone reader
    gpu.rs           GpuBackend trait, NVML implementation, raw v2 memory API
    process.rs       process collection, command cache, sorting
  ui/
    mod.rs           top-level draw and responsive layout helpers
    bars.rs          horizontal bar widget
    table.rs         process table widget
```

---

## 11. Refresh Cadence and Cost Budget

- Event loop polls input with a maximum 100 ms wait.
- Full metric collection happens only on refresh ticks.
- GPU sampling runs every 250 ms while unpaused and is independent from full
  UI refresh.
- Paused mode stops GPU sampling too.
- The disk and mount layout is rescanned at most every 5 seconds; between
  scans a collection costs one `stat` and one `statfs` per mounted
  filesystem.
- Each scan reads one temperature file per physical disk, which is one query
  to the drive; none is read between scans or with `--no-temp`.
- Default steady-state budget: target under 1% CPU on an 8-logical-core host
  at 1 Hz.

---

## 12. Required Verification Before Public Release

Automated:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo test --locked --release`
- [ ] `shellcheck installer.sh ramcleaner/ztop-ramcleaner scripts/version.sh tests/version.sh`
- [ ] `scripts/version.sh --check` and `tests/version.sh`
- [ ] `"$HOME/.venv/bin/python" -m unittest discover -s tests/shell -v` (fixtures only)
- [ ] `ZTOP_TEST_BIN=./target/release/ztop "$HOME/.venv/bin/python" tests/runtime.py`
- [ ] `./installer.sh --check`
- [ ] `ztop --json --top 1 --no-gpu --no-temp` returns schema version `2`

Manual / hardware:

- [ ] Unprivileged run on a host without NVIDIA: no panic, GPU panel is `n/a`.
- [ ] Unprivileged run with NVIDIA: GPU panels populated; per-process GPU% may
      show `—`.
- [ ] Root or `CAP_SYS_ADMIN` run with NVIDIA: per-process GPU% populated when
      the driver supports it.
- [ ] `--no-pid-gpu` keeps device telemetry while disabling per-process
      inventory/accounting; process GPU% and VRAM render `—`.
- [ ] `--no-gpu` skips NVML and opens no NVIDIA devices.
- [ ] Multi-GPU host shows one panel per device and sums process VRAM.
- [ ] Multi-disk host shows two rows per physical disk; a disk with nothing
      mounted says `no mounted filesystem` and has no percentage or bar.
- [ ] Each disk's first row names its model and kind as `lsblk -d -o
      MODEL,TRAN,ROTA` does (a SATA model may be cut to 16 characters).
- [ ] The bar of every disk starts in the column of the CPU, RAM, GPU and
      VRAM bars at 80, 100 and wide widths.
- [ ] NVMe disks show `temp:` matching `smartctl -A`; SATA disks show it
      once `drivetemp` is loaded and `n/a` before that; `--no-temp` reads
      `n/a` on every disk.
- [ ] Terminal resize behaves at 40, 50, 60, 70, 76, 80, 100, and wide widths.
- [ ] Idle CPU cost is measured and recorded for the release candidate.

---

## 13. Installer Verification

- Build as the normal user with the locked dependency graph. Acquire the
  checkout's mutation lock before building or installing.
- Every mutating run restores ownership of the entire checkout, `.git/`
  included, to the installing user (`ZTOP_OWNER=user:group` overrides it)
  immediately after acquiring the lock and again on exit, success or failure.
  `sudo chown -Rf` runs only when foreign-owned entries exist. `--help` and
  `--check` remain read-only.
- A writable user prefix requires no sudo for the monitor copy. Authenticate
  when system packages or a protected installation destination require it.
- NVIDIA preflight preserves `nvidia-smi` errors and reports every device.
  Runtime/device files without `nvidia-smi` indicate expected GPU support, not
  successful access. The installer does not install NVIDIA development packages.
- After installation, verify the binary with
  `--no-config --gpu --no-pid-gpu --no-temp --json --top 1`. Require fresh
  telemetry for every reported NVIDIA device. A working private runtime may
  satisfy this check even when system `nvidia-smi` remains broken.
- Return nonzero if expected NVIDIA telemetry fails; explain that the binary
  is installed but GPU monitoring needs repair. A host with no detected NVIDIA
  runtime may pass with an empty GPU array.
- `--check` performs the same read-only verification against an existing
  installed binary and sources the existing Cargo environment. It does not
  install packages or reload drivers. Private libraries remain administrator
  managed and are preserved by uninstall.

---

## 14. Checkout Ownership

The entire checkout, every directory and file including `.git/`, must always be
owned by the project user `spabam` (`spabam:spabam`). This is mandatory: the
installer refuses foreign-owned build artifacts, the Dropbox client cannot
reliably sync files it does not own, and git run as another user rewrites
`.git/index` with the wrong owner. Any process that runs as root inside the
checkout (elevated shells, assistant sessions, scheduled jobs) must build
outside the tree (`CARGO_TARGET_DIR`) and restore ownership with
`chown -R spabam:spabam` before it finishes. `find . -not -user spabam` must
print nothing. The installer enforces this at the start and end of every
mutating run (§13). See README §Install.

---

© 2026 Andrea Bodei <info@andreabodei.com> — Dual-licensed MIT OR Apache-2.0.
