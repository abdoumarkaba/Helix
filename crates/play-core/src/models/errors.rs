use std::path::PathBuf;
use thiserror::Error;

/// Never use anyhow for user-facing errors. Define the taxonomy explicitly.
/// Every variant carries the context needed for a helpful error message.
#[derive(Debug, Error)]
pub enum PlayError {
    #[error("Binary analysis failed for {path}: {reason}")]
    BinaryAnalysis { path: PathBuf, reason: String },

    #[error("Hardware detection incomplete: {component} could not be determined")]
    HardwareDetection { component: String },

    #[error(
        "Anti-cheat {name} is present and has no Linux support. \
         Check https://areweanticheatyet.com for status updates."
    )]
    AntiCheatBlocked { name: String },

    #[error("Runner download failed: {url} (attempt {attempt}/3): {reason}")]
    RunnerDownload {
        url: String,
        attempt: u8,
        reason: String,
    },

    #[error(
        "Runner checksum mismatch. Expected {expected}, got {actual}. \
         File deleted for safety."
    )]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Package manager {pm} failed to install {package}: {stderr}")]
    PackageInstall {
        pm: String,
        package: String,
        stderr: String,
    },

    #[error("Wine prefix creation failed at {prefix_path}: {reason}")]
    PrefixCreation {
        prefix_path: PathBuf,
        reason: String,
    },

    #[error("Sysctl write to {key} failed (requires elevated helper): {reason}")]
    SysctlWrite { key: String, reason: String },

    #[error(
        "Game launched but crashed after {seconds}s. \
         Log: {log_path}. Consider filing a report with --report."
    )]
    GameCrash { seconds: u32, log_path: PathBuf },

    #[error(
        "Rollback failed for {key}: {reason}. \
         Manual restoration: write '{previous_value}' to {path}"
    )]
    RollbackFailed {
        key: String,
        reason: String,
        previous_value: String,
        path: PathBuf,
    },

    #[error("State file corrupted at {path}. Run `play --reset {{game}}` to start fresh.")]
    StateCorrupted { path: PathBuf },

    #[error(
        "Unsupported distro: {distro}. Supported: Ubuntu 22.04+, \
         Fedora 38+, Arch, Debian 12+"
    )]
    UnsupportedDistro { distro: String },

    #[error(
        "Insufficient VRAM: game likely requires ~{required_mb}MB, \
         detected {available_mb}MB"
    )]
    InsufficientVram { required_mb: u32, available_mb: u32 },

    #[error(
        "play-db entry for {hash} is corrupted: {reason}. \
         Delete the entry and run `play --update-db`."
    )]
    DatabaseCorrupted { hash: String, reason: String },

    #[error("runners.toml not found at {path}. Run `play --update-db` to fetch it.")]
    RunnersManifestMissing { path: PathBuf },

    #[error(
        "No {runner_type} runner satisfies version floor {version_min}. \
         Run `play --update-db` to refresh the runner manifest."
    )]
    NoRunnerAvailable {
        runner_type: String,
        version_min: String,
    },

    #[error("Planning failed: {reason}")]
    PlanningFailed { reason: String },
}
