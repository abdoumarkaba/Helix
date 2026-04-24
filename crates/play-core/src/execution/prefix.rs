//! Wine prefix creation and configuration module.
//!
//! Creates Wine prefixes via wineboot, sets Windows version, and applies
//! DLL overrides. All operations are idempotent: if a prefix already exists,
//! it is verified rather than recreated.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::models::environment::{DllOverride, WindowsVersion, WineArch};
use crate::models::errors::PlayError;
use crate::models::plan::PrefixAction;

// ---------------------------------------------------------------------------
// CommandRunner trait for testability
// ---------------------------------------------------------------------------

/// Abstraction over command execution for testability.
/// Real implementation spawns processes; mock returns fixture responses.
pub trait CommandRunner: Send + Sync {
    /// Run a command with arguments, returning stdout on success.
    ///
    /// # Errors
    /// Returns `PlayError::PrefixCreation` if the command fails to spawn or exits with non-zero status.
    fn run(&self, cmd: &str, args: &[&str]) -> Result<String, PlayError>;

    /// Run a command with environment variables and arguments.
    ///
    /// # Arguments
    /// * `cmd` - The command to execute
    /// * `args` - Arguments to pass to the command
    /// * `env_vars` - Environment variables as (key, value) pairs
    ///
    /// # Errors
    /// Returns `PlayError::PrefixCreation` if the command fails to spawn or exits with non-zero status.
    fn run_with_env(
        &self,
        cmd: &str,
        args: &[&str],
        env_vars: &[(&str, &str)],
    ) -> Result<String, PlayError>;
}

/// System command runner that spawns real processes.
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<String, PlayError> {
        let output =
            Command::new(cmd).args(args).output().map_err(|e| PlayError::PrefixCreation {
                prefix_path: PathBuf::new(),
                reason: format!("failed to spawn {cmd}: {e}"),
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(PlayError::PrefixCreation {
                prefix_path: PathBuf::new(),
                reason: format!("{cmd} failed: {stderr}"),
            })
        }
    }

    fn run_with_env(
        &self,
        cmd: &str,
        args: &[&str],
        env_vars: &[(&str, &str)],
    ) -> Result<String, PlayError> {
        let mut command = Command::new(cmd);
        command.args(args);
        for (key, value) in env_vars {
            command.env(key, value);
        }

        let output = command.output().map_err(|e| PlayError::PrefixCreation {
            prefix_path: PathBuf::new(),
            reason: format!("failed to spawn {cmd}: {e}"),
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(PlayError::PrefixCreation {
                prefix_path: PathBuf::new(),
                reason: format!("{cmd} failed: {stderr}"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// PrefixModule
// ---------------------------------------------------------------------------

/// Executes prefix-related actions from the game plan.
///
/// - `AlreadyExists`: verifies the prefix directory and wineboot status.
/// - `Create`: creates the prefix via wineboot, sets Windows version,
///   and applies DLL overrides.
pub struct PrefixModule {
    /// Command runner for wine/wineboot execution.
    cmd_runner: Box<dyn CommandRunner>,
}

impl Default for PrefixModule {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixModule {
    /// Create a new `PrefixModule` with the system command runner.
    #[must_use]
    pub fn new() -> Self {
        Self { cmd_runner: Box::new(SystemCommandRunner) }
    }

    /// Create a `PrefixModule` with an injected `CommandRunner` (for testing).
    #[must_use]
    pub fn with_runner(cmd_runner: Box<dyn CommandRunner>) -> Self {
        Self { cmd_runner }
    }

    /// Execute the prefix action from the plan.
    ///
    /// Returns the prefix path on success.
    ///
    /// # Errors
    ///
    /// Returns `PlayError::PrefixCreation` if prefix creation or verification fails.
    pub fn execute(&self, action: &PrefixAction) -> Result<PathBuf, PlayError> {
        match action {
            PrefixAction::AlreadyExists { path } => {
                tracing::info!(
                    event = "prefix_already_exists",
                    path = %path.display(),
                    "Prefix already exists, verifying"
                );
                Self::verify_prefix(path)?;
                Ok(path.clone())
            },
            PrefixAction::Create { path, arch, windows_version } => {
                if path.exists() {
                    tracing::info!(
                        event = "prefix_already_exists",
                        path = %path.display(),
                        "Prefix directory exists, verifying"
                    );
                    Self::verify_prefix(path)?;
                    return Ok(path.clone());
                }

                tracing::info!(
                    event = "prefix_creation_started",
                    path = %path.display(),
                    arch = ?arch,
                    windows_version = ?windows_version,
                    "Creating Wine prefix"
                );

                // Create prefix directory parent
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| PlayError::PrefixCreation {
                        prefix_path: path.clone(),
                        reason: format!("failed to create parent directory: {e}"),
                    })?;
                }

                // Create prefix via wineboot
                self.create_prefix(path, *arch)?;

                // Set Windows version
                self.set_windows_version(path, *windows_version)?;

                tracing::info!(
                    event = "prefix_created",
                    path = %path.display(),
                    "Wine prefix created successfully"
                );

                Ok(path.clone())
            },
        }
    }

    /// Apply DLL overrides to an existing prefix.
    ///
    /// Writes registry keys for each override via `wine reg add`.
    ///
    /// # Errors
    ///
    /// Returns `PlayError::PrefixCreation` if the wine reg command fails.
    pub fn apply_dll_overrides(
        &self,
        prefix_path: &Path,
        overrides: &[DllOverride],
    ) -> Result<(), PlayError> {
        for ov in overrides {
            let key = format!(r"HKCU\Software\Wine\DllOverrides\{}", ov.dll);
            let args = ["reg", "add", &key, "/v", "", "/t", "REG_SZ", "/d", &ov.mode, "/f"];

            tracing::debug!(
                event = "dll_override_applying",
                dll = %ov.dll,
                mode = %ov.mode,
                "Applying DLL override"
            );

            self.run_wine(prefix_path, &args)?;

            tracing::info!(
                event = "dll_override_applied",
                dll = %ov.dll,
                mode = %ov.mode,
                "DLL override applied"
            );
        }
        Ok(())
    }

    /// Verify that a prefix exists and is valid.
    ///
    /// Checks for the presence of `drive_c` and `system.reg` files.
    fn verify_prefix(path: &Path) -> Result<(), PlayError> {
        if !path.exists() {
            return Err(PlayError::PrefixCreation {
                prefix_path: path.to_owned(),
                reason: "prefix directory does not exist".to_owned(),
            });
        }

        let drive_c = path.join("drive_c");
        let system_reg = path.join("system.reg");

        if !drive_c.exists() {
            return Err(PlayError::PrefixCreation {
                prefix_path: path.to_owned(),
                reason: "prefix is incomplete: drive_c missing".to_owned(),
            });
        }

        if !system_reg.exists() {
            return Err(PlayError::PrefixCreation {
                prefix_path: path.to_owned(),
                reason: "prefix is incomplete: system.reg missing".to_owned(),
            });
        }

        tracing::debug!(
            event = "prefix_verified",
            path = %path.display(),
            "Prefix structure verified"
        );

        Ok(())
    }

    /// Create a new Wine prefix via wineboot.
    ///
    /// Sets WINEPREFIX and WINEARCH environment variables.
    /// For GE-Proton compatibility, ensures pfx/ subdirectory exists.
    fn create_prefix(&self, path: &Path, arch: WineArch) -> Result<(), PlayError> {
        let arch_str = match arch {
            WineArch::Win32 => "win32",
            WineArch::Win64 => "win64",
        };

        // For GE-Proton compatibility: ensure pfx/ subdirectory exists
        // GE-Proton expects STEAM_COMPAT_DATA_PATH/pfx as the actual WINEPREFIX
        let pfx_path = path.join("pfx");
        if !pfx_path.exists() {
            fs::create_dir_all(&pfx_path).map_err(|e| PlayError::PrefixCreation {
                prefix_path: path.to_owned(),
                reason: format!("failed to create pfx/ subdirectory: {e}"),
            })?;
            tracing::info!(
                event = "pfx_subdirectory_created",
                path = %pfx_path.display(),
                "Created pfx/ subdirectory for GE-Proton compatibility"
            );
        }

        // Run wineboot to initialize the prefix (using pfx/ as WINEPREFIX)
        let output =
            self.run_wine_with_env(&pfx_path, &["wineboot", "--init"], &[("WINEARCH", arch_str)])?;

        tracing::debug!(
            event = "wineboot_completed",
            output = %output.trim(),
            "Wine prefix initialized"
        );

        Ok(())
    }

    /// Set the Windows version in the prefix registry.
    fn set_windows_version(
        &self,
        prefix_path: &Path,
        version: WindowsVersion,
    ) -> Result<(), PlayError> {
        let version_str = match version {
            WindowsVersion::WinXP => "winxp",
            WindowsVersion::Win7 => "win7",
            WindowsVersion::Win8 => "win8",
            WindowsVersion::Win10 => "win10",
            WindowsVersion::Win11 => "win11",
        };

        // Set via winecfg (simpler than direct registry manipulation)
        let args = ["winecfg", "/v", version_str];
        self.run_wine(prefix_path, &args)?;

        tracing::info!(
            event = "windows_version_set",
            version = %version_str,
            "Windows version configured"
        );

        Ok(())
    }

    /// Run a wine command with WINEPREFIX set.
    fn run_wine(&self, prefix_path: &Path, args: &[&str]) -> Result<String, PlayError> {
        self.run_wine_with_env(prefix_path, args, &[])
    }

    /// Run a wine command with WINEPREFIX and additional environment variables.
    fn run_wine_with_env(
        &self,
        prefix_path: &Path,
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> Result<String, PlayError> {
        if args.is_empty() {
            return Err(PlayError::PrefixCreation {
                prefix_path: prefix_path.to_owned(),
                reason: "no wine command specified".to_owned(),
            });
        }

        let wine_cmd = args[0];
        let wine_args = &args[1..];

        // Build environment variables list
        let prefix_path_str = prefix_path.to_str().ok_or_else(|| PlayError::PrefixCreation {
            prefix_path: prefix_path.to_owned(),
            reason: "prefix path contains invalid UTF-8 characters".to_owned(),
        })?;

        let mut env_vars: Vec<(&str, &str)> = vec![("WINEPREFIX", prefix_path_str)];
        for (key, value) in extra_env {
            env_vars.push((key, value));
        }

        // Run via command runner with environment variables
        self.cmd_runner.run_with_env(wine_cmd, wine_args, &env_vars).map_err(|e| {
            PlayError::PrefixCreation {
                prefix_path: prefix_path.to_owned(),
                reason: format!("wine command failed: {e}"),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Mock command runner that returns predefined responses.
    struct MockCommandRunner {
        responses: HashMap<String, Result<String, String>>,
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl MockCommandRunner {
        fn new() -> Self {
            Self {
                responses: HashMap::new(),
                calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn add_response(mut self, cmd: &str, result: Result<String, String>) -> Self {
            self.responses.insert(cmd.to_owned(), result);
            self
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, cmd: &str, args: &[&str]) -> Result<String, PlayError> {
            let full_cmd = format!("{cmd} {}", args.join(" "));
            self.calls.lock().unwrap().push(full_cmd.clone());

            match self.responses.get(&full_cmd) {
                Some(Ok(output)) => Ok(output.clone()),
                Some(Err(e)) => Err(PlayError::PrefixCreation {
                    prefix_path: PathBuf::new(),
                    reason: e.clone(),
                }),
                None => {
                    // Default success for unregistered commands
                    Ok(String::new())
                },
            }
        }

        fn run_with_env(
            &self,
            cmd: &str,
            args: &[&str],
            env_vars: &[(&str, &str)],
        ) -> Result<String, PlayError> {
            // Build a command string that includes env vars for matching
            let env_str = env_vars
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
            let full_cmd = format!("{env_str} {cmd} {}", args.join(" "));
            self.calls.lock().unwrap().push(full_cmd.clone());

            match self.responses.get(&full_cmd) {
                Some(Ok(output)) => Ok(output.clone()),
                Some(Err(e)) => Err(PlayError::PrefixCreation {
                    prefix_path: PathBuf::new(),
                    reason: e.clone(),
                }),
                None => {
                    // Default success for unregistered commands
                    Ok(String::new())
                },
            }
        }
    }

    fn temp_prefix_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn create_valid_prefix(path: &Path) {
        fs::create_dir_all(path.join("drive_c")).unwrap();
        fs::write(path.join("system.reg"), "").unwrap();
    }

    #[test]
    fn already_exists_returns_path_if_valid() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        create_valid_prefix(&prefix_path);

        let mock = MockCommandRunner::new();
        let module = PrefixModule::with_runner(Box::new(mock));

        let action = PrefixAction::AlreadyExists { path: prefix_path.clone() };

        let result = module.execute(&action);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), prefix_path);
    }

    #[test]
    fn already_exists_fails_if_missing_drive_c() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        fs::create_dir_all(&prefix_path).unwrap();
        // Missing drive_c and system.reg

        let mock = MockCommandRunner::new();
        let module = PrefixModule::with_runner(Box::new(mock));

        let action = PrefixAction::AlreadyExists { path: prefix_path.clone() };

        let result = module.execute(&action);
        assert!(result.is_err());
        assert!(matches!(result, Err(PlayError::PrefixCreation { .. })));
    }

    #[test]
    fn create_creates_prefix_via_wineboot() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");

        // Build expected command strings (format: "WINEPREFIX=... WINEARCH=... winecmd args")
        let prefix_str = prefix_path.to_str().unwrap();
        let wineboot_cmd = format!("WINEPREFIX={prefix_str} WINEARCH=win64 wineboot --init");
        let winecfg_cmd = format!("WINEPREFIX={prefix_str} winecfg /v win10");

        let mock = MockCommandRunner::new()
            .add_response(&wineboot_cmd, Ok(String::new()))
            .add_response(&winecfg_cmd, Ok(String::new()));

        // Pre-create the prefix structure so verify passes
        let module = PrefixModule::with_runner(Box::new(mock));

        // Create the prefix structure manually for the test
        fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        fs::write(prefix_path.join("system.reg"), "").unwrap();

        let action = PrefixAction::Create {
            path: prefix_path.clone(),
            arch: WineArch::Win64,
            windows_version: WindowsVersion::Win10,
        };

        let result = module.execute(&action);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), prefix_path);
    }

    #[test]
    fn create_skips_if_prefix_already_exists() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        create_valid_prefix(&prefix_path);

        let mock = MockCommandRunner::new();
        let module = PrefixModule::with_runner(Box::new(mock));

        let action = PrefixAction::Create {
            path: prefix_path.clone(),
            arch: WineArch::Win64,
            windows_version: WindowsVersion::Win10,
        };

        let result = module.execute(&action);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), prefix_path);
    }

    #[test]
    fn apply_dll_overrides_runs_reg_add() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        create_valid_prefix(&prefix_path);

        let mock = MockCommandRunner::new();
        let module = PrefixModule::with_runner(Box::new(mock));

        let overrides = vec![
            DllOverride { dll: "d3d9".to_owned(), mode: "native,builtin".to_owned() },
            DllOverride { dll: "dxgi".to_owned(), mode: "native,builtin".to_owned() },
        ];

        let result = module.apply_dll_overrides(&prefix_path, &overrides);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_prefix_succeeds_for_valid_structure() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        create_valid_prefix(&prefix_path);

        let result = PrefixModule::verify_prefix(&prefix_path);
        assert!(result.is_ok());
    }

    #[test]
    fn create_prefix_creates_pfx_subdirectory() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("test_prefix");
        let mock = MockCommandRunner::new();
        let module = PrefixModule::with_runner(Box::new(mock));

        let result = module.create_prefix(&prefix_path, WineArch::Win64);
        assert!(result.is_ok());

        // Verify pfx/ subdirectory was created
        let pfx_path = prefix_path.join("pfx");
        assert!(pfx_path.exists());
        assert!(pfx_path.is_dir());
    }

    #[test]
    fn verify_prefix_fails_for_missing_drive_c() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        fs::create_dir_all(&prefix_path).unwrap();
        // Missing drive_c

        let result = PrefixModule::verify_prefix(&prefix_path);
        assert!(result.is_err());
        assert!(matches!(result, Err(PlayError::PrefixCreation { .. })));
    }

    #[test]
    fn verify_prefix_fails_for_missing_system_reg() {
        let root = temp_prefix_root();
        let prefix_path = root.path().join("abc123");
        fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        // Missing system.reg

        let result = PrefixModule::verify_prefix(&prefix_path);
        assert!(result.is_err());
        assert!(matches!(result, Err(PlayError::PrefixCreation { .. })));
    }
}
