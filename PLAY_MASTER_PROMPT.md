# PLAY — Master Project Brief
## For: Claude Opus 4.6 Extended Thinking | Windsurf Project Rules

---

<project_identity>
# Project: `play`

A distro-agnostic Linux CLI that orchestrates the existing open-source gaming
compatibility stack to enable post-2005 Windows games to run as well as the
user's hardware physically allows — with zero manual configuration required.

**Invocation contract (sacred, never changes):**
```
play [game.exe | game.AppImage]
```
No flags required. No configuration files required from the user.
No prior knowledge required. One command. Works or tells you exactly why it won't.

**What this tool IS:**
An intelligent orchestrator of Wine/Proton/DXVK/VKD3D-Proton/GameMode/MangoHud
and related tools. It detects, decides, installs, configures, and launches.

**What this tool IS NOT:**
- A compatibility layer (it uses existing ones)
- A game launcher GUI (pure CLI)
- A package manager (it uses the system's)
- A Wine replacement
- Anything that requires root to run normally

**License:** FOSS (MIT or GPL-3.0 — decide before first commit, default GPL-3.0
for copyleft protection of the ecosystem work embedded in this tool)

**Target distros v1:** Ubuntu 22.04+, Fedora 38+, Arch Linux, Debian 12+
**Deferred:** Gentoo, NixOS, source-based distros (Phase 2)
**Language:** Rust (edition 2021, MSRV 1.75)
**No unsafe unless absolutely unavoidable and documented with justification**
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
</core_philosophy>

---

<architecture>
# System Architecture: Multi-Agent Orchestration

## Agent Model

The system is composed of specialized agents with strict contracts.
Agents communicate ONLY through typed data structures via the Orchestrator.
Agents NEVER call each other directly. Agents have NO side effects outside
their designated responsibility domain.

```
CLI Entry Point (main.rs)
        │
        ▼
┌───────────────────────────────────────────────────────┐
│                    Orchestrator                        │
│  Owns: GamePlan, ExecutionState, EventLog              │
│  Drives: agent sequencing, state machine, checkpoints  │
└──────────┬────────────────────────────────────────────┘
           │ dispatches typed inputs, receives typed outputs
     ┌─────┴──────────────────────────────────┐
     │                                        │
     ▼                                        ▼
DetectionAgent                         ValidationAgent
  ├─ HardwareDetector                    ├─ LaunchMonitor
  ├─ BinaryAnalyzer (PE parser)          ├─ GpuActivityChecker
  ├─ KernelInspector                     ├─ FpsLogger (MangoHud)
  └─ AudioStackDetector                  └─ CrashDetector
           │
           ▼ DetectionResult
     PlanningAgent
       ├─ DatabaseClient (local cache)
       ├─ ConstraintChecker
       ├─ DecisionEngine (rule tree)
       └─ PlanBuilder → GamePlan
           │
           ▼ GamePlan (after user confirmation)
     ExecutionAgents (sequential, checkpointed)
       ├─ PackageAgent   (system dependencies)
       ├─ RunnerAgent    (Proton/Wine download+verify)
       ├─ PrefixAgent    (Wine prefix setup)
       ├─ SystemAgent    (sysctl + Drop guards)
       └─ LaunchAgent    (final process spawn)
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
    Running     { pid: u32, guards: Vec<Box<dyn RestoreGuard>> },
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
/// This is what gets serialized to state files and what agents pass between them.
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
    pub driver_type:     DriverType,         // Mesa | NvidiaProprietray | IntelANV
    pub features:        GpuFeatureSet,
    pub is_laptop_gpu:   bool,               // gates thermal-risky tweaks
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
    pub vm_max_map_count:      u64,     // current value, needs >= 2097152
    pub thp_mode:              ThpMode, // always | madvise | never
    pub split_lock_mitigate:   bool,
    pub sched_autogroup:       bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub server:          DisplayServer,  // X11 | Wayland
    pub primary_res:     (u32, u32),
    pub refresh_hz:      f32,
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
    pub version:        SemVer,
    pub install_path:   PathBuf,
    pub verified:       bool,        // SHA256 verified on install
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunnerType {
    ProtonOfficial,
    ProtonGE,
    WineGE,
    WineStaging,
    SodaWine,
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
    pub vm_max_map_count:     Option<u64>,
    pub thp_mode:             Option<ThpMode>,
    pub sched_autogroup:      Option<bool>,
    pub split_lock_mitigate:  Option<bool>,
    pub ulimit_nofile:        Option<u64>,
    pub cpu_governor:         Option<CpuGovernor>,    // session-scoped
    pub gpu_perf_mode:        Option<GpuPerfMode>,    // session-scoped
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolutionSource {
    FullyAutomatic,
    DatabaseAssisted { entry_id: String, confidence: f32 },
    UserOverridden   { fields: Vec<String> },
    Hybrid           { db_fields: Vec<String>, heuristic_fields: Vec<String> },
}

/// Every decision the Planning Agent makes is recorded here.
/// This is what users see in verbose mode and what goes into bug reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionDecision {
    pub field:      String,      // "runner.runner_type"
    pub chosen:     String,      // "ProtonGE"
    pub reason:     String,      // "Game uses WMF video; GE includes media patches"
    pub confidence: Confidence,
    pub source:     DecisionSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Confidence { High, Medium, Low, Inferred }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionSource { Database, Heuristic, HardwareDetection, UserOverride }
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

4. RUNNER SELECTION:
   has_video_cutscenes = true   → RunnerType::ProtonGE  (WMF patches)
   GameEngine = REEngine        → RunnerType::ProtonGE  (specific patches exist)
   dx_version = D3D12           → RunnerType::ProtonGE  (better VKD3D integration)
   pe_arch = x86 (32-bit)       → RunnerType::WineGE or ProtonGE (32-bit prefix)
   default                      → RunnerType::ProtonGE  (GE is always safer default)

5. WINDOWS VERSION REPORTED TO GAME:
   D3D12        → WindowsVersion::Win10
   D3D11        → WindowsVersion::Win10
   D3D9 + old   → WindowsVersion::Win7  (some old games reject Win10)
   default      → WindowsVersion::Win10

6. FSYNC vs ESYNC:
   kernel.has_futex2 = true    → fsync=true, esync=false
   kernel.has_futex2 = false   → fsync=false, esync=true
                                  + check ulimit_nofile >= 524288
                                  + if not: add ulimit raise to plan

7. SYSTEM TWEAKS (each is a TweakConstraint check before adding to plan):
   Always evaluate vm_max_map_count: if current < 2097152, add to plan
   RAM >= 16384 MB: evaluate thp_mode → madvise
   x86 CPU: evaluate split_lock_mitigate → 0
   Wine/Proton active: evaluate sched_autogroup → 0
   NOT laptop GPU: evaluate gpu_perf_mode → max
   NOT battery: evaluate cpu_governor → performance

8. VKD3D FEATURE LEVEL:
   GpuFeatureSet.dx12_feature_level is Some(level) → set VKD3D_FEATURE_LEVEL env
   Otherwise: leave unset (VKD3D auto-detects, safer than wrong explicit value)

9. LARGE ADDRESS AWARE:
   pe_arch = x86 (32-bit) AND vram_mb > 2048 → PrefixConfig.large_address_aware = true
```

## TweakConstraint: Gate every risky tweak behind typed constraints

```rust
pub struct TweakConstraint {
    pub tweak:                   SystemTweak,
    pub min_ram_mb:              Option<u32>,
    pub kernel_version_min:      Option<KernelVersion>,
    pub gpu_vendor_exclusions:   Vec<GpuVendor>,
    pub cpu_arch_required:       Option<CpuArch>,
    pub requires_desktop:        bool,    // false = applies to laptops too
    pub reversible:              bool,
    pub reboot_required:         bool,
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
| vm.max_map_count | sysctl via helper binary, RollbackEntry written first | always if < 2097152 |
| kernel.sched_autogroup | sysctl via helper | Wine/Proton active |
| kernel.split_lock_mitigate | sysctl via helper | x86 arch only |
| THP → madvise | Write to `/sys/kernel/mm/transparent_hugepage/enabled` | RAM >= 16GB |
| ulimit nofile | `/etc/security/limits.d/play-{game}.conf` | esync required |
| Audio sample rate | Wine registry in prefix via `wine reg add` | always |
| Per-game Wine prefix | Create `~/.local/share/play/prefixes/{hash}/` | always |
| Windows version in prefix | `wine reg add HKLM\...` | determined by dx_version |

## Class C — Hardware-conditional, explicit user consent per invocation

| Tweak | Implementation | Condition | Desktop Only |
|-------|---------------|-----------|-------------|
| NVIDIA max perf mode | `nvidia-smi -pm 1` via helper, GpuPerfGuard Drop restore | NVIDIA GPU | YES |
| AMD perf level → high | Write `high` to sysfs power_dpm path, GpuPerfGuard Drop restore | AMD GPU | YES |
| Optimized kernel | RECOMMEND ONLY, never auto-install | binary distro | N/A |

## NEVER touch (document why in code comments)

- BIOS/UEFI settings
- GPU overclocking  
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
│           ├── nvidia.toml    # GPU-conditional variant if needed
│           └── amd.toml
├── reports/                   # raw reports, append-only, never edited post-submit
│   └── {year}/{month}/
│       └── {report_id}.toml
└── tools/
    ├── validate.py            # run in CI on every PR
    ├── compute_confidence.py  # recomputes confidence scores nightly
    └── promote_report.py      # maintainer helper: report → entry
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
gpu_vendor          = ["AMD", "NVIDIA"]  # empty = all vendors
gpu_vulkan_min      = "1.3"
kernel_min          = "6.0"
runner_type         = "ProtonGE"
runner_version_min  = "8.0"
anti_cheat          = []                 # must have NONE of these to apply

[entry.outcome]
result       = "Optimal"       # Broken | Degraded | Playable | Optimal
confidence   = 0.91            # COMPUTED, never manually set
degradations = ["shader_stutter_first_run"]
blockers     = []

[entry.configuration]
# Direct mutations to GameEnvironment fields
[entry.configuration.runner]
type        = "ProtonGE"
version_min = "8.0"

[entry.configuration.graphics]
translation_layer = "Vkd3dProton"
dxvk_async        = false

[entry.configuration.system]
vm_max_map_count  = 16777216
fsync             = true

[entry.configuration.prefix]
windows_version = "Win10"
dll_overrides   = []

[entry.configuration.env_vars]
WINE_LARGE_ADDRESS_AWARE = "0"  # 64-bit game, not needed
VKD3D_FEATURE_LEVEL      = "12_1"

[entry.metadata]
created           = "2024-11-01"
last_validated    = "2025-01-15"   # auto-expires: entry becomes "Unverified" after 90 days
contributor_count = 47
report_ids        = ["report-uuid-1", "report-uuid-2"]
maintainer_notes  = "Confirmed stable across 3 GE-Proton versions"
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
// An entry verified on RX 6800 XT applies to RX 7900 XT with reduced score.
// That's better than returning nothing.
```
</database_design>

---

<binary_analysis>
# Static Binary Analysis (DetectionAgent)

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
        anti_cheat.push(AntiCheat::EasyAntiCheat { linux_supported: false }); // check DB later
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
    // Known engine DLL signatures
    if imports.iter().any(|i| i.contains("phonon"))     { return Some(GameEngine::UnrealEngine4); }
    if imports.iter().any(|i| i == "gameoverlayrenderer64.dll") { /* likely UE */ }
    // RE Engine: known specific DLL
    if imports.iter().any(|i| i.contains("mt_framework")) { return Some(GameEngine::REEngine); }
    None
}
```
</binary_analysis>

---

<hardware_detection>
# Hardware Detection (DetectionAgent)

## Detection Strategy Per Component

```
GPU vendor: lspci -mm | grep -i "VGA\|3D" → parse vendor ID
            Fallback: /sys/class/drm/card0/device/vendor (hex ID)
            NVIDIA confirm: check /proc/driver/nvidia/version exists

GPU model: lspci output, or /sys/class/drm/card0/device/product_name

GPU VRAM:  NVIDIA: nvidia-smi --query-gpu=memory.total --format=csv,noheader
           AMD:    /sys/class/drm/card0/device/mem_info_vram_total
           Intel:  vulkaninfo | grep "maxMemoryAllocationCount"

Driver version:
           NVIDIA: nvidia-smi --query-gpu=driver_version --format=csv,noheader
           AMD:    glxinfo -B | grep "OpenGL version" | parse Mesa version
                   Fallback: /sys/class/drm/card0/device/uevent

Vulkan:    vulkaninfo --json | jq '.properties.apiVersion'
           CRITICAL: parse as packed uint32: (major << 22) | (minor << 12) | patch

Kernel:    fs::read_to_string("/proc/version") → parse semver
           futex2: kernel >= 5.16 (check precisely, not approximately)
           sched_autogroup current: fs::read_to_string("/proc/sys/kernel/sched_autogroup_enabled")
           vm.max_map_count: fs::read_to_string("/proc/sys/vm/max_map_count")
           split_lock: /proc/sys/kernel/split_lock_mitigate

CPU:       fs::read_to_string("/proc/cpuinfo")
           physical_cores: count unique "core id" values  
           logical_cores: count "processor" entries
           AVX2: flags line contains "avx2"
           laptop: check if any CPU freq policy is "powersave" on AC

Audio:     Check process list for "pipewire" or "pulseaudio" running
           PipeWire: pw-dump --json | parse defaultNodes sample rate
           PulseAudio: pactl info | grep "Default Sample Spec" | parse rate

Display:   Check $WAYLAND_DISPLAY (set → Wayland) or $DISPLAY (set → X11)
           Resolution: xrandr --current | parse primary, or wlr-randr

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
├── DESIGN.md               # Plan→Confirm→Execute model, agent contracts, error philosophy
├── CONTRIBUTING.md
├── LICENSE                 # GPL-3.0
│
├── crates/
│   ├── play-cli/           # Binary crate: main.rs, CLI arg parsing (clap)
│   │   └── src/
│   │       └── main.rs
│   │
│   ├── play-core/          # Library crate: all agents, orchestrator, data model
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── models/     # GameEnvironment and all types
│   │       │   ├── mod.rs
│   │       │   ├── environment.rs
│   │       │   ├── hardware.rs
│   │       │   └── errors.rs
│   │       ├── agents/
│   │       │   ├── mod.rs
│   │       │   ├── detection.rs
│   │       │   ├── planning.rs
│   │       │   ├── execution/
│   │       │   │   ├── mod.rs
│   │       │   │   ├── packages.rs
│   │       │   │   ├── runner.rs
│   │       │   │   ├── prefix.rs
│   │       │   │   └── system.rs
│   │       │   └── validation.rs
│   │       ├── orchestrator.rs
│   │       ├── database.rs
│   │       ├── guards/     # Drop-based restore guards
│   │       │   ├── mod.rs
│   │       │   ├── governor.rs
│   │       │   └── gpu_perf.rs
│   │       ├── binary/     # PE/AppImage analysis
│   │       │   └── pe.rs
│   │       └── state/      # Checkpoint, rollback manifest, state files
│   │           └── mod.rs
│   │
│   └── play-helper/        # Privileged helper binary (minimal, auditable)
│       └── src/
│           └── main.rs     # Accepts JSON commands on stdin, executes, returns JSON
│
├── tests/
│   ├── integration/        # Full agent pipeline tests with mocked hardware
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
indexmap    = { version = "2", features = ["serde"] }  # ordered HashMap
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
tracing-appender     = "0.2"    # file logging alongside terminal

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
codegen-units = 1   # best binary size + perf
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
and auditable. It does EXACTLY four things and refuses everything else.

## play-helper Command Interface

```json
// Stdin: JSON command. Stdout: JSON result. Stderr: error details.
// play-helper ONLY accepts commands on stdin. Never command-line args with values.

// Write sysctl
{ "cmd": "sysctl_write", "key": "vm.max_map_count", "value": "16777216" }

// Read sysctl (to capture rollback value)
{ "cmd": "sysctl_read", "key": "vm.max_map_count" }

// Write to sysfs path (GPU perf level, THP mode, etc.)
// Path is validated against a whitelist — never pass arbitrary paths
{ "cmd": "sysfs_write", "path": "gpu_perf_level", "value": "high" }
// "path" is a KEY into a hardcoded whitelist inside play-helper, NOT a raw path

// Set GPU persistence mode (NVIDIA only)
{ "cmd": "nvidia_pm", "enable": true }

// All other commands → immediate rejection with non-zero exit
```

## Sysfs Whitelist in play-helper

```rust
// play-helper/src/main.rs — the ENTIRE set of paths it will write to
// Adding to this list requires code review and justification comment
const SYSFS_WHITELIST: &[(&str, &str)] = &[
    ("gpu_perf_level", "/sys/class/drm/card0/device/power_dpm_force_performance_level"),
    ("thp_mode",       "/sys/kernel/mm/transparent_hugepage/enabled"),
    ("split_lock",     "/proc/sys/kernel/split_lock_mitigate"),
    ("autogroup",      "/proc/sys/kernel/sched_autogroup_enabled"),
    ("max_map_count",  "/proc/sys/vm/max_map_count"),
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
│   └── entries/
├── runners/                           # downloaded runners
│   ├── ProtonGE-8.26/
│   └── WineGE-8.0/
└── games/
    └── {exe_hash}/
        ├── environment.toml           # complete GameEnvironment (the canonical record)
        ├── checkpoint.toml            # current execution state (for resumption)
        ├── rollback.toml              # every persistent change, pre-change values
        ├── session.log                # tracing output from last session (rotated, 3 kept)
        ├── prefix/                    # Wine prefix (symlink or actual dir)
        └── reports/
            └── {timestamp}.toml      # pending report submissions
```

## Rollback Manifest

```toml
# rollback.toml — written BEFORE each change, never after
schema_version = 1

[[changes]]
kind           = "Sysctl"
key            = "vm.max_map_count"
previous_value = "65530"
current_value  = "16777216"
changed_at     = "2025-03-01T14:22:11Z"
play_version   = "0.1.0"

[[changes]]
kind           = "Sysctl"
key            = "kernel.sched_autogroup_enabled"
previous_value = "1"
current_value  = "0"
changed_at     = "2025-03-01T14:22:12Z"
play_version   = "0.1.0"

[[changes]]
kind           = "PackageInstalled"
package        = "gamemode"
package_manager = "dnf"
changed_at     = "2025-03-01T14:22:09Z"
play_version   = "0.1.0"
# Note: packages are NOT auto-uninstalled on rollback.
# Record is informational. User prompted to review.
```
</state_management>

---

<testing_requirements>
# Testing Requirements

## What Must Be Tested

Every agent function that touches /proc, /sys, or external binaries
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

- PE analysis: test against fixture binaries for each DX version
- Decision engine: unit test every branch with typed inputs
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
   → Mitigation: SHA256 verify every download against a manifest
     The manifest itself is fetched over TLS from the canonical GitHub release
     and its hash is pinned in the tool binary for known versions

3. **Database entry injection** (malicious PR to play-db)
   → Mitigation: database entries only mutate GameEnvironment fields
     They cannot execute arbitrary code. No script fields in the schema ever.
     CI validates every entry against the schema before merge

4. **play-helper privilege escalation**
   → Mitigation: hardcoded command whitelist, hardcoded sysfs path whitelist,
     JSON-only interface, minimal binary, no dynamic dispatch, reviewed on change

5. **State file tampering**
   → Mitigation: state files are user-owned, in user home dir
     The tool reads them but validates schema_version and required fields
     A corrupted state file triggers `PlayError::StateCorrupted` with clear
     recovery instructions, not silent misbehavior

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
# Implementation Guidance for the AI Agent

## Start Here — Exact Build Sequence

Build in this exact order. Do not skip ahead. Each step produces something
runnable before the next step begins.

```
Step 1: models/environment.rs — define ALL types. No logic yet.
Step 2: models/errors.rs — define ALL PlayError variants with messages.
Step 3: binary/pe.rs — analyze_binary() with goblin. Test against fixtures.
Step 4: agents/detection.rs — kernel, CPU, GPU, audio detection with injectable paths.
Step 5: guards/governor.rs — GovernorGuard with Drop. Test the Drop fires on panic.
Step 6: guards/gpu_perf.rs — GpuPerfGuard with Drop.
Step 7: agents/planning.rs — decision engine, TweakConstraint checks, ResolutionDecision.
Step 8: database.rs — local cache loading, scored query algorithm.
Step 9: agents/execution/packages.rs — PackageAgent with distro detection.
Step 10: agents/execution/runner.rs — RunnerAgent with download+verify.
Step 11: agents/execution/prefix.rs — PrefixAgent with Wine prefix setup.
Step 12: agents/execution/system.rs — SystemAgent with sysctl via play-helper.
Step 13: play-helper/src/main.rs — minimal privileged binary.
Step 14: orchestrator.rs — state machine, checkpoint system, agent sequencing.
Step 15: agents/validation.rs — launch monitoring, GPU activity check.
Step 16: play-cli/src/main.rs — CLI surface, plan display, user confirmation.
```

## Code Quality Non-Negotiables

```toml
# .cargo/config.toml — enforced from day one
[build]
rustflags = [
    "-D", "warnings",           # warnings are errors
    "-D", "clippy::all",
    "-D", "clippy::pedantic",
    "-A", "clippy::module_name_repetitions",  # common false positive
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
- `trace`: internal agent state, every file read
- `debug`: detection values, decision scoring
- `info`: every state transition, every tweak applied, game launch
- `warn`: degraded fallback used, low-confidence database match
- `error`: failures, rollback events

## Dependency Injection Pattern

Every agent takes its dependencies as constructor arguments, never reads
global state. This enables testing with mocked dependencies:

```rust
pub struct DetectionAgent {
    proc_root: PathBuf,   // "/" in production, test fixture path in tests
    sys_root:  PathBuf,
    run_cmd:   Box<dyn CommandRunner>,  // real subprocess | mock in tests
}
```

## The Plan Display Format

This is what users see before confirming. It must be this clear:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  PLAY — Cyberpunk 2077
  Analyzed: CyberpunkGame.exe (sha256:a3f4…)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  CONFIGURATION
  ─────────────────────────────────────────────
  Runner        ProtonGE 8.26
                ↳ Game uses WMF video cutscenes; GE includes required media patches

  Translation   VKD3D-Proton 2.11
                ↳ D3D12 detected in binary imports

  Sync          fsync
                ↳ Kernel 6.7.4 supports futex2

  SYSTEM CHANGES  (persistent, reversible with: play --undo CyberpunkGame.exe)
  ─────────────────────────────────────────────
  ~ vm.max_map_count    65530  →  16777216
  ~ sched_autogroup     1      →  0

  SESSION CHANGES  (automatically restored when game exits)
  ─────────────────────────────────────────────
  ~ CPU governor        schedutil  →  performance
  ~ GPU perf level      auto       →  high

  INSTALLS REQUIRED
  ─────────────────────────────────────────────
  + ProtonGE-8.26       ~500 MB download
  + gamemode            via dnf

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Proceed? [y/N]  (--yes to skip in scripts)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## AppImage Handling

AppImages require inspection before deciding what to do:

```rust
pub enum AppImageContent {
    NativeLinuxBinary,          // already Linux, just make executable + run
    BundledWineGame(PathBuf),   // contains a .exe — extract and treat as PE
    WineRuntime,                // contains Wine itself — use its Wine, not ours
}

pub fn inspect_appimage(path: &Path) -> Result<AppImageContent, PlayError> {
    // AppImages are self-mounting ISOs. Mount with `--appimage-mount` or
    // extract with `--appimage-extract`. Check for .exe files in AppDir.
    // Check desktop file Exec= line for clues.
    // If contains Wine binary, flag as WineRuntime — don't double-wrap.
}
```

## Distro Abstraction

```rust
pub trait PackageManager: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_package_installed(&self, package: &str) -> Result<bool, PlayError>;
    fn install_packages(&self, packages: &[&str]) -> Result<(), PlayError>;
    fn get_package_for(&self, capability: &Capability) -> Option<&'static str>;
}

// Capability → package name mapping per distro
// Because "gamemode" on Arch is "gamemode",
// but on Ubuntu might need a PPA, and on Fedora is "gamemode" too
pub enum Capability {
    GameMode,
    MangoHud,
    Vulkan,
    VulkanTools,     // vulkaninfo binary
    WineBase,
    CabExtract,
    // ... extend as needed
}

pub fn detect_package_manager(sys_root: &Path) -> Result<Box<dyn PackageManager>, PlayError> {
    // Check in order: apt, dnf, pacman, zypper, xbps
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
```
</antipatterns>

---

<first_commit_checklist>
# Before First Commit Checklist

- [ ] `DESIGN.md` written: Plan→Confirm→Execute, agent contracts, error philosophy
- [ ] `GPL-3.0` LICENSE file present
- [ ] Workspace Cargo.toml with all three crates defined
- [ ] All types in `models/environment.rs` — compile, no logic
- [ ] All variants in `models/errors.rs` with thiserror messages
- [ ] `.cargo/config.toml` with `-D warnings` enforced
- [ ] `rustfmt.toml` configured
- [ ] `.github/workflows/ci.yml` running `fmt + clippy + test` on PR
- [ ] `tests/fixtures/` directory with at least one test PE binary
- [ ] `CONTRIBUTING.md` referencing the database schema repo
- [ ] `play-db` repository created with schema and CI validator
- [ ] Git hooks: `cargo fmt --check` on commit

This is your foundation. Everything else is built on top of it.
The architecture is load-bearing. The types are the design.
Ship slowly, ship correctly, ship something no one else has.
</first_commit_checklist>
