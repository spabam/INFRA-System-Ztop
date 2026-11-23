// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

mod cpu;
mod disk;
mod gpu;
mod memory;
mod process;
mod temp;

use anyhow::Result;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{System, Users};

use crate::app::{AppState, SortMode};
use crate::cli::Cli;

pub use disk::DiskSnapshot;
pub use gpu::{GpuSampleStatus, GpuSnapshot, PerPidGpu};
pub use process::ProcRow;

/// How many consecutive failed GPU polls (250 ms apart) are tolerated before
/// the panel degrades to "n/a". A single transient NVML hiccup should not
/// blank the display or reset peak continuity.
const GPU_FAIL_GRACE_SAMPLES: u32 = 8;
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub sampled_at_unix_ms: u64,
    pub hostname: String,
    pub uptime_secs: u64,
    pub cpu_brand: String,
    /// Physical cores; `None` when the platform hides the topology. The
    /// thread (logical CPU) count is `cpu_per_core_pct.len()`.
    pub cpu_physical_cores: Option<usize>,
    pub cpu_total_pct: f32,
    pub cpu_per_core_pct: Vec<f32>,
    pub cpu_temp_c: Option<f32>,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub gpus: Vec<GpuSnapshot>,
    /// Physical disks in device-name order; empty where none can be found.
    pub disks: Vec<DiskSnapshot>,
    pub procs: Vec<ProcRow>,
}

impl Snapshot {
    pub fn sort_processes(&mut self, sort: SortMode) {
        let vram_total_bytes = self.vram_total_bytes();
        process::sort_rows(
            &mut self.procs,
            sort,
            self.cpu_per_core_pct.len().max(1),
            vram_total_bytes,
        );
    }

    pub(crate) fn vram_total_bytes(&self) -> u64 {
        self.gpus.iter().map(|g| g.vram_total_bytes).sum()
    }
}

/// Merge every device's per-PID table into one map for the process table:
/// VRAM sums across devices, GPU% takes the busiest device (so the value
/// stays on a 0..100 scale).
fn combined_per_pid(gpus: &[GpuSnapshot]) -> HashMap<u32, PerPidGpu> {
    let mut merged: HashMap<u32, PerPidGpu> = HashMap::new();
    for g in gpus.iter().filter(|g| g.status == GpuSampleStatus::Fresh) {
        for (pid, e) in &g.per_pid {
            let m = merged.entry(*pid).or_default();
            m.vram_bytes += e.vram_bytes;
            m.util_pct = match (m.util_pct, e.util_pct) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }
    }
    merged
}

pub struct Collector {
    sys: System,
    users: Users,
    last_user_refresh: Instant,
    gpu: Option<Box<dyn gpu::GpuBackend>>,
    gpu_snapshots: Vec<GpuSnapshot>,
    gpu_fail_streak: HashMap<String, u32>,
    gpu_peaks: HashMap<String, GpuPeakHistory>,
    gpu_peak_hold: Duration,
    gpu_sample_interval: Duration,
    last_gpu_sample: Option<Instant>,
    temp: Option<temp::CpuTempReader>,
    disks: disk::Probe,
    /// Physical core count, read once at startup because sysinfo re-parses
    /// `/proc/cpuinfo` on every query.
    cpu_physical_cores: Option<usize>,
    /// (pid, start_time) -> command line. Building the joined argv string for
    /// every process on every tick is the main per-tick allocation cost; the
    /// (pid, start_time) key uniquely identifies a process instance, so the
    /// string is built once per process lifetime.
    cmd_cache: HashMap<(u32, u64), String>,
}

impl Collector {
    pub fn new(cli: &Cli) -> Result<Self> {
        let mut sys = System::new();
        refresh_system(&mut sys);

        let gpu = if cli.no_gpu {
            None
        } else {
            match gpu::GpuProbe::init(!cli.no_pid_gpu, cli.verbose) {
                Ok(probe) => Some(Box::new(probe) as Box<dyn gpu::GpuBackend>),
                Err(e) => {
                    eprintln!("ztop: GPU init failed ({e}); continuing without GPU metrics");
                    None
                }
            }
        };

        Ok(Self {
            sys,
            users: Users::new_with_refreshed_list(),
            last_user_refresh: Instant::now(),
            gpu,
            gpu_snapshots: Vec::new(),
            gpu_fail_streak: HashMap::new(),
            gpu_peaks: HashMap::new(),
            gpu_peak_hold: Duration::from_millis((cli.gpu_peak_hold * 1000.0).round() as u64),
            gpu_sample_interval: Duration::from_millis(250),
            last_gpu_sample: None,
            temp: (!cli.no_temp).then(temp::CpuTempReader::new),
            disks: disk::Probe::default(),
            cpu_physical_cores: cpu::physical_count(),
            cmd_cache: HashMap::new(),
        })
    }

    pub fn sample_gpu(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        let snaps = gpu.poll().unwrap_or_default();
        let sampled_at = Instant::now();
        self.last_gpu_sample = Some(sampled_at);

        let mut observed = HashSet::with_capacity(snaps.len());
        for mut snap in snaps {
            observed.insert(snap.id.clone());
            self.gpu_fail_streak.insert(snap.id.clone(), 0);
            snap.status = GpuSampleStatus::Fresh;
            self.gpu_peaks
                .entry(snap.id.clone())
                .or_insert_with(|| GpuPeakHistory::new(self.gpu_peak_hold))
                .apply(sampled_at, &mut snap);
            // Match observations by identity rather than their position.
            if let Some(previous) = self.gpu_snapshots.iter_mut().find(|s| s.id == snap.id) {
                *previous = snap;
            } else {
                self.gpu_snapshots.push(snap);
            }
        }
        // A device first observed after startup belongs in its original NVML
        // slot, even when a higher-index device was observed first.
        self.gpu_snapshots.sort_by_key(|snap| snap.index);
        for snap in &mut self.gpu_snapshots {
            if observed.contains(&snap.id) {
                continue;
            }
            let failures = self.gpu_fail_streak.entry(snap.id.clone()).or_default();
            *failures = failures.saturating_add(1);
            // Stale PID data could be attributed to a different process after
            // PID reuse. Keep only the device's explicitly marked last sample.
            snap.per_pid.clear();
            if *failures >= GPU_FAIL_GRACE_SAMPLES {
                snap.status = GpuSampleStatus::Unavailable;
                snap.util_pct = None;
                snap.temp_c = None;
                snap.util_peak_pct = None;
                snap.vram_peak_used_bytes = None;
            } else {
                snap.status = GpuSampleStatus::Stale;
                if let Some(peaks) = self.gpu_peaks.get_mut(&snap.id) {
                    peaks.refresh_peaks(sampled_at, snap);
                }
            }
        }
    }

    pub fn sample_gpu_if_due(&mut self, now: Instant) {
        if self.gpu_sample_wait(now) == Some(Duration::ZERO) {
            self.sample_gpu();
        }
    }

    pub fn gpu_sample_wait(&self, now: Instant) -> Option<Duration> {
        self.gpu.as_ref()?;
        Some(match self.last_gpu_sample {
            Some(last) => self
                .gpu_sample_interval
                .saturating_sub(now.duration_since(last)),
            None => Duration::ZERO,
        })
    }

    pub fn collect(&mut self, app: &AppState) -> Snapshot {
        if self.last_user_refresh.elapsed() >= Duration::from_secs(60) {
            self.users.refresh();
            self.last_user_refresh = Instant::now();
        }

        refresh_system(&mut self.sys);

        let cpu_brand = cpu::brand(&self.sys);
        let cpu_total_pct = cpu::total_usage(&self.sys);
        let cpu_per_core_pct = cpu::per_core_usage(&self.sys);
        let cpu_temp_c = self.temp.as_mut().and_then(temp::CpuTempReader::read);

        let (ram_used_bytes, ram_total_bytes) = memory::used_total(&self.sys);
        let (swap_used_bytes, swap_total_bytes) = memory::swap_used_total(&self.sys);

        let gpus = self.gpu_snapshots.clone();
        let per_pid = combined_per_pid(&gpus);
        let vram_total_bytes: u64 = gpus.iter().map(|g| g.vram_total_bytes).sum();

        let procs = process::collect(
            &self.sys,
            &self.users,
            &per_pid,
            vram_total_bytes,
            app.sort,
            cpu::logical_count(&self.sys),
            &mut self.cmd_cache,
        );

        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            sampled_at_unix_ms: sampled_at_unix_ms(),
            hostname: sysinfo::System::host_name().unwrap_or_else(|| "unknown".into()),
            uptime_secs: sysinfo::System::uptime(),
            cpu_brand,
            cpu_physical_cores: self.cpu_physical_cores,
            cpu_total_pct,
            cpu_per_core_pct,
            cpu_temp_c,
            ram_used_bytes,
            ram_total_bytes,
            swap_used_bytes,
            swap_total_bytes,
            gpus,
            disks: self.disks.sample(),
            procs,
        }
    }
}

fn process_refresh_kind() -> sysinfo::ProcessRefreshKind {
    // `nothing()` still enables Linux tasks in sysinfo 0.38. Disable them
    // explicitly: thread IDs are not separate processes for this dashboard.
    sysinfo::ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cpu()
        .with_memory()
        .with_user(sysinfo::UpdateKind::OnlyIfNotSet)
        .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
}

fn refresh_system(sys: &mut System) {
    // Frequency is not displayed; avoid one additional sysfs read per core.
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        process_refresh_kind(),
    );
}

fn sampled_at_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

struct GpuPeakHistory {
    hold: Duration,
    samples: VecDeque<GpuPeakSample>,
}

struct GpuPeakSample {
    at: Instant,
    util_pct: Option<u32>,
    vram_used_bytes: u64,
}

impl GpuPeakHistory {
    fn new(hold: Duration) -> Self {
        Self {
            hold,
            samples: VecDeque::new(),
        }
    }

    fn apply(&mut self, now: Instant, snap: &mut GpuSnapshot) {
        if !self.hold.is_zero() {
            self.samples.push_back(GpuPeakSample {
                at: now,
                util_pct: snap.util_pct,
                vram_used_bytes: snap.vram_used_bytes,
            });
        }
        self.refresh_peaks(now, snap);
    }

    fn refresh_peaks(&mut self, now: Instant, snap: &mut GpuSnapshot) {
        if self.hold.is_zero() {
            self.samples.clear();
            snap.util_peak_pct = None;
            snap.vram_peak_used_bytes = None;
            return;
        }

        while self
            .samples
            .front()
            .is_some_and(|sample| now.duration_since(sample.at) > self.hold)
        {
            self.samples.pop_front();
        }

        snap.util_peak_pct = self
            .samples
            .iter()
            .filter_map(|sample| sample.util_pct)
            .max();
        snap.vram_peak_used_bytes = self
            .samples
            .iter()
            .map(|sample| sample.vram_used_bytes)
            .max();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        combined_per_pid, Collector, DiskSnapshot, GpuPeakHistory, GpuSampleStatus, GpuSnapshot,
        PerPidGpu, Snapshot, GPU_FAIL_GRACE_SAMPLES, SNAPSHOT_SCHEMA_VERSION,
    };
    use std::{
        collections::{HashMap, VecDeque},
        time::{Duration, Instant},
    };
    use sysinfo::{System, Users};

    enum FakePoll {
        Ok(Vec<GpuSnapshot>),
        Err(&'static str),
    }

    struct FakeGpuBackend {
        responses: VecDeque<FakePoll>,
    }

    impl FakeGpuBackend {
        fn new(responses: impl IntoIterator<Item = FakePoll>) -> Self {
            Self {
                responses: responses.into_iter().collect(),
            }
        }
    }

    impl super::gpu::GpuBackend for FakeGpuBackend {
        fn poll(&mut self) -> anyhow::Result<Vec<GpuSnapshot>> {
            match self
                .responses
                .pop_front()
                .unwrap_or(FakePoll::Err("fake backend exhausted"))
            {
                FakePoll::Ok(snaps) => Ok(snaps),
                FakePoll::Err(message) => Err(anyhow::anyhow!(message)),
            }
        }
    }

    fn collector_with_backend(backend: FakeGpuBackend) -> Collector {
        Collector {
            sys: System::new(),
            users: Users::new_with_refreshed_list(),
            last_user_refresh: Instant::now(),
            gpu: Some(Box::new(backend)),
            gpu_snapshots: Vec::new(),
            gpu_fail_streak: HashMap::new(),
            gpu_peaks: HashMap::new(),
            gpu_peak_hold: Duration::from_secs(3),
            gpu_sample_interval: Duration::from_millis(250),
            last_gpu_sample: None,
            temp: None,
            disks: super::disk::Probe::default(),
            cpu_physical_cores: None,
            cmd_cache: HashMap::new(),
        }
    }

    #[test]
    fn gpu_peak_history_holds_recent_maxima() {
        let mut peaks = GpuPeakHistory::new(Duration::from_secs(3));
        let start = std::time::Instant::now();

        let mut first = gpu_snapshot(10, 100);
        peaks.apply(start, &mut first);
        assert_eq!(first.util_peak_pct, Some(10));
        assert_eq!(first.vram_peak_used_bytes, Some(100));

        let mut spike = gpu_snapshot(96, 1_000);
        peaks.apply(start + Duration::from_secs(1), &mut spike);
        assert_eq!(spike.util_peak_pct, Some(96));
        assert_eq!(spike.vram_peak_used_bytes, Some(1_000));

        let mut after = gpu_snapshot(0, 200);
        peaks.apply(start + Duration::from_secs(2), &mut after);
        assert_eq!(after.util_peak_pct, Some(96));
        assert_eq!(after.vram_peak_used_bytes, Some(1_000));

        let mut expired = gpu_snapshot(5, 150);
        peaks.apply(start + Duration::from_secs(6), &mut expired);
        assert_eq!(expired.util_peak_pct, Some(5));
        assert_eq!(expired.vram_peak_used_bytes, Some(150));
    }

    #[test]
    fn gpu_peak_history_can_be_disabled() {
        let mut peaks = GpuPeakHistory::new(Duration::ZERO);
        let mut snap = gpu_snapshot(96, 1_000);

        peaks.apply(std::time::Instant::now(), &mut snap);

        assert_eq!(snap.util_peak_pct, None);
        assert_eq!(snap.vram_peak_used_bytes, None);
    }

    #[test]
    fn per_pid_merge_sums_vram_and_takes_max_util() {
        let mut gpu0 = gpu_snapshot(50, 100);
        gpu0.per_pid.insert(
            42,
            PerPidGpu {
                util_pct: Some(30),
                vram_bytes: 1_000,
            },
        );
        let mut gpu1 = gpu_snapshot(10, 200);
        gpu1.per_pid.insert(
            42,
            PerPidGpu {
                util_pct: Some(70),
                vram_bytes: 500,
            },
        );
        gpu1.per_pid.insert(
            7,
            PerPidGpu {
                util_pct: None,
                vram_bytes: 250,
            },
        );

        let merged = combined_per_pid(&[gpu0, gpu1]);

        let p42 = merged.get(&42).unwrap();
        assert_eq!(p42.vram_bytes, 1_500);
        assert_eq!(p42.util_pct, Some(70));
        let p7 = merged.get(&7).unwrap();
        assert_eq!(p7.vram_bytes, 250);
        assert_eq!(p7.util_pct, None);
    }

    #[test]
    fn stale_device_process_data_is_not_attributed_to_live_pids() {
        let mut stale = gpu_snapshot(90, 100);
        stale.status = GpuSampleStatus::Stale;
        stale.per_pid.insert(
            42,
            PerPidGpu {
                util_pct: Some(90),
                vram_bytes: 100,
            },
        );
        assert!(combined_per_pid(&[stale]).is_empty());
    }

    #[test]
    fn collector_applies_fake_multi_gpu_samples() {
        let backend = FakeGpuBackend::new([FakePoll::Ok(vec![
            gpu_snapshot_on(0, 10, 100),
            gpu_snapshot_on(1, 90, 900),
        ])]);
        let mut collector = collector_with_backend(backend);

        collector.sample_gpu();

        assert_eq!(collector.gpu_snapshots.len(), 2);
        assert_eq!(collector.gpu_snapshots[0].util_peak_pct, Some(10));
        assert_eq!(collector.gpu_snapshots[1].util_peak_pct, Some(90));
        assert_eq!(collector.gpu_snapshots[1].vram_peak_used_bytes, Some(900));
    }

    #[test]
    fn collector_retains_last_gpu_sample_until_failure_grace_expires() {
        let mut responses = vec![FakePoll::Ok(vec![gpu_snapshot(42, 1_024)])];
        responses
            .extend((0..GPU_FAIL_GRACE_SAMPLES).map(|_| FakePoll::Err("transient NVML failure")));
        let mut collector = collector_with_backend(FakeGpuBackend::new(responses));

        collector.sample_gpu();
        assert_eq!(collector.gpu_snapshots.len(), 1);

        for _ in 0..(GPU_FAIL_GRACE_SAMPLES - 1) {
            collector.sample_gpu();
            assert_eq!(collector.gpu_snapshots.len(), 1);
            assert_eq!(collector.gpu_snapshots[0].status, GpuSampleStatus::Stale);
            assert_eq!(collector.gpu_snapshots[0].util_pct, Some(42));
            assert_eq!(collector.gpu_snapshots[0].sampled_at_unix_ms, 1);
        }

        collector.sample_gpu();
        let snap = &collector.gpu_snapshots[0];
        assert_eq!(snap.status, GpuSampleStatus::Unavailable);
        assert_eq!(snap.util_pct, None);
        assert_eq!(snap.temp_c, None);
        assert_eq!(snap.util_peak_pct, None);
        assert_eq!(snap.vram_peak_used_bytes, None);
        assert_eq!(snap.sampled_at_unix_ms, 1);
    }

    #[test]
    fn partial_gpu_failure_preserves_device_identity_peaks_and_recovers() {
        let mut first = gpu_snapshot_on(0, 90, 1_500);
        first.per_pid.insert(
            42,
            PerPidGpu {
                util_pct: Some(90),
                vram_bytes: 1_000,
            },
        );
        let second = gpu_snapshot_on(1, 10, 100);
        let mut responses = vec![FakePoll::Ok(vec![first, second])];
        responses.extend(
            (0..GPU_FAIL_GRACE_SAMPLES).map(|_| FakePoll::Ok(vec![gpu_snapshot_on(1, 20, 200)])),
        );
        let mut recovered = gpu_snapshot_on(0, 30, 300);
        recovered.sampled_at_unix_ms = 2;
        // Reverse backend ordering to exercise identity rather than position.
        responses.push(FakePoll::Ok(vec![gpu_snapshot_on(1, 5, 50), recovered]));
        let mut collector = collector_with_backend(FakeGpuBackend::new(responses));
        collector.sample_gpu();
        collector.sample_gpu();
        assert_eq!(collector.gpu_snapshots[0].id, "gpu-0");
        assert_eq!(collector.gpu_snapshots[0].status, GpuSampleStatus::Stale);
        assert!(collector.gpu_snapshots[0].per_pid.is_empty());
        assert_eq!(collector.gpu_snapshots[1].id, "gpu-1");
        assert_eq!(collector.gpu_snapshots[1].status, GpuSampleStatus::Fresh);
        assert_eq!(collector.gpu_snapshots[1].util_peak_pct, Some(20));
        assert_eq!(collector.gpu_snapshots[1].vram_peak_used_bytes, Some(200));
        for _ in 1..GPU_FAIL_GRACE_SAMPLES {
            collector.sample_gpu();
        }
        assert_eq!(
            collector.gpu_snapshots[0].status,
            GpuSampleStatus::Unavailable
        );
        assert_eq!(collector.gpu_snapshots[1].status, GpuSampleStatus::Fresh);
        collector.sample_gpu();
        assert_eq!(collector.gpu_snapshots[0].status, GpuSampleStatus::Fresh);
        assert_eq!(collector.gpu_snapshots[0].sampled_at_unix_ms, 2);
        assert_eq!(collector.gpu_snapshots[0].util_pct, Some(30));
        assert_eq!(collector.gpu_snapshots[0].util_peak_pct, Some(90));
        assert_eq!(collector.gpu_snapshots[1].util_peak_pct, Some(20));
    }

    #[test]
    fn gpu_missing_on_first_sample_recovers_in_physical_index_order() {
        let backend = FakeGpuBackend::new([
            FakePoll::Ok(vec![gpu_snapshot_on(1, 90, 900)]),
            FakePoll::Ok(vec![
                gpu_snapshot_on(1, 20, 200),
                gpu_snapshot_on(0, 10, 100),
            ]),
        ]);
        let mut collector = collector_with_backend(backend);
        collector.sample_gpu();
        assert_eq!(collector.gpu_snapshots.len(), 1);
        assert_eq!(collector.gpu_snapshots[0].index, 1);
        assert_eq!(collector.gpu_snapshots[0].id, "gpu-1");
        collector.sample_gpu();
        assert_eq!(
            collector
                .gpu_snapshots
                .iter()
                .map(|gpu| gpu.index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(collector.gpu_snapshots[0].id, "gpu-0");
        assert_eq!(collector.gpu_snapshots[0].util_peak_pct, Some(10));
        assert_eq!(collector.gpu_snapshots[1].id, "gpu-1");
        assert_eq!(collector.gpu_snapshots[1].util_peak_pct, Some(90));
    }

    #[test]
    fn unknown_gpu_utilization_does_not_create_a_zero_peak() {
        let mut peaks = GpuPeakHistory::new(Duration::from_secs(3));
        let mut snap = gpu_snapshot(0, 100);
        snap.util_pct = None;
        peaks.apply(Instant::now(), &mut snap);
        assert_eq!(snap.util_peak_pct, None);
        assert_eq!(snap.vram_peak_used_bytes, Some(100));
    }

    #[test]
    fn stale_gpu_peaks_expire_without_repeating_the_old_sample() {
        let mut peaks = GpuPeakHistory::new(Duration::from_secs(1));
        let mut snap = gpu_snapshot(90, 100);
        let start = Instant::now();
        peaks.apply(start, &mut snap);
        peaks.refresh_peaks(start + Duration::from_secs(2), &mut snap);
        assert_eq!(snap.util_peak_pct, None);
        assert_eq!(snap.vram_peak_used_bytes, None);
    }

    #[test]
    fn snapshot_json_carries_schema_metadata() {
        let mut gpu = gpu_snapshot(10, 100);
        gpu.util_pct = None;
        gpu.temp_c = None;
        gpu.status = GpuSampleStatus::Stale;
        let snapshot = Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            sampled_at_unix_ms: 1_783_261_200_000,
            hostname: "host".into(),
            uptime_secs: 1,
            cpu_brand: "CPU".into(),
            cpu_physical_cores: Some(1),
            cpu_total_pct: 0.0,
            cpu_per_core_pct: vec![0.0],
            cpu_temp_c: None,
            ram_used_bytes: 0,
            ram_total_bytes: 1,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            gpus: vec![gpu],
            disks: vec![DiskSnapshot {
                name: "nvme0n1".into(),
                size_bytes: 2_000,
                filesystems: 3,
                fs_used_bytes: 400,
                fs_total_bytes: 1_800,
            }],
            procs: Vec::new(),
        };

        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["schema_version"], SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(json["sampled_at_unix_ms"], 1_783_261_200_000_u64);
        assert_eq!(json["cpu_physical_cores"], 1);
        assert_eq!(json["gpus"][0]["id"], "gpu-0");
        assert_eq!(json["gpus"][0]["index"], 0);
        assert_eq!(json["gpus"][0]["status"], "stale");
        assert_eq!(json["gpus"][0]["sampled_at_unix_ms"], 1);
        assert!(json["gpus"][0]["util_pct"].is_null());
        assert!(json["gpus"][0]["temp_c"].is_null());
        assert!(json["gpus"][0].get("per_pid").is_none());
        assert_eq!(
            json["disks"][0],
            serde_json::json!({
                "name": "nvme0n1",
                "size_bytes": 2_000,
                "filesystems": 3,
                "fs_used_bytes": 400,
                "fs_total_bytes": 1_800,
            })
        );
    }

    fn gpu_snapshot(util_pct: u32, vram_used_bytes: u64) -> GpuSnapshot {
        gpu_snapshot_on(0, util_pct, vram_used_bytes)
    }

    fn gpu_snapshot_on(index: u32, util_pct: u32, vram_used_bytes: u64) -> GpuSnapshot {
        GpuSnapshot {
            index,
            id: format!("gpu-{index}"),
            name: "GPU".into(),
            sampled_at_unix_ms: 1,
            status: GpuSampleStatus::Fresh,
            util_pct: Some(util_pct),
            util_peak_pct: None,
            temp_c: Some(40),
            vram_used_bytes,
            vram_peak_used_bytes: None,
            vram_total_bytes: 2_000,
            per_pid: HashMap::new(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_and_recurring_collection_exclude_live_thread_ids() {
        use clap::Parser;
        use std::sync::mpsc;

        // The fixture reports its Linux TID and remains live across both
        // collections. Dropping its sender releases it even if an assertion
        // fails, so the test does not leave sleeping fixture threads behind.
        struct ThreadFixture {
            finish: Option<mpsc::Sender<()>>,
            worker: Option<std::thread::JoinHandle<()>>,
        }
        impl Drop for ThreadFixture {
            fn drop(&mut self) {
                self.finish.take();
                self.worker.take().unwrap().join().unwrap();
            }
        }
        let (tid_sender, tid_receiver) = mpsc::channel();
        let mut fixtures = Vec::new();
        for _ in 0..3 {
            let sender = tid_sender.clone();
            let (finish, wait) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let status = std::fs::read_to_string("/proc/thread-self/status").unwrap();
                let tid: u32 = status
                    .lines()
                    .find_map(|line| line.strip_prefix("Pid:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                sender.send(tid).unwrap();
                let _ = wait.recv();
            });
            fixtures.push(ThreadFixture {
                finish: Some(finish),
                worker: Some(worker),
            });
        }
        let tids: Vec<_> = (0..3).map(|_| tid_receiver.recv().unwrap()).collect();
        assert!(tids.iter().all(|tid| *tid != std::process::id()));
        let cli = crate::cli::Cli::parse_from(["ztop", "--no-gpu", "--no-temp"]);
        let app = crate::app::AppState::from_cli(&cli);
        let mut collector = Collector::new(&cli).unwrap();
        for tid in &tids {
            assert!(
                !collector
                    .sys
                    .processes()
                    .contains_key(&sysinfo::Pid::from_u32(*tid)),
                "startup collected thread {tid} as a process"
            );
        }
        let snapshot = collector.collect(&app);
        assert!(snapshot
            .procs
            .iter()
            .any(|row| row.pid == std::process::id()));
        for tid in tids {
            assert!(
                !snapshot.procs.iter().any(|row| row.pid == tid),
                "recurring refresh collected thread {tid} as a process"
            );
        }
    }

    #[test]
    #[ignore = "manual collection timing; run with --ignored --nocapture --release"]
    fn benchmark_collection_refresh() {
        use clap::Parser;
        let cli = crate::cli::Cli::parse_from(["ztop", "--no-gpu", "--no-temp"]);
        let app = crate::app::AppState::from_cli(&cli);
        let mut collector = Collector::new(&cli).unwrap();
        collector.collect(&app);
        let samples = 30;
        let start = Instant::now();
        let mut rows = 0;
        for _ in 0..samples {
            rows = std::hint::black_box(collector.collect(&app)).procs.len();
        }
        eprintln!(
            "collection: {rows} processes, {:.3} ms/sample ({samples} samples)",
            start.elapsed().as_secs_f64() * 1_000.0 / f64::from(samples)
        );
    }
}
