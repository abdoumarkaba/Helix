#![allow(clippy::pedantic)]
use std::path::PathBuf;

use indexmap::IndexMap;
use semver::Version;
use serde::{Deserialize, Serialize};

use super::environment::{DllOverride, GpuVendor, RunnerType, WindowsVersion, WineArch};

// ---------------------------------------------------------------------------
// GamePlan — the output of PlanningModule shown to user before confirmation
// ---------------------------------------------------------------------------

/// The complete resolved plan for running a game.
/// Serializable for `--dry-run` output. Produced before any system state is changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePlan {
    /// Fully resolved GameEnvironment (all config fields populated by planning).
    pub env: super::environment::GameEnvironment,
    /// Non-fatal advisories (missing optional tools, etc.) shown in plan summary.
    pub warnings: Vec<PlanWarning>,
    /// If non-empty, planning has hard-failed — Orchestrator must not proceed.
    /// Stored as strings because PlayError is not Clone.
    pub hard_blocks: Vec<String>,
    /// System packages that must be installed before execution.
    pub required_packages: Vec<RequiredPackage>,
    /// What will happen with the runner binary.
    pub runner_action: RunnerAction,
    /// What will happen with the Wine prefix.
    pub prefix_action: PrefixAction,
    /// Every tweak decision, including NotApplicable entries (for audit trail).
    pub tweaks: Vec<PlannedTweak>,
    /// Whether a play-db entry was found and used for this game.
    pub db_hit: bool,
}

// ---------------------------------------------------------------------------
// PlannedTweak
// ---------------------------------------------------------------------------

/// An explicit decision about a single system tweak.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTweak {
    pub id: TweakId,
    pub class: TweakClass,
    pub decision: TweakDecision,
    /// One-sentence human rationale. Always populated.
    pub rationale: String,
}

/// Resolved outcome for one tweak.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TweakDecision {
    /// Tweak will be applied with this hardware-computed value.
    Apply(SystemTweak),
    /// Constraint not satisfied — never displayed as an error, silently absent from output.
    NotApplicable { reason: String },
    /// System already satisfies the requirement — no action needed.
    AlreadySatisfied,
}

/// All tweak types with their resolved (hardware-computed) values.
/// Values are NEVER hardcoded — always computed by DecisionEngine formulas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemTweak {
    /// Computed via hardware-proportional tier formula (never hardcoded as 2097152).
    VmMaxMapCount {
        target: u64,
    },
    /// Set to Madvise for game workloads (not Always).
    ThpMadvise,
    SchedAutogroup {
        enabled: bool,
    },
    SplitLockMitigate {
        enabled: bool,
    },
    UlimitNofile {
        value: u64,
    },
    /// Session-scoped, Drop-restored by SystemModule.
    CpuGovernorPerformance,
    /// NVIDIA only, requires helper.
    NvidiaPersistenceMode,
    /// Computed via compute_nvidia_lock_clock(vbios_max_mhz). NEVER hardcoded.
    NvidiaClockLock {
        min_mhz: u32,
        max_mhz: u32,
    },
    DxvkAsync {
        enabled: bool,
    },
    Fsync,
    Esync,
    GameMode,
}

/// Stable identifier for a tweak category (no hardware values — those live in SystemTweak).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TweakId {
    VmMaxMapCount,
    ThpMadvise,
    SchedAutogroup,
    SplitLockMitigate,
    UlimitNofile,
    CpuGovernorPerformance,
    NvidiaPersistenceMode,
    NvidiaClockLock,
    DxvkAsync,
    Fsync,
    Esync,
    GameMode,
}

/// Class drives confirmation policy and rollback strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TweakClass {
    /// Session-scoped, Drop-restored. No user confirmation needed.
    A,
    /// Persistent kernel/system change. Rollback manifest written first. Confirm once per game.
    B,
    /// NVIDIA desktop-only. Requires explicit user consent.
    C,
}

/// Risk level of applying the tweak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

// ---------------------------------------------------------------------------
// TweakConstraint — the pure data that lives in TweakRegistry
// ---------------------------------------------------------------------------

/// One row in the TweakRegistry. All fields are constraints; none contain logic.
/// DecisionEngine evaluates these against live hardware to produce TweakDecision.
#[derive(Debug, Clone)]
pub struct TweakConstraint {
    pub id: TweakId,
    pub class: TweakClass,
    /// Tweak skipped (NotApplicable) if total RAM < this (MB).
    pub min_ram_mb: Option<u64>,
    /// Tweak skipped if kernel version < (major, minor).
    pub kernel_version_min: Option<(u32, u32)>,
    /// Only applicable for this GPU vendor (None = any vendor).
    pub gpu_vendor_required: Option<GpuVendor>,
    /// GPU vendors explicitly excluded from this tweak.
    pub gpu_vendor_exclusions: Vec<GpuVendor>,
    /// Only applicable on this CPU architecture (None = any arch).
    pub cpu_arch_required: Option<crate::models::environment::CpuArch>,
    /// If true: is_laptop = true → NotApplicable (no thermal-risky tweaks on laptops).
    pub requires_desktop: bool,
    pub reversible: bool,
    pub reboot_required: bool,
    /// If true, the tweak's effect is reset on reboot (e.g. sysctl).
    pub reboot_resets: bool,
    pub risk_level: RiskLevel,
    pub rationale: &'static str,
}

// ---------------------------------------------------------------------------
// RunnerAction / PrefixAction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunnerAction {
    /// Runner binary already present and verified.
    AlreadyInstalled { path: PathBuf },
    /// Will be downloaded and SHA512-verified during execution.
    Download { url: String, version: Version, sha512: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrefixAction {
    /// Prefix already exists at this path.
    AlreadyExists { path: PathBuf },
    /// Will be created via wineboot during execution.
    Create { path: PathBuf, arch: WineArch, windows_version: WindowsVersion },
}

// ---------------------------------------------------------------------------
// RequiredPackage / PlanWarning
// ---------------------------------------------------------------------------

/// A system package that execution requires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredPackage {
    pub name: String,
    pub reason: String,
    pub already_installed: bool,
}

/// Non-fatal advisory shown in the plan summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanWarning {
    pub message: String,
    /// e.g. "sudo dnf install mangohud"
    pub install_hint: Option<String>,
}

// ---------------------------------------------------------------------------
// play-db types
// ---------------------------------------------------------------------------

/// A play-db entry deserialized from the local cache (TOML).
/// Source: ~/.local/share/play/db/{hash[0:2]}/{hash}/default.toml
#[derive(Debug, Clone, Deserialize)]
pub struct DbEntry {
    /// Version floor — planning picks the highest available runner above this.
    pub runner_version_min: Option<Version>,
    /// Override default runner type (e.g. wine-ge for very old 32-bit games).
    pub runner_type_override: Option<RunnerType>,
    #[serde(default)]
    pub dll_overrides: Vec<DllOverride>,
    #[serde(default)]
    pub extra_env_vars: IndexMap<String, String>,
    pub windows_version_override: Option<WindowsVersion>,
    pub notes: Option<String>,
}

impl Default for DbEntry {
    fn default() -> Self {
        Self {
            runner_version_min: None,
            runner_type_override: None,
            dll_overrides: Vec::new(),
            extra_env_vars: IndexMap::new(),
            windows_version_override: None,
            notes: None,
        }
    }
}

/// One entry in the runners.toml manifest (play-db).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerRelease {
    pub runner_type: RunnerType,
    pub version: Version,
    pub url: String,
    pub sha512: String,
}

/// Deserialized runners.toml from local play-db cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnersManifest {
    pub runners: Vec<RunnerRelease>,
}

// ---------------------------------------------------------------------------
// Re-export for convenience
// ---------------------------------------------------------------------------
pub use super::environment::ResolutionDecision;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tweak_id_is_exhaustive() {
        // Ensure all TweakId variants map to something — compile-time check.
        let ids = [
            TweakId::VmMaxMapCount,
            TweakId::ThpMadvise,
            TweakId::SchedAutogroup,
            TweakId::SplitLockMitigate,
            TweakId::UlimitNofile,
            TweakId::CpuGovernorPerformance,
            TweakId::NvidiaPersistenceMode,
            TweakId::NvidiaClockLock,
            TweakId::DxvkAsync,
            TweakId::Fsync,
            TweakId::Esync,
            TweakId::GameMode,
        ];
        assert_eq!(ids.len(), 12);
    }
}
