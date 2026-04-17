#![allow(clippy::pedantic)]

//! Validation module for the `play` CLI.
//!
//! Verifies game launched successfully, monitors GPU activity, collects MangoHud
//! metrics, and detects crashes. Runs after the Execution phase before the
//! Orchestrator marks the session as validated.
//!
//! # Architecture
//!
//! - `ValidationModule`: Main entry point with injectable dependencies
//! - `LaunchMonitor`: Verifies game process exists via /proc/{pid}
//! - `GpuActivityChecker`: Query GPU utilization (nvidia-smi or sysfs for AMD)
//! - `FpsLogger`: Parse MangoHud log file if MangoHud was enabled
//! - `CrashDetector`: Check if process exited with non-zero code

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::models::environment::GpuVendor;
use crate::models::errors::PlayError;
use crate::modules::detection::CommandRunner;

// ---------------------------------------------------------------------------
// Public Types
// ---------------------------------------------------------------------------

/// Result of validation checks after game launch.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationResult {
    /// Game process is running
    pub process_running: bool,
    /// GPU is being utilized (nvidia-smi or amd stats)
    pub gpu_active: bool,
    /// FPS metrics from MangoHud log (if available)
    pub fps_metrics: Option<FpsMetrics>,
}

/// FPS metrics parsed from MangoHud log output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FpsMetrics {
    /// Average FPS over the session
    pub avg_fps: f32,
    /// Minimum FPS observed
    pub min_fps: f32,
    /// Maximum FPS observed
    pub max_fps: f32,
    /// 1% low FPS (1st percentile)
    pub percentile_1_low: f32,
}

// ---------------------------------------------------------------------------
// ValidationModule
// ---------------------------------------------------------------------------

/// Validates game launch success and monitors runtime metrics.
pub struct ValidationModule {
    /// Injectable proc root for testability
    proc_root: PathBuf,
    /// Injectable command runner
    cmd_runner: Box<dyn CommandRunner>,
    /// MangoHud log path (if enabled)
    mangohud_log_path: Option<PathBuf>,
}

impl ValidationModule {
    /// Create a new ValidationModule with default system paths.
    pub fn new(cmd_runner: Box<dyn CommandRunner>) -> Self {
        Self { proc_root: PathBuf::from("/proc"), cmd_runner, mangohud_log_path: None }
    }

    /// Create with injectable proc root (for testing).
    pub fn with_proc_root(proc_root: PathBuf, cmd_runner: Box<dyn CommandRunner>) -> Self {
        Self { proc_root, cmd_runner, mangohud_log_path: None }
    }

    /// Set the MangoHud log path for FPS metrics collection.
    pub fn with_mangohud_log(mut self, path: PathBuf) -> Self {
        self.mangohud_log_path = Some(path);
        self
    }

    /// Validate that the game is running correctly.
    ///
    /// This performs non-blocking checks:
    /// 1. Verify process exists in /proc
    /// 2. Check GPU activity
    /// 3. Parse MangoHud metrics if available
    ///
    /// Returns a `ValidationResult` with all check results.
    /// Does NOT return Err for soft failures (graceful degradation).
    pub fn validate(&self, game_process: &mut Child, gpu_vendor: GpuVendor) -> ValidationResult {
        let pid = game_process.id();

        // Check 1: Process running
        let process_running = self.check_process_running(pid);

        // Check 2: GPU activity (vendor-aware)
        let gpu_active = self.check_gpu_activity(gpu_vendor, pid);

        // Check 3: MangoHud FPS metrics
        let fps_metrics = self.mangohud_log_path.as_ref().and_then(|path| {
            match Self::parse_mangohud_log(path) {
                Ok(metrics) => {
                    info!(event = "mangohud_parsed", avg_fps = metrics.avg_fps, "Parsed MangoHud log");
                    Some(metrics)
                }
                Err(e) => {
                    warn!(event = "mangohud_parse_failed", error = %e, "Failed to parse MangoHud log");
                    None
                }
            }
        });

        ValidationResult { process_running, gpu_active, fps_metrics }
    }

    /// Quick validation that only checks process status.
    /// Used for lightweight health checks.
    pub fn validate_process_only(&self, game_process: &mut Child) -> ValidationResult {
        let pid = game_process.id();

        ValidationResult {
            process_running: self.check_process_running(pid),
            gpu_active: false,
            fps_metrics: None,
        }
    }

    /// Check if a process exists in /proc by PID.
    fn check_process_running(&self, pid: u32) -> bool {
        let proc_path = self.proc_root.join(pid.to_string());
        proc_path.exists()
    }

    /// Check GPU activity for the given process.
    ///
    /// Vendor-aware implementation:
    /// - NVIDIA: Uses nvidia-smi pmon to check GPU utilization
    /// - AMD: Reads from sysfs for GPU busy percentage
    /// - Intel/Unknown: Returns false (no reliable check)
    fn check_gpu_activity(&self, vendor: GpuVendor, pid: u32) -> bool {
        match vendor {
            GpuVendor::NVIDIA => self.check_nvidia_gpu_activity(pid),
            GpuVendor::AMD => self.check_amd_gpu_activity(),
            _ => {
                info!(event = "gpu_check_skipped", vendor = ?vendor, "GPU activity check not supported for vendor");
                false
            },
        }
    }

    /// Check NVIDIA GPU activity using nvidia-smi.
    fn check_nvidia_gpu_activity(&self, pid: u32) -> bool {
        // Try nvidia-smi pmon to see if our process is using GPU
        let args = ["pmon", "-s", "um", "-c", "1"];
        match self.cmd_runner.run_command("nvidia-smi", &args) {
            Ok(output) => {
                // Parse pmon output looking for our PID
                for line in output.lines().skip(2) {
                    // Skip header lines
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        // Check if any column matches our PID
                        for part in &parts {
                            if let Ok(found_pid) = part.parse::<u32>() {
                                if found_pid == pid {
                                    info!(
                                        event = "nvidia_gpu_active",
                                        pid, "NVIDIA GPU active for process"
                                    );
                                    return true;
                                }
                            }
                        }
                    }
                }
                false
            },
            Err(e) => {
                warn!(event = "nvidia_smi_failed", error = %e, "Failed to query NVIDIA GPU");
                false
            },
        }
    }

    /// Check AMD GPU activity using sysfs.
    fn check_amd_gpu_activity(&self) -> bool {
        // Try to read GPU busy percent from sysfs
        // Common paths: /sys/class/drm/card0/device/gpu_busy_percent
        let card_paths = ["/sys/class/drm/card0/device", "/sys/class/drm/card1/device"];

        for card_path in &card_paths {
            let busy_path = Path::new(card_path).join("gpu_busy_percent");
            if let Ok(content) = fs::read_to_string(&busy_path) {
                if let Ok(busy_percent) = content.trim().parse::<u32>() {
                    let active = busy_percent > 0;
                    if active {
                        info!(event = "amd_gpu_active", percent = busy_percent, "AMD GPU active");
                    }
                    return active;
                }
            }
        }

        // Fallback: try rocm-smi
        match self.cmd_runner.run_command("rocm-smi", &["--showuse"]) {
            Ok(output) => {
                for line in output.lines() {
                    if let Some(percent_str) =
                        line.split('%').next().and_then(|s| s.split_whitespace().last())
                    {
                        if let Ok(percent) = percent_str.parse::<u32>() {
                            if percent > 0 {
                                info!(
                                    event = "amd_gpu_active_rocm",
                                    percent, "AMD GPU active (rocm-smi)"
                                );
                                return true;
                            }
                        }
                    }
                }
                false
            },
            Err(_) => {
                warn!(event = "amd_gpu_check_failed", "Could not check AMD GPU activity");
                false
            },
        }
    }

    /// Parse MangoHud log file for FPS metrics.
    ///
    /// Expected log format (CSV-like):
    /// ```text
    /// fps,frametime
    /// 60.0,16.67
    /// 58.5,17.09
    /// ...
    /// ```
    fn parse_mangohud_log(path: &Path) -> Result<FpsMetrics, PlayError> {
        let content = fs::read_to_string(path).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to read MangoHud log: {e}"),
        })?;

        let mut fps_values: Vec<f32> = Vec::new();

        for line in content.lines().skip(1) {
            // Skip header
            let parts: Vec<&str> = line.split(',').collect();
            if let Some(fps_str) = parts.first() {
                if let Ok(fps) = fps_str.trim().parse::<f32>() {
                    if fps > 0.0 && fps.is_finite() {
                        fps_values.push(fps);
                    }
                }
            }
        }

        if fps_values.is_empty() {
            return Err(PlayError::ValidationFailed {
                reason: "No valid FPS data in MangoHud log".to_string(),
            });
        }

        // Calculate metrics
        fps_values.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min_fps = *fps_values.first().unwrap();
        let max_fps = *fps_values.last().unwrap();
        let avg_fps = fps_values.iter().sum::<f32>() / fps_values.len() as f32;

        // Calculate 1% low (1st percentile)
        let percentile_1_idx = (fps_values.len() as f32 * 0.01) as usize;
        let percentile_1_low = fps_values.get(percentile_1_idx).copied().unwrap_or(min_fps);

        Ok(FpsMetrics { avg_fps, min_fps, max_fps, percentile_1_low })
    }
}

// ---------------------------------------------------------------------------
// CrashDetector
// ---------------------------------------------------------------------------

/// Detects if a game process crashed or exited abnormally.
pub struct CrashDetector;

impl CrashDetector {
    /// Check if the process crashed based on exit status.
    pub fn check_exit_status(game_process: &mut Child) -> Result<(), PlayError> {
        match game_process.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    info!(event = "process_exit_clean", "Game process exited cleanly");
                    Ok(())
                } else {
                    let code = status.code().unwrap_or(-1);
                    warn!(
                        event = "process_exit_error",
                        exit_code = code,
                        "Game process exited with error"
                    );
                    Err(PlayError::ValidationFailed {
                        reason: format!("Game process exited with code {code}"),
                    })
                }
            },
            Ok(None) => {
                // Process still running - this is expected during validation
                Ok(())
            },
            Err(e) => {
                warn!(event = "wait_failed", error = %e, "Failed to check process status");
                Err(PlayError::ValidationFailed {
                    reason: format!("Failed to check process status: {e}"),
                })
            },
        }
    }

    /// Wait for process to exit and check exit code.
    /// Use this for post-game crash detection.
    pub fn wait_and_check(game_process: &mut Child) -> Result<(), PlayError> {
        match game_process.wait() {
            Ok(status) => {
                if status.success() {
                    info!(event = "process_exit_clean", "Game process exited cleanly");
                    Ok(())
                } else {
                    let code = status.code().unwrap_or(-1);
                    warn!(
                        event = "process_exit_error",
                        exit_code = code,
                        "Game process exited with error"
                    );
                    Err(PlayError::ValidationFailed {
                        reason: format!("Game process exited with code {code}"),
                    })
                }
            },
            Err(e) => Err(PlayError::ValidationFailed {
                reason: format!("Failed to wait for process: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    /// Mock command runner for tests
    #[derive(Clone)]
    struct MockCommandRunner {
        responses: HashMap<String, Result<String, String>>,
    }

    impl MockCommandRunner {
        fn new() -> Self {
            Self { responses: HashMap::new() }
        }

        fn with_response(mut self, cmd: &str, response: Result<String, String>) -> Self {
            self.responses.insert(cmd.to_string(), response);
            self
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run_command(&self, program: &str, _args: &[&str]) -> Result<String, String> {
            self.responses
                .get(program)
                .cloned()
                .unwrap_or(Err(format!("Command not mocked: {program}")))
        }

        fn clone_boxed(&self) -> Box<dyn CommandRunner> {
            Box::new(self.clone())
        }
    }

    #[test]
    fn test_parse_mangohud_log_valid() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("mangohud.csv");

        let log_content = "fps,frametime\n60.0,16.67\n58.5,17.09\n62.3,16.05\n55.0,18.18\n";
        fs::write(&log_path, log_content).unwrap();

        let metrics = ValidationModule::parse_mangohud_log(&log_path).unwrap();

        assert!((metrics.avg_fps - 58.95).abs() < 0.01);
        assert!((metrics.min_fps - 55.0).abs() < 0.01);
        assert!((metrics.max_fps - 62.3).abs() < 0.01);
    }

    #[test]
    fn test_parse_mangohud_log_empty() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("mangohud.csv");

        fs::write(&log_path, "fps,frametime\n").unwrap();

        let result = ValidationModule::parse_mangohud_log(&log_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_mangohud_log_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("nonexistent.csv");

        let result = ValidationModule::parse_mangohud_log(&log_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_mangohud_log_large_dataset() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("mangohud.csv");

        // Generate 100 FPS values from 30 to 129
        let mut content = String::from("fps,frametime\n");
        for i in 0..100 {
            let i_f32 = i as f32;
            content.push_str(&format!("{},{}\n", 30.0 + i_f32, 1000.0 / (30.0 + i_f32)));
        }
        fs::write(&log_path, content).unwrap();

        let metrics = ValidationModule::parse_mangohud_log(&log_path).unwrap();

        assert!((metrics.avg_fps - 79.5).abs() < 1.0); // Average of 30-129
        assert!((metrics.min_fps - 30.0).abs() < 0.01);
        assert!((metrics.max_fps - 129.0).abs() < 0.01);
        // 1% low should be around 30 (first percentile of sorted values)
        assert!(metrics.percentile_1_low >= 30.0);
        assert!(metrics.percentile_1_low <= 35.0);
    }

    #[test]
    fn test_check_process_running_with_mock_proc() {
        let temp_dir = TempDir::new().unwrap();
        let proc_dir = temp_dir.path().join("1234");
        fs::create_dir(&proc_dir).unwrap();

        let runner = MockCommandRunner::new();
        let module =
            ValidationModule::with_proc_root(temp_dir.path().to_path_buf(), Box::new(runner));

        assert!(module.check_process_running(1234));
        assert!(!module.check_process_running(9999));
    }

    #[test]
    fn test_nvidia_gpu_activity_detection() {
        // Mock nvidia-smi output with our PID present
        let nvidia_output = Ok("# gpu        pid  type    sm   mem   enc   dec   command\n"
            .to_string()
            + "# Idx        #   C/G     %    %     %     %     name\n"
            + "    0      1234     C    45    30     0     0     game.exe\n"
            + "    0      5678     G    10     5     0     0     Xorg\n");

        let runner = MockCommandRunner::new().with_response("nvidia-smi", nvidia_output);
        let module = ValidationModule::with_proc_root(PathBuf::from("/proc"), Box::new(runner));

        assert!(module.check_nvidia_gpu_activity(1234));
        assert!(!module.check_nvidia_gpu_activity(9999));
    }

    #[test]
    fn test_nvidia_gpu_activity_no_match() {
        // Mock nvidia-smi output without our PID
        let nvidia_output = Ok("# gpu        pid  type    sm   mem   enc   dec   command\n"
            .to_string()
            + "    0      5678     G    10     5     0     0     Xorg\n");

        let runner = MockCommandRunner::new().with_response("nvidia-smi", nvidia_output);
        let module = ValidationModule::with_proc_root(PathBuf::from("/proc"), Box::new(runner));

        assert!(!module.check_nvidia_gpu_activity(1234));
    }

    #[test]
    fn test_nvidia_gpu_activity_command_failure() {
        let runner = MockCommandRunner::new()
            .with_response("nvidia-smi", Err("nvidia-smi not found".to_string()));
        let module = ValidationModule::with_proc_root(PathBuf::from("/proc"), Box::new(runner));

        // Should gracefully return false on command failure
        assert!(!module.check_nvidia_gpu_activity(1234));
    }

    #[test]
    fn test_validation_result_default() {
        let result =
            ValidationResult { process_running: true, gpu_active: false, fps_metrics: None };

        assert!(result.process_running);
        assert!(!result.gpu_active);
        assert!(result.fps_metrics.is_none());
    }

    #[test]
    fn test_fps_metrics_equality() {
        let m1 =
            FpsMetrics { avg_fps: 60.0, min_fps: 30.0, max_fps: 120.0, percentile_1_low: 45.0 };
        let m2 =
            FpsMetrics { avg_fps: 60.0, min_fps: 30.0, max_fps: 120.0, percentile_1_low: 45.0 };
        let m3 =
            FpsMetrics { avg_fps: 61.0, min_fps: 30.0, max_fps: 120.0, percentile_1_low: 45.0 };

        assert_eq!(m1, m2);
        assert_ne!(m1, m3);
    }

    #[test]
    fn test_crash_detector_with_true_child() {
        // Spawn a simple command that exits successfully
        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", "exit 0"]).stdout(Stdio::null()).spawn().unwrap()
        } else {
            Command::new("true").stdout(Stdio::null()).spawn().unwrap()
        };

        // Wait for the process to actually exit
        std::thread::sleep(std::time::Duration::from_millis(100));

        let result = CrashDetector::check_exit_status(&mut child);
        assert!(result.is_ok());
    }

    #[test]
    fn test_crash_detector_with_false_child() {
        // Spawn a command that exits with error
        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", "exit 1"]).stdout(Stdio::null()).spawn().unwrap()
        } else {
            Command::new("false").stdout(Stdio::null()).spawn().unwrap()
        };

        // Wait for the process to actually exit
        std::thread::sleep(std::time::Duration::from_millis(100));

        let result = CrashDetector::check_exit_status(&mut child);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_gpu_activity_intel_skipped() {
        let runner = MockCommandRunner::new();
        let module = ValidationModule::with_proc_root(PathBuf::from("/proc"), Box::new(runner));

        // Intel GPU check should return false (not supported)
        assert!(!module.check_gpu_activity(GpuVendor::Intel, 1234));
    }

    #[test]
    fn test_check_gpu_activity_unknown_skipped() {
        let runner = MockCommandRunner::new();
        let module = ValidationModule::with_proc_root(PathBuf::from("/proc"), Box::new(runner));

        // Unknown GPU check should return false (not supported)
        assert!(!module.check_gpu_activity(GpuVendor::Unknown, 1234));
    }

    #[test]
    fn test_validation_module_builder() {
        let runner = MockCommandRunner::new();
        let module = ValidationModule::new(Box::new(runner))
            .with_mangohud_log(PathBuf::from("/tmp/mangohud.csv"));

        assert!(module.mangohud_log_path.is_some());
        assert_eq!(module.mangohud_log_path.unwrap(), PathBuf::from("/tmp/mangohud.csv"));
    }
}
