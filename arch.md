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
- **Game launch and execution**, RimWorld for test (verified working: 90fps @ 30% GPU usage)

### ✅ Recent Fixes

**1. Permission denied error (os error 13)**

**Root Cause:**
Runner path was pointing to the Proton directory (`/path/to/GE-Proton10-34`) instead of the actual proton binary (`/path/to/GE-Proton10-34/proton`).

**Fix Applied:**
Updated `decision_engine.rs` to append `/proton` to the path when resolving `RunnerAction::AlreadyInstalled` from Steam compatibility tools directory.

**Status:**
Fixed. Runner now correctly points to the proton binary executable.

---

**2. Launch verification false positives**

**Root Cause:**
`try_wait()` on spawned child handle didn't detect Proton/Wine daemonization (parent exits, child continues). This caused false "launch successful" messages when the process had actually exited.

**Fix Applied:**
Implemented multi-layered verification in `launch.rs`:
- Polling checks every 2 seconds for 12 seconds
- `/proc/<pid>` existence verification
- Child process detection for daemonized runners
- Requires 50% of checks to pass for success

**Status:**
Fixed. Launch verification now correctly detects daemonization and zombie processes.

---

**3. Ulimit application timing**

**Root Cause:**
Writing to `/etc/security/limits.d/play.conf` only affects new PAM sessions, not the current game process.

**Fix Applied:**
Changed from limits.d file writing to direct `setrlimit()` call before spawning game process in `launch.rs`.

**Status:**
Fixed. Ulimit now applies to the game process immediately.

### 🚧 Known Issues / Blockers

**1. Long startup time (~10 minutes)**

**Symptoms:**
- RimWorld took ~10 minutes from launch to playable state
- 2-minute stall at 0fps during initial launch (MangoHud)
- Wayland compositor shows "not responding" dialog during stalls
- Game eventually stabilizes at 90fps with 30% GPU usage

**Root Cause:**
- First-run shader compilation by Proton/DXVK
- Large games have thousands of Vulkan shaders that compile on-demand
- Wayland compositor ping timeout (~5 seconds) triggers during heavy I/O
- No frame submission during shader compilation/loading phases

**Impact:**
- User experience: poor (long wait, confusing dialogs)
- Game functionality: works correctly after compilation
- Performance: excellent once loaded (90fps stable)

**Potential Mitigations:**
- Pre-compile shaders using DXVK cache tools
- Increase Wayland compositor timeout (user configuration)
- Add loading screen overlay to keep compositor happy
- Detect first-run and warn users about expected delay
- Set `PROTON_NO_ESYNC=1` if esync causes hangs (not yet implemented)

**Status:**
Known limitation of Proton/Wine on first launch. Not a blocker for core functionality, but UX issue.

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
