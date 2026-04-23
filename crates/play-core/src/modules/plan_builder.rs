#![allow(clippy::pedantic)]
/// PlanBuilder — orchestrates all DecisionEngine calls and assembles the GamePlan.
///
/// Calling `build()` is the only side-effect-free path through planning.
/// Nothing is written to the filesystem here.
use std::path::PathBuf;

use tracing::info;

use crate::models::environment::{
    GameEnvironment, ResolutionDecision, SystemTuning, TranslationLayer,
};
use crate::models::errors::PlayError;
use crate::models::plan::{
    GamePlan, LaunchAction, PlanWarning, PlannedTweak, RequiredPackage, RunnerAction,
    RunnersManifest, TweakDecision, TweakId,
};

use super::decision_engine::DecisionEngine;
use super::tweak_registry;

pub struct PlanBuilder<'a> {
    env: &'a GameEnvironment,
    manifest: &'a RunnersManifest,
    prefix_root: PathBuf,
    runners_install_root: PathBuf,
}

impl<'a> PlanBuilder<'a> {
    pub fn new(
        env: &'a GameEnvironment,
        manifest: &'a RunnersManifest,
        prefix_root: PathBuf,
        runners_install_root: PathBuf,
    ) -> Self {
        Self { env, manifest, prefix_root, runners_install_root }
    }

    /// Build the GamePlan. Pure: no filesystem writes, no system calls.
    pub fn build(self) -> Result<GamePlan, PlayError> {
        let mut warnings: Vec<PlanWarning> = Vec::new();
        let mut hard_blocks: Vec<String> = Vec::new();
        let mut decisions: Vec<ResolutionDecision> = Vec::new();

        // --- 1. Hard block check ---
        info!("Checking hard blocks...");
        if let Err(e) = DecisionEngine::check_hard_blocks(&self.env.identity.anti_cheat) {
            hard_blocks.push(e.to_string());
            info!("Hard block found: {}", e);
            // Return early plan with hard block — Orchestrator will not proceed.
            return Ok(self.early_fail_plan(hard_blocks, false));
        }
        info!("No hard blocks found");

        // --- 4. Translation layer ---
        info!("Selecting translation layer...");
        let (translation_layer, dec) =
            DecisionEngine::select_translation_layer(self.env.identity.dx_version);
        info!("Translation layer: {:?}", translation_layer);
        decisions.push(dec);

        // --- 5. DXVK async ---
        info!("Configuring DXVK async...");
        let (async_compile, dec) = DecisionEngine::configure_dxvk_async(&self.env.identity.anti_cheat);
        info!("DXVK async: {}", async_compile);
        decisions.push(dec);

        // --- 6. Runner ---
        info!("Selecting runner...");
        let db_hit = false; // No play-db in local mode
        let (runner_type, runner_version, dec) = DecisionEngine::select_runner(
            &self.env.identity.anti_cheat,
            self.manifest,
            &self.env.identity,
        )?;
        info!("Runner: {:?} {}", runner_type, runner_version);
        decisions.push(dec);

        info!("Resolving runner action...");
        let runner_action = DecisionEngine::resolve_runner_action(
            runner_type,
            &runner_version,
            self.manifest,
            &self.runners_install_root,
        )?;

        // Extract runner path from runner_action for use in launch_action
        let runner_install_path = match &runner_action {
            RunnerAction::AlreadyInstalled { path } => path.clone(),
            RunnerAction::Download { version, .. } => self
                .runners_install_root
                .join(format!("{runner_type:?}"))
                .join(version.to_string()),
        };

        // --- 7. Sync mode ---
        info!("Selecting sync mode...");
        let (fsync, esync, dec) = DecisionEngine::select_sync_mode(&self.env.hardware.kernel);
        info!("Sync: fsync={}, esync={}", fsync, esync);
        decisions.push(dec);

        // --- 8. Audio driver ---
        info!("Selecting audio driver...");
        let (wine_driver, dec) = DecisionEngine::select_audio_driver(self.env.audio.backend);
        info!("Audio driver: {:?}", wine_driver);
        decisions.push(dec);

        // --- 9. Prefix arch + Windows version ---
        info!("Selecting prefix configuration...");
        let (prefix_arch, dec) = DecisionEngine::select_prefix_arch(self.env.identity.pe_arch);
        decisions.push(dec);

        let (windows_version, dec) = DecisionEngine::select_windows_version(self.env.identity.dx_version);
        info!("Prefix: {:?}, Windows: {:?}", prefix_arch, windows_version);
        decisions.push(dec);

        info!("Resolving prefix action...");
        let (prefix_action, dec) = DecisionEngine::resolve_prefix_action(
            &self.prefix_root,
            &self.env.identity.exe_hash,
            prefix_arch,
            windows_version,
        );
        decisions.push(dec);

        // --- 10. Tweaks ---
        info!("Resolving tweaks...");
        let tweaks = self.resolve_tweaks(&mut warnings, fsync, esync, async_compile);
        let applied_count = tweaks.iter().filter(|t| matches!(t.decision, TweakDecision::Apply(_))).count();
        info!("Applied {} tweaks", applied_count);

        // --- 11. Required packages ---
        info!("Computing required packages...");
        let required_packages = self.compute_required_packages(&translation_layer, &mut warnings);
        info!("Required packages: {}", required_packages.len());

        // --- 12. Build resolved GameEnvironment ---
        info!("Building resolved environment...");
        let mut env = self.env.clone();

        // Fill graphics
        env.graphics.translation_layer = translation_layer;
        env.graphics.dxvk_config.async_compile = async_compile;
        env.graphics.gamemode = tweaks.iter().any(|t| {
            matches!(t.decision, TweakDecision::Apply(crate::models::plan::SystemTweak::GameMode))
        });

        // Fill runner
        env.runner.runner_type = runner_type;
        env.runner.version = runner_version;
        env.runner.install_path = runner_install_path.clone();

        // Fill audio
        env.audio.wine_driver = wine_driver;

        // Fill prefix
        env.prefix.arch = prefix_arch;
        env.prefix.windows_version = windows_version;

        // No play-db integration - use heuristic defaults only

        // Fill system tuning
        for t in &tweaks {
            apply_tweak_to_system(&mut env.system, t);
        }

        // Spec Class A: "PROTON_NO_ESYNC | set when using fsync | prevents double-activation"
        if fsync {
            env.launch.env.insert("PROTON_NO_ESYNC".to_owned(), "1".to_owned());
        }

        // Spec §12: WINE_LARGE_ADDRESS_AWARE for 32-bit binaries with >2GB VRAM
        if self.env.identity.pe_arch == crate::models::environment::PeArchitecture::X86
            && self.env.hardware.gpu.vram_mb > 2048
        {
            env.launch.env.insert("WINE_LARGE_ADDRESS_AWARE".to_owned(), "1".to_owned());
        }

        // Spec §11: VKD3D_FEATURE_LEVEL when VKD3D-Proton is selected
        if translation_layer == crate::models::environment::TranslationLayer::Vkd3dProton {
            if let Some(ref fl) = self.env.hardware.gpu.features.dx12_feature_level {
                env.launch.env.insert("VKD3D_FEATURE_LEVEL".to_owned(), format!("12_{}", fl));
            }
        }

        // Auto-enable MangoHud if installed
        if self.env.graphics.mangohud.as_ref().is_some_and(|m| m.enabled) {
            env.launch.env.insert("MANGOHUD".to_owned(), "1".to_owned());
            info!("MangoHud auto-enabled (installed and detected)");
        }

        // Log completion stats before moving decisions
        info!("Plan complete: {} decisions, {} tweaks, {} warnings", decisions.len(), tweaks.len(), warnings.len());

        // Record all decisions in metadata
        env.metadata.decisions = decisions;
        env.metadata.resolution_source = crate::models::environment::ResolutionSource::FullyAutomatic;

        // --- 13. Build launch action ---
        let exe_path = env.identity.exe_path.clone();
        let working_dir = exe_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();

        env.launch.exe_path = exe_path.clone();
        env.launch.working_dir = working_dir.clone();

        let launch_action = LaunchAction::Spawn {
            exe_path,
            working_dir,
            args: Vec::new(), // TODO: Allow CLI args
            env: env.launch.env.clone(),
            runner_path: runner_install_path,
            runner_type: env.runner.runner_type,
        };

        Ok(GamePlan {
            env,
            warnings,
            hard_blocks,
            required_packages,
            runner_action,
            prefix_action,
            launch_action,
            tweaks,
            db_hit, // Set to false at line 60 (no play-db in local mode)
        })
    }

    fn resolve_tweaks(
        &self,
        warnings: &mut Vec<PlanWarning>,
        fsync: bool,
        esync: bool,
        async_compile: bool,
    ) -> Vec<PlannedTweak> {
        let hw = &self.env.hardware;
        let current_vm = hw.kernel.vm_max_map_count;
        let registry = tweak_registry::all();
        let mut tweaks: Vec<PlannedTweak> = Vec::new();

        for constraint in registry {
            // Fsync/Esync are resolved by select_sync_mode, not hardware constraints.
            let decision = match constraint.id {
                TweakId::Fsync => {
                    if fsync {
                        TweakDecision::Apply(crate::models::plan::SystemTweak::Fsync)
                    } else {
                        TweakDecision::NotApplicable {
                            reason: "Kernel lacks futex2; esync used instead.".to_owned(),
                        }
                    }
                },
                TweakId::Esync => {
                    if esync {
                        TweakDecision::Apply(crate::models::plan::SystemTweak::Esync)
                    } else {
                        TweakDecision::NotApplicable {
                            reason: "Fsync available; esync not needed.".to_owned(),
                        }
                    }
                },
                TweakId::DxvkAsync => {
                    if async_compile {
                        TweakDecision::Apply(crate::models::plan::SystemTweak::DxvkAsync {
                            enabled: true,
                        })
                    } else {
                        TweakDecision::NotApplicable {
                            reason: "Anti-cheat present; async shader compilation disabled."
                                .to_owned(),
                        }
                    }
                },
                _ => DecisionEngine::resolve_tweak(constraint, hw, current_vm),
            };

            tweaks.push(PlannedTweak {
                id: constraint.id,
                class: constraint.class,
                decision,
                rationale: constraint.rationale.to_owned(),
            });
        }

        // Check for optional tools — emit warnings if absent.
        // (In a real implementation we'd check if gamemoded binary exists.)
        // For now, absence is detected via `gamemode` flag in env.
        // MangoHud: warn only if not installed (None in env), not if just not configured
        if self.env.graphics.mangohud.is_none() {
            warnings.push(PlanWarning {
                message: "MangoHud not installed — performance overlay unavailable.".to_owned(),
                install_hint: Some(self.mangohud_hint()),
            });
        }

        // Check NVIDIA driver version - warn if outdated
        use crate::models::environment::{DriverType, GpuVendor};
        if self.env.hardware.gpu.vendor == GpuVendor::NVIDIA
            && self.env.hardware.gpu.driver_type == DriverType::NvidiaProprietary
        {
            let min_driver = semver::Version::new(535, 0, 0);
            if self.env.hardware.gpu.driver_version < min_driver {
                let distro = &self.env.hardware.distro.distro;
                let install_cmd = match distro {
                    crate::models::environment::Distro::Ubuntu |
                    crate::models::environment::Distro::Debian => "sudo apt install nvidia-driver-535",
                    crate::models::environment::Distro::Fedora => "sudo dnf install akmod-nvidia",
                    crate::models::environment::Distro::Arch => "sudo pacman -S nvidia",
                    _ => "Update your NVIDIA driver",
                };
                warnings.push(PlanWarning {
                    message: format!(
                        "NVIDIA driver {} may be outdated. Recommended: 535+ for modern games.",
                        self.env.hardware.gpu.driver_version
                    ),
                    install_hint: Some(install_cmd.to_owned()),
                });
            }
        }

        tweaks
    }

    fn compute_required_packages(
        &self,
        layer: &TranslationLayer,
        _warnings: &mut Vec<PlanWarning>,
    ) -> Vec<RequiredPackage> {
        let mut packages = Vec::new();

        // Check wine installation from detected gaming tools
        let wine_installed = self.env.hardware.gaming_tools.wine_installed;
        if !wine_installed {
            packages.push(RequiredPackage {
                name: "wine".to_owned(),
                reason: "Wine runtime for Windows binary execution.".to_owned(),
                already_installed: false,
            });
        }

        // Check gamemode
        let gamemode_installed = self.env.hardware.gaming_tools.gamemode_installed;
        if !gamemode_installed {
            packages.push(RequiredPackage {
                name: "gamemode".to_owned(),
                reason: "GameMode for performance optimization in games.".to_owned(),
                already_installed: false,
            });
        }

        // Check vulkan tools
        let vulkan_available = self.env.hardware.gaming_tools.vulkan_available;
        if !vulkan_available {
            packages.push(RequiredPackage {
                name: "vulkan-tools".to_owned(),
                reason: "Vulkan tools for GPU detection and debugging.".to_owned(),
                already_installed: false,
            });
        }

        // DXVK and VKD3D-Proton are bundled in Proton-GE, no separate installation needed.
        let _ = layer;

        packages
    }

    fn mangohud_hint(&self) -> String {
        use crate::models::environment::Distro;
        match self.env.hardware.distro.distro {
            Distro::Fedora => "sudo dnf install mangohud".to_owned(),
            Distro::Ubuntu | Distro::Debian => "sudo apt install mangohud".to_owned(),
            Distro::Arch => "sudo pacman -S mangohud".to_owned(),
            Distro::OpenSUSE => "sudo zypper install mangohud".to_owned(),
            Distro::Unknown => {
                "Install mangohud from your distribution's package manager.".to_owned()
            },
        }
    }

    fn early_fail_plan(self, hard_blocks: Vec<String>, db_hit: bool) -> GamePlan {
        use crate::models::plan::{LaunchAction, PrefixAction, RunnerAction};
        let exe_path = self.env.identity.exe_path.clone();
        let working_dir = exe_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();

        GamePlan {
            env: self.env.clone(),
            warnings: Vec::new(),
            hard_blocks,
            required_packages: Vec::new(),
            runner_action: RunnerAction::AlreadyInstalled { path: PathBuf::new() },
            prefix_action: PrefixAction::AlreadyExists { path: PathBuf::new() },
            launch_action: LaunchAction::Spawn {
                exe_path,
                working_dir,
                args: Vec::new(),
                env: self.env.launch.env.clone(),
                runner_path: PathBuf::new(),
                runner_type: self.env.runner.runner_type,
            },
            tweaks: Vec::new(),
            db_hit,
        }
    }
}

/// Apply a resolved tweak back to the SystemTuning struct in GameEnvironment.
fn apply_tweak_to_system(system: &mut SystemTuning, t: &PlannedTweak) {
    use crate::models::environment::{CpuGovernor, ThpMode};
    use crate::models::plan::SystemTweak;
    if let TweakDecision::Apply(ref tw) = t.decision {
        match tw {
            SystemTweak::VmMaxMapCount { target } => {
                system.vm_max_map_count = Some(*target);
            },
            SystemTweak::ThpMadvise => {
                system.thp_mode = Some(ThpMode::Madvise);
            },
            SystemTweak::SchedAutogroup { enabled } => {
                system.sched_autogroup = Some(*enabled);
            },
            SystemTweak::SplitLockMitigate { enabled } => {
                system.split_lock_mitigate = Some(*enabled);
            },
            SystemTweak::UlimitNofile { value } => {
                system.ulimit_nofile = Some(*value);
            },
            SystemTweak::CpuGovernorPerformance => {
                system.cpu_governor = Some(CpuGovernor::Performance);
            },
            SystemTweak::Fsync => {
                system.fsync = true;
            },
            SystemTweak::Esync => {
                system.esync = true;
            },
            SystemTweak::GameMode => {
                system.gamemode = true;
            },
            SystemTweak::NvidiaPersistenceMode | SystemTweak::DxvkAsync { .. } => {
                // Handled by GraphicsConfig or written by SystemModule directly.
            },
            SystemTweak::NvidiaClockLock { max_mhz, .. } => {
                system.nvidia_clock_lock_mhz = Some(*max_mhz);
            },
        }
    }
}
