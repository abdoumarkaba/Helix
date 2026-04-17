#![allow(clippy::pedantic)]

//! Session-scoped CPU governor guard.
//!
//! Sets all cores to `performance` on creation, restores previous values on Drop.
//! This is a Class A tweak — no user confirmation needed, automatically restored
//! when the game exits (even on panic).

use std::path::{Path, PathBuf};

use crate::models::errors::PlayError;
use crate::modules::detection::CommandRunner;

// ---------------------------------------------------------------------------
// GovernorGuard
// ---------------------------------------------------------------------------

/// RAII guard that sets CPU scaling governors to `performance` and restores
/// them on Drop. If `cores` is empty, the guard is a no-op (laptop CPU or
/// no cpufreq support).
///
/// Rust guarantees `Drop::drop` runs even on panic. This is not optional.
pub struct GovernorGuard {
    /// (sysfs_path, previous_governor_value) per core that was changed.
    cores: Vec<(PathBuf, String)>,
    /// Injected command runner — used by Drop to restore state.
    cmd_runner: Box<dyn CommandRunner>,
}

impl GovernorGuard {
    /// Set all CPU cores to `performance` governor.
    ///
    /// - `sys_root`: injectable root path (e.g. `/` in production, temp dir in tests).
    /// - `is_laptop_cpu`: if true, returns a no-op guard without touching sysfs.
    /// - `cmd_runner`: injectable command runner. In production, routes through
    ///   `play-helper` for privilege escalation; in tests, use a mock.
    ///
    /// Returns an error only if a sysfs write fails after successfully reading
    /// the previous value (partial application is cleaned up by dropping the
    /// guard on the error path).
    pub fn set_performance(
        sys_root: &Path,
        is_laptop_cpu: bool,
        cmd_runner: Box<dyn CommandRunner>,
    ) -> Result<Self, PlayError> {
        if is_laptop_cpu {
            tracing::info!(
                event = "tweak_skipped",
                tweak = "cpu_governor",
                reason = "laptop CPU detected — skipping to avoid thermal risk"
            );
            return Ok(Self { cores: vec![], cmd_runner });
        }

        let pattern = sys_root
            .join("sys/devices/system/cpu")
            .join("cpu*/cpufreq/scaling_governor");

        let entries: Vec<PathBuf> = glob::glob(pattern.to_string_lossy().as_ref())
            .map_err(|e| PlayError::GovernorWrite {
                core: "glob".to_owned(),
                reason: e.to_string(),
            })?
            .filter_map(Result::ok)
            .collect();

        if entries.is_empty() {
            // No cpufreq support (some containers / VMs). Not an error — no-op.
            tracing::warn!(
                event = "tweak_skipped",
                tweak = "cpu_governor",
                reason = "no cpufreq entries found"
            );
            return Ok(Self { cores: vec![], cmd_runner });
        }

        let mut cores: Vec<(PathBuf, String)> = Vec::with_capacity(entries.len());

        for path in &entries {
            let path_str = path.to_string_lossy();
            let previous = cmd_runner
                .run_command("play-helper", &["sysfs-read", &path_str])
                .map_err(|e| PlayError::GovernorWrite {
                    core: path.display().to_string(),
                    reason: format!("read failed: {e}"),
                })?
                .trim()
                .to_owned();

            cmd_runner
                .run_command("play-helper", &["sysfs-write", &path_str, "performance"])
                .map_err(|e| {
                    // Manual rollback of already-written cores (can't clone Box<dyn CommandRunner>)
                    for (prev_path, prev_val) in cores.iter().rev() {
                        let prev_path_str = prev_path.to_string_lossy();
                        let _ = cmd_runner.run_command(
                            "play-helper",
                            &["sysfs-write", &prev_path_str, prev_val],
                        );
                    }
                    PlayError::GovernorWrite {
                        core: path.display().to_string(),
                        reason: format!("write failed: {e}"),
                    }
                })?;

            cores.push((path.clone(), previous));
        }

        tracing::info!(
            event = "tweak_applied",
            tweak = "cpu_governor",
            value = "performance",
            cores_changed = cores.len()
        );

        Ok(Self { cores, cmd_runner })
    }

    /// Returns true if this guard actually changed any cores (not a no-op).
    pub fn is_active(&self) -> bool {
        !self.cores.is_empty()
    }
}

impl Drop for GovernorGuard {
    fn drop(&mut self) {
        // Restore in reverse order for symmetry
        for (path, previous) in self.cores.iter().rev() {
            let path_str = path.to_string_lossy();
            if let Err(e) =
                self.cmd_runner.run_command("play-helper", &["sysfs-write", &path_str, previous])
            {
                // NEVER panic in Drop — log the error and continue
                tracing::error!(
                    event = "restore_failed",
                    tweak = "cpu_governor",
                    path = %path.display(),
                    error = %e
                );
            }
        }

        if !self.cores.is_empty() {
            tracing::info!(
                event = "tweak_restored",
                tweak = "cpu_governor",
                cores_restored = self.cores.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::panic::AssertUnwindSafe;
    use std::sync::{Arc, Mutex};

    // Need fs accessible for MockCommandRunner sysfs-read/sysfs-write

    // -----------------------------------------------------------------------
    // Mock CommandRunner — simulates play-helper sysfs-read/sysfs-write
    // -----------------------------------------------------------------------

    /// Mock that simulates `play-helper sysfs-read <path>` and
    /// `play-helper sysfs-write <path> <value>` by reading/writing real
    /// tempdir files. This preserves integration semantics while routing
    /// through the CommandRunner interface.
    ///
    /// Uses `Arc<Mutex<...>>` for the call log so it can be cloned into
    /// the guard and inspected after the guard is dropped.
    #[derive(Clone)]
    struct MockCommandRunner {
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }

    impl MockCommandRunner {
        fn new() -> Self {
            Self {
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

            if program != "play-helper" {
                return Err(format!("unexpected program: {program}"));
            }

            match args.first().copied() {
                Some("sysfs-read") => {
                    let path = args.get(1).ok_or("sysfs-read missing path arg")?;
                    fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))
                }
                Some("sysfs-write") => {
                    let path = args.get(1).ok_or("sysfs-write missing path arg")?;
                    let value = args.get(2).ok_or("sysfs-write missing value arg")?;
                    fs::write(path, format!("{value}\n"))
                        .map_err(|e| format!("write failed: {e}"))?;
                    Ok(String::new())
                }
                _ => Err(format!("unexpected play-helper subcommand: {:?}", args)),
            }
        }

        fn clone_boxed(&self) -> Box<dyn CommandRunner> {
            Box::new(self.clone())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn setup_mock_sysfs(dir: &Path, core_count: usize, initial_governor: &str) {
        for i in 0..core_count {
            let cpu_dir = dir.join(format!("sys/devices/system/cpu/cpu{i}/cpufreq"));
            fs::create_dir_all(&cpu_dir).unwrap();
            let gov_path = cpu_dir.join("scaling_governor");
            fs::write(&gov_path, format!("{initial_governor}\n")).unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn set_performance_writes_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 4, "schedutil");
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), false, Box::new(runner)).unwrap();
        assert!(guard.is_active());

        // Verify all cores now say "performance"
        for i in 0..4 {
            let path = dir.path().join(format!(
                "sys/devices/system/cpu/cpu{i}/cpufreq/scaling_governor"
            ));
            assert_eq!(fs::read_to_string(&path).unwrap().trim(), "performance");
        }

        // Drop the guard — should restore via stored mock runner
        drop(guard);

        // Verify cores restored to "schedutil"
        for i in 0..4 {
            let path = dir.path().join(format!(
                "sys/devices/system/cpu/cpu{i}/cpufreq/scaling_governor"
            ));
            assert_eq!(fs::read_to_string(&path).unwrap().trim(), "schedutil");
        }
    }

    #[test]
    fn laptop_cpu_returns_noop_guard() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 4, "schedutil");
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), true, Box::new(runner)).unwrap();
        assert!(!guard.is_active());

        // Cores should remain unchanged
        for i in 0..4 {
            let path = dir.path().join(format!(
                "sys/devices/system/cpu/cpu{i}/cpufreq/scaling_governor"
            ));
            assert_eq!(fs::read_to_string(&path).unwrap().trim(), "schedutil");
        }
    }

    #[test]
    fn drop_restores_on_panic() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 2, "powersave");
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), false, Box::new(runner)).unwrap();

        // Verify guard captured the right previous values
        assert!(guard.is_active());

        // Simulate a panic after guard is created.
        // Drop uses the stored mock runner, so files are actually restored.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = guard;
            panic!("simulated game crash");
        }));

        assert!(result.is_err());

        // Guard should have restored despite panic
        for i in 0..2 {
            let path = dir.path().join(format!(
                "sys/devices/system/cpu/cpu{i}/cpufreq/scaling_governor"
            ));
            assert_eq!(fs::read_to_string(&path).unwrap().trim(), "powersave");
        }
    }

    #[test]
    fn drop_logs_error_on_failed_restore() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 1, "ondemand");
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), false, Box::new(runner)).unwrap();

        // Remove the file between set and drop — restore will fail but must not panic
        let path = dir.path().join("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
        fs::remove_file(&path).unwrap();

        // Drop should not panic (mock runner will get a read error, logged via tracing)
        drop(guard);
    }

    #[test]
    fn no_cpufreq_entries_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        // Create cpu dirs WITHOUT cpufreq subdirectories
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0")).unwrap();
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), false, Box::new(runner)).unwrap();
        assert!(!guard.is_active());
    }

    #[test]
    fn commands_route_through_play_helper() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 2, "ondemand");
        let runner = MockCommandRunner::new();

        let guard =
            GovernorGuard::set_performance(dir.path(), false, Box::new(runner.clone())).unwrap();

        let calls = runner.calls();

        // First call should be sysfs-read for cpu0
        assert_eq!(calls[0].0, "play-helper");
        assert_eq!(calls[0].1[0], "sysfs-read");
        assert!(calls[0].1[1].contains("cpu0"));

        // Second call should be sysfs-write for cpu0
        assert_eq!(calls[1].0, "play-helper");
        assert_eq!(calls[1].1[0], "sysfs-write");
        assert!(calls[1].1[1].contains("cpu0"));
        assert_eq!(calls[1].1[2], "performance");

        // Third call should be sysfs-read for cpu1
        assert_eq!(calls[2].0, "play-helper");
        assert_eq!(calls[2].1[0], "sysfs-read");
        assert!(calls[2].1[1].contains("cpu1"));

        // Fourth call should be sysfs-write for cpu1
        assert_eq!(calls[3].0, "play-helper");
        assert_eq!(calls[3].1[0], "sysfs-write");
        assert!(calls[3].1[1].contains("cpu1"));
        assert_eq!(calls[3].1[2], "performance");

        drop(guard);
    }
}
