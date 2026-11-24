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
}

/// Samples every physical disk, in device-name order.
#[cfg(target_os = "linux")]
pub use linux::Probe;

/// Only Linux is probed; elsewhere there are no disks to report.
#[cfg(not(target_os = "linux"))]
#[derive(Default)]
pub struct Probe;

#[cfg(not(target_os = "linux"))]
impl Probe {
    pub fn sample(&mut self) -> Vec<DiskSnapshot> {
        Vec::new()
    }
}
