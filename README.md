# Helix

**Zero-config Windows gaming on Linux.**

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Platform](https://img.shields.io/badge/platform-Linux-informational.svg)](https://www.linux.org/)

Helix is a distro-agnostic CLI orchestrator that automatically configures your Linux system for optimal Windows gaming. It detects your hardware, selects the best compatibility stack (Proton-GE, DXVK, VKD3D), applies performance-optimizing kernel tweaks, and launches your game — all with a single command.

```bash
helix game.exe
```

No config files. No setup wizard. No flags required.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Features](#features)
- [How It Works](#how-it-works)
- [Tested Games](#tested-games)
- [System Tweaks](#system-tweaks)
- [Installation](#installation)
- [CLI Reference](#cli-reference)
- [Security Architecture](#security-architecture)
- [Technical Specifications](#technical-specifications)
- [Project Structure](#project-structure)
- [Development](#development)
- [Roadmap](#roadmap)
- [License](#license)

---

## Quick Start

### Install

```bash
curl -sSL https://raw.githubusercontent.com/abdoumarkt/Just-Play/main/install.sh | bash
```

Or install manually:

```bash
# Download latest release
curl -sSL https://github.com/abdoumarkt/Just-Play/releases/latest/download/helix-x86_64.tar.gz | tar xz

# Install binary
sudo mv helix /usr/local/bin/

# Install polkit policy (for system tweaks)
sudo mv com.github.abdoumarkt.play.policy /usr/share/polkit-1/actions/
```

### Run

```bash
helix /path/to/game.exe
```

Helix will:
1. Detect your hardware (GPU, CPU, kernel, audio)
2. Analyze the game binary (DirectX version, anti-cheat, engine)
3. Show you a plan with all optimizations
4. Ask for confirmation
5. Apply tweaks, configure Proton, and launch

---

## Features

### 🎮 Automatic Hardware Detection

- **GPU profiling**: NVIDIA (proprietary), AMD (RADV), Intel (ANV)
- **CPU detection**: Vendor, cores, architecture, AVX support
- **Kernel analysis**: Version, futex2/fsync support, current sysctl values
- **Audio backend**: PipeWire, PulseAudio, ALSA auto-detection
- **Display server**: Wayland/X11 with refresh rate detection

### 🔍 PE Binary Analysis

- DirectX version detection (D3D8–D3D12, Vulkan, OpenGL)
- Anti-cheat identification (EAC, BattlEye, Denuvo, VMProtect, GameGuard)
- Game engine detection (Unreal 5, Unity, Source, RE Engine, id Tech)
- Bink video detection for WMF codec configuration
- SHA256 hash computation for game identification

### ⚙️ Smart Configuration Selection

- **Translation layer**: DXVK (D3D8–D3D11), VKD3D-Proton (D3D12), or native
- **Runner selection**: Proton-GE with version from manifest
- **Sync mode**: fsync (kernel 5.16+) or esync fallback
- **Audio driver**: Pulse for PipeWire/PulseAudio, ALSA otherwise
- **Prefix architecture**: Win64 for 64-bit executables

### 🚀 System Optimization

12 performance-optimizing kernel tweaks applied automatically:

| Tweak | Purpose |
|-------|---------|
| `fsync` | Fast synchronization for multi-threaded games |
| `vm.max_map_count` | Prevent memory mapping failures |
| `THP madvise` | Reduce TLB pressure for large game heaps |
| `sched_autogroup` | Isolate game scheduler group |
| `split_lock_mitigate` | Disable stalls from unaligned atomics |
| `ulimit nofile` | High file descriptor limit for esync |
| CPU governor | Performance mode during gameplay |
| GPU perf mode | Maximum performance on NVIDIA/AMD |
| GameMode | CPU scheduler hints and power management |
| NVIDIA persistence | Keep kernel module loaded |
| NVIDIA clock lock | Eliminate frequency instability |

### 🔒 Security-First Design

- **Rollback guarantee**: Every system change is reversible
- **Polkit integration**: Privileged operations via `helix-helper`
- **SHA256 verification**: All downloaded runners are checksum-verified
- **No shell interpolation**: All commands use explicit argument arrays
- **Whitelisted operations**: Helper only allows specific sysctl/sysfs writes

### 📊 MangoHud Integration

- Automatic FPS logging to `~/.local/share/play/logs/`
- Session metrics: average, min, max, 1% low FPS
- CPU/GPU load, RAM/VRAM usage tracking
- Review past sessions with `helix --review-mangohud`

---

## How It Works

Helix follows a strict **Plan → Confirm → Execute** lifecycle. No system mutation occurs before you approve.

```
┌─────────────────┐
│   User Input    │  helix game.exe
│   (game.exe)    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Detection      │  • Hardware profiling (CPU, GPU, kernel, audio)
│  Phase          │  • Binary analysis (DX version, anti-cheat, engine)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Planning       │  • Decision engine selects optimal config
│  Phase          │  • Tweak registry evaluates constraints
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Confirmation   │  • Display plan to user
│  Phase          │  • User approves or aborts
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Execution      │  • Download/verify runner
│  Phase          │  • Create Wine prefix
│                 │  • Apply system tweaks
│                 │  • Launch game
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Validation     │  • Verify game is running
│  Phase          │  • Drop session guards (restore tweaks)
└─────────────────┘
```

### Rollback Guarantee

Every persistent system change (Class B/C tweaks) writes a **rollback manifest** before applying. If execution fails at any point, Helix walks the manifest in reverse to restore your system. If rollback itself fails, you receive explicit instructions for manual restoration.

---

## Tested Games

Real-world testing on Fedora 43 with NVIDIA RTX 3050 Laptop (6GB VRAM):

| Game | Result | Notes |
|------|--------|-------|
| **Sekiro: Shadows Die Twice** | ✅ High FPS, no stutters | Smooth gameplay throughout |
| **Elden Ring** | ✅ High FPS, no stutters | Excellent performance |
| **RimWorld** | ✅ 90 FPS @ 30% GPU | Verified stable |
| **Max Payne 3** | ✅ High FPS, no stutters | No issues detected |

---

## System Tweaks

Tweaks are classified by scope and reversibility:

### Class A — Session-Scoped (Auto-Restored)

Applied on launch, automatically restored when game exits. No confirmation needed.

| Tweak | Rationale |
|-------|-----------|
| `fsync` | Kernel ≥5.16 has futex2; outperforms esync for multi-threaded games |
| `esync` | Fallback when futex2 unavailable; reduces context switches |
| `dxvk_async` | Async shader compilation eliminates first-run hitches |
| `cpu_governor_performance` | Eliminates frequency scaling latency (desktop only) |
| `gamemode` | CPU scheduler hints and power management inhibition |

### Class B — Persistent (Rollback Manifest)

Written to system config, persisted across reboots. Confirmed once per game.

| Tweak | Rationale |
|-------|-----------|
| `vm.max_map_count` | Many games exceed default 65536; prevents OOM |
| `thp_madvise` | Reduces TLB pressure for large heaps (16GB+ RAM) |
| `sched_autogroup` | Isolates game scheduler group from desktop processes |
| `split_lock_mitigate` | Eliminates stalls from Windows unaligned atomics |
| `ulimit_nofile` | Esync requires high file descriptor limit (524288) |

### Class C — NVIDIA Desktop-Only (Explicit Consent)

Requires explicit user consent per invocation. Never applied on laptops.

| Tweak | Rationale |
|-------|-----------|
| `nvidia_persistence_mode` | Keeps kernel module loaded; eliminates first-launch latency |
| `nvidia_clock_lock` | Locks to 95% VBIOS max; eliminates boost instability |

---

## Installation

### One-Liner

```bash
curl -sSL https://raw.githubusercontent.com/abdoumarkt/Just-Play/main/install.sh | bash
```

### From Source

```bash
git clone https://github.com/abdoumarkt/Just-Play.git
cd Just-Play
cargo build --release

# Install binaries
sudo cp target/release/helix /usr/local/bin/
sudo cp target/release/helix-helper /usr/local/bin/

# Install polkit policy
sudo cp com.github.abdoumarkt.play.policy /usr/share/polkit-1/actions/
```

### Shell Completion

```bash
# Bash
helix --generate-completion bash > ~/.local/share/bash-completion/completions/helix

# Zsh
helix --generate-completion zsh > ~/.zsh/completions/_helix

# Fish
helix --generate-completion fish > ~/.config/fish/completions/helix.fish
```

### Requirements

- **OS**: Ubuntu 22.04+, Fedora 38+, Arch, Debian 12+
- **Rust**: 1.75+ (for building from source)
- **GPU**: NVIDIA (proprietary), AMD (RADV), or Intel (ANV)
- **Tools**: `pciutils`, `vulkan-tools` (for hardware detection)

---

## CLI Reference

```
helix [OPTIONS] <game.exe>

Commands:
  helix game.exe              Launch game with optimal configuration
  helix --dry-run game.exe    Show plan without executing
  helix --undo game.exe       Rollback previous session
  helix --review-mangohud     Review FPS logs from past sessions
  helix --report game.exe     File a crash report
  helix --generate-completion <shell>  Generate shell completions

Options:
  -y, --yes         Auto-confirm without interactive prompt
  -v, --verbose     Enable detailed tracing output
  --dry-run         Plan only, do not execute
  --undo            Rollback previous session
  --report          File a crash report for last session
  --review-mangohud Review MangoHud logs from previous runs
  --force-fresh     Ignore checkpoint and start fresh session
  --log-path        Show log file location
  --about           Show about information
  --help            Show help
  --version         Show version
```

### Examples

```bash
# Basic launch
helix ~/games/EldenRing.exe

# Preview configuration without applying
helix --dry-run ~/games/Sekiro.exe

# Non-interactive mode (for scripts)
helix --yes ~/games/RimWorld.exe

# Analyze past session performance
helix --review-mangohud

# Rollback a failed session
helix --undo ~/games/game.exe
```

---

## Security Architecture

### Privilege Escalation

Helix uses **polkit** for secure privilege escalation:

```
helix (unprivileged)
       │
       │ pkexec (polkit auth)
       ▼
helix-helper (privileged)
       │
       ├──► Writes to /proc/sys
       ├──► Writes to /sys
       └──► Writes to /etc/security/limits.d
```

The polkit policy (`com.github.abdoumarkt.play.policy`) allows active admin users to authenticate via `auth_admin_keep`, caching the password for the session.

### Whitelisted Operations

The `helix-helper` binary only accepts these operations:
- `sysctl-write <key> <value>` — Write to `/proc/sys`
- `sysfs-write <path> <value>` — Write to `/sys`
- `write-file <path> <content>` — Write to `/etc/security/limits.d`

All other operations are rejected.

### Runner Verification

All downloaded Proton-GE runners are verified against SHA256 checksums from the official release. If verification fails, the file is deleted immediately.

### No Shell Interpolation

Helix never uses `sh -c` or `shell: true`. All commands are executed with explicit argument arrays, preventing injection attacks from file paths containing spaces or special characters.

---

## Technical Specifications

| Component | Details |
|-----------|---------|
| **Language** | Rust 2021 Edition |
| **Minimum Rust** | 1.75 |
| **License** | GPL-3.0 |
| **Binary Size** | ~5MB (stripped, LTO) |
| **Runtime Dependencies** | Zero (compiled binary) |
| **Build Profile** | Release: opt-level 3, LTO thin, codegen-units 1 |

### Supported Runtimes

| Runner | Status |
|--------|--------|
| Proton-GE | ✅ Primary |
| Proton Official | ✅ Supported |
| Wine-GE | ✅ Supported |
| Wine Staging | ✅ Supported |
| Soda Wine | ✅ Supported |

### GPU Support

| Vendor | Driver | Status |
|--------|--------|--------|
| NVIDIA | Proprietary | ✅ Full support |
| AMD | RADV (Mesa) | ✅ Full support |
| Intel | ANV (Mesa) | ✅ Full support |

---

## Project Structure

```
helix/
├── crates/
│   ├── helix-cli/           # CLI entry point, argument parsing
│   │   └── src/
│   │       └── main.rs      # clap-based CLI with tracing
│   │
│   ├── helix-core/          # Core library — all logic
│   │   └── src/
│   │       ├── binary/      # PE analysis (goblin)
│   │       ├── execution/   # Launch, runner, prefix, system tweaks
│   │       ├── guards/      # RAII guards for session tweaks
│   │       ├── models/      # GameEnvironment, GamePlan, PlayError
│   │       ├── modules/     # Detection, planning, validation
│   │       └── orchestrator.rs  # Lifecycle coordinator
│   │
│   ├── helix-helper/        # Privileged helper (polkit)
│   │   └── src/main.rs      # Whitelisted sysctl/sysfs writes
│   │
│   └── helix-db-tools/      # Database maintenance utilities
│
├── helix-db/                # Community game database
│   ├── runners.toml         # Proton-GE manifest with SHA256
│   └── entries/             # Per-game compatibility notes
│
├── tests/
│   └── fixtures/            # PE binaries, procfs fixtures
│
├── DESIGN.md                # Architecture documentation
├── arch.md                  # High-level flow diagrams
└── install.sh               # One-liner installer
```

### Key Modules

| Module | Responsibility |
|--------|----------------|
| `Orchestrator` | Coordinates all phases, manages checkpoints |
| `DetectionModule` | Hardware profiling via /proc, /sys, commands |
| `PlanningModule` | Decision engine, produces GamePlan |
| `SystemModule` | Applies tweaks, manages RAII guards |
| `LaunchModule` | Spawns game process with runner |
| `ValidationModule` | Verifies game is running correctly |

---

## Development

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install development tools
rustup component add clippy rustfmt
```

### Build & Test

```bash
# Check compilation
cargo check --workspace

# Run tests
cargo test --workspace

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt -- --check
```

### Architecture Rules

1. **No `.unwrap()` outside tests** — Use `?` with typed `PlayError`
2. **No hardcoded `/proc` or `/sys` paths** — Use injectable roots
3. **No `unsafe` without justification** — Comment why it's needed
4. **Tweaks are data** — Logic lives in `DecisionEngine`, not `TweakRegistry`
5. **Rollback manifest before mutation** — Never write first, manifest later

### Adding a New Tweak

Tweaks are defined as pure data in `TweakRegistry`. See `tweak.md` for the full process:

1. Add `TweakConstraint` entry to registry
2. Implement application method (sysfs write, env var, etc.)
3. Add rollback handling if Class B/C
4. Write tests for applied/skipped/restored cases

---

## Roadmap

### Near-Term

- [ ] Steam integration (`helix %command%` launch option)
- [ ] Persistent tweak state (fewer password prompts)
- [ ] GUI progress dialog for long operations
- [ ] Game name in cache directory (not just hash)

### Known Limitations

- **First-run shader compilation**: ~2-minute stall on new games while DXVK compiles shaders. This is a Proton/Wine limitation, not Helix-specific.
- **Wayland "not responding" dialogs**: May appear during heavy I/O. Increase compositor timeout or wait for shader compilation to complete.

---

## Contributing

Contributions are welcome! Please read:

- `DESIGN.md` for architecture philosophy
- `.clinerules` for code standards
- `module.md` for adding new modules
- `tweak.md` for adding new tweaks

### Commit Style

- Use conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`
- Reference issues: `fix: handle daemonized Proton process (#123)`

---

## License

Helix is licensed under the **GNU General Public License v3.0**.

```
Helix - Zero-config Windows gaming on Linux
Copyright (C) 2024 abdoumarkt

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
```

---

## Acknowledgments

Helix builds on the incredible work of:

- [Valve/Proton](https://github.com/ValveSoftware/Proton) — Windows compatibility
- [GloriousEggroll/proton-ge-custom](https://github.com/GloriousEggroll/proton-ge-custom) — Proton-GE builds
- [doitsujin/dxvk](https://github.com/doitsujin/dxvk) — DirectX 9/10/11 translation
- [HansKristian-Work/vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton) — DirectX 12 translation
- [flightlessmango/MangoHud](https://github.com/flightlessmango/MangoHud) — Performance overlay
- [ValveSoftware/wine](https://github.com/ValveSoftware/wine) — Wine runtime

---

<p align="center">
  <strong>One command. Zero config. Just play.</strong>
</p>