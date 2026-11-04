// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Serialize;
use std::collections::HashMap;
use sysinfo::{Process, System, Users};

use crate::app::SortMode;
use crate::metrics::gpu::PerPidGpu;

#[derive(Serialize)]
pub struct ProcRow {
    pub pid: u32,
    pub user: String,
    pub cpu_pct: f32,
    pub ram_pct: f32,
    pub gpu_pct: Option<u32>,
    pub vram_bytes: u64,
    pub command: String,
}

#[allow(clippy::too_many_arguments)]
pub fn collect(
    sys: &System,
    users: &Users,
    per_pid_gpu: &HashMap<u32, PerPidGpu>,
    vram_total_bytes: u64,
    sort: SortMode,
    n_logical_cpus: usize,
    cmd_cache: &mut HashMap<(u32, u64), String>,
) -> Vec<ProcRow> {
    let total_ram = sys.total_memory().max(1) as f32;

    // Entries left in `retired` after the loop belong to dead processes and
    // are dropped with it; live ones move back into `cmd_cache`.
    let mut retired = std::mem::take(cmd_cache);

    let mut rows: Vec<ProcRow> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let pid_u32 = pid.as_u32();
            let user = p
                .user_id()
                .and_then(|uid| users.get_user_by_id(uid))
                .map(|u| u.name().to_string())
                .unwrap_or_else(|| "?".into());
            let ram_bytes = p.memory();
            let ram_pct = (ram_bytes as f32 / total_ram) * 100.0;
            let (gpu_pct, vram_bytes) = per_pid_gpu
                .get(&pid_u32)
                .map(|e| (e.util_pct, e.vram_bytes))
                .unwrap_or((None, 0));
            let cache_key = (pid_u32, p.start_time());
            let command = retired
                .remove(&cache_key)
                .unwrap_or_else(|| command_line(p));
            cmd_cache.insert(cache_key, command.clone());
            ProcRow {
                pid: pid_u32,
                user,
                cpu_pct: p.cpu_usage(),
                ram_pct,
                gpu_pct,
                vram_bytes,
                command,
            }
        })
        .collect();

    sort_rows(&mut rows, sort, n_logical_cpus, vram_total_bytes);
    rows
}

pub(crate) fn sort_rows(
    rows: &mut [ProcRow],
    sort: SortMode,
    n_cpus: usize,
    vram_total_bytes: u64,
) {
    let vram_total = vram_total_bytes.max(1) as f32;
    let n_cpus = n_cpus.max(1) as f32;
    let key = |r: &ProcRow| -> f32 {
        let value = match sort {
            SortMode::Cpu => r.cpu_pct,
            SortMode::Ram => r.ram_pct,
            SortMode::Gpu => r.gpu_pct.unwrap_or(0) as f32,
            SortMode::Vram => r.vram_bytes as f32,
            SortMode::Total => {
                let cpu_share = r.cpu_pct / n_cpus;
                let vram_pct = (r.vram_bytes as f32 / vram_total) * 100.0;
                [
                    cpu_share,
                    r.ram_pct,
                    r.gpu_pct.unwrap_or(0) as f32,
                    vram_pct,
                ]
                .into_iter()
                .fold(0.0_f32, f32::max)
            }
        };
        // A non-finite reading must not break the comparator's total order
        // (std's sort may panic on inconsistent comparisons since Rust 1.81).
        if value.is_finite() {
            value
        } else {
            0.0
        }
    };
    // Descending by key; ties broken by PID so equal rows (typically idle
    // processes at 0.0) keep a stable order instead of following HashMap
    // iteration order from tick to tick.
    rows.sort_by(|a, b| key(b).total_cmp(&key(a)).then_with(|| a.pid.cmp(&b.pid)));
}

fn command_line(process: &Process) -> String {
    let cmd = process
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");

    if cmd.trim().is_empty() {
        process.name().to_string_lossy().to_string()
    } else {
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::{sort_rows, ProcRow};
    use crate::app::SortMode;

    fn row(pid: u32, cpu: f32, ram: f32, gpu: Option<u32>, vram: u64) -> ProcRow {
        ProcRow {
            pid,
            user: "u".into(),
            cpu_pct: cpu,
            ram_pct: ram,
            gpu_pct: gpu,
            vram_bytes: vram,
            command: "cmd".into(),
        }
    }

    fn pids(rows: &[ProcRow]) -> Vec<u32> {
        rows.iter().map(|r| r.pid).collect()
    }

    #[test]
    fn sorts_descending_by_each_single_key() {
        let mut rows = vec![
            row(1, 10.0, 30.0, Some(5), 100),
            row(2, 90.0, 10.0, None, 900),
            row(3, 50.0, 60.0, Some(80), 0),
        ];

        sort_rows(&mut rows, SortMode::Cpu, 8, 1_000);
        assert_eq!(pids(&rows), [2, 3, 1]);

        sort_rows(&mut rows, SortMode::Ram, 8, 1_000);
        assert_eq!(pids(&rows), [3, 1, 2]);

        sort_rows(&mut rows, SortMode::Gpu, 8, 1_000);
        assert_eq!(pids(&rows), [3, 1, 2]);

        sort_rows(&mut rows, SortMode::Vram, 8, 1_000);
        assert_eq!(pids(&rows), [2, 1, 3]);
    }

    #[test]
    fn total_normalizes_cpu_by_logical_core_count() {
        // 400% CPU on 8 cores is a 50 device-wide share — less than 60% RAM.
        let mut rows = vec![row(1, 400.0, 0.0, None, 0), row(2, 0.0, 60.0, None, 0)];
        sort_rows(&mut rows, SortMode::Total, 8, 1_000);
        assert_eq!(pids(&rows), [2, 1]);

        // On a single core, the same 90% CPU beats 60% RAM.
        let mut rows = vec![row(1, 90.0, 0.0, None, 0), row(2, 0.0, 60.0, None, 0)];
        sort_rows(&mut rows, SortMode::Total, 1, 1_000);
        assert_eq!(pids(&rows), [1, 2]);
    }

    #[test]
    fn total_compares_vram_as_percentage_of_device_total() {
        // 800 of 1000 VRAM bytes = 80% beats 70% GPU util.
        let mut rows = vec![row(1, 0.0, 0.0, Some(70), 0), row(2, 0.0, 0.0, None, 800)];
        sort_rows(&mut rows, SortMode::Total, 8, 1_000);
        assert_eq!(pids(&rows), [2, 1]);
    }

    #[test]
    fn ties_break_by_pid_and_non_finite_values_sort_as_zero() {
        let mut rows = vec![
            row(30, 0.0, 0.0, None, 0),
            row(10, f32::NAN, 0.0, None, 0),
            row(20, 0.0, 0.0, None, 0),
            row(5, 1.0, 0.0, None, 0),
            row(1, f32::INFINITY, 0.0, None, 0),
        ];
        sort_rows(&mut rows, SortMode::Cpu, 8, 1_000);
        assert_eq!(pids(&rows), [5, 1, 10, 20, 30]);
    }

    #[test]
    fn missing_gpu_percentages_sort_as_zero() {
        let mut rows = vec![row(1, 0.0, 0.0, None, 0), row(2, 0.0, 0.0, Some(1), 0)];
        sort_rows(&mut rows, SortMode::Gpu, 8, 1_000);
        assert_eq!(pids(&rows), [2, 1]);
    }
}
