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
- **Checkpoint system** for crash recovery and session resumption
- **Fast-launch mode** - skip detection/planning/confirmation when validated checkpoint exists
- **Multi-distro CI testing** (Ubuntu 24.04, Fedora 41, Arch Linux, Linux Mint 22)

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

---

**4. System tweak persistence (BatchedTweakCommand mismatch)**

**Root Cause:**
`play-core` included `UlimitNofile` in `BatchedTweakCommand` enum, but `play-helper` was missing this variant, causing deserialization failures when batching system tweaks.

**Fix Applied:**
- Added `UlimitNofile(u64)` variant to `play-helper`'s `BatchedTweakCommand` enum
- Added handler in `play-helper`'s `batch_execute()` to write to `/etc/security/limits.d/play.conf`
- Modified `play-core`'s `apply_tweaks()` to include `UlimitNofile` in the batch instead of separate call
- Made batch failures return errors instead of silently succeeding

**Status:**
Fixed. Ulimit tweaks are now properly batched with other Class B tweaks via play-helper.

---

**5. Checkpoint fast-launch path not found**

**Root Cause:**
Orchestrator created a temporary state directory (`pending-*`) before detection, then renamed to SHA256 hash after detection. The CLI's `create_orchestrator()` always used the temp path, so `recover()` couldn't find existing checkpoints.

**Fix Applied:**
- Modified `create_orchestrator()` to search for existing checkpoint by exe_path before creating orchestrator
- If checkpoint found, use that state root directly via `Orchestrator::with_roots()`
- Added `Planned` to `can_resume()` phases (previously only `Confirmed` and `Executed`)

**Status:**
Fixed. Fast-launch now correctly finds and uses existing validated checkpoints.

---

**6. SchedAutogroup warning when disabled**

**Root Cause:**
Code checked if `/proc/sys/kernel/sched_autogroup` exists even when disabling the tweak, causing a warning when the parameter doesn't exist but we're not trying to use it.

**Fix Applied:**
Only check kernel parameter existence when `enabled: true`. When disabling, the parameter doesn't need to exist.

**Status:**
Fixed. No spurious warnings when disabling sched_autogroup.

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

---

**2. Occasional frame drops**

**Symptoms:**
- Huge frame drops occur occasionally during gameplay
- Not consistent timing or pattern

**Root Cause:**
Unknown - needs investigation.

**Status:**
Open issue. Requires profiling and log analysis.

---

**3. No clean quit action**

**Symptoms:**
- After game launch, user must use desktop window manager's X button to quit
- No CLI command to cleanly stop the game
- Feels amateurish

**Root Cause:**
No quit/stop command implemented in CLI.

**Status:**
Open issue. Need to implement a clean game termination mechanism.

---

**4. Directory naming for debugging**

**Symptoms:**
- Cache directories at `~/.local/share/play/cache/` use pure hashes
- Checkpoint directories use pure hashes
- Difficult to identify which game a directory belongs to

**Root Cause:**
Using SHA256 hashes exclusively for directory names.

**Potential Fix:**
Use format like `{game_name}-{hash}` or `{game_name}-{timestamp}-{hash}` for easier identification.

**Status:**
Open issue. Design decision needed.

---

**5. System tweak persistence UX**

**Symptoms:**
- User must type password on every launch for pkexec
- Frustrating for frequent launches
- Some tweaks are "safe" and could be persistent

**Root Cause:**
All Class B/C tweaks require pkexec authentication every time.

**Potential Fix:**
- Categorize tweaks by safety level
- Keep "safe" tweaks persistent (e.g., ulimit, vm.max_map_count)
- Only prompt for "unsafe" tweaks (e.g., governor changes)
- Or implement a "trust this system" mode

**Status:**
Open issue. UX design decision needed.

---

**6. Steam integration**

**Symptoms:**
- Most users launch games through Steam
- No seamless way to use play's config with Steam

**Potential Fix:**
Support Steam launch options like `play %command%` to wrap Steam's game launching.

**Status:**
Open issue. Feature request.

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
