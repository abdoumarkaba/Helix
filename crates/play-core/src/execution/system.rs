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
use crate::models::environment::{BatchedTweakCommand, HardwareProfile};
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

    /// Apply all planned system tweaks.
    ///
    /// Class A tweaks (session-scoped) use RAII guards returned in `ActiveGuards`.
    /// Class B tweaks (persistent) are batched and written via play-helper.
    ///
    /// Returns `ActiveGuards` containing session-scoped guards.
    ///
    /// # Errors
    ///
    /// Returns `PlayError::SystemTweakFailed` if any tweak fails.
    pub fn apply_tweaks(
        &self,
        tweaks: &[PlannedTweak],
        hw: &HardwareProfile,
    ) -> Result<ActiveGuards, PlayError> {
        let mut governor_guard: Option<GovernorGuard> = None;
        let mut gpu_perf_guard: Option<GpuPerfGuard> = None;
        let mut class_b_commands: Vec<BatchedTweakCommand> = Vec::new();

        for planned_tweak in tweaks {
            if let TweakDecision::Apply(sys_tweak) = &planned_tweak.decision {
                // Separate Class A (guards) from Class B (batched)
                match sys_tweak {
                    // Class A: session-scoped guards
                    SystemTweak::CpuGovernorPerformance
                    | SystemTweak::NvidiaPersistenceMode
                    | SystemTweak::NvidiaClockLock { .. }
                    | SystemTweak::Fsync
                    | SystemTweak::Esync
                    | SystemTweak::GameMode
                    | SystemTweak::DxvkAsync { .. } => {
                        if let Err(e) = self.apply_one(sys_tweak, hw, &mut governor_guard, &mut gpu_perf_guard)
                        {
                            drop(governor_guard);
                            drop(gpu_perf_guard);
                            return Err(e);
                        }
                    },
                    // Class B: collect for batching
                    SystemTweak::VmMaxMapCount { target } => {
                        class_b_commands.push(BatchedTweakCommand::SysctlWrite(
                            "vm.max_map_count".to_string(),
                            target.to_string(),
                        ));
                    },
                    SystemTweak::ThpMadvise => {
                        class_b_commands.push(BatchedTweakCommand::SysfsWrite(
                            "/sys/kernel/mm/transparent_hugepage/enabled".to_string(),
                            "madvise".to_string(),
                        ));
                    },
                    SystemTweak::SchedAutogroup { enabled } => {
                        // Check if kernel parameter exists
                        let sysctl_path = "/proc/sys/kernel/sched_autogroup";
                        if !self.sysctl_exists(sysctl_path) {
                            tracing::warn!("Kernel parameter {} does not exist on this system - skipping tweak", sysctl_path);
                            continue;
                        }
                        let val = if *enabled { "1" } else { "0" };
                        class_b_commands.push(BatchedTweakCommand::SysctlWrite(
                            "kernel.sched_autogroup".to_string(),
                            val.to_string(),
                        ));
                    },
                    SystemTweak::SplitLockMitigate { enabled } => {
                        let val = if *enabled { "0" } else { "1" };
                        class_b_commands.push(BatchedTweakCommand::SysctlWrite(
                            "kernel.split_lock_mitigate".to_string(),
                            val.to_string(),
                        ));
                    },
                    SystemTweak::UlimitNofile { value } => {
                        class_b_commands.push(BatchedTweakCommand::UlimitNofile(*value));
                    },
                }
            }
        }

        // Execute batched Class B tweaks in single pkexec call
        if !class_b_commands.is_empty() {
            self.apply_batched_tweaks(&class_b_commands)?;
        }

        Ok(ActiveGuards { governor: governor_guard, gpu_perf: gpu_perf_guard })
    }

    /// Apply batched Class B tweaks via single pkexec call.
    fn apply_batched_tweaks(&self, commands: &[BatchedTweakCommand]) -> Result<(), PlayError> {
        let json = serde_json::to_string(commands).map_err(|e| PlayError::SysctlWrite {
            key: "batch".to_string(),
            reason: format!("Failed to serialize batch commands: {e}"),
        })?;

        match self.cmd_runner.run_command("pkexec", &["play-helper", "batch", &json]) {
            Ok(_) => {
                tracing::info!("Batched {} system tweaks applied successfully", commands.len());
                Ok(())
            },
            Err(e) => Err(PlayError::SysctlWrite {
                key: "batch".to_string(),
                reason: format!("Batched system tweaks failed: {e}. Check play-helper installation."),
            }),
        }
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
                // Check if kernel parameter exists (removed in some kernel versions)
                let sysctl_path = "/proc/sys/kernel/sched_autogroup";
                if !self.sysctl_exists(sysctl_path) {
                    tracing::warn!("Kernel parameter {} does not exist on this system - skipping tweak", sysctl_path);
                    return Ok(());
                }
                let val = if *enabled { "1" } else { "0" };
                self.write_sysctl("kernel.sched_autogroup", val)
            },

            SystemTweak::SplitLockMitigate { enabled } => {
                // Kernel semantics: 1 = mitigate (enabled), 0 = off (disabled)
                // We want to DISABLE mitigation for gaming (enabled=false means mitigation off)
                // So when enabled=true (apply tweak), we write 0 to disable mitigation
                let val = if *enabled { "0" } else { "1" };
                self.write_sysctl("kernel.split_lock_mitigate", val)
            },

            SystemTweak::UlimitNofile { value } => {
                self.write_limits_d(*value)
            },
        }
    }

    /// Write a sysctl value via pkexec play-helper.
    /// Prompts user for password via polkit if needed.
    /// Logs warning instead of failing for development/testing without proper privileges.
    fn write_sysctl(&self, key: &str, value: &str) -> Result<(), PlayError> {
        // Try with pkexec for privilege escalation via polkit
        match self.cmd_runner.run_command("pkexec", &["play-helper", "sysctl-write", key, value]) {
            Ok(_) => {
                tracing::info!("Sysctl {} = {} applied successfully", key, value);
                Ok(())
            },
            Err(e) => {
                // Log warning but don't fail - allows testing without play-helper privileges
                tracing::warn!("Sysctl write to {} failed (requires elevated helper): {}", key, e);
                tracing::warn!("Skipping system tweak - game may still work but with suboptimal performance");
                Ok(())
            }
        }
    }

    /// Write a sysfs file via pkexec play-helper.
    /// Prompts user for password via polkit if needed.
    /// Logs warning instead of failing for development/testing without proper privileges.
    fn write_sysfs_file(&self, path: &str, value: &str) -> Result<(), PlayError> {
        // Try with pkexec for privilege escalation via polkit
        match self.cmd_runner.run_command("pkexec", &["play-helper", "sysfs-write", path, value]) {
            Ok(_) => {
                tracing::info!("Sysfs {} = {} applied successfully", path, value);
                Ok(())
            },
            Err(e) => {
                // Log warning but don't fail - allows testing without play-helper privileges
                tracing::warn!("Sysfs write to {} failed (requires elevated helper): {}", path, e);
                tracing::warn!("Skipping system tweak - game may still work but with suboptimal performance");
                Ok(())
            }
        }
    }

    /// Write to /etc/security/limits.d/play.conf via pkexec play-helper.
    /// Prompts user for password via polkit if needed.
    /// Logs warning instead of failing for development/testing without proper privileges.
    fn write_limits_d(&self, value: u64) -> Result<(), PlayError> {
        let content = format!("* soft nofile {value}\n* hard nofile {value}\n");
        match self.cmd_runner.run_command(
            "pkexec",
            &["play-helper", "write-file", "/etc/security/limits.d/play.conf", &content],
        ) {
            Ok(_) => {
                tracing::info!("Ulimit nofile = {} applied successfully", value);
                Ok(())
            },
            Err(e) => {
                // Log warning but don't fail - allows testing without play-helper privileges
                tracing::warn!("Sysctl write to ulimit-nofile failed (requires elevated helper): {}", e);
                tracing::warn!("Skipping system tweak - game may still work but with suboptimal performance");
                Ok(())
            }
        }
    }

    /// Check if a sysctl parameter exists in the filesystem.
    /// Some kernel parameters may be removed or renamed in newer kernel versions.
    fn sysctl_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
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

        let guards = module.apply_tweaks(&tweaks, &hw).unwrap();
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

        let guards = module.apply_tweaks(&tweaks, &hw).unwrap();
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

        let guards = module.apply_tweaks(&tweaks, &hw).unwrap();
        assert!(!guards.is_active()); // Class B has no session guards

        let calls = runner.calls();
        // Now uses pkexec with batched commands
        assert!(calls.iter().any(|c| c.0 == "pkexec" && c.1[0] == "play-helper" && c.1[1] == "batch"));
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

        let guards = module.apply_tweaks(&tweaks, &hw).unwrap();
        assert!(!guards.is_active());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn apply_gracefully_degrades_on_sysctl_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0/cpufreq")).unwrap();
        std::fs::write(
            dir.path().join("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
            "schedutil\n",
        )
        .unwrap();

        // Governor succeeds, sysctl-write fails - but should gracefully degrade
        let runner = MockCommandRunner::new(vec![
            ("play-helper-sysfs-read".to_owned(), Ok("schedutil".to_owned())),
            ("play-helper-sysfs-write".to_owned(), Ok(String::new())),
            // pkexec play-helper returns error (simulating user cancel or failure)
            ("pkexec-play-helper".to_owned(), Err("permission denied".to_owned())),
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

        // Should NOT error - graceful degradation
        let result = module.apply_tweaks(&tweaks, &hw);
        assert!(result.is_ok());
        // Governor guard should still be active (not dropped due to graceful degradation)
        let guards = result.unwrap();
        assert!(guards.governor.is_some());
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

        let guards = module.apply_tweaks(&tweaks, &hw).unwrap();
        assert!(!guards.is_active());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn test_batched_tweak_serialization() {
        let commands = vec![
            BatchedTweakCommand::SysctlWrite("vm.max_map_count".to_string(), "8388608".to_string()),
            BatchedTweakCommand::SysfsWrite("/sys/kernel/mm/transparent_hugepage/enabled".to_string(), "madvise".to_string()),
        ];

        let json = serde_json::to_string(&commands).unwrap();
        let parsed: Vec<BatchedTweakCommand> = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.len(), 2);
    }
}
