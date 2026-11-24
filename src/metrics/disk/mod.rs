// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Physical disks and how full the filesystems mounted on them are.

use serde::Serialize;

#[cfg(target_os = "linux")]
mod linux;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiskSnapshot {
    /// Kernel name of the whole device, such as `nvme0n1` or `sda`.
    pub name: String,
    /// The drive's model as it names itself, such as `WD_BLACK SN850X
    /// 2000GB`. A SATA drive reports only the first 16 characters. `None`
    /// where the kernel has no name for it, as for most virtual disks.
    pub model: Option<String>,
    /// What sort of drive it is, such as `NVMe SSD`, `SATA HDD` or `USB
    /// disk`: the bus it hangs off, and whether it spins. `None` when the
    /// kernel says neither.
    pub kind: Option<String>,
    /// Raw capacity of the device.
    pub size_bytes: u64,
    /// Mounted filesystems measured on this disk. Zero means nothing on it is
    /// mounted, and then the two fields below are zero as well.
    pub filesystems: u32,
    /// The used space and capacity of those filesystems, as `df` reports
    /// them. Each disk of an MD RAID 1 mirror holds the whole filesystem and
    /// reports all of it; any other filesystem that spans several disks is
    /// split between them in proportion to the space it occupies on each.
    pub fs_used_bytes: u64,
    pub fs_total_bytes: u64,
    /// What an unprivileged user can still write to those filesystems: free
    /// space less the blocks reserved for root, `df`'s "Avail".
    pub fs_available_bytes: u64,
    /// Temperature of the drive in whole degrees Celsius, from the sensor the
    /// kernel exposes for it: every NVMe drive, and SATA or SAS drives while
    /// the `drivetemp` module is loaded. `None` for a disk without such a
    /// sensor and when temperature probing is turned off.
    pub temp_c: Option<i32>,
}

/// Samples every physical disk, in device-name order.
#[cfg(target_os = "linux")]
pub use linux::Probe;

/// Only Linux is probed; elsewhere there are no disks to report.
#[cfg(not(target_os = "linux"))]
pub struct Probe;

#[cfg(not(target_os = "linux"))]
impl Probe {
    pub fn new(_temperatures: bool) -> Self {
        Self
    }

    pub fn sample(&mut self) -> Vec<DiskSnapshot> {
        Vec::new()
    }
}
