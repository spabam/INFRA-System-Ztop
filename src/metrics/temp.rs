// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fs;
use std::path::{Path, PathBuf};

pub struct CpuTempReader {
    source: Option<PathBuf>,
}

impl CpuTempReader {
    /// Cache the selected CPU package temperature path.
    ///
    /// Strategy (per SPEC §2.3):
    ///   1. Walk /sys/class/hwmon/*, prefer entries named `coretemp` (Intel), `k10temp` or `zenpower` (AMD).
    ///   2. Within chosen hwmon, prefer tempN_label matching "Package id 0" or "Tctl",
    ///      else temp1_input.
    ///   3. Fallback: /sys/class/thermal/thermal_zone* with type "x86_pkg_temp",
    ///      else first available zone.
    pub fn new() -> Self {
        Self {
            source: find_cpu_temp_path(),
        }
    }

    pub fn read(&mut self) -> Option<f32> {
        if let Some(path) = &self.source {
            if let Some(temp) = read_temp_input(path) {
                return Some(temp);
            }
        }

        self.source = find_cpu_temp_path();
        self.source.as_deref().and_then(read_temp_input)
    }
}

fn find_cpu_temp_path() -> Option<PathBuf> {
    find_hwmon_temp_path(Path::new("/sys/class/hwmon"))
        .or_else(|| find_thermal_zone_temp_path(Path::new("/sys/class/thermal")))
}

fn find_hwmon_temp_path(hwmon_root: &Path) -> Option<PathBuf> {
    const PREFERRED: &[&str] = &["coretemp", "k10temp", "zenpower"];
    let entries = fs::read_dir(hwmon_root).ok()?;

    let mut candidates: Vec<(usize, PathBuf, String)> = Vec::new();
    for ent in entries.flatten() {
        let path = ent.path();
        let name = fs::read_to_string(path.join("name"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let rank = PREFERRED
            .iter()
            .position(|p| *p == name)
            .unwrap_or(usize::MAX);
        candidates.push((rank, path, name));
    }
    candidates.sort_by_key(|c| c.0);

    for (rank, dir, _name) in candidates {
        if rank == usize::MAX {
            continue; // only use known CPU sensors at this stage
        }
        if let Some(path) = labeled_temp_path(&dir) {
            return Some(path);
        }
        let path = dir.join("temp1_input");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn labeled_temp_path(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for ent in entries.flatten() {
        let p = ent.path();
        let fname = p.file_name()?.to_str()?.to_string();
        if !fname.ends_with("_label") {
            continue;
        }
        let label = fs::read_to_string(&p).ok()?.trim().to_string();
        if label == "Package id 0" || label == "Tctl" || label == "Tdie" {
            let input = dir.join(fname.replace("_label", "_input"));
            if input.exists() {
                return Some(input);
            }
        }
    }
    None
}

fn read_temp_input(path: &Path) -> Option<f32> {
    let s = fs::read_to_string(path).ok()?;
    let v: i32 = s.trim().parse().ok()?;
    Some(v as f32 / 1000.0)
}

fn find_thermal_zone_temp_path(thermal_root: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(thermal_root).ok()?;
    let mut zones: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("thermal_zone"))
                .unwrap_or(false)
        })
        .collect();
    zones.sort();

    // Prefer x86_pkg_temp
    for z in &zones {
        let ty = fs::read_to_string(z.join("type"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if ty == "x86_pkg_temp" {
            let path = z.join("temp");
            if path.exists() {
                return Some(path);
            }
        }
    }
    // Fallback to first readable zone
    for z in &zones {
        let path = z.join("temp");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{find_hwmon_temp_path, find_thermal_zone_temp_path, read_temp_input};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Unique fixture root per test, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(test: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("ztop-temp-{}-{test}", std::process::id()));
            fs::remove_dir_all(&root).ok();
            fs::create_dir_all(&root).unwrap();
            Fixture(root)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn file(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn prefers_labeled_package_input_on_known_sensor() {
        let fx = Fixture::new("intel");
        // A non-CPU hwmon that must be ignored even though it sorts first.
        fx.file("hwmon0/name", "nvme\n");
        fx.file("hwmon0/temp1_input", "70000\n");
        // coretemp with a labeled package sensor plus a per-core sensor.
        fx.file("hwmon1/name", "coretemp\n");
        fx.file("hwmon1/temp1_label", "Core 0\n");
        fx.file("hwmon1/temp1_input", "38000\n");
        fx.file("hwmon1/temp2_label", "Package id 0\n");
        fx.file("hwmon1/temp2_input", "45000\n");

        let picked = find_hwmon_temp_path(&fx.0).unwrap();
        assert_eq!(picked, fx.path("hwmon1/temp2_input"));
        assert_eq!(read_temp_input(&picked), Some(45.0));
    }

    #[test]
    fn amd_tctl_label_is_recognized() {
        let fx = Fixture::new("amd");
        fx.file("hwmon0/name", "k10temp\n");
        fx.file("hwmon0/temp1_label", "Tctl\n");
        fx.file("hwmon0/temp1_input", "52500\n");

        let picked = find_hwmon_temp_path(&fx.0).unwrap();
        assert_eq!(read_temp_input(&picked), Some(52.5));
    }

    #[test]
    fn unlabeled_known_sensor_falls_back_to_temp1_input() {
        let fx = Fixture::new("nolabel");
        fx.file("hwmon0/name", "zenpower\n");
        fx.file("hwmon0/temp1_input", "41000\n");

        let picked = find_hwmon_temp_path(&fx.0).unwrap();
        assert_eq!(picked, fx.path("hwmon0/temp1_input"));
    }

    #[test]
    fn unknown_sensors_only_yields_none() {
        let fx = Fixture::new("unknown");
        fx.file("hwmon0/name", "iwlwifi_1\n");
        fx.file("hwmon0/temp1_input", "60000\n");

        assert_eq!(find_hwmon_temp_path(&fx.0), None);
    }

    #[test]
    fn thermal_zone_prefers_x86_pkg_temp() {
        let fx = Fixture::new("zones");
        fx.file("thermal_zone0/type", "acpitz\n");
        fx.file("thermal_zone0/temp", "30000\n");
        fx.file("thermal_zone1/type", "x86_pkg_temp\n");
        fx.file("thermal_zone1/temp", "47000\n");
        fx.dir("cooling_device0"); // must be ignored

        let picked = find_thermal_zone_temp_path(&fx.0).unwrap();
        assert_eq!(picked, fx.path("thermal_zone1/temp"));
    }

    #[test]
    fn thermal_zone_falls_back_to_first_zone() {
        let fx = Fixture::new("zonefallback");
        fx.file("thermal_zone0/type", "acpitz\n");
        fx.file("thermal_zone0/temp", "30000\n");

        let picked = find_thermal_zone_temp_path(&fx.0).unwrap();
        assert_eq!(picked, fx.path("thermal_zone0/temp"));
    }

    #[test]
    fn read_temp_input_parses_millidegrees() {
        let fx = Fixture::new("read");
        fx.file("t", "45500\n");
        assert_eq!(read_temp_input(&fx.path("t")), Some(45.5));
        assert_eq!(read_temp_input(Path::new("/nonexistent/ztop")), None);
    }
}
