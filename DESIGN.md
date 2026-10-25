# `play` — Design Document

## Identity

`play` is a distro-agnostic Linux CLI for orchestrating open-source gaming
compatibility tools. It is a **deterministic compiled Rust binary** with **zero
runtime external dependencies or AI calls**. AI agents are used at
development-time only; the shipped binary contains compiled rule trees, not
inference.

One command: `play game.exe`. No flags, no config files, no setup wizard.

---

## Core Philosophy

### Plan → Confirm → Execute

Every invocation follows this three-phase pipeline:

1. **Plan** — Detect hardware, analyze the binary, resolve all decisions,
   produce a `GamePlan`. No system state is written.
2. **Confirm** — Display the plan to the user. They approve or abort.
3. **Execute** — Apply tweaks, install runner, create prefix, launch game.
   Rollback on failure.

No system mutation occurs before user confirmation. Ever.

### Idempotency

Running `play game.exe` twice must produce the same result. If a tweak is
already applied, it is `AlreadySatisfied`. If a runner is already installed,
it is `AlreadyInstalled`. If a prefix already exists, it is `AlreadyExists`.

### Complete Rollback Guarantee

Every persistent system change (Class B/C tweaks) writes a rollback manifest
*before* the change is applied. If execution fails at any point, the
Orchestrator walks the manifest in reverse. If rollback itself fails, the user
receives an explicit `RollbackFailed` error with the exact manual restoration
instructions.

### Fail Loudly and Specifically, Never Silently

Errors are never swallowed. Every error path produces a `PlayError` variant
with enough context for the user to act. No `unwrap()`, no `anyhow`, no
"something went wrong." The error taxonomy is explicit and exhaustive.

### Explainable Decisions

Every decision the engine makes is recorded as a `ResolutionDecision` with
`field`, `chosen`, `reason`, `confidence`, and `source`. These are shown in
verbose mode and included in bug reports. Users never wonder *why* something
was configured.

### Output as UI

The terminal is the only interface. Structured, colored, readable. No TUI
frameworks, no GUI. Plan display uses `console` + `indicatif`. Progress bars
for downloads. Clear section headers.

### Security By Default

- Path sanitization on all user-supplied paths
- SHA256 verification on all downloaded runners
- Schema validation on all play-db entries
- `play-helper` whitelist for privileged operations
- No shell interpolation, no `sh -c`, no `shell: true`

### Tweak as Data

Tweaks are defined in `TweakRegistry` as pure data (`TweakConstraint`).
Decision logic lives in `DecisionEngine`. The registry contains no functions.
Adding a new tweak means adding a row, not writing code.

---

## Architecture

```
┌─────────────┐
│  play-cli   │  CLI entry point, argument parsing
└──────┬──────┘
       │
┌──────▼──────┐
│ Orchestrator│  Plan → Confirm → Execute lifecycle
└──────┬──────┘
       │
   ┌───┼───────────────┐
   │   │               │
┌──▼──┐ ┌▼────────┐ ┌──▼──────────┐
│Detect│ │  Plan   │ │   Execute   │
│  ion │ │         │ │             │
└──┬───┘ └┬────────┘ └┬────────────┘
   │      │           │
   │  ┌───┼────┐     │
   │  │   │    │     │
┌──▼┐┌▼──┐┌▼───┐ ┌──▼──────┐
│HW ││DB ││Dec │ │  System  │
│   ││   ││Eng │ │  Module  │
└───┘└───┘└────┘ └─────────┘
```

Modules communicate via typed data structures (`GameEnvironment`, `GamePlan`,
`PlayError`). No module reaches into another module's internals.

---

## Module Contracts

### DetectionModule

**Input**: Injectable path roots (`/proc`, `/sys`, `/etc`), `CommandRunner`
**Output**: `HardwareProfile`
**Side effects**: None. All reads are through injectable paths for testability.

### PlanningModule

**Input**: `GameEnvironment` (identity + hardware populated)
**Output**: `GamePlan`
**Side effects**: None. Pure computation. No filesystem writes.

### DecisionEngine

**Input**: Hardware profile, binary identity, DB entry, runners manifest
**Output**: `ResolutionDecision` for each field
**Side effects**: None. All methods are pure functions.

### ExecutionModule

**Input**: `GamePlan` (user-confirmed)
**Output**: Game exit code or `PlayError`
**Side effects**: System mutation (tweaks, downloads, prefix creation, launch).
All mutations are preceded by rollback manifest writes.

### ValidationModule

**Input**: `GameEnvironment` after execution
**Output**: Pass/fail with diagnostics
**Side effects**: None. Read-only verification.

### play-helper

**Input**: Privileged operation request (sysfs write, sysctl)
**Output**: Success/failure
**Side effects**: Writes to `/proc/sys`, `/sys`. Whitelisted paths only.
Read operations are performed unprivileged — helper is write-only.

---

## Data Model

### GameEnvironment

The single source of truth. Contains:

- `GameIdentity` — exe hash, name, DX version, anti-cheat, engine hint
- `HardwareProfile` — GPU, CPU, memory, kernel, display, distro
- `GraphicsConfig` — translation layer, DXVK config, MangoHud
- `RunnerConfig` — runner type, version, install path
- `AudioConfig` — backend, Wine driver, sample rate
- `PrefixConfig` — arch, Windows version, DLL overrides, env vars
- `SystemTuning` — all tweak targets (vm.max_map_count, THP, fsync, etc.)
- `LaunchConfig` — exe path, args, env, pre/post hooks
- `EnvironmentMetadata` — schema version, timestamps, decision audit trail

### PlayError

Explicit error taxonomy. Every variant carries actionable context.
No `anyhow`. No `unwrap()`. No silent fallbacks.

### GamePlan

The output of planning, shown to the user before confirmation:

- Resolved `GameEnvironment` with all fields populated
- Warnings (non-fatal advisories)
- Hard blocks (planning failures — abort)
- Required packages
- Runner action (download or already installed)
- Prefix action (create or already exists)
- All tweak decisions with rationale

---

## Decision Logic

All decisions are compiled Rust rule trees. No AI, no network, no runtime
inference. The `DecisionEngine` is a set of pure functions:

- **Translation layer**: DX version → VKD3D-Proton / DXVK / Native / WineOpenGL
- **Runner selection**: DB override or ProtonGE default, version from manifest
- **Sync mode**: `has_futex2` → fsync, else esync
- **Audio driver**: PipeWire/PulseAudio → Pulse, ALSA → ALSA
- **Prefix arch**: PE architecture → Win32/Win64
- **Windows version**: DB override or Win10 default
- **Tweak resolution**: Registry constraints evaluated against live hardware

Every decision produces a `ResolutionDecision` for the audit trail.

---

## Tweak Classification

| Class | Scope | Rollback | Confirmation |
|-------|-------|----------|--------------|
| A | Session-scoped | Drop-restored on exit | Not needed |
| B | Persistent | Rollback manifest first | Once per game |
| C | NVIDIA desktop-only | Rollback manifest first | Explicit consent |

---

## State Management

Per-game state lives in `~/.local/share/play/games/{exe_hash}/`:

- `checkpoint.json` — serialized `GameEnvironment` at each phase
- `rollback.json` — manifest of system changes to reverse
- `session.log` — structured log for this invocation
- `metrics.json` — launch time, crash count, last run

---

## Testing Requirements

- All `/proc` and `/sys` reads use injectable path roots
- All external commands use `CommandRunner` trait
- Mock fixtures for PE analysis in `tests/fixtures/binaries/`
- DB fixtures in `tests/fixtures/db/`
- CI matrix: Ubuntu 22.04, Fedora 38+, Arch, Debian 12+
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` must pass

---

## Project Structure

```
play/
├── crates/
│   ├── play-cli/        # Binary crate — CLI entry point
│   ├── play-core/       # Library crate — all logic
│   ├── play-helper/     # Privileged helper binary
│   └── play-db-tools/   # DB maintenance utilities
├── .cargo/config.toml   # -D warnings enforced
├── .github/workflows/   # CI + cross-distro
├── tests/fixtures/      # PE binaries + DB fixtures
├── DESIGN.md            # This file
├── CONTRIBUTING.md      # Contribution guide
└── Cargo.toml           # Workspace root
```
