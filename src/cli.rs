// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

const LONG_ABOUT: &str = "\
A real-time terminal dashboard for CPU, RAM, swap, GPU, VRAM, and the top
resource-consuming processes. Designed for Linux workstations, GPU servers,
AI research machines, and developers who need a focused, fast monitor.

Features:
  - CPU model with core/thread counts, total + per-core utilization, package temperature
  - RAM and swap used/total/percentage (merged panel)
  - NVIDIA GPU model, utilization, temperature, VRAM used/total/percentage
  - 250 ms GPU sampling with a rolling peak hold for short utilization spikes
  - Top-N processes with PID, user, CPU%, RAM%, GPU%, VRAM%, command
  - Per-process VRAM via NVML; per-process GPU% when permitted (root or CAP_SYS_ADMIN)
  - --no-pid-gpu skips per-process NVML accounting; the NVIDIA driver may still
    open /dev/nvidia-uvm for device telemetry. Use --no-gpu to skip NVML entirely
  - Multi-GPU: every NVIDIA device gets its own panel; per-process VRAM is
    summed across devices
  - 1-second refresh by default, configurable 0.25-5.0s
  - Top-N process table, configurable 1-200 rows (also adjustable live with [ and ])
  - --json prints one metrics snapshot; --json-stream emits newline-delimited JSON
  - Process search, scrolling, and compact panels
  - --verbose prints detailed GPU init diagnostics before the TUI starts
  - Single executable, no libsensors or nvidia-smi runtime dependencies
  - Responsive layout that adapts to narrow and short terminals

JSON output:
  schema_version = 2. All memory fields are bytes; CPU/RAM/GPU percentages
  are percentages; temperatures are Celsius; sampled_at_unix_ms is UTC Unix
  time in milliseconds. GPU id, index, status, and sampled_at_unix_ms identify each
  device and observation; unavailable utilization and temperature are null.
  cpu_physical_cores is the physical core count (null when the host hides it);
  the thread count is the length of cpu_per_core_pct.

Configuration file (optional):
  ~/.config/ztop/config.toml is read at startup ($XDG_CONFIG_HOME honored).
  Recognized keys: refresh_interval, sort_by, top_processes, show_gpu,
  show_temperatures, pid_gpu, gpu_peak_hold, process_filter. Command-line flags
  always win. Use --config PATH or --no-config to control config loading.

Color palette:
  Cyan       section labels (CPU, RAM, SWAP, GPU, VRAM, temp, refresh:, host:, ...)
  Grey       CPU and GPU brand strings
  Yellow     temperature readings
  Green      bar fill when usage < 50%
  Yellow     bar fill when usage 50-80%
  Red        bar fill when usage >= 80%
  Dark gray  empty bar cells";

const AFTER_LONG_HELP: &str = "\
Keyboard (while running):
  q / Ctrl-C quit (Esc dismisses search/help before quitting)
  c          sort by CPU
  r          sort by RAM
  g          sort by GPU
  v          sort by VRAM
  t          sort by total (normalized max)
  Space      pause / resume
  + / -      adjust refresh rate by +/- 0.25 s (clamped 0.25..5.0)
  [ / ]      show one fewer / one more process row (clamped 1..200)
  ?          toggle the help overlay
  /          search commands, users, and PIDs; Enter accepts, Esc cancels
  Up / Down  scroll process rows (PageUp / PageDown scroll a page)
  Home / End jump to the first / last process page
  Ctrl-U     clear the filter while editing
  { / }      page through GPU panels

Examples:
  ztop                        Default 1-second refresh, sorted by CPU
  ztop --refresh 0.5          Half-second refresh
  ztop --gpu-peak-hold 5      Keep GPU/VRAM peaks visible for 5 seconds
  ztop --sort total --top 20  Top 20 processes ranked by normalized score
  ztop --no-gpu               Skip NVML; useful on non-NVIDIA hosts
  ztop --no-pid-gpu           Keep GPU panel but skip per-process VRAM/GPU%
  ztop --json                 Print one snapshot as JSON and exit
  ztop --json-stream --count 5 Emit five snapshots at the refresh interval
  ztop --filter python        Show matching commands, users, or PIDs
  ztop --no-config --gpu      Ignore config defaults and enable GPU collection
  ztop --verbose              Print GPU init diagnostics before the TUI starts

Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
Dual-licensed MIT OR Apache-2.0";

const VERSION_TEXT: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (c) 2026 Andrea Bodei <info@andreabodei.com>"
);

#[derive(Parser, Debug)]
#[command(
    name = "ztop",
    version = VERSION_TEXT,
    about = "A simple, GPU-aware Linux system monitor",
    long_about = LONG_ABOUT,
    after_long_help = AFTER_LONG_HELP,
)]
pub struct Cli {
    /// Refresh interval in seconds (0.25..=5.0)
    #[arg(short, long, default_value_t = 1.0, value_parser = parse_refresh)]
    pub refresh: f64,

    /// Initial process sort mode
    #[arg(short, long, value_enum, default_value_t = SortArg::Cpu)]
    pub sort: SortArg,

    /// Number of processes shown in the process table (1..=200)
    #[arg(short = 'n', long, default_value_t = 10, value_parser = parse_top)]
    pub top: usize,

    /// Case-insensitive process filter matching command, user, or PID
    #[arg(long, default_value = "", value_name = "TEXT")]
    pub filter: String,

    /// Skip GPU initialization entirely
    #[arg(long)]
    pub no_gpu: bool,

    /// Enable GPU collection even when disabled in the config file
    #[arg(long, conflicts_with = "no_gpu")]
    pub gpu: bool,

    /// Skip per-process GPU/VRAM accounting (keeps device-level GPU metrics).
    /// The per-PID GPU% and VRAM columns will read "—" for every process.
    /// NVIDIA's driver may still open /dev/nvidia-uvm during initialization
    /// or device queries. Use --no-gpu to skip NVML entirely.
    #[arg(long)]
    pub no_pid_gpu: bool,

    /// Enable per-process GPU accounting even when disabled in config
    #[arg(long, conflicts_with = "no_pid_gpu")]
    pub pid_gpu: bool,

    /// Skip CPU temperature probing
    #[arg(long)]
    pub no_temp: bool,

    /// Enable CPU temperature probing even when disabled in config
    #[arg(long, conflicts_with = "no_temp")]
    pub temp: bool,

    /// Rolling GPU peak hold in seconds; 0 disables peak display (0..=30)
    #[arg(long, default_value_t = 3.0, value_parser = parse_gpu_peak_hold, value_name = "SECONDS")]
    pub gpu_peak_hold: f64,

    /// Print one metrics snapshot as JSON to stdout and exit (no TUI)
    #[arg(long, conflicts_with = "json_stream")]
    pub json: bool,

    /// Emit one compact JSON snapshot per line at the refresh interval
    #[arg(long)]
    pub json_stream: bool,

    /// Stop streaming after this many snapshots (otherwise stream until stopped)
    #[arg(long, requires = "json_stream", value_parser = clap::value_parser!(u64).range(1..))]
    pub count: Option<u64>,

    /// Read this config file instead of the default location
    #[arg(long, value_name = "PATH", conflicts_with = "no_config")]
    pub config: Option<PathBuf>,

    /// Ignore the default config file
    #[arg(long)]
    pub no_config: bool,

    /// Print detailed startup diagnostics before entering the TUI
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SortArg {
    Cpu,
    Ram,
    Gpu,
    Vram,
    Total,
}

pub(crate) const REFRESH_RANGE: std::ops::RangeInclusive<f64> = 0.25..=5.0;
pub(crate) const TOP_RANGE: std::ops::RangeInclusive<usize> = 1..=200;
pub(crate) const GPU_PEAK_HOLD_RANGE: std::ops::RangeInclusive<f64> = 0.0..=30.0;

fn parse_refresh(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|e: std::num::ParseFloatError| e.to_string())?;
    if !REFRESH_RANGE.contains(&v) {
        return Err(format!("refresh must be in 0.25..=5.0 (got {v})"));
    }
    Ok(v)
}

fn parse_top(s: &str) -> Result<usize, String> {
    let v: usize = s
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    if !TOP_RANGE.contains(&v) {
        return Err(format!("top must be in 1..=200 (got {v})"));
    }
    Ok(v)
}

fn parse_gpu_peak_hold(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|e: std::num::ParseFloatError| e.to_string())?;
    if !GPU_PEAK_HOLD_RANGE.contains(&v) {
        return Err(format!("gpu-peak-hold must be in 0..=30 (got {v})"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use clap::{error::ErrorKind, Parser};

    use super::{parse_gpu_peak_hold, parse_refresh, parse_top, Cli};

    #[test]
    fn parses_refresh_bounds() {
        assert_eq!(parse_refresh("0.25").unwrap(), 0.25);
        assert_eq!(parse_refresh("5").unwrap(), 5.0);
        assert!(parse_refresh("0.24").is_err());
        assert!(parse_refresh("5.01").is_err());
    }

    #[test]
    fn parses_top_bounds() {
        assert_eq!(parse_top("1").unwrap(), 1);
        assert_eq!(parse_top("200").unwrap(), 200);
        assert!(parse_top("0").is_err());
        assert!(parse_top("201").is_err());
    }

    #[test]
    fn parses_gpu_peak_hold_bounds() {
        assert_eq!(parse_gpu_peak_hold("0").unwrap(), 0.0);
        assert_eq!(parse_gpu_peak_hold("3").unwrap(), 3.0);
        assert_eq!(parse_gpu_peak_hold("30").unwrap(), 30.0);
        assert!(parse_gpu_peak_hold("-0.1").is_err());
        assert!(parse_gpu_peak_hold("30.1").is_err());
    }

    #[test]
    fn version_flag_prints_package_version() {
        let err = Cli::try_parse_from(["ztop", "--version"]).unwrap_err();
        let output = err.to_string();

        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert!(output.contains(env!("CARGO_PKG_VERSION")));
        assert!(output.contains("Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>"));
    }

    #[test]
    fn conflicting_modes_and_flags_are_rejected() {
        for args in [
            vec!["ztop", "--json", "--json-stream"],
            vec!["ztop", "--count", "2"],
            vec!["ztop", "--json-stream", "--count", "0"],
            vec!["ztop", "--gpu", "--no-gpu"],
            vec!["ztop", "--pid-gpu", "--no-pid-gpu"],
            vec!["ztop", "--temp", "--no-temp"],
            vec!["ztop", "--config", "config.toml", "--no-config"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
