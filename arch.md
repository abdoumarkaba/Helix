# Play Architecture

## Overview
Play is a distro-agnostic Linux CLI that orchestrates existing open-source gaming compatibility stacks (Proton, Wine, DXVK, etc.) to run Windows games on Linux with optimal performance.

## High-Level Flow

```
User Input (game.exe)
        |
        v
+-------------------+
|   Orchestrator    |  <-- Main entry point, coordinates all phases
+-------------------+
        |
        v
+-------------------+
|  Detection Phase  |  <-- Hardware, GPU, audio, tools detection
+-------------------+
        |
        v
+-------------------+
|  Planning Phase   |  <-- DecisionEngine selects optimal config
+-------------------+
        |
        v
+-------------------+
| Confirmation Phase|  <-- User reviews plan before execution
+-------------------+
        |
        v
+-------------------+
|  Execution Phase  |  <-- Download runner, apply tweaks, launch game
+-------------------+
```

## Core Modules

### crates/play-core/src/

```
models/
├── environment.rs      # GameEnvironment, HardwareProfile, GraphicsConfig
├── plan.rs             # GamePlan, SystemTweak, LaunchAction, RunnerAction
├── errors.rs           # PlayError enum
└── identity.rs         # GameIdentity (exe path, hash, PE arch)

modules/
├── detection.rs        # Hardware detection (CPU, GPU, kernel, audio)
├── decision_engine.rs  # Selects optimal config based on hardware/game
├── plan_builder.rs     # Builds GamePlan from decisions
├── tweak_registry.rs   # Registry of all known system tweaks
└── orchestrator.rs     # Main coordinator, runs all phases

execution/
├── launch.rs           # Spawns game process via runner
├── system.rs           # Applies system tweaks (sysctl, limits.d)
├── runner.rs           # Downloads/verifies Proton runners
├── prefix.rs           # Manages Wine prefixes
└── package.rs          # Installs required system packages
```

## Decision Flow

```
GameIdentity (exe)
        |
        v
+-------------------+
| DecisionEngine    |
+-------------------+
        |
        +---> Translation Layer (Dxvk/Vkd3d)
        |
        +---> Runner (ProtonGE version)
        |
        +---> Sync Mode (fsync/esync)
        |
        +---> Audio Driver (Pulse/PipeWire)
        |
        +---> Prefix Config (Win64/Win10)
        |
        +---> System Tweaks (7 tweaks applied)
```

## System Tweak Architecture

```
SystemModule::apply()
        |
        +---> Class A (Session-scoped, RAII guards)
        |       |
        |       +---> GovernorGuard (CPU frequency)
        |       |
        |       +---> GpuPerfGuard (GPU performance mode)
        |
        +---> Class B (Persistent, via play-helper)
                |
                +---> sysctl writes (/proc/sys/*)
                |
                +---> sysfs writes (/sys/*)
                |
                +---> limits.d writes (/etc/security/limits.d/*)
```

## Privilege Escalation

```
play-core (unprivileged)
        |
        | pkexec (polkit auth)
        v
play-helper (privileged)
        |
        +---> Writes to /proc/sys
        |
        +---> Writes to /sys
        |
        +---> Writes to /etc/security/limits.d
```

Polkit policy: `/usr/share/polkit-1/actions/com.github.abdoumarkt.play.policy`
- Allows active admin users to authenticate via `auth_admin_keep`
- Password caching enabled for session

## Current Implementation Status

### ✅ Completed
- Hardware detection (CPU, GPU, kernel, audio, Vulkan)
- Decision engine for optimal config selection
- System tweak application with graceful degradation
- Polkit integration for privilege escalation
- Runner download and verification
- Prefix creation and management
- Package installation
- Kernel parameter existence checks (sched_autogroup)
- Helpful error messages for permission issues

### ✅ Recent Fix

**Issue: Game launch fails with "Permission denied (os error 13)"**

**Root Cause:**
Runner path was pointing to the Proton directory (`/path/to/GE-Proton10-34`) instead of the actual proton binary (`/path/to/GE-Proton10-34/proton`).

**Fix Applied:**
Updated `decision_engine.rs` to append `/proton` to the path when resolving `RunnerAction::AlreadyInstalled` from Steam compatibility tools directory.

**Status:**
Fixed. Runner now correctly points to the proton binary executable.

## Key Data Structures

```rust
GamePlan {
    env: GameEnvironment,           // All detected/selected config
    runner_action: RunnerAction,    // Download or AlreadyInstalled
    prefix_action: PrefixAction,    // Create or AlreadyExists
    launch_action: LaunchAction,    // Spawn with runner_path
    tweaks: Vec<PlannedTweak>,      // System tweaks to apply
}

LaunchAction::Spawn {
    exe_path: PathBuf,              // Game executable
    working_dir: PathBuf,           // Game directory
    runner_path: PathBuf,           // Proton installation path
    runner_type: RunnerType,        // ProtonGE, Wine, etc.
    env: HashMap<String, String>,   // Environment variables
}
```

## Error Handling Strategy

- **Class A tweaks**: Fail hard - critical for game performance
- **Class B tweaks**: Graceful degradation - warn and continue
- **Runner download**: Retry with exponential backoff
- **Game launch**: Detailed error messages with actionable fixes
