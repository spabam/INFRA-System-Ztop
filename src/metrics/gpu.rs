// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::fmt::Arguments;
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::mem;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use nvml_wrapper::{enum_wrappers::device::TemperatureSensor, error::NvmlError, Nvml};
#[cfg(target_os = "linux")]
use nvml_wrapper_sys::bindings::{
    nvmlDevice_t, nvmlMemory_v2_t, nvmlReturn_enum_NVML_SUCCESS, NvmlLib,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuSampleStatus {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Clone, Serialize)]
pub struct GpuSnapshot {
    /// NVML device index captured at discovery; independent of panel position.
    pub index: u32,
    /// NVIDIA UUID, with a device-index fallback when UUID lookup fails.
    pub id: String,
    pub name: String,
    /// Time of the last successful device sample, including for stale data.
    pub sampled_at_unix_ms: u64,
    pub status: GpuSampleStatus,
    pub util_pct: Option<u32>,
    pub util_peak_pct: Option<u32>,
    pub temp_c: Option<u32>,
    /// Last observed memory values; consult status before treating as current.
    pub vram_used_bytes: u64,
    pub vram_peak_used_bytes: Option<u64>,
    pub vram_total_bytes: u64,
    /// PID -> (gpu_util_pct_or_none, vram_bytes) on this device
    #[serde(skip)]
    pub per_pid: HashMap<u32, PerPidGpu>,
}

#[derive(Default, Clone, Copy)]
pub struct PerPidGpu {
    pub util_pct: Option<u32>,
    pub vram_bytes: u64,
}

pub trait GpuBackend {
    fn poll(&mut self) -> Result<Vec<GpuSnapshot>>;
}

#[cfg(target_os = "linux")]
struct DeviceState {
    index: u32,
    id: String,
    name: String,
    per_proc_util_supported: bool,
    last_util_query_us: u64,
}

#[cfg(target_os = "linux")]
pub struct GpuProbe {
    nvml: Nvml,
    // Second, raw NVML session: nvml-wrapper does not expose the v2 memory
    // API (needed for the `reserved` field that aligns VRAM with nvidia-smi),
    // so those calls go through the raw bindings. Shut down in Drop.
    nvml_raw: Option<NvmlLib>,
    devices: Vec<DeviceState>,
    /// When false, per-process GPU/VRAM accounting APIs are never called.
    /// NVML may still open `/dev/nvidia-uvm` during initialization or device
    /// queries; avoiding NVML entirely requires `--no-gpu`.
    pid_gpu_enabled: bool,
    /// ZTOP_DEBUG_GPU sink. Writing to stderr would corrupt the alternate
    /// screen, so per-poll diagnostics append to a file instead.
    debug_log: Option<std::fs::File>,
}

#[cfg(not(target_os = "linux"))]
pub struct GpuProbe;

#[cfg(target_os = "linux")]
impl GpuProbe {
    pub fn init(pid_gpu_enabled: bool, verbose: bool) -> Result<Self> {
        // Default lib name in nvml-wrapper is `libnvidia-ml.so` (unversioned
        // symlink), which ships with the NVIDIA -dev package but not with a
        // runtime-only driver install. On Debian/Ubuntu/Fedora server boxes
        // the runtime ships only the versioned soname `libnvidia-ml.so.1`,
        // so fall back to that when the default open fails.
        let mut debug_log = open_debug_log();
        let (nvml, library_path, private_library) = init_nvml_with(
            |path| Nvml::builder().lib_path(path).init(),
            || {
                let executable = std::env::current_exe().ok()?;
                let version = std::fs::read_to_string("/sys/module/nvidia/version").ok()?;
                private_nvml_path(&executable, &version)
            },
        )?;
        if private_library {
            eprintln!(
                "ztop: using matching private NVML library {}; the host NVIDIA driver/library mismatch still requires repair",
                library_path.display()
            );
        }
        let count = nvml
            .device_count()
            .map_err(|e| anyhow!("NVML device_count: {e}"))?;
        if count == 0 {
            return Err(anyhow!("no NVIDIA devices"));
        }
        let nvml_raw = load_nvml_raw(library_path.as_os_str());

        // Probe each device's capabilities. Detailed success diagnostics are
        // opt-in; the default startup path should keep the user's shell clean.
        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let dev = match nvml.device_by_index(index) {
                Ok(dev) => dev,
                Err(error) => {
                    gpu_diag(
                        verbose,
                        &mut debug_log,
                        format_args!("device {index} unavailable at init ({error}); will retry"),
                    );
                    // Keep polling this index so a temporarily inaccessible
                    // GPU can recover without restarting the monitor. Its
                    // fallback ID remains stable for this collector session.
                    devices.push(DeviceState {
                        index,
                        id: format!("nvml-index-{index}"),
                        name: "NVIDIA GPU".into(),
                        per_proc_util_supported: pid_gpu_enabled,
                        last_util_query_us: 0,
                    });
                    continue;
                }
            };
            let name = dev.name().unwrap_or_else(|_| "?".into());
            gpu_diag(
                verbose,
                &mut debug_log,
                format_args!("NVML init OK; device {index} = \"{name}\" (of {count})"),
            );
            match dev.utilization_rates() {
                Ok(u) => gpu_diag(
                    verbose,
                    &mut debug_log,
                    format_args!(
                        "  utilization_rates at init: gpu={}% memory_io={}%",
                        u.gpu, u.memory
                    ),
                ),
                Err(e) => gpu_diag(
                    verbose,
                    &mut debug_log,
                    format_args!("  utilization_rates FAILED at init: {e}"),
                ),
            }
            match memory_info_active(nvml_raw.as_ref(), index).or_else(|| {
                dev.memory_info().ok().map(|m| ActiveMemoryInfo {
                    used: m.used,
                    total: m.total,
                    reserved: 0,
                })
            }) {
                Some(m) => gpu_diag(
                    verbose,
                    &mut debug_log,
                    format_args!(
                        "  memory_info: used={} MiB, reserved={} MiB, total={} MiB",
                        m.used / 1_048_576,
                        m.reserved / 1_048_576,
                        m.total / 1_048_576
                    ),
                ),
                None => gpu_diag(
                    verbose,
                    &mut debug_log,
                    format_args!("  memory_info FAILED"),
                ),
            }
            let per_proc_util_supported = if pid_gpu_enabled {
                let result = dev.process_utilization_stats(None);
                let supported = result.as_ref().err().is_none_or(retry_utilization_error);
                if let Err(error) = result {
                    gpu_diag(
                        verbose,
                        &mut debug_log,
                        format_args!(
                            "  per-process GPU utilization unavailable ({error}); retry={supported}"
                        ),
                    );
                }
                supported
            } else {
                false
            };
            devices.push(DeviceState {
                index,
                id: dev.uuid().unwrap_or_else(|_| format!("nvml-index-{index}")),
                name,
                per_proc_util_supported,
                last_util_query_us: 0,
            });
        }
        if !pid_gpu_enabled {
            gpu_diag(
                verbose,
                &mut debug_log,
                format_args!(
                    "  per-process GPU accounting disabled via --no-pid-gpu; device telemetry remains enabled"
                ),
            );
        }
        Ok(Self {
            nvml,
            nvml_raw,
            devices,
            pid_gpu_enabled,
            debug_log,
        })
    }

    /// Poll every device. Devices that fail individually are skipped for
    /// this sample; Err is returned only when no device produced data.
    fn poll_nvml(&mut self) -> Result<Vec<GpuSnapshot>> {
        let mut snaps = Vec::with_capacity(self.devices.len());
        for state in &mut self.devices {
            let Ok(dev) = self.nvml.device_by_index(state.index) else {
                continue;
            };
            if state.name == "NVIDIA GPU" {
                if let Ok(name) = dev.name() {
                    state.name = name;
                }
            }
            let device_util_pct = dev.utilization_rates().ok().map(|u| u.gpu);
            let Some(active_mem) =
                memory_info_active(self.nvml_raw.as_ref(), state.index).or_else(|| {
                    dev.memory_info().ok().map(|mem| ActiveMemoryInfo {
                        used: mem.used,
                        total: mem.total,
                        reserved: 0,
                    })
                })
            else {
                continue;
            };
            let temp = dev.temperature(TemperatureSensor::Gpu).ok();

            let mut per_pid: HashMap<u32, PerPidGpu> = HashMap::new();

            // Skip all per-process queries when accounting is disabled.
            // Device-level NVML calls may still open /dev/nvidia-uvm.
            if self.pid_gpu_enabled {
                // Per-process VRAM (compute + graphics)
                if let Ok(procs) = dev.running_compute_processes() {
                    for p in procs {
                        let entry = per_pid.entry(p.pid).or_default();
                        if let nvml_wrapper::enums::device::UsedGpuMemory::Used(bytes) =
                            p.used_gpu_memory
                        {
                            entry.vram_bytes = entry.vram_bytes.max(bytes);
                        }
                    }
                }
                if let Ok(procs) = dev.running_graphics_processes() {
                    for p in procs {
                        let entry = per_pid.entry(p.pid).or_default();
                        if let nvml_wrapper::enums::device::UsedGpuMemory::Used(bytes) =
                            p.used_gpu_memory
                        {
                            entry.vram_bytes = entry.vram_bytes.max(bytes);
                        }
                    }
                }

                // Per-process GPU utilization (only if supported)
                if state.per_proc_util_supported {
                    match dev.process_utilization_stats(Some(state.last_util_query_us)) {
                        Ok(stats) => {
                            for s in &stats {
                                let entry = per_pid.entry(s.pid).or_default();
                                entry.util_pct = Some(s.sm_util);
                            }
                            if let Some(last) = stats.iter().map(|s| s.timestamp).max() {
                                state.last_util_query_us = last;
                            }
                        }
                        Err(error) => {
                            state.per_proc_util_supported = retry_utilization_error(&error);
                        }
                    }
                }
            }

            // A successful device reading is authoritative, including zero.
            // Only estimate from per-process data when the device API failed.
            let util_pct = select_utilization(device_util_pct, &per_pid);

            if let Some(log) = self.debug_log.as_mut() {
                writeln!(
                    log,
                    "ztop: gpu{} poll: util={:?} used={} MiB reserved={} MiB total={} MiB",
                    state.index,
                    util_pct,
                    active_mem.used / 1_048_576,
                    active_mem.reserved / 1_048_576,
                    active_mem.total / 1_048_576
                )
                .ok();
            }

            snaps.push(GpuSnapshot {
                index: state.index,
                id: state.id.clone(),
                name: state.name.clone(),
                sampled_at_unix_ms: super::sampled_at_unix_ms(),
                status: GpuSampleStatus::Fresh,
                util_pct,
                util_peak_pct: None,
                temp_c: temp,
                vram_used_bytes: active_mem.used,
                vram_peak_used_bytes: None,
                vram_total_bytes: active_mem.total,
                per_pid,
            });
        }

        if snaps.is_empty() {
            return Err(anyhow!("no NVIDIA device produced a sample"));
        }
        Ok(snaps)
    }
}

/// A private runtime can bridge a driver upgrade until a planned reboot. It is
/// considered only for an actual version mismatch, and a healthy system NVML
/// always wins. The returned path also selects the raw memory-query session.
#[cfg(target_os = "linux")]
fn init_nvml_with<T>(
    mut init: impl FnMut(&OsStr) -> std::result::Result<T, NvmlError>,
    private_path: impl FnOnce() -> Option<PathBuf>,
) -> Result<(T, PathBuf, bool)> {
    let mut last_error = NvmlError::LibraryNotFound;
    let mut mismatch = false;
    for name in ["libnvidia-ml.so", "libnvidia-ml.so.1"] {
        match init(OsStr::new(name)) {
            Ok(nvml) => return Ok((nvml, PathBuf::from(name), false)),
            Err(error) => {
                mismatch |= matches!(error, NvmlError::LibRmVersionMismatch);
                last_error = error;
            }
        }
    }
    if mismatch {
        if let Some(path) = private_path() {
            return init(path.as_os_str())
                .map(|nvml| (nvml, path.clone(), true))
                .map_err(|error| {
                    anyhow!(
                        "NVML init: driver/library version mismatch; matching private library {} failed ({error}); align the host NVIDIA userspace libraries and loaded kernel driver",
                        path.display()
                    )
                });
        }
        return Err(anyhow!(
            "NVML init: driver/library version mismatch; align the host NVIDIA userspace libraries and loaded kernel driver (a planned reboot after a driver upgrade may be required)"
        ));
    }
    Err(anyhow!("NVML init: {last_error}"))
}

#[cfg(target_os = "linux")]
fn private_nvml_path(executable: &Path, version: &str) -> Option<PathBuf> {
    let version = version.trim();
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    Some(
        bin.parent()?
            .join("lib/ztop/nvml")
            .join(version)
            .join("libnvidia-ml.so.1"),
    )
}

#[cfg(target_os = "linux")]
fn retry_utilization_error(error: &NvmlError) -> bool {
    !matches!(
        error,
        NvmlError::NotSupported | NvmlError::FunctionNotFound | NvmlError::FailedToLoadSymbol(_)
    )
}

#[cfg(any(target_os = "linux", test))]
fn select_utilization(
    device_util_pct: Option<u32>,
    per_pid: &HashMap<u32, PerPidGpu>,
) -> Option<u32> {
    device_util_pct.or_else(|| {
        per_pid
            .values()
            .filter_map(|entry| entry.util_pct)
            .reduce(u32::saturating_add)
            .map(|sum| sum.min(100))
    })
}

#[cfg(target_os = "linux")]
impl GpuBackend for GpuProbe {
    fn poll(&mut self) -> Result<Vec<GpuSnapshot>> {
        self.poll_nvml()
    }
}

#[cfg(target_os = "linux")]
impl Drop for GpuProbe {
    fn drop(&mut self) {
        // The wrapper's Nvml handle shuts itself down; the raw session opened
        // for the v2 memory API needs an explicit shutdown.
        if let Some(raw) = &self.nvml_raw {
            if raw.nvmlShutdown.is_ok() {
                unsafe {
                    raw.nvmlShutdown();
                }
            }
        }
    }
}

/// ZTOP_DEBUG_GPU: when set, per-poll GPU diagnostics are appended to a file
/// (stderr would corrupt the TUI's alternate screen). The variable's value is
/// the target path; empty or "1" falls back to ztop-gpu-debug.log in the
/// system temp directory.
#[cfg(target_os = "linux")]
fn open_debug_log() -> Option<std::fs::File> {
    let value = std::env::var_os("ZTOP_DEBUG_GPU")?;
    let path = if value.is_empty() || value == "1" {
        std::env::temp_dir().join("ztop-gpu-debug.log")
    } else {
        std::path::PathBuf::from(value)
    };
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => {
            eprintln!("ztop: GPU debug log -> {}", path.display());
            Some(f)
        }
        Err(e) => {
            eprintln!("ztop: cannot open GPU debug log {}: {e}", path.display());
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn gpu_diag(verbose: bool, debug_log: &mut Option<std::fs::File>, args: Arguments<'_>) {
    if let Some(log) = debug_log.as_mut() {
        writeln!(log, "ztop: {args}").ok();
    } else if verbose {
        eprintln!("ztop: {args}");
    }
}

#[cfg(target_os = "linux")]
struct ActiveMemoryInfo {
    used: u64,
    total: u64,
    reserved: u64,
}

#[cfg(target_os = "linux")]
fn load_nvml_raw(library_path: &OsStr) -> Option<NvmlLib> {
    unsafe {
        let nvml_raw = NvmlLib::new(library_path).ok()?;

        if nvml_raw.nvmlInit_v2.is_err() || nvml_raw.nvmlInit_v2() != nvmlReturn_enum_NVML_SUCCESS {
            return None;
        }

        Some(nvml_raw)
    }
}

#[cfg(target_os = "linux")]
fn memory_info_active(nvml_raw: Option<&NvmlLib>, index: u32) -> Option<ActiveMemoryInfo> {
    let nvml_raw = nvml_raw?;
    if nvml_raw.nvmlDeviceGetHandleByIndex_v2.is_err()
        || nvml_raw.nvmlDeviceGetMemoryInfo_v2.is_err()
    {
        return None;
    }

    unsafe {
        let mut device: nvmlDevice_t = mem::zeroed();
        if nvml_raw.nvmlDeviceGetHandleByIndex_v2(index, &mut device)
            != nvmlReturn_enum_NVML_SUCCESS
        {
            return None;
        }

        let mut mem_info: nvmlMemory_v2_t = mem::zeroed();
        mem_info.version = nvml_struct_version::<nvmlMemory_v2_t>(2);

        if nvml_raw.nvmlDeviceGetMemoryInfo_v2(device, &mut mem_info)
            != nvmlReturn_enum_NVML_SUCCESS
        {
            return None;
        }

        Some(ActiveMemoryInfo {
            used: mem_info.used,
            total: mem_info.total,
            reserved: mem_info.reserved,
        })
    }
}

#[cfg(target_os = "linux")]
fn nvml_struct_version<T>(version: u32) -> u32 {
    mem::size_of::<T>() as u32 | (version << 24)
}

#[cfg(not(target_os = "linux"))]
impl GpuProbe {
    pub fn init(_pid_gpu_enabled: bool, _verbose: bool) -> Result<Self> {
        Err(anyhow!("NVML supported on Linux only"))
    }
}

#[cfg(not(target_os = "linux"))]
impl GpuBackend for GpuProbe {
    fn poll(&mut self) -> Result<Vec<GpuSnapshot>> {
        Err(anyhow!("NVML supported on Linux only"))
    }
}

#[cfg(test)]
mod tests {
    use super::{select_utilization, PerPidGpu};
    use std::collections::HashMap;

    #[cfg(target_os = "linux")]
    #[test]
    fn private_library_path_is_scoped_to_install_prefix_and_numeric_driver_version() {
        use std::path::{Path, PathBuf};

        assert_eq!(
            super::private_nvml_path(Path::new("/opt/my ztop/bin/ztop"), "615.71.09\n"),
            Some(PathBuf::from(
                "/opt/my ztop/lib/ztop/nvml/615.71.09/libnvidia-ml.so.1"
            ))
        );
        for version in [
            "",
            "../615.71.09",
            "615/71/09",
            "615.71",
            "615..09",
            "615.71.09-x",
        ] {
            assert!(super::private_nvml_path(Path::new("/usr/bin/ztop"), version).is_none());
        }
        assert!(
            super::private_nvml_path(Path::new("/tmp/target/release/ztop"), "615.71.09").is_none()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn healthy_system_library_wins_without_considering_private_runtime() {
        use nvml_wrapper::error::NvmlError;
        use std::ffi::OsStr;

        let mut attempted = Vec::new();
        let (value, selected, private) = super::init_nvml_with(
            |path| {
                attempted.push(path.to_owned());
                if path == OsStr::new("libnvidia-ml.so") {
                    Err(NvmlError::LibraryNotFound)
                } else {
                    Ok(42)
                }
            },
            || panic!("a healthy runtime must not consult private libraries"),
        )
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(selected.as_os_str(), OsStr::new("libnvidia-ml.so.1"));
        assert!(!private);
        assert_eq!(attempted.len(), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mismatch_recovers_with_exact_private_library_path() {
        use nvml_wrapper::error::NvmlError;
        use std::path::PathBuf;

        let matching = PathBuf::from("/usr/lib/ztop/nvml/615.71.09/libnvidia-ml.so.1");
        let mut attempted = Vec::new();
        let (value, selected, private) = super::init_nvml_with(
            |path| {
                attempted.push(path.to_owned());
                if path == matching.as_os_str() {
                    Ok(42)
                } else {
                    Err(NvmlError::LibRmVersionMismatch)
                }
            },
            || Some(matching.clone()),
        )
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(selected, matching);
        assert!(private);
        assert_eq!(attempted.len(), 3);
        assert_eq!(attempted[2], selected.as_os_str());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn permissions_and_missing_driver_never_consult_private_libraries() {
        use nvml_wrapper::error::NvmlError;

        for permission_error in [true, false] {
            let mut attempted = 0;
            let result = super::init_nvml_with::<()>(
                |_| {
                    attempted += 1;
                    Err(if permission_error {
                        NvmlError::NoPermission
                    } else {
                        NvmlError::DriverNotLoaded
                    })
                },
                || panic!("only driver/library mismatches may use a private runtime"),
            );
            assert!(result.is_err());
            assert_eq!(attempted, 2);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn absent_or_broken_private_runtime_preserves_mismatch_diagnosis() {
        use nvml_wrapper::error::NvmlError;
        use std::path::PathBuf;

        let error = super::init_nvml_with::<()>(|_| Err(NvmlError::LibRmVersionMismatch), || None)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("driver/library version mismatch"));
        let error = super::init_nvml_with::<()>(
            |path| {
                if path.to_string_lossy().starts_with('/') {
                    Err(NvmlError::LibraryNotFound)
                } else {
                    Err(NvmlError::LibRmVersionMismatch)
                }
            },
            || {
                Some(PathBuf::from(
                    "/usr/lib/ztop/nvml/615.71.09/libnvidia-ml.so.1",
                ))
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("driver/library version mismatch"));
        assert!(message.contains("/usr/lib/ztop/nvml/615.71.09/libnvidia-ml.so.1"));
    }

    #[test]
    fn successful_device_utilization_is_authoritative_including_zero() {
        let per_pid = HashMap::from([(
            42,
            PerPidGpu {
                util_pct: Some(90),
                vram_bytes: 0,
            },
        )]);
        assert_eq!(select_utilization(Some(5), &per_pid), Some(5));
        assert_eq!(select_utilization(Some(0), &per_pid), Some(0));
        assert_eq!(select_utilization(None, &per_pid), Some(90));
        assert_eq!(select_utilization(None, &HashMap::new()), None);
    }

    #[test]
    fn process_fallback_is_bounded_and_distinguishes_missing_from_zero() {
        let mut per_pid = HashMap::from([(
            42,
            PerPidGpu {
                util_pct: None,
                vram_bytes: 1,
            },
        )]);
        assert_eq!(select_utilization(None, &per_pid), None);
        per_pid.get_mut(&42).unwrap().util_pct = Some(0);
        assert_eq!(select_utilization(None, &per_pid), Some(0));
        per_pid.get_mut(&42).unwrap().util_pct = Some(90);
        per_pid.insert(
            43,
            PerPidGpu {
                util_pct: Some(80),
                vram_bytes: 1,
            },
        );
        assert_eq!(select_utilization(None, &per_pid), Some(100));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_samples_and_transient_errors_keep_utilization_polling_enabled() {
        use nvml_wrapper::error::NvmlError;
        assert!(super::retry_utilization_error(&NvmlError::NotFound));
        assert!(super::retry_utilization_error(&NvmlError::GpuLost));
        assert!(super::retry_utilization_error(&NvmlError::Unknown));
        assert!(!super::retry_utilization_error(&NvmlError::NotSupported));
        assert!(!super::retry_utilization_error(
            &NvmlError::FunctionNotFound
        ));
    }
}
