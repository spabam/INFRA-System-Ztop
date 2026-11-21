// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Optional config file: `~/.config/ztop/config.toml` ($XDG_CONFIG_HOME honored).
//!
//! Precedence: command-line flag (explicitly given) > config file > built-in
//! default. Explicitness is detected via clap's `ValueSource`, so `-r 1`
//! still overrides a config `refresh_interval` even though 1.0 is also the
//! default value.

use anyhow::{anyhow, Context, Result};
use clap::parser::ValueSource;
use clap::ArgMatches;
use serde::Deserialize;
use std::path::PathBuf;

use crate::cli::{Cli, SortArg, GPU_PEAK_HOLD_RANGE, REFRESH_RANGE, TOP_RANGE};

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub refresh_interval: Option<f64>,
    pub sort_by: Option<String>,
    pub top_processes: Option<usize>,
    pub show_gpu: Option<bool>,
    pub show_temperatures: Option<bool>,
    pub pid_gpu: Option<bool>,
    pub gpu_peak_hold: Option<f64>,
    pub process_filter: Option<String>,
}

pub fn default_path() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => Some(PathBuf::from(x).join("ztop/config.toml")),
        _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/ztop/config.toml")),
    }
}

/// Load the default config file (if any) and fold it into `cli` for every
/// option the user did not set explicitly on the command line.
pub fn apply_default_file(cli: &mut Cli, matches: &ArgMatches) -> Result<()> {
    if cli.no_config {
        return Ok(());
    }
    let Some(path) = cli.config.clone().or_else(default_path) else {
        return Ok(());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && cli.config.is_none() => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let cfg: FileConfig =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    apply(cli, matches, &cfg).with_context(|| format!("in {}", path.display()))
}

fn apply(cli: &mut Cli, matches: &ArgMatches, cfg: &FileConfig) -> Result<()> {
    let from_cli = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);

    if let Some(v) = cfg.refresh_interval.filter(|_| !from_cli("refresh")) {
        if !REFRESH_RANGE.contains(&v) {
            return Err(anyhow!("refresh_interval must be in 0.25..=5.0 (got {v})"));
        }
        cli.refresh = v;
    }
    if let Some(s) = cfg.sort_by.as_deref().filter(|_| !from_cli("sort")) {
        cli.sort = <SortArg as clap::ValueEnum>::from_str(s, true)
            .map_err(|_| anyhow!("sort_by must be one of cpu|ram|gpu|vram|total (got {s:?})"))?;
    }
    if let Some(v) = cfg.top_processes.filter(|_| !from_cli("top")) {
        if !TOP_RANGE.contains(&v) {
            return Err(anyhow!("top_processes must be in 1..=200 (got {v})"));
        }
        cli.top = v;
    }
    if let Some(v) = cfg.gpu_peak_hold.filter(|_| !from_cli("gpu_peak_hold")) {
        if !GPU_PEAK_HOLD_RANGE.contains(&v) {
            return Err(anyhow!("gpu_peak_hold must be in 0..=30 (got {v})"));
        }
        cli.gpu_peak_hold = v;
    }
    if let Some(filter) = &cfg.process_filter {
        if !from_cli("filter") {
            cli.filter.clone_from(filter);
        }
    }
    if let Some(enabled) = cfg.show_gpu {
        if !from_cli("no_gpu") && !from_cli("gpu") {
            cli.no_gpu = !enabled;
        }
    }
    if let Some(enabled) = cfg.show_temperatures {
        if !from_cli("no_temp") && !from_cli("temp") {
            cli.no_temp = !enabled;
        }
    }
    if let Some(enabled) = cfg.pid_gpu {
        if !from_cli("no_pid_gpu") && !from_cli("pid_gpu") {
            cli.no_pid_gpu = !enabled;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches};

    use super::{apply, FileConfig};
    use crate::cli::{Cli, SortArg};

    fn parse(args: &[&str]) -> (Cli, clap::ArgMatches) {
        let matches = Cli::command().get_matches_from(args);
        let cli = Cli::from_arg_matches(&matches).unwrap();
        (cli, matches)
    }

    #[test]
    fn parses_readme_example() {
        let cfg: FileConfig = toml::from_str(
            r#"
            refresh_interval = 2.0
            sort_by = "ram"
            show_gpu = true
            show_temperatures = true
            top_processes = 15
            "#,
        )
        .unwrap();
        assert_eq!(cfg.refresh_interval, Some(2.0));
        assert_eq!(cfg.sort_by.as_deref(), Some("ram"));
        assert_eq!(cfg.top_processes, Some(15));
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<FileConfig>("refreshinterval = 1.0").is_err());
    }

    #[test]
    fn config_fills_in_unset_options() {
        let (mut cli, matches) = parse(&["ztop"]);
        let cfg: FileConfig = toml::from_str(
            "refresh_interval = 2.0\nsort_by = \"vram\"\ntop_processes = 20\nshow_gpu = false",
        )
        .unwrap();

        apply(&mut cli, &matches, &cfg).unwrap();

        assert_eq!(cli.refresh, 2.0);
        assert!(matches!(cli.sort, crate::cli::SortArg::Vram));
        assert_eq!(cli.top, 20);
        assert!(cli.no_gpu);
    }

    #[test]
    fn explicit_cli_flags_beat_config() {
        let (mut cli, matches) = parse(&["ztop", "--refresh", "1", "--sort", "cpu"]);
        let cfg: FileConfig = toml::from_str("refresh_interval = 2.0\nsort_by = \"vram\"").unwrap();

        apply(&mut cli, &matches, &cfg).unwrap();

        assert_eq!(cli.refresh, 1.0);
        assert!(matches!(cli.sort, crate::cli::SortArg::Cpu));
    }

    #[test]
    fn out_of_range_config_values_error() {
        let (mut cli, matches) = parse(&["ztop"]);

        let cfg: FileConfig = toml::from_str("refresh_interval = 9.0").unwrap();
        assert!(apply(&mut cli, &matches, &cfg).is_err());

        let cfg: FileConfig = toml::from_str("top_processes = 0").unwrap();
        assert!(apply(&mut cli, &matches, &cfg).is_err());

        let cfg: FileConfig = toml::from_str("sort_by = \"bogus\"").unwrap();
        assert!(apply(&mut cli, &matches, &cfg).is_err());
    }

    #[test]
    fn valid_explicit_options_override_invalid_file_values() {
        let (mut cli, matches) = parse(&[
            "ztop", "--refresh", "1", "--sort", "ram", "--top", "4", "--gpu-peak-hold", "3",
        ]);
        let cfg: FileConfig = toml::from_str(
            "refresh_interval = 9.0\nsort_by = 'bogus'\ntop_processes = 0\ngpu_peak_hold = 31.0",
        ).unwrap();
        apply(&mut cli, &matches, &cfg).unwrap();
        assert_eq!(cli.refresh, 1.0);
        assert!(matches!(cli.sort, SortArg::Ram));
        assert_eq!(cli.top, 4);
        assert_eq!(cli.gpu_peak_hold, 3.0);
    }

    #[test]
    fn positive_and_negative_flags_override_file_booleans() {
        let (mut cli, matches) = parse(&["ztop", "--gpu", "--pid-gpu", "--temp", "--filter", ""]);
        let cfg: FileConfig = toml::from_str(
            "show_gpu = false\npid_gpu = false\nshow_temperatures = false\nprocess_filter = 'python'",
        ).unwrap();
        apply(&mut cli, &matches, &cfg).unwrap();
        assert!(!cli.no_gpu && !cli.no_pid_gpu && !cli.no_temp);
        assert!(cli.filter.is_empty());

        let (mut cli, matches) = parse(&["ztop", "--no-gpu", "--no-pid-gpu", "--no-temp"]);
        let cfg: FileConfig =
            toml::from_str("show_gpu = true\npid_gpu = true\nshow_temperatures = true").unwrap();
        apply(&mut cli, &matches, &cfg).unwrap();
        assert!(cli.no_gpu && cli.no_pid_gpu && cli.no_temp);
    }

    #[test]
    fn explicit_config_path_is_loaded_and_missing_explicit_file_errors() {
        let path =
            std::env::temp_dir().join(format!("ztop-config-test-{}.toml", std::process::id()));
        struct RemoveFile(std::path::PathBuf);
        impl Drop for RemoveFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = RemoveFile(path.clone());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        file.write_all(b"refresh_interval = 0.5\nprocess_filter = 'python'")
            .unwrap();
        drop(file);
        let (mut cli, matches) = parse(&["ztop", "--config", path.to_str().unwrap()]);
        super::apply_default_file(&mut cli, &matches).unwrap();
        assert_eq!(cli.refresh, 0.5);
        assert_eq!(cli.filter, "python");
        std::fs::remove_file(&path).unwrap();
        assert!(super::apply_default_file(&mut cli, &matches).is_err());
    }
}
