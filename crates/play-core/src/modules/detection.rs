//! Hardware detection module for the `play` CLI.
//!
//! Detects GPU, CPU, memory, kernel, audio, display, and distro information.
//! All file reads use injectable path roots so the module can be unit-tested
//! with mocked /proc and /sys content.
//!
//! # Architecture
//!
//! Detection is structured as a series of targeted detector functions, each
//! taking injectable path roots. The top-level [`detect_hardware`] runs all
//! sub-detectors and assembles a [`HardwareProfile`].
//!
//! Detection errors for **non-critical** components (e.g. audio rate,
//! display refresh) use safe defaults — the tool can still run.  Detection
//! failures for **critical** components (GPU vendor, CPU cores) are surfaced
//! as [`PlayError::HardwareDetection`] so the orchestrator can fail fast.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::Version;
use sysinfo::System;

use crate::models::environment::{
    AudioBackend, AudioConfig, CpuProfile, CpuVendor, DisplayProfile, DisplayServer,
    DriverType, GameEnvironment, GpuFeatureSet, GpuProfile, GpuVendor, HardwareProfile,
    KernelProfile, KernelVersion, MemoryProfile, ThpMode, WineAudioDriver,
};
use crate::models::errors::PlayError;

// ---------------------------------------------------------------------------
// Package-manager abstraction
// ---------------------------------------------------------------------------

/// Capability that can be mapped to a package name per distro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    GameMode,
    MangoHud,
    Vulkan,
    VulkanTools,
    WineBase,
    CabExtract,
}

/// Trait for distro package managers.
///
/// Each implementation knows how to install packages and map
/// a [`Capability`] to the correct package name on its distro.
pub trait PackageManager: Send + Sync {
    /// Human-readable name, e.g. `"apt"`, `"dnf"`, `"pacman"`.
    fn name(&self) -> &'static str;

    /// Check if a package is already installed.
    fn is_installed(&self, package: &str) -> bool;

    /// Install one or more packages. Returns the first error encountered.
    fn install(&self, packages: &[&str]) -> Result<(), PlayError>;

    /// Map a capability to the correct package name, or `None` if unsupported.
    fn package_for(&self, capability: Capability) -> Option<&'static str>;
}

pub struct AptPm;

impl PackageManager for AptPm {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn is_installed(&self, package: &str) -> bool {
        Command::new("dpkg")
            .args(["-s", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        let output = Command::new("apt-get")
            .args(["install", "-y"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "apt".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "apt".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("libvulkan1"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine64"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct DnfPm;

impl PackageManager for DnfPm {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn is_installed(&self, package: &str) -> bool {
        Command::new("rpm")
            .args(["-q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        let output = Command::new("dnf")
            .args(["install", "-y"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "dnf".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "dnf".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-loader"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct PacmanPm;

impl PackageManager for PacmanPm {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn is_installed(&self, package: &str) -> bool {
        Command::new("pacman")
            .args(["-Q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        let output = Command::new("pacman")
            .args(["-S", "--noconfirm"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "pacman".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "pacman".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-validation-layers"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct ZypperPm;

impl PackageManager for ZypperPm {
    fn name(&self) -> &'static str {
        "zypper"
    }

    fn is_installed(&self, package: &str) -> bool {
        Command::new("rpm")
            .args(["-q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        let output = Command::new("zypper")
            .args(["install", "-y"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "zypper".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "zypper".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-loader"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

/// Detect the available package manager and return the appropriate implementation.
///
/// Checks in order: apt, dnf, pacman, zypper. Returns the first found.
/// Returns an error only if no supported package manager is found.
pub fn detect_package_manager() -> Result<Box<dyn PackageManager>, PlayError> {
    // apt
    if Command::new("apt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(Box::new(AptPm));
    }
    // dnf / yum
    if Command::new("dnf")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(Box::new(DnfPm));
    }
    // pacman
    if Command::new("pacman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(Box::new(PacmanPm));
    }
    // zypper (SUSE / openSUSE)
    if Command::new("zypper")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok(Box::new(ZypperPm));
    }

    Err(PlayError::UnsupportedDistro {
        distro: "unknown".into(),
    })
}

// ---------------------------------------------------------------------------
// Kernel detection
// ---------------------------------------------------------------------------

/// Parse a kernel version string like "5.15.0-48-generic" into components.
pub fn parse_kernel_version(s: &str) -> Option<KernelVersion> {
    let s = s.split_whitespace().next()?;
    let version_part = if s.starts_with("Linux version ") {
        &s["Linux version ".len()..]
    } else {
        s
    };
    // Strip trailing suffix like "-arch" or "-gentoo"
    let version_str = version_part.split('-').next()?;
    let mut components = version_str.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next().unwrap_or("0").parse().ok()?;
    let patch = components.next().unwrap_or("0").parse().ok()?;
    Some(KernelVersion {
        major,
        minor,
        patch,
    })
}

/// Read /proc/version and parse it into a [`KernelVersion`].
pub fn read_kernel_version(proc_root: &Path) -> Result<KernelVersion, PlayError> {
    let path = proc_root.join("version");
    let contents =
        fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
            component: "kernel.version".into(),
        })?;
    parse_kernel_version(&contents).ok_or_else(|| {
        PlayError::HardwareDetection {
            component: "kernel.version".into(),
        }
    })
}

/// Check whether futex2 is supported (kernel >= 5.16).
pub fn kernel_has_futex2(kernel: &KernelVersion) -> bool {
    if kernel.major > 5 {
        true
    } else if kernel.major == 5 {
        kernel.minor >= 16
    } else {
        false
    }
}

/// Read the current vm.max_map_count value.
pub fn read_vm_max_map_count(proc_root: &Path) -> Result<u64, PlayError> {
    let path = proc_root.join("sys/vm/max_map_count");
    let contents =
        fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
            component: "vm.max_map_count".into(),
        })?;
    contents.trim().parse().map_err(|_| {
        PlayError::HardwareDetection {
            component: "vm.max_map_count".into(),
        }
    })
}

/// Read THP mode from sysfs. Returns [`ThpMode::Never`] on read failure.
pub fn read_thp_mode(sys_root: &Path) -> ThpMode {
    let path = sys_root.join("kernel/mm/transparent_hugepage/enabled");
    let contents = fs::read_to_string(&path).unwrap_or_default();
    let first_line = contents.lines().next().unwrap_or("");
    if first_line.contains("[always]") {
        ThpMode::Always
    } else if first_line.contains("[madvise]") {
        ThpMode::Madvise
    } else {
        ThpMode::Never
    }
}

/// Read sched_autogroup setting. Returns `false` on read failure.
pub fn read_sched_autogroup(proc_root: &Path) -> bool {
    let path = proc_root.join("sys/kernel/sched_autogroup_enabled");
    let contents = fs::read_to_string(&path).unwrap_or_default();
    contents.trim() == "1"
}

/// Read split_lock_mitigate setting. Returns `false` on read failure.
pub fn read_split_lock_mitigate(proc_root: &Path) -> bool {
    let path = proc_root.join("sys/kernel/split_lock_mitigate");
    let contents = fs::read_to_string(&path).unwrap_or_default();
    contents.trim() == "1"
}

/// Build a [`KernelProfile`] from the system.
pub fn detect_kernel(
    proc_root: &Path,
    sys_root: &Path,
) -> Result<KernelProfile, PlayError> {
    let version = read_kernel_version(proc_root)?;
    let has_futex2 = kernel_has_futex2(&version);
    // The Proton fsync patchset is equivalent to futex2 support
    let has_fsync = has_futex2;
    let vm_max_map_count = read_vm_max_map_count(proc_root)?;
    let thp_mode = read_thp_mode(sys_root);
    let sched_autogroup = read_sched_autogroup(proc_root);
    let split_lock_mitigate = read_split_lock_mitigate(proc_root);

    Ok(KernelProfile {
        version,
        has_futex2,
        has_fsync,
        vm_max_map_count,
        thp_mode,
        split_lock_mitigate,
        sched_autogroup,
    })
}

// ---------------------------------------------------------------------------
// CPU detection
// ---------------------------------------------------------------------------

/// Detect whether a battery is present — used to gate laptop-only tweaks.
fn is_laptop() -> bool {
    Path::new("/sys/class/power_supply/BAT0/present").exists()
        || Path::new("/sys/class/power_supply/BAT1/present").exists()
}

/// Read CPU info from /proc/cpuinfo and build a [`CpuProfile`].
pub fn detect_cpu(proc_root: &Path) -> Result<CpuProfile, PlayError> {
    let path = proc_root.join("cpuinfo");
    let contents =
        fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
            component: "cpuinfo".into(),
        })?;

    // Group lines into per-processor blocks
    let mut processors: HashMap<u32, HashMap<String, String>> = HashMap::new();
    let mut current_proc: u32 = 0;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if key == "processor" {
                if let Ok(n) = value.parse::<u32>() {
                    current_proc = n;
                }
            }
            processors
                .entry(current_proc)
                .or_default()
                .insert(key.to_string(), value.to_string());
        }
    }

    let cpu0 = processors.get(&0).ok_or_else(|| PlayError::HardwareDetection {
        component: "cpuinfo processor 0".into(),
    })?;

    let vendor_str = cpu0.get("vendor_id").cloned().unwrap_or_default();
    let model_str = cpu0
        .get("model name")
        .cloned()
        .unwrap_or_else(|| "Unknown CPU".to_string());
    let flags: Vec<String> = cpu0
        .get("flags")
        .map(|f| f.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let logical_cores = processors.len() as u32;

    // Physical cores: count unique (physical id, core id) pairs
    let mut core_ids: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for proc_data in processors.values() {
        let phys_id = proc_data
            .get("physical id")
            .cloned()
            .unwrap_or_default();
        let core_id = proc_data.get("core id").cloned().unwrap_or_default();
        core_ids.insert((phys_id, core_id));
    }
    let physical_cores = core_ids.len() as u32;

    // Base frequency from cpu MHz field (may be 0 on some kernels)
    let base_freq_mhz = cpu0
        .get("cpu MHz")
        .and_then(|m| m.parse::<f32>().ok())
        .map(|f| f as u32)
        .unwrap_or(0);

    let supports_avx2 = flags.iter().any(|f| f == "avx2");
    let supports_avx512 = flags.iter().any(|f| f == "avx512f");

    let cpu_vendor = if vendor_str.contains("AuthenticAMD") {
        CpuVendor::AMD
    } else if vendor_str.contains("GenuineIntel") {
        CpuVendor::Intel
    } else {
        CpuVendor::Unknown
    };

    Ok(CpuProfile {
        vendor: cpu_vendor,
        model: model_str,
        physical_cores,
        logical_cores,
        base_freq_mhz,
        supports_avx2,
        supports_avx512,
        is_laptop_cpu: is_laptop(),
    })
}

// ---------------------------------------------------------------------------
// Memory detection
//