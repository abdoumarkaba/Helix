# Final CLAUDE.md (after all edits — now ~98 lines / < 4 kB)

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

# PLAY — Claude Code Project Context
> Loaded automatically by Claude Code. **THIS FILE MUST REMAIN < 4 kB**. Kept extremely stable. Update "Current Status" **only at end of session**. Reference material lives in CLAUDE-REFERENCE.md.

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
play game.exe

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

## COST GUARDRAIL (highest priority — checked on every action)

- Before any non-trivial action (code change >30 lines, >2 consecutive Bash tool calls, or any multi-file edit) the model MUST internally project cost.
- If projected session total would exceed **$0.25**, output exactly:
[COST LIMIT] Projected session cost would exceed $0.25. Session auto-terminated.
Current cost: $X.XX
and refuse all further tool use.
- Prefer `/compact` before every major phase. Use batch edits. Never switch models.

---

## Architecture (summary only — full details in CLAUDE-REFERENCE.md)

High-level pipeline: CLI → Orchestrator → Detection → Planning → Execution (checkpointed).
Modules communicate **only** via typed GameEnvironment. No direct calls between modules.

---

## Current Status (update only at end of session)

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

## Development Standards (summary)

- Modules in `crates/play-core/src/modules/`
- Inject `sys_root` + `CommandRunner` trait for testability.
- All tests use fixtures, no real system access.
- Pre-commit: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`

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
