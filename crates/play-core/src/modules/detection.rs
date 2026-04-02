#![allow(clippy::pedantic)]

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
//! display refresh) use safe defaults — the tool can still run. Detection
//! failures for **critical** components (GPU vendor, CPU cores) are surfaced
//! as [`PlayError::HardwareDetection`] so the orchestrator can fail fast.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use semver::Version;
use sysinfo::System;
use tracing::{info, warn};

use crate::models::environment::{
    AudioBackend, AudioConfig, CpuProfile, CpuVendor, DisplayProfile, DisplayServer, Distro,
    DistroInfo, DriverType, GpuFeatureSet, GpuProfile, GpuVendor, HardwareProfile, KernelProfile,
    KernelVersion, MemoryProfile, ThpMode, WineAudioDriver,
};
use crate::models::errors::PlayError;

// ---------------------------------------------------------------------------
// CommandRunner trait for testability
// ---------------------------------------------------------------------------

/// Trait for running external commands. Abstracted to allow mocking in tests.
pub trait CommandRunner: Send + Sync {
    /// Run a command and return stdout.
    fn run_command(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

/// Real command runner using `std::process::Command`.
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run_command(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

// ---------------------------------------------------------------------------
// Kernel detection
// ---------------------------------------------------------------------------

/// Parse a kernel version string like "5.15.0-48-generic" into components.
pub fn parse_kernel_version(s: &str) -> Option<KernelVersion> {
    let s = s.split_whitespace().next()?;
    let version_part = if let Some(stripped) = s.strip_prefix("Linux version ") {
        stripped
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
    let contents = fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
        component: "kernel.version".into(),
    })?;
    parse_kernel_version(&contents).ok_or_else(|| PlayError::HardwareDetection {
        component: "kernel.version".into(),
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
    let contents = fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
        component: "vm.max_map_count".into(),
    })?;
    contents
        .trim()
        .parse()
        .map_err(|_| PlayError::HardwareDetection {
            component: "vm.max_map_count".into(),
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
pub fn detect_kernel(proc_root: &Path, sys_root: &Path) -> Result<KernelProfile, PlayError> {
    let version = read_kernel_version(proc_root)?;
    let has_futex2 = kernel_has_futex2(&version);
    let has_fsync = has_futex2;
    let vm_max_map_count = read_vm_max_map_count(proc_root)?;
    let thp_mode = read_thp_mode(sys_root);
    let sched_autogroup = read_sched_autogroup(proc_root);
    let split_lock_mitigate = read_split_lock_mitigate(proc_root);

    info!(
        kernel_version = ?version,
        has_futex2,
        has_fsync,
        "kernel detection complete"
    );

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
fn is_laptop(sys_root: &Path) -> bool {
    sys_root.join("class/power_supply/BAT0/present").exists()
        || sys_root.join("class/power_supply/BAT1/present").exists()
}

/// Read CPU info from /proc/cpuinfo and build a [`CpuProfile`].
pub fn detect_cpu(proc_root: &Path, sys_root: &Path) -> Result<CpuProfile, PlayError> {
    let path = proc_root.join("cpuinfo");
    let contents = fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
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

    let cpu0 = processors
        .get(&0)
        .ok_or_else(|| PlayError::HardwareDetection {
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
    let mut core_ids: HashSet<(String, String)> = HashSet::new();
    for proc_data in processors.values() {
        let phys_id = proc_data.get("physical id").cloned().unwrap_or_default();
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

    info!(
        vendor = ?cpu_vendor,
        model = %model_str,
        physical_cores,
        logical_cores,
        "cpu detection complete"
    );

    Ok(CpuProfile {
        vendor: cpu_vendor,
        model: model_str,
        physical_cores,
        logical_cores,
        base_freq_mhz,
        supports_avx2,
        supports_avx512,
        is_laptop_cpu: is_laptop(sys_root),
    })
}

// ---------------------------------------------------------------------------
// Memory detection
// ---------------------------------------------------------------------------

/// Detect system memory using sysinfo.
pub fn detect_memory() -> MemoryProfile {
    let mut sys = System::new_all();
    sys.refresh_memory();

    let total_mb = sys.total_memory() / 1024;
    let available_mb = sys.available_memory() / 1024;
    let swap_total_mb = sys.total_swap() / 1024;

    info!(
        total_mb,
        available_mb, swap_total_mb, "memory detection complete"
    );

    MemoryProfile {
        total_mb,
        available_mb,
        swap_total_mb,
    }
}

// ---------------------------------------------------------------------------
// GPU detection
// ---------------------------------------------------------------------------

/// Detect GPU from lspci (basic vendor + model).
pub fn detect_gpu_lspci(cmd_runner: &dyn CommandRunner) -> Result<GpuProfile, PlayError> {
    let output =
        cmd_runner
            .run_command("lspci", &["-mm"])
            .map_err(|_| PlayError::HardwareDetection {
                component: "lspci".into(),
            })?;

    let mut gpu_vendor = GpuVendor::Unknown;
    let mut gpu_model = String::from("Unknown GPU");

    // Find first VGA/3D controller line
    for line in output.lines() {
        if !line.contains("VGA compatible") && !line.contains("3D controller") {
            continue;
        }

        // lspci -mm format: "slot Class Vendor Device SVendor SDevice"
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() > 2 {
            let vendor_id = parts.get(2).unwrap_or(&"").trim_matches('"');
            if vendor_id.starts_with("10de") {
                gpu_vendor = GpuVendor::NVIDIA;
            } else if vendor_id.starts_with("1002") || vendor_id.starts_with("1022") {
                gpu_vendor = GpuVendor::AMD;
            } else if vendor_id.starts_with("8086") {
                gpu_vendor = GpuVendor::Intel;
            }

            if let Some(desc) = parts.get(4) {
                gpu_model = desc.trim_matches('"').to_string();
            }
        }
        break;
    }

    info!(vendor = ?gpu_vendor, model = %gpu_model, "gpu lspci detection");

    Ok(GpuProfile {
        vendor: gpu_vendor,
        model: gpu_model,
        vram_mb: 0,
        driver_version: Version::new(0, 0, 0),
        vulkan_version: None,
        driver_type: DriverType::Unknown,
        features: GpuFeatureSet {
            vulkan_1_2: false,
            vulkan_1_3: false,
            ray_tracing: false,
            mesh_shaders: false,
            resizable_bar: false,
            dx12_feature_level: None,
        },
        is_laptop_gpu: false,
        nvidia_vbios_max_clock_mhz: None,
    })
}

/// Enrich NVIDIA GPU profile with nvidia-smi data.
pub fn enrich_nvidia_gpu(
    gpu: &mut GpuProfile,
    cmd_runner: &dyn CommandRunner,
) -> Result<(), PlayError> {
    let query_output = cmd_runner
        .run_command(
            "nvidia-smi",
            &[
                "--query-gpu=clocks.max.gr,memory.total,driver_version",
                "--format=csv,noheader",
            ],
        )
        .map_err(|_| {
            warn!("nvidia-smi query failed, using safe defaults");
            PlayError::HardwareDetection {
                component: "nvidia-smi".into(),
            }
        })?;

    let parts: Vec<&str> = query_output.trim().split(',').collect();
    if parts.len() >= 3 {
        if let Ok(clock_mhz) = parts[0].trim().parse::<u32>() {
            gpu.nvidia_vbios_max_clock_mhz = Some(clock_mhz);
        }
        if let Ok(vram_mb) = parts[1].trim().parse::<u32>() {
            gpu.vram_mb = vram_mb;
        }
        if let Ok(version) = Version::parse(parts[2].trim()) {
            gpu.driver_version = version;
        }
        gpu.driver_type = DriverType::NvidiaProprietary;
    }

    info!(
        vram_mb = gpu.vram_mb,
        driver_version = %gpu.driver_version,
        nvidia_vbios_max_clock_mhz = gpu.nvidia_vbios_max_clock_mhz,
        "nvidia gpu enrichment complete"
    );
    Ok(())
}

/// Detect AMD GPU (stub for v1 — returns NotApplicable decision).
pub fn enrich_amd_gpu(_gpu: &mut GpuProfile) {
    info!("amd gpu detection stubbed for v1");
}

/// Detect Intel GPU (stub for v1).
pub fn enrich_intel_gpu(_gpu: &mut GpuProfile) {
    info!("intel gpu detection stubbed for v1");
}

// ---------------------------------------------------------------------------
// Audio detection
// ---------------------------------------------------------------------------

/// Detect audio backend: `PipeWire`, `PulseAudio`, or `ALSA`.
pub fn detect_audio(cmd_runner: &dyn CommandRunner) -> AudioConfig {
    // Check for PipeWire
    if cmd_runner
        .run_command("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .is_ok()
    {
        info!("audio backend: PipeWire detected");
        return AudioConfig {
            backend: AudioBackend::PipeWire,
            server_rate: 48000,
            wine_driver: WineAudioDriver::Pulse,
            latency_ms: None,
        };
    }

    // Check for PulseAudio
    if cmd_runner
        .run_command("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .is_ok()
    {
        info!("audio backend: PulseAudio detected");
        return AudioConfig {
            backend: AudioBackend::PulseAudio,
            server_rate: 48000,
            wine_driver: WineAudioDriver::Pulse,
            latency_ms: None,
        };
    }

    // Default to ALSA
    warn!("audio backend: defaulting to ALSA");
    AudioConfig {
        backend: AudioBackend::ALSA,
        server_rate: 48000,
        wine_driver: WineAudioDriver::Alsa,
        latency_ms: None,
    }
}

// ---------------------------------------------------------------------------
// Display detection
// ---------------------------------------------------------------------------

/// Detect display server and resolution.
pub fn detect_display(cmd_runner: &dyn CommandRunner) -> DisplayProfile {
    let display_server = if std::env::var("WAYLAND_DISPLAY").is_ok() {
        DisplayServer::Wayland
    } else if std::env::var("DISPLAY").is_ok() {
        // Try to use xrandr for primary resolution on X11
        if let Ok(output) = cmd_runner.run_command("xrandr", &["--current"]) {
            // Look for primary connected monitor
            for line in output.lines() {
                if line.contains("primary") && line.contains("connected") {
                    if let Some(res_part) = line.split_whitespace().nth(3) {
                        if let Some((width, height)) = res_part.split_once('x') {
                            if let (Ok(w), Ok(h)) = (width.parse::<u32>(), height.parse::<u32>()) {
                                info!(display_server = "X11", width, height, "display detection");
                                return DisplayProfile {
                                    server: DisplayServer::X11,
                                    primary_res: (w, h),
                                    refresh_hz: 60.0,
                                };
                            }
                        }
                    }
                }
            }
        }
        DisplayServer::X11
    } else {
        DisplayServer::Unknown
    };

    info!(
        server = ?display_server,
        "display server detection complete"
    );

    DisplayProfile {
        server: display_server,
        primary_res: (1920, 1080),
        refresh_hz: 60.0,
    }
}

// ---------------------------------------------------------------------------
// Distro detection
// ---------------------------------------------------------------------------

/// Detect Linux distro from /etc/os-release.
pub fn detect_distro(etc_root: &Path) -> Result<DistroInfo, PlayError> {
    let path = etc_root.join("os-release");
    let contents = fs::read_to_string(&path).map_err(|_| PlayError::HardwareDetection {
        component: "distro".into(),
    })?;

    let mut id = String::new();
    let mut version_id = String::new();
    let mut pretty_name = String::new();

    for line in contents.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "ID" => id = value.to_string(),
                "VERSION_ID" => version_id = value.to_string(),
                "PRETTY_NAME" => pretty_name = value.to_string(),
                _ => {}
            }
        }
    }

    let distro = match id.as_str() {
        "ubuntu" => Distro::Ubuntu,
        "fedora" => Distro::Fedora,
        "arch" => Distro::Arch,
        "debian" => Distro::Debian,
        "opensuse-leap" | "opensuse-tumbleweed" => Distro::OpenSUSE,
        _ => Distro::Unknown,
    };

    if distro == Distro::Unknown {
        return Err(PlayError::UnsupportedDistro { distro: id });
    }

    info!(
        distro = ?distro,
        version = %version_id,
        "distro detection complete"
    );

    Ok(DistroInfo {
        distro,
        version_id,
        pretty_name,
    })
}

// ---------------------------------------------------------------------------
// Top-level detection orchestrator
// ---------------------------------------------------------------------------

/// Detect all hardware components and return a complete [`HardwareProfile`].
pub fn detect_hardware(
    proc_root: &Path,
    sys_root: &Path,
    etc_root: &Path,
    cmd_runner: &dyn CommandRunner,
) -> Result<HardwareProfile, PlayError> {
    let kernel = detect_kernel(proc_root, sys_root)?;
    let cpu = detect_cpu(proc_root, sys_root)?;
    let memory = detect_memory();

    // GPU detection: base + vendor enrichment
    let mut gpu = detect_gpu_lspci(cmd_runner)?;
    match gpu.vendor {
        GpuVendor::NVIDIA => {
            if let Err(e) = enrich_nvidia_gpu(&mut gpu, cmd_runner) {
                warn!("nvidia gpu enrichment failed: {:?}", e);
            }
        }
        GpuVendor::AMD => {
            enrich_amd_gpu(&mut gpu);
        }
        GpuVendor::Intel => {
            enrich_intel_gpu(&mut gpu);
        }
        GpuVendor::Unknown => {}
    }

    // Check if GPU is a laptop GPU
    gpu.is_laptop_gpu = is_laptop(sys_root);

    let _audio = detect_audio(cmd_runner);
    let display = detect_display(cmd_runner);
    let distro = detect_distro(etc_root)?;

    info!(
        "hardware detection complete: {}/{}/{}",
        cpu.vendor as i32, gpu.vendor as i32, distro.distro as i32
    );

    Ok(HardwareProfile {
        gpu,
        cpu,
        memory,
        kernel,
        display,
        distro,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCommandRunner {
        responses: HashMap<String, Result<String, String>>,
    }

    impl CommandRunner for MockCommandRunner {
        fn run_command(&self, program: &str, _args: &[&str]) -> Result<String, String> {
            self.responses
                .get(program)
                .cloned()
                .unwrap_or(Err("command not mocked".into()))
        }
    }

    #[test]
    fn test_parse_kernel_version() {
        let v = parse_kernel_version("6.18.13-200.fc43.x86_64").unwrap();
        assert_eq!(v.major, 6);
        assert_eq!(v.minor, 18);
        assert_eq!(v.patch, 13);
    }

    #[test]
    fn test_kernel_has_futex2_true() {
        let v = KernelVersion {
            major: 5,
            minor: 16,
            patch: 0,
        };
        assert!(kernel_has_futex2(&v));
    }

    #[test]
    fn test_kernel_has_futex2_false() {
        let v = KernelVersion {
            major: 5,
            minor: 15,
            patch: 0,
        };
        assert!(!kernel_has_futex2(&v));
    }

    #[test]
    fn test_is_laptop_with_bat0() {
        let tempdir = tempfile::tempdir().unwrap();
        let sys = tempdir.path().join("class/power_supply/BAT0");
        fs::create_dir_all(&sys).unwrap();
        fs::write(sys.join("present"), "1").unwrap();

        assert!(is_laptop(tempdir.path()));
    }

    #[test]
    fn test_is_laptop_without_battery() {
        let tempdir = tempfile::tempdir().unwrap();
        fs::create_dir_all(tempdir.path().join("class/power_supply")).unwrap();

        assert!(!is_laptop(tempdir.path()));
    }

    #[test]
    fn test_detect_memory() {
        let mem = detect_memory();
        assert!(mem.total_mb > 0);
    }

    #[test]
    fn test_detect_gpu_lspci_empty() {
        let runner = MockCommandRunner {
            responses: vec![("lspci".to_string(), Ok(String::new()))]
                .into_iter()
                .collect(),
        };

        let gpu = detect_gpu_lspci(&runner).unwrap();
        assert_eq!(gpu.vendor, GpuVendor::Unknown);
    }

    #[test]
    fn test_detect_gpu_nvidia() {
        let output =
            "00:1f.0\t\"VGA compatible\"\t\"10de:2506\"\t\"Nvidia Corp\"\t\"RTX3050\"\t\"1028:000\"\n";
        let runner = MockCommandRunner {
            responses: vec![("lspci".to_string(), Ok(output.to_string()))]
                .into_iter()
                .collect(),
        };

        let gpu = detect_gpu_lspci(&runner).unwrap();
        assert_eq!(gpu.vendor, GpuVendor::NVIDIA);
    }

    #[test]
    fn test_enrich_nvidia_gpu_success() {
        let mut gpu = GpuProfile {
            vendor: GpuVendor::NVIDIA,
            model: "RTX3050".to_string(),
            vram_mb: 0,
            driver_version: Version::new(0, 0, 0),
            vulkan_version: None,
            driver_type: DriverType::Unknown,
            features: GpuFeatureSet {
                vulkan_1_2: false,
                vulkan_1_3: false,
                ray_tracing: false,
                mesh_shaders: false,
                resizable_bar: false,
                dx12_feature_level: None,
            },
            is_laptop_gpu: false,
            nvidia_vbios_max_clock_mhz: None,
        };

        let runner = MockCommandRunner {
            responses: vec![(
                "nvidia-smi".to_string(),
                Ok("1777, 6144, 535.113\n".to_string()),
            )]
            .into_iter()
            .collect(),
        };

        enrich_nvidia_gpu(&mut gpu, &runner).unwrap();
        assert_eq!(gpu.vram_mb, 6144);
        assert_eq!(gpu.nvidia_vbios_max_clock_mhz, Some(1777));
    }

    #[test]
    fn test_detect_audio_pipewire() {
        let runner = MockCommandRunner {
            responses: vec![("wpctl".to_string(), Ok("Volume: 0.50".to_string()))]
                .into_iter()
                .collect(),
        };

        let audio = detect_audio(&runner);
        assert_eq!(audio.backend, AudioBackend::PipeWire);
    }

    #[test]
    fn test_detect_audio_pulseaudio() {
        let mut responses = HashMap::new();
        responses.insert("wpctl".to_string(), Err("not found".into()));
        responses.insert("pactl".to_string(), Ok("Volume: 0.50".into()));

        let runner = MockCommandRunner { responses };
        let audio = detect_audio(&runner);
        assert_eq!(audio.backend, AudioBackend::PulseAudio);
    }

    #[test]
    fn test_detect_distro_ubuntu() {
        let tempdir = tempfile::tempdir().unwrap();
        fs::write(
            tempdir.path().join("os-release"),
            "ID=ubuntu\nVERSION_ID=22.04\nPRETTY_NAME=\"Ubuntu 22.04 LTS\"\n",
        )
        .unwrap();

        let distro = detect_distro(tempdir.path()).unwrap();
        assert_eq!(distro.distro, Distro::Ubuntu);
        assert_eq!(distro.version_id, "22.04");
    }

    #[test]
    fn test_detect_distro_fedora() {
        let tempdir = tempfile::tempdir().unwrap();
        fs::write(
            tempdir.path().join("os-release"),
            "ID=fedora\nVERSION_ID=38\nPRETTY_NAME=\"Fedora 38\"\n",
        )
        .unwrap();

        let distro = detect_distro(tempdir.path()).unwrap();
        assert_eq!(distro.distro, Distro::Fedora);
    }

    #[test]
    fn test_detect_distro_unknown() {
        let tempdir = tempfile::tempdir().unwrap();
        fs::write(
            tempdir.path().join("os-release"),
            "ID=unknown_distro\nVERSION_ID=1.0\n",
        )
        .unwrap();

        let result = detect_distro(tempdir.path());
        assert!(result.is_err());
    }
}
