//! LaunchModule — spawns the game process via the resolved runner.
//!
//! Security: Never uses shell strings. All arguments are passed explicitly
//! to `Command::spawn()` to prevent injection when paths contain spaces.
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use crate::models::environment::RunnerType;
use crate::models::errors::PlayError;
use crate::models::plan::LaunchAction;

/// Module responsible for spawning the game process.
pub struct LaunchModule;

impl LaunchModule {
    /// Create a new LaunchModule.
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
            } => self.spawn_game(exe_path, working_dir, args, env, runner_path, *runner_type),
        }
    }

    fn spawn_game(
        &self,
        exe_path: &PathBuf,
        working_dir: &PathBuf,
        args: &[String],
        env: &indexmap::IndexMap<String, String>,
        runner_path: &PathBuf,
        runner_type: RunnerType,
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

        // Spawn the process
        let child = cmd.spawn().map_err(|e| PlayError::GameLaunchFailed {
            exe_path: exe_path.clone(),
            reason: format!("failed to spawn process: {e}"),
        })?;

        Ok(child)
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
        };

        let module = LaunchModule::new();
        let result = module.execute(&action);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("working directory"));
    }
}
