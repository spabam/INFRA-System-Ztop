# ZTOP Release Checklist

ZTOP is still unreleased. Before tagging a public build, complete this checklist
and copy the measured/manual results into the release notes.

## Automated Checks

Run with the pinned Rust toolchain:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --release
shellcheck installer.sh ramcleaner/ztop-ramcleaner scripts/version.sh tests/version.sh
./scripts/version.sh --check
./tests/version.sh
"$HOME/.venv/bin/python" -m unittest discover -s tests/shell -v
./installer.sh --check
cargo build --locked --release
./target/release/ztop --no-config --json --top 1 --no-gpu --no-temp
ZTOP_TEST_BIN=./target/release/ztop "$HOME/.venv/bin/python" tests/runtime.py
```

The JSON smoke output must contain `schema_version: 2`, no GPUs, and at most
one process row.

## Manual Hardware Checks

- Run unprivileged on a host without NVIDIA; the GPU panel must show `n/a`.
- Run unprivileged on a host with NVIDIA; device panels must populate.
- Run as root or with `CAP_SYS_ADMIN` on NVIDIA; per-process GPU% should
  populate when the driver supports it.
- Run with `--no-pid-gpu`: device telemetry must remain available, per-process
  inventory/accounting must be disabled, and process GPU%/VRAM must show `—`.
  The driver may still open `/dev/nvidia-uvm` during initialization or telemetry.
- Run with `--no-gpu` and verify NVML is not loaded/initialized and no NVIDIA
  devices are opened.
- Run on a multi-GPU host; each GPU must have a panel and process VRAM must
  sum across devices.
- Run on a multi-disk host; each physical disk must have a row whose numbers
  match `df` for the filesystems on it, and a disk with nothing mounted must
  show its size and `no mounted filesystem`.
- Resize the terminal through 40, 50, 60, 70, 76, 80, 100, and wide widths.
- Measure idle CPU at the default 1 s refresh and record host CPU count,
  process count, GPU count, and observed CPU cost.

See `SPEC.md` for the full engineering contract.

## Companion qualification

The normal installer installs only the monitor. Test optional installation in a
throwaway VM: a new ramcleaner config must have DRY_RUN=1 and T3 disabled; existing
operator config must be preserved; the verification pass must remain a preview.
Do not exercise real swapoff or kernel memory writes in CI or on a development
host. Before enabling T3 in production, assess application latency and pressure,
explicit swap priority/options, pending recovery after failures, and status/exit
reporting. `ztop-ramcleaner --recover` restores pending swap while suspended
without starting new memory actions. T4 is unavailable until service-specific idle probes are implemented.

## Performance measurement

Run the ignored collection benchmark explicitly:

```bash
cargo test --locked --release benchmark_collection_refresh -- --ignored --nocapture
```

Record CPU/process counts and compare repeated runs on the same host/workload.
This measures full collection time, not GPU sampling or rendering. Separately
measure TUI CPU use after warmup with default refresh; include GPU/temperature
flags and terminal dimensions with the result.

## Version preparation

Use `scripts/version.sh MAJOR.MINOR.PATCH "release change summary"` and review the
new changelog entry. Release cores must increase; timestamps identify builds and
do not establish ordering. Schema 2 consumers must accept nullable GPU sensors
and inspect device status/timestamps before using retained memory values.

## Installer validation — 2026-11-02

Version `0.9.5+20261102.1656`, Rust 1.88.0, Debian 13 on the CPU-only
workstation `acer-mini` (no NVIDIA runtime, PSI enabled, no zram):

- Running `./installer.sh` as `spabam` from this Dropbox-synced checkout failed
  15 s into the build: Dropbox renamed the selectively excluded `target/` to
  `target (Selective Sync Conflict)` seconds after cargo created it, and every
  later write failed with `No such file or directory`. The installer now builds
  in `~/.cache/ztop-target` unless `CARGO_TARGET_DIR` is set; the rerun built
  in 30 s and left no `target/` in the checkout.
- Ran the complete installer as `spabam` with a writable staging prefix; no
  sudo was needed, and the final snapshot verification passed with GPU
  monitoring reported unavailable. The privileged copy to `/usr/bin/ztop` was
  performed separately from the same build (`root:root`, mode `0755`, SHA-256
  `47892d72df92806abbae0e381df395c40d4fabee071f03e656cfeea51c84c20f`). No
  previous installation existed on this host.
- `./installer.sh --check` passed against the installed monitor. Bash syntax,
  ShellCheck, version consistency, the version fixture, all 51 shell fixtures,
  and the five runtime tests against the built binary passed.

No companion was installed, and no Rust sources changed in this release.

## NVIDIA installation validation — 2026-10-26

Version `0.9.3+20261026.0923`, Rust 1.88.0, Debian 13 on the Proxmox host
`hypervisor00`:

- Formatting, Clippy with warnings denied, and the locked release build passed.
- 67 Rust tests passed in both debug and release; the manual collection
  benchmark remained ignored. All 49 shell fixtures, five runtime tests,
  ShellCheck, Bash syntax, and version consistency/bump checks passed.
- Ran the complete installer as `spabam` using a writable staging prefix and
  `CARGO_TARGET_DIR=/home/spabam/.cache/ztop-target`. No sudo was needed for that
  installation, and its final GPU verification passed. The privileged final
  copy to `/usr/bin/ztop` was performed separately; it is `root:root`, mode
  `0755`, with SHA-256
  `406ca828b779e9b690ffae30c5b5ef5dbdcc7a8e73cabf5f53e3d1c1ab79bc40`,
  matching the release binary. The previous 0.8.18 binary was backed up under
  `/root/.cache/ztop-backups`.
- The physical NVIDIA A2 was present, but system NVML 615.71.10 could not
  initialize against the loaded 615.71.09 kernel driver. Xorg held the GPU
  open, and the running `7.0.14-20-pve` kernel had no installed NVIDIA module
  available for a safe reload. Driver reload and reboot were not attempted.
- Downloaded and extracted official NVIDIA package
  `libnvidia-ml1_615.71.09-2_amd64.deb`; verified its SHA-256 against APT metadata:
  `737b4ee1b3eb66c695948bad3d7c2bb72c1cfdb7a02e3f5e1e1f7a3576d1913e`.
  Installed only its NVML runtime and license under
  `/usr/lib/ztop/nvml/615.71.09/`, without downgrading system packages. The
  source, checksum, and cleanup instructions are recorded in that directory's
  `README.txt`.
- Normal `ztop` invocation as `spabam` detected NVIDIA A2 UUID
  `GPU-d3c770ac-f146-47e6-376b-266092377c44` with fresh telemetry. Four stream
  samples matched `nvidia-smi` run with the private runtime: 15356 MiB VRAM,
  valid utilization, and temperature within 3 C. Per-process VRAM included
  Xorg. A 140-by-36 terminal rendered the A2 and VRAM panels, quit successfully,
  and restored terminal flags.
- `./installer.sh --check` passed for the installed monitor while preserving
  the system mismatch warning. Healthy system NVML takes precedence over the
  private runtime when the host driver versions match again. System-wide
  `nvidia-smi` and CUDA driver consistency still require a coordinated host
  repair or reboot into an appropriate installed kernel and driver.

No companion update or active memory-management pass was performed. Physical
multi-GPU recovery, a host without NVIDIA, and the broader public-release
hardware checklist remain separate qualifications; the automated fixtures
cover the corresponding installer and selection branches.

## Local validation — 2026-09-05

Version `0.9.0+20260905.2041`, Rust 1.88.0:

- Formatting, Clippy with warnings denied, and release build passed.
- 62 Rust tests passed in both debug and release; the separate collection
  benchmark also passed.
- 28 shell fixture tests, five built-binary runtime tests, ShellCheck, Bash
  syntax, version consistency/bump fixtures, and installer inspection passed.
- NVIDIA RTX PRO 6000 Blackwell Workstation Edition returned fresh schema-2
  observations as root and as unprivileged user `spabam`. Partial multi-device
  failures/recovery were exercised with fake backends, not hardware resets.
- Runtime tests verified process leaders only, filtered NDJSON/count behavior,
  clean closed-pipe exit, and Ctrl-C terminal restoration during search.
- Collection benchmark: 745 processes, 30 collections, mean 4.591 ms/sample.
- Short paired TUI comparison on this 32-logical-CPU host: 1 s refresh,
  `--no-gpu --no-temp`, 100×24 terminal, 2 s warmup, 6 s measurement, three
  runs per binary. Original CPU percentages of one core: 1.833, 2.167, 1.667;
  updated: 0.833, 0.667, 0.667. Medians: **1.833% → 0.667%**. This is a local
  workload observation, not qualification of the eight-core release target.

This initial 0.9.0 qualification preceded system installation. No active
ramcleaner operation was performed. Real swap recycling, crash recovery,
multi-GPU hardware, and a host without NVIDIA remain manual release
qualifications; automated tests use fixtures for these paths.

## Installation follow-up — 2026-09-05

- Installed `0.9.1+20260905.2102` at `/usr/bin/ztop` as `root:root`, mode `0755`;
  its SHA-256 checksum matched the release binary. The prior 0.7.6 binary was backed up under
  `/root/.cache/ztop-install-backups`.
- All five runtime tests passed against the installed binary as `spabam`.
- Tracing `open`, `openat`, and `openat2` with `strace -f` for installed
  `ztop --no-config --no-gpu --no-temp --json --top 1` showed no
  `libnvidia-ml` load or `/dev/nvidia*` opens; JSON contained `gpus: []`.
- A real-GPU PTY run exercised pause, help/Esc, resizing to 40×12, resume,
  search, and Ctrl-C. It exited with status 0 and restored terminal flags.
- Three GPU stream samples each as root and `spabam` matched `nvidia-smi`
  device identity and had disabled per-process accounting with `--no-pid-gpu`.
- Installation testing found that this NVIDIA driver opens `/dev/nvidia-uvm`
  inside `nvmlInit_v2`, even without process queries. `nvmlInitWithFlags` with
  `NO_ATTACH` also opened it. Version 0.9.1 corrects the help, diagnostics, and
  documentation; `--no-pid-gpu` only skips per-process accounting.

The monitor was installed directly from the locked release build using the
installer's mutation lock, since this root session cannot complete the normal
user installer's interactive sudo authentication. No companion was installed.

## Process VRAM percentage update — 2026-09-05

Version `0.9.2+20260905.2107` displays per-process VRAM as a percentage of the
combined GPU memory capacity, with one decimal place. Formatting, Clippy, and
all 62 Rust tests passed, including the updated table checks for single/multiple
GPUs and unavailable capacity. The installed `/usr/bin/ztop` checksum matched
the release build; the previous installed binary was backed up.
