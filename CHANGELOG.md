# Changelog

All notable changes to ZTOP are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Release policy:** Starting with 0.9.0, release versions increase monotonically using SemVer. UTC build metadata identifies the build but does not affect precedence. Historical calendar-derived versions remain below. Use `scripts/version.sh` to prepare and validate release versions.

---

## [0.9.12+20261121.1113] — 2026-11-21

- Upgrade terminal and error-handling dependencies to address security advisories.

## [0.9.11+20261121.0757] — 2026-11-21

- Honor explicit CLI overrides before validating saved values.

## [0.9.10+20261120.0942] — 2026-11-20

- Verify CPU-only monitoring without probing unavailable NVML; support root orchestrator staging.

## [0.9.9+20261104.0925] — 2026-11-04

### Removed

- The `C` key and the hide-cores feature. The per-core rows can no longer be
  hidden, so an accidental keypress cannot take them away.

### Changed

- Per-core rows take flexible screen space first, ahead of the history strip,
  extra GPU panels, and the taller GPU panel. A 24-row terminal now shows
  every core of a 14-thread host next to the process table; before, it showed
  none. The rows are trimmed only when the terminal cannot hold them next to
  the eight process rows and one three-row GPU panel.

### Validation

- Rust 1.88: `cargo fmt --check`, clippy with `-D warnings`, and `cargo test`
  (71 passed, 1 ignored) green. The installed TUI was captured through a
  pseudo-terminal at 151x35 and 80x24; both show the title counts and all 14
  per-core cells.

## [0.9.8+20261104.0921] — 2026-11-04

### Changed

- The CPU panel title is now just ` CPU · 12 cores / 14 threads `. The
  `C hide/show per-core` key hint and the auto-fit notes no longer appear in
  the title; `C` stays documented in the `?` help overlay and auto-fit limits
  the per-core rows silently.

### Validation

- Rust 1.88: `cargo fmt --check`, clippy with `-D warnings`, and `cargo test`
  green; the TUI was captured through a pseudo-terminal at 151x35 and 80x24.

## [0.9.7+20261104.0908] — 2026-11-04

### Fixed

- The core and thread counts now sit in the CPU panel title, where the
  per-core toggle hint used to stand alone:
  ` CPU · 12 cores / 14 threads · C hide per-core `. 0.9.6 had put them on
  the model line below, which is where nobody looked; the model line is back
  to the brand string only. When the title does not fit, the key hint is
  dropped before the counts. The auto-fit title keeps the counts too
  (`showing 16/128 · auto-fit`, or `auto-fit · per-core hidden` when the
  terminal is too short for any per-core row).

### Validation

- Rust 1.88: `cargo fmt --check`, clippy with `-D warnings`, and `cargo test`
  green; release build OK. The TUI was captured through a pseudo-terminal at
  151x35 and 80x24 and shows the counts in the panel title.

## [0.9.6+20261104.0809] — 2026-11-04

### Fixed

- CPU panel: the physical core and thread counts are shown next to the CPU
  model again (`Intel(R) Core(TM) Ultra 5 125U · 12 cores / 14 threads`). The
  physical count is read once at startup via sysinfo and cached, so the
  per-tick refresh cost is unchanged. On narrow terminals the model name is
  truncated before the counts are dropped; platforms that hide the topology
  (some containers and virtual machines) show only the thread count. The
  compact short-terminal summary shows the same counts instead of the
  ambiguous `N cores`, which was actually the thread count.

### Added

- `cpu_physical_cores` in `--json` / `--json-stream` snapshots (`null` when
  the platform hides it). Additive and nullable, so `schema_version` stays 2.
- UI regression tests for the brand line, the compact summary, the unknown
  physical-count fallback, and count survival under brand truncation.
- Installer: every mutating run (`./installer.sh`, `--uninstall`, the
  ramcleaner commands) restores ownership of the entire checkout, `.git/`
  included, to the installing user right after taking the lock and again on
  exit, success or failure. `sudo chown -Rf` runs only when foreign-owned
  entries exist, so a clean tree never prompts. `ZTOP_OWNER=user:group`
  overrides the detected owner; `--help` and `--check` stay read-only.

### Documentation

- Mandatory ownership rule: the entire checkout, including `.git/`, must
  always be owned by `spabam:spabam`; any root process working in the tree
  must build outside it and restore ownership before it finishes
  (README §Install, SPEC §14, CLAUDE.md).

### Validation

- Rust 1.88: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test` (71 passed, 1 ignored) green; release build OK.
- `ztop --no-config --json --top 2` on the Intel Core Ultra 5 125U host reports
  `cpu_physical_cores: 12` alongside 14 per-core entries.
- 57 shell fixtures (six new for the ownership restore), ShellCheck, Bash
  syntax, five runtime tests, and `scripts/version.sh --check` passed.

## [0.9.5+20261102.1656] — 2026-11-02

### Fixed

- Installer: build in `$XDG_CACHE_HOME/ztop-target` (`~/.cache/ztop-target`)
  instead of the checkout's `target/` unless `CARGO_TARGET_DIR` is set. Inside
  a Dropbox-synced checkout the build died within seconds: Dropbox renamed the
  selectively excluded `target/` to `target (Selective Sync Conflict)` and every
  later cargo write failed with `No such file or directory`. The installer now
  logs the build directory and names it when a build fails.
- Installer: on a host without any NVIDIA runtime, the monitor's own
  `GPU init failed` startup line is reported as information during
  verification instead of as a second warning; it only restated the preflight
  result. Hosts with NVIDIA files still see it as a warning.
- Tests: the permission fixtures follow the new default build directory, a new
  fixture covers `CARGO_TARGET_DIR`, `XDG_CACHE_HOME`, `HOME`, and the
  `target/` fallback without a home directory, and a CPU-only fixture checks
  the verification diagnostics.

## [0.9.4+20261101.1627] — 2026-11-01

- Stable process ordering: break sort ties by PID and treat non-finite readings as zero so the comparator stays a total order (no panic risk on Rust >= 1.81). Mark the active sort column in the process-table header with ▼.

## [0.9.3+20261026.0923] — 2026-10-26

### Fixed

- Verify fresh GPU telemetry from the installed binary, including every device
  reported by `nvidia-smi`. Installation and `--check` return nonzero when a
  detected NVIDIA setup cannot produce valid samples, while CPU-only hosts
  remain supported.
- Preserve NVIDIA query errors and explain driver/library version mismatches.
  Stop installing unnecessary NVIDIA development packages: the runtime
  `libnvidia-ml.so.1` is sufficient.
- Find the existing Rust toolchain during `--check` and avoid requesting sudo
  for a writable user prefix when no system dependencies need installation.

### Added

- A private NVML fallback for hosts awaiting a driver repair or reboot. On a
  system driver/library mismatch only, try the administrator-provided library
  under `$PREFIX/lib/ztop/nvml/<loaded-driver-version>/libnvidia-ml.so.1`.
  Healthy system NVML takes precedence, both NVML sessions use the same selected
  library, and startup warns that the host mismatch remains unresolved.
- Regression tests for library selection, driver-version path validation,
  multi-GPU verification, missing/stale telemetry, driver errors, and writable
  installation prefixes.

### Validation

- Rust 1.88 formatting, Clippy, release build, and 67 tests in each of debug and
  release passed; one manual benchmark remained ignored. All 49 shell fixtures,
  five runtime tests, ShellCheck, Bash syntax, and release-version checks passed.
- Installed at `/usr/bin/ztop` on Debian 13/Proxmox. As the normal user, four
  fresh NVIDIA A2 samples matched the compatible `nvidia-smi` device identity
  and 15356 MiB memory capacity; the GPU terminal panel and clean quit passed.
  The private 615.71.09 NVML runtime worked with the loaded 615.71.09 driver
  despite system userspace packages being 615.71.10. No driver reload or reboot
  was performed.

## [0.9.2+20260905.2107] — 2026-09-05

- Display per-process VRAM as a percentage of total memory across detected GPUs, with a VRAM% header and one decimal place, matching the other process resource columns.

## [0.9.1+20260905.2102] — 2026-09-05

- Correct the --no-pid-gpu guarantee: per-process accounting is skipped, but the NVIDIA driver may open /dev/nvidia-uvm during initialization or device queries. Use --no-gpu to skip NVML entirely. This supersedes the UVM claims in the historical 0.3.16 notes.

## [0.9.0+20260905.2041] — 2026-09-05 — Accurate process accounting and dependable monitoring

### Fixed

- Explicitly disable sysinfo task enumeration at startup and refresh, preventing
  threads from crowding out distinct processes in the table. Refresh only CPU
  usage rather than unused frequency counters.
- Key GPU history and failure grace by stable device identity. Preserve each
  device through partial failures and recovery, distinguish missing readings
  from zero, and retry transient/no-sample per-process utilization queries.
- Parse installer inspection commands before any mutation; remove unrelated
  authentication bootstrap and external recursive cleanup. Serialize mutations,
  use locked Cargo builds, and detect PSI by reading its contents.
- Address short-terminal overflow, long PIDs/CPU percentages, Unicode clipping,
  Ctrl-C handling, and binary memory unit labels.

### Added

- Process search/filtering, viewport scrolling, collapsible core details, GPU
  paging, and bounded histories for CPU/RAM/GPU/VRAM.
- `--filter`, `--config`, `--no-config`, and positive GPU/temperature overrides.
- `--json-stream` newline-delimited output with optional `--count`, filtering,
  independent GPU sampling, and clean handling of closed output pipes.
- Regression coverage for thread exclusion, device loss/recovery, compact
  layouts, input handling, streaming, configuration precedence, shell actions,
  and release version consistency; collection benchmark hook.

### Changed

- JSON schema **2** adds GPU identity, status, and observation timestamps;
  unavailable GPU utilization and temperature are null. Stale/unavailable GPU
  memory fields retain their last observed values and require checking status.
- Normal installation installs only ZTOP. Ramcleaner is separately installed in
  preview mode; automatic swap recycling is disabled by default and requires
  fresh conservative headroom checks. Incomplete service parking is unavailable.
  Action outcomes and swap restoration failures are visible in status/exit codes.
  `--recover` restores pending swap while suspended without starting new actions;
  invalid recovery metadata cannot arm the exit handler.
- Release versions increase monotonically; `scripts/version.sh` synchronizes
  component versions and checks consistency without relying on build metadata
  for ordering.

### Validation

- Rust 1.88: formatting, Clippy, release build, and 62 tests in each of debug and
  release passed. 28 shell fixtures, five runtime tests, and release-version
  checks passed; real GPU snapshots worked unprivileged and as root.
- Local paired TUI measurements reduced median CPU use from 1.833% to 0.667% of
  one core with GPU/temperature collection disabled. Full conditions and hardware
  limitations are recorded in `RELEASE.md`.

## [0.8.18+20260818.2240] — 2026-08-18 — ramcleaner: non-disruptive RAM/swap optimiser

### Added

- New `ramcleaner/` component: `ztop-ramcleaner`, a tiered RAM/swap optimiser
  that reclaims memory **without terminating any process**. It only moves pages
  — cgroup-v2 proactive reclaim (`memory.reclaim`), free-page compaction, and
  guarded swap recycling — so applications, browser tabs, and desktop sessions
  all survive every pass. No `kill`, no OOM handling, no process discarding.
- Four tiers, each with its own trigger and cooldown: T1 proactive reclaim of
  cold pages per cgroup slice, T2 `vm.compact_memory`, T3 `swapoff`/`swapon` to
  empty stale swap, T4 parking long-idle *system services* (opt-in, default off,
  never user applications).
- `installer.sh` gained `--install-ramcleaner`, `--uninstall-ramcleaner`, and
  `--install-ramcleaner-tuning`. A full `install` now also installs ramcleaner
  (skip with `NO_RAMCLEANER=1`); `--uninstall` removes it again.
- ramcleaner runs from `/etc/cron.d/ztop-ramcleaner` every 10 minutes and is
  installed **active**, so a plain `./installer.sh` yields automatic RAM/swap
  optimisation with no follow-up step. Every action is non-destructive by
  construction, so acting is the default; `RAMCLEANER_DRY_RUN=1 ./installer.sh`
  installs it inert for preview, and the installer runs one verification pass
  immediately rather than leaving the first proof to a cron run 10 minutes away.
- The installer now pre-authenticates sudo before the long bootstrap/build
  stages, explaining why root is needed instead of surfacing an unannounced
  password prompt mid-build. With no TTY it fails with actionable guidance
  (`sudo ./installer.sh`, `sudo -v` first, or `SUDO_ASKPASS=`) rather than
  sudo's bare "a terminal is required".
- `installer.sh --check` now reports ramcleaner presence, its armed/dry-run
  mode, whether kernel PSI is available, and whether zram swap is active.
- Reference configs for the two structural prerequisites that matter more than
  the cron job itself: `zram-generator.conf.example` and optional `vm.*` sysctl
  tuning in `99-ztop-ramcleaner.sysctl.conf`.

### Fixed

- `installer.sh --help` printed the VANTAGE bootstrap prologue instead of the
  usage banner: the hardcoded `sed -n '2,21p'` range no longer matched after the
  prologue was prepended. The range is now located by marker at runtime, so it
  cannot drift again, and it resolves via `$SELF`/`BASH_SOURCE` so it is also
  correct when the installer is sourced rather than executed.
- `installer.sh` ignored `CARGO_TARGET_DIR` and looked for the built binary
  under `$HERE/target` unconditionally, so a build redirected elsewhere failed
  with "build did not produce ...". It now looks where cargo actually wrote it.

### Fixed (concurrent-run safety)

- **A non-building subcommand could destroy a concurrent build.** The VANTAGE
  prologue arms an EXIT trap that `rm -rf`s build-artifact directories
  (`target/`, `cache/`, `build/`, ...) under the repo, and it was armed
  unconditionally before argument parsing. So `--check` or `--install-ramcleaner`
  -- which build nothing -- would, on exit, delete the `target/` of a
  *concurrently running* `./installer.sh` build, killing it with
  "failed to create file target/release/.fingerprint/.../output-bin-ztop"
  and rustc exiting 101 on a truncated JSON stream. Every non-building
  subcommand now disarms the trap first; only a run that actually built
  something may clean up after itself.
- `install` now takes an flock (`$TMPDIR/ztop-installer.lock`) so two concurrent
  installs serialise instead of corrupting a shared `target/`. It degrades to
  proceeding unlocked if flock or the lockfile is unavailable, so it cannot
  wedge the installer.
- The shared `VANTAGE/.installer_cleanup.sh` is deliberately left untouched:
  other VANTAGE installers depend on its semantics.

### Fixed (ramcleaner T3 multi-device accounting)

- T3 gated on *total* swap utilisation while only ever `swapoff`-ing a single
  device, so adding a second swap device silently disabled the tier: with a
  980 MiB partition plus a 3 GiB file, the 80% trigger needed 3.2 GiB used and
  would then have demanded 5.3 GiB of RAM headroom — unreachable on an 8 GiB
  host. Trigger and headroom gate are now both per-device.
- The headroom gate also credits free space on the *other* swap devices, since
  pages leaving the target device land there before spilling into RAM. Only the
  unabsorbed remainder is charged against `MemAvailable`.

### Security

- T3's headroom gate is enforced as an invariant rather than a trigger.
  `swapoff` must fit every swapped page back into RAM; below
  `MemAvailable > swap_used × 1.5 + 512 MB` (or at high load) it is refused, and
  `tier3_is_safe` is re-checked inside the action so `--force-tier 3` cannot
  induce an OOM kill. `swapoff` additionally runs under `timeout`, and failure
  is a no-op that leaves swap untouched.
- `--force-tier N` overrides the cooldown for tier N only, no longer for every
  other tier as well.

---

## [0.7.6+20260706.1612] — 2026-07-06 — Contract, JSON, diagnostics, and CI hardening

### Added

- JSON snapshots now carry `schema_version: 1` and `sampled_at_unix_ms`, with
  the unit contract documented in README, SPEC, and CLI help.
- Added a narrow `GpuBackend` abstraction so collector behavior can be tested
  with fake GPU backends instead of requiring live NVIDIA hardware.
- Added `--verbose` for detailed GPU startup diagnostics. Default startup is
  now quiet except for actionable failures; `ZTOP_DEBUG_GPU` remains the
  per-poll diagnostics file path.
- Added `RELEASE.md` and replaced the stale v0.1 SPEC with the current
  engineering contract.

### Tests

- Added fake-GPU collector tests for multi-GPU samples and transient NVML
  failure grace behavior.
- Added JSON headless-path coverage for `--top`, `--no-gpu`, `--no-temp`, and
  schema metadata.
- Added ratatui `TestBackend` rendering checks for the minimum width warning,
  table column breakpoints, help overlay, multi-GPU titles, and high-core-count
  CPU wrapping.
- CI now runs release-mode tests, shellchecks the installer, runs installer
  `--check`, builds the release binary, and asserts the JSON smoke schema with
  `jq`.

## [0.7.6] — 2026-07-06 — Calendar-derived versioning

### Changed

- **Version numbers are now calendar-derived: `MAJOR.MONTH.DAY`.** The minor
  number is the month and the patch number is the day of the modification
  (UTC): a change on June 2 produces `0.6.2`, on July 1 `0.7.1`; today's
  change makes this release `0.7.6`. The `+YYYYMMDD.HHMM` build-metadata
  timestamp introduced with 0.4.0 is retained — it distinguishes multiple
  same-day modifications and keeps builds orderable across year boundaries
  (after `0.12.31` the next January modification is `0.1.1`). `MAJOR` remains
  the only manually chosen number. Documented in README §Versioning,
  `Cargo.toml`, and `CLAUDE.md`; the jump from 0.4.0 straight to 0.7.6 is
  this scheme taking effect, not three skipped feature releases.

## [0.4.0] — 2026-07-06 — Multi-GPU, config file, JSON output, event-loop fixes

### Added

- **Timestamped versions.** The `Cargo.toml` version now carries semver build
  metadata in the form `+YYYYMMDD.HHMM` (UTC), refreshed on every modification
  so `ztop --version` pins the exact source-tree state a binary came from.
  The convention is documented in README §Versioning and applies to all
  future changes.
- **Version in the dashboard.** The header shows the running version next to
  the `ZTOP` title (full timestamped form on terminals ≥ 100 columns, bare
  `vMAJOR.MINOR.PATCH` on narrower ones so host/uptime stay visible).

- **Multi-GPU support.** Every NVIDIA device now gets its own panel (labeled
  `GPU0:`, `GPU1:`, … when more than one is present). Per-process VRAM is
  summed across devices; per-process GPU% reports the busiest device so the
  column stays on a 0–100 scale. Devices that fail to poll individually are
  skipped for that sample instead of blanking the others.
- **Configuration file.** `~/.config/ztop/config.toml` ($XDG_CONFIG_HOME
  honored) with `refresh_interval`, `sort_by`, `top_processes`, `show_gpu`,
  `show_temperatures`, `pid_gpu`, and `gpu_peak_hold`. Explicit command-line
  flags always win; unknown keys and out-of-range values are rejected with a
  clear error.
- **`--json` headless mode.** Prints one pretty-printed metrics snapshot
  (host, CPU, memory, per-GPU, top-N processes) to stdout and exits without
  entering the TUI — usable from scripts and cron.
- **Live row count.** `[` / `]` adjust the number of process rows shown
  (1..200) while running; the footer shows the current count.
- **`?` help overlay.** A centered popup lists all key bindings; Esc closes
  the overlay before it quits the app.
- Project is now tracked in git, and CI lints test targets too
  (`clippy --all-targets`).

### Fixed

- **Terminal resize now redraws immediately.** Resize events were ignored by
  the event loop, leaving a stale layout until the next tick — or forever
  while paused.
- **Transient NVML failures no longer blank the GPU panel.** The last good
  sample is kept for up to 8 consecutive failed polls (~2 s) before the panel
  degrades to `n/a`, preserving peak-hold continuity across driver hiccups.
- **Mouse capture removed.** ztop never used mouse input; capturing it broke
  terminal text selection/copy and woke the event loop on every scroll.
- A non-press key event (release/repeat) no longer skips a due refresh tick.
- `ZTOP_DEBUG_GPU` diagnostics no longer write to stderr mid-TUI (which
  corrupted the alternate screen); they append to the file named by the
  variable's value (empty or `1` = `ztop-gpu-debug.log` in the system temp
  directory).
- The raw NVML session opened for the v2 memory API is now explicitly shut
  down on exit.

### Changed

- **Steady-state CPU cost cut ~12x** (measured 25.8% → 2.1% of one core with
  GPU sampling, 17.8% → 1.5% without, on a 32-core host with ~670 processes
  at the default 1 s refresh). Two causes fixed: sysinfo was walking every
  `/proc/<pid>/task/*` thread as a full process entry each tick (~2,500 extra
  file reads/s on that host), and it was reading disk-I/O counters plus the
  `exe` symlink for every PID every tick. ztop now requests only cpu, memory,
  user (once per process), and cmdline (once per process). This also fixes a
  latent bug where processes spawned after startup showed a bare name and `?`
  user because the incremental refresh never fetched cmd/user at all.
- sysinfo upgraded 0.32 → 0.38 (needed for the per-thread walk opt-out;
  still builds on the pinned Rust 1.88).
- **Pause now means quiet.** While paused, the 250 ms GPU sampling stops too,
  so a paused ztop costs (almost) nothing.
- Per-process command lines are cached per process lifetime
  (keyed by PID + start time) instead of being rebuilt from argv every tick —
  the main steady-state allocation cost on busy hosts.
- The process-table COMMAND column now uses the full remaining terminal width
  instead of truncating at a fixed 64 characters.
- The footer was slimmed to state (`sort`, `paused`, `rows`) plus `q quit` and
  `? help`; the full key list lives in the `?` overlay.
- Redundant double system scan removed from collector startup.

### Tests

- New unit tests for the temperature-source selection (hwmon preference,
  label matching, thermal-zone fallback — via fixture sysfs trees), process
  sorting including the normalized `total` score, per-PID GPU merging across
  devices, config parsing/precedence, and the `[`/`]` clamping. 29 tests total
  (up from 11).

- Installer now aborts with a clear message — before any build or sudo
  prompt — when `target/` or `Cargo.lock` contains artifacts owned by another
  user (e.g. after a build ran as root), naming the exact `chown` fix. On
  that failure path the VANTAGE cleanup EXIT trap is disarmed too: it cannot
  delete a foreign-owned `target/` either, and its `sudo rm -rf` fallback
  used to surface a confusing password prompt right after the error message. The
  build step also gained an explicit `|| die`, so a failed build can never
  fall through and install a stale `target/release/ztop`. Previously this
  scenario surfaced as a cryptic cargo `Permission denied` followed by a
  surprise sudo password prompt (the VANTAGE cleanup trap attempting
  `sudo rm -rf` on the root-owned `target/`).

### Fixed (installer & docs, folded in from the pre-0.4.0 unreleased queue)

- Installer now checks the active `rustc` version before building. If a distro
  toolchain is present but too old for the locked dependency graph, it installs
  and uses the pinned Rust 1.88.0 toolchain via rustup instead of failing during
  `cargo build`.
- Installer now prints elapsed progress lines during long Rust toolchain
  installs and release builds, so quiet downloads do not look hung.
- Fixed the rustup bootstrap path after adding visible download progress: when
  the installer saves `sh.rustup.rs` to a temp file, it now invokes that script
  directly instead of passing the stdin-only `-s --` shell arguments.
- Installer Rust bootstrap now installs only the minimal build toolchain first;
  clippy/rustfmt are left to CI/development setup so optional component download
  failures cannot block installing the binary.
- Project metadata, CI, and documentation now consistently declare Rust 1.88 as
  the minimum supported toolchain.
- Fixed stale source commentary about per-process GPU opt-in; the implemented
  switch is `--no-pid-gpu`.

## [0.3.17] — 2026-05-25 — README documents `--no-pid-gpu`

### Documented

- README's Command-Line Usage section now lists `--no-pid-gpu` alongside `--no-gpu`, with a short note explaining the `lsof /dev/nvidia-uvm` use case. Without this entry the only place the flag was discoverable was `ztop --help`.
- Project Status block bumped to 0.3.17 and now mentions the `--no-pid-gpu` capability in the running feature summary.

## [0.3.16] — 2026-05-24 — Optional UVM-free GPU mode

### Added

- New `--no-pid-gpu` flag disables only the per-process VRAM/GPU% accounting. With it set, ztop never calls the NVML APIs (`running_compute_processes`, `running_graphics_processes`, `process_utilization_stats`) that cause NVML to open `/dev/nvidia-uvm` for inventory queries. Device-level GPU metrics (utilization, temperature, VRAM totals) keep working. Useful when auditing GPU clients with `lsof /dev/nvidia*` — ztop will no longer appear among `/dev/nvidia-uvm` holders. Default behaviour is unchanged: all per-process columns are populated and ztop transparently uses NVML's process tables.

### Documented

- Clarified in the help text and README that ztop opening `/dev/nvidia-uvm` is a read-only NVML inventory side effect, not a CUDA context, and does not allocate or hold GPU memory. This is to prevent the common misdiagnosis where stale VRAM from a dead inference runner is blamed on monitoring tools that share NVML.

## [0.3.15] — 2026-05-20 — Installer NVML symlink helper

### Changed

- `installer.sh` now checks for the unversioned `libnvidia-ml.so` symlink after detecting an NVIDIA driver. When it is missing on apt-based systems and `libnvidia-ml-dev` is a candidate in the configured repos (Ubuntu, NVIDIA CUDA repo on Debian), the installer offers to install it; otherwise it logs that ztop will use its built-in `libnvidia-ml.so.1` fallback. This stops the `libnvidia-ml.so: cannot open shared object file` failure on stock Debian Trixie hosts where only the versioned soname ships in `libnvidia-ml1`.

## [0.3.14] — 2026-05-20 — GPU peak hold

### Added

- GPU sampling now runs every 250 ms independently from the main UI refresh, so short NVML utilization events can be captured without refreshing the full process table four times per second.
- The GPU panel now shows a rolling utilization peak, defaulting to a 3-second hold window, so brief model-load spikes remain visible on the next UI refresh.
- Added `--gpu-peak-hold SECONDS` with range `0..=30`; `0` disables peak tracking and hides the peak values.
- The GPU title line also reports rolling peak VRAM while peak tracking is enabled.

## [0.3.13] — 2026-05-20 — Live NVML v2 refresh

### Fixed

- The raw NVML memory-info v2 handle now initializes itself with `nvmlInit_v2()`, so active VRAM readings update correctly when an Ollama model is loaded after `ztop` is already running.

## [0.3.12] — 2026-05-20 — Aligned metric bars

### Fixed

- CPU and GPU utilization bars now start at the same column as the RAM, SWAP, and VRAM bars, matching the tighter visual alignment used by tools like `btop`.
- Metric lines now use one shared bar-start column helper instead of separate per-line subtraction constants.

## [0.3.11] — 2026-05-20 — NVIDIA active VRAM accounting

### Fixed

- VRAM used now prefers NVML memory-info v2, matching `nvidia-smi memory.used` by excluding driver/firmware reserved memory from the active-used total.
- Startup GPU diagnostics now print reserved memory alongside used and total memory when NVML memory-info v2 is available.

## [0.3.10] — 2026-05-20 — Copyrighted version output

### Changed

- `ztop --version` now prints the package version followed by `Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>`.
- Expanded the version-flag regression test to assert that the copyright line is present.

## [0.3.9] — 2026-05-20 — Explicit version flag

### Added

- `ztop --version` is now explicitly wired to `CARGO_PKG_VERSION` in the CLI definition and covered by a regression test.

## [0.3.8] — 2026-05-20 — Wide CPU-core packing

### Fixed

- Per-core CPU rows now use the available terminal width instead of always stopping at 8 cores per row. This reduces the CPU panel height on wide terminals, so high-core-count servers such as 128-logical-core hosts can show all cores instead of clipping the lower rows.
- Core IDs now reserve enough width for three-digit indices, keeping `100` through `127` aligned in the CPU panel.

## [0.3.7] — 2026-05-20 — Runtime hygiene and ownership-safe install

### Added

- Validated `--top` range (`1..=200`) and added focused unit tests for CLI bounds and refresh-rate clamping.
- `LICENSE-MIT` and `LICENSE-APACHE` files for the dual-license declaration.
- GitHub Actions CI workflow for `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test --locked`.

### Changed

- The TUI event loop now redraws only after input or refresh ticks, reducing idle rendering work.
- Sort hotkeys immediately re-sort the current process snapshot, including while paused.
- Process rows now prefer the full command line over the executable name when `/proc` exposes it.
- User lookup and CPU temperature source discovery are cached instead of rebuilt on every tick.
- Terminal cleanup now uses a drop guard, and release builds use unwinding panics so the guard can restore the terminal if a panic occurs.
- `installer.sh` now refuses to build as root; install as a normal user, then run `sudo ztop` on hardware that needs root for per-process GPU metrics.
- `Cargo.lock` is no longer ignored; this is a binary application and should keep reproducible dependency resolution.

### Removed

- Removed the stale `README.md~` backup file from the project.

## [0.3.6] — 2026-05-20 — Installer NVML symlink bootstrap

The 0.3.4 binary already falls back from `libnvidia-ml.so` (unversioned, dev-package symlink) to `libnvidia-ml.so.1` (runtime soname) at NVML load time. This release adds the *complementary* installer-side fix: when NVIDIA is detected and the unversioned symlink is missing, the installer tries to install it via the distro's package manager so the canonical default-name open path works without needing the binary's fallback.

### Added

- `installer.sh::check_nvidia` is now two-phase:
  1. Detect NVIDIA presence via `nvidia-smi` or any of the standard `libnvidia-ml.so.1` locations.
  2. If NVIDIA is present, check whether the unversioned `libnvidia-ml.so` symlink exists in `/usr/lib/x86_64-linux-gnu`, `/usr/lib64`, `/usr/lib`, `/lib`, or `/lib64`. If it does not, attempt to install the package that provides it.
- Per-package-manager handling for the unversioned symlink:
  - **apt** — runs `apt-cache policy libnvidia-ml-dev` first and only attempts the install when there's a valid candidate. This avoids `E: no installation candidate` on stock Debian (where the symlink ships only via the NVIDIA CUDA repo as `cuda-nvml-dev-XX-X`, not in main).
  - **dnf / yum** — warns that the symlink is missing and points at `cuda-nvml-devel-*` from the CUDA repo as the canonical source.
  - **other / unknown** — warns and relies on the binary's `.so.1` fallback.
- Refactored NVIDIA detection so the symlink check only runs when a driver is actually present (no point installing `-dev` headers on a host that doesn't have the driver itself).

### Notes

- The binary's `.so.1` fallback from 0.3.4 remains the safety net: if `libnvidia-ml-dev` can't be installed (no candidate, package manager unsupported, install fails), `ztop` still initializes NVML by opening `libnvidia-ml.so.1` directly. The installer prints `ztop will use the .so.1 fallback` in those paths so the user knows what happened.
- No source-code change in this release — only `installer.sh` and the version field in `Cargo.toml`.

## [0.3.5] — 2026-05-20 — GPU utilization diagnostics and fallback

User report: "GPU usage is always 0% even when using ollama and llms" on multiple hosts. Adding visibility into what NVML is actually returning at startup, plus a per-process utilization fallback for the case where `nvmlDeviceGetUtilizationRates` returns NOT_SUPPORTED or 0 while ollama is producing real work.

### Added

- **Init-time NVML diagnostic dump to stderr** (one-shot, before the TUI's alt-screen takes over so the lines stay in the user's shell history). Prints device name and count, the result of `utilization_rates()` at init (success with values, or the actual error message), the result of `memory_info()`, and whether per-process utilization is permitted. This lets a "GPU 0%" report be triaged: did the API error, return 0, or return a real value?
- **Process-util-sum fallback** in `GpuProbe::poll()`. When `utilization_rates()` returns 0 — either because it failed (suppressed) or because the driver reports NOT_SUPPORTED for the device class — the device-wide util_pct now reports `max(device_util, sum(per_process_sm_util))`. Only effective when per-process utilization is available (root or `CAP_SYS_ADMIN`); harmless to non-root sessions because the per_pid util_pct values are all `None` and the sum is 0.

### Notes for triage

If the init line reads `utilization_rates at init: gpu=0% memory_io=0%`, NVML is responding but the driver is reporting 0 utilization for the device — likely a sampling-window or power-state issue, or the GPU is genuinely idle at startup. Run `nvidia-smi dmon -s u` alongside ztop to compare.

If the init line reads `utilization_rates FAILED at init: <error>`, the API is not supported on the device or driver. The fallback will populate util from per-process stats when running as root.

## [0.3.4] — 2026-05-20 — NVML and installer robustness

### Fixed

- **NVML init failed on servers without the `nvidia-utils-*-dev` package.** `nvml-wrapper` defaults to opening `libnvidia-ml.so` (the unversioned symlink), which only ships with the `-dev` package. On runtime-only installs (Debian/Ubuntu/Fedora server images, the user's `hypervisor00` with NVIDIA A2), only the versioned soname `libnvidia-ml.so.1` is present, and ztop reported `libnvidia-ml.so: cannot open shared object file` even with a working driver. `metrics::gpu::GpuProbe::init()` now tries the default open and falls back to `libnvidia-ml.so.1` via `Nvml::builder().lib_path(...)`.
- **Installer reinstalled rustup on every run** in shells that hadn't sourced `~/.cargo/env`. `command -v cargo` only checks the current shell's `PATH`; rustup writes the PATH update into `.profile` / `.bashrc` which non-login non-interactive shells don't always source. `installer.sh::ensure_rust` now sources `~/.cargo/env` before the `command -v` check, so a previously-installed toolchain is detected without redoing the curl-pipe-to-sh.

## [0.3.3] — 2026-05-20 — Rich --help

### Added

- `ztop --help` now prints a full feature tour, color palette legend, keyboard bindings table, usage examples, and the copyright/license line. The short `-h` form keeps the concise one-line summary; `--help` triggers the long form via clap's `long_about` + `after_long_help`.
- Per-flag doc comments in `src/cli.rs` so each option has an inline description in the `Options:` block of `--help` (e.g. `--refresh` now shows "Refresh interval in seconds (0.25..=5.0)" instead of being undocumented).
- Copyright line in `--help` output: `Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>` followed by the dual MIT/Apache-2.0 declaration.

### Changed

- `src/cli.rs` reorganized with `LONG_ABOUT` and `AFTER_LONG_HELP` `const &str` blocks at the top of the file, so the help text is co-located with the CLI definition and trivially editable.

## [0.3.2] — 2026-05-20 — Installer bootstrap

### Added

- `installer.sh` now **bootstraps its own dependencies**:
  - If `cargo` is missing, installs the Rust stable toolchain via the official rustup one-liner (`--profile minimal`, non-interactive). If `curl` is missing it is installed first via the detected package manager.
  - If `cc` or `make` is missing, installs build essentials (`build-essential` + `pkg-config` on Debian/Ubuntu; `gcc make pkg-config` on Fedora/RHEL; `base-devel` on Arch; `devel_basis` pattern on openSUSE; `build-base` on Alpine).
  - Detects NVIDIA driver presence (`nvidia-smi` or `libnvidia-ml.so.1`) and warns if missing — the driver itself is **not** auto-installed because it requires kernel modules and is entangled with secure boot.
- New `--check` subcommand: prints the environment audit (rust, build tools, NVIDIA, installed binary) without making any changes.
- `pkg_install` helper that abstracts over apt / dnf / yum / pacman / zypper / apk.

### Changed

- Installer help text expanded to document the new subcommands and bootstrap semantics.

## [0.3.1] — 2026-05-20

### Changed

- **RAM and SWAP merged into a single bordered panel** (height 4: 2 content lines + 2 borders). Previously each had its own frame stacked vertically; sharing one frame reduces visual noise without changing the information content.
- `draw_ram()` and `draw_swap()` collapsed into one `draw_memory()` that emits both lines into the same `Paragraph`.
- Vertical layout shrank from seven chunks back to six (header, cpu, memory, gpu, processes, footer).

## [0.3.0] — 2026-05-20 — Color palette

### Added

- **Cyan** for section labels (`CPU`, `RAM`, `SWAP`, `GPU`, `VRAM`, `temp`, `Cores`), header field names (`refresh:`, `host:`, `uptime:`), footer keybinds, and process-table column headers.
- **Grey** for brand strings (CPU model line, GPU model line) — chosen for a quieter look that lets the cyan labels and yellow temperatures carry the visual emphasis. (Bold magenta was tried first and explicitly downgraded.)
- **Yellow** for temperature readings.
- **Load-graded bar fills**: green when usage < 50 %, yellow at 50–80 %, red ≥ 80 %.
- **Dark gray** for empty bar cells, clearly separating filled from unfilled portions.
- `ui::bars::bar_spans()` replaces the old plain-text `bar()`; returns a `Vec<Span<'static>>` so each bar can be styled per character class (fill vs empty vs brackets) within a single line.
- Compile-time `const Style` palette entries (`LABEL`, `BRAND`, `TEMP`, `TITLE`) at the top of `src/ui/mod.rs` so the colors are declared in one place and trivially tweakable.

### Changed

- All panel labels now produced as individual styled spans instead of being baked into one raw format string. Visible character counts preserved exactly so the resize breakpoints in SPEC §3 are unaffected.
- Process table header row now bold cyan instead of bold default.

## [0.2.1] — 2026-05-20 — Licensing & distribution

### Added

- SPDX-style copyright headers on every source, config, and doc file: `Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>` + `SPDX-License-Identifier: MIT OR Apache-2.0`.
- `authors = ["Andrea Bodei <info@andreabodei.com>"]` in `Cargo.toml`.
- `installer.sh` for `/usr/bin/ztop` install (with `PREFIX` override and `--uninstall` subcommand). Sudo is used only for the copy step so `target/` never ends up root-owned.

### Changed

- License declaration: "to be decided" → dual MIT OR Apache-2.0.
- README rewritten to reflect the implemented state, with `Install`, `Build and Run (manual)`, and `Keyboard Bindings` sections.

## [0.2.0] — 2026-05-20 — Swap monitor and brand lines

### Added

- **Swap memory panel** between RAM and GPU. Used / total GB and percentage with a horizontal bar. When no swap device is configured, the panel displays `SWAP  none configured` instead of a divide-by-zero placeholder.
- **CPU brand line** at the top of the CPU panel, mirroring the GPU panel's device-name treatment. Sourced from `sysinfo::Cpu::brand()` (e.g. `13th Gen Intel(R) Core(TM) i9-13980HX`).
- `metrics::memory::swap_used_total()` helper.
- `metrics::cpu::brand()` helper with empty-string and missing-CPU fallback to `"unknown CPU"`.
- `ui::mod::truncate()` shared helper for narrow-terminal brand-line trimming.

### Changed

- **GPU panel device name moved from the third line to the first.** New order: GPU model → GPU util + temp + bar → VRAM used/total + bar. The relocation matches the new CPU panel layout.
- CPU panel height grew by one line to accommodate the brand line (`ceil(n_cores / 8) + 4`).
- `Snapshot` gained `cpu_brand`, `swap_used_bytes`, `swap_total_bytes` fields.
- Vertical layout grew from six chunks to seven (header, cpu, ram, swap, gpu, processes, footer).

## [0.1.0] — 2026-05-20 — Initial scaffold

The inaugural development cycle: project goes from a README-only concept to a working, end-to-end Linux/NVIDIA monitor that runs against real hardware.

### Added

#### Engineering contract

- `SPEC.md` resolving the five open design questions raised in the concept review:
  - per-process GPU/VRAM graceful degradation when `nvmlDeviceGetProcessUtilization` is not permitted,
  - normalized `total` score so CPU%/RAM%/GPU%/VRAM% are comparable on 0..100,
  - hwmon-based CPU temperature with thermal_zone fallback,
  - explicit keyboard bindings,
  - `--json` deferred (also removed from the duplicated CLI listing).
- 80-column layout validation with per-column byte budgets and shrinkage breakpoints (76 / 70 / 60 / 50 / 40 cols).
- Pre-release verification checklist (SPEC §7).

#### Rust scaffold

- `Cargo.toml` with `ratatui` 0.29, `crossterm` 0.28, `sysinfo` 0.32, `nvml-wrapper` 0.10 (Linux-only), `clap` 4, `anyhow` 1; release profile with thin LTO and strip.
- Module layout under `src/`:
  - `main.rs` — terminal setup/teardown, event loop, key dispatch.
  - `cli.rs` — `clap` definition with validated `--refresh` range.
  - `app.rs` — `AppState`, `SortMode`, refresh rate clamping.
  - `metrics/mod.rs` — `Collector`, `Snapshot`.
  - `metrics/cpu.rs`, `metrics/memory.rs` — sysinfo wrappers.
  - `metrics/temp.rs` — `/sys/class/hwmon` scanner with thermal_zone fallback.
  - `metrics/gpu.rs` — NVML probe; per-PID VRAM merge from compute + graphics processes; per-PID GPU% when permitted.
  - `metrics/process.rs` — sysinfo + NVML join, sort by selected mode, truncate to top-N.
  - `ui/mod.rs` — vertical layout, panel renderers, width-based column drops.
  - `ui/bars.rs` — horizontal bar widget.
  - `ui/table.rs` — process table with adaptive columns.

#### Runtime features

- CLI flags: `--refresh` (0.25..5.0), `--sort {cpu|ram|gpu|vram|total}`, `--top N`, `--no-gpu`, `--no-temp`.
- Keyboard bindings: `q`/Esc quit, `c`/`r`/`g`/`v`/`t` sort, Space pause, `+`/`-` adjust refresh by 0.25 s.
- Live footer showing active sort and pause state.
- Width-responsive process table that drops `VRAM` < 76 cols, `GPU%` < 70, `USER` < 60, and refuses to render below 40.
- One-line stderr notice when per-process GPU utilization is unavailable (driver/permission), with `—` in that column.

### Changed (vs original README concept)

- Default sort: `total` → `cpu` (matches `top` and `htop`, least surprise).
- `total` score formula: `cpu% + ram% + gpu% + vram%` → `max(cpu% / n_logical_cpus, ram%, gpu%, vram%)` so per-process CPU% becomes comparable with the other 0..100 components.
- CPU temperature primary source: `/sys/class/thermal` → `/sys/class/hwmon` scan preferring `coretemp`/`k10temp`/`zenpower`.

### Fixed

- Per-core panel was hard-coded to 4 lines; on a 32-core machine only the first 4 cores rendered. Panel height is now `ceil(n_cores / 8) + 3` and cores pack 8 per row.
- Bar bracket clipping on RAM and VRAM lines — the width math omitted the 2-character block borders.

### Removed

- `--json` CLI flag (deferred to a later release alongside Prometheus export, when an output schema is defined).

---

## Still outstanding before the first public release

- Run as root and confirm per-process GPU% column populates.
- Run on a host without an NVIDIA GPU and confirm the GPU panel shows `n/a` without panicking.
- Measure idle CPU budget rigorously (`perf stat`) and confirm < 1 % on an 8-core box at default refresh.
- Walk every resize breakpoint visually with a live terminal.

---

© 2026 Andrea Bodei <info@andreabodei.com> — Dual-licensed MIT OR Apache-2.0.
