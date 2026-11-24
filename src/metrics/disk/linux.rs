// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `/sys/block` names the disks and `/proc/self/mountinfo` the mounted
//! filesystems. sysfs then leads from each filesystem's block device down to
//! the disks underneath it: through partitions, and through the `slaves` of
//! device-mapper (LVM, LUKS) and MD RAID devices. Each disk of an MD RAID 1
//! mirror holds the whole filesystem; any other filesystem on several disks
//! is divided between them. A drive's model, its kind and its temperature
//! (the hwmon sensor) are what sysfs lists under the drive's own hardware.

use std::{
    collections::BTreeMap,
    ffi::{CString, OsString},
    fs,
    mem::MaybeUninit,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::DiskSnapshot;

/// sysfs counts every `size` in 512-byte sectors, whatever the device uses.
const SECTOR_BYTES: u64 = 512;
/// Real stacks are a few levels deep (a filesystem on LVM on LUKS on RAID on
/// a partition). Stop well past that instead of trusting sysfs to be a tree.
const MAX_STACK_DEPTH: u8 = 8;
/// How long a scan of the mount table and sysfs is trusted. Disks and mounts
/// change rarely, and reading the mount table is the costly part: it grows
/// with every container on the host. Usage is measured on every sample.
const TOPOLOGY_TTL: Duration = Duration::from_secs(5);
/// Degrees Celsius a drive can really be at. A drive that measures nothing
/// reports zero kelvin, and a failed query can leave any number behind.
const PLAUSIBLE_CELSIUS: std::ops::RangeInclusive<i64> = -50..=150;

/// `major:minor`.
type DeviceNumber = (u32, u32);

struct Mount {
    /// Device number of the filesystem. Major 0 marks one the kernel numbers
    /// itself rather than after its block device: btrfs, and everything that
    /// has no block device at all (overlay, tmpfs, network filesystems).
    device: DeviceNumber,
    mount_point: PathBuf,
    fs_type: String,
    source: PathBuf,
}

struct Usage {
    total_bytes: u64,
    used_bytes: u64,
    available_bytes: u64,
}

/// Samples the disks, remembering between samples where each filesystem lives.
pub struct Probe {
    topology: Topology,
    scanned_at: Option<Instant>,
    temperatures: bool,
}

impl Probe {
    /// `temperatures` is whether the drives are asked for their temperature.
    pub fn new(temperatures: bool) -> Self {
        Self {
            topology: Topology::default(),
            scanned_at: None,
            temperatures,
        }
    }

    pub fn sample(&mut self) -> Vec<DiskSnapshot> {
        if self
            .scanned_at
            .is_none_or(|at| at.elapsed() >= TOPOLOGY_TTL)
        {
            let sys = Path::new("/sys");
            let mountinfo = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
            self.topology = Topology::scan(sys, &mountinfo, &device_number);
            // A reading makes the kernel query the drive, and a drive warms
            // and cools slowly: take one per scan, not one per sample.
            if self.temperatures {
                self.topology.read_temperatures(sys);
            }
            self.scanned_at = Some(Instant::now());
        }
        self.topology.measure(&usage)
    }
}

/// The physical disks and the mounted filesystems that rest on them.
#[derive(Default)]
struct Topology {
    /// In display order, with nothing measured yet.
    disks: Vec<DiskSnapshot>,
    filesystems: Vec<Filesystem>,
}

struct Filesystem {
    /// Every place it is mounted.
    mounts: Vec<Mount>,
    /// `(index into disks, fraction of the filesystem held there)`; never
    /// empty. The fractions add up to 1 unless a mirror repeats the data: each
    /// disk of a mirror holds all of it.
    shares: Vec<(usize, f64)>,
}

impl Topology {
    fn scan(
        sys: &Path,
        mountinfo: &str,
        device_number: &dyn Fn(&Path) -> Option<DeviceNumber>,
    ) -> Self {
        let disks = physical_disks(sys);
        if disks.is_empty() {
            return Self::default();
        }

        // One entry per filesystem, however often it is mounted, keyed by the
        // block devices it sits on.
        let mut mounted: BTreeMap<Vec<String>, Vec<Mount>> = BTreeMap::new();
        for mount in mountinfo.lines().filter_map(parse_mount) {
            let devices = backing_devices(sys, &mount, device_number);
            if !devices.is_empty() {
                mounted.entry(devices).or_default().push(mount);
            }
        }

        let filesystems = mounted
            .into_iter()
            .filter_map(|(devices, mounts)| {
                let mut reached = Vec::new();
                for (device, fraction) in divide(sys, &devices, 1.0) {
                    disk_shares(sys, &device, MAX_STACK_DEPTH, fraction, &mut reached);
                }
                let mut shares: Vec<(usize, f64)> = Vec::new();
                for (name, fraction) in reached {
                    // Loop devices, zram and the like end on no physical disk.
                    let Some(disk) = disks.iter().position(|disk| disk.name == name) else {
                        continue;
                    };
                    match shares.iter_mut().find(|(index, _)| *index == disk) {
                        // A disk cannot hold more than the whole filesystem,
                        // even with both halves of a mirror on it.
                        Some(share) => share.1 = (share.1 + fraction).min(1.0),
                        None => shares.push((disk, fraction)),
                    }
                }
                (!shares.is_empty()).then_some(Filesystem { mounts, shares })
            })
            .collect();
        Self { disks, filesystems }
    }

    /// Asks every disk for its temperature. Measuring usage afterwards
    /// repeats these readings until the next scan.
    fn read_temperatures(&mut self, sys: &Path) {
        let block = sys.join("block");
        for disk in &mut self.disks {
            disk.temp_c = temperature(&block.join(&disk.name));
        }
    }

    fn measure(&self, usage: &dyn Fn(&Mount) -> Option<Usage>) -> Vec<DiskSnapshot> {
        let mut disks = self.disks.clone();
        for filesystem in &self.filesystems {
            // Some mount points may be unreadable or covered by another
            // mount; any one that answers describes the whole filesystem.
            let Some(usage) = filesystem.mounts.iter().find_map(usage) else {
                continue;
            };
            for &(disk, fraction) in &filesystem.shares {
                let share = |whole: u64| {
                    if fraction >= 1.0 {
                        whole
                    } else {
                        (whole as f64 * fraction) as u64
                    }
                };
                let disk = &mut disks[disk];
                disk.filesystems += 1;
                disk.fs_total_bytes = disk.fs_total_bytes.saturating_add(share(usage.total_bytes));
                disk.fs_used_bytes = disk.fs_used_bytes.saturating_add(share(usage.used_bytes));
                disk.fs_available_bytes = disk
                    .fs_available_bytes
                    .saturating_add(share(usage.available_bytes));
            }
        }
        disks
    }
}

/// Whole devices that belong to hardware and hold media, in display order.
fn physical_disks(sys: &Path) -> Vec<DiskSnapshot> {
    let block = sys.join("block");
    let mut disks: Vec<_> = entry_names(&block)
        .into_iter()
        .filter_map(|name| {
            let node = block.join(&name);
            // Hardware-backed devices link to the device they belong to; loop,
            // zram, device-mapper and MD devices have no such link.
            let hardware = node.join("device").exists()
                // Optical drives, and the boot and RPMB areas of an eMMC.
                && !name.starts_with("sr")
                && !(name.starts_with("mmcblk")
                    && (name.contains("boot") || name.ends_with("rpmb")))
                // The per-controller paths of a multipath NVMe namespace.
                && fs::read_to_string(node.join("hidden")).map_or(true, |v| v.trim() != "1");
            if !hardware {
                return None;
            }
            // A card reader or drive without media reports no sectors.
            let size_bytes = sectors(&node) * SECTOR_BYTES;
            (size_bytes > 0).then(|| DiskSnapshot {
                model: model(&node),
                kind: kind(sys, &name),
                name,
                size_bytes,
                filesystems: 0,
                fs_used_bytes: 0,
                fs_total_bytes: 0,
                fs_available_bytes: 0,
                temp_c: None,
            })
        })
        .collect();
    disks.sort_by_cached_key(|disk| natural_key(&disk.name));
    disks
}

/// The model the drive at `node` (`/sys/block/<name>`) reports. An NVMe
/// controller gives the whole name. A SCSI device gives 16 characters and a
/// vendor, which is the placeholder `ATA` for every SATA drive and is then
/// left out. A memory card has a short product name.
fn model(node: &Path) -> Option<String> {
    let text = |file: &str| {
        let text = fs::read_to_string(node.join("device").join(file)).ok()?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    };
    let model = text("model").or_else(|| text("name"))?;
    match text("vendor") {
        Some(vendor)
            if vendor != "ATA" && !model.to_lowercase().starts_with(&vendor.to_lowercase()) =>
        {
            Some(format!("{vendor} {model}"))
        }
        _ => Some(model),
    }
}

/// What sort of drive `name` is: the bus it hangs off, read from its place in
/// the sysfs device tree, and whether the kernel counts it as rotating.
fn kind(sys: &Path, name: &str) -> Option<String> {
    let node = sys.join("block").join(name);
    if name.starts_with("nvme") {
        return Some("NVMe SSD".to_owned());
    }
    if name.starts_with("mmcblk") {
        let card = fs::read_to_string(node.join("device/type")).unwrap_or_default();
        return Some(
            match card.trim() {
                "SD" => "SD card",
                "MMC" => "eMMC",
                _ => "memory card",
            }
            .to_owned(),
        );
    }
    // .../ata3/host2/target2:0:0/2:0:0:0/block/sda: each directory between
    // the root of sysfs and the disk is a device it is reached through.
    let path = fs::canonicalize(&node).ok()?;
    let root = fs::canonicalize(sys).ok()?;
    let through: Vec<&str> = path
        .strip_prefix(&root)
        .ok()?
        .iter()
        .filter_map(|part| part.to_str())
        .collect();
    let via = |bus: &str| through.iter().any(|part| numbered(part, bus));
    let starts = |prefix: &str| through.iter().any(|part| part.starts_with(prefix));
    if via("usb") {
        // A bridge hides what is behind it, and a stick is no disk that spins.
        return Some("USB disk".to_owned());
    }
    if via("virtio") || starts("vbd-") {
        return Some("virtual disk".to_owned());
    }
    let bus = if via("ata") {
        "SATA"
    } else if starts("end_device-") {
        "SAS"
    } else if name.starts_with("sd") {
        "SCSI"
    } else {
        return None;
    };
    let medium = match fs::read_to_string(node.join("queue/rotational"))
        .unwrap_or_default()
        .trim()
    {
        "0" => "SSD",
        "1" => "HDD",
        _ => "disk",
    };
    Some(format!("{bus} {medium}"))
}

/// Temperature of the disk at `node` (`/sys/block/<name>`) in whole degrees
/// Celsius, from the hwmon device the kernel registers for the drive's own
/// hardware. An NVMe controller always has one. A SATA or SAS disk has one
/// while the `drivetemp` module is loaded. USB bridges, memory cards and
/// virtual disks have none. `temp1_input` is the drive's main sensor in both
/// drivers; for NVMe that is the Composite temperature SMART reports.
fn temperature(node: &Path) -> Option<i32> {
    let device = node.join("device");
    // A namespace that several controllers lead to hangs off its NVMe
    // subsystem, which links to each of them. Every other disk hangs off the
    // hardware itself.
    let mut hardware = vec![device.clone()];
    hardware.extend(
        entry_names(&device)
            .into_iter()
            .filter(|name| numbered(name, "nvme"))
            .map(|controller| device.join(controller)),
    );
    hardware
        .iter()
        // An NVMe controller holds its hwmon device directly, a SCSI device
        // in a `hwmon` directory.
        .flat_map(|parent| [parent.clone(), parent.join("hwmon")])
        .flat_map(|holder| {
            entry_names(&holder)
                .into_iter()
                .filter(|name| numbered(name, "hwmon"))
                .map(move |name| holder.join(name))
        })
        .filter_map(|hwmon| {
            let millidegrees: i64 = fs::read_to_string(hwmon.join("temp1_input"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            let degrees = (millidegrees + 500).div_euclid(1000);
            PLAUSIBLE_CELSIUS
                .contains(&degrees)
                .then_some(degrees as i32)
        })
        // The controllers of one drive agree; should they not, the hotter
        // reading is the one to act on.
        .max()
}

/// `prefix` followed by digits and nothing else: `hwmon3`, `nvme0`.
fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Kernel names of the block devices a mounted filesystem sits on. Empty for
/// anything that is not a filesystem on local block devices.
fn backing_devices(
    sys: &Path,
    mount: &Mount,
    device_number: &dyn Fn(&Path) -> Option<DeviceNumber>,
) -> Vec<String> {
    let mut names = if mount.device.0 != 0 {
        device_name(sys, mount.device).into_iter().collect()
    } else if mount.fs_type == "btrfs" {
        // The mount source is one member device; sysfs lists them all.
        device_number(&mount.source)
            .and_then(|number| device_name(sys, number))
            .map(|member| btrfs_members(sys, member))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    names.sort();
    names.dedup();
    names
}

/// Every device of the btrfs filesystem that `member` belongs to, from
/// `/sys/fs/btrfs/<fsid>/devices/`; just `member` when sysfs does not say.
fn btrfs_members(sys: &Path, member: String) -> Vec<String> {
    let btrfs = sys.join("fs/btrfs");
    entry_names(&btrfs)
        .into_iter()
        .map(|fsid| btrfs.join(fsid).join("devices"))
        .find(|devices| devices.join(&member).exists())
        .map(|devices| entry_names(&devices))
        .unwrap_or_else(|| vec![member])
}

/// Appends `(disk, fraction)` for each place `device` comes to rest on a whole
/// device with nothing underneath it, where `fraction` is how much of the
/// filesystem that place holds. Every member of an MD RAID 1 mirror holds all
/// of what the array holds; the members of anything else (LVM, a stripe, a
/// parity array) divide it between them.
fn disk_shares(
    sys: &Path,
    device: &str,
    depth: u8,
    fraction: f64,
    shares: &mut Vec<(String, f64)>,
) {
    let class = sys.join("class/block");
    let node = class.join(device);
    let whole = if node.join("partition").exists() {
        // .../block/nvme0n1/nvme0n1p3: a partition lives inside its disk.
        fs::read_link(&node).ok().and_then(|target| {
            let disk = target.parent()?.file_name()?;
            Some(disk.to_str()?.to_owned())
        })
    } else {
        Some(device.to_owned())
    };
    let Some(whole) = whole else {
        return;
    };
    let slaves = if depth == 0 {
        Vec::new()
    } else {
        entry_names(&class.join(&whole).join("slaves"))
    };
    if slaves.is_empty() {
        shares.push((whole, fraction));
        return;
    }
    let mirror = fs::read_to_string(class.join(&whole).join("md/level"))
        .is_ok_and(|level| level.trim() == "raid1");
    let members = if mirror {
        slaves.into_iter().map(|slave| (slave, fraction)).collect()
    } else {
        divide(sys, &slaves, fraction)
    };
    for (slave, fraction) in members {
        disk_shares(sys, &slave, depth - 1, fraction, shares);
    }
}

/// Divides `fraction` between `devices` in proportion to their sizes, or
/// evenly when sysfs gives none of them a size.
fn divide(sys: &Path, devices: &[String], fraction: f64) -> Vec<(String, f64)> {
    let class = sys.join("class/block");
    let sizes: Vec<u64> = devices
        .iter()
        .map(|device| sectors(&class.join(device)))
        .collect();
    let total: u64 = sizes.iter().sum();
    devices
        .iter()
        .zip(sizes)
        .map(|(device, size)| {
            let part = if total == 0 {
                1.0 / devices.len() as f64
            } else {
                size as f64 / total as f64
            };
            (device.clone(), fraction * part)
        })
        .collect()
}

fn device_name(sys: &Path, (major, minor): DeviceNumber) -> Option<String> {
    let target = fs::read_link(sys.join(format!("dev/block/{major}:{minor}"))).ok()?;
    Some(target.file_name()?.to_str()?.to_owned())
}

fn sectors(node: &Path) -> u64 {
    fs::read_to_string(node.join("size"))
        .ok()
        .and_then(|size| size.trim().parse().ok())
        .unwrap_or(0)
}

/// Names in a directory, sorted so results do not depend on readdir order.
fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// One `/proc/self/mountinfo` line:
/// `36 35 98:0 /root /mount/point rw,noatime shared:1 - ext4 /dev/sda1 rw`.
/// A lone `-` ends the variable number of optional fields.
fn parse_mount(line: &str) -> Option<Mount> {
    let (mount, filesystem) = line.split_once(" - ")?;
    let mut mount = mount.split(' ');
    let (major, minor) = mount.nth(2)?.split_once(':')?;
    let mount_point = unescape(mount.nth(1)?);
    let mut filesystem = filesystem.split(' ');
    Some(Mount {
        device: (major.parse().ok()?, minor.parse().ok()?),
        mount_point,
        fs_type: filesystem.next()?.to_owned(),
        source: unescape(filesystem.next()?),
    })
}

/// mountinfo writes space, tab, newline and backslash as `\ooo` in octal.
fn unescape(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut path = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'\\')
            .then(|| bytes.get(at + 1..at + 4))
            .flatten()
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        if let Some(byte) = escaped {
            path.push(byte);
            at += 4;
        } else {
            path.push(bytes[at]);
            at += 1;
        }
    }
    PathBuf::from(OsString::from_vec(path))
}

/// `nvme2n1` sorts before `nvme10n1`: runs of digits compare as numbers.
fn natural_key(name: &str) -> Vec<(String, u64)> {
    let mut key = Vec::new();
    let mut rest = name;
    while !rest.is_empty() {
        let text = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        let (text, tail) = rest.split_at(text);
        let digits = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        let (digits, tail) = tail.split_at(digits);
        key.push((text.to_owned(), digits.parse().unwrap_or(0)));
        rest = tail;
    }
    key
}

fn device_number(path: &Path) -> Option<DeviceNumber> {
    let meta = fs::metadata(path).ok()?;
    meta.file_type()
        .is_block_device()
        .then(|| (libc::major(meta.rdev()), libc::minor(meta.rdev())))
}

/// Capacity, used and available space of the filesystem mounted at `mount`,
/// as `df` counts them: used is every block that is not free, and available
/// is what an unprivileged user can still write, which leaves out the blocks
/// reserved for root.
fn usage(mount: &Mount) -> Option<Usage> {
    // Another mount may cover this mount point. A filesystem named after its
    // block device reports that number for its files too, so check that the
    // path still leads to it. (btrfs gives each subvolume a number of its
    // own, which never matches the mount table.)
    if mount.device.0 != 0 {
        let seen = fs::metadata(&mount.mount_point).ok()?.dev();
        if (libc::major(seen), libc::minor(seen)) != mount.device {
            return None;
        }
    }
    let path = CString::new(mount.mount_point.as_os_str().as_bytes()).ok()?;
    let mut stat = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a NUL-terminated string and `stat` is a writable
    // buffer of the size statfs(2) fills in. It is read only after the call
    // reports success, which is when the kernel has initialized all of it.
    let stat = unsafe {
        if libc::statfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    let block = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    } as u64;
    // A filesystem driver in user space can report anything; do not let its
    // numbers overflow.
    let blocks = stat.f_blocks as u64;
    Some(Usage {
        total_bytes: blocks.saturating_mul(block),
        used_bytes: blocks
            .saturating_sub(stat.f_bfree as u64)
            .saturating_mul(block),
        available_bytes: (stat.f_bavail as u64).saturating_mul(block),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, os::unix::fs::symlink};

    const GIB: u64 = 1 << 30;

    /// A sysfs tree under a unique temporary root, removed on drop.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(test: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("ztop-disk-{}-{test}", std::process::id()));
            fs::remove_dir_all(&root).ok();
            for dir in ["block", "class/block", "dev/block", "devices", "fs/btrfs"] {
                fs::create_dir_all(root.join(dir)).unwrap();
            }
            Fixture(root)
        }

        fn sys(&self) -> &Path {
            &self.0
        }

        /// A block device directory with its sysfs links. `path` is relative
        /// to `devices/`: `sda` for a whole device, `sda/sda1` for a partition.
        fn node(&self, path: &str, number: &str, sectors: u64) -> PathBuf {
            let dir = self.0.join("devices").join(path);
            let name = dir.file_name().unwrap().to_owned();
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("size"), format!("{sectors}\n")).unwrap();
            symlink(&dir, self.0.join("class/block").join(&name)).unwrap();
            symlink(&dir, self.0.join("dev/block").join(number)).unwrap();
            dir
        }

        /// A whole device. Hardware has a `device` link; virtual devices
        /// (loop, device-mapper, MD) do not.
        fn disk(&self, name: &str, number: &str, sectors: u64, hardware: bool) -> PathBuf {
            let dir = self.node(name, number, sectors);
            fs::create_dir_all(dir.join("slaves")).unwrap();
            if hardware {
                fs::create_dir_all(dir.join("device")).unwrap();
            }
            symlink(&dir, self.0.join("block").join(name)).unwrap();
            dir
        }

        fn partition(&self, disk: &str, name: &str, number: &str, sectors: u64) {
            let dir = self.node(&format!("{disk}/{name}"), number, sectors);
            fs::write(dir.join("partition"), "1\n").unwrap();
        }

        /// A hardware disk at `path` below `devices/`. The directories on the
        /// way are the devices it is reached through, as in sysfs; `files`
        /// are what the kernel says about it, relative to the disk.
        fn drive(&self, path: &str, number: &str, files: &[(&str, &str)]) {
            let dir = self.node(path, number, 1000);
            for part in ["slaves", "device", "queue"] {
                fs::create_dir_all(dir.join(part)).unwrap();
            }
            for (file, content) in files {
                fs::write(dir.join(file), content).unwrap();
            }
            symlink(&dir, self.0.join("block").join(dir.file_name().unwrap())).unwrap();
        }

        /// The temperature sensor of `disk`. `holder` is where its hardware
        /// keeps the hwmon device, below the disk's `device` directory.
        fn sensor(&self, disk: &str, holder: &str, millidegrees: &str) {
            let dir = self
                .0
                .join("devices")
                .join(disk)
                .join("device")
                .join(holder);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("temp1_input"), millidegrees).unwrap();
        }

        /// `device` is built on `slave`, as LVM, LUKS and MD devices are.
        fn slave(&self, device: &str, slave: &str) {
            let class = self.0.join("class/block");
            symlink(
                class.join(slave),
                class.join(device).join("slaves").join(slave),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    /// Samples with a fixed table of filesystem sizes by mount point. A
    /// mount point missing from the table cannot be read.
    fn sample(
        fx: &Fixture,
        mountinfo: &str,
        sources: &[(&str, DeviceNumber)],
        usages: &[(&str, u64, u64)],
    ) -> Vec<DiskSnapshot> {
        let sources: HashMap<&Path, DeviceNumber> = sources
            .iter()
            .map(|(path, number)| (Path::new(*path), *number))
            .collect();
        let topology = Topology::scan(fx.sys(), mountinfo, &|path| sources.get(path).copied());
        measure(&topology, usages)
    }

    fn measure(topology: &Topology, usages: &[(&str, u64, u64)]) -> Vec<DiskSnapshot> {
        topology.measure(&|mount| {
            usages
                .iter()
                .find(|(mount_point, _, _)| Path::new(mount_point) == mount.mount_point)
                .map(|&(_, used_bytes, total_bytes)| Usage {
                    total_bytes,
                    used_bytes,
                    available_bytes: total_bytes - used_bytes,
                })
        })
    }

    fn disk(name: &str, size: u64, filesystems: u32, used: u64, total: u64) -> DiskSnapshot {
        DiskSnapshot {
            name: name.into(),
            model: None,
            kind: kind_of(name),
            size_bytes: size,
            filesystems,
            fs_used_bytes: used,
            fs_total_bytes: total,
            fs_available_bytes: total - used,
            temp_c: None,
        }
    }

    /// What the plain fixture disks are: they hang off no bus, so the name
    /// alone decides.
    fn kind_of(name: &str) -> Option<String> {
        if name.starts_with("nvme") {
            Some("NVMe SSD".into())
        } else if name.starts_with("sd") {
            Some("SCSI disk".into())
        } else {
            None
        }
    }

    #[test]
    fn stacked_devices_are_attributed_to_the_disk_underneath() {
        let fx = Fixture::new("stacked");
        // An encrypted LVM install: root and swap volumes inside LUKS inside
        // the third partition, with /boot and the ESP beside it.
        fx.disk("nvme0n1", "259:0", 4000 * GIB / 512, true);
        fx.partition("nvme0n1", "nvme0n1p1", "259:1", GIB / 512);
        fx.partition("nvme0n1", "nvme0n1p2", "259:2", GIB / 512);
        fx.partition("nvme0n1", "nvme0n1p3", "259:3", 3998 * GIB / 512);
        fx.disk("dm-0", "254:0", 3998 * GIB / 512, false);
        fx.slave("dm-0", "nvme0n1p3");
        fx.disk("dm-1", "254:1", 3900 * GIB / 512, false);
        fx.slave("dm-1", "dm-0");
        fx.disk("dm-2", "254:2", 98 * GIB / 512, false);
        fx.slave("dm-2", "dm-0");
        // A second disk with nothing mounted, and a loop-mounted image.
        fx.disk("sda", "8:0", 2000 * GIB / 512, true);
        fx.disk("loop0", "7:0", GIB / 512, false);
        let mountinfo = "\
24 1 254:1 / / rw,relatime shared:1 - ext4 /dev/mapper/vg-root rw
25 24 0:5 / /dev rw,nosuid shared:2 - devtmpfs udev rw,size=4096k
26 24 259:2 / /boot rw,relatime shared:3 - ext4 /dev/nvme0n1p2 rw
27 26 259:1 / /boot/efi rw,relatime shared:4 - vfat /dev/nvme0n1p1 rw,fmask=0077
28 24 7:0 / /snap/core/1 ro,nodev shared:5 - squashfs /dev/loop0 ro
29 24 0:40 / /var/lib/docker/overlay2/a/merged rw - overlay overlay rw,lowerdir=/l
30 24 0:41 / /mnt/nas rw - nfs4 nas:/export rw,vers=4.2
";
        let disks = sample(
            &fx,
            mountinfo,
            &[],
            &[
                ("/", 400 * GIB, 3800 * GIB),
                ("/boot", GIB / 2, GIB),
                ("/boot/efi", GIB / 4, GIB),
                ("/snap/core/1", GIB, GIB),
                ("/var/lib/docker/overlay2/a/merged", 400 * GIB, 3800 * GIB),
                ("/mnt/nas", 9000 * GIB, 9999 * GIB),
            ],
        );
        assert_eq!(
            disks,
            vec![
                disk(
                    "nvme0n1",
                    4000 * GIB,
                    3,
                    400 * GIB + GIB / 2 + GIB / 4,
                    3802 * GIB
                ),
                disk("sda", 2000 * GIB, 0, 0, 0),
            ]
        );
    }

    #[test]
    fn a_filesystem_across_disks_is_split_by_the_space_it_occupies_on_each() {
        let fx = Fixture::new("split");
        // md0 stripes a small and a large partition.
        fx.disk("sda", "8:0", 1000, true);
        fx.partition("sda", "sda1", "8:1", 1000);
        fx.disk("sdb", "8:16", 3000, true);
        fx.partition("sdb", "sdb1", "8:17", 3000);
        fx.disk("md0", "9:0", 4000, false);
        fx.slave("md0", "sda1");
        fx.slave("md0", "sdb1");
        // md1 sits on a whole disk and is itself partitioned.
        fx.disk("sdc", "8:32", 5000, true);
        fx.disk("md1", "9:1", 5000, false);
        fx.slave("md1", "sdc");
        fx.partition("md1", "md1p1", "259:0", 4000);
        let mountinfo = "\
30 1 9:0 / /data rw - xfs /dev/md0 rw
31 1 259:0 / /scratch rw - ext4 /dev/md1p1 rw
";
        let disks = sample(
            &fx,
            mountinfo,
            &[],
            &[("/data", 2000, 4000), ("/scratch", 100, 800)],
        );
        assert_eq!(
            disks,
            vec![
                disk("sda", 1000 * 512, 1, 500, 1000),
                disk("sdb", 3000 * 512, 1, 1500, 3000),
                disk("sdc", 5000 * 512, 1, 100, 800),
            ]
        );
    }

    #[test]
    fn every_disk_of_a_mirror_holds_the_whole_filesystem() {
        let fx = Fixture::new("mirror");
        // Two disks, each with a small and a large partition; md0 and md1
        // mirror them pairwise, as a RAID 1 install lays out /boot and /.
        for (disk, number, minor) in [("nvme0n1", "259:0", 1), ("nvme1n1", "259:8", 9)] {
            fx.disk(disk, number, 5000, true);
            fx.partition(
                disk,
                &format!("{disk}p2"),
                &format!("259:{}", minor + 1),
                1000,
            );
            fx.partition(
                disk,
                &format!("{disk}p4"),
                &format!("259:{}", minor + 3),
                4000,
            );
        }
        for (md, number, partition) in [("md0", "9:0", "p2"), ("md1", "9:1", "p4")] {
            let dir = fx.disk(md, number, 1000, false);
            fs::create_dir_all(dir.join("md")).unwrap();
            fs::write(dir.join("md/level"), "raid1\n").unwrap();
            fx.slave(md, &format!("nvme0n1{partition}"));
            fx.slave(md, &format!("nvme1n1{partition}"));
        }
        let mountinfo = "\
30 1 9:1 / / rw - ext4 /dev/md1 rw
31 30 9:0 / /boot rw - ext4 /dev/md0 rw
";
        let disks = sample(&fx, mountinfo, &[], &[("/", 599, 928), ("/boot", 1, 2)]);
        // Neither row is halved: both read what `df` reports.
        assert_eq!(
            disks,
            vec![
                disk("nvme0n1", 5000 * 512, 2, 600, 930),
                disk("nvme1n1", 5000 * 512, 2, 600, 930),
            ]
        );

        // A volume group on two mirrors: each mirror holds its part of the
        // filesystem, and each of its disks holds all of that part.
        let fx = Fixture::new("mirror-lvm");
        for (index, disk) in ["sda", "sdb", "sdc", "sdd"].into_iter().enumerate() {
            fx.disk(disk, &format!("8:{}", index * 16), 3000, true);
        }
        for (md, number, sectors, members) in [
            ("md0", "9:0", 1000, ["sda", "sdb"]),
            ("md1", "9:1", 3000, ["sdc", "sdd"]),
        ] {
            let dir = fx.disk(md, number, sectors, false);
            fs::create_dir_all(dir.join("md")).unwrap();
            fs::write(dir.join("md/level"), "raid1\n").unwrap();
            for member in members {
                fx.slave(md, member);
            }
        }
        fx.disk("dm-0", "254:0", 4000, false);
        fx.slave("dm-0", "md0");
        fx.slave("dm-0", "md1");
        let mountinfo = "30 1 254:0 / /data rw - xfs /dev/mapper/vg-data rw\n";
        assert_eq!(
            sample(&fx, mountinfo, &[], &[("/data", 2000, 4000)]),
            vec![
                disk("sda", 3000 * 512, 1, 500, 1000),
                disk("sdb", 3000 * 512, 1, 500, 1000),
                disk("sdc", 3000 * 512, 1, 1500, 3000),
                disk("sdd", 3000 * 512, 1, 1500, 3000),
            ]
        );
    }

    #[test]
    fn a_filesystem_mounted_twice_counts_once_through_any_readable_mount_point() {
        let fx = Fixture::new("twice");
        fx.disk("sda", "8:0", 1000, true);
        fx.partition("sda", "sda1", "8:1", 1000);
        let mountinfo = "\
30 1 8:1 / /mnt/private/data rw - ext4 /dev/sda1 rw
31 1 8:1 /pub /srv/pub rw - ext4 /dev/sda1 rw
32 1 8:1 / /mnt/also\\040here rw - ext4 /dev/sda1 rw
";
        // The first mount point cannot be read; the second answers.
        let usages = [("/srv/pub", 300, 900), ("/mnt/also here", 300, 900)];
        assert_eq!(
            sample(&fx, mountinfo, &[], &usages),
            vec![disk("sda", 1000 * 512, 1, 300, 900)]
        );
        // With no readable mount point the disk is listed without usage.
        assert_eq!(
            sample(&fx, mountinfo, &[], &[]),
            vec![disk("sda", 1000 * 512, 0, 0, 0)]
        );
    }

    #[test]
    fn btrfs_is_resolved_through_its_source_device_and_member_list() {
        let fx = Fixture::new("btrfs");
        fx.disk("sda", "8:0", 2000, true);
        fx.partition("sda", "sda2", "8:2", 1000);
        fx.disk("sdb", "8:16", 1000, true);
        fx.partition("sdb", "sdb1", "8:17", 1000);
        // Subvolumes share the filesystem's own device number, 0:30.
        let mountinfo = "\
30 1 0:30 /@ / rw - btrfs /dev/sda2 rw,subvol=/@
31 30 0:30 /@home /home rw - btrfs /dev/sda2 rw,subvol=/@home
";
        let sources = [("/dev/sda2", (8, 2))];
        let usages = [("/", 600, 1000), ("/home", 600, 1000)];
        // sysfs does not know the filesystem: the source device has it all.
        assert_eq!(
            sample(&fx, mountinfo, &sources, &usages),
            vec![
                disk("sda", 2000 * 512, 1, 600, 1000),
                disk("sdb", 1000 * 512, 0, 0, 0),
            ]
        );
        // With the member list, both devices share it.
        let members = fx.sys().join("fs/btrfs/6d1f-fsid/devices");
        fs::create_dir_all(&members).unwrap();
        for member in ["sda2", "sdb1"] {
            symlink(
                fx.sys().join("class/block").join(member),
                members.join(member),
            )
            .unwrap();
        }
        assert_eq!(
            sample(&fx, mountinfo, &sources, &usages),
            vec![
                disk("sda", 2000 * 512, 1, 300, 500),
                disk("sdb", 1000 * 512, 1, 300, 500),
            ]
        );
    }

    #[test]
    fn only_hardware_with_media_is_a_disk_and_names_sort_naturally() {
        let fx = Fixture::new("hardware");
        for (name, number) in [
            ("nvme10n1", "259:10"),
            ("nvme2n1", "259:2"),
            ("mmcblk0", "179:0"),
        ] {
            fx.disk(name, number, 1000, true);
        }
        fx.disk("nvme0n1", "259:0", 1000, true);
        // Virtual devices have no `device` link.
        for (name, number) in [("loop0", "7:0"), ("dm-0", "254:0"), ("zram0", "252:0")] {
            fx.disk(name, number, 1000, false);
        }
        // Hardware that is not a disk to report on.
        fx.disk("sr0", "11:0", 1000, true);
        fx.disk("mmcblk0boot0", "179:8", 1000, true);
        fx.disk("sdc", "8:32", 0, true);
        let hidden = fx.disk("nvme0c0n1", "259:20", 1000, true);
        fs::write(hidden.join("hidden"), "1\n").unwrap();
        fs::write(fx.sys().join("block/nvme0n1/hidden"), "0\n").unwrap();

        let names: Vec<_> = sample(&fx, "", &[], &[])
            .into_iter()
            .map(|disk| disk.name)
            .collect();
        assert_eq!(names, ["mmcblk0", "nvme0n1", "nvme2n1", "nvme10n1"]);
    }

    #[test]
    fn mount_table_lines_are_parsed_and_unescaped() {
        let mount = parse_mount(
            "36 35 98:0 /sub /mnt/my\\040disk\\134a rw,noatime shared:1 master:2 - ext3 /dev/root rw,errors=continue",
        )
        .unwrap();
        assert_eq!(mount.device, (98, 0));
        assert_eq!(mount.mount_point, Path::new("/mnt/my disk\\a"));
        assert_eq!(mount.fs_type, "ext3");
        assert_eq!(mount.source, Path::new("/dev/root"));
        // No optional fields at all.
        let mount = parse_mount("21 1 0:20 / /sys rw - sysfs sysfs rw").unwrap();
        assert_eq!((mount.device, mount.fs_type.as_str()), ((0, 20), "sysfs"));
        assert!(parse_mount("36 35 98:0 / /mnt rw").is_none());
        assert!(parse_mount("").is_none());
        assert_eq!(unescape("trailing\\04"), Path::new("trailing\\04"));
    }

    #[test]
    fn a_scan_is_measured_again_as_usage_and_mounts_change() {
        let fx = Fixture::new("remeasure");
        fx.disk("sda", "8:0", 1000, true);
        fx.partition("sda", "sda1", "8:1", 1000);
        let topology = Topology::scan(
            fx.sys(),
            "30 1 8:1 / /data rw - ext4 /dev/sda1 rw\n",
            &|_| None,
        );
        assert_eq!(
            measure(&topology, &[("/data", 100, 900)]),
            vec![disk("sda", 1000 * 512, 1, 100, 900)]
        );
        assert_eq!(
            measure(&topology, &[("/data", 250, 900)]),
            vec![disk("sda", 1000 * 512, 1, 250, 900)]
        );
        // Unmounted since the scan: the disk stays, its usage does not.
        assert_eq!(
            measure(&topology, &[]),
            vec![disk("sda", 1000 * 512, 0, 0, 0)]
        );
    }

    #[test]
    fn a_drive_is_described_by_its_model_and_the_bus_it_hangs_off() {
        let fx = Fixture::new("described");
        // An NVMe controller reports the whole model, padded with spaces.
        fx.drive(
            "pci0/nvme/nvme0/nvme0n1",
            "259:0",
            &[
                ("device/model", "WD_BLACK SN850X 2000GB                  \n"),
                ("queue/rotational", "0\n"),
            ],
        );
        // SATA: sixteen characters of the model behind the placeholder
        // vendor every ATA drive has, on a port of the controller.
        fx.drive(
            "pci0/ata3/host2/target2:0:0/2:0:0:0/block/sda",
            "8:0",
            &[
                ("device/vendor", "ATA     \n"),
                ("device/model", "Samsung SSD 870 \n"),
                ("queue/rotational", "0\n"),
            ],
        );
        fx.drive(
            "pci0/ata4/host3/target3:0:0/3:0:0:0/block/sdb",
            "8:16",
            &[
                ("device/vendor", "ATA     \n"),
                ("device/model", "ST4000DM004-2CV1\n"),
                ("queue/rotational", "1\n"),
            ],
        );
        // SAS: a vendor of its own, behind an end device of the expander.
        fx.drive(
            "pci0/host0/port-0:0/end_device-0:0/target0:0:0/0:0:0:0/block/sdc",
            "8:32",
            &[
                ("device/vendor", "SEAGATE \n"),
                ("device/model", "ST1200MM0009    \n"),
                ("queue/rotational", "1\n"),
            ],
        );
        // USB: the bridge repeats its vendor in the model, and says the
        // medium rotates whatever it is.
        fx.drive(
            "pci0/usb2/2-1/2-1:1.0/host4/target4:0:0/4:0:0:0/block/sdd",
            "8:48",
            &[
                ("device/vendor", "SanDisk \n"),
                ("device/model", "SanDisk Ultra   \n"),
                ("queue/rotational", "1\n"),
            ],
        );
        // A volume of a RAID controller: SCSI and nothing more is known.
        fx.drive(
            "pci0/host5/target5:2:0/5:2:0:0/block/sde",
            "8:64",
            &[
                ("device/vendor", "DELL    \n"),
                ("device/model", "PERC H730 Mini  \n"),
            ],
        );
        // Virtual machines: a virtio disk has no model, and a disk on a
        // virtio SCSI adapter is still virtual.
        fx.drive("pci0/virtio1/block/vda", "254:0", &[]);
        fx.drive(
            "pci0/virtio2/host6/target6:0:0/6:0:0:1/block/sdf",
            "8:80",
            &[
                ("device/vendor", "QEMU    \n"),
                ("device/model", "QEMU HARDDISK   \n"),
                ("queue/rotational", "1\n"),
            ],
        );
        // A memory card has a product name and says what kind of card it is.
        fx.drive(
            "pci0/mmc_host/mmc0/mmc0:0001/block/mmcblk0",
            "179:0",
            &[("device/name", "SD64G\n"), ("device/type", "SD\n")],
        );

        let described: Vec<_> = Topology::scan(fx.sys(), "", &|_| None)
            .disks
            .into_iter()
            .map(|disk| {
                (
                    disk.name,
                    disk.model.unwrap_or_default(),
                    disk.kind.unwrap_or_default(),
                )
            })
            .collect();
        let expected = [
            ("mmcblk0", "SD64G", "SD card"),
            ("nvme0n1", "WD_BLACK SN850X 2000GB", "NVMe SSD"),
            ("sda", "Samsung SSD 870", "SATA SSD"),
            ("sdb", "ST4000DM004-2CV1", "SATA HDD"),
            ("sdc", "SEAGATE ST1200MM0009", "SAS HDD"),
            ("sdd", "SanDisk Ultra", "USB disk"),
            ("sde", "DELL PERC H730 Mini", "SCSI disk"),
            ("sdf", "QEMU HARDDISK", "virtual disk"),
            ("vda", "", "virtual disk"),
        ]
        .map(|(name, model, kind)| (name.to_owned(), model.to_owned(), kind.to_owned()));
        assert_eq!(described, expected);
    }

    #[test]
    fn each_disk_reports_the_temperature_of_its_own_hardware() {
        let fx = Fixture::new("temperature");
        // An NVMe controller keeps its sensor beside the namespace. The
        // reading is thousandths of a degree, converted from whole kelvins.
        fx.disk("nvme0n1", "259:0", 1000, true);
        fx.sensor("nvme0n1", "hwmon3", "51850\n");
        // A namespace behind two controllers hangs off its subsystem, which
        // links to both; its own block directory there is no controller.
        fx.disk("nvme1n1", "259:8", 1000, true);
        fx.sensor("nvme1n1", "nvme1/hwmon5", "40850\n");
        fx.sensor("nvme1n1", "nvme2/hwmon6", "42850\n");
        fx.sensor("nvme1n1", "nvme1n1/hwmon7", "99000\n");
        // drivetemp puts the sensor in a hwmon directory of the SCSI device.
        fx.disk("sda", "8:0", 1000, true);
        fx.sensor("sda", "hwmon/hwmon4", "34000\n");
        // No sensor: a USB stick, a virtual disk, SATA without drivetemp.
        fx.disk("sdb", "8:16", 1000, true);
        // Sensors that answer nothing usable: zero kelvin, and no number.
        fx.disk("sdc", "8:32", 1000, true);
        fx.sensor("sdc", "hwmon/hwmon8", "-273150\n");
        fx.disk("sdd", "8:48", 1000, true);
        fx.sensor("sdd", "hwmon/hwmon9", "\n");

        let mut topology = Topology::scan(fx.sys(), "", &|_| None);
        // Scanning alone asks no drive: that is what --no-temp leaves.
        assert!(topology.disks.iter().all(|disk| disk.temp_c.is_none()));
        topology.read_temperatures(fx.sys());
        // The readings stay with the disks through every measurement.
        for _ in 0..2 {
            let temperatures: Vec<_> = measure(&topology, &[])
                .into_iter()
                .map(|disk| (disk.name, disk.temp_c))
                .collect();
            assert_eq!(
                temperatures,
                [
                    ("nvme0n1".to_owned(), Some(52)),
                    ("nvme1n1".to_owned(), Some(43)),
                    ("sda".to_owned(), Some(34)),
                    ("sdb".to_owned(), None),
                    ("sdc".to_owned(), None),
                    ("sdd".to_owned(), None),
                ]
            );
        }
    }

    #[test]
    fn temperatures_are_whole_degrees_within_what_a_drive_can_be_at() {
        let fx = Fixture::new("degrees");
        fx.disk("sda", "8:0", 1000, true);
        for (millidegrees, degrees) in [
            ("34499", Some(34)),
            ("34500", Some(35)),
            ("0", Some(0)),
            ("-4500", Some(-4)),
            ("-5501", Some(-6)),
            ("150499", Some(150)),
            ("150500", None),
            ("-50500", Some(-50)),
            ("-50501", None),
            ("warm", None),
        ] {
            fx.sensor("sda", "hwmon/hwmon1", millidegrees);
            assert_eq!(
                temperature(&fx.sys().join("block/sda")),
                degrees,
                "{millidegrees}"
            );
        }
        // Only numbered hwmon and controller directories are looked into.
        assert!(numbered("hwmon12", "hwmon") && numbered("nvme0", "nvme"));
        for name in ["hwmon", "hwmonx", "nvme0n1", "nvme", "xhwmon1"] {
            assert!(
                !numbered(name, "hwmon") && !numbered(name, "nvme"),
                "{name}"
            );
        }
    }

    #[test]
    fn the_running_system_reports_consistent_disks() {
        let mut probe = Probe::new(true);
        let disks = probe.sample();
        let names: Vec<_> = disks.iter().map(|disk| disk.name.clone()).collect();
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(names, unique);
        // The second sample reuses the scan and still lists every disk.
        let again: Vec<_> = probe.sample().into_iter().map(|disk| disk.name).collect();
        assert_eq!(names, again);
        for disk in &disks {
            assert!(disk.size_bytes > 0, "{disk:?}");
            assert!(disk.fs_used_bytes <= disk.fs_total_bytes, "{disk:?}");
            assert!(
                disk.fs_available_bytes <= disk.fs_total_bytes - disk.fs_used_bytes,
                "{disk:?}"
            );
            if disk.filesystems == 0 {
                assert_eq!(disk.fs_total_bytes, 0, "{disk:?}");
            }
            // What the kernel says of the drive is printed as it stands.
            for text in [&disk.model, &disk.kind].into_iter().flatten() {
                assert!(!text.is_empty() && text.trim() == text, "{disk:?}");
            }
            assert!(
                disk.temp_c
                    .is_none_or(|degrees| PLAUSIBLE_CELSIUS.contains(&i64::from(degrees))),
                "{disk:?}"
            );
        }
        // With temperature probing off the same disks are listed, unasked.
        let unasked = Probe::new(false).sample();
        assert_eq!(
            names,
            unasked
                .iter()
                .map(|disk| disk.name.clone())
                .collect::<Vec<_>>()
        );
        assert!(unasked.iter().all(|disk| disk.temp_c.is_none()));
    }
}
