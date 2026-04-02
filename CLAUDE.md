# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

# PLAY — Claude Code Project Context
> Loaded automatically by Claude Code. Kept stable so prompt cache hits are maximized.
> Update the "Current Status" section as the project progresses.

---

## Project Identity

`play` is a **distro-agnostic Linux CLI** that orchestrates the existing open-source gaming
compatibility stack (Wine/Proton/DXVK/VKD3D-Proton/GameMode/MangoHud) so that post-2005
Windows games run at maximum performance with **zero manual configuration**.

**THIS IS A DETERMINISTIC RUST BINARY. NOT AN AI AGENT SYSTEM.**
- Ships as a compiled Rust binary with zero runtime external dependencies
- No API keys, no network calls to AI services, no cloud-based decision making
- All intelligence lives in compiled rule trees and the local `play-db` knowledge cache
- AI is a DEVELOPMENT-TIME tool only (generating/validating `play-db`). NEVER part of the shipped binary

**Sacred invocation contract (never changes):**
```
play game.exe
```

**Language:** Rust edition 2021, MSRV 1.75. `no unsafe` unless absolutely unavoidable and documented.
**License:** GPL-3.0
**Target distros v1:** Ubuntu 22.04+, Fedora 38+, Arch Linux, Debian 12+

**Developer hardware (relevant: this is what play must detect correctly on this machine):**
- CPU: Intel i7-13650HX (Intel HX class — affects vm.max_map_count tier)
- GPU 1: NVIDIA RTX 3050 6GB Laptop GPU [Discrete] — `is_laptop_gpu = true`
  - VBIOS max clock: ~1777 MHz → computed lock = 1680 MHz
  - `is_laptop_gpu = true` means Class C GPU tweaks (clock lock) are SKIPPED (desktop only)
- GPU 2: Intel Raptor Lake-S UHD [Integrated]
- RAM: 15.29 GiB total (~15,657 MB)
- Kernel: 6.18.13 (has_futex2 = true → fsync enabled, not esync)
- OS: Fedora 43 Workstation → package manager: `dnf`

---

## Eight Inviolable Design Principles

Every implementation decision MUST satisfy ALL of these simultaneously. Violation = redesign.

1. **Plan → Confirm → Execute (NEVER skipped)**
   Produce a complete human-readable plan of every intended action BEFORE executing any.
   EXCEPTION: read-only operations. EXCEPTION: `--yes` flag.

2. **Idempotency Everywhere**
   `play game.exe` twice = identical results. Every operation checks current state before acting.

3. **Complete Rollback Guarantee**
   Every persistent modification is recorded in a rollback manifest BEFORE the change.
   `play --undo game.exe` reverses everything. Session-scoped changes restore via Rust `Drop` guards — always.

4. **Fail Loudly and Specifically, Never Silently**
   Either fully succeed or fully roll back. Never swallow errors. Every error variant carries context.

5. **Every Decision is Explainable in One Sentence**
   Every `ResolutionDecision` carries its rationale as a string shown in verbose mode.

6. **The Output Is a First-Class Interface**
   Visual language: `→` pending, `⟳` in-progress, `✓` done, `⊘` skipped, `⚠` warning, `✗` error.

7. **Security By Default**
   Never run as root. Privileged ops go through `play-helper` (narrow JSON command interface).
   Never execute downloaded binaries without SHA256 verification. Never trust game-provided paths.

8. **A Tweak Is Data, Not Code**
   Every optimization maps to the same `Tweak` struct in `TweakRegistry`. The Orchestrator
   never contains tweak-specific logic — it iterates the registry. Adding tweak #200 = one-file change.

---

## Architecture: Module Pipeline

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
```

---

## Key Types (abbreviated — full definitions in models/)

```rust
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
```

---

## Decision Logic: Key Rules

**Runner selection:**
- `AntiCheat::EasyAntiCheat { linux_supported: false }` → hard fail, `PlayError::AntiCheatBlocked`
- `D3D12` → `TranslationLayer::Vkd3dProton`
- `D3D9/10/11/8` → `TranslationLayer::Dxvk`
- `Denuvo` or `EAC(linux_supported: true)` → `DxvkConfig.async_compile = false`
- Default runner: `RunnerType::ProtonGE` (always; GE is the safer default)
- Version: query play-db for `version_min`, then select LATEST stable above floor from `runners.toml`
- **NEVER hardcode a version string like "9.10". NEVER.**

**System tweaks (critical formulas — do not approximate):**
```rust
// vm.max_map_count: hardware-proportional, NEVER hardcoded as 2097152
combined_mb = hardware.memory.total_mb + hardware.gpu.vram_mb
// On dev machine: ~15657 + 6144 = ~21801 MB → tier 16_385..=32_768 → target = 8_388_608

// NVIDIA clock lock: NEVER hardcode 1530
fn compute_nvidia_lock_clock(vbios_max_mhz: u32) -> u32 {
    ((vbios_max_mhz as f32 * 0.95) as u32 / 15) * 15
}
// RTX 3050: max=1777 → 1688 → rounded = 1680 MHz

// Sync: kernel >= 5.16 (has_futex2) → fsync. Otherwise → esync + ulimit check
```

**Drop guard pattern (mandatory for ALL session-scoped changes):**
```rust
// MUST use Drop for cpu_governor, gpu_perf_mode, nvidia_clock_lock, any session change.
// Rust guarantees drop() runs even on panic. Every guard has a Noop variant for no-op.
impl Drop for GovernorGuard {
    fn drop(&mut self) {
        // Log errors but NEVER panic in Drop
        for (path, prev) in &self.cores {
            let _ = fs::write(path, prev).map_err(|e| tracing::error!(...));
        }
    }
}
```

---

## Tweak Classes (reference)

**Class A — session-scoped, Drop-restored, no confirmation needed:**
fsync/esync env vars, CPU governor, GameMode LD_PRELOAD, DXVK state cache path,
DXVK async, VKD3D feature level, MangoHud inject, WINE_LARGE_ADDRESS_AWARE.

**Class B — persistent, rollback manifest FIRST, confirm once per game:**
vm.max_map_count (via helper), sched_autogroup (via helper), split_lock_mitigate (via helper),
THP→madvise, ulimit nofile (/etc/security/limits.d/), audio sample rate, Wine prefix creation,
Windows version in registry.

**Class C — NVIDIA only (v1), desktop only, explicit consent:**
NVIDIA persistence mode (`nvidia-smi -pm 1`), NVIDIA clock lock (computed formula, never hardcoded).
AMD → `TweakDecision::NotApplicable` (Phase 2 stub, never panic, never error).

**Class C — laptop rule:**
`GpuProfile.is_laptop_gpu = true` → ALL Class C GPU tweaks return `TweakDecision::NotApplicable`.
On the dev machine (RTX 3050 Laptop): no clock lock, no persistence mode tweaks.

---

## Antipatterns — NEVER Do These

```
✗ String comparisons for distro/vendor detection ("ubuntu".contains(...))
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
```

---

## Error Model

```rust
// Never use anyhow for user-facing errors. Use the typed PlayError taxonomy.
// Every variant must carry the context needed for a helpful error message.
// Pattern: #[error("...")] with named fields, never just a String.
// Use thiserror. Propagate with ?. The Orchestrator catches and handles.
```

Key error variants to remember: `AntiCheatBlocked`, `ChecksumMismatch`, `PrefixCreation`,
`SysctlWrite`, `GameCrash`, `RollbackFailed`, `UnsupportedDistro`, `InsufficientVram`.

---

## Security Model

```
play (unprivileged)  →  play-helper (narrow privilege boundary)
                         JSON command interface, never shell strings
                         Logs every privileged operation with timestamp
```

Never run the main binary as root. The helper binary handles: sysctl writes, nvidia-smi
perf changes, ulimit conf file writes. Read /proc /sys directly as the user.

---

## play-db Design (separate repo)

play-db is a community database of per-game configuration. play reads it as a local cache.
It is NEVER fetched at game launch time (only on `play --update-db`).

Entry format: TOML files at `entries/{hash[0:2]}/{hash}/default.toml` (+ optional `nvidia.toml`).
Runner manifest: `runners.toml` — lists available GE-Proton versions with SHA256.
Confidence scores: computed by `tools/compute_confidence.py`, never manually assigned.

---

## Current Status (update as project progresses)

### Completed ✓
- **Step 4: Hardware Detection (COMPLETE)** — all 6 file changes implemented and tested
  - `crates/play-core/src/models/environment.rs` — added `Distro` enum, `DistroInfo` struct, `GpuProfile.nvidia_vbios_max_clock_mhz`, `HardwareProfile.distro`
  - `crates/play-core/src/modules/detection.rs` — full detection impl: kernel, CPU (fixed `is_laptop(sys_root)`), memory, GPU lspci + nvidia-smi enrichment, audio (PipeWire/Pulse/ALSA), display (X11/Wayland), distro (/etc/os-release). CommandRunner trait for testability. 14+ passing tests
  - `crates/play-core/src/modules/package_manager.rs` — new file: PackageManager trait, AptPm, DnfPm, PacmanPm, ZypperPm, detect_package_manager()
  - `crates/play-core/src/lib.rs` — added `pub mod modules;`
  - `crates/play-core/src/modules/mod.rs` — added `pub mod package_manager;`
  - `crates/play-core/tests/fixtures/` — static fixture files for unit testing
  - All crates compile successfully; all 31 tests pass
- `crates/play-core/src/models/environment.rs` — all types defined, compiles
- `crates/play-core/src/models/errors.rs` — all PlayError variants with thiserror messages
- `crates/play-core/src/binary/pe.rs` — analyze_binary() with 17+ passing tests
- `crates/play-db-tools/` — stub crate created to satisfy workspace

### Before First Commit (in progress)
- [ ] `DESIGN.md` — Plan→Confirm→Execute, module contracts, error philosophy
- [ ] `GPL-3.0` LICENSE file
- [ ] Workspace `Cargo.toml` (three crates: `play`, `play-helper`, `play-db-tools`)
- [ ] `.cargo/config.toml` with `-D warnings`
- [ ] `rustfmt.toml`
- [ ] `.github/workflows/ci.yml` (fmt + clippy + test on PR)
- [ ] `.github/workflows/cross-distro.yml` (Ubuntu/Fedora/Arch matrix)
- [ ] `tests/fixtures/binaries/` — minimal PE fixtures (d3d9, d3d11, d3d12, vulkan, eac, bink, no_imports)
- [ ] `CONTRIBUTING.md`
- [ ] `play-db` repository scaffold

### Not Yet Started
- PlanningModule (TweakRegistry, DecisionEngine, PlanBuilder)
- ExecutionModules (Package, Runner, Prefix, System, Launch)
- Orchestrator state machine
- play-helper binary
- CLI entry point (main.rs)

---

## Development Standards

**Module structure:** each module in `crates/play-core/src/modules/{module}.rs` (or `{module}/mod.rs` for larger modules) with a single public entry function
that takes typed input and returns `Result<TypedOutput, PlayError>`. No direct I/O in module
internals — inject `sys_root: PathBuf` and `run_cmd: Box<dyn CommandRunner>` for testability.

**Testing approach:**
- Unit tests in the same file (`#[cfg(test)] mod tests { ... }`)
- Integration tests in `tests/` using the fixture PE binaries
- Property-based tests with `proptest` for parsing and formula functions
- All tests must be runnable without any system access (use injectable sys_root)
- Test the `--dry-run` path exhaustively; it must never touch disk

**Logging:** use `tracing` crate. Every significant event has a structured log:
```rust
tracing::info!(event = "tweak_applied", tweak = "cpu_governor", value = "performance");
```
Never use `println!` in library code. `eprintln!` only in main.rs for fatal startup errors.

**Dependencies:** prefer the Rust ecosystem standard choices:
`clap` (CLI), `serde`+`serde_json`+`toml` (data), `thiserror` (errors), `tracing`+`tracing-subscriber`
(logging), `indicatif` (progress), `sha2` (checksum), `reqwest` (downloads), `indexmap` (ordered maps),
`chrono` (timestamps), `tempfile` (test fixtures), `proptest` (property testing).

**Cargo workspace layout:**
```
crates/play-cli/       (main binary — CLI entry point, main.rs)
crates/play-core/      (library crate — all models, modules, detection logic)
crates/play-helper/    (privilege helper, minimal deps)
crates/play-db-tools/  (db validation/promotion tooling, dev-only)
```

**Source layout within play-core:**
```
crates/play-core/src/
  lib.rs
  models/          (environment.rs, errors.rs — types shared across all modules)
  binary/          (pe.rs — PE binary analyzer)
  modules/         (detection.rs, and future planning/execution modules)
```

---

## Commands

```bash
# Build
cargo build                          # debug build (all crates)
cargo build --release                # release build

# Test
cargo test                           # run all tests
cargo test -p play-core              # run tests for a single crate
cargo test -p play-core binary::pe   # run a specific test module
cargo test -- --nocapture            # show println! output during tests

# Lint / format (must all pass before commit)
cargo fmt --check                    # check formatting without changing files
cargo fmt                            # auto-format
cargo clippy -- -D warnings          # clippy with warnings-as-errors

# Combined pre-commit check (same as CI)
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

---

## Working With This Codebase

**Before implementing any feature:** run `/plan` to produce a written plan. Review it against
the 8 inviolable principles. If any principle is violated in the plan, redesign.

**Before adding any tweak:** it must map to the `TweakConstraint` struct. Add it to the
TweakRegistry as a data entry, not as code in the Orchestrator. Use `/tweak` command.

**Before adding any new module:** use `/module` command to get the correct scaffolding
with typed contracts, injectable test seams, and proper error propagation.

**After writing any significant code block:** run `/review` to check against antipatterns.

**Commit message format:** `feat(module): description` | `fix(module): description` | `test(module): description`
Every commit must pass: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`

**Cost optimization hints:**
- Use `/compact` before long implementation sessions to compress context
- Switch to Haiku (`claude --model claude-haiku-4-5-20251001`) for mechanical tasks
  (renaming, formatting fixes, boilerplate generation, reading docs)
- Use Sonnet (default) for architecture decisions, complex logic, code review
- Keep CLAUDE.md stable (don't edit mid-session) to maximize prompt cache hits
- Use custom commands (below) — they pack complex instructions into short invocations
