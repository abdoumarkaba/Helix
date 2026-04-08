# PLAY — Master Project Brief
## Windsurf Project Rules

---

<project_identity>
# Project: `play`

A distro-agnostic Linux CLI that orchestrates the existing open-source gaming
compatibility stack to enable post-2005 Windows games to run as well as the
user's hardware physically allows — with zero manual configuration required.

**THIS IS A DETERMINISTIC RUST BINARY. NOT AN AI AGENT SYSTEM.**
`play` ships as a compiled Rust binary. It has zero runtime external dependencies:
no API keys, no network calls to AI services, no cloud-based decision making.
All intelligence lives in compiled logic and the local `play-db` knowledge cache.
AI agents are a DEVELOPMENT-TIME tool only (for generating and validating the
`play-db` knowledge base). They are NEVER part of the shipped binary's runtime.

**Invocation contract (sacred, never changes):**
```
play game.exe
```
No flags required. No configuration files required from the user.
No prior knowledge required. One command. Works or tells you exactly why it won't.

**Standard CLI flags always supported:**
```
play --help / -h          Show usage and flag reference
play --version            Print semver, build date, play-db version
play --yes                Non-interactive mode (skip confirmation prompt)
play --undo game.exe      Reverse all persistent changes for this game
play --update-db          Sync local play-db cache
play --verbose            Show all ResolutionDecision rationale
play --dry-run            Print plan, write nothing to disk
```

**What this tool IS:**
A deterministic orchestrator of Wine/Proton/DXVK/VKD3D-Proton/GameMode/MangoHud
and related tools. It detects, decides, installs, configures, and launches.
Every decision is made by compiled rule trees, not runtime AI calls.

**What this tool IS NOT:**
- A compatibility layer (it uses existing ones)
- A game launcher GUI (pure CLI)
- A package manager (it uses the system's)
- A Wine replacement
- Anything that requires root to run normally
- An AI agent or cloud-dependent service

**Primary goal of every invocation:** The game runs. Successfully. At maximum
performance for this hardware. Post-launch, `play` records session metrics
(FPS via MangoHud, VRAM usage, crash status) to the game's state file.
If a game exits cleanly, the session is logged as successful. If it crashes,
the log is preserved and the user is prompted to file a report.

**License:** GPL-3.0 (copyleft protection for the ecosystem work embedded here)

**Target distros v1:** Ubuntu 22.04+, Fedora 38+, Arch Linux, Debian 12+
**Deferred:** Gentoo, NixOS, source-based distros (Phase 2)
**Language:** Rust (edition 2021, MSRV 1.75)
**No unsafe unless absolutely unavoidable and documented with justification**

**GPU support scope:**
- v1: NVIDIA (proprietary driver stack, nvidia-smi, persistence mode)
- Phase 2: AMD (AMDGPU/RADV, sysfs DPM level) — architecture is modular; stub
  the `GpuVendor::AMD` path behind a typed enum arm that returns
  `TweakDecision::NotApplicable` until Phase 2. Never panic on AMD hardware.
</project_identity>

---

<core_philosophy>
# Inviolable Design Principles

These are load-bearing constraints. Every implementation decision must satisfy
all of them simultaneously. If a proposed implementation violates any, redesign.

## 1. Plan → Confirm → Execute (NEVER skipped)
The system MUST produce a complete human-readable plan of every intended action
BEFORE executing any of them. The user sees exactly what will happen, confirms,
then execution begins. No surprises. No silent system modification.

```
EXCEPTION: read-only operations (detection, analysis) need no confirmation.
EXCEPTION: --yes flag allows non-interactive use in scripts (logged prominently).
```

## 2. Idempotency Everywhere
Running `play game.exe` twice produces identical results. Every operation checks
current state before acting. Installing a runner that exists is a verified no-op.
Configuring a prefix that exists means verifying it, not recreating it.
A user must be able to re-run after any failure safely.

## 3. Complete Rollback Guarantee
Every persistent system modification (sysctl, packages, files) is recorded in a
rollback manifest BEFORE the change is made. `play --undo game.exe` reverses
everything the tool did for that game. Session-scoped changes (CPU governor, GPU
perf mode) restore automatically via Rust Drop guards — not optionally, always.

## 4. Fail Loudly and Specifically, Never Silently
Partial success is the worst outcome. Either fully succeed or fully roll back and
report EXACTLY what happened with actionable guidance. Never swallow errors to
keep the happy path clean. Every error variant carries context, not just a string.

## 5. Every Decision is Explainable in One Sentence
If you cannot write a single human-readable sentence explaining why the system
chose a specific runner, translation layer, or configuration, the decision logic
is wrong. Every `ResolutionDecision` carries its rationale as a string that is
shown to the user in verbose mode and stored in the state file.

## 6. The Output Is a First-Class Interface
Terminal output is a UI. Every line has a purpose. Consistent visual language:
`→` pending, `⟳` in-progress, `✓` done, `⊘` skipped, `⚠` warning, `✗` error.
A non-technical user must understand the plan output. A technical user must find
the verbose output sufficient for debugging without source access.

## 7. Security By Default
- Never run the full tool as root. Privileged operations go through a narrow
  helper binary (`play-helper`) with a JSON command interface.
- Never execute downloaded binaries without SHA256 verification against a
  manifest signed with a known public key.
- Never trust game-provided data for path construction (path traversal prevention).
- Audit log every privileged operation with timestamp, operation, and actor.

## 8. A Tweak Is Data, Not Code
Every optimization maps to the same `Tweak` struct in the `TweakRegistry`.
The Orchestrator never contains tweak-specific logic — it only iterates the
registry. Adding tweak #200 is a one-file change. No agent, orchestrator,
or schema changes required. This is the architectural principle that allows
scaling to 200+ tweaks without rewrites.
</core_philosophy>

---

<architecture>
# System Architecture: Deterministic Pipeline

## Module Pipeline

`play` is composed of specialized, deterministic code modules with strict typed
contracts. These are NOT AI agents — they are Rust structs and functions that
execute compiled decision logic. They are called "agents" in code for semantic
clarity (each has a single responsibility), but they make zero runtime AI calls.

Modules communicate ONLY through typed data structures via the Orchestrator.
Modules NEVER call each other directly. Modules have NO side effects outside
their designated responsibility domain.

```
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
```

## Orchestrator State Machine

```rust
// Every state transition is logged. The tool can resume from any state.
enum ExecutionState {
    Idle,
    Detecting,
    Planning,
    AwaitingConfirmation { plan: GamePlan },
    Executing { phase: ExecutionPhase, checkpoint: Checkpoint },
    Launching,
    Running     { pid: u32, guards: ActiveGuards },
    Cleanup,
    Done        { result: RunResult },
    Failed      { phase: ExecutionPhase, error: PlayError, rollback: RollbackManifest },
}

enum ExecutionPhase {
    Packages,
    Runner,
    Prefix,
    System,
}
```

## The Checkpoint System

After each ExecutionPhase completes successfully, write a checkpoint:
```toml
# ~/.local/share/play/states/{game_hash}/checkpoint.toml
[checkpoint]
schema_version = 1
game_hash      = "sha256:abc123..."
reached_phase  = "Prefix"
timestamp      = "2025-03-01T14:22:11Z"
plan_hash      = "sha256:def456..."  # detects if plan changed between runs
```

On next invocation, detect checkpoint and offer: resume from `Prefix` or restart.
</architecture>

---

<data_model>
# Canonical Data Model

## Primary Contract: GameEnvironment

This struct is the single source of truth for everything about a configured game.
Detection fills it. Planning mutates it. Execution reads from it. Never bypass it.

```rust
/// The complete, versioned description of a game's runtime environment.
/// This is what gets serialized to state files and what modules pass between them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEnvironment {
    pub identity:   GameIdentity,
    pub hardware:   HardwareProfile,
    pub graphics:   GraphicsConfig,
    pub runner:     RunnerConfig,
    pub audio:      AudioConfig,
    pub prefix:     PrefixConfig,
    pub system:     SystemTuning,
    pub launch:     LaunchConfig,
    pub metadata:   EnvironmentMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameIdentity {
    pub exe_hash:       String,              // SHA256, primary key everywhere
    pub exe_name:       String,
    pub steam_app_id:   Option<u32>,
    pub detected_name:  Option<String>,
    pub dx_version:     DirectXVersion,      // from PE import table, static
    pub pe_arch:        PeArchitecture,      // x86 | x86_64
    pub anti_cheat:     Vec<AntiCheat>,
    pub engine_hint:    Option<GameEngine>,
    pub has_video_cutscenes: Option<bool>,   // WMF requirement signal
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectXVersion {
    D3D8, D3D9, D3D10, D3D11, D3D12, Vulkan, OpenGL, Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AntiCheat {
    EasyAntiCheat { linux_supported: bool },
    BattlEye      { linux_supported: bool },
    Denuvo,
    VMProtect,
    GameGuard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameEngine {
    UnrealEngine4, UnrealEngine5,
    Unity,
    Source, Source2,
    REEngine,        // Capcom, very specific behavior
    IDAEngine,       // id Software
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub gpu:    GpuProfile,
    pub cpu:    CpuProfile,
    pub memory: MemoryProfile,
    pub kernel: KernelProfile,
    pub display: DisplayProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProfile {
    pub vendor:          GpuVendor,
    pub model:           String,
    pub vram_mb:         u32,
    pub driver_version:  SemVer,
    pub vulkan_version:  Option<SemVer>,
    pub driver_type:     DriverType,         // NvidiaProprietary | Mesa | IntelANV
    pub features:        GpuFeatureSet,
    pub is_laptop_gpu:   bool,               // gates thermal-risky tweaks
    // NVIDIA-specific: populated by nvidia-smi, None for non-NVIDIA
    pub nvidia_vbios_max_clock_mhz: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuFeatureSet {
    pub vulkan_1_2:      bool,
    pub vulkan_1_3:      bool,
    pub ray_tracing:     bool,
    pub mesh_shaders:    bool,
    pub resizable_bar:   bool,
    pub dx12_feature_level: Option<String>,  // "12_0" | "12_1" | "12_2"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuProfile {
    pub vendor:            CpuVendor,
    pub model:             String,
    pub physical_cores:    u32,
    pub logical_cores:     u32,
    pub base_freq_mhz:     u32,
    pub supports_avx2:     bool,
    pub supports_avx512:   bool,
    pub is_laptop_cpu:     bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelProfile {
    pub version:               KernelVersion,
    pub has_futex2:            bool,    // fsync support, kernel >= 5.16
    pub has_fsync:             bool,    // proton fsync patchset
    pub vm_max_map_count:      u64,     // current value
    pub thp_mode:              ThpMode, // always | madvise | never
    pub split_lock_mitigate:   bool,
    pub sched_autogroup:       bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub server: DisplayServer,           // X11 | Wayland
    pub primary_res: (u32, u32),
    pub refresh_hz: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicsConfig {
    pub translation_layer:  TranslationLayer,
    pub dxvk_version:       Option<SemVer>,
    pub vkd3d_version:      Option<SemVer>,
    pub dxvk_config:        DxvkConfig,
    pub mangohud:           Option<MangoHudConfig>,
    pub gamemode:           bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TranslationLayer {
    Dxvk,           // D3D9/10/11 → Vulkan
    Vkd3dProton,    // D3D12 → Vulkan
    WineOpenGL,     // OpenGL passthrough
    Native,         // Vulkan or native Linux
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DxvkConfig {
    pub async_compile:   bool,       // disable if VAC/EAC present
    pub frame_limit:     Option<u32>,
    pub hud:             DxvkHud,
    pub state_cache:     bool,       // always true
    pub config_path:     PathBuf,    // per-game dxvk.conf
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub runner_type:    RunnerType,
    pub version:        SemVer,      // exact version, always latest stable
    pub install_path:   PathBuf,
    pub verified:       bool,        // SHA256 verified on install
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunnerType {
    ProtonGE,        // Primary for v1: GE-Proton (latest stable)
    WineGE,          // 32-bit-only fallback
    ProtonOfficial,  // Fallback when GE unavailable
    WineStaging,     // Last resort
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub backend:        AudioBackend,    // PipeWire | PulseAudio | ALSA
    pub server_rate:    u32,             // detected from server
    pub wine_driver:    WineAudioDriver, // must match backend
    pub latency_ms:     Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixConfig {
    pub path:            PathBuf,        // ~/.local/share/play/prefixes/{hash}
    pub arch:            WineArch,       // Win32 | Win64
    pub windows_version: WindowsVersion, // reported to game
    pub dll_overrides:   Vec<DllOverride>,
    pub env_vars:        IndexMap<String, String>, // ordered for determinism
    pub large_address_aware: bool,       // for 32-bit games with >2GB assets
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTuning {
    // Each Option = None means "no change needed/safe", Some = "apply this"
    pub vm_max_map_count:     Option<u64>,  // hardware-proportional, never hardcoded
    pub thp_mode:             Option<ThpMode>,
    pub sched_autogroup:      Option<bool>,
    pub split_lock_mitigate:  Option<bool>,
    pub ulimit_nofile:        Option<u64>,
    pub cpu_governor:         Option<CpuGovernor>,    // session-scoped
    pub gpu_perf_mode:        Option<GpuPerfMode>,    // session-scoped, NVIDIA v1
    pub nvidia_clock_lock_mhz: Option<u32>,           // computed, never hardcoded
    pub esync:                bool,
    pub fsync:                bool,
    pub gamemode:             bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchConfig {
    pub exe_path:     PathBuf,
    pub working_dir:  PathBuf,
    pub args:         Vec<String>,
    pub env:          IndexMap<String, String>,
    pub pre_launch:   Vec<HookCommand>,
    pub post_exit:    Vec<HookCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentMetadata {
    pub schema_version:    u32,          // bump on breaking changes, never reset
    pub created_at:        DateTime<Utc>,
    pub last_run:          Option<DateTime<Utc>>,
    pub tool_version:      SemVer,
    pub resolution_source: ResolutionSource,
    pub decisions:         Vec<ResolutionDecision>, // audit trail
}

/// Every decision the Planning module makes is recorded here.
/// This is what users see in verbose mode and what goes into bug reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionDecision {
    pub field:      String,      // "runner.version"
    pub chosen:     String,      // "GE-Proton9-27"
    pub reason:     String,      // "Latest stable GE-Proton; 9.27 fixes Cyberpunk VKD3D regression"
    pub confidence: Confidence,
    pub source:     DecisionSource,
}
```

## Error Model

```rust
/// Never use anyhow for user-facing errors. Define the taxonomy explicitly.
/// Every variant carries the context needed for a helpful error message.
#[derive(Debug, thiserror::Error)]
pub enum PlayError {
    #[error("Binary analysis failed for {path}: {reason}")]
    BinaryAnalysis { path: PathBuf, reason: String },

    #[error("Hardware detection incomplete: {component} could not be determined")]
    HardwareDetection { component: String },

    #[error("Anti-cheat {name} is present and has no Linux support. \
             Check https://areweanticheatyet.com for status updates.")]
    AntiCheatBlocked { name: String },

    #[error("Runner download failed: {url} (attempt {attempt}/3): {reason}")]
    RunnerDownload { url: String, attempt: u8, reason: String },

    #[error("Runner checksum mismatch. Expected {expected}, got {actual}. \
             File deleted for safety.")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Package manager {pm} failed to install {package}: {stderr}")]
    PackageInstall { pm: String, package: String, stderr: String },

    #[error("Wine prefix creation failed at {prefix_path}: {reason}")]
    PrefixCreation { prefix_path: PathBuf, reason: String },

    #[error("Sysctl write to {key} failed (requires elevated helper): {reason}")]
    SysctlWrite { key: String, reason: String },

    #[error("Game launched but crashed after {seconds}s. \
             Log: {log_path}. Consider filing a report with --report.")]
    GameCrash { seconds: u32, log_path: PathBuf },

    #[error("Rollback failed for {key}: {reason}. \
             Manual restoration: write '{previous_value}' to {path}")]
    RollbackFailed { key: String, reason: String, previous_value: String, path: PathBuf },

    #[error("State file corrupted at {path}. Run `play --reset {game}` to start fresh.")]
    StateCorrupted { path: PathBuf },

    #[error("Unsupported distro: {distro}. Supported: Ubuntu 22.04+, \
             Fedora 38+, Arch, Debian 12+")]
    UnsupportedDistro { distro: String },

    #[error("Insufficient VRAM: game likely requires ~{required_mb}MB, \
             detected {available_mb}MB")]
    InsufficientVram { required_mb: u32, available_mb: u32 },
}
```
</data_model>

---

<decision_logic>
# Decision Engine: If-Then Logic

All logic here is compiled Rust. No runtime AI calls. No network lookups during
game launch. The database cache is consulted offline.

## The Runner Selection Tree (implement as match on typed enums, never strings)

```
INPUT: DirectXVersion + AntiCheat[] + GameEngine + has_video_cutscenes + GpuVendor

1. HARD GATES (fail fast, no runner will help):
   AntiCheat::EasyAntiCheat { linux_supported: false } → PlayError::AntiCheatBlocked
   AntiCheat::BattlEye      { linux_supported: false } → PlayError::AntiCheatBlocked

2. TRANSLATION LAYER (determined by DX version, not runner):
   D3D12        → TranslationLayer::Vkd3dProton
   D3D9/10/11   → TranslationLayer::Dxvk
   Vulkan       → TranslationLayer::Native
   OpenGL       → TranslationLayer::WineOpenGL
   D3D8         → TranslationLayer::Dxvk (DXVK handles D3D8 via d3d8.dll)

3. DXVK ASYNC DISABLE CONDITIONS:
   AntiCheat contains Denuvo OR EasyAntiCheat (linux_supported: true)
   → DxvkConfig.async_compile = false

4. RUNNER TYPE SELECTION:
   has_video_cutscenes = true   → RunnerType::ProtonGE  (WMF patches)
   GameEngine = REEngine        → RunnerType::ProtonGE  (specific patches exist)
   dx_version = D3D12           → RunnerType::ProtonGE  (better VKD3D integration)
   pe_arch = x86 (32-bit)       → RunnerType::ProtonGE  (32-bit prefix support)
   default                      → RunnerType::ProtonGE  (GE is always safer default)

5. RUNNER VERSION SELECTION (version matters — this is not optional):
   - Query play-db cache for game-specific version_min constraints first.
   - If db has a version_min entry (e.g. "9.10"), use that as the floor.
   - ALWAYS select the latest stable GE-Proton release that meets the floor.
   - Latest stable = highest version tag in the local db runner manifest
     (runners.toml in play-db cache, updated on --update-db).
   - NEVER hardcode a version string. NEVER default to "latest" without
     recording the resolved concrete version in ResolutionDecision.
   - If no db entry exists, select the most recent stable GE-Proton release.
   - Record the specific version chosen: "GE-Proton9-27" not "ProtonGE latest"

6. WINDOWS VERSION REPORTED TO GAME:
   D3D12        → WindowsVersion::Win10
   D3D11        → WindowsVersion::Win10
   D3D9 + old   → WindowsVersion::Win7  (some old games reject Win10)
   default      → WindowsVersion::Win10

7. FSYNC vs ESYNC:
   kernel.has_futex2 = true    → fsync=true, esync=false
   kernel.has_futex2 = false   → fsync=false, esync=true
                                  + check ulimit_nofile >= 524288
                                  + if not: add ulimit raise to plan

8. vm.max_map_count — HARDWARE-PROPORTIONAL, NEVER HARDCODED:
   Compute target value from (RAM_mb + VRAM_mb) combined addressable memory:

   combined_mb = hardware.memory.total_mb + hardware.gpu.vram_mb as u64

   match combined_mb {
       0..=16_384         => 2_097_152,         // SteamOS baseline
       16_385..=32_768    => 8_388_608,          // 4× baseline
       32_769..=65_536    => 16_777_216,         // 8× baseline (Intel HX class)
       _                  => 2_147_483_642,      // MAX_INT-5, SteamOS max
   }

   Only add to plan if current value < target. Write target to rollback manifest
   BEFORE changing. The rationale displayed to user must include the computed
   value and the system class it maps to.

9. NVIDIA GPU CLOCK LOCK — COMPUTED, NEVER HARDCODED:
   // Common guides hardcode -lgc 1530,1530. This is WRONG for most GPUs.
   // 1530 MHz is below boost clock on RTX 3080+ and below base on RTX 4090.
   // Correct: 95% of VBIOS max, rounded to nearest 15 MHz step.

   fn compute_nvidia_lock_clock(vbios_max_mhz: u32) -> u32 {
       ((vbios_max_mhz as f32 * 0.95) as u32 / 15) * 15
   }
   // RTX 4090: max=2520 → lock=2385
   // RTX 3080: max=1710 → lock=1620
   // RTX 3050: max=1777 → lock=1680
   // Source: nvidia-smi --query-gpu=clocks.max.gr --format=csv,noheader

   Stored in SystemTuning.nvidia_clock_lock_mhz.
   Applied via play-helper: nvidia-smi -lgc {value},{value}
   Restored via GpuPerfGuard Drop: nvidia-smi -rgc

10. SYSTEM TWEAKS (each is a TweakConstraint check before adding to plan):
    Always evaluate vm_max_map_count (hardware-proportional as above)
    RAM >= 16384 MB: evaluate thp_mode → madvise
    x86 CPU: evaluate split_lock_mitigate → 0
    Wine/Proton active: evaluate sched_autogroup → 0
    NVIDIA GPU + NOT laptop: evaluate gpu_perf_mode + clock lock
    NOT laptop, NOT battery: evaluate cpu_governor → performance

11. VKD3D FEATURE LEVEL:
    GpuFeatureSet.dx12_feature_level is Some(level) → set VKD3D_FEATURE_LEVEL env
    Otherwise: leave unset (VKD3D auto-detects, safer than wrong explicit value)

12. LARGE ADDRESS AWARE:
    pe_arch = x86 (32-bit) AND vram_mb > 2048 → PrefixConfig.large_address_aware = true
```

## TweakConstraint: Gate every risky tweak behind typed constraints

```rust
pub struct TweakConstraint {
    pub tweak:                   SystemTweak,
    pub min_ram_mb:              Option<u32>,
    pub kernel_version_min:      Option<KernelVersion>,
    pub gpu_vendor_required:     Option<GpuVendor>,  // None = all vendors
    pub gpu_vendor_exclusions:   Vec<GpuVendor>,
    pub cpu_arch_required:       Option<CpuArch>,
    pub requires_desktop:        bool,    // false = applies to laptops too
    pub reversible:              bool,
    pub reboot_required:         bool,
    pub reboot_resets:           bool,    // sysctl resets on reboot anyway
    pub risk_level:              RiskLevel,
    pub rationale:               &'static str,
}

// If constraints not satisfied: tweak is INVISIBLE in the plan. Not disabled. INVISIBLE.
// Only show tweaks that are safe to apply on this specific hardware.
fn evaluate_tweak(tweak: &TweakConstraint, hw: &HardwareProfile) -> TweakDecision {
    if !constraints_satisfied(tweak, hw) {
        return TweakDecision::NotApplicable; // silent, never shown
    }
    TweakDecision::Include {
        reason: tweak.rationale,
        risk: tweak.risk_level,
    }
}
```
</decision_logic>

---

<system_tweaks_reference>
# Complete Optimization Suite

Implement ALL of these. Sorted by Class and impact.

## Class A — Session-scoped, restored via Drop, no confirmation needed

| Tweak | Implementation | Condition |
|-------|---------------|-----------|
| fsync | `PROTON_ENABLE_FSYNC=1` env var | kernel >= 5.16 |
| esync fallback | `PROTON_ENABLE_ESYNC=1` + ulimit | kernel < 5.16 |
| CPU governor | Write `performance` to `/sys/.../scaling_governor` via GovernorGuard | not laptop, not battery |
| GameMode | `LD_PRELOAD=libgamemodeauto.so` or `gamemoderun %command%` | gamemode installed |
| DXVK state cache | `DXVK_STATE_CACHE_PATH={prefix}/cache` | DX9/10/11 |
| DXVK async | `DXVK_ASYNC=1` | no VAC/EAC |
| VKD3D feature level | `VKD3D_FEATURE_LEVEL=12_1` | D3D12, GPU supports it |
| MangoHud inject | `MANGOHUD=1 MANGOHUD_CONFIG=...` | mangohud installed |
| WINE_LARGE_ADDRESS_AWARE | `WINE_LARGE_ADDRESS_AWARE=1` | pe_arch = x86 |
| DXVK_HUD | configurable per user preference | optional |
| PROTON_NO_ESYNC | set when using fsync | prevents double-activation |

## Class B — Persistent, written to rollback manifest first, confirm once per game

| Tweak | Implementation | Condition |
|-------|---------------|-----------|
| vm.max_map_count | sysctl via helper binary, hardware-proportional value | always if current < target |
| kernel.sched_autogroup | sysctl via helper | Wine/Proton active |
| kernel.split_lock_mitigate | sysctl via helper | x86 arch only |
| THP → madvise | Write to `/sys/kernel/mm/transparent_hugepage/enabled` | RAM >= 16GB |
| ulimit nofile | `/etc/security/limits.d/play-{game}.conf` | esync required |
| Audio sample rate | Wine registry in prefix via `wine reg add` | always |
| Per-game Wine prefix | Create `~/.local/share/play/prefixes/{hash}/` | always |
| Windows version in prefix | `wine reg add HKLM\...` | determined by dx_version |

## Class C — Hardware-conditional, explicit user consent per invocation

### NVIDIA (v1 — fully implemented)

| Tweak | Implementation | Condition | Desktop Only |
|-------|---------------|-----------|-------------|
| NVIDIA persistence mode | `nvidia-smi -pm 1` via helper, GpuPerfGuard Drop restore | NVIDIA GPU | YES |
| NVIDIA clock lock | `nvidia-smi -lgc {computed},{computed}` via helper, restored with `-rgc` | NVIDIA GPU, NOT laptop | YES |

### AMD (Phase 2 — stubbed, architecture ready)

```rust
// AMD perf tweaks are NOT implemented in v1.
// The GpuVendor::AMD arm in the TweakRegistry returns TweakDecision::NotApplicable
// for all Class C GPU tweaks. This is NOT a panic, NOT an error — silent skip.
// When Phase 2 begins:
//   - amd_perf_level_high: sysfs power_dpm_force_performance_level → "high"
//   - Drop guard restores to "auto"
// The GpuPerfGuard enum will gain an Amd variant. No other code changes.
impl GpuPerfGuard {
    pub fn set_max(vendor: &GpuVendor, hw: &GpuProfile) -> Result<Self, PlayError> {
        match vendor {
            GpuVendor::NVIDIA if !hw.is_laptop_gpu => { /* implemented */ }
            GpuVendor::AMD    if !hw.is_laptop_gpu => {
                // Phase 2: write to power_dpm_force_performance_level
                tracing::warn!(event = "phase2_stub", tweak = "amd_perf_level");
                Ok(Self::Noop) // silent no-op, not an error
            }
            _ => Ok(Self::Noop),
        }
    }
}
```

### Recommendation-only (never auto-apply)

| Tweak | Implementation | Condition |
|-------|---------------|-----------|
| Optimized kernel | RECOMMEND ONLY, never auto-install | binary distro, user consent |

## NEVER touch (document why in code comments)

- BIOS/UEFI settings
- GPU overclocking beyond the computed 95% clock lock
- MCE behavior
- Direct kernel module modification
- System-wide mitigation disable (`mitigations=off`)
- Any operation requiring full root on the main process

## Drop Guard Pattern (use for ALL session-scoped changes)

```rust
// This pattern must be used for every reversible session change.
// Rust guarantees drop() runs even on panic. This is not optional.
pub struct GovernorGuard {
    cores: Vec<(PathBuf, String)>,  // (sysfs_path, previous_value)
}

impl GovernorGuard {
    pub fn set_performance(hw: &HardwareProfile) -> Result<Self, PlayError> {
        if hw.cpu.is_laptop_cpu { return Ok(Self { cores: vec![] }); } // no-op guard
        let paths = glob("/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor")?;
        let cores = paths.map(|p| {
            let prev = fs::read_to_string(&p)?.trim().to_string();
            fs::write(&p, "performance")?;
            Ok((p, prev))
        }).collect::<Result<Vec<_>, PlayError>>()?;
        tracing::info!(event = "tweak_applied", tweak = "cpu_governor", value = "performance");
        Ok(Self { cores })
    }
}

impl Drop for GovernorGuard {
    fn drop(&mut self) {
        for (path, prev) in &self.cores {
            if let Err(e) = fs::write(path, prev) {
                // Log but never panic in Drop
                tracing::error!(event = "restore_failed", path = ?path, error = %e);
            }
        }
        tracing::info!(event = "tweak_restored", tweak = "cpu_governor");
    }
}

// The Orchestrator holds all guards together:
struct ActiveGuards {
    governor: Option<GovernorGuard>,
    gpu_perf: Option<GpuPerfGuard>,
    // Add new session-scoped guards here as the tool grows
}
// When game exits → ActiveGuards drops → all restores run in sequence
```
</system_tweaks_reference>

---

<database_design>
# Community Database Design

## Repository Structure (separate repo: play-db)

```
play-db/
├── schema/
│   ├── entry.schema.json       # JSON Schema, CI validates all entries against it
│   ├── report.schema.json
│   └── CHANGELOG.md            # schema version history, migration notes
├── entries/
│   └── {exe_hash[0:2]}/       # first 2 chars of hash (git-object-store pattern)
│       └── {exe_hash}/
│           ├── default.toml   # primary entry (all GPU vendors)
│           └── nvidia.toml    # GPU-conditional variant (AMD stub for Phase 2)
├── runners.toml               # canonical runner version manifest
│                              # lists all available GE-Proton versions + SHA256
│                              # updated by maintainer CI, never runtime-fetched
├── reports/                   # raw reports, append-only, never edited post-submit
│   └── {year}/{month}/
│       └── {report_id}.toml
└── tools/
    ├── validate.py            # run in CI on every PR
    ├── compute_confidence.py  # recomputes confidence scores nightly
    └── promote_report.rs      # Rust binary: report → entry (in play-db-tools workspace)
```

## runners.toml — Runner Version Manifest

```toml
# runners.toml — updated by CI when new GE-Proton is released
# This file is what play reads to select "latest stable"
# play NEVER fetches runner metadata from the internet at game launch time

[proton_ge]
latest_stable = "GE-Proton9-27"

[[proton_ge.releases]]
version    = "GE-Proton9-27"
url        = "https://github.com/GloriousEggroll/proton-ge-custom/releases/..."
sha256     = "abc123..."
released   = "2025-02-15"
notes      = "Fixes VKD3D regression in DX12 games with Ray Tracing enabled"

[[proton_ge.releases]]
version    = "GE-Proton9-26"
url        = "..."
sha256     = "def456..."
released   = "2025-01-28"
notes      = ""
```

## Entry Schema (TOML)

```toml
[entry]
id              = "uuid-v4"
schema_version  = 1           # bump only on breaking changes

[entry.identity]
exe_hash        = "sha256:..."
exe_name        = "CyberpunkGame.exe"
steam_app_id    = 1091500
canonical_name  = "Cyberpunk 2077"

# ALL conditions must match for entry to apply.
# Missing fields match ANY value (open constraint).
[entry.conditions]
dx_version          = "D3D12"
gpu_vendor          = ["NVIDIA"]     # v1: NVIDIA only entries; AMD Phase 2
gpu_vulkan_min      = "1.3"
kernel_min          = "6.0"
runner_type         = "ProtonGE"
runner_version_min  = "9.10"         # floor version — play selects latest above this
anti_cheat          = []             # must have NONE of these to apply

[entry.outcome]
result       = "Optimal"       # Broken | Degraded | Playable | Optimal
confidence   = 0.91            # COMPUTED, never manually set
degradations = ["shader_stutter_first_run"]
blockers     = []

[entry.configuration]
[entry.configuration.runner]
type        = "ProtonGE"
version_min = "9.10"           # play will resolve to latest stable >= this

[entry.configuration.graphics]
translation_layer = "Vkd3dProton"
dxvk_async        = false

[entry.configuration.system]
vm_max_map_count  = 16777216   # hardware class: Intel HX + discrete
fsync             = true

[entry.configuration.prefix]
windows_version = "Win10"
dll_overrides   = []

[entry.configuration.env_vars]
VKD3D_FEATURE_LEVEL = "12_1"

[entry.metadata]
created           = "2024-11-01"
last_validated    = "2025-01-15"   # auto-expires: entry becomes "Unverified" after 90 days
contributor_count = 47
report_ids        = ["report-uuid-1", "report-uuid-2"]
maintainer_notes  = "Confirmed stable on GE-Proton9-20 through 9-27"
```

## Confidence Score Formula (computed, never assigned)

```python
# compute_confidence.py — runs nightly via CI
def compute_confidence(reports: List[Report], entry: Entry) -> float:
    now = datetime.utcnow()

    recency  = recency_weight(entry.last_validated, now)    # decays to 0.3 after 180 days
    volume   = volume_weight(len(reports))                  # log scale, saturates at ~50 reports
    diversity = hardware_diversity(reports)                 # unique GPU models / total reports
    stability = version_stability(reports)                  # consistent across runner versions?

    return recency * volume * diversity * stability  # all factors in [0, 1]
    # Result < 0.3 → entry becomes "Unverified" automatically
    # Result > 0.8 → entry gets "Verified" badge
```

## Local Cache Strategy

```
~/.local/share/play/db/
├── cache.toml          # metadata: last_sync, entry_count, db_version
├── runners.toml        # local copy of runner version manifest
├── entries/            # local mirror of play-db/entries/
└── reports/            # pending reports awaiting submission
```

Update strategy: sync on first run of the day, or `--update-db` flag.
Offline mode: use cache, degrade gracefully, warn user about cache age.
Cache is never required: if absent, fall back to heuristic-only planning.

## Hardware-Aware Query Algorithm

```rust
// Not a simple lookup. A scored match.
pub fn query_database(
    identity: &GameIdentity,
    hardware: &HardwareProfile,
    db: &LocalDatabase,
) -> Option<DatabaseMatch> {
    let candidates = db.entries_for_hash(&identity.exe_hash);

    candidates
        .filter_map(|entry| score_entry(entry, hardware))
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
}

fn score_entry(entry: &DbEntry, hw: &HardwareProfile) -> Option<ScoredEntry> {
    // Hard fail: explicit exclusions
    if entry.conditions.anti_cheat.iter().any(|ac| hw_has_ac(hw, ac)) {
        return None;
    }
    // Score by condition match quality
    let score = entry.confidence
        * vendor_match_score(&entry.conditions.gpu_vendor, &hw.gpu.vendor)
        * kernel_match_score(&entry.conditions.kernel_min, &hw.kernel.version)
        * runner_compat_score(&entry.conditions.runner_version_min);

    Some(ScoredEntry { entry, score })
}
// An entry verified on RTX 3080 applies to RTX 4070 with reduced score.
// That's better than returning nothing.
```
</database_design>

---

<binary_analysis>
# Static Binary Analysis (DetectionModule)

## PE Parser (goblin crate — no process execution)

```rust
use goblin::pe::PE;
use sha2::{Sha256, Digest};

pub struct BinaryAnalysis {
    pub hash:           String,
    pub dx_version:     DirectXVersion,
    pub pe_arch:        PeArchitecture,
    pub anti_cheat:     Vec<AntiCheat>,
    pub engine_hint:    Option<GameEngine>,
    pub has_bink_video: bool,  // RADGame Tools = likely WMF cutscenes
}

pub fn analyze_binary(path: &Path) -> Result<BinaryAnalysis, PlayError> {
    let bytes = fs::read(path)
        .map_err(|e| PlayError::BinaryAnalysis { path: path.into(), reason: e.to_string() })?;

    // Hash before parsing — we need it regardless of parse success
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));

    let pe = PE::parse(&bytes)
        .map_err(|e| PlayError::BinaryAnalysis { path: path.into(), reason: e.to_string() })?;

    let imports: Vec<String> = pe.libraries.iter()
        .map(|s| s.to_lowercase())
        .collect();

    // DX version: check most specific first
    let dx_version = if imports.contains(&"d3d12.dll".into())       { DirectXVersion::D3D12 }
        else if imports.contains(&"d3d11.dll".into())                { DirectXVersion::D3D11 }
        else if imports.contains(&"d3d10.dll".into())                { DirectXVersion::D3D10 }
        else if imports.contains(&"d3d9.dll".into())                 { DirectXVersion::D3D9  }
        else if imports.contains(&"d3d8.dll".into())                 { DirectXVersion::D3D8  }
        else if imports.contains(&"vulkan-1.dll".into())             { DirectXVersion::Vulkan }
        else if imports.contains(&"opengl32.dll".into())             { DirectXVersion::OpenGL }
        else                                                         { DirectXVersion::Unknown };

    // Anti-cheat: DLL name signatures
    let mut anti_cheat = vec![];
    if imports.iter().any(|i| i.contains("easyanticheat"))    {
        anti_cheat.push(AntiCheat::EasyAntiCheat { linux_supported: false });
    }
    if imports.iter().any(|i| i.contains("battleye"))         {
        anti_cheat.push(AntiCheat::BattlEye { linux_supported: false });
    }

    // Engine hints from known DLL patterns
    let engine_hint = detect_engine(&imports);

    // Bink video = likely WMF dependency
    let has_bink_video = imports.iter().any(|i| i.contains("bink") || i.contains("rad"));

    let pe_arch = match pe.is_64 { true => PeArchitecture::X86_64, false => PeArchitecture::X86 };

    Ok(BinaryAnalysis { hash, dx_version, pe_arch, anti_cheat, engine_hint, has_bink_video })
}

fn detect_engine(imports: &[String]) -> Option<GameEngine> {
    if imports.iter().any(|i| i.contains("phonon"))      { return Some(GameEngine::UnrealEngine4); }
    if imports.iter().any(|i| i.contains("mt_framework")) { return Some(GameEngine::REEngine); }
    if imports.iter().any(|i| i.contains("unityplayer") || i.contains("mono")) {
        return Some(GameEngine::Unity);
    }
    if imports.iter().any(|i| i == "tier0.dll" || i == "vstdlib.dll") {
        return Some(GameEngine::Source);
    }
    None
}
```
</binary_analysis>

---

<hardware_detection>
# Hardware Detection (DetectionModule)

## Detection Strategy Per Component

```
GPU vendor: lspci -mm | grep -i "VGA\|3D" → parse vendor ID
            Fallback: /sys/class/drm/card0/device/vendor (hex ID)
            NVIDIA confirm: check /proc/driver/nvidia/version exists

GPU model: lspci output, or /sys/class/drm/card0/device/product_name

GPU VRAM:  NVIDIA: nvidia-smi --query-gpu=memory.total --format=csv,noheader
           Intel:  vulkaninfo | grep "maxMemoryAllocationCount"
           AMD (Phase 2): /sys/class/drm/card0/device/mem_info_vram_total

Driver version:
           NVIDIA: nvidia-smi --query-gpu=driver_version --format=csv,noheader
           AMD (Phase 2): glxinfo -B | grep "OpenGL version" | parse Mesa version

NVIDIA VBIOS max clock (required for computed clock lock):
           nvidia-smi --query-gpu=clocks.max.gr --format=csv,noheader
           Parse as u32 MHz. Store in GpuProfile.nvidia_vbios_max_clock_mhz.
           If this query fails: clock lock tweak becomes NotApplicable.

Vulkan:    vulkaninfo --json | jq '.properties.apiVersion'
           CRITICAL: parse as packed uint32: (major << 22) | (minor << 12) | patch

Kernel:    fs::read_to_string("/proc/version") → parse semver
           futex2: kernel >= 5.16 (check precisely, not approximately)
           sched_autogroup current: /proc/sys/kernel/sched_autogroup_enabled
           vm.max_map_count: /proc/sys/vm/max_map_count
           split_lock: /proc/sys/kernel/split_lock_mitigate

CPU:       fs::read_to_string("/proc/cpuinfo")
           physical_cores: count unique "core id" values
           logical_cores: count "processor" entries
           AVX2: flags line contains "avx2"
           laptop: check /sys/class/power_supply/BAT0/present == 1

Audio:     Check process list for "pipewire" or "pulseaudio" running
           PipeWire: pw-dump --json | parse defaultNodes sample rate
           PulseAudio: pactl info | grep "Default Sample Spec" | parse rate

Display:   Check $WAYLAND_DISPLAY (set → Wayland) or $DISPLAY (set → X11)
           Resolution: wlr-randr (Wayland) or xrandr (X11)

Distro:    /etc/os-release: ID, VERSION_ID, ID_LIKE
           Package manager: which apt / dnf / pacman / zypper (first found wins)

Laptop detection (gates thermal tweaks):
           /sys/class/power_supply/BAT0/present == 1 → is laptop
           /sys/class/power_supply/AC/online → current power state
```

## Detection Error Policy

Detection errors are NEVER fatal for components that have safe defaults.
Record what couldn't be detected, use conservative defaults, log the gap.
Only GPU vendor and DX version detection failures are hard errors
(we cannot proceed without knowing what we're translating from/to).
</hardware_detection>

---

<project_structure>
# Project File Structure

```
play/
├── Cargo.toml
├── Cargo.lock              # committed — reproducible builds
├── DESIGN.md               # Plan→Confirm→Execute model, module contracts, error philosophy
├── CONTRIBUTING.md
├── LICENSE                 # GPL-3.0
│
├── crates/
│   ├── play-cli/           # Binary crate: main.rs, CLI arg parsing (clap)
│   │   └── src/
│   │       └── main.rs
│   │
│   ├── play-core/          # Library crate: all modules, orchestrator, data model
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models/     # GameEnvironment and all types
│   │       │   ├── mod.rs
│   │       │   ├── environment.rs
│   │       │   ├── hardware.rs
│   │       │   └── errors.rs
│   │       ├── tweaks/     # THE REGISTRY — each tweak is one file
│   │       │   ├── mod.rs  # TweakRegistry, Tweak struct, TweakDecision
│   │       │   ├── fsync.rs
│   │       │   ├── sched_autogroup.rs
│   │       │   ├── vm_max_map.rs
│   │       │   ├── split_lock.rs
│   │       │   ├── cpu_governor.rs
│   │       │   ├── gpu_perf_nvidia.rs  # NVIDIA only; AMD stub returns NotApplicable
│   │       │   ├── audio_rate.rs
│   │       │   └── ... (one file per tweak, forever)
│   │       ├── modules/
│   │       │   ├── mod.rs
│   │       │   ├── detection.rs
│   │       │   ├── planning.rs
│   │       │   ├── execution/
│   │       │   │   ├── mod.rs
│   │       │   │   ├── packages.rs
│   │       │   │   ├── runner.rs
│   │       │   │   ├── prefix.rs
│   │       │   │   └── system.rs   # iterates TweakRegistry — no tweak logic
│   │       │   └── validation.rs
│   │       ├── orchestrator.rs
│   │       ├── database.rs
│   │       ├── guards/     # Drop-based restore guards
│   │       │   ├── mod.rs
│   │       │   ├── governor.rs
│   │       │   └── gpu_perf.rs     # GpuPerfGuard::Nvidia implemented; ::Amd = Noop
│   │       ├── binary/     # PE analysis
│   │       │   └── pe.rs
│   │       └── state/      # Checkpoint, rollback manifest, state files
│   │           └── mod.rs
│   │
│   └── play-helper/        # Privileged helper binary (minimal, auditable)
│       └── src/
│           └── main.rs     # Accepts JSON commands on stdin, executes, returns JSON
│
├── tests/
│   ├── integration/        # Full module pipeline tests with mocked hardware
│   └── fixtures/
│       ├── proc/           # Mocked /proc content for testing
│       ├── sys/            # Mocked /sys content for testing
│       └── binaries/       # Known test PE files (tiny, legally distributable)
│
└── .github/
    └── workflows/
        ├── ci.yml          # clippy + test + fmt on PR
        └── cross-distro.yml # Matrix test on Ubuntu, Fedora, Arch containers
```

## Cargo.toml Dependencies

```toml
[workspace]
members = ["crates/play-cli", "crates/play-core", "crates/play-helper"]

# play-core/Cargo.toml
[dependencies]
# Data
serde       = { version = "1", features = ["derive"] }
toml        = "0.8"
indexmap    = { version = "2", features = ["serde"] }
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4", "serde"] }
semver      = "1"

# Binary analysis
goblin      = "0.8"
sha2        = "0.10"

# System
nix         = { version = "0.27", features = ["process", "resource", "sysinfo"] }
glob        = "0.3"
sysinfo     = "0.30"

# CLI + Output
clap        = { version = "4", features = ["derive"] }
indicatif   = "0.17"    # progress bars
console     = "0.15"    # terminal colors, cursor control
dialoguer   = "0.11"    # user confirmation prompts

# Logging + Tracing
tracing              = "0.1"
tracing-subscriber   = { version = "0.3", features = ["env-filter", "json"] }
tracing-appender     = "0.2"

# Networking (runner downloads)
ureq        = { version = "2", features = ["json"] }  # blocking, no tokio needed v1
tempfile    = "3"

# Error handling
thiserror   = "1"

[dev-dependencies]
tempfile    = "3"
assert_cmd  = "2"
predicates  = "3"

[profile.release]
opt-level   = 3
lto         = "thin"
codegen-units = 1
strip       = true
```
</project_structure>

---

<privileged_helper>
# play-helper: Privileged Operations

## Why a Separate Binary

The main `play` process must NEVER run as root. A separate small binary
(`play-helper`) handles the narrow set of operations requiring elevated privilege.
This binary is installed setuid or called via polkit. It is intentionally minimal
and auditable. It does EXACTLY five things and refuses everything else.

## play-helper Command Interface

```json
// Stdin: JSON command. Stdout: JSON result. Stderr: error details.
// play-helper ONLY accepts commands on stdin. Never command-line args with values.

// Write sysctl
{ "cmd": "sysctl_write", "key": "vm.max_map_count", "value": "16777216" }

// Read sysctl (to capture rollback value)
{ "cmd": "sysctl_read", "key": "vm.max_map_count" }

// Write to sysfs path (THP mode, etc.)
{ "cmd": "sysfs_write", "path": "thp_mode", "value": "madvise" }
// "path" is a KEY into a hardcoded whitelist inside play-helper, NOT a raw path

// NVIDIA persistence mode on/off
{ "cmd": "nvidia_pm", "enable": true }

// NVIDIA clock lock (set) — value is the computed MHz, e.g. 1680
{ "cmd": "nvidia_clock_lock", "mhz": 1680 }

// NVIDIA clock lock (reset / restore)
{ "cmd": "nvidia_clock_reset" }

// All other commands → immediate rejection with non-zero exit
```

## Sysfs Whitelist in play-helper

```rust
// play-helper/src/main.rs — the ENTIRE set of paths it will write to
// Adding to this list requires code review and justification comment
const SYSFS_WHITELIST: &[(&str, &str)] = &[
    ("thp_mode",       "/sys/kernel/mm/transparent_hugepage/enabled"),
    ("split_lock",     "/proc/sys/kernel/split_lock_mitigate"),
    ("autogroup",      "/proc/sys/kernel/sched_autogroup_enabled"),
    ("max_map_count",  "/proc/sys/vm/max_map_count"),
    // NOTE: AMD gpu_perf_level intentionally omitted until Phase 2
    // ("gpu_perf_level", "/sys/class/drm/card0/device/power_dpm_force_performance_level"),
];
```
</privileged_helper>

---

<state_management>
# State Files: The Complete Picture

## Directory Layout per Game

```
~/.local/share/play/
├── config.toml                        # global tool config
├── db/                                # local database cache
│   ├── cache.toml
│   ├── runners.toml                   # runner version manifest
│   └── entries/
├── runners/                           # downloaded + verified runners
│   └── GE-Proton9-27/
└── games/
    └── {exe_hash}/
        ├── environment.toml           # complete GameEnvironment
        ├── checkpoint.toml            # execution state for resumption
        ├── rollback.toml              # every persistent change, pre-change values
        ├── session.log                # tracing output, 3 rotated
        ├── metrics.toml               # post-launch FPS, VRAM, session duration log
        ├── prefix/                    # Wine prefix
        └── reports/
            └── {timestamp}.toml
```

## Rollback Manifest

```toml
# rollback.toml — written BEFORE each change, never after
schema_version = 2
game_hash      = "sha256:abc123..."
play_version   = "0.1.0"
created_at     = "2025-03-01T14:22:08Z"

[[changes]]
kind            = "Sysctl"
key             = "vm.max_map_count"
previous_value  = "65530"
current_value   = "16777216"
changed_at      = "2025-03-01T14:22:11Z"
reboot_resets   = true       # sysctl resets on reboot; --undo skips if already reset
undo_strategy   = "sysctl"  # "sysctl" | "sysfs" | "file_delete" | "package" | "registry"
undo_command    = "sysctl -w vm.max_map_count=65530"  # what --undo will run
verified_before = true       # pre-change value was read and confirmed before writing

[[changes]]
kind            = "Sysctl"
key             = "kernel.sched_autogroup_enabled"
previous_value  = "1"
current_value   = "0"
changed_at      = "2025-03-01T14:22:12Z"
reboot_resets   = true
undo_strategy   = "sysctl"
undo_command    = "sysctl -w kernel.sched_autogroup_enabled=1"
verified_before = true

[[changes]]
kind             = "PackageInstalled"
package          = "gamemode"
package_manager  = "dnf"
changed_at       = "2025-03-01T14:22:09Z"
undo_strategy    = "package"
undo_command     = "dnf remove -y gamemode"
# NOTE: packages are NOT auto-uninstalled on rollback.
# Record is informational. --undo for packages prompts the user.

[[changes]]
kind            = "WineRegistry"
prefix_path     = "~/.local/share/play/prefixes/sha256abc123"
key             = "HKCU\\Software\\Wine\\Drivers\\winepulse.drv"
value_name      = "DefaultSamplesPerSec"
previous_value  = ""           # key did not exist before
current_value   = "48000"
changed_at      = "2025-03-01T14:22:14Z"
reboot_resets   = false
undo_strategy   = "registry"
undo_command    = "wine reg delete HKCU\\... /v DefaultSamplesPerSec /f"
verified_before = false        # key was absent, not a read-modify-write
```
</state_management>

---

<testing_requirements>
# Testing Requirements

## What Must Be Tested

Every module function that touches /proc, /sys, or external binaries
MUST be testable with mocked filesystem content. Design detection functions
to accept an injectable path root:

```rust
// Good: testable
pub fn read_vm_max_map_count(proc_root: &Path) -> Result<u64, PlayError> {
    let path = proc_root.join("sys/vm/max_map_count");
    fs::read_to_string(&path)?.trim().parse()
        .map_err(|_| PlayError::HardwareDetection { component: "vm.max_map_count".into() })
}

// Bad: untestable
pub fn read_vm_max_map_count() -> Result<u64, PlayError> {
    fs::read_to_string("/proc/sys/vm/max_map_count")?.trim().parse()...
}
```

## Test Coverage Requirements

- PE analysis: test against fixture binaries for each DX version (Steps 1–7 in build sequence)
- Decision engine: unit test every branch with typed inputs
- vm.max_map_count formula: unit test all four RAM+VRAM tiers
- NVIDIA clock lock formula: unit test compute_nvidia_lock_clock for known GPU specs
- Sysctl rollback: verify rollback file written before sysctl changed
- GovernorGuard: verify restoration on Drop even after panic
- Database query: test scored matching with partial condition match
- Error messages: every PlayError variant produces a message with actionable text

## CI Matrix (GitHub Actions)

```yaml
strategy:
  matrix:
    container:
      - ubuntu:22.04
      - fedora:38
      - archlinux:latest
    # Hardware differences tested via mocked /sys content, not actual hardware
```
</testing_requirements>

---

<security_requirements>
# Security Requirements

## Threat Model

1. **Malicious game executable** providing paths designed for traversal
   → Mitigation: sanitize all paths from binary analysis, never use game-provided
     strings in path construction without canonicalization and prefix-checking

2. **Compromised runner download** (MITM or CDN compromise)
   → Mitigation: SHA256 verify every download against runners.toml manifest.
     The manifest is fetched over TLS from the canonical GitHub release and
     its hash is pinned in the db cache.

3. **Database entry injection** (malicious PR to play-db)
   → Mitigation: database entries only mutate GameEnvironment fields.
     They cannot execute arbitrary code. No script fields in the schema ever.
     CI validates every entry against the schema before merge.

4. **play-helper privilege escalation**
   → Mitigation: hardcoded command whitelist, hardcoded sysfs path whitelist,
     JSON-only interface, minimal binary, no dynamic dispatch, reviewed on change

5. **State file tampering**
   → Mitigation: state files are user-owned, in user home dir.
     The tool reads them but validates schema_version and required fields.
     A corrupted state file triggers `PlayError::StateCorrupted` with clear
     recovery instructions, not silent misbehavior.

## Input Sanitization Rules

```rust
// Every path derived from external input goes through this:
pub fn safe_game_path(input: &Path, allowed_root: &Path) -> Result<PathBuf, PlayError> {
    let canonical = input.canonicalize()
        .map_err(|_| PlayError::BinaryAnalysis {
            path: input.into(),
            reason: "path does not exist or is not accessible".into()
        })?;
    if !canonical.starts_with(allowed_root) {
        return Err(PlayError::BinaryAnalysis {
            path: input.into(),
            reason: "path traversal detected".into()
        });
    }
    Ok(canonical)
}
```
</security_requirements>

---

<implementation_guidance>
# Implementation Guidance

## Current Status: Steps 1–3 Complete

Steps 1–3 of the build sequence are done and passing tests:
- `models/environment.rs` — all types defined ✓
- `models/errors.rs` — all PlayError variants ✓
- `binary/pe.rs` — analyze_binary() with 17+ passing tests ✓

Continue from Step 4.

## Build Sequence — Steps 4 Onward

```
Step 4: modules/detection.rs
        — kernel, CPU, GPU, audio detection with injectable paths
        — NVIDIA: query vbios_max_clock_mhz via nvidia-smi, store in GpuProfile
        — detect_package_manager() → Box<dyn PackageManager>

Step 5: guards/governor.rs
        — GovernorGuard with Drop. Test Drop fires on panic.

Step 6: guards/gpu_perf.rs
        — GpuPerfGuard: Nvidia variant (nvidia-smi -pm 1 + -lgc {computed})
        — Amd variant returns Noop (Phase 2 stub, zero panic risk)
        — Drop restores: nvidia-smi -pm 0 + nvidia-smi -rgc

Step 7: tweaks/mod.rs + tweaks/*.rs
        — TweakRegistry, Tweak struct, TweakDecision
        — Register all tweaks from system_tweaks_reference
        — vm_max_map.rs: hardware-proportional formula, not hardcoded
        — gpu_perf_nvidia.rs: compute_nvidia_lock_clock(vbios_max)

Step 8: modules/planning.rs
        — Decision engine, TweakConstraint evaluation, ResolutionDecision
        — Runner version resolution: read runners.toml, apply db floor, select latest

Step 9: database.rs
        — Local cache loading, scored query algorithm
        — runners.toml loading and version comparison

Step 10: execution/packages.rs
         — PackageModule with distro detection

Step 11: execution/runner.rs
         — RunnerModule: download + SHA256 verify against runners.toml manifest

Step 12: execution/prefix.rs
         — PrefixModule: Wine prefix setup

Step 13: execution/system.rs
         — SystemModule: iterates TweakRegistry only — zero tweak-specific logic

Step 14: play-helper/src/main.rs
         — Minimal privileged binary
         — Add nvidia_clock_lock and nvidia_clock_reset commands

Step 15: orchestrator.rs
         — State machine, checkpoint system, module sequencing

Step 16: modules/validation.rs
         — Launch monitoring, GPU activity check, MangoHud metrics collection

Step 17: play-cli/src/main.rs
         — CLI surface: clap with all standard flags (--help, --version, --yes,
           --undo, --update-db, --verbose, --dry-run)
         — Plan display, user confirmation prompt
         — Post-session: write metrics.toml on clean exit; prompt --report on crash
```

## Code Quality Non-Negotiables

```toml
# .cargo/config.toml — enforced from day one
[build]
rustflags = [
    "-D", "warnings",
    "-D", "clippy::all",
    "-D", "clippy::pedantic",
    "-A", "clippy::module_name_repetitions",
]
```

Run before every commit:
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Logging Contract

Every significant event uses structured tracing fields, not format strings:

```rust
// Good
tracing::info!(
    event = "tweak_applied",
    tweak = "cpu_governor",
    value = "performance",
    previous = "schedutil",
    game_hash = %env.identity.exe_hash
);

// Bad
tracing::info!("Set CPU governor to performance for game {}", hash);
```

Log levels:
- `trace`: internal module state, every file read
- `debug`: detection values, decision scoring
- `info`: every state transition, every tweak applied, game launch
- `warn`: degraded fallback used, low-confidence database match
- `error`: failures, rollback events

## Dependency Injection Pattern

Every module takes its dependencies as constructor arguments, never reads
global state. This enables testing with mocked dependencies:

```rust
pub struct DetectionModule {
    proc_root: PathBuf,   // "/" in production, test fixture path in tests
    sys_root:  PathBuf,
    run_cmd:   Box<dyn CommandRunner>,  // real subprocess | mock in tests
}
```

## The Plan Display Format

This is what users see before confirming. A non-technical user must understand it.
A technical user must find it sufficient for debugging.

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  PLAY — Cyberpunk 2077
  Analyzed: CyberpunkGame.exe (sha256:a3f4…)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  CONFIGURATION
  ─────────────────────────────────────────────
  Runner        GE-Proton9-27
                ↳ Game uses WMF video cutscenes; GE includes required media patches
                ↳ 9.27 is latest stable (db floor: 9.10)

  Translation   VKD3D-Proton 2.11
                ↳ D3D12 detected in binary imports

  Sync          fsync
                ↳ Kernel 6.7.4 supports futex2

  SYSTEM CHANGES  (persistent, reversible with: play --undo CyberpunkGame.exe)
  ─────────────────────────────────────────────
  ~ vm.max_map_count    65530  →  16777216
    ↳ System class: HX + discrete GPU (33–64GB combined); 8× baseline

  ~ sched_autogroup     1      →  0
    ↳ Wine thread pool competes individually in scheduler

  SESSION CHANGES  (automatically restored when game exits)
  ─────────────────────────────────────────────
  ~ CPU governor        schedutil  →  performance
  ~ GPU perf mode       adaptive   →  persistence on + clock lock 1680 MHz
    ↳ RTX 3050 VBIOS max 1777 MHz × 95% = 1688 → rounded to 1680

  INSTALLS REQUIRED
  ─────────────────────────────────────────────
  + GE-Proton9-27       ~500 MB download
  + gamemode            via dnf

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Proceed? [y/N]  (--yes to skip in scripts)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Distro Abstraction

```rust
pub trait PackageManager: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_package_installed(&self, package: &str) -> Result<bool, PlayError>;
    fn install_packages(&self, packages: &[&str]) -> Result<(), PlayError>;
    fn get_package_for(&self, capability: &Capability) -> Option<&'static str>;
}

pub enum Capability {
    GameMode,
    MangoHud,
    Vulkan,
    VulkanTools,
    WineBase,
    CabExtract,
}

pub fn detect_package_manager(sys_root: &Path) -> Result<Box<dyn PackageManager>, PlayError> {
    // Check in order: apt, dnf, pacman, zypper
    // Return first found. Error if none found (unsupported distro).
}
```
</implementation_guidance>

---

<antipatterns>
# Explicit Antipatterns — Never Do These

```
✗ String comparisons for distro/vendor detection ("ubuntu".contains(...))
  → Use typed enums. Detect once, store typed, match on enum everywhere.

✗ .unwrap() or .expect() outside of tests
  → Every ? propagates PlayError. Every error is typed and contextual.

✗ Hardcoding paths without the injectable root parameter
  → All /proc /sys paths use the injected root for testability.

✗ Writing to system state before recording the rollback entry
  → Rollback manifest is written first. Always. No exceptions.

✗ Running play-helper for read operations
  → Read /proc /sys directly as unprivileged user. Helper is write-only.

✗ Blocking the main thread during downloads
  → Use indicatif progress bar with a thread. User must see progress.

✗ Silently falling back when a critical component is missing
  → Warn explicitly in the plan: "MangoHud not installed — performance
    overlay unavailable. Install with: sudo dnf install mangohud"

✗ Script fields in database entries
  → Database entries are pure data. No executable content. Ever.

✗ Assuming Wine prefix creation always succeeds silently
  → wine --version first, capture stderr, parse wineserver PID,
    wait for prefix initialization to complete before proceeding.

✗ Spawning child processes with shell=true / sh -c "..."
  → Use Command with explicit args. Never interpolate user data into shell strings.

✗ Different code paths for AMD vs NVIDIA deep in the same function
  → GPU-vendor-specific code lives in separate impl blocks behind a trait.
    The orchestrator never contains vendor conditionals.

✗ Hardcoding a runner version string (e.g. "GE-Proton8-26", "9.10")
  → Runner version is ALWAYS resolved at plan time from runners.toml.
    Store the concrete resolved version in ResolutionDecision, never bake
    a literal version into source code.

✗ Hardcoding vm.max_map_count as 2097152
  → Use the hardware-proportional formula from decision_logic §8.
    The correct value depends on combined RAM + VRAM.

✗ Hardcoding nvidia-smi -lgc 1530,1530
  → 1530 MHz is wrong for most GPUs. Compute from vbios_max_clock_mhz.
    Use compute_nvidia_lock_clock(vbios_max) from decision_logic §9.

✗ Panicking or erroring on AMD hardware in v1
  → AMD GPU arms in the TweakRegistry return TweakDecision::NotApplicable.
    Silent, clean, never an error. Phase 2 will fill them in.

✗ Making any runtime network call for AI inference or decision-making
  → play is a deterministic binary. All decisions are compiled logic.
    The play-db cache is the only knowledge source at runtime.
```
</antipatterns>

---

<first_commit_checklist>
# Before First Commit Checklist

Steps 1–3 are already complete. The checklist covers the remaining prerequisites
before the first push to the repository.

## Already Done ✓
- [x] `models/environment.rs` — all types defined, compiles
- [x] `models/errors.rs` — all PlayError variants with thiserror messages
- [x] `binary/pe.rs` — analyze_binary() with 17+ passing tests

## Must Exist Before First Commit
- [ ] `DESIGN.md` written: Plan→Confirm→Execute, module contracts, error philosophy,
      explicit statement that this is a deterministic binary with no AI runtime deps
- [ ] `GPL-3.0` LICENSE file present
- [ ] Workspace `Cargo.toml` with all three crates defined
- [ ] `.cargo/config.toml` with `-D warnings` enforced
- [ ] `rustfmt.toml` configured
- [ ] `.github/workflows/ci.yml` running `fmt + clippy + test` on PR
- [ ] `.github/workflows/cross-distro.yml` matrix on Ubuntu/Fedora/Arch containers
- [ ] `tests/fixtures/binaries/` with the full fixture set committed (see below)
- [ ] `CONTRIBUTING.md` referencing the play-db schema repo
- [ ] `play-db` repository created with:
      - `entry.schema.json` with all required fields
      - `runners.toml` with at least one GE-Proton entry + SHA256
      - `validate.py` running in GitHub Actions on every PR
      - One example entry for a known-working NVIDIA game
- [ ] Git hooks: `cargo fmt --check` on commit

## PE Fixture Set (commit before writing any more detection logic)

Minimum required fixtures in `tests/fixtures/binaries/`:
- `d3d9_32bit.exe`      — known D3D9, x86
- `d3d11_64bit.exe`     — known D3D11, x86_64
- `d3d12_64bit.exe`     — known D3D12, x86_64
- `vulkan_64bit.exe`    — known Vulkan, x86_64
- `eac_present.exe`     — has EasyAntiCheat imports
- `bink_video.exe`      — has Bink/RAD imports (WMF signal)
- `no_imports.exe`      — no recognized imports (Unknown case — must not crash)

These must be legally distributable: free game demos, public domain
software, or purpose-built minimal PE files. Commit fixtures BEFORE
writing further detection or planning logic.

This is your foundation. Everything else is built on top of it.
The architecture is load-bearing. The types are the design.
Ship slowly, ship correctly, ship something no one else has.
</first_commit_checklist>
