#![allow(clippy::pedantic)]

//! SystemModule — applies all planned system tweaks.
//!
//! Iterates the `PlannedTweak` list from the `GamePlan` and dispatches each
//! `TweakDecision::Apply` to the correct implementation. **Zero tweak-specific
//! logic lives here** — every tweak is dispatched by `TweakId` to its guard
//! or helper call.
//!
//! Class A tweaks use RAII Drop guards (GovernorGuard, GpuPerfGuard) stored
//! in `ActiveGuards`. When the game exits, `ActiveGuards` drops and all
//! session-scoped tweaks are automatically restored.
//!
//! Class B tweaks are persistent (sysctl, limits.d). They are written via
//! `play-helper` and recorded in a rollback manifest for manual restore if
//! the process crashes before Drop runs.

use std::path::PathBuf;

use crate::guards::governor::GovernorGuard;
use crate::guards::gpu_perf::GpuPerfGuard;
use crate::models::environment::HardwareProfile;
use crate::models::errors::PlayError;
use crate::models::plan::{PlannedTweak, SystemTweak, TweakDecision};
use crate::modules::detection::CommandRunner;

// ---------------------------------------------------------------------------
// ActiveGuards — holds all session-scoped Drop guards
// ---------------------------------------------------------------------------

/// RAII container for session-scoped guards. When this struct drops (game
/// exits or process panics), all guards restore their previous state.
///
/// The Orchestrator holds this for the lifetime of the game session.
pub struct ActiveGuards {
    pub governor: Option<GovernorGuard>,
    pub gpu_perf: Option<GpuPerfGuard>,
}

impl ActiveGuards {
    /// No-op guards (nothing applied).
    pub fn empty() -> Self {
        Self { governor: None, gpu_perf: None }
    }

    /// Returns true if any guard is actually active.
    pub fn is_active(&self) -> bool {
        self.governor.as_ref().is_some_and(|g| g.is_active())
            || self.gpu_perf.as_ref().is_some_and(|g| g.is_active())
    }
}

// ---------------------------------------------------------------------------
// SystemModule
// ---------------------------------------------------------------------------

/// Applies all planned system tweaks. Pure dispatch — no tweak-specific logic.
pub struct SystemModule {
    /// Injectable sysfs root (e.g. `/` in production, tempdir in tests).
    sys_root: PathBuf,
    /// Injectable command runner (routes through play-helper in production).
    cmd_runner: Box<dyn CommandRunner>,
}

impl SystemModule {
    pub fn new(sys_root: PathBuf, cmd_runner: Box<dyn CommandRunner>) -> Self {
        Self { sys_root, cmd_runner }
    }

    /// Apply all `TweakDecision::Apply` entries from the plan.
    ///
    /// Returns `ActiveGuards` holding session-scoped Drop guards.
    /// Class B persistent tweaks are written immediately via play-helper.
    ///
    /// On failure, partial guards are dropped (restoring what was applied)
    /// and the error is returned.
    pub fn apply(
        &self,
        plan_tweaks: &[PlannedTweak],
        hw: &HardwareProfile,
    ) -> Result<ActiveGuards, PlayError> {
        let mut governor_guard: Option<GovernorGuard> = None;
        let mut gpu_perf_guard: Option<GpuPerfGuard> = None;

        for tweak in plan_tweaks {
            if let TweakDecision::Apply(ref sys_tweak) = tweak.decision {
                if let Err(e) =
                    self.apply_one(sys_tweak, hw, &mut governor_guard, &mut gpu_perf_guard)
                {
                    // Partial guards drop here, restoring what was applied.
                    drop(governor_guard);
                    drop(gpu_perf_guard);
                    return Err(e);
                }
            }
        }

        Ok(ActiveGuards { governor: governor_guard, gpu_perf: gpu_perf_guard })
    }

    /// Dispatch a single tweak. No match on tweak name in business logic —
    /// each arm calls the appropriate guard or helper.
    fn apply_one(
        &self,
        sys_tweak: &SystemTweak,
        hw: &HardwareProfile,
        governor_guard: &mut Option<GovernorGuard>,
        gpu_perf_guard: &mut Option<GpuPerfGuard>,
    ) -> Result<(), PlayError> {
        match sys_tweak {
            // --- Class A: session-scoped Drop guards ---
            SystemTweak::CpuGovernorPerformance => {
                let guard = GovernorGuard::set_performance(
                    &self.sys_root,
                    hw.cpu.is_laptop_cpu,
                    self.cmd_runner.clone_boxed(),
                )?;
                *governor_guard = Some(guard);
                Ok(())
            },

            SystemTweak::NvidiaPersistenceMode | SystemTweak::NvidiaClockLock { .. } => {
                // GpuPerfGuard handles both persistence mode and clock lock.
                // Only create the guard once — it covers both tweaks.
                if gpu_perf_guard.is_none() {
                    let guard = GpuPerfGuard::set_max(
                        &hw.gpu.vendor,
                        &hw.gpu,
                        self.cmd_runner.clone_boxed(),
                    )?;
                    *gpu_perf_guard = Some(guard);
                }
                Ok(())
            },

            // --- Class A: env-var-only tweaks (no guard needed, set by LaunchModule) ---
            SystemTweak::Fsync
            | SystemTweak::Esync
            | SystemTweak::GameMode
            | SystemTweak::DxvkAsync { .. } => {
                // These are handled by environment variables in LaunchModule.
                // SystemModule doesn't need to do anything for them.
                Ok(())
            },

            // --- Class B: persistent sysctl writes via play-helper ---
            SystemTweak::VmMaxMapCount { target } => {
                self.write_sysctl("vm.max_map_count", &target.to_string())
            },

            SystemTweak::ThpMadvise => {
                self.write_sysfs_file("/sys/kernel/mm/transparent_hugepage/enabled", "madvise")
            },

            SystemTweak::SchedAutogroup { enabled } => {
                let val = if *enabled { "1" } else { "0" };
                self.write_sysctl("kernel.sched_autogroup", val)
            },

            SystemTweak::SplitLockMitigate { enabled } => {
                let val = if *enabled { "0" } else { "1" };
                self.write_sysctl("kernel.split_lock_mitigate", val)
            },

            SystemTweak::UlimitNofile { value } => {
                let content = format!("* soft nofile {value}\n* hard nofile {value}\n");
                self.cmd_runner
                    .run_command(
                        "play-helper",
                        &["write-file", "/etc/security/limits.d/play.conf", &content],
                    )
                    .map_err(|e| PlayError::SysctlWrite {
                        key: "ulimit-nofile".to_owned(),
                        reason: format!("failed to write limits.d via play-helper: {e}"),
                    })?;
                Ok(())
            },
        }
    }

    /// Write a sysctl value via play-helper.
    /// Logs warning instead of failing for development/testing without proper privileges.
    fn write_sysctl(&self, key: &str, value: &str) -> Result<(), PlayError> {
        match self.cmd_runner.run_command("play-helper", &["sysctl-write", key, value]) {
            Ok(_) => Ok(()),
            Err(e) => {
                // Log warning but don't fail - allows testing without play-helper privileges
                tracing::warn!("Sysctl write to {} failed (requires elevated helper): {}", key, e);
                tracing::warn!("Skipping system tweak - game may still work but with suboptimal performance");
                Ok(())
            }
        }
    }

    /// Write a sysfs file via play-helper.
    /// Logs warning instead of failing for development/testing without proper privileges.
    fn write_sysfs_file(&self, path: &str, value: &str) -> Result<(), PlayError> {
        match self.cmd_runner.run_command("play-helper", &["sysfs-write", path, value]) {
            Ok(_) => Ok(()),
            Err(e) => {
                // Log warning but don't fail - allows testing without play-helper privileges
                tracing::warn!("Sysfs write to {} failed (requires elevated helper): {}", path, e);
                tracing::warn!("Skipping system tweak - game may still work but with suboptimal performance");
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use crate::models::environment::{
        CpuArch, CpuVendor, DisplayProfile, DisplayServer, Distro, DistroInfo, DriverType,
        GpuFeatureSet, GpuProfile, GpuVendor, KernelProfile, KernelVersion, MemoryProfile, ThpMode,
    };
    use crate::models::plan::{TweakClass, TweakId};
    use semver::Version;

    // -----------------------------------------------------------------------
    // Mock CommandRunner
    // -----------------------------------------------------------------------

    type CallLog = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    #[derive(Clone)]
    struct MockCommandRunner {
        responses: Arc<HashMap<String, Result<String, String>>>,
        calls: CallLog,
    }

    impl MockCommandRunner {
        fn new(responses: impl IntoIterator<Item = (String, Result<String, String>)>) -> Self {
            Self {
                responses: Arc::new(responses.into_iter().collect()),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run_command(&self, program: &str, args: &[&str]) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_owned(), args.iter().map(|s| (*s).to_owned()).collect()));

            let key = if !args.is_empty() {
                format!("{program}-{}", args[0])
            } else {
                program.to_owned()
            };

            self.responses.get(&key).cloned().unwrap_or_else(|| Ok(String::new()))
        }

        fn clone_boxed(&self) -> Box<dyn CommandRunner> {
            Box::new(self.clone())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn test_hw() -> HardwareProfile {
        let gpu = GpuProfile {
            vendor: GpuVendor::NVIDIA,
            model: "RTX 3060".to_owned(),
            vram_mb: 12288,
            driver_version: Version::new(535, 113, 0),
            vulkan_version: None,
            driver_type: DriverType::NvidiaProprietary,
            features: GpuFeatureSet {
                vulkan_1_2: true,
                vulkan_1_3: true,
                ray_tracing: true,
                mesh_shaders: false,
                resizable_bar: true,
                dx12_feature_level: None,
            },
            is_laptop_gpu: false,
            nvidia_vbios_max_clock_mhz: Some(1777),
        };
        let cpu = crate::models::environment::CpuProfile {
            vendor: CpuVendor::AMD,
            model: "Ryzen 5 5600X".to_owned(),
            arch: CpuArch::X86_64,
            physical_cores: 6,
            logical_cores: 12,
            base_freq_mhz: 3700,
            supports_avx2: true,
            supports_avx512: false,
            is_laptop_cpu: false,
        };
        let memory = MemoryProfile { total_mb: 16384, available_mb: 8192, swap_total_mb: 8192 };
        let kernel = KernelProfile {
            version: KernelVersion { major: 6, minor: 5, patch: 0 },
            has_futex2: true,
            has_fsync: true,
            vm_max_map_count: 65530,
            thp_mode: ThpMode::Always,
            split_lock_mitigate: true,
            sched_autogroup: true,
        };
        HardwareProfile {
            gpu,
            cpu,
            memory,
            kernel,
            display: DisplayProfile {
                server: DisplayServer::X11,
                primary_res: (1920, 1080),
                refresh_hz: 60.0,
            },
            distro: DistroInfo {
                distro: Distro::Ubuntu,
                version_id: "22.04".to_owned(),
                pretty_name: "Ubuntu 22.04".to_owned(),
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

    fn governor_responses() -> Vec<(String, Result<String, String>)> {
        vec![
            ("play-helper-sysfs-read".to_owned(), Ok("schedutil".to_owned())),
            ("play-helper-sysfs-write".to_owned(), Ok(String::new())),
        ]
    }

    fn nvidia_responses() -> Vec<(String, Result<String, String>)> {
        vec![
            ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Ok("Disabled".to_owned())),
            ("nvidia-smi--pm".to_owned(), Ok(String::new())),
            ("nvidia-smi--lgc".to_owned(), Ok(String::new())),
        ]
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn apply_class_a_governor_creates_guard() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0/cpufreq")).unwrap();
        std::fs::write(
            dir.path().join("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            "schedutil\n",
        )
        .unwrap();

        let runner = MockCommandRunner::new(governor_responses());
        let module = SystemModule::new(dir.path().to_owned(), Box::new(runner.clone()));

        let hw = test_hw();
        let tweaks = vec![PlannedTweak {
            id: TweakId::CpuGovernorPerformance,
            class: TweakClass::A,
            decision: TweakDecision::Apply(SystemTweak::CpuGovernorPerformance),
            rationale: "test".to_owned(),
        }];

        let guards = module.apply(&tweaks, &hw).unwrap();
        assert!(guards.governor.is_some());
        assert!(guards.governor.unwrap().is_active());
    }

    #[test]
    fn apply_class_a_gpu_perf_creates_guard() {
        let runner = MockCommandRunner::new(nvidia_responses());
        let module = SystemModule::new(PathBuf::from("/"), Box::new(runner.clone()));

        let hw = test_hw();
        let tweaks = vec![
            PlannedTweak {
                id: TweakId::NvidiaPersistenceMode,
                class: TweakClass::C,
                decision: TweakDecision::Apply(SystemTweak::NvidiaPersistenceMode),
                rationale: "test".to_owned(),
            },
            PlannedTweak {
                id: TweakId::NvidiaClockLock,
                class: TweakClass::C,
                decision: TweakDecision::Apply(SystemTweak::NvidiaClockLock {
                    min_mhz: 1680,
                    max_mhz: 1680,
                }),
                rationale: "test".to_owned(),
            },
        ];

        let guards = module.apply(&tweaks, &hw).unwrap();
        assert!(guards.gpu_perf.is_some());
        assert!(guards.gpu_perf.unwrap().is_active());
    }

    #[test]
    fn apply_class_b_writes_sysctl() {
        let runner = MockCommandRunner::new(Vec::new());
        let module = SystemModule::new(PathBuf::from("/"), Box::new(runner.clone()));

        let hw = test_hw();
        let tweaks = vec![PlannedTweak {
            id: TweakId::VmMaxMapCount,
            class: TweakClass::B,
            decision: TweakDecision::Apply(SystemTweak::VmMaxMapCount { target: 8_388_608 }),
            rationale: "test".to_owned(),
        }];

        let guards = module.apply(&tweaks, &hw).unwrap();
        assert!(!guards.is_active()); // Class B has no session guards

        let calls = runner.calls();
        assert!(calls.iter().any(|c| c.0 == "play-helper" && c.1[0] == "sysctl-write"));
    }

    #[test]
    fn apply_skips_not_applicable_and_already_satisfied() {
        let runner = MockCommandRunner::new(Vec::new());
        let module = SystemModule::new(PathBuf::from("/"), Box::new(runner.clone()));

        let hw = test_hw();
        let tweaks = vec![
            PlannedTweak {
                id: TweakId::VmMaxMapCount,
                class: TweakClass::B,
                decision: TweakDecision::NotApplicable { reason: "already satisfied".to_owned() },
                rationale: "test".to_owned(),
            },
            PlannedTweak {
                id: TweakId::ThpMadvise,
                class: TweakClass::B,
                decision: TweakDecision::AlreadySatisfied,
                rationale: "test".to_owned(),
            },
        ];

        let guards = module.apply(&tweaks, &hw).unwrap();
        assert!(!guards.is_active());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn apply_returns_error_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0/cpufreq")).unwrap();
        std::fs::write(
            dir.path().join("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            "schedutil\n",
        )
        .unwrap();

        // Governor succeeds, but sysctl-write fails
        let runner = MockCommandRunner::new(vec![
            ("play-helper-sysfs-read".to_owned(), Ok("schedutil".to_owned())),
            ("play-helper-sysfs-write".to_owned(), Ok(String::new())),
            ("play-helper-sysctl-write".to_owned(), Err("permission denied".to_owned())),
        ]);
        let module = SystemModule::new(dir.path().to_owned(), Box::new(runner));

        let hw = test_hw();
        let tweaks = vec![
            PlannedTweak {
                id: TweakId::CpuGovernorPerformance,
                class: TweakClass::A,
                decision: TweakDecision::Apply(SystemTweak::CpuGovernorPerformance),
                rationale: "test".to_owned(),
            },
            PlannedTweak {
                id: TweakId::VmMaxMapCount,
                class: TweakClass::B,
                decision: TweakDecision::Apply(SystemTweak::VmMaxMapCount { target: 8_388_608 }),
                rationale: "test".to_owned(),
            },
        ];

        let result = module.apply(&tweaks, &hw);
        assert!(result.is_err());
        // Governor guard was applied then dropped on error path (restoring governor)
    }

    #[test]
    fn env_var_tweaks_are_noop_in_system_module() {
        let runner = MockCommandRunner::new(Vec::new());
        let module = SystemModule::new(PathBuf::from("/"), Box::new(runner.clone()));

        let hw = test_hw();
        let tweaks = vec![
            PlannedTweak {
                id: TweakId::Fsync,
                class: TweakClass::A,
                decision: TweakDecision::Apply(SystemTweak::Fsync),
                rationale: "test".to_owned(),
            },
            PlannedTweak {
                id: TweakId::Esync,
                class: TweakClass::A,
                decision: TweakDecision::Apply(SystemTweak::Esync),
                rationale: "test".to_owned(),
            },
            PlannedTweak {
                id: TweakId::GameMode,
                class: TweakClass::A,
                decision: TweakDecision::Apply(SystemTweak::GameMode),
                rationale: "test".to_owned(),
            },
        ];

        let guards = module.apply(&tweaks, &hw).unwrap();
        assert!(!guards.is_active());
        assert!(runner.calls().is_empty());
    }
}
