
```markdown
# Final CLAUDE-REFERENCE.md (exact original content of all deleted sections — no semantic changes)

# CLAUDE-REFERENCE.md
Full reference material for the play project. Load ONLY when needed via `cat CLAUDE-REFERENCE.md`.

## Architecture: Module Pipeline
CLI Entry Point (main.rs)
│
▼
┌───────────────────────────────────────────────────────┐
│                    Orchestrator                        │
│  Owns: GamePlan, ExecutionState, EventLog              │
│  Drives: module sequencing, state machine, checkpoints │
└──────────┬────────────────────────────────────────────┘
│ dispatches typed inputs, receives typed outputs
┌─────┴──────────────────────────────────┐
│                                        │
▼                                        ▼
DetectionModule                        ValidationModule
├─ HardwareDetector                    ├─ LaunchMonitor
├─ BinaryAnalyzer (PE parser)          ├─ GpuActivityChecker
├─ KernelInspector                     ├─ FpsLogger (MangoHud)
└─ AudioStackDetector                  └─ CrashDetector
│
▼ DetectionResult
PlanningModule
├─ DatabaseClient (local cache, NO network at runtime)
├─ TweakRegistry  (all tweaks as data structs)
├─ ConstraintChecker
├─ DecisionEngine (compiled rule tree — no AI)
└─ PlanBuilder → GamePlan
│
▼ GamePlan (after user confirmation)
ExecutionModules (sequential, checkpointed)
├─ PackageModule   (system dependencies)
├─ RunnerModule    (Proton/Wine download+verify)
├─ PrefixModule    (Wine prefix setup)
├─ SystemModule    (sysctl + Drop guards, iterates TweakRegistry)
└─ LaunchModule    (final process spawn + metrics collection)

**Critical module rules:**
- Modules communicate ONLY through typed data structures via the Orchestrator
- Modules NEVER call each other directly
- Modules have NO side effects outside their designated responsibility domain
- Every state transition is logged; the tool can resume from any checkpoint state

**Orchestrator state machine:**
```rust
enum ExecutionState {
    Idle, Detecting, Planning,
    AwaitingConfirmation { plan: GamePlan },
    Executing { phase: ExecutionPhase, checkpoint: Checkpoint },
    Launching,
    Running { pid: u32, guards: ActiveGuards },
    Cleanup,
    Done { result: RunResult },
    Failed { phase: ExecutionPhase, error: PlayError, rollback: RollbackManifest },
}

// The single source of truth passed between all modules. Detection fills it.
// Planning mutates it. Execution reads it. Never bypass it.
pub struct GameEnvironment {
    pub identity:  GameIdentity,    // exe_hash (primary key), dx_version, pe_arch, anti_cheat
    pub hardware:  HardwareProfile, // GpuProfile, CpuProfile, KernelProfile, DisplayProfile
    pub graphics:  GraphicsConfig,  // TranslationLayer, dxvk_version, vkd3d_version
    pub runner:    RunnerConfig,    // RunnerType, version (always resolved, never hardcoded)
    pub audio:     AudioConfig,     // PipeWire | PulseAudio | ALSA
    pub prefix:    PrefixConfig,    // path, arch, windows_version, dll_overrides, env_vars
    pub system:    SystemTuning,    // all tweaks as Option<T> (None = no change needed)
    pub launch:    LaunchConfig,    // exe_path, args, env, pre_launch hooks
    pub metadata:  EnvironmentMetadata, // decisions: Vec<ResolutionDecision>
}

// Every planning decision recorded here — the audit trail.
pub struct ResolutionDecision {
    pub field:      String,   // e.g. "runner.version"
    pub chosen:     String,   // e.g. "GE-Proton9-27"
    pub reason:     String,   // human-readable sentence, shown in --verbose, stored in state
    pub confidence: Confidence,
    pub source:     DecisionSource,
}

// All tweaks are data, not code. Orchestrator iterates this registry.
pub struct TweakConstraint {
    pub tweak:               SystemTweak,
    pub min_ram_mb:          Option<u32>,
    pub kernel_version_min:  Option<KernelVersion>,
    pub gpu_vendor_required: Option<GpuVendor>,
    pub requires_desktop:    bool,  // if true: tweak silently skipped on laptops
    pub reversible:          bool,
    pub reboot_required:     bool,
    pub risk_level:          RiskLevel,
    pub rationale:           &'static str,
}
// If constraints not satisfied → TweakDecision::NotApplicable (INVISIBLE, not disabled)

// Key enums (use these, never string comparisons):
enum DirectXVersion { D3D8, D3D9, D3D10, D3D11, D3D12, Vulkan, OpenGL, Unknown }
enum RunnerType { ProtonGE, WineGE, ProtonOfficial, WineStaging }
enum TranslationLayer { Dxvk, Vkd3dProton, WineOpenGL, Native }
enum GpuVendor { NVIDIA, AMD, Intel }
enum AntiCheat { EasyAntiCheat { linux_supported: bool }, BattlEye { linux_supported: bool },
                 Denuvo, VMProtect, GameGuard }
                 
Decision Logic: Key Rules
Runner selection:

AntiCheat::EasyAntiCheat { linux_supported: false } → hard fail, PlayError::AntiCheatBlocked
D3D12 → TranslationLayer::Vkd3dProton
D3D9/10/11/8 → TranslationLayer::Dxvk
Denuvo or EAC(linux_supported: true) → DxvkConfig.async_compile = false
Default runner: RunnerType::ProtonGE (always; GE is the safer default)
Version: query play-db for version_min, then select LATEST stable above floor from runners.toml
NEVER hardcode a version string like "9.10". NEVER.

System tweaks (critical formulas — do not approximate):

// vm.max_map_count: hardware-proportional, NEVER hardcoded as 2097152
combined_mb = hardware.memory.total_mb + hardware.gpu.vram_mb
// On dev machine: ~15657 + 6144 = ~21801 MB → tier 16_385..=32_768 → target = 8_388_608

// NVIDIA clock lock: NEVER hardcode 1530
fn compute_nvidia_lock_clock(vbios_max_mhz: u32) -> u32 {
    ((vbios_max_mhz as f32 * 0.95) as u32 / 15) * 15
}
// RTX 3050: max=1777 → 1688 → rounded = 1680 MHz

// Sync: kernel >= 5.16 (has_futex2) → fsync. Otherwise → esync + ulimit check

Drop guard pattern (mandatory for ALL session-scoped changes):
Rust// MUST use Drop for cpu_governor, gpu_perf_mode, nvidia_clock_lock, any session change.
// Rust guarantees drop() runs even on panic. Every guard has a Noop variant for no-op.
impl Drop for GovernorGuard {
    fn drop(&mut self) {
        // Log errors but NEVER panic in Drop
        for (path, prev) in &self.cores {
            let _ = fs::write(path, prev).map_err(|e| tracing::error!(...));
        }
    }
}

Tweak Classes (reference)
Class A — session-scoped, Drop-restored, no confirmation needed:
fsync/esync env vars, CPU governor, GameMode LD_PRELOAD, DXVK state cache path,
DXVK async, VKD3D feature level, MangoHud inject, WINE_LARGE_ADDRESS_AWARE.
Class B — persistent, rollback manifest FIRST, confirm once per game:
vm.max_map_count (via helper), sched_autogroup (via helper), split_lock_mitigate (via helper),
THP→madvise, ulimit nofile (/etc/security/limits.d/), audio sample rate, Wine prefix creation,
Windows version in registry.
Class C — NVIDIA only (v1), desktop only, explicit consent:
NVIDIA persistence mode (nvidia-smi -pm 1), NVIDIA clock lock (computed formula, never hardcoded).
AMD → TweakDecision::NotApplicable (Phase 2 stub, never panic, never error).
Class C — laptop rule:
GpuProfile.is_laptop_gpu = true → ALL Class C GPU tweaks return TweakDecision::NotApplicable.
On the dev machine (RTX 3050 Laptop): no clock lock, no persistence mode tweaks.
Antipatterns — NEVER Do These
text✗ String comparisons for distro/vendor detection ("ubuntu".contains(...))
  → Use typed enums. Detect once, store typed, match on enum everywhere.

✗ .unwrap() or .expect() outside of tests
  → Every ? propagates PlayError. Every error is typed and contextual.

✗ Hardcoding paths without the injectable root parameter
  → All /proc /sys paths use the injected sys_root for testability.

✗ Writing to system state before recording the rollback entry
  → Rollback manifest written FIRST. Always. No exceptions.

✗ Running play-helper for read operations
  → Read /proc /sys directly as unprivileged user. Helper is write-only.

✗ Blocking the main thread during downloads
  → Use indicatif progress bar with a thread. User must see progress.

✗ Silently falling back when a critical component is missing
  → Warn explicitly in the plan: "MangoHud not installed — performance
    overlay unavailable. Install with: sudo dnf install mangohud"

✗ Script fields in database entries
  → Database entries are pure data. No executable content. Ever.

✗ Spawning child processes with shell=true / sh -c "..."
  → Use Command with explicit args. Never interpolate user data into shell strings.

✗ Different GPU vendor code paths deep in the same function
  → GPU-specific code lives in separate impl blocks behind a trait.

✗ Hardcoding a runner version string (e.g. "GE-Proton9-27")
  → Version ALWAYS resolved at plan time from runners.toml. Never baked into source.

✗ Hardcoding vm.max_map_count as 2097152
  → Use the hardware-proportional formula. Depends on combined RAM + VRAM.

✗ Hardcoding nvidia-smi -lgc 1530,1530
  → Compute from vbios_max_clock_mhz. Use compute_nvidia_lock_clock().

✗ Panicking or erroring on AMD hardware in v1
  → AMD arms return TweakDecision::NotApplicable. Silent, clean, never an error.

✗ Making any runtime network call for AI inference or decision-making
  → play is a deterministic binary. play-db cache is the ONLY knowledge source at runtime.

✗ Assuming Wine prefix creation always succeeds silently
  → wine --version first, capture stderr, parse wineserver PID, wait for initialization.
Error Model
Rust// Never use anyhow for user-facing errors. Use the typed PlayError taxonomy.
// Every variant must carry the context needed for a helpful error message.
// Pattern: #[error("...")] with named fields, never just a String.
// Use thiserror. Propagate with ?. The Orchestrator catches and handles.
Key error variants to remember: AntiCheatBlocked, ChecksumMismatch, PrefixCreation,
SysctlWrite, GameCrash, RollbackFailed, UnsupportedDistro, InsufficientVram.
Security Model
textplay (unprivileged)  →  play-helper (narrow privilege boundary)
                         JSON command interface, never shell strings
                         Logs every privileged operation with timestamp
Never run the main binary as root. The helper binary handles: sysctl writes, nvidia-smi
perf changes, ulimit conf file writes. Read /proc /sys directly as the user.
play-db Design (separate repo)
play-db is a community database of per-game configuration. play reads it as a local cache.
It is NEVER fetched at game launch time (only on play --update-db).
Entry format: TOML files at entries/{hash[0:2]}/{hash}/default.toml (+ optional nvidia.toml).
Runner manifest: runners.toml — lists available GE-Proton versions with SHA256.
Confidence scores: computed by tools/compute_confidence.py, never manually assigned.
text```markdown

