# ZTOP

`ZTOP` is a tool to monitor CPU, RAM, GPU, VRAM in the top/htop style and with modern compatibility to nvidia drivers (it works where btop fails). Written from scratch in Rust. `by Andrea Bodei info@andreabodei.com 2026`.

**Status:** v0.9.13+20261123.1607 (unreleased development build). See [Versioning](#versioning), [`CHANGELOG.md`](./CHANGELOG.md), and [`SPEC.md`](./SPEC.md) for release history and the current engineering contract.

A simple, clean, real-time Linux system monitor inspired by `btop`, focused on the essential metrics:

- CPU model and usage
- Per-core CPU usage
- CPU temperature
- RAM usage
- Swap usage
- GPU model, usage and temperature
- VRAM usage
- Top 10 most resource-consuming commands
- User running each command

ZTOP is designed for Linux workstations, GPU servers, AI research machines, cybersecurity labs, and developers who need a fast terminal dashboard without unnecessary complexity.

---

## Goal

ZTOP aims to provide a minimal and readable terminal interface updated every second.

Unlike general-purpose monitors, ZTOP treats GPU and VRAM usage as first-class metrics.

The objective is not to replace `btop`, `htop`, or `nvtop`, but to provide a simpler tool focused on:

```text
CPU + RAM + GPU + VRAM + temperature + top consuming commands
```

---

## Example Interface

Conceptual layout (the actual render adds horizontal bars and a footer with live sort/pause state — see [`SPEC.md`](./SPEC.md#9-layout-and-width-rules) for the verified 80-column layout):

```text
┌──────────────────────────── ZTOP ────────────────────────────┐
│ Refresh: 1s        Host: workstation-01        Uptime: 04:21:33 │
├ CPU · 24 cores / 32 threads ───────────────────────────────────┤
│ 13th Gen Intel(R) Core(TM) i9-13980HX                          │
│ CPU      42%   Temp: 68°C                                       │
│ Cores    0: 31%  1: 46%  2: 77%  3: 22%                         │
│          4: 55%  5: 39%  6: 81%  7: 18%                         │
├─────────────────────────────────────────────────────────────────┤
│ RAM      18.4 / 32.0 GB    57%                                  │
│ SWAP      0.6 / 31.6 GB     1.8%                                │
├─────────────────────────────────────────────────────────────────┤
│ NVIDIA GeForce RTX 4070 Laptop GPU                              │
│ GPU      73%   Temp: 71°C                                       │
│ VRAM     9.2 / 12.0 GB     76%                                  │
├──────────────────── Top 10 Consuming Commands ──────────────────┤
│ PID     USER       CPU%   RAM%   GPU%   VRAM%    COMMAND        │
│ 8123    giaime     188    12.1   65     65.0     python train.py │
│ 2411    root       24     4.3    0      —        Xorg           │
│ 9931    giaime     18     2.1    4      5.0      blender        │
└─────────────────────────────────────────────────────────────────┘
```

The header shows the running version right next to the `ZTOP` title (e.g. `ZTOP v0.9.3+20261026.0923`), so the dashboard itself tells you which build you are looking at; terminals narrower than 100 columns show the bare `vMAJOR.MINOR.PATCH` to keep host and uptime visible.

The CPU panel title carries the physical core and logical CPU (thread) counts, e.g. ` CPU · 12 cores / 14 threads `, directly above the CPU model line. On platforms that hide the physical topology (some containers and virtual machines) only the thread count is shown. The title carries no key hints: press `C` to hide or show the per-core rows, as listed in the `?` help overlay. The compact short-terminal summary carries the same counts.

The layout adapts to terminal height as well as width. It reserves space for processes, automatically collapses core details, and pages GPU panels when necessary. `C` toggles core details; `{` / `}` change GPU pages. Rows the panels leave over go to the process table.

Per-core layout automatically uses the available terminal width, so wide terminals pack more cores per row and high-core-count servers need fewer CPU-panel rows. The process table drops columns as the terminal narrows (VRAM below 76 cols, GPU% below 70, USER below 60).

### Colors

The live UI uses a small palette so the eye can separate labels from values at a glance:

| Style              | Meaning                                                |
| ------------------ | ------------------------------------------------------ |
| **Cyan**           | Section labels (`CPU`, `RAM`, `SWAP`, `GPU`, `VRAM`, `temp`, `Cores`), header field names (`refresh:`, `host:`, `uptime:`), footer keybinds, process-table column headers |
| **Grey**           | Brand strings — CPU model on the first line of the CPU panel, GPU model on the first line of the GPU panel |
| **Yellow**         | Temperature readings                                    |
| **Green / yellow / red** | Bar fill, load-graded: green < 50 %, yellow 50–80 %, red ≥ 80 % |
| **Dark gray**      | Empty bar cells                                         |
| **Default**        | Numeric values, sizes, percentages, PIDs, usernames, command names |

The scheme works on any 256-color or truecolor terminal; on a strictly 16-color terminal the dark-gray empty cells may fall back to plain gray.

---

## Features

### System Metrics

- CPU model (brand string, e.g. `13th Gen Intel(R) Core(TM) i9-13980HX`) with its physical core and thread counts in the panel title (`24 cores / 32 threads`)
- Total CPU usage
- Per-core CPU usage
- CPU temperature
- RAM used and total RAM
- RAM percentage usage
- Swap used and total
- Swap percentage usage
- System uptime
- Hostname

### GPU Metrics

Initial target: NVIDIA GPUs.

- GPU model (e.g. `NVIDIA GeForce RTX 4070 Laptop GPU`, displayed on the first line of the GPU panel)
- GPU utilization
- GPU temperature
- VRAM used
- VRAM total
- VRAM percentage usage
- GPU process usage where available

Multi-GPU systems are supported: every NVIDIA device gets its own panel
(labeled `GPU0:`, `GPU1:`, … when more than one is present), and per-process
VRAM is summed across devices while per-process GPU% reports the busiest
device so it stays on a 0–100 scale.

Future support:

- AMD GPUs
- Intel GPUs

### Process Table

ZTOP displays the top 10 most resource-consuming commands.

Each process row includes:

- PID
- Linux user
- CPU percentage
- RAM percentage
- GPU percentage, where available
- VRAM percentage, where available (process VRAM divided by total memory across all detected GPUs)
- Command name

---

## Refresh Rate

Default refresh interval:

```bash
1 second
```

The interface should update continuously without flickering or excessive CPU usage.
GPU metrics are sampled every 250 ms independently from the main UI refresh. The GPU panel keeps a rolling peak for short utilization spikes, defaulting to 3 seconds.

Example:

```bash
ztop --refresh 1
```

---

## Install

For a platform deployment, run `bash installer.sh --only ztop --dry-run` from
the platform checkout root. Use `--apply` to build and install the monitor.
When root runs this entry point it sends the pinned Rust sources to an
unprivileged builder and publishes the verified executable to
`${PREFIX:-/usr}/bin/ztop`. A root-owned checkout with no invoking user uses a
temporary systemd build identity. The receiver needs the Rust dependencies
available locally or reachable during the build. From a shell on the receiving
machine, `ztop --version` reports the build and `ztop --no-config --no-gpu
--json --top 1` prints one CPU/RAM sample with config-file settings disabled.
The optional ramcleaner companion has its own installation command below.

The bundled installer bootstraps any missing dependencies, builds in release mode, and copies the binary into `/usr/bin/ztop`:

```bash
./installer.sh
```

If `cargo` is missing or the active `rustc` is older than 1.88, the installer fetches the pinned Rust 1.88.0 toolchain automatically via rustup (minimal profile). Long Rust/toolchain/build steps print elapsed progress lines so the install does not appear stalled while downloads are running. Missing C build tools (`cc`, `make`) come from your distro's package manager (apt / dnf / yum / pacman / zypper / apk are auto-detected). The NVIDIA driver is **not** auto-installed (kernel modules and secure boot make that unsafe from a shell script); the installer warns if it's missing, and ztop will still run with the GPU panel showing `n/a`.

Run the installer as your normal user. `sudo` is invoked only for the package install + final copy steps; the cargo build runs as the current user. On hardware where NVML needs elevated privileges for per-process GPU utilization, install normally and run the installed binary with `sudo ztop`.

### File ownership (mandatory)

The entire checkout, every directory and file, including `.git/`, the docs, the
scripts, the tests, and anything a tool, build, or assistant session creates,
**must always be owned by the project user, `spabam`** (`spabam:spabam`). Files
owned by `root` or any other user must never remain in the tree: the installer
refuses to build over foreign-owned artifacts, the Dropbox client runs as
`spabam` and cannot reliably sync files it does not own, and git commands run as
another user rewrite `.git/index` with the wrong owner. Anything that runs as
root inside this folder (an elevated shell, an assistant session, a scheduled
job) must build outside the tree (`CARGO_TARGET_DIR`) and restore ownership
before it finishes:

```bash
sudo chown -R spabam:spabam /home/spabam/Dropbox/Public/VANTAGE/AI/ZTOP
find /home/spabam/Dropbox/Public/VANTAGE/AI/ZTOP -not -user spabam   # must print nothing
```

The installer enforces this rule itself: every mutating run (`./installer.sh`,
`--uninstall`, and the ramcleaner commands) restores ownership of the entire
checkout, `.git/` included, to the installing user right after taking its lock
and again when it exits, whether it succeeded or failed. It calls
`sudo chown -Rf` only when something in the tree is owned by another user, so a
clean checkout never prompts. `--help` and `--check` stay read-only. Set
`ZTOP_OWNER=user:group` to override the detected owner; on this project it must
be `spabam:spabam`.

The release build is written to `$XDG_CACHE_HOME/ztop-target` (normally
`~/.cache/ztop-target`), not to the checkout's `target/`. This keeps build
artifacts out of the source tree and away from file-sync clients: Dropbox, for
example, renames a selectively excluded `target/` to
`target (Selective Sync Conflict)` seconds after cargo creates it, and the
build then fails with `No such file or directory`. Set `CARGO_TARGET_DIR` to
build somewhere else:

```bash
CARGO_TARGET_DIR=/var/tmp/ztop-build ./installer.sh
```

If cached Cargo build scripts have lost their execute permissions (for example,
after copying a build cache), the installer restores those permissions before
building. It also restores the release binary's execute permission if needed.
If execution is still blocked by a `noexec` mount, set `CARGO_TARGET_DIR` to a
writable directory on a filesystem that allows execution and rerun the installer.

Preview what would change without touching anything:

```bash
./installer.sh --check
```

The NVIDIA check preserves driver errors and verifies fresh GPU samples from
the installed monitor. A detected but unusable NVIDIA setup makes `--check`
return a nonzero status; machines without NVIDIA hardware can still install
and use the CPU/RAM monitor. NVML's runtime library (`libnvidia-ml.so.1`) is
sufficient; NVIDIA development packages are not required.
Normal installation performs the same GPU verification after copying the
binary. If it fails, the installer exits nonzero and explains that the monitor
is installed but GPU monitoring still needs repair. Verification ignores user
configuration and skips per-process accounting, so it works without elevated
GPU accounting privileges.

If NVML reports `Driver/library version mismatch`, the loaded NVIDIA kernel
driver and userspace library need matching versions. Schedule a host driver
repair or reboot after checking the installed kernel and driver. For a running
host that cannot restart yet, an administrator can supply the matching NVIDIA
NVML runtime at
`$PREFIX/lib/ztop/nvml/<loaded-driver-version>/libnvidia-ml.so.1` (normally under
`/usr/lib/ztop`). ZTOP tries this private library only after a system NVML
version mismatch and prints a warning when it uses it. A healthy system NVML
always takes precedence, including after reboot. This keeps ZTOP usable while
the system driver mismatch remains; the installer does not download driver
libraries or reload kernel modules. Private libraries are managed by the
administrator and are preserved by `--uninstall`; remove them after the host
driver setup is repaired.

Install under a different prefix:

```bash
PREFIX=/usr/local ./installer.sh
```

A writable user prefix also works without sudo when build prerequisites are
already installed:

```bash
PREFIX="$HOME/.local" ./installer.sh
```

Ensure `$HOME/.local/bin` is on your shell's `PATH` to run that copy as `ztop`.

Remove an installed copy (this also removes ramcleaner):

```bash
./installer.sh --uninstall
```

### Optional ramcleaner companion

The normal installer installs **only the monitor**. Memory management is a
separate, explicit choice:

```bash
./installer.sh --install-ramcleaner
sudo ztop-ramcleaner --status
sudo ztop-ramcleaner --dry-run --verbose
```

A new ramcleaner installation defaults to **preview mode** (`DRY_RUN=1`): it
logs decisions without changing memory settings. Existing operator configuration
is preserved. Review `/etc/ztop/ramcleaner.conf` before enabling actions.

| Tier | Behavior | Default |
|---|---|---|
| T1 | Request cgroup-v2 memory reclaim | Preview only |
| T2 | Request memory compaction | Preview only |
| T3 | Recycle a selected disk swap device after fresh headroom/load checks | Disabled |
| T4 | Stop services based on a verified idle probe | Unavailable; uptime alone is insufficient |

The companion records attempted and failed operations and exposes its latest
outcome in `--status`. It does not promise that reclaim, compaction, or swap
operations are free of performance impact or memory-pressure risk. T3 requires
explicit configuration and retains the device's swap priority when restoring it.
A fresh safety check reduces risk; it cannot reserve RAM against concurrent
allocations. Do not enable unattended swap recycling without evaluating it on
your workload.

```bash
sudo touch /run/ztop-ramcleaner.disable  # suspend scheduled passes
sudo ztop-ramcleaner --recover           # restore pending swap even while suspended
./installer.sh --uninstall-ramcleaner
```

Details, prerequisites, outcomes, and operator controls are documented in
[`ramcleaner/README.md`](./ramcleaner/README.md).

---

## Build and Run (manual)

ZTOP is written in Rust. With a stable toolchain (≥ 1.88):

```bash
cargo build --release
./target/release/ztop
```

The binary is about 2 MB stripped and has no runtime dependency on `libsensors` or `nvidia-smi` (NVML is loaded directly).

Some NVIDIA drivers expose per-process GPU utilization only to root or processes with `CAP_SYS_ADMIN`. In that case, build/install as your normal user to keep project files owned by you, then run the installed monitor as root:

```bash
sudo ztop
```

---

## Command-Line Usage

For a concise summary use `-h`; for the full feature tour, color legend, keyboard table, examples, and copyright run `ztop --help`:

```bash
ztop -h        # one-line option summary
ztop --help    # features + colors + keyboard + examples + copyright
```

Other invocations:

```bash
ztop
```

Set refresh interval (0.25–5.0 seconds):

```bash
ztop --refresh 1
```

Set GPU peak hold duration (0–30 seconds, default 3; `0` disables peak display):

```bash
ztop --gpu-peak-hold 5
```

Sort by CPU, RAM, GPU, VRAM, or total:

```bash
ztop --sort cpu
ztop --sort ram
ztop --sort gpu
ztop --sort vram
ztop --sort total
```

Number of processes shown (default 10, range 1–200):

```bash
ztop --top 15
```

Disable GPU monitoring and skip NVML entirely:

```bash
ztop --no-gpu
```

Keep the GPU panel but skip per-process inventory and VRAM/GPU% accounting. Per-process values are unavailable; their table columns read `—`:

```bash
ztop --no-pid-gpu
```

The NVIDIA driver may still open `/dev/nvidia-uvm` during NVML initialization or device telemetry, so `--no-pid-gpu` does not guarantee that ZTOP disappears from `lsof /dev/nvidia*`. Use `--no-gpu` when NVML initialization and GPU device access must be avoided.

Disable temperature probing:

```bash
ztop --no-temp
```

Print one metrics snapshot as pretty-printed JSON to stdout and exit — no TUI, suitable for scripts and cron jobs (`--top`/`--sort`/`--no-gpu` etc. apply as usual):

```bash
ztop --json
ztop --json --top 5 --sort vram
```

Print detailed startup diagnostics before entering the TUI:

```bash
ztop --verbose
```

Default startup is quiet except for actionable failures. For per-poll GPU
diagnostics, set `ZTOP_DEBUG_GPU` to a target log path (empty or `1` writes to
`ztop-gpu-debug.log` in the system temp directory).

### JSON Snapshot

`ztop --json` uses `schema_version: 2`. Memory and VRAM fields are bytes,
CPU/RAM/GPU fields ending in `_pct` are percentages, temperatures are Celsius,
and `sampled_at_unix_ms` is a UTC Unix timestamp in milliseconds.
`cpu_physical_cores` is the physical core count, or `null` when the platform
hides it; the thread (logical CPU) count is the length of `cpu_per_core_pct`.
Per-PID GPU maps are internal and are intentionally omitted from JSON. Backward-incompatible
JSON changes will bump `schema_version`. GPU records include a stable `id` and NVML `index`, a `status` (`fresh`, `stale`, or `unavailable`), and the time of their last observation. GPU utilization and temperature are `null` when unavailable; memory fields on stale/unavailable devices describe the last observation.

---

### Streaming and filtering

```bash
ztop --json-stream --refresh 1              # one compact JSON object per line
ztop --json-stream --count 10 --filter python
ztop --json --filter python --top 5
ztop --no-config --gpu                      # override config for diagnostics
ztop --config ./workstation.toml
```

Streaming uses the same refresh interval, sorting, and filtering as the TUI.
`--count` limits the number of snapshots; without it, streaming continues until
stopped. A slow output consumer applies backpressure to collection. Closing the output
pipe is a normal exit. JSON modes never enter the TUI.
Filtering is case-insensitive and matches command, user, or PID before applying
`--top`. Interactive `/` edits the filter, Enter applies it, and Escape cancels.
Arrow keys and PageUp/PageDown scroll through the selected top processes.

## Keyboard Bindings

| Key       | Action                                          |
| --------- | ----------------------------------------------- |
| `q` / Ctrl-C | Quit (Ctrl-C also works while searching)      |
| Esc       | Cancel search, close help, clear filter, then quit |
| `c`       | Sort by CPU                                     |
| `r`       | Sort by RAM                                     |
| `g`       | Sort by GPU                                     |
| `v`       | Sort by VRAM                                    |
| `t`       | Sort by total                                   |
| Space     | Pause / resume refresh (GPU sampling pauses too)|
| `+` / `-` | Refresh rate ± 0.25 s (clamped 0.25..5)         |
| `[` / `]` | Process rows shown ± 1 (clamped 1..200)         |
| `?`       | Toggle the key-binding help overlay             |

---

Additional controls: `/` searches; Enter accepts a search and Escape restores
the previous query. Escape outside search dismisses help, clears an active filter,
then quits. Up/Down, PageUp/PageDown, Home/End move through process rows;
`C` toggles core details; `{` / `}` select GPU pages.

## Sorting Logic

Default sort: `cpu` (matches `top` and `htop`, least surprise).

The `total` mode uses a *normalized* score so the four components are comparable on a 0..100 scale:

```text
score = max(cpu_pct / n_logical_cpus,  ram_pct,  gpu_pct,  vram_pct)
```

(Per-process CPU% from `sysinfo` is reported as a share of one core and can exceed 100 on multithreaded workloads; dividing by the logical-CPU count yields a device-wide share comparable to RAM/GPU/VRAM percentages.)

Available sorting modes:

```text
cpu     ram     gpu     vram     total
```

---

## Minimum Viable Product

Version `0.1` (implemented, unreleased) covers:

- CPU brand line at the top of the CPU panel
- CPU total usage
- Per-core CPU usage (auto-wraps by terminal width so wide servers fit more cores per row)
- CPU temperature (hwmon-first, thermal_zone fallback)
- RAM used and total RAM
- Swap used and total (panel displays "none configured" when there is no swap)
- NVIDIA GPU name at the top of the GPU panel
- NVIDIA GPU usage
- NVIDIA GPU temperature
- NVIDIA VRAM used and total VRAM
- Per-process VRAM via NVML (per-process GPU% degrades to `—` without root)
- Top 10 processes by resource usage
- Process owner
- Command name
- 1-second refresh (configurable 0.25–5 s)
- Clean terminal UI with horizontal bars and a live footer

---

## Suggested Tech Stack

### Preferred: Rust

Rust is recommended for the first stable implementation because it provides:

- High performance
- Low memory overhead
- Safe system-level programming
- Easy single-binary distribution

Suggested libraries:

```text
ratatui       Terminal UI
crossterm     Terminal input/output
sysinfo       CPU, RAM, and process data
nvml-wrapper  NVIDIA GPU metrics
```

---

## Data Sources

### CPU and RAM

Possible Linux sources:

```text
/proc/stat
/proc/meminfo
/proc/[pid]/stat
/proc/[pid]/status
```

Alternative library:

```text
sysinfo
```

### CPU Temperature

ZTOP reads from sysfs directly — no `libsensors` dependency:

1. Primary: walk `/sys/class/hwmon/*` and prefer entries named `coretemp` (Intel), `k10temp` or `zenpower` (AMD). Pick the input whose label is `Package id 0` or `Tctl`.
2. Fallback: `/sys/class/thermal/thermal_zone*`, preferring zones of type `x86_pkg_temp`.

### NVIDIA GPU

Use NVIDIA Management Library:

```text
NVML
```

NVML can provide:

- GPU utilization
- GPU temperature
- VRAM total
- VRAM used
- GPU process information

### AMD GPU

Possible future sources:

```text
rocm-smi
/sys/class/drm/
```

### Intel GPU

Possible future sources:

```text
intel_gpu_top
/sys/class/drm/
```

---

## Configuration

ZTOP reads an optional configuration file at startup:

```text
~/.config/ztop/config.toml    ($XDG_CONFIG_HOME honored)
```

Example:

```toml
refresh_interval = 1.0    # 0.25..=5.0 seconds
sort_by = "cpu"           # cpu | ram | gpu | vram | total
show_gpu = true           # false = same as --no-gpu
show_temperatures = true  # false = same as --no-temp
pid_gpu = true            # false = same as --no-pid-gpu
top_processes = 10        # 1..=200
process_filter = ""          # optional command/user/PID substring
gpu_peak_hold = 3.0       # 0..=30 seconds; 0 disables peak display
```

Every key is optional. Command-line flags always override the config file;
unknown keys and out-of-range values are rejected with a clear error so typos
cannot silently change behavior.

---

Use `--gpu`, `--pid-gpu`, and `--temp` to re-enable features disabled by your
config for one invocation. `--no-config` bypasses the default file; `--config PATH`
selects an explicit file and reports an error if it cannot be read.

## Future Features

Implemented since the MVP: multi-GPU support, JSON output / headless snapshot
mode (`--json`), the configuration file, quiet-by-default GPU diagnostics with
`--verbose` detail mode, and rendering/JSON/collector regression tests.

Possible features for later versions:

- AMD GPU support
- Intel GPU support
- Prometheus metrics export
- SSH-friendly mode
- Docker/container process detection
- Temperature warnings
- Configurable themes
- Minimal mode
- Detailed process mode
- Per-process GPU utilization without root (currently requires `CAP_SYS_ADMIN` on most distros — see [`SPEC.md` §2.1](./SPEC.md))

---

## Target Users

ZTOP is designed for:

- AI researchers
- CUDA users
- Linux workstation users
- GPU server administrators
- Cybersecurity researchers
- Digital forensic analysts
- Developers
- System administrators

---

## Design Philosophy

ZTOP should be:

### Simple

Only essential information should be shown.

### Clean

The interface should be readable at a glance.

### Fast

The monitor must use minimal CPU and RAM.

### GPU-aware

GPU, VRAM, and GPU temperature should be visible by default.

### User-aware

The process table must clearly show which Linux user is running each command.

---

## Non-Goals

ZTOP should not try to become a full replacement for:

- `btop`
- `htop`
- `top`
- `nvtop`
- `glances`

It should remain focused on a compact and practical resource view.

---

## License

Dual-licensed under either of:

- MIT License
- Apache License 2.0

at your option.

---

## Versioning

Releases use **monotonically increasing SemVer**: `MAJOR.MINOR.PATCH`.
UTC build metadata (`+YYYYMMDD.HHMM`) identifies the build and does **not**
change version precedence. Release versions no longer reset at a month or year
boundary. The historical calendar versions are retained in the changelog.

`Cargo.toml` is the version source. Prepare a release with a short change summary:

```bash
./scripts/version.sh 0.9.4 "Describe the release change"
./scripts/version.sh --check
```

The script rejects versions that do not increase, adds the UTC timestamp, and
synchronizes Cargo.toml, Cargo.lock, this README's Status line, the companion
version, and a new changelog entry. CI checks consistency and version ordering.
Expand the generated changelog entry with relevant behavior and validation before
publishing. Ordinary development edits do not require a new release number.

## Project Status

The current development version is listed in **Status** above and reported by
`ztop --version`. CPU/RAM/swap, NVIDIA multi-GPU telemetry, process filtering
and scrolling, config overrides, one-shot JSON, and JSON streaming are
implemented. The memory-management companion is separately enabled
and defaults to preview mode. See [`RELEASE.md`](./RELEASE.md) for remaining
hardware qualification before a public release.

Initial platform:

```text
Linux
```

Initial GPU support:

```text
NVIDIA (via NVML)
```

## © 2026 Andrea Bodei <info@andreabodei.com> — Dual-licensed MIT OR Apache-2.0.

<!-- VANTAGE-PYTHON-ENV-STANDARD:START -->
## VANTAGE Python environment standard

All Python tools in this VANTAGE workspace use one environment per Unix account: `$HOME/.venv`. Do not create or activate a project-local `venv`, `.venv`, `.venv-*`, or per-tool pipx/uv environment.

From the VANTAGE workspace root:

```bash
./python-env.sh ensure
./python-env.sh pip install -r path/to/requirements.txt
./python-env.sh python path/to/script.py
./python-env.sh audit
```

Direct commands, when a project script cannot call the workspace helper:

```bash
test -x "$HOME/.venv/bin/python" || python3 -m venv "$HOME/.venv"
"$HOME/.venv/bin/python" -m pip install -r requirements.txt
"$HOME/.venv/bin/python" script.py
```

Activation is unnecessary. Always invoke `$HOME/.venv/bin/python` directly and run pip only as `$HOME/.venv/bin/python -m pip`. Containers use the container account's `$HOME/.venv`; only explicitly documented OS-managed service runtimes are exceptions.
<!-- VANTAGE-PYTHON-ENV-STANDARD:END -->
