#![allow(clippy::pedantic)]

//! Package manager detection and abstraction layer.
//!
//! Provides a trait-based abstraction over various Linux distro package managers
//! (apt, dnf, pacman, zypper) and detection logic to identify the available manager.

use std::process::Command;

use crate::models::errors::PlayError;

// ---------------------------------------------------------------------------
// Package-manager abstraction
// ---------------------------------------------------------------------------

/// Capability that can be mapped to a package name per distro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    GameMode,
    MangoHud,
    Vulkan,
    VulkanTools,
    WineBase,
    CabExtract,
}

/// Trait for distro package managers.
///
/// Each implementation knows how to install packages and map
/// a [`Capability`] to the correct package name on its distro.
pub trait PackageManager: Send + Sync {
    /// Human-readable name, e.g. `"apt"`, `"dnf"`, `"pacman"`.
    fn name(&self) -> &'static str;

    /// Check if a package is already installed.
    fn is_installed(&self, package: &str) -> bool;

    /// Install one or more packages. Returns the first error encountered.
    fn install(&self, packages: &[&str]) -> Result<(), PlayError>;

    /// Map a capability to the correct package name, or `None` if unsupported.
    fn package_for(&self, capability: Capability) -> Option<&'static str>;
}

pub struct AptPm;

impl PackageManager for AptPm {
    fn name(&self) -> &'static str {
        "apt"
    }

    fn is_installed(&self, package: &str) -> bool {
        // Check via package manager first
        if Command::new("dpkg")
            .args(["-s", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }

        // Also check for common binary if package check fails
        match package {
            "wine64" => Command::new("wine")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            "gamemode" => {
                Command::new("gamemoderun").output().map(|o| o.status.success()).unwrap_or(false)
                    || Command::new("gamemode")
                        .arg("--version")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            },
            _ => false,
        }
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        // Try with sudo first (for interactive terminals)
        let output = Command::new("sudo")
            .args(["apt-get", "install", "-y"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "apt".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "apt".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("libvulkan1"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine64"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct DnfPm;

impl PackageManager for DnfPm {
    fn name(&self) -> &'static str {
        "dnf"
    }

    fn is_installed(&self, package: &str) -> bool {
        // Check via package manager first
        if Command::new("rpm")
            .args(["-q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }

        // Also check for common binary if package check fails
        // (some packages like wine might be meta-packages)
        match package {
            "wine" => Command::new("wine")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            "gamemode" => {
                Command::new("gamemoderun").output().map(|o| o.status.success()).unwrap_or(false)
                    || Command::new("gamemode")
                        .arg("--version")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            },
            _ => false,
        }
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        // Try with sudo first (for interactive terminals)
        let output =
            Command::new("sudo").args(["dnf", "install", "-y"]).args(packages).output().map_err(
                |e| PlayError::PackageInstall {
                    pm: "dnf".into(),
                    package: packages.join(", "),
                    stderr: e.to_string(),
                },
            )?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "dnf".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-loader"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct PacmanPm;

impl PackageManager for PacmanPm {
    fn name(&self) -> &'static str {
        "pacman"
    }

    fn is_installed(&self, package: &str) -> bool {
        // Check via package manager first
        if Command::new("pacman")
            .args(["-Q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }

        // Also check for common binary if package check fails
        match package {
            "wine" => Command::new("wine")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            "gamemode" => {
                Command::new("gamemoderun").output().map(|o| o.status.success()).unwrap_or(false)
                    || Command::new("gamemode")
                        .arg("--version")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            },
            _ => false,
        }
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        // Try with sudo first (for interactive terminals)
        let output = Command::new("sudo")
            .args(["pacman", "-S", "--noconfirm"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "pacman".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "pacman".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-validation-layers"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

pub struct ZypperPm;

impl PackageManager for ZypperPm {
    fn name(&self) -> &'static str {
        "zypper"
    }

    fn is_installed(&self, package: &str) -> bool {
        // Check via package manager first
        if Command::new("rpm")
            .args(["-q", package])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }

        // Also check for common binary if package check fails
        match package {
            "wine" => Command::new("wine")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false),
            "gamemode" => {
                Command::new("gamemoderun").output().map(|o| o.status.success()).unwrap_or(false)
                    || Command::new("gamemode")
                        .arg("--version")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
            },
            _ => false,
        }
    }

    fn install(&self, packages: &[&str]) -> Result<(), PlayError> {
        // Try with sudo first (for interactive terminals)
        let output = Command::new("sudo")
            .args(["zypper", "install", "-y"])
            .args(packages)
            .output()
            .map_err(|e| PlayError::PackageInstall {
                pm: "zypper".into(),
                package: packages.join(", "),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlayError::PackageInstall {
                pm: "zypper".into(),
                package: packages.join(", "),
                stderr: stderr.to_string(),
            });
        }
        Ok(())
    }

    fn package_for(&self, cap: Capability) -> Option<&'static str> {
        match cap {
            Capability::GameMode => Some("gamemode"),
            Capability::MangoHud => Some("mangohud"),
            Capability::Vulkan => Some("vulkan-loader"),
            Capability::VulkanTools => Some("vulkan-tools"),
            Capability::WineBase => Some("wine"),
            Capability::CabExtract => Some("cabextract"),
        }
    }
}

/// Detect the available package manager and return the appropriate implementation.
///
/// Checks in order: apt, dnf, pacman, zypper. Returns the first found.
/// Uses `cmd_runner` for testability instead of calling `Command::new` directly.
/// Returns an error only if no supported package manager is found.
pub fn detect_package_manager(
    cmd_runner: &dyn crate::modules::detection::CommandRunner,
) -> Result<Box<dyn PackageManager>, PlayError> {
    // apt
    if cmd_runner.run_command("apt", &["--version"]).map(|_| true).unwrap_or(false) {
        return Ok(Box::new(AptPm));
    }
    // dnf / yum
    if cmd_runner.run_command("dnf", &["--version"]).map(|_| true).unwrap_or(false) {
        return Ok(Box::new(DnfPm));
    }
    // pacman
    if cmd_runner.run_command("pacman", &["--version"]).map(|_| true).unwrap_or(false) {
        return Ok(Box::new(PacmanPm));
    }
    // zypper (SUSE / openSUSE)
    if cmd_runner.run_command("zypper", &["--version"]).map(|_| true).unwrap_or(false) {
        return Ok(Box::new(ZypperPm));
    }

    Err(PlayError::UnsupportedDistro { distro: "unknown".into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::detection::CommandRunner;

    #[derive(Clone)]
    struct MockRunner {
        available: &'static str,
    }

    impl CommandRunner for MockRunner {
        fn run_command(&self, program: &str, _args: &[&str]) -> Result<String, String> {
            if program == self.available {
                Ok(String::new())
            } else {
                Err("not found".to_owned())
            }
        }
        fn clone_boxed(&self) -> Box<dyn CommandRunner> {
            Box::new(self.clone())
        }
    }

    #[test]
    fn pm_names() {
        assert_eq!(AptPm.name(), "apt");
        assert_eq!(DnfPm.name(), "dnf");
        assert_eq!(PacmanPm.name(), "pacman");
        assert_eq!(ZypperPm.name(), "zypper");
    }

    #[test]
    fn package_for_all_capabilities() {
        use Capability::*;
        let apt = AptPm;
        for cap in [GameMode, MangoHud, Vulkan, VulkanTools, WineBase, CabExtract] {
            assert!(apt.package_for(cap).is_some());
        }
        let dnf = DnfPm;
        for cap in [GameMode, MangoHud, Vulkan, VulkanTools, WineBase, CabExtract] {
            assert!(dnf.package_for(cap).is_some());
        }
        let pacman = PacmanPm;
        for cap in [GameMode, MangoHud, Vulkan, VulkanTools, WineBase, CabExtract] {
            assert!(pacman.package_for(cap).is_some());
        }
        let zypper = ZypperPm;
        for cap in [GameMode, MangoHud, Vulkan, VulkanTools, WineBase, CabExtract] {
            assert!(zypper.package_for(cap).is_some());
        }
    }

    #[test]
    fn detect_package_manager_apt() {
        let runner = MockRunner { available: "apt" };
        let pm = detect_package_manager(&runner).unwrap();
        assert_eq!(pm.name(), "apt");
    }

    #[test]
    fn detect_package_manager_dnf() {
        let runner = MockRunner { available: "dnf" };
        let pm = detect_package_manager(&runner).unwrap();
        assert_eq!(pm.name(), "dnf");
    }

    #[test]
    fn detect_package_manager_pacman() {
        let runner = MockRunner { available: "pacman" };
        let pm = detect_package_manager(&runner).unwrap();
        assert_eq!(pm.name(), "pacman");
    }

    #[test]
    fn detect_package_manager_zypper() {
        let runner = MockRunner { available: "zypper" };
        let pm = detect_package_manager(&runner).unwrap();
        assert_eq!(pm.name(), "zypper");
    }

    #[test]
    fn detect_package_manager_none() {
        let runner = MockRunner { available: "" };
        assert!(detect_package_manager(&runner).is_err());
    }
}
