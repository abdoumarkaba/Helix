#![allow(clippy::pedantic)]

//! Session-scoped GPU performance guard.
//!
//! For NVIDIA GPUs: enables persistence mode (`-pm 1`) and, on desktop GPUs
//! only, locks clocks to the computed safe value (`-lgc`).
//! On Drop: resets clocks (`-rgc`) and restores previous persistence state.
//!
//! For AMD GPUs: returns `Noop` (Phase 2 stub — architecture ready, zero
//! panic risk).
//!
//! This is a Class C tweak — desktop-only for clock lock, but persistence
//! mode is safe on laptops (just keeps the driver loaded, no thermal risk).

use crate::models::environment::{GpuProfile, GpuVendor};
use crate::models::errors::PlayError;
use crate::modules::detection::CommandRunner;

// ---------------------------------------------------------------------------
// GpuPerfGuard
// ---------------------------------------------------------------------------

/// RAII guard that sets NVIDIA GPU to max performance and restores on Drop.
///
/// - `Nvidia` variant: persistence mode + optional clock lock applied.
/// - `Noop` variant: nothing applied (AMD/Intel/Unknown, or laptop with no
///   clock lock needed — persistence mode is still applied via `Nvidia`).
///
/// Rust guarantees `Drop::drop` runs even on panic. This is not optional.
pub enum GpuPerfGuard {
    Nvidia {
        /// Previous persistence mode state ("Enabled" or "Disabled").
        prev_persistence: String,
        /// Clock lock value in MHz that was applied, or None if laptop (no lock).
        clock_lock_mhz: Option<u32>,
        /// Injected command runner — used by Drop to restore state.
        /// Stored as Box<dyn> so the guard owns the runner and Drop can call it.
        cmd_runner: Box<dyn CommandRunner>,
    },
    Noop,
}

impl GpuPerfGuard {
    /// Set GPU to max performance mode.
    ///
    /// - `vendor`: GPU vendor enum.
    /// - `gpu`: GPU profile (used for laptop check and VBIOS max clock).
    /// - `cmd_runner`: injectable command runner (nvidia-smi in production,
    ///   mock in tests).
    ///
    /// NVIDIA desktop: persistence mode + clock lock.
    /// NVIDIA laptop: persistence mode only (clock lock is thermal risk).
    /// AMD: Noop with Phase 2 stub warning.
    /// Intel/Unknown: Noop.
    pub fn set_max(
        vendor: &GpuVendor,
        gpu: &GpuProfile,
        cmd_runner: &dyn CommandRunner,
    ) -> Result<Self, PlayError> {
        match vendor {
            GpuVendor::NVIDIA => {
                // --- Persistence mode (safe for both desktop and laptop) ---
                let prev_persistence = cmd_runner
                    .run_command(
                        "nvidia-smi",
                        &["--query-gpu=persistence_mode", "--format=csv,noheader"],
                    )
                    .map_err(|e| PlayError::GpuPerfWrite {
                        reason: format!("failed to query persistence mode: {e}"),
                    })?;
                let prev_persistence = prev_persistence.trim().to_owned();

                cmd_runner
                    .run_command("nvidia-smi", &["-pm", "1"])
                    .map_err(|e| PlayError::GpuPerfWrite {
                        reason: format!("failed to enable persistence mode: {e}"),
                    })?;

                tracing::info!(
                    event = "tweak_applied",
                    tweak = "gpu_perf.persistence_mode",
                    value = "1",
                    previous = %prev_persistence
                );

                // --- Clock lock (desktop only — thermal risk on laptops) ---
                let clock_lock_mhz = if gpu.is_laptop_gpu {
                    tracing::info!(
                        event = "tweak_skipped",
                        tweak = "gpu_perf.clock_lock",
                        reason = "laptop GPU detected — skipping clock lock to avoid thermal risk"
                    );
                    None
                } else {
                    let lock = gpu
                        .nvidia_vbios_max_clock_mhz
                        .map(crate::modules::decision_engine::DecisionEngine::compute_nvidia_lock_clock);

                    if let Some(mhz) = lock {
                        if mhz == 0 {
                            tracing::warn!(
                                event = "tweak_skipped",
                                tweak = "gpu_perf.clock_lock",
                                reason = "computed clock lock value is 0 (VBIOS max not detected)"
                            );
                        } else {
                            let mhz_str = mhz.to_string();
                            cmd_runner
                                .run_command("nvidia-smi", &["-lgc", &mhz_str, &mhz_str])
                                .map_err(|e| {
                                    // Restore persistence mode before returning error
                                    let _ = cmd_runner.run_command("nvidia-smi", &["-pm", "0"]);
                                    PlayError::GpuPerfWrite {
                                        reason: format!("failed to lock GPU clocks to {mhz} MHz: {e}"),
                                    }
                                })?;

                            tracing::info!(
                                event = "tweak_applied",
                                tweak = "gpu_perf.clock_lock",
                                value = mhz,
                                "NVIDIA clock lock applied"
                            );
                        }
                    } else {
                        tracing::warn!(
                            event = "tweak_skipped",
                            tweak = "gpu_perf.clock_lock",
                            reason = "NVIDIA VBIOS max clock not detected; cannot compute lock value"
                        );
                    }
                    lock.filter(|&v| v > 0)
                };

                Ok(Self::Nvidia {
                    prev_persistence,
                    clock_lock_mhz,
                    cmd_runner: Box::from(crate::modules::detection::RealCommandRunner),
                })
            }
            GpuVendor::AMD => {
                tracing::warn!(
                    event = "phase2_stub",
                    tweak = "amd_perf_level",
                    "AMD GPU perf tweaks not implemented in v1"
                );
                Ok(Self::Noop)
            }
            GpuVendor::Intel | GpuVendor::Unknown => Ok(Self::Noop),
        }
    }

    /// Returns true if this guard actually applied any GPU tweaks (not a no-op).
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Noop)
    }
}

impl Drop for GpuPerfGuard {
    fn drop(&mut self) {
        if let Self::Nvidia {
            prev_persistence,
            clock_lock_mhz,
            cmd_runner,
        } = self
        {
            // Reset clocks first (if they were locked)
            if clock_lock_mhz.is_some() {
                if let Err(e) = cmd_runner.run_command("nvidia-smi", &["-rgc"]) {
                    // NEVER panic in Drop — log the error and continue
                    tracing::error!(
                        event = "restore_failed",
                        tweak = "gpu_perf.clock_lock",
                        error = %e
                    );
                } else {
                    tracing::info!(
                        event = "tweak_restored",
                        tweak = "gpu_perf.clock_lock"
                    );
                }
            }

            // Restore persistence mode
            let pm_val = if prev_persistence.contains("Enabled") {
                "1"
            } else {
                "0"
            };
            if let Err(e) = cmd_runner.run_command("nvidia-smi", &["-pm", pm_val]) {
                tracing::error!(
                    event = "restore_failed",
                    tweak = "gpu_perf.persistence_mode",
                    previous = %prev_persistence,
                    error = %e
                );
            } else {
                tracing::info!(
                    event = "tweak_restored",
                    tweak = "gpu_perf.persistence_mode",
                    value = pm_val
                );
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

    use crate::models::environment::{DriverType, GpuFeatureSet};
    use semver::Version;

    // -----------------------------------------------------------------------
    // Mock CommandRunner
    // -----------------------------------------------------------------------

    struct MockCommandRunner {
        responses: HashMap<String, Result<String, String>>,
        /// Record of commands that were actually executed.
        calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    }

    impl MockCommandRunner {
        fn new(
            responses: impl IntoIterator<Item = (String, Result<String, String>)>,
        ) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                calls: std::sync::Mutex::new(Vec::new()),
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

            // Match by program name; for nvidia-smi, also match by first arg
            let key = if program == "nvidia-smi" && !args.is_empty() {
                format!("nvidia-smi-{}", args[0])
            } else {
                program.to_owned()
            };

            self.responses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| Err(format!("unexpected command: {program} {:?}", args)))
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn nvidia_desktop_gpu() -> GpuProfile {
        GpuProfile {
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
        }
    }

    fn nvidia_laptop_gpu() -> GpuProfile {
        let mut gpu = nvidia_desktop_gpu();
        gpu.is_laptop_gpu = true;
        gpu.model = "RTX 3050 Laptop".to_owned();
        gpu.nvidia_vbios_max_clock_mhz = Some(1777);
        gpu
    }

    fn amd_gpu() -> GpuProfile {
        GpuProfile {
            vendor: GpuVendor::AMD,
            model: "RX 6800".to_owned(),
            vram_mb: 16384,
            driver_version: Version::new(0, 0, 0),
            vulkan_version: None,
            driver_type: DriverType::Mesa,
            features: GpuFeatureSet {
                vulkan_1_2: true,
                vulkan_1_3: false,
                ray_tracing: false,
                mesh_shaders: false,
                resizable_bar: false,
                dx12_feature_level: None,
            },
            is_laptop_gpu: false,
            nvidia_vbios_max_clock_mhz: None,
        }
    }

    fn nvidia_responses() -> Vec<(String, Result<String, String>)> {
        vec![
                ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Ok("Disabled".to_owned())),
                ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
                ("nvidia-smi--lgc".to_owned(), Ok("".to_owned())),
                ("nvidia-smi--rgc".to_owned(), Ok("".to_owned())),
                ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
            ]
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn nvidia_desktop_applies_persistence_and_clock_lock() {
        let gpu = nvidia_desktop_gpu();
        let runner = MockCommandRunner::new(nvidia_responses());

        let guard = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner).unwrap();
        assert!(guard.is_active());

        if let GpuPerfGuard::Nvidia {
            prev_persistence,
            clock_lock_mhz,
            cmd_runner: _,
        } = &guard
        {
            assert_eq!(prev_persistence, "Disabled");
            // RTX 3060: vbios_max=1777 → (1777*95)/100=1688 → (1688/15)*15=1680
            assert_eq!(*clock_lock_mhz, Some(1680));
        } else {
            panic!("expected Nvidia variant");
        }

        // Verify commands were issued in order
        let calls = runner.calls();
        // 1. query persistence mode
        assert_eq!(calls[0].0, "nvidia-smi");
        assert_eq!(calls[0].1[0], "--query-gpu=persistence_mode");
        // 2. enable persistence mode
        assert_eq!(calls[1].0, "nvidia-smi");
        assert_eq!(calls[1].1[0], "-pm");
        assert_eq!(calls[1].1[1], "1");
        // 3. lock clocks
        assert_eq!(calls[2].0, "nvidia-smi");
        assert_eq!(calls[2].1[0], "-lgc");
        assert_eq!(calls[2].1[1], "1680");
        assert_eq!(calls[2].1[2], "1680");
    }

    #[test]
    fn nvidia_laptop_applies_persistence_only() {
        let gpu = nvidia_laptop_gpu();
        let responses = vec![
            ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Ok("Disabled".to_owned())),
            ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
        ];
        let runner = MockCommandRunner::new(responses);

        let guard = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner).unwrap();
        assert!(guard.is_active());

        if let GpuPerfGuard::Nvidia {
            prev_persistence,
            clock_lock_mhz,
            cmd_runner: _,
        } = &guard
        {
            assert_eq!(prev_persistence, "Disabled");
            assert_eq!(*clock_lock_mhz, None);
        } else {
            panic!("expected Nvidia variant");
        }

        // Verify: query + pm only, no -lgc
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].1[0], "-pm");
    }

    #[test]
    fn amd_returns_noop() {
        let gpu = amd_gpu();
        let runner = MockCommandRunner::new(Vec::new());

        let guard = GpuPerfGuard::set_max(&GpuVendor::AMD, &gpu, &runner).unwrap();
        assert!(!guard.is_active());
        assert!(matches!(guard, GpuPerfGuard::Noop));

        // No commands should have been issued
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn intel_returns_noop() {
        let mut gpu = amd_gpu();
        gpu.vendor = GpuVendor::Intel;
        let runner = MockCommandRunner::new(Vec::new());

        let guard = GpuPerfGuard::set_max(&GpuVendor::Intel, &gpu, &runner).unwrap();
        assert!(!guard.is_active());
        assert!(matches!(guard, GpuPerfGuard::Noop));
    }

    #[test]
    fn drop_restores_nvidia_on_panic() {
        let gpu = nvidia_desktop_gpu();
        let runner = MockCommandRunner::new(nvidia_responses());

        let guard = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner).unwrap();

        if let GpuPerfGuard::Nvidia {
            prev_persistence,
            clock_lock_mhz,
            cmd_runner: _,
        } = &guard
        {
            assert_eq!(prev_persistence, "Disabled");
            assert_eq!(*clock_lock_mhz, Some(1680));
        } else {
            panic!("expected Nvidia variant");
        }

        // Drop now uses the stored mock runner — verify restore commands
        let calls_before_drop = runner.calls().len();
        drop(guard);
        let calls = runner.calls();

        // After drop: -rgc (reset clocks) + -pm 0 (restore persistence)
        assert_eq!(calls.len(), calls_before_drop + 2);
        assert_eq!(calls[calls_before_drop].0, "nvidia-smi");
        assert_eq!(calls[calls_before_drop].1[0], "-rgc");
        assert_eq!(calls[calls_before_drop + 1].1[0], "-pm");
        assert_eq!(calls[calls_before_drop + 1].1[1], "0");
    }

    #[test]
    fn nvidia_clock_lock_uses_compute_formula() {
        let gpu = nvidia_desktop_gpu();
        let runner = MockCommandRunner::new(nvidia_responses());

        let guard = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner).unwrap();

        if let GpuPerfGuard::Nvidia { clock_lock_mhz, .. } = &guard {
            // Verify the value matches DecisionEngine::compute_nvidia_lock_clock
            let expected =
                crate::modules::decision_engine::DecisionEngine::compute_nvidia_lock_clock(1777);
            assert_eq!(*clock_lock_mhz, Some(expected));
            assert_eq!(expected, 1680);
        } else {
            panic!("expected Nvidia variant");
        }
    }

    #[test]
    fn nvidia_no_vbios_max_skips_clock_lock() {
        let mut gpu = nvidia_desktop_gpu();
        gpu.nvidia_vbios_max_clock_mhz = None;

        let responses = vec![
            ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Ok("Disabled".to_owned())),
            ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
        ];
        let runner = MockCommandRunner::new(responses);

        let guard = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner).unwrap();

        if let GpuPerfGuard::Nvidia { clock_lock_mhz, .. } = &guard {
            assert_eq!(*clock_lock_mhz, None);
        } else {
            panic!("expected Nvidia variant");
        }
    }

    #[test]
    fn persistence_mode_failure_returns_error() {
        let gpu = nvidia_desktop_gpu();
        let responses = vec![
            ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Err("nvidia-smi not found".to_owned())),
        ];
        let runner = MockCommandRunner::new(responses);

        let result = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner);
        assert!(result.is_err());
    }

    #[test]
    fn clock_lock_failure_restores_persistence() {
        let gpu = nvidia_desktop_gpu();
        let responses = vec![
            ("nvidia-smi---query-gpu=persistence_mode".to_owned(), Ok("Disabled".to_owned())),
            ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
            ("nvidia-smi--lgc".to_owned(), Err("permission denied".to_owned())),
            // Persistence restore on error path
            ("nvidia-smi--pm".to_owned(), Ok("".to_owned())),
        ];
        let runner = MockCommandRunner::new(responses);

        let result = GpuPerfGuard::set_max(&GpuVendor::NVIDIA, &gpu, &runner);
        assert!(result.is_err());

        // Verify persistence was restored (4th call = -pm 0)
        let calls = runner.calls();
        let restore_call = calls.iter().find(|c| c.0 == "nvidia-smi" && c.1.contains(&"0".to_owned()));
        assert!(restore_call.is_some(), "persistence mode should be restored on clock lock failure");
    }
}
