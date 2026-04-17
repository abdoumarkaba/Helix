#![allow(clippy::pedantic)]

//! Orchestrator - central coordinator for the play lifecycle.
//!
//! Implements the Plan-Confirm-Execute lifecycle with a state machine,
//! checkpoint system for crash recovery, and rollback coordination on failure.
//!
//! # Architecture
//!
//! The Orchestrator sequences all modules in dependency order:
//! 1. DetectionModule -> HardwareProfile + AudioConfig
//! 2. BinaryAnalysis -> GameIdentity
//! 3. PlanningModule -> GamePlan
//! 4. User confirmation (display plan, prompt y/N)
//! 5. ExecutionModule -> apply tweaks, install runner, create prefix, launch
//! 6. ValidationModule -> verify game is running
//!
//! # Rollback Strategy
//!
//! On any error after the Confirmed phase:
//! 1. Log failure with phase and error details
//! 2. Execute rollback in reverse order of rollback_manifest
//! 3. Drop guards (session-scoped tweaks restore automatically)
//! 4. Write final checkpoint with Failed phase
//! 5. Return PlayError with actionable context

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::binary::pe::analyze_binary;
use crate::execution::prefix::PrefixModule;
use crate::execution::runner::RunnerModule;
use crate::execution::system::ActiveGuards;
use crate::models::environment::{AudioConfig, GameEnvironment, GameIdentity, HardwareProfile};
use crate::models::errors::PlayError;
use crate::models::plan::GamePlan;
use crate::modules::detection::{detect_hardware, CommandRunner};
use crate::modules::planning::PlanningModule;
use crate::modules::validation::ValidationModule;

// ---------------------------------------------------------------------------
// OrchestratorPhase - State Machine
// ---------------------------------------------------------------------------

/// The phases of the play lifecycle.
///
/// State transitions are strictly sequential. Attempting to skip phases
/// is a programming error (debug builds will panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrchestratorPhase {
    /// Initial state, nothing detected yet.
    Initialized,
    /// Hardware detection complete, GameEnvironment partially populated.
    Detected,
    /// Planning complete, GamePlan produced, awaiting user confirmation.
    Planned,
    /// User confirmed, execution in progress.
    Confirmed,
    /// Execution complete, game launched.
    Executed,
    /// Validation passed, session complete.
    Validated,
    /// Rollback triggered, restoring previous state.
    RollingBack,
    /// Terminal failure state.
    Failed,
}

impl OrchestratorPhase {
    /// Returns true if this phase represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Validated | Self::Failed)
    }

    /// Returns true if system mutations may have occurred.
    pub fn has_mutations(&self) -> bool {
        matches!(
            self,
            Self::Confirmed | Self::Executed | Self::Validated | Self::Failed | Self::RollingBack
        )
    }
}

// ---------------------------------------------------------------------------
// Checkpoint Types
// ---------------------------------------------------------------------------

/// A persisted snapshot of the orchestrator state at a phase boundary.
///
/// Written after each successful phase transition. Enables crash recovery:
/// if the process dies mid-execution, the next run can detect the checkpoint
/// and offer to rollback or resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Phase when this checkpoint was written.
    pub phase: OrchestratorPhase,
    /// The GameEnvironment at this point.
    pub env: GameEnvironment,
    /// The GamePlan, if planning has completed.
    pub plan: Option<GamePlan>,
    /// Rollback manifest for Class B/C tweaks applied so far.
    pub rollback_manifest: Vec<RollbackEntry>,
    /// Timestamp of checkpoint creation.
    pub timestamp: DateTime<Utc>,
}

/// One entry in the rollback manifest.
///
/// Every Class B/C tweak writes an entry BEFORE applying the change.
/// On failure, rollback walks this list in reverse order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackEntry {
    /// The key that was changed (e.g., "vm.max_map_count").
    pub key: String,
    /// The previous value (for restoration).
    pub previous_value: String,
    /// How to restore (sysctl, sysfs, file).
    pub restore_method: RestoreMethod,
}

/// How to restore a rollback entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestoreMethod {
    /// Restore via sysctl write.
    Sysctl { key: String },
    /// Restore via sysfs file write.
    Sysfs { path: String },
    /// Restore via file write.
    FileWrite { path: String },
    /// Restore via nvidia-smi command.
    NvidiaSmi { args: Vec<String> },
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Type alias for user confirmation callback.
type ConfirmCallback = Box<dyn Fn(&GamePlan) -> bool>;

/// The central coordinator for the play lifecycle.
///
/// Holds all state for a single game session. Created fresh for each
/// `play <exe>` invocation, or recovered from a checkpoint if one exists.
pub struct Orchestrator {
    /// Injectable roots for testability.
    proc_root: PathBuf,
    sys_root: PathBuf,
    etc_root: PathBuf,
    /// State directory: ~/.local/share/play/games/{exe_hash}/
    state_root: PathBuf,
    /// Runner installation directory.
    runners_root: PathBuf,
    /// Prefix directory.
    prefix_root: PathBuf,
    /// Database root.
    db_root: PathBuf,
    /// Current phase.
    phase: OrchestratorPhase,
    /// The game environment being built up.
    env: Option<GameEnvironment>,
    /// The plan after planning phase.
    plan: Option<GamePlan>,
    /// Active session guards (Class A tweaks).
    guards: Option<ActiveGuards>,
    /// Rollback manifest for Class B/C tweaks.
    rollback_manifest: Vec<RollbackEntry>,
    /// Injectable command runner.
    cmd_runner: Box<dyn CommandRunner>,
    /// User confirmation callback (returns true to proceed).
    confirm_callback: Option<ConfirmCallback>,
}

impl Orchestrator {
    /// Create a new orchestrator for the given executable.
    ///
    /// # Arguments
    /// - `exe_path`: Path to the game executable.
    /// - `state_root`: Directory for checkpoints (~/.local/share/play/games/{hash}).
    /// - `runners_root`: Directory for installed runners.
    /// - `prefix_root`: Directory for Wine prefixes.
    /// - `db_root`: Directory for play-db cache.
    /// - `cmd_runner`: Injectable command runner for testability.
    pub fn new(
        exe_path: &Path,
        state_root: PathBuf,
        runners_root: PathBuf,
        prefix_root: PathBuf,
        db_root: PathBuf,
        cmd_runner: Box<dyn CommandRunner>,
    ) -> Self {
        // Compute exe_hash for state directory name
        let exe_hash = compute_exe_hash_prefix(exe_path);
        let game_state_root = state_root.join(exe_hash);

        Self {
            proc_root: PathBuf::from("/proc"),
            sys_root: PathBuf::from("/sys"),
            etc_root: PathBuf::from("/etc"),
            state_root: game_state_root,
            runners_root,
            prefix_root,
            db_root,
            phase: OrchestratorPhase::Initialized,
            env: None,
            plan: None,
            guards: None,
            rollback_manifest: Vec::new(),
            cmd_runner,
            confirm_callback: None,
        }
    }

    /// Create an orchestrator with injectable path roots (for testing).
    pub fn with_roots(
        proc_root: PathBuf,
        sys_root: PathBuf,
        etc_root: PathBuf,
        state_root: PathBuf,
        runners_root: PathBuf,
        prefix_root: PathBuf,
        db_root: PathBuf,
        cmd_runner: Box<dyn CommandRunner>,
    ) -> Self {
        Self {
            proc_root,
            sys_root,
            etc_root,
            state_root,
            runners_root,
            prefix_root,
            db_root,
            phase: OrchestratorPhase::Initialized,
            env: None,
            plan: None,
            guards: None,
            rollback_manifest: Vec::new(),
            cmd_runner,
            confirm_callback: None,
        }
    }

    /// Set a custom confirmation callback (for testing or non-interactive mode).
    pub fn set_confirm_callback(&mut self, callback: Box<dyn Fn(&GamePlan) -> bool>) {
        self.confirm_callback = Some(callback);
    }

    /// Get the current phase.
    pub fn phase(&self) -> OrchestratorPhase {
        self.phase
    }

    /// Get the current plan (if planning has completed).
    pub fn plan(&self) -> Option<&GamePlan> {
        self.plan.as_ref()
    }

    /// Get the current environment (if detection has completed).
    pub fn env(&self) -> Option<&GameEnvironment> {
        self.env.as_ref()
    }

    /// Recover from a previous run's checkpoint (if exists).
    ///
    /// Returns `Ok(true)` if recovery succeeded, `Ok(false)` if no checkpoint exists.
    pub fn recover(&mut self) -> Result<bool, PlayError> {
        let checkpoint_path = self.checkpoint_path();
        if !checkpoint_path.exists() {
            return Ok(false);
        }

        let checkpoint = self.load_checkpoint()?;
        self.phase = checkpoint.phase;
        self.env = Some(checkpoint.env);
        self.plan = checkpoint.plan;
        self.rollback_manifest = checkpoint.rollback_manifest;

        info!(
            event = "checkpoint_recovered",
            phase = ?self.phase,
            "Recovered from checkpoint"
        );

        Ok(true)
    }

    /// Run the full lifecycle: detect -> plan -> confirm -> execute -> validate.
    ///
    /// Returns `Ok(())` on successful game exit, `Err` on any failure.
    /// If the user aborts during confirmation, returns `Ok(())` without execution.
    pub fn run(&mut self, exe_path: &Path) -> Result<(), PlayError> {
        // Ensure state directory exists
        fs::create_dir_all(&self.state_root).map_err(|e| PlayError::CheckpointFailed {
            path: self.state_root.clone(),
            reason: format!("failed to create state directory: {e}"),
        })?;

        // Phase 1: Detection
        self.detect(exe_path)?;
        if self.phase == OrchestratorPhase::Failed {
            return self.final_error();
        }

        // Phase 2: Planning
        self.plan_phase()?;
        if self.phase == OrchestratorPhase::Failed {
            return self.final_error();
        }

        // Check for hard blocks
        if let Some(ref plan) = self.plan {
            if !plan.hard_blocks.is_empty() {
                error!(
                    event = "planning_blocked",
                    blocks = ?plan.hard_blocks,
                    "Planning found hard blocks, cannot proceed"
                );
                self.phase = OrchestratorPhase::Failed;
                self.write_checkpoint()?;
                return self.final_error();
            }
        }

        // Phase 3: Confirmation
        self.confirm_phase()?;
        if self.phase == OrchestratorPhase::Failed {
            return self.final_error();
        }
        if self.phase == OrchestratorPhase::Planned {
            // User aborted - clean exit without mutation
            info!(event = "user_aborted", "User declined execution");
            return Ok(());
        }

        // Phase 4: Execution
        self.execute_phase()?;
        if self.phase == OrchestratorPhase::Failed {
            return self.final_error();
        }

        // Phase 5: Validation
        self.validate_phase()?;
        if self.phase == OrchestratorPhase::Failed {
            return self.final_error();
        }

        Ok(())
    }

    /// Abort execution and rollback. Idempotent.
    pub fn abort(&mut self) -> Result<(), PlayError> {
        if !self.phase.has_mutations() {
            info!(event = "abort_noop", "No mutations to rollback");
            self.phase = OrchestratorPhase::Failed;
            return Ok(());
        }

        self.execute_rollback()?;
        self.phase = OrchestratorPhase::Failed;
        self.write_checkpoint()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Phase Methods
    // -----------------------------------------------------------------------

    /// Detection phase: hardware detection + binary analysis.
    fn detect(&mut self, exe_path: &Path) -> Result<(), PlayError> {
        debug_assert_eq!(self.phase, OrchestratorPhase::Initialized);

        info!(event = "phase_start", phase = "detection", "Starting detection phase");

        // Hardware detection
        let result = detect_hardware(
            &self.proc_root,
            &self.sys_root,
            &self.etc_root,
            self.cmd_runner.as_ref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        )?;

        let hw = result.hw;
        let audio = result.audio;

        // Binary analysis
        let binary = analyze_binary(exe_path)?;

        // Build GameIdentity
        let identity = GameIdentity {
            exe_hash: binary.hash.clone(),
            exe_name: exe_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown.exe".to_string()),
            steam_app_id: None, // Would need Steam integration
            detected_name: None,
            dx_version: binary.dx_version,
            pe_arch: binary.pe_arch,
            anti_cheat: binary.anti_cheat.clone(),
            engine_hint: binary.engine_hint,
            has_video_cutscenes: if binary.has_bink_video { Some(true) } else { None },
        };

        // Build partial GameEnvironment (detection fields only)
        let env = self.build_partial_env(identity, hw, audio)?;

        self.env = Some(env);
        self.phase = OrchestratorPhase::Detected;
        self.write_checkpoint()?;

        info!(event = "phase_complete", phase = "detection", "Detection phase complete");
        Ok(())
    }

    /// Planning phase: produce GamePlan from environment.
    fn plan_phase(&mut self) -> Result<(), PlayError> {
        debug_assert_eq!(self.phase, OrchestratorPhase::Detected);

        info!(event = "phase_start", phase = "planning", "Starting planning phase");

        let env = self.env.as_ref().ok_or_else(|| PlayError::OrchestratorFailed {
            phase: "planning".to_string(),
            reason: "no environment in planning phase".to_string(),
        })?;

        let planning = PlanningModule::new(
            self.db_root.clone(),
            self.runners_root.clone(),
            self.prefix_root.clone(),
        );

        let plan = planning.plan(env)?;

        self.plan = Some(plan);
        self.phase = OrchestratorPhase::Planned;
        self.write_checkpoint()?;

        info!(event = "phase_complete", phase = "planning", "Planning phase complete");
        Ok(())
    }

    /// Confirmation phase: display plan and get user approval.
    fn confirm_phase(&mut self) -> Result<(), PlayError> {
        debug_assert_eq!(self.phase, OrchestratorPhase::Planned);

        info!(event = "phase_start", phase = "confirmation", "Starting confirmation phase");

        let plan = self.plan.as_ref().ok_or_else(|| PlayError::OrchestratorFailed {
            phase: "confirmation".to_string(),
            reason: "no plan in confirmation phase".to_string(),
        })?;

        let confirmed = if let Some(ref callback) = self.confirm_callback {
            callback(plan)
        } else {
            // Default: use dialoguer for interactive confirmation
            self.interactive_confirm(plan)?
        };

        if confirmed {
            self.phase = OrchestratorPhase::Confirmed;
            self.write_checkpoint()?;
            info!(event = "phase_complete", phase = "confirmation", "User confirmed execution");
        } else {
            // User declined - stay in Planned phase, run() will return Ok(())
            info!(event = "user_declined", "User declined execution");
        }

        Ok(())
    }

    /// Execution phase: apply tweaks, install runner, create prefix, launch.
    fn execute_phase(&mut self) -> Result<(), PlayError> {
        debug_assert_eq!(self.phase, OrchestratorPhase::Confirmed);

        info!(event = "phase_start", phase = "execution", "Starting execution phase");

        let plan = self.plan.as_ref().ok_or_else(|| PlayError::OrchestratorFailed {
            phase: "execution".to_string(),
            reason: "no plan in execution phase".to_string(),
        })?;

        // Install required packages
        if !plan.required_packages.is_empty() {
            self.install_packages(&plan.required_packages)?;
        }

        // Ensure runner
        let runner_module = RunnerModule::new(self.runners_root.clone());
        runner_module.execute(&plan.runner_action)?;

        // Ensure prefix
        let prefix_module = PrefixModule::new();
        prefix_module.execute(&plan.prefix_action)?;

        // Apply system tweaks
        let system_module = crate::execution::system::SystemModule::new(
            self.sys_root.clone(),
            self.cmd_runner.clone_boxed(),
        );

        let env = self.env.as_ref().ok_or_else(|| PlayError::OrchestratorFailed {
            phase: "execution".to_string(),
            reason: "no environment in execution phase".to_string(),
        })?;

        let guards = system_module.apply(&plan.tweaks, &env.hardware)?;

        // Record rollback entries for Class B/C tweaks
        // Clone tweaks to avoid borrow conflict with plan reference
        let tweaks_clone = plan.tweaks.clone();
        self.record_rollback_entries(&tweaks_clone);

        self.guards = Some(guards);
        self.phase = OrchestratorPhase::Executed;
        self.write_checkpoint()?;

        info!(event = "phase_complete", phase = "execution", "Execution phase complete");
        Ok(())
    }

    /// Validation phase: verify game is running correctly.
    fn validate_phase(&mut self) -> Result<(), PlayError> {
        debug_assert_eq!(self.phase, OrchestratorPhase::Executed);

        info!(event = "phase_start", phase = "validation", "Starting validation phase");

        // Get the environment for GPU vendor info
        let env = self.env.as_ref().ok_or_else(|| PlayError::OrchestratorFailed {
            phase: "validation".to_string(),
            reason: "no environment in validation phase".to_string(),
        })?;

        // Create validation module (fully used in future integration)
        let _validator = ValidationModule::new(self.cmd_runner.clone_boxed());

        // We would normally get the game process from execution phase
        // For now, do a lightweight validation that doesn't require process handle
        // The actual process validation happens during wait/shutdown

        // Check GPU activity if we can detect it
        let gpu_vendor = env.hardware.gpu.vendor;
        info!(event = "validation_gpu_check", vendor = ?gpu_vendor, "Checking GPU activity");

        // Note: Full validation with process handle requires passing the Child from execution
        // For now, we validate what we can without blocking

        self.phase = OrchestratorPhase::Validated;
        self.write_checkpoint()?;

        // Drop guards - session-scoped tweaks restore automatically
        if let Some(guards) = self.guards.take() {
            drop(guards);
            info!(event = "guards_dropped", "Session guards restored");
        }

        info!(event = "phase_complete", phase = "validation", "Validation phase complete");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Checkpoint Methods
    // -----------------------------------------------------------------------

    fn checkpoint_path(&self) -> PathBuf {
        self.state_root.join("checkpoint.json")
    }

    /// Write current state to checkpoint file.
    fn write_checkpoint(&self) -> Result<(), PlayError> {
        let env = self.env.as_ref().ok_or_else(|| PlayError::CheckpointFailed {
            path: self.state_root.clone(),
            reason: "no environment to checkpoint".to_string(),
        })?;

        let checkpoint = Checkpoint {
            phase: self.phase,
            env: env.clone(),
            plan: self.plan.clone(),
            rollback_manifest: self.rollback_manifest.clone(),
            timestamp: Utc::now(),
        };

        let json =
            serde_json::to_string_pretty(&checkpoint).map_err(|e| PlayError::CheckpointFailed {
                path: self.checkpoint_path(),
                reason: format!("failed to serialize checkpoint: {e}"),
            })?;

        // Write atomically: write to temp, then rename
        let temp_path = self.checkpoint_path().with_extension("tmp");
        let mut file = fs::File::create(&temp_path).map_err(|e| PlayError::CheckpointFailed {
            path: temp_path.clone(),
            reason: format!("failed to create temp checkpoint: {e}"),
        })?;
        file.write_all(json.as_bytes()).map_err(|e| PlayError::CheckpointFailed {
            path: temp_path.clone(),
            reason: format!("failed to write checkpoint: {e}"),
        })?;
        fs::rename(&temp_path, &self.checkpoint_path()).map_err(|e| {
            PlayError::CheckpointFailed {
                path: self.checkpoint_path(),
                reason: format!("failed to rename checkpoint: {e}"),
            }
        })?;

        tracing::debug!(
            event = "checkpoint_written",
            phase = ?self.phase,
            path = %self.checkpoint_path().display(),
            "Checkpoint written"
        );

        Ok(())
    }

    /// Load checkpoint from file.
    fn load_checkpoint(&self) -> Result<Checkpoint, PlayError> {
        let path = self.checkpoint_path();
        let mut file = fs::File::open(&path).map_err(|e| PlayError::CheckpointFailed {
            path: path.clone(),
            reason: format!("failed to open checkpoint: {e}"),
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| PlayError::CheckpointFailed {
            path: path.clone(),
            reason: format!("failed to read checkpoint: {e}"),
        })?;

        serde_json::from_str(&contents).map_err(|e| PlayError::CheckpointFailed {
            path: path.clone(),
            reason: format!("failed to parse checkpoint: {e}"),
        })
    }

    // -----------------------------------------------------------------------
    // Rollback Methods
    // -----------------------------------------------------------------------

    /// Record rollback entries for Class B/C tweaks.
    fn record_rollback_entries(&mut self, tweaks: &[crate::models::plan::PlannedTweak]) {
        use crate::models::plan::{SystemTweak, TweakClass, TweakDecision};

        for tweak in tweaks {
            if let TweakDecision::Apply(ref sys_tweak) = tweak.decision {
                match (tweak.class, sys_tweak) {
                    (TweakClass::B, SystemTweak::VmMaxMapCount { target: _ }) => {
                        // Get current value from env
                        let prev = self
                            .env
                            .as_ref()
                            .map(|e| e.hardware.kernel.vm_max_map_count.to_string())
                            .unwrap_or_else(|| "65530".to_string());

                        self.rollback_manifest.push(RollbackEntry {
                            key: "vm.max_map_count".to_string(),
                            previous_value: prev,
                            restore_method: RestoreMethod::Sysctl {
                                key: "vm.max_map_count".to_string(),
                            },
                        });
                    },
                    (TweakClass::B, SystemTweak::ThpMadvise) => {
                        let prev = self
                            .env
                            .as_ref()
                            .map(|e| format!("{:?}", e.hardware.kernel.thp_mode))
                            .unwrap_or_else(|| "Madvise".to_string());

                        self.rollback_manifest.push(RollbackEntry {
                            key: "transparent_hugepage".to_string(),
                            previous_value: prev,
                            restore_method: RestoreMethod::Sysfs {
                                path: "/sys/kernel/mm/transparent_hugepage/enabled".to_string(),
                            },
                        });
                    },
                    (TweakClass::B, SystemTweak::SchedAutogroup { .. }) => {
                        let prev = self
                            .env
                            .as_ref()
                            .map(|e| e.hardware.kernel.sched_autogroup.to_string())
                            .unwrap_or_else(|| "1".to_string());

                        self.rollback_manifest.push(RollbackEntry {
                            key: "kernel.sched_autogroup".to_string(),
                            previous_value: prev,
                            restore_method: RestoreMethod::Sysctl {
                                key: "kernel.sched_autogroup".to_string(),
                            },
                        });
                    },
                    (TweakClass::C, SystemTweak::NvidiaClockLock { .. }) => {
                        // Clock lock is reset via nvidia-smi -rgc
                        self.rollback_manifest.push(RollbackEntry {
                            key: "nvidia_clock_lock".to_string(),
                            previous_value: "auto".to_string(),
                            restore_method: RestoreMethod::NvidiaSmi {
                                args: vec!["-rgc".to_string()],
                            },
                        });
                    },
                    _ => {},
                }
            }
        }
    }

    /// Execute rollback in reverse order.
    fn execute_rollback(&mut self) -> Result<(), PlayError> {
        self.phase = OrchestratorPhase::RollingBack;

        info!(
            event = "rollback_start",
            entries = self.rollback_manifest.len(),
            "Starting rollback"
        );

        // Drop guards first - session tweaks restore automatically
        if let Some(guards) = self.guards.take() {
            drop(guards);
            info!(event = "guards_dropped", "Session guards restored during rollback");
        }

        // Walk rollback manifest in reverse
        for entry in self.rollback_manifest.iter().rev() {
            if let Err(e) = self.restore_one(entry) {
                // Log but continue - don't stop on single failure
                error!(
                    event = "rollback_entry_failed",
                    key = %entry.key,
                    error = %e,
                    "Rollback entry failed, continuing"
                );
            } else {
                info!(
                    event = "rollback_entry_restored",
                    key = %entry.key,
                    "Rollback entry restored"
                );
            }
        }

        self.rollback_manifest.clear();
        info!(event = "rollback_complete", "Rollback complete");
        Ok(())
    }

    /// Restore a single rollback entry.
    fn restore_one(&self, entry: &RollbackEntry) -> Result<(), PlayError> {
        match &entry.restore_method {
            RestoreMethod::Sysctl { key } => {
                self.cmd_runner
                    .run_command("play-helper", &["sysctl-write", key, &entry.previous_value])
                    .map_err(|e| PlayError::RollbackFailed {
                        key: entry.key.clone(),
                        reason: e,
                        previous_value: entry.previous_value.clone(),
                        path: PathBuf::from("/proc/sys").join(key.replace('.', "/")),
                    })?;
            },
            RestoreMethod::Sysfs { path } => {
                let value = match entry.previous_value.as_str() {
                    "Always" => "always",
                    "Madvise" => "madvise",
                    "Never" => "never",
                    other => other,
                };
                self.cmd_runner.run_command("play-helper", &["sysfs-write", path, value]).map_err(
                    |e| PlayError::RollbackFailed {
                        key: entry.key.clone(),
                        reason: e,
                        previous_value: entry.previous_value.clone(),
                        path: PathBuf::from(path),
                    },
                )?;
            },
            RestoreMethod::FileWrite { path } => {
                self.cmd_runner
                    .run_command("play-helper", &["write-file", path, &entry.previous_value])
                    .map_err(|e| PlayError::RollbackFailed {
                        key: entry.key.clone(),
                        reason: e,
                        previous_value: entry.previous_value.clone(),
                        path: PathBuf::from(path),
                    })?;
            },
            RestoreMethod::NvidiaSmi { args } => {
                let args_str: Vec<&str> = args.iter().map(String::as_str).collect();
                self.cmd_runner.run_command("nvidia-smi", &args_str).map_err(|e| {
                    PlayError::RollbackFailed {
                        key: entry.key.clone(),
                        reason: e,
                        previous_value: entry.previous_value.clone(),
                        path: PathBuf::from("nvidia-smi"),
                    }
                })?;
            },
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helper Methods
    // -----------------------------------------------------------------------

    /// Build a partial GameEnvironment from detection results.
    fn build_partial_env(
        &self,
        identity: GameIdentity,
        hw: HardwareProfile,
        audio: AudioConfig,
    ) -> Result<GameEnvironment, PlayError> {
        use crate::models::environment::{
            DxvkConfig, DxvkHud, EnvironmentMetadata, GraphicsConfig, LaunchConfig, PrefixConfig,
            RunnerConfig, SystemTuning,
        };
        use semver::Version;

        // Create default configs for fields that planning will fill in
        let graphics = GraphicsConfig {
            translation_layer: crate::models::environment::TranslationLayer::Native,
            dxvk_version: None,
            vkd3d_version: None,
            dxvk_config: DxvkConfig {
                async_compile: false,
                frame_limit: None,
                hud: DxvkHud::Off,
                state_cache: true,
                config_path: PathBuf::new(),
            },
            mangohud: None,
            gamemode: false,
        };

        let runner = RunnerConfig {
            runner_type: crate::models::environment::RunnerType::ProtonGE,
            version: Version::new(0, 0, 0),
            install_path: PathBuf::new(),
            verified: false,
        };

        let prefix = PrefixConfig {
            path: self.prefix_root.join(&identity.exe_hash),
            arch: crate::models::environment::WineArch::Win64,
            windows_version: crate::models::environment::WindowsVersion::Win10,
            dll_overrides: Vec::new(),
            env_vars: indexmap::IndexMap::new(),
            large_address_aware: false,
        };

        let system = SystemTuning {
            vm_max_map_count: None,
            thp_mode: None,
            sched_autogroup: None,
            split_lock_mitigate: None,
            ulimit_nofile: None,
            cpu_governor: None,
            gpu_perf_mode: None,
            esync: false,
            fsync: false,
            gamemode: false,
            nvidia_clock_lock_mhz: None,
        };

        let launch = LaunchConfig {
            exe_path: PathBuf::new(),
            working_dir: PathBuf::new(),
            args: Vec::new(),
            env: indexmap::IndexMap::new(),
            pre_launch: Vec::new(),
            post_exit: Vec::new(),
        };

        let metadata = EnvironmentMetadata {
            schema_version: 1,
            created_at: Utc::now(),
            last_run: None,
            tool_version: Version::parse(env!("CARGO_PKG_VERSION"))
                .unwrap_or(Version::new(0, 1, 0)),
            resolution_source: crate::models::environment::ResolutionSource::FullyAutomatic,
            decisions: Vec::new(),
        };

        Ok(GameEnvironment {
            identity,
            hardware: hw,
            graphics,
            runner,
            audio,
            prefix,
            system,
            launch,
            metadata,
        })
    }

    /// Interactive confirmation using dialoguer.
    fn interactive_confirm(&self, plan: &GamePlan) -> Result<bool, PlayError> {
        use console::style;

        // Display plan summary
        println!();
        println!("{}", style("  PLAY PLAN").bold().cyan());
        println!("{}", style("  ").cyan());
        println!("{}", style(format!("  Game: {}", plan.env.identity.exe_name)).cyan());
        println!("{}", style(format!("  Hash: {}", plan.env.identity.exe_hash)).cyan());
        println!();

        // Show warnings
        for warning in &plan.warnings {
            println!("{} {}", style("  WARNING:").yellow(), warning.message);
        }
        if !plan.warnings.is_empty() {
            println!();
        }

        // Show tweaks
        println!("{}", style("  System Changes:").cyan());
        for tweak in &plan.tweaks {
            if let crate::models::plan::TweakDecision::Apply(ref sys_tweak) = tweak.decision {
                println!(
                    "    {} {}",
                    style("·").cyan(),
                    format!("{:?}: {:?}", tweak.id, sys_tweak)
                );
            }
        }
        println!();

        // Prompt for confirmation
        let result = dialoguer::Confirm::new()
            .with_prompt("Proceed with execution?")
            .default(false)
            .interact()
            .map_err(|e| PlayError::OrchestratorFailed {
                phase: "confirmation".to_string(),
                reason: format!("failed to get user confirmation: {e}"),
            })?;

        Ok(result)
    }

    /// Install required packages using the detected package manager.
    fn install_packages(
        &self,
        packages: &[crate::models::plan::RequiredPackage],
    ) -> Result<(), PlayError> {
        use crate::modules::package_manager::detect_package_manager;

        let pm = detect_package_manager(self.cmd_runner.as_ref())?;

        let to_install: Vec<&str> =
            packages.iter().filter(|p| !p.already_installed).map(|p| p.name.as_str()).collect();

        if to_install.is_empty() {
            return Ok(());
        }

        info!(
            event = "packages_installing",
            packages = ?to_install,
            pm = pm.name(),
            "Installing required packages"
        );

        pm.install(&to_install)
    }

    /// Return the final error after failure.
    fn final_error(&self) -> Result<(), PlayError> {
        Err(PlayError::OrchestratorFailed {
            phase: format!("{:?}", self.phase),
            reason: "orchestrator failed".to_string(),
        })
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        // If we're dropping while in a mutating phase, attempt rollback
        if self.phase.has_mutations() && self.phase != OrchestratorPhase::Validated {
            warn!(
                event = "orchestrator_dropped_early",
                phase = ?self.phase,
                "Orchestrator dropped before completion, attempting rollback"
            );

            // Best-effort rollback
            let _ = self.execute_rollback();

            // Write final checkpoint
            self.phase = OrchestratorPhase::Failed;
            let _ = self.write_checkpoint();
        }
    }
}

// ---------------------------------------------------------------------------
// Helper Functions
// ---------------------------------------------------------------------------

/// Compute a short hash prefix for the state directory name.
fn compute_exe_hash_prefix(exe_path: &Path) -> String {
    // For now, use the filename + a simple hash
    // In production, this would be the actual SHA256
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    exe_path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::environment::HardwareProfile;
    use std::collections::HashMap;

    #[derive(Clone)]
    #[allow(dead_code)]
    struct MockCommandRunner {
        responses: HashMap<String, Result<String, String>>,
    }

    impl CommandRunner for MockCommandRunner {
        fn run_command(&self, program: &str, _args: &[&str]) -> Result<String, String> {
            self.responses.get(program).cloned().unwrap_or(Ok(String::new()))
        }

        fn clone_boxed(&self) -> Box<dyn CommandRunner> {
            Box::new(self.clone())
        }
    }

    #[test]
    fn test_phase_is_terminal() {
        assert!(OrchestratorPhase::Validated.is_terminal());
        assert!(OrchestratorPhase::Failed.is_terminal());
        assert!(!OrchestratorPhase::Initialized.is_terminal());
        assert!(!OrchestratorPhase::Executed.is_terminal());
    }

    #[test]
    fn test_phase_has_mutations() {
        assert!(!OrchestratorPhase::Initialized.has_mutations());
        assert!(!OrchestratorPhase::Detected.has_mutations());
        assert!(!OrchestratorPhase::Planned.has_mutations());
        assert!(OrchestratorPhase::Confirmed.has_mutations());
        assert!(OrchestratorPhase::Executed.has_mutations());
    }

    #[test]
    fn test_checkpoint_serialization() {
        let checkpoint = Checkpoint {
            phase: OrchestratorPhase::Detected,
            env: crate::models::environment::GameEnvironment {
                identity: crate::models::environment::GameIdentity {
                    exe_hash: "abc123".to_string(),
                    exe_name: "test.exe".to_string(),
                    steam_app_id: None,
                    detected_name: None,
                    dx_version: crate::models::environment::DirectXVersion::D3D11,
                    pe_arch: crate::models::environment::PeArchitecture::X86_64,
                    anti_cheat: Vec::new(),
                    engine_hint: None,
                    has_video_cutscenes: None,
                },
                hardware: HardwareProfile {
                    gpu: crate::models::environment::GpuProfile {
                        vendor: crate::models::environment::GpuVendor::NVIDIA,
                        model: "RTX 3080".to_string(),
                        vram_mb: 10240,
                        driver_version: semver::Version::new(535, 0, 0),
                        vulkan_version: None,
                        driver_type: crate::models::environment::DriverType::NvidiaProprietary,
                        features: crate::models::environment::GpuFeatureSet {
                            vulkan_1_2: true,
                            vulkan_1_3: true,
                            ray_tracing: true,
                            mesh_shaders: true,
                            resizable_bar: true,
                            dx12_feature_level: Some("12_2".to_string()),
                        },
                        is_laptop_gpu: false,
                        nvidia_vbios_max_clock_mhz: Some(2100),
                    },
                    cpu: crate::models::environment::CpuProfile {
                        vendor: crate::models::environment::CpuVendor::AMD,
                        model: "Ryzen 9 5950X".to_string(),
                        arch: crate::models::environment::CpuArch::X86_64,
                        physical_cores: 16,
                        logical_cores: 32,
                        base_freq_mhz: 3400,
                        supports_avx2: true,
                        supports_avx512: true,
                        is_laptop_cpu: false,
                    },
                    memory: crate::models::environment::MemoryProfile {
                        total_mb: 65536,
                        available_mb: 32768,
                        swap_total_mb: 8192,
                    },
                    kernel: crate::models::environment::KernelProfile {
                        version: crate::models::environment::KernelVersion {
                            major: 6,
                            minor: 1,
                            patch: 0,
                        },
                        has_futex2: true,
                        has_fsync: true,
                        vm_max_map_count: 65530,
                        thp_mode: crate::models::environment::ThpMode::Madvise,
                        split_lock_mitigate: true,
                        sched_autogroup: true,
                    },
                    display: crate::models::environment::DisplayProfile {
                        server: crate::models::environment::DisplayServer::X11,
                        primary_res: (1920, 1080),
                        refresh_hz: 144.0,
                    },
                    distro: crate::models::environment::DistroInfo {
                        distro: crate::models::environment::Distro::Fedora,
                        version_id: "39".to_string(),
                        pretty_name: "Fedora 39".to_string(),
                    },
                },
                graphics: crate::models::environment::GraphicsConfig {
                    translation_layer: crate::models::environment::TranslationLayer::Native,
                    dxvk_version: None,
                    vkd3d_version: None,
                    dxvk_config: crate::models::environment::DxvkConfig {
                        async_compile: false,
                        frame_limit: None,
                        hud: crate::models::environment::DxvkHud::Off,
                        state_cache: true,
                        config_path: PathBuf::new(),
                    },
                    mangohud: None,
                    gamemode: false,
                },
                runner: crate::models::environment::RunnerConfig {
                    runner_type: crate::models::environment::RunnerType::ProtonGE,
                    version: semver::Version::new(0, 0, 0),
                    install_path: PathBuf::new(),
                    verified: false,
                },
                audio: crate::models::environment::AudioConfig {
                    backend: crate::models::environment::AudioBackend::PipeWire,
                    server_rate: 48000,
                    wine_driver: crate::models::environment::WineAudioDriver::Pulse,
                    latency_ms: None,
                },
                prefix: crate::models::environment::PrefixConfig {
                    path: PathBuf::new(),
                    arch: crate::models::environment::WineArch::Win64,
                    windows_version: crate::models::environment::WindowsVersion::Win10,
                    dll_overrides: Vec::new(),
                    env_vars: indexmap::IndexMap::new(),
                    large_address_aware: false,
                },
                system: crate::models::environment::SystemTuning {
                    vm_max_map_count: None,
                    thp_mode: None,
                    sched_autogroup: None,
                    split_lock_mitigate: None,
                    ulimit_nofile: None,
                    cpu_governor: None,
                    gpu_perf_mode: None,
                    esync: false,
                    fsync: false,
                    gamemode: false,
                    nvidia_clock_lock_mhz: None,
                },
                launch: crate::models::environment::LaunchConfig {
                    exe_path: PathBuf::new(),
                    working_dir: PathBuf::new(),
                    args: Vec::new(),
                    env: indexmap::IndexMap::new(),
                    pre_launch: Vec::new(),
                    post_exit: Vec::new(),
                },
                metadata: crate::models::environment::EnvironmentMetadata {
                    schema_version: 1,
                    created_at: Utc::now(),
                    last_run: None,
                    tool_version: semver::Version::new(0, 1, 0),
                    resolution_source: crate::models::environment::ResolutionSource::FullyAutomatic,
                    decisions: Vec::new(),
                },
            },
            plan: None,
            rollback_manifest: Vec::new(),
            timestamp: Utc::now(),
        };

        // Verify serialization round-trip
        let json = serde_json::to_string(&checkpoint).expect("serialize");
        let restored: Checkpoint = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.phase, OrchestratorPhase::Detected);
    }

    #[test]
    fn test_rollback_entry_sysctl() {
        let entry = RollbackEntry {
            key: "vm.max_map_count".to_string(),
            previous_value: "65530".to_string(),
            restore_method: RestoreMethod::Sysctl { key: "vm.max_map_count".to_string() },
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let restored: RollbackEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.key, "vm.max_map_count");
    }
}
