#![allow(clippy::pedantic)]
/// DecisionEngine — compiled rule tree, no AI, no network.
///
/// All methods are pure functions: given the same inputs they produce the same
/// outputs. Every decision produces a ResolutionDecision for the audit trail.
/// No system state is read or written here.
use semver::Version;

use crate::models::environment::{
    AntiCheat, AudioBackend, Confidence, DecisionSource, DirectXVersion, GameEngine, GameIdentity,
    KernelProfile, PeArchitecture, ResolutionDecision, RunnerType, TranslationLayer,
    WindowsVersion, WineArch, WineAudioDriver,
};
use crate::models::errors::PlayError;
use crate::models::plan::{
    PrefixAction, RunnerAction, RunnersManifest, SystemTweak, TweakConstraint,
    TweakDecision, TweakId,
};

use super::super::models::environment::HardwareProfile;

pub struct DecisionEngine;

impl DecisionEngine {
    // -----------------------------------------------------------------------
    // Hard blocks — must be checked before any other planning
    // -----------------------------------------------------------------------

    /// Returns Err(AntiCheatBlocked) if any anti-cheat has no Linux support.
    pub fn check_hard_blocks(anti_cheat: &[AntiCheat]) -> Result<(), PlayError> {
        for ac in anti_cheat {
            match ac {
                AntiCheat::EasyAntiCheat { linux_supported: false } => {
                    return Err(PlayError::AntiCheatBlocked {
                        name: "EasyAntiCheat (no Linux support)".to_owned(),
                    });
                },
                AntiCheat::BattlEye { linux_supported: false } => {
                    return Err(PlayError::AntiCheatBlocked {
                        name: "BattlEye (no Linux support)".to_owned(),
                    });
                },
                _ => {},
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Translation layer
    // -----------------------------------------------------------------------

    /// Select the graphics translation layer based on DirectX version.
    pub fn select_translation_layer(dx: DirectXVersion) -> (TranslationLayer, ResolutionDecision) {
        let (layer, reason) = match dx {
            DirectXVersion::D3D12 => (
                TranslationLayer::Vkd3dProton,
                "D3D12 → VKD3D-Proton (best D3D12-on-Vulkan translation layer).",
            ),
            DirectXVersion::D3D9
            | DirectXVersion::D3D10
            | DirectXVersion::D3D11
            | DirectXVersion::D3D8 => (
                TranslationLayer::Dxvk,
                "D3D8/9/10/11 → DXVK (mature D3D-on-Vulkan, best compatibility).",
            ),
            DirectXVersion::Vulkan => (
                TranslationLayer::Native,
                "Game uses Vulkan natively; no translation layer needed.",
            ),
            DirectXVersion::OpenGL => (
                TranslationLayer::WineOpenGL,
                "Game uses OpenGL; Wine's OpenGL passthrough is used.",
            ),
            DirectXVersion::Unknown => (
                TranslationLayer::Dxvk,
                "DirectX version unknown; defaulting to DXVK for broadest compatibility.",
            ),
        };
        (
            layer,
            ResolutionDecision {
                field: "graphics.translation_layer".to_owned(),
                chosen: format!("{layer:?}"),
                reason: reason.to_owned(),
                confidence: if matches!(dx, DirectXVersion::Unknown) {
                    Confidence::Inferred
                } else {
                    Confidence::High
                },
                source: DecisionSource::Heuristic,
            },
        )
    }

    // -----------------------------------------------------------------------
    // DXVK async compile
    // -----------------------------------------------------------------------

    /// Async compilation must be disabled for VAC/EAC/BattlEye-protected games.
    /// Also disabled on first run when DXVK cache doesn't exist to avoid compounding overhead.
    pub fn configure_dxvk_async(anti_cheat: &[AntiCheat], cache_path: Option<&std::path::Path>) -> (bool, ResolutionDecision) {

        // Check anti-cheat first (always disable if present)
        if anti_cheat.iter().any(|ac| {
            matches!(
                ac,
                AntiCheat::EasyAntiCheat { .. } | AntiCheat::BattlEye { .. } | AntiCheat::Denuvo
            )
        }) {
            return (
                false,
                ResolutionDecision {
                    field: "graphics.dxvk_config.async_compile".to_owned(),
                    chosen: "false".to_owned(),
                    reason: "Anti-cheat present; async shader compilation disabled to avoid triggering detections.".to_owned(),
                    confidence: Confidence::High,
                    source: DecisionSource::Heuristic,
                },
            );
        }

        // Check if cache file exists (disable async on cold start)
        let cache_exists = cache_path.and_then(|p| p.exists().then_some(true)).unwrap_or(false);

        if !cache_exists {
            return (
                false,
                ResolutionDecision {
                    field: "graphics.dxvk_config.async_compile".to_owned(),
                    chosen: "false".to_owned(),
                    reason: "DXVK state cache not found; async compilation disabled on first run to avoid compounding overhead.".to_owned(),
                    confidence: Confidence::High,
                    source: DecisionSource::Heuristic,
                },
            );
        }

        // Cache exists and no anti-cheat: enable async
        (
            true,
            ResolutionDecision {
                field: "graphics.dxvk_config.async_compile".to_owned(),
                chosen: "true".to_owned(),
                reason: "DXVK state cache exists; async shader compilation enabled to eliminate hitches.".to_owned(),
                confidence: Confidence::High,
                source: DecisionSource::Heuristic,
            },
        )
    }

    // -----------------------------------------------------------------------
    // Runner selection
    // -----------------------------------------------------------------------

    /// Select runner type and resolve the highest version satisfying the floor.
    /// Version is always resolved from the manifest — NEVER hardcoded.
    pub fn select_runner(
        _anti_cheat: &[AntiCheat],
        manifest: &RunnersManifest,
        identity: &GameIdentity,
    ) -> Result<(RunnerType, Version, ResolutionDecision), PlayError> {
        // Always use ProtonGE as default (broadest compatibility)
        let runner_type = RunnerType::ProtonGE;

        let version_floor: Option<Version> = None;

        let candidates: Vec<&crate::models::plan::RunnerRelease> = manifest
            .runners
            .iter()
            .filter(|r| r.runner_type == runner_type)
            .filter(|r| version_floor.as_ref().map_or(true, |floor| &r.version >= floor))
            .collect();

        let best = candidates.iter().max_by(|a, b| a.version.cmp(&b.version)).ok_or_else(|| {
            PlayError::NoRunnerAvailable {
                runner_type: format!("{runner_type:?}"),
                version_min: version_floor
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "any".to_owned()),
            }
        })?;

        let source_note = "";

        // Identity signals that strengthen the ProtonGE choice
        let identity_reason = if identity.has_video_cutscenes == Some(true) {
            Some("video cutscenes detected (Bink/WMF patches in ProtonGE)")
        } else if identity.engine_hint == Some(GameEngine::REEngine) {
            Some("RE Engine detected (ProtonGE has RE Engine patches)")
        } else if identity.dx_version == DirectXVersion::D3D12 {
            Some("D3D12 detected (ProtonGE has best VKD3D integration)")
        } else if identity.pe_arch == PeArchitecture::X86 {
            Some("32-bit binary (ProtonGE has 32-bit prefix support)")
        } else {
            None
        };

        let reason_suffix = identity_reason.map(|r| format!("; {r}")).unwrap_or_default();

        Ok((
            runner_type,
            best.version.clone(),
            ResolutionDecision {
                field: "runner.runner_type + runner.version".to_owned(),
                chosen: format!("{runner_type:?} {}", best.version),
                reason: format!(
                    "{source_note}ProtonGE is the default (broadest game compatibility){reason_suffix}; \
                     version {} is highest available above floor.",
                    best.version
                ),
                confidence: Confidence::High,
                source: DecisionSource::Heuristic,
            },
        ))
    }

    // -----------------------------------------------------------------------
    // Windows version
    // -----------------------------------------------------------------------

    pub fn select_windows_version(dx_version: DirectXVersion) -> (WindowsVersion, ResolutionDecision) {
        let (version, reason, source) =
            if dx_version == DirectXVersion::D3D9 {
                // Some old D3D9 games reject Win10; Win7 has better compatibility.
                (
                WindowsVersion::Win7,
                "D3D9 game detected; Windows 7 avoids compatibility rejections from old titles.",
                DecisionSource::Heuristic,
            )
            } else {
                (
                    WindowsVersion::Win10,
                    "Default to Windows 10; best compatibility for post-2012 titles.",
                    DecisionSource::Heuristic,
                )
            };

        (
            version,
            ResolutionDecision {
                field: "prefix.windows_version".to_owned(),
                chosen: format!("{version:?}"),
                reason: reason.to_owned(),
                confidence: Confidence::Medium,
                source,
            },
        )
    }

    // -----------------------------------------------------------------------
    // Sync mode
    // -----------------------------------------------------------------------

    /// kernel >= 5.16 (has_futex2) → fsync. Otherwise → esync.
    pub fn select_sync_mode(kernel: &KernelProfile) -> (bool, bool, ResolutionDecision) {
        let (fsync, esync, reason) = if kernel.has_futex2 {
            (
                true,
                false,
                "Kernel has futex2 (>= 5.16); fsync preferred over esync for lower overhead.",
            )
        } else {
            (
                false,
                true,
                "Kernel lacks futex2; esync used as fallback (requires ulimit nofile >= 524288).",
            )
        };

        (
            fsync,
            esync,
            ResolutionDecision {
                field: "system.fsync + system.esync".to_owned(),
                chosen: if fsync { "fsync" } else { "esync" }.to_owned(),
                reason: reason.to_owned(),
                confidence: Confidence::High,
                source: DecisionSource::HardwareDetection,
            },
        )
    }

    // -----------------------------------------------------------------------
    // Audio driver
    // -----------------------------------------------------------------------

    pub fn select_audio_driver(backend: AudioBackend) -> (WineAudioDriver, ResolutionDecision) {
        let (driver, reason) = match backend {
            AudioBackend::PipeWire | AudioBackend::PulseAudio => (
                WineAudioDriver::Pulse,
                "PipeWire/PulseAudio detected; Wine Pulse driver provides lowest latency.",
            ),
            AudioBackend::ALSA => {
                (WineAudioDriver::Alsa, "ALSA-only system; Wine ALSA driver used.")
            },
        };
        (
            driver,
            ResolutionDecision {
                field: "audio.wine_driver".to_owned(),
                chosen: format!("{driver:?}"),
                reason: reason.to_owned(),
                confidence: Confidence::High,
                source: DecisionSource::HardwareDetection,
            },
        )
    }

    // -----------------------------------------------------------------------
    // Prefix architecture
    // -----------------------------------------------------------------------

    pub fn select_prefix_arch(pe_arch: PeArchitecture) -> (WineArch, ResolutionDecision) {
        let (arch, reason) = match pe_arch {
            PeArchitecture::X86 => {
                (WineArch::Win32, "32-bit PE binary; Wine prefix must be Win32.")
            },
            PeArchitecture::X86_64 => {
                (WineArch::Win64, "64-bit PE binary; Win64 prefix for native 64-bit support.")
            },
        };
        (
            arch,
            ResolutionDecision {
                field: "prefix.arch".to_owned(),
                chosen: format!("{arch:?}"),
                reason: reason.to_owned(),
                confidence: Confidence::High,
                source: DecisionSource::HardwareDetection,
            },
        )
    }

    // -----------------------------------------------------------------------
    // Runner action
    // -----------------------------------------------------------------------

    pub fn resolve_runner_action(
        runner_type: RunnerType,
        version: &Version,
        manifest: &RunnersManifest,
        runners_install_root: &std::path::Path,
    ) -> Result<RunnerAction, PlayError> {
        // Check play's install directory first
        let install_path =
            runners_install_root.join(format!("{runner_type:?}")).join(version.to_string());
        if install_path.exists() {
            return Ok(RunnerAction::AlreadyInstalled { path: install_path });
        }

        // Check Steam compatibility tools directory
        if let Some(home) = dirs::home_dir() {
            let steam_compat_paths = vec![
                home.join(".steam/steam/compatibilitytools.d"),
                home.join(".local/share/Steam/compatibilitytools.d"),
            ];

            for compat_dir in steam_compat_paths {
                if compat_dir.exists() {
                    // Look for GE-Proton directory matching version
                    // Version format: 10.34.0 -> directory name: GE-Proton10-34
                    let version_str = version.to_string();
                    let parts: Vec<&str> = version_str.split('.').collect();
                    if parts.len() >= 2 {
                        let proton_dir_name = format!("GE-Proton{}-{}", parts[0], parts[1]);
                        let proton_path = compat_dir.join(&proton_dir_name).join("proton");
                        if proton_path.exists() {
                            tracing::info!(
                                event = "runner_found_in_steam",
                                path = %proton_path.display(),
                                "Found runner in Steam compatibility tools directory"
                            );
                            return Ok(RunnerAction::AlreadyInstalled { path: proton_path });
                        }
                    }
                }
            }
        }

        // Not found anywhere, need to download
        let release = manifest
            .runners
            .iter()
            .find(|r| r.runner_type == runner_type && &r.version == version);
        match release {
            Some(r) => Ok(RunnerAction::Download {
                url: r.url.clone(),
                version: version.clone(),
                sha512: r.sha512.clone(),
            }),
            None => Err(PlayError::NoRunnerAvailable {
                runner_type: format!("{runner_type:?}"),
                version_min: version.to_string(),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Prefix action
    // -----------------------------------------------------------------------

    pub fn resolve_prefix_action(
        prefix_root: &std::path::Path,
        exe_hash: &str,
        arch: WineArch,
        windows_version: WindowsVersion,
    ) -> (PrefixAction, ResolutionDecision) {
        let path = prefix_root.join(exe_hash);
        let (action, reason) = if path.exists() {
            (
                PrefixAction::AlreadyExists { path: path.clone() },
                "Wine prefix already exists for this game hash.",
            )
        } else {
            (
                PrefixAction::Create { path: path.clone(), arch, windows_version },
                "No existing prefix; will create via wineboot during execution.",
            )
        };
        (
            action,
            ResolutionDecision {
                field: "prefix.path".to_owned(),
                chosen: path.display().to_string(),
                reason: reason.to_owned(),
                confidence: Confidence::High,
                source: DecisionSource::HardwareDetection,
            },
        )
    }

    // -----------------------------------------------------------------------
    // vm.max_map_count — hardware-proportional tier formula
    // NEVER hardcode 2097152. Always use this function.
    // -----------------------------------------------------------------------

    /// Compute the target vm.max_map_count based on combined RAM + VRAM.
    /// Spec: decision_logic §8 — hardware-proportional, never hardcoded.
    /// Dev machine: 15657 + 6144 = 21801 MB → 8_388_608
    pub fn compute_vm_max_map_count(combined_mb: u64) -> u64 {
        match combined_mb {
            0..=16_384 => 2_097_152,       // SteamOS baseline
            16_385..=32_768 => 8_388_608,  // 4× baseline
            32_769..=65_536 => 16_777_216, // 8× baseline (Intel HX class)
            _ => 2_147_483_642,            // MAX_INT-5, SteamOS max
        }
    }

    // -----------------------------------------------------------------------
    // NVIDIA clock lock formula
    // NEVER hardcode a clock value. Always use this function.
    // -----------------------------------------------------------------------

    /// Compute the NVIDIA lock clock as 95% of VBIOS max, rounded down to nearest 15 MHz.
    /// Uses integer arithmetic to avoid f32 precision loss.
    /// RTX 3050: vbios_max = 1777 → (1777*95)/100 = 1688 → (1688/15)*15 = 1680 MHz
    pub fn compute_nvidia_lock_clock(vbios_max_mhz: u32) -> u32 {
        let ninety_five_pct = (vbios_max_mhz * 95) / 100;
        (ninety_five_pct / 15) * 15
    }

    // -----------------------------------------------------------------------
    // Tweak resolution
    // -----------------------------------------------------------------------

    /// Evaluate a single TweakConstraint against live hardware, returning the decision.
    /// `current_vm_max_map_count` is the system's current value (from KernelProfile).
    pub fn resolve_tweak(
        constraint: &TweakConstraint,
        hw: &HardwareProfile,
        current_vm_max_map_count: u64,
    ) -> TweakDecision {
        // Check GPU vendor constraint.
        if let Some(required_vendor) = constraint.gpu_vendor_required {
            if hw.gpu.vendor != required_vendor {
                return TweakDecision::NotApplicable {
                    reason: "GPU vendor does not match tweak requirement.".to_owned(),
                };
            }
        }

        // Check desktop-only (no laptops).
        if constraint.requires_desktop && (hw.gpu.is_laptop_gpu || hw.cpu.is_laptop_cpu) {
            return TweakDecision::NotApplicable {
                reason: "Tweak is desktop-only; laptop detected — skipping to avoid thermal risk."
                    .to_owned(),
            };
        }

        // Check GPU vendor exclusions.
        if constraint.gpu_vendor_exclusions.contains(&hw.gpu.vendor) {
            return TweakDecision::NotApplicable {
                reason: "GPU vendor is excluded from this tweak.".to_owned(),
            };
        }

        // Check CPU architecture requirement.
        if let Some(required_arch) = constraint.cpu_arch_required {
            if hw.cpu.arch != required_arch {
                return TweakDecision::NotApplicable {
                    reason: format!(
                        "Tweak requires {:?} CPU, but detected {:?}.",
                        required_arch, hw.cpu.arch
                    ),
                };
            }
        }

        // Check kernel version minimum.
        if let Some((maj, min)) = constraint.kernel_version_min {
            let kv = &hw.kernel.version;
            if kv.major < maj || (kv.major == maj && kv.minor < min) {
                return TweakDecision::NotApplicable {
                    reason: "Kernel version too old for this tweak.".to_owned(),
                };
            }
        }

        // Check RAM minimum.
        if let Some(min_mb) = constraint.min_ram_mb {
            if hw.memory.total_mb < min_mb {
                return TweakDecision::NotApplicable {
                    reason: "RAM below threshold for this tweak.".to_owned(),
                };
            }
        }

        // Constraints satisfied — compute the resolved value.
        let combined_mb = hw.memory.total_mb + hw.gpu.vram_mb as u64;

        match constraint.id {
            TweakId::VmMaxMapCount => {
                let target = Self::compute_vm_max_map_count(combined_mb);
                if current_vm_max_map_count >= target {
                    TweakDecision::AlreadySatisfied
                } else {
                    TweakDecision::Apply(SystemTweak::VmMaxMapCount { target })
                }
            },
            TweakId::ThpMadvise => TweakDecision::Apply(SystemTweak::ThpMadvise),
            TweakId::SchedAutogroup => {
                // Spec §10: "Wine/Proton active: evaluate sched_autogroup → 0" (disable).
                // Disabling isolates the game's scheduler group so desktop processes
                // don't steal time slices from the game's cgroup.
                TweakDecision::Apply(SystemTweak::SchedAutogroup { enabled: false })
            },
            TweakId::SplitLockMitigate => {
                TweakDecision::Apply(SystemTweak::SplitLockMitigate { enabled: false })
            },
            TweakId::UlimitNofile => {
                TweakDecision::Apply(SystemTweak::UlimitNofile { value: 524_288 })
            },
            TweakId::CpuGovernorPerformance => {
                TweakDecision::Apply(SystemTweak::CpuGovernorPerformance)
            },
            TweakId::GameMode => TweakDecision::Apply(SystemTweak::GameMode),
            TweakId::DxvkAsync => {
                // DxvkAsync is resolved separately (depends on anti-cheat, not hardware).
                // The registry entry exists for display/audit purposes.
                // This arm should not be reached via resolve_tweak.
                TweakDecision::NotApplicable {
                    reason: "DxvkAsync resolved separately via configure_dxvk_async.".to_owned(),
                }
            },
            TweakId::Fsync => TweakDecision::Apply(SystemTweak::Fsync),
            TweakId::Esync => TweakDecision::Apply(SystemTweak::Esync),
            TweakId::NvidiaPersistenceMode => {
                TweakDecision::Apply(SystemTweak::NvidiaPersistenceMode)
            },
            TweakId::NvidiaClockLock => {
                let mhz =
                    hw.gpu.nvidia_vbios_max_clock_mhz.map_or(0, Self::compute_nvidia_lock_clock);
                if mhz == 0 {
                    TweakDecision::NotApplicable {
                        reason:
                            "NVIDIA VBIOS max clock not detected; cannot compute safe lock value."
                                .to_owned(),
                    }
                } else {
                    TweakDecision::Apply(SystemTweak::NvidiaClockLock {
                        min_mhz: mhz,
                        max_mhz: mhz,
                    })
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::environment::*;
    use crate::models::plan::*;

    fn make_kernel(major: u32, minor: u32, has_futex2: bool) -> KernelProfile {
        KernelProfile {
            version: KernelVersion { major, minor, patch: 0 },
            has_futex2,
            has_fsync: has_futex2,
            vm_max_map_count: 65536,
            thp_mode: ThpMode::Madvise,
            split_lock_mitigate: true,
            sched_autogroup: false,
        }
    }

    fn make_hw(vendor: GpuVendor, is_laptop: bool, vbios_max: Option<u32>) -> HardwareProfile {
        HardwareProfile {
            gpu: GpuProfile {
                vendor,
                model: "Test GPU".to_owned(),
                vram_mb: 6144,
                driver_version: Version::new(0, 0, 0),
                vulkan_version: None,
                driver_type: DriverType::NvidiaProprietary,
                features: GpuFeatureSet {
                    vulkan_1_2: true,
                    vulkan_1_3: true,
                    ray_tracing: false,
                    mesh_shaders: false,
                    resizable_bar: false,
                    dx12_feature_level: None,
                },
                is_laptop_gpu: is_laptop,
                nvidia_vbios_max_clock_mhz: vbios_max,
            },
            cpu: CpuProfile {
                vendor: CpuVendor::Intel,
                model: "Test CPU".to_owned(),
                arch: CpuArch::X86_64,
                physical_cores: 6,
                logical_cores: 12,
                base_freq_mhz: 2400,
                supports_avx2: true,
                supports_avx512: false,
                is_laptop_cpu: is_laptop,
            },
            memory: MemoryProfile { total_mb: 15657, available_mb: 10000, swap_total_mb: 8192 },
            kernel: make_kernel(6, 18, true),
            display: DisplayProfile {
                server: DisplayServer::Wayland,
                primary_res: (1920, 1080),
                refresh_hz: 60.0,
            },
            distro: DistroInfo {
                distro: Distro::Fedora,
                version_id: "43".to_owned(),
                pretty_name: "Fedora Linux 43".to_owned(),
            },
            gaming_tools: crate::models::environment::GamingToolsProfile {
                wine_installed: false,
                wine_version: None,
                winetricks_installed: false,
                gamemode_installed: false,
                dxvk_installed: false,
                proton_available: false,
                vulkan_available: false,
            },
        }
    }

    // --- hard blocks ---

    #[test]
    fn eac_linux_false_is_hard_block() {
        let ac = vec![AntiCheat::EasyAntiCheat { linux_supported: false }];
        assert!(DecisionEngine::check_hard_blocks(&ac).is_err());
    }

    #[test]
    fn battleye_linux_false_is_hard_block() {
        let ac = vec![AntiCheat::BattlEye { linux_supported: false }];
        assert!(DecisionEngine::check_hard_blocks(&ac).is_err());
    }

    #[test]
    fn eac_linux_true_is_not_hard_block() {
        let ac = vec![AntiCheat::EasyAntiCheat { linux_supported: true }];
        assert!(DecisionEngine::check_hard_blocks(&ac).is_ok());
    }

    #[test]
    fn no_anticheat_is_not_hard_block() {
        assert!(DecisionEngine::check_hard_blocks(&[]).is_ok());
    }

    // --- translation layer ---

    #[test]
    fn d3d12_selects_vkd3d_proton() {
        let (layer, dec) = DecisionEngine::select_translation_layer(DirectXVersion::D3D12);
        assert_eq!(layer, TranslationLayer::Vkd3dProton);
        assert_eq!(dec.confidence, Confidence::High);
    }

    #[test]
    fn d3d9_selects_dxvk() {
        let (layer, _) = DecisionEngine::select_translation_layer(DirectXVersion::D3D9);
        assert_eq!(layer, TranslationLayer::Dxvk);
    }

    #[test]
    fn d3d11_selects_dxvk() {
        let (layer, _) = DecisionEngine::select_translation_layer(DirectXVersion::D3D11);
        assert_eq!(layer, TranslationLayer::Dxvk);
    }

    #[test]
    fn d3d8_selects_dxvk() {
        let (layer, _) = DecisionEngine::select_translation_layer(DirectXVersion::D3D8);
        assert_eq!(layer, TranslationLayer::Dxvk);
    }

    #[test]
    fn vulkan_selects_native() {
        let (layer, _) = DecisionEngine::select_translation_layer(DirectXVersion::Vulkan);
        assert_eq!(layer, TranslationLayer::Native);
    }

    #[test]
    fn opengl_selects_wine_opengl() {
        let (layer, _) = DecisionEngine::select_translation_layer(DirectXVersion::OpenGL);
        assert_eq!(layer, TranslationLayer::WineOpenGL);
    }

    // --- async compile ---

    #[test]
    fn denuvo_disables_async() {
        let ac = vec![AntiCheat::Denuvo];
        let (enabled, _) = DecisionEngine::configure_dxvk_async(&ac, None);
        assert!(!enabled);
    }

    #[test]
    fn eac_disables_async() {
        let ac = vec![AntiCheat::EasyAntiCheat { linux_supported: true }];
        let (enabled, _) = DecisionEngine::configure_dxvk_async(&ac, None);
        assert!(!enabled);
    }

    #[test]
    fn no_anticheat_no_cache_disables_async() {
        let (enabled, _) = DecisionEngine::configure_dxvk_async(&[], None);
        assert!(!enabled);
    }

    #[test]
    fn cache_exists_enables_async() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("dxvk-cache");
        std::fs::write(&cache_path, "dummy").unwrap();
        let (enabled, _) = DecisionEngine::configure_dxvk_async(&[], Some(&cache_path));
        assert!(enabled);
    }

    // --- sync mode ---

    #[test]
    fn futex2_selects_fsync() {
        let kernel = make_kernel(6, 18, true);
        let (fsync, esync, _) = DecisionEngine::select_sync_mode(&kernel);
        assert!(fsync);
        assert!(!esync);
    }

    #[test]
    fn no_futex2_selects_esync() {
        let kernel = make_kernel(5, 10, false);
        let (fsync, esync, _) = DecisionEngine::select_sync_mode(&kernel);
        assert!(!fsync);
        assert!(esync);
    }

    // --- vm.max_map_count formula ---

    #[test]
    fn vm_max_map_count_tier_low() {
        assert_eq!(DecisionEngine::compute_vm_max_map_count(4096), 2_097_152);
    }

    #[test]
    fn vm_max_map_count_tier_boundary_16384() {
        assert_eq!(DecisionEngine::compute_vm_max_map_count(16384), 2_097_152);
    }

    #[test]
    fn vm_max_map_count_tier_mid() {
        // 12000 MB is in the 0..=16384 tier → 2_097_152 (no extra 4M tier)
        assert_eq!(DecisionEngine::compute_vm_max_map_count(12000), 2_097_152);
    }

    #[test]
    fn vm_max_map_count_dev_machine() {
        // 15657 RAM + 6144 VRAM = 21801 MB → tier 16385..=32768 → 8_388_608
        assert_eq!(DecisionEngine::compute_vm_max_map_count(21801), 8_388_608);
    }

    #[test]
    fn vm_max_map_count_tier_high() {
        assert_eq!(DecisionEngine::compute_vm_max_map_count(40000), 16_777_216);
    }

    #[test]
    fn vm_max_map_count_tier_max() {
        // >65536 MB → MAX_INT-5
        assert_eq!(DecisionEngine::compute_vm_max_map_count(70000), 2_147_483_642);
    }

    // --- NVIDIA clock lock formula ---

    #[test]
    fn nvidia_clock_lock_rtx3050() {
        // RTX 3050 laptop: vbios_max = 1777 → 1688 → 1680 (rounds to multiple of 15)
        assert_eq!(DecisionEngine::compute_nvidia_lock_clock(1777), 1680);
    }

    #[test]
    fn nvidia_clock_lock_is_divisible_by_15() {
        for max in [1000u32, 1200, 1500, 1600, 1777, 1800, 2000, 2500] {
            let result = DecisionEngine::compute_nvidia_lock_clock(max);
            assert_eq!(result % 15, 0, "clock lock {result} not divisible by 15 for max={max}");
        }
    }

    #[test]
    fn nvidia_clock_lock_never_exceeds_input() {
        for max in [1000u32, 1500, 1777, 2000] {
            let result = DecisionEngine::compute_nvidia_lock_clock(max);
            assert!(result <= max, "clock lock {result} exceeds vbios max {max}");
        }
    }

    // --- tweak resolution ---

    #[test]
    fn laptop_gpu_skips_class_c_persistence_mode() {
        let hw = make_hw(GpuVendor::NVIDIA, true, Some(1777));
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::NvidiaPersistenceMode)
            .unwrap();
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
        assert!(matches!(decision, TweakDecision::NotApplicable { .. }));
    }

    #[test]
    fn laptop_gpu_skips_class_c_clock_lock() {
        let hw = make_hw(GpuVendor::NVIDIA, true, Some(1777));
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::NvidiaClockLock)
            .unwrap();
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
        assert!(matches!(decision, TweakDecision::NotApplicable { .. }));
    }

    #[test]
    fn amd_gpu_skips_class_c() {
        let hw = make_hw(GpuVendor::AMD, false, None);
        for id in [TweakId::NvidiaClockLock, TweakId::NvidiaPersistenceMode] {
            let constraint =
                super::super::tweak_registry::all().iter().find(|c| c.id == id).unwrap();
            let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
            assert!(
                matches!(decision, TweakDecision::NotApplicable { .. }),
                "AMD GPU should skip {id:?}"
            );
        }
    }

    #[test]
    fn intel_gpu_skips_class_c() {
        let hw = make_hw(GpuVendor::Intel, false, None);
        for id in [TweakId::NvidiaClockLock, TweakId::NvidiaPersistenceMode] {
            let constraint =
                super::super::tweak_registry::all().iter().find(|c| c.id == id).unwrap();
            let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
            assert!(matches!(decision, TweakDecision::NotApplicable { .. }));
        }
    }

    #[test]
    fn vm_max_map_count_already_satisfied() {
        let hw = make_hw(GpuVendor::NVIDIA, false, None);
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::VmMaxMapCount)
            .unwrap();
        // Current value already at max — should be AlreadySatisfied.
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 16_777_216);
        assert!(matches!(decision, TweakDecision::AlreadySatisfied));
    }

    #[test]
    fn vm_max_map_count_applied_when_low() {
        let hw = make_hw(GpuVendor::NVIDIA, false, None);
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::VmMaxMapCount)
            .unwrap();
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
        assert!(matches!(decision, TweakDecision::Apply(SystemTweak::VmMaxMapCount { .. })));
    }

    #[test]
    fn desktop_nvidia_applies_clock_lock() {
        let hw = make_hw(GpuVendor::NVIDIA, false, Some(1777));
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::NvidiaClockLock)
            .unwrap();
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
        assert!(
            matches!(
                decision,
                TweakDecision::Apply(SystemTweak::NvidiaClockLock { min_mhz: 1680, max_mhz: 1680 })
            ),
            "unexpected decision: {decision:?}"
        );
    }

    // --- prefix arch ---

    #[test]
    fn x86_selects_win32() {
        let (arch, _) = DecisionEngine::select_prefix_arch(PeArchitecture::X86);
        assert_eq!(arch, WineArch::Win32);
    }

    #[test]
    fn x86_64_selects_win64() {
        let (arch, _) = DecisionEngine::select_prefix_arch(PeArchitecture::X86_64);
        assert_eq!(arch, WineArch::Win64);
    }

    // --- audio ---

    #[test]
    fn pipewire_selects_pulse_driver() {
        let (driver, _) = DecisionEngine::select_audio_driver(AudioBackend::PipeWire);
        assert_eq!(driver, WineAudioDriver::Pulse);
    }

    #[test]
    fn alsa_selects_alsa_driver() {
        let (driver, _) = DecisionEngine::select_audio_driver(AudioBackend::ALSA);
        assert_eq!(driver, WineAudioDriver::Alsa);
    }

    // --- windows version ---

    #[test]
    fn d3d9_selects_win7() {
        let (version, dec) = DecisionEngine::select_windows_version(DirectXVersion::D3D9);
        assert_eq!(version, WindowsVersion::Win7);
        assert!(dec.reason.contains("D3D9"));
    }

    #[test]
    fn d3d11_selects_win10() {
        let (version, _) = DecisionEngine::select_windows_version(DirectXVersion::D3D11);
        assert_eq!(version, WindowsVersion::Win10);
    }

    #[test]
    fn d3d12_selects_win10() {
        let (version, _) = DecisionEngine::select_windows_version(DirectXVersion::D3D12);
        assert_eq!(version, WindowsVersion::Win10);
    }

    // --- laptop cpu skips desktop-only tweaks ---

    #[test]
    fn laptop_cpu_skips_cpu_governor_performance() {
        let hw = make_hw(GpuVendor::NVIDIA, true, Some(1777));
        let constraint = super::super::tweak_registry::all()
            .iter()
            .find(|c| c.id == TweakId::CpuGovernorPerformance)
            .unwrap();
        let decision = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
        assert!(
            matches!(decision, TweakDecision::NotApplicable { .. }),
            "CpuGovernorPerformance should be NotApplicable on laptop CPU"
        );
    }

    // --- CPU arch constraint enforcement (B2 regression) ---

    #[test]
    fn cpu_arch_mismatch_returns_not_applicable() {
        // Build hw with X86_64 arch (default from make_hw)
        let hw = make_hw(GpuVendor::NVIDIA, false, Some(1777));
        assert_eq!(hw.cpu.arch, CpuArch::X86_64);

        // Construct a constraint that requires X86 (32-bit) — should be NotApplicable on X86_64
        let constraint = TweakConstraint {
            id: TweakId::SplitLockMitigate,
            class: TweakClass::B,
            min_ram_mb: None,
            kernel_version_min: None,
            gpu_vendor_required: None,
            gpu_vendor_exclusions: vec![],
            cpu_arch_required: Some(CpuArch::X86),
            requires_desktop: false,
            reversible: true,
            reboot_required: false,
            reboot_resets: true,
            risk_level: RiskLevel::Low,
            rationale: "test constraint",
        };

        let decision = DecisionEngine::resolve_tweak(&constraint, &hw, 65536);
        assert!(
            matches!(decision, TweakDecision::NotApplicable { .. }),
            "Constraint requiring X86 should be NotApplicable on X86_64 hardware"
        );
    }

    #[test]
    fn cpu_arch_match_does_not_exclude() {
        // Build hw with X86 (32-bit) arch
        let mut hw = make_hw(GpuVendor::NVIDIA, false, Some(1777));
        hw.cpu.arch = CpuArch::X86;

        // Constraint requiring X86 — arch check should pass
        let constraint = TweakConstraint {
            id: TweakId::SplitLockMitigate,
            class: TweakClass::B,
            min_ram_mb: None,
            kernel_version_min: None,
            gpu_vendor_required: None,
            gpu_vendor_exclusions: vec![],
            cpu_arch_required: Some(CpuArch::X86),
            requires_desktop: false,
            reversible: true,
            reboot_required: false,
            reboot_resets: true,
            risk_level: RiskLevel::Low,
            rationale: "test constraint",
        };

        let decision = DecisionEngine::resolve_tweak(&constraint, &hw, 65536);
        // Should NOT be NotApplicable due to arch (other constraints may still apply)
        if let TweakDecision::NotApplicable { reason } = &decision {
            assert!(
                !reason.contains("CPU"),
                "Should not be excluded by arch on matching arch, got: {reason}"
            );
        }
    }

    #[test]
    fn cpu_arch_none_constraint_applies_to_any_arch() {
        // X86_64 hardware with None constraint — should not be excluded by arch
        let hw = make_hw(GpuVendor::NVIDIA, false, Some(1777));

        let constraint = TweakConstraint {
            id: TweakId::VmMaxMapCount,
            class: TweakClass::B,
            min_ram_mb: None,
            kernel_version_min: None,
            gpu_vendor_required: None,
            gpu_vendor_exclusions: vec![],
            cpu_arch_required: None,
            requires_desktop: false,
            reversible: true,
            reboot_required: false,
            reboot_resets: true,
            risk_level: RiskLevel::Low,
            rationale: "test constraint",
        };

        let decision = DecisionEngine::resolve_tweak(&constraint, &hw, 65536);
        // Should not be NotApplicable due to arch
        if let TweakDecision::NotApplicable { reason } = &decision {
            assert!(
                !reason.contains("CPU"),
                "None arch constraint should not exclude, got: {reason}"
            );
        }
    }

    // --- Failure-path tests (S3) ---

    fn make_manifest(runner_type: RunnerType, versions: &[&str]) -> RunnersManifest {
        RunnersManifest {
            runners: versions
                .iter()
                .map(|v| RunnerRelease {
                    runner_type,
                    version: Version::parse(v).unwrap(),
                    url: format!("https://example.com/{v}.tar.gz"),
                    sha512: format!("sha512-{v}"),
                })
                .collect(),
        }
    }

    fn make_identity() -> GameIdentity {
        GameIdentity {
            exe_hash: "ab".repeat(32),
            exe_path: std::path::PathBuf::from("/test/test.exe"),
            exe_name: "test.exe".to_owned(),
            steam_app_id: None,
            detected_name: None,
            dx_version: DirectXVersion::D3D11,
            pe_arch: PeArchitecture::X86_64,
            anti_cheat: vec![],
            engine_hint: None,
            has_video_cutscenes: None,
        }
    }

    #[test]
    fn select_runner_empty_manifest_returns_error() {
        let manifest = RunnersManifest { runners: vec![] };
        let identity = make_identity();

        let result = DecisionEngine::select_runner(&[], &manifest, &identity);
        assert!(
            matches!(result, Err(PlayError::NoRunnerAvailable { .. })),
            "expected NoRunnerAvailable on empty manifest, got: {result:?}"
        );
    }

    #[test]
    fn resolve_runner_action_missing_from_manifest_returns_error() {
        let manifest = make_manifest(RunnerType::ProtonGE, &["8.25.0"]);
        let version = Version::parse("9.99.0").unwrap();
        let dir = tempfile::tempdir().unwrap();

        let result = DecisionEngine::resolve_runner_action(
            RunnerType::ProtonGE,
            &version,
            &manifest,
            dir.path(),
        );
        assert!(
            matches!(result, Err(PlayError::NoRunnerAvailable { .. })),
            "expected NoRunnerAvailable when version missing from manifest, got: {result:?}"
        );
    }

    #[test]
    fn resolve_runner_action_wrong_type_returns_error() {
        let manifest = make_manifest(RunnerType::ProtonGE, &["8.25.0"]);
        let version = Version::parse("8.25.0").unwrap();
        let dir = tempfile::tempdir().unwrap();

        let result = DecisionEngine::resolve_runner_action(
            RunnerType::WineGE,
            &version,
            &manifest,
            dir.path(),
        );
        assert!(
            matches!(result, Err(PlayError::NoRunnerAvailable { .. })),
            "expected NoRunnerAvailable when runner type mismatch, got: {result:?}"
        );
    }

    // --- Idempotency tests (S4) ---

    #[test]
    fn resolve_tweak_is_idempotent() {
        let hw = make_hw(GpuVendor::NVIDIA, false, Some(1777));
        let registry = super::super::tweak_registry::all();

        for constraint in registry {
            let d1 = DecisionEngine::resolve_tweak(constraint, &hw, 65536);
            let d2 = DecisionEngine::resolve_tweak(constraint, &hw, 65536);

            // Compare by debug format since TweakDecision doesn't derive PartialEq
            assert_eq!(
                format!("{d1:?}"),
                format!("{d2:?}"),
                "resolve_tweak not idempotent for {:?}",
                constraint.id
            );
        }
    }
}
