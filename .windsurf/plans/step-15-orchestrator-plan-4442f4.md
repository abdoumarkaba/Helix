# Step 15: orchestrator.rs Implementation Plan

**Target**: `/home/abdoufoundit/play/crates/play-core/src/orchestrator.rs`

**Spec Reference**: `PLAY_MASTER_PROMPT.md:1539-1540`
> Step 15: orchestrator.rs
>        - State machine, checkpoint system, module sequencing

---

## 1. Overview

The Orchestrator is the central coordinator that implements the **Plan-Confirm-Execute** lifecycle. It sequences all modules, manages state transitions, handles rollback on failure, and persists checkpoints for recovery.

### Key Responsibilities

- **State Machine**: Track execution phase (Detection, Planning, Confirmation, Execution, Validation)
- **Checkpoint System**: Persist `GameEnvironment` at each phase boundary
- **Module Sequencing**: Call modules in dependency order with correct inputs
- **Rollback Coordination**: Walk rollback manifest on failure
- **Session Lifecycle**: Hold `ActiveGuards` for session-scoped tweaks

---

## 2. Types Required

### 2.1 OrchestratorState (State Machine)

```rust
/// The phases of the play lifecycle.
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
```

### 2.2 Checkpoint

```rust
/// A persisted snapshot of the orchestrator state at a phase boundary.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackEntry {
    /// The key that was changed (e.g., "vm.max_map_count").
    pub key: String,
    /// The previous value (for restoration).
    pub previous_value: String,
    /// How to restore (sysctl, sysfs, file).
    pub restore_method: RestoreMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestoreMethod {
    Sysctl { key: String },
    Sysfs { path: String },
    FileWrite { path: String },
    NvidiaSmi { args: Vec<String> },
}
```

### 2.3 Orchestrator

```rust
/// The central coordinator for the play lifecycle.
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
}
```

---

## 3. Error Cases

| Error | PlayError Variant | Scenario |
|-------|-------------------|----------|
| Checkpoint read failure | `StateCorrupted` | Checkpoint file exists but cannot be parsed |
| Checkpoint write failure | `StateCorrupted` | Cannot write checkpoint to state directory |
| Rollback failure | `RollbackFailed` | Restoration of a tweak fails during rollback |
| Phase transition violation | (internal panic in debug) | Attempting to skip phases |
| User abort | Return `Ok(())` with no execution | User declines confirmation |

---

## 4. Eight Principles Checklist

| Principle | Satisfaction |
|-----------|--------------|
| Plan-Confirm-Execute | **Satisfied**: Orchestrator enforces phase transitions; no mutation before confirmation |
| Idempotency | **Satisfied**: Checkpoint recovery allows resuming; AlreadySatisfied checks prevent re-application |
| Rollback guarantee | **Satisfied**: Rollback manifest written before each Class B/C tweak |
| Fail loudly | **Satisfied**: Every error is typed PlayError with context |
| Explainable decisions | **Satisfied**: Decisions recorded in GamePlan, displayed to user |
| Output as UI | **Satisfied**: Plan display rendered before confirmation |
| Security by default | **Satisfied**: All privileged ops through play-helper; paths sanitized |
| Tweak is data | **Satisfied**: Orchestrator contains no tweak-specific logic; dispatches via TweakId |

---

## 5. Antipattern Check

| Antipattern | Risk |
|------------|------|
| `.unwrap()` outside tests | **None**: All errors propagated via `?` |
| Hardcoded paths | **None**: All paths injected via constructor |
| GPU vendor conditionals in Orchestrator | **None**: Vendor logic in GpuPerfGuard trait impl |
| Hardcoded runner version | **None**: Version resolved from manifest |
| Hardcoded vm.max_map_count | **None**: Uses `compute_vm_max_map_count()` |
| Hardcoded NVIDIA clock | **None**: Uses `compute_nvidia_lock_clock()` |

---

## 6. Test Plan

### Unit Tests

1. **Phase transitions**: Verify each valid transition, reject invalid skips
2. **Checkpoint persistence**: Write, read, verify round-trip
3. **Rollback manifest**: Verify entries are written before sysctl changes
4. **Guard Drop on failure**: Simulate error mid-execution, verify guards restore

### Integration Tests

1. **Full happy path**: Detection -> Planning -> Confirmation -> Execution -> Validation
2. **User abort**: Plan displayed, user declines, no system mutation
3. **Mid-execution failure**: Error during execution, rollback fires, guards restore
4. **Crash recovery**: Kill process mid-execution, restart with checkpoint recovery

### Property Tests

1. **Idempotency**: Running orchestrator twice with same input produces same output
2. **Rollback completeness**: Every Class B/C tweak has a corresponding rollback entry

---

## 7. Implementation Order

1. **Types** (`models/orchestrator.rs` or inline)
   - `OrchestratorPhase`
   - `Checkpoint`
   - `RollbackEntry`
   - `RestoreMethod`

2. **Orchestrator struct** (`orchestrator.rs`)
   - Constructor with injectable paths
   - Phase getter
   - State getters

3. **Phase methods**
   - `detect()` -> Result<(), PlayError>
   - `plan()` -> Result<GamePlan, PlayError>
   - `confirm()` -> Result<(), PlayError> (user prompt)
   - `execute()` -> Result<(), PlayError>
   - `validate()` -> Result<(), PlayError>

4. **Checkpoint methods**
   - `write_checkpoint()`
   - `load_checkpoint()`
   - `recover_from_checkpoint()`

5. **Rollback methods**
   - `record_rollback_entry()`
   - `execute_rollback()`

6. **Tests**
   - Unit tests for each method
   - Integration test for full lifecycle

---

## 8. Module Sequencing Diagram

```
Orchestrator::run(exe_path)
    |
    v
[Phase: Initialized]
    |
    +-- DetectionModule::detect(proc_root, sys_root, etc_root, cmd_runner)
    |       |
    |       v
    |   HardwareProfile + AudioConfig
    |       |
    |       v
    |   BinaryAnalysis::analyze(exe_path)
    |       |
    |       v
    |   GameIdentity
    |
    v
[Phase: Detected] -- checkpoint written
    |
    +-- PlanningModule::plan(env)
    |       |
    |       v
    |   GamePlan (env, warnings, hard_blocks, tweaks, runner_action, prefix_action)
    |
    v
[Phase: Planned] -- checkpoint written
    |
    +-- display_plan(plan) -> user_confirmation
    |       |
    |       v
    |   [y/N]? -> if N, return Ok(())
    |
    v
[Phase: Confirmed]
    |
    +-- ExecutionModule::execute(plan)
    |       |
    |       +-- PackageModule::install_required(packages)
    |       +-- RunnerModule::ensure_runner(runner_action)
    |       +-- PrefixModule::ensure_prefix(prefix_action)
    |       +-- SystemModule::apply(tweaks) -> ActiveGuards
    |       +-- LaunchModule::launch(env)
    |
    v
[Phase: Executed] -- guards held, game running
    |
    +-- ValidationModule::validate(env)
    |
    v
[Phase: Validated]
    |
    +-- Drop guards (restore session tweaks)
    |
    v
[Complete]
```

---

## 9. Rollback Strategy

On any error after `Confirmed` phase:

1. **Log the failure** with phase and error details
2. **Execute rollback** in reverse order of `rollback_manifest`
3. **Drop guards** (session-scoped tweaks restore automatically)
4. **Write final checkpoint** with `Failed` phase
5. **Return `PlayError`** with actionable context

If rollback itself fails:
- Emit `RollbackFailed` with manual restoration instructions
- Continue to next rollback entry (do not stop on single failure)

---

## 10. API Sketch

```rust
impl Orchestrator {
    /// Create a new orchestrator for the given executable.
    pub fn new(exe_path: &Path, cmd_runner: Box<dyn CommandRunner>) -> Self;
    
    /// Run the full lifecycle: detect -> plan -> confirm -> execute -> validate.
    /// Returns Ok(()) on successful game exit, Err on any failure.
    pub fn run(&mut self) -> Result<(), PlayError>;
    
    /// Recover from a previous run's checkpoint (if exists).
    pub fn recover(&mut self) -> Result<bool, PlayError>;
    
    /// Get the current phase.
    pub fn phase(&self) -> OrchestratorPhase;
    
    /// Get the current plan (if planning has completed).
    pub fn plan(&self) -> Option<&GamePlan>;
    
    /// Abort execution and rollback. Idempotent.
    pub fn abort(&mut self) -> Result<(), PlayError>;
}
```

---

## 11. Dependencies on Prior Steps

| Step | Module | Status | Dependency |
|------|--------|--------|------------|
| 4 | DetectionModule | Implemented | Required for `detect()` |
| 5-6 | Guards | Implemented | Required for session tweaks |
| 7 | TweakRegistry | Implemented | Required for tweak dispatch |
| 8 | PlanningModule | Implemented | Required for `plan()` |
| 9 | DatabaseClient | Implemented | Required by PlanningModule |
| 10-13 | ExecutionModules | Implemented | Required for `execute()` |
| 14 | play-helper | Implemented | Required for privileged writes |

---

## 12. Confidence Rating

**Approved** - The types, methods, and sequencing are well-defined in the master prompt. The existing codebase provides all required modules. No architectural ambiguity.

---

## 13. Blockers

None identified.

---

## 14. Suggestions

1. **Tracing integration**: Add structured tracing spans for each phase transition
2. **Metrics collection**: Track phase duration for performance analysis
3. **Dry-run mode**: Add `--dry-run` flag that stops after planning without confirmation
4. **Undo command**: Implement `play --undo <exe>` that reads checkpoint and rolls back

---

## 15. Next Steps After Approval

1. Create `orchestrator.rs` in `crates/play-core/src/`
2. Add `pub mod orchestrator;` to `lib.rs`
3. Implement types first, then orchestrator methods
4. Write unit tests alongside implementation
5. Add integration test in `tests/` directory
6. Update `play-cli/src/main.rs` to use Orchestrator
