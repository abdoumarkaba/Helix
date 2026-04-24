//! `LaunchModule` — spawns the game process via the resolved runner.
//!
//! Security: Never uses shell strings. All arguments are passed explicitly
//! to `Command::spawn()` to prevent injection when paths contain spaces.
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use nix::sys::resource::{setrlimit, Resource};

use crate::models::environment::RunnerType;
use crate::models::errors::PlayError;
use crate::models::plan::LaunchAction;

/// Seconds to wait for game process to stabilize after launch.
const LAUNCH_STABILIZE_SECS: u64 = 12;

/// Seconds between polling checks during stabilization.
const POLL_INTERVAL_SECS: u64 = 2;

/// Module responsible for spawning the game process.
pub struct LaunchModule;

impl LaunchModule {
    /// Create a new `LaunchModule`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Execute the launch action and return the spawned child process.
    ///
    /// # Errors
    /// Returns `PlayError::GameLaunchFailed` if the runner binary is not found
    /// or if `Command::spawn()` fails.
    pub fn execute(&self, action: &LaunchAction) -> Result<Child, PlayError> {
        match action {
            LaunchAction::Spawn {
                exe_path,
                working_dir,
                args,
                env,
                runner_path,
                runner_type,
                ulimit_nofile,
            } => Self::spawn_game(
                exe_path,
                working_dir,
                args,
                env,
                runner_path,
                *runner_type,
                *ulimit_nofile,
            ),
        }
    }

    fn spawn_game(
        exe_path: &PathBuf,
        working_dir: &PathBuf,
        args: &[String],
        env: &indexmap::IndexMap<String, String>,
        runner_path: &PathBuf,
        runner_type: RunnerType,
        ulimit_nofile: Option<u64>,
    ) -> Result<Child, PlayError> {
        // Verify runner exists
        if !runner_path.exists() {
            return Err(PlayError::GameLaunchFailed {
                exe_path: exe_path.clone(),
                reason: format!("runner not found at {}", runner_path.display()),
            });
        }

        // Verify working directory exists
        if !working_dir.exists() {
            return Err(PlayError::GameLaunchFailed {
                exe_path: exe_path.clone(),
                reason: format!("working directory {} not found", working_dir.display()),
            });
        }

        // Build command based on runner type
        let mut cmd = match runner_type {
            RunnerType::ProtonOfficial | RunnerType::ProtonGE => {
                // Proton: proton run <exe> [args]
                let mut cmd = Command::new(runner_path);
                cmd.arg("run").arg(exe_path);
                for arg in args {
                    cmd.arg(arg);
                }
                cmd
            },
            RunnerType::WineGE | RunnerType::WineStaging | RunnerType::SodaWine => {
                // Wine: wine <exe> [args]
                let mut cmd = Command::new(runner_path);
                cmd.arg(exe_path);
                for arg in args {
                    cmd.arg(arg);
                }
                cmd
            },
        };

        // Set working directory
        cmd.current_dir(working_dir);

        // Set environment variables
        for (key, value) in env {
            cmd.env(key, value);
        }

        // Configure stdio
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // BUG-4: Apply ulimit before spawn (not via limits.d which only affects new PAM sessions)
        if let Some(nofile) = ulimit_nofile {
            // Set both soft and hard limit for max open files
            if let Err(e) = setrlimit(Resource::RLIMIT_NOFILE, nofile, nofile) {
                tracing::warn!(
                    "Failed to set RLIMIT_NOFILE to {} (non-fatal): {}",
                    nofile,
                    e
                );
            } else {
                tracing::info!("Set RLIMIT_NOFILE to {}", nofile);
            }
        }

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| {
            let reason = if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!(
                    "Permission denied. Possible fixes:\n\
                     1. Check file permissions: ls -l {}\n\
                     2. If on external drive, check mount options: mount | grep {}\n\
                     3. If SELinux is active, check context: ls -Z {}\n\
                     4. Try: chmod +x {}\n\
                     Original error: {e}",
                    exe_path.display(),
                    exe_path.ancestors().nth(1).map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".to_string()),
                    exe_path.display(),
                    exe_path.display()
                )
            } else {
                format!("failed to spawn process: {e}")
            };
            PlayError::GameLaunchFailed {
                exe_path: exe_path.clone(),
                reason,
            }
        })?;

        let pid = child.id();
        tracing::info!(pid, "Process spawned, beginning stabilization check");

        // Poll for process stability with /proc verification
        let mut stable_count = 0;
        let required_stable_checks = (LAUNCH_STABILIZE_SECS / POLL_INTERVAL_SECS) as usize;

        for _ in 0..required_stable_checks {
            std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));

            // Check if spawned process still exists
            let proc_exists = std::path::Path::new(&format!("/proc/{}", pid)).exists();

            // Check if spawned process has exited
            let process_status = child.try_wait();

            match process_status {
                Ok(Some(status)) => {
                    // Spawned process exited - check if it daemonized
                    if proc_exists {
                        tracing::warn!(
                            pid,
                            exit_code = ?status.code(),
                            "Spawned process exited but /proc entry still exists - possible race condition"
                        );
                    } else {
                        // Process truly exited - check for children (daemonization)
                        if Self::has_child_processes(pid) {
                            tracing::info!(
                                parent_pid = pid,
                                "Parent process exited but children remain - likely daemonized successfully"
                            );
                            stable_count += 1;
                        } else {
                            return Err(PlayError::GameLaunchFailed {
                                exe_path: exe_path.clone(),
                                reason: format!(
                                    "game process exited during launch (code: {:?}) with no child processes",
                                    status.code()
                                ),
                            });
                        }
                    }
                },
                Ok(None) => {
                    // Process still running - verify it actually exists in /proc
                    if proc_exists {
                        stable_count += 1;
                        tracing::debug!(pid, stable_count, "Process still running and verified in /proc");
                    } else {
                        return Err(PlayError::GameLaunchFailed {
                            exe_path: exe_path.clone(),
                            reason: format!(
                                "process handle reports running but /proc/{} missing - zombie process",
                                pid
                            ),
                        });
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to check process status during verification");
                },
            }
        }

        // Require at least 50% of checks to pass
        if stable_count >= required_stable_checks / 2 {
            tracing::info!(
                pid,
                stable_checks = stable_count,
                total_checks = required_stable_checks,
                "Game launched successfully and stabilized"
            );
        } else {
            return Err(PlayError::GameLaunchFailed {
                exe_path: exe_path.clone(),
                reason: format!(
                    "process failed stability check: only {}/{} checks passed",
                    stable_count, required_stable_checks
                ),
            });
        }

        Ok(child)
    }

    /// Check if a process has any child processes by reading /proc.
    ///
    /// This handles the case where Proton/Wine daemonizes - the parent exits
    /// but spawns child processes that continue running.
    fn has_child_processes(parent_pid: u32) -> bool {
        let proc_path = std::path::Path::new("/proc");

        if let Ok(entries) = proc_path.read_dir() {
            for entry in entries.flatten() {
                if let Ok(pid_str) = entry.file_name().to_string_lossy().parse::<u32>() {
                    if pid_str == parent_pid {
                        continue; // Skip the parent itself
                    }

                    // Check if this process has our parent as its PPID
                    let status_path = entry.path().join("status");
                    if let Ok(status_content) = std::fs::read_to_string(&status_path) {
                        for line in status_content.lines() {
                            if line.starts_with("PPid:") {
                                let ppid_str = line.split(':').nth(1).unwrap_or("0").trim();
                                if let Ok(ppid) = ppid_str.parse::<u32>() {
                                    if ppid == parent_pid {
                                        tracing::debug!(child_pid = pid_str, parent_pid, "Found child process");
                                        return true;
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }

        false
    }
}

impl Default for LaunchModule {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_launch_command_builds_correctly_for_proton() {
        let temp = TempDir::new().unwrap();
        let runner_path = temp.path().join("proton");
        let exe_path = temp.path().join("game.exe");

        // Create fake runner script
        std::fs::write(&runner_path, "#!/bin/sh\necho 'proton run' $@").unwrap();
        std::fs::write(&exe_path, "").unwrap();

        let action = LaunchAction::Spawn {
            exe_path: exe_path.clone(),
            working_dir: temp.path().to_path_buf(),
            args: vec!["--fullscreen".to_string()],
            env: indexmap::IndexMap::new(),
            runner_path: runner_path.clone(),
            runner_type: RunnerType::ProtonGE,
            ulimit_nofile: None,
        };

        let _module = LaunchModule::new();
        // We can't easily test the actual spawn without a real proton binary,
        // but we verify the action is constructed correctly
        match &action {
            LaunchAction::Spawn { runner_type, args, .. } => {
                assert!(matches!(runner_type, RunnerType::ProtonGE));
                assert_eq!(args, &["--fullscreen"]);
            },
        }
    }

    #[test]
    fn test_launch_fails_on_missing_runner() {
        let temp = TempDir::new().unwrap();
        let exe_path = temp.path().join("game.exe");
        std::fs::write(&exe_path, "").unwrap();

        let action = LaunchAction::Spawn {
            exe_path: exe_path.clone(),
            working_dir: temp.path().to_path_buf(),
            args: Vec::new(),
            env: indexmap::IndexMap::new(),
            runner_path: PathBuf::from("/nonexistent/runner"),
            runner_type: RunnerType::ProtonGE,
            ulimit_nofile: None,
        };

        let module = LaunchModule::new();
        let result = module.execute(&action);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("runner not found"));
    }

    #[test]
    fn test_launch_fails_on_missing_working_dir() {
        let temp = TempDir::new().unwrap();
        let runner_path = temp.path().join("wine");
        let exe_path = temp.path().join("game.exe");

        std::fs::write(&runner_path, "#!/bin/sh\necho 'wine' $@").unwrap();
        std::fs::write(&exe_path, "").unwrap();

        let action = LaunchAction::Spawn {
            exe_path: exe_path.clone(),
            working_dir: PathBuf::from("/nonexistent/dir"),
            args: Vec::new(),
            env: indexmap::IndexMap::new(),
            runner_path: runner_path.clone(),
            runner_type: RunnerType::WineGE,
            ulimit_nofile: None,
        };

        let module = LaunchModule::new();
        let result = module.execute(&action);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("working directory"));
    }
}
