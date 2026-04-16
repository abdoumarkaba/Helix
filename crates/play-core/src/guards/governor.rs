#![allow(clippy::pedantic)]

//! Session-scoped CPU governor guard.
//!
//! Sets all cores to `performance` on creation, restores previous values on Drop.
//! This is a Class A tweak — no user confirmation needed, automatically restored
//! when the game exits (even on panic).

use std::fs;
use std::path::{Path, PathBuf};

use crate::models::errors::PlayError;

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
}

impl GovernorGuard {
    /// Set all CPU cores to `performance` governor.
    ///
    /// - `sys_root`: injectable root path (e.g. `/` in production, temp dir in tests).
    /// - `is_laptop_cpu`: if true, returns a no-op guard without touching sysfs.
    ///
    /// Returns an error only if a sysfs write fails after successfully reading
    /// the previous value (partial application is cleaned up by dropping the
    /// guard on the error path).
    pub fn set_performance(sys_root: &Path, is_laptop_cpu: bool) -> Result<Self, PlayError> {
        if is_laptop_cpu {
            tracing::info!(
                event = "tweak_skipped",
                tweak = "cpu_governor",
                reason = "laptop CPU detected — skipping to avoid thermal risk"
            );
            return Ok(Self { cores: vec![] });
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
            return Ok(Self { cores: vec![] });
        }

        let mut cores: Vec<(PathBuf, String)> = Vec::with_capacity(entries.len());

        for path in &entries {
            let previous = fs::read_to_string(path)
                .map_err(|e| PlayError::GovernorWrite {
                    core: path.display().to_string(),
                    reason: format!("read failed: {e}"),
                })?
                .trim()
                .to_owned();

            fs::write(path, "performance\n").map_err(|e| {
                // Partial application — drop what we've already changed
                let partial = Self { cores: std::mem::take(&mut cores) };
                // Drop runs and restores the partial changes
                drop(partial);
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

        Ok(Self { cores })
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
            if let Err(e) = fs::write(path, format!("{previous}\n")) {
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
    use std::panic::AssertUnwindSafe;

    fn setup_mock_sysfs(dir: &Path, core_count: usize, initial_governor: &str) {
        for i in 0..core_count {
            let cpu_dir = dir.join(format!("sys/devices/system/cpu/cpu{i}/cpufreq"));
            fs::create_dir_all(&cpu_dir).unwrap();
            let gov_path = cpu_dir.join("scaling_governor");
            fs::write(&gov_path, format!("{initial_governor}\n")).unwrap();
        }
    }

    #[test]
    fn set_performance_writes_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        setup_mock_sysfs(dir.path(), 4, "schedutil");

        let guard = GovernorGuard::set_performance(dir.path(), false).unwrap();
        assert!(guard.is_active());

        // Verify all cores now say "performance"
        for i in 0..4 {
            let path = dir.path().join(format!(
                "sys/devices/system/cpu/cpu{i}/cpufreq/scaling_governor"
            ));
            assert_eq!(fs::read_to_string(&path).unwrap().trim(), "performance");
        }

        // Drop the guard — should restore
        drop(guard);

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

        let guard = GovernorGuard::set_performance(dir.path(), true).unwrap();
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

        let guard = GovernorGuard::set_performance(dir.path(), false).unwrap();

        // Simulate a panic after guard is created
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = guard; // guard moved into closure
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

        let guard = GovernorGuard::set_performance(dir.path(), false).unwrap();

        // Remove the file between set and drop — restore will fail but must not panic
        let path = dir.path().join("sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
        fs::remove_file(&path).unwrap();

        // Drop should not panic
        drop(guard);
    }

    #[test]
    fn no_cpufreq_entries_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        // Create cpu dirs WITHOUT cpufreq subdirectories
        fs::create_dir_all(dir.path().join("sys/devices/system/cpu/cpu0")).unwrap();

        let guard = GovernorGuard::set_performance(dir.path(), false).unwrap();
        assert!(!guard.is_active());
    }
}
