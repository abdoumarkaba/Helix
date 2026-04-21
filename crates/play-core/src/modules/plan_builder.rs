#![allow(clippy::pedantic)]
/// PlanBuilder — orchestrates all DecisionEngine calls and assembles the GamePlan.
///
/// Calling `build()` is the only side-effect-free path through planning.
/// Nothing is written to the filesystem here.
use std::path::PathBuf;

use crate::models::environment::{
    GameEnvironment, ResolutionDecision, SystemTuning, TranslationLayer,
};
use crate::models::errors::PlayError;
use crate::models::plan::{
    DbEntry, GamePlan, LaunchAction, PlanWarning, PlannedTweak, RequiredPackage, RunnersManifest,
    TweakDecision, TweakId,
};

use super::database_client::DatabaseReader;
use super::decision_engine::DecisionEngine;
use super::tweak_registry;

pub struct PlanBuilder<'a> {
    env: &'a GameEnvironment,
    db: &'a dyn DatabaseReader,
    prefix_root: PathBuf,
    runners_install_root: PathBuf,
}

impl<'a> PlanBuilder<'a> {
    pub fn new(
        env: &'a GameEnvironment,
        db: &'a dyn DatabaseReader,
        prefix_root: PathBuf,
        runners_install_root: PathBuf,
    ) -> Self {
        Self { env, db, prefix_root, runners_install_root }
    }

    /// Build the GamePlan. Pure: no filesystem writes, no system calls.
    pub fn build(self) -> Result<GamePlan, PlayError> {
        let mut warnings: Vec<PlanWarning> = Vec::new();
        let mut hard_blocks: Vec<String> = Vec::new();
        let mut decisions: Vec<ResolutionDecision> = Vec::new();

        // --- 1. Load DB entry ---
        let db_entry: Option<DbEntry> = self.db.lookup(&self.env.identity.exe_hash)?;
        let db_hit = db_entry.is_some();

        // --- 2. Load runner manifest ---
        let manifest: RunnersManifest = self.db.load_runners_manifest()?;

        // --- 3. Hard block check ---
        if let Err(e) = DecisionEngine::check_hard_blocks(&self.env.identity.anti_cheat) {
            hard_blocks.push(e.to_string());
            // Return early plan with hard block — Orchestrator will not proceed.
            return Ok(self.early_fail_plan(hard_blocks, db_hit));
        }

        // --- 4. Translation layer ---
        let (translation_layer, dec) =
            DecisionEngine::select_translation_layer(self.env.identity.dx_version);
        decisions.push(dec);

        // --- 5. DXVK async ---
        let (async_compile, dec) =
            DecisionEngine::configure_dxvk_async(&self.env.identity.anti_cheat, db_entry.as_ref());
        decisions.push(dec);

        // --- 6. Runner ---
        let (runner_type, runner_version, dec) = DecisionEngine::select_runner(
            &self.env.identity.anti_cheat,
            db_entry.as_ref(),
            &manifest,
            &self.env.identity,
        )?;
        decisions.push(dec);

        let runner_action = DecisionEngine::resolve_runner_action(
            runner_type,
            &runner_version,
            &manifest,
            &self.runners_install_root,
        )?;

        // --- 7. Sync mode ---
        let (fsync, esync, dec) = DecisionEngine::select_sync_mode(&self.env.hardware.kernel);
        decisions.push(dec);

        // --- 8. Audio driver ---
        let (wine_driver, dec) = DecisionEngine::select_audio_driver(self.env.audio.backend);
        decisions.push(dec);

        // --- 9. Prefix arch + Windows version ---
        let (prefix_arch, dec) = DecisionEngine::select_prefix_arch(self.env.identity.pe_arch);
        decisions.push(dec);

        let (windows_version, dec) =
            DecisionEngine::select_windows_version(db_entry.as_ref(), self.env.identity.dx_version);
        decisions.push(dec);

        let (prefix_action, dec) = DecisionEngine::resolve_prefix_action(
            &self.prefix_root,
            &self.env.identity.exe_hash,
            prefix_arch,
            windows_version,
        );
        decisions.push(dec);

        // --- 10. Tweaks ---
        let tweaks = self.resolve_tweaks(&mut warnings, fsync, esync, async_compile);

        // --- 11. Required packages ---
        let required_packages = self.compute_required_packages(&translation_layer, &mut warnings);

        // --- 12. Build resolved GameEnvironment ---
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

        // Fill audio
        env.audio.wine_driver = wine_driver;

        // Fill prefix
        env.prefix.arch = prefix_arch;
        env.prefix.windows_version = windows_version;

        // Merge DB dll_overrides and env_vars
        if let Some(ref db) = db_entry {
            for ov in &db.dll_overrides {
                if !env.prefix.dll_overrides.iter().any(|o| o.dll == ov.dll) {
                    env.prefix.dll_overrides.push(ov.clone());
                }
            }
            for (k, v) in &db.extra_env_vars {
                env.prefix.env_vars.insert(k.clone(), v.clone());
            }
        }

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

        // Record all decisions in metadata
        env.metadata.decisions = decisions;
        if db_hit {
            env.metadata.resolution_source =
                crate::models::environment::ResolutionSource::DatabaseAssisted {
                    entry_id: env.identity.exe_hash.clone(),
                    confidence: 0.8,
                };
        }

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
            runner_path: env.runner.install_path.clone(),
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
            db_hit,
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
        // MangoHud: not currently in HardwareProfile — warn unconditionally if not in env.
        if !self.env.graphics.mangohud.as_ref().is_some_and(|m| m.enabled) {
            warnings.push(PlanWarning {
                message: "MangoHud not configured — performance overlay unavailable.".to_owned(),
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
        // Wine is always required.
        // DXVK and VKD3D-Proton are bundled in Proton-GE, no separate installation needed.
        let _ = layer;
        vec![RequiredPackage {
            name: "wine".to_owned(),
            reason: "Wine runtime for Windows binary execution.".to_owned(),
            already_installed: false, // ExecutionModule will verify
        }]
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
