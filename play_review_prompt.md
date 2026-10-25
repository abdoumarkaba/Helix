# play — Comprehensive Code Review & Bug Identification Prompt

You are a senior Rust systems engineer specializing in Linux gaming tooling, Wine/Proton integration, and systems programming. You will perform an exhaustive review of the `play` project — a distro-agnostic Linux CLI that orchestrates Wine/Proton/DXVK/VKD3D-Proton/GameMode/MangoHud to run Windows games on Linux.

Your task is to identify **every bug, logic error, missing edge case, and design flaw** present in the codebase, then produce a **prioritized, actionable fix plan**. Do not stop at the obvious. Dig into every phase, every data path, every failure mode.

---

## Project Architecture Summary

The tool runs a deterministic pipeline:

```
play game.exe
  → Detection Phase   (hardware, GPU, kernel, audio, tools)
  → Planning Phase    (DecisionEngine selects config)
  → Confirmation Phase (user reviews plan)
  → Execution Phase   (apply tweaks, setup prefix, launch game)
```

Key modules:

- `modules/detection.rs` — detects CPU, GPU (via nvidia-smi/sysfs), kernel params, audio (pw-dump/pactl), Vulkan, installed tools
- `modules/decision_engine.rs` — selects runner, translation layer, sync mode, prefix config, tweaks
- `modules/tweak_registry.rs` — registry of all system tweaks (Class A session-scoped, Class B persistent via play-helper)
- `execution/launch.rs` — spawns game process via Proton runner
- `execution/system.rs` — applies sysctl/sysfs/ulimit tweaks via `play-helper` (privileged helper via pkexec/polkit)
- `execution/runner.rs` — downloads/verifies GE-Proton runners
- `execution/prefix.rs` — manages per-game Wine prefixes
- `play-helper` — minimal privileged binary, JSON stdin interface, sysfs whitelist

Privilege model: main process is unprivileged. Privileged writes go through `play-helper` invoked via `pkexec`.

RAII guards (`GovernorGuard`, `GpuPerfGuard`) restore session-scoped changes on Drop.

---

## Real Test Run Log (the tool just failed in production)

The following is the complete structured log from a real game launch attempt. Study every line carefully for anomalies:

```
2026-04-24T11:10:22.421836Z  INFO Starting fresh session event="fresh_start"
2026-04-24T11:10:22.421993Z  INFO Starting detection phase event="phase_start" phase="detection"
2026-04-24T11:10:22.422726Z  INFO kernel detection complete kernel_version=KernelVersion { major: 6, minor: 19, patch: 13 } has_futex2=true has_fsync=true
2026-04-24T11:10:22.423215Z  INFO cpu detection complete vendor=Intel model=13th Gen Intel(R) Core(TM) i7-13650HX physical_cores=14 logical_cores=20
2026-04-24T11:10:22.431021Z  INFO memory detection complete (meminfo) total_mb=15661 available_mb=7686 swap_total_mb=8191
2026-04-24T11:10:22.431045Z  INFO Attempting nvidia-smi detection...
2026-04-24T11:10:22.457329Z  INFO gpu detection vendor="NVIDIA" model=NVIDIA GeForce RTX 3050 6GB Laptop GPU vram_mb=6 driver=580.142 method="nvidia-smi"
2026-04-24T11:10:22.489814Z  INFO nvidia gpu enrichment complete vram_mb=6 driver_version=580.0.0
2026-04-24T11:10:22.489895Z  INFO Checking for PipeWire via wpctl...
2026-04-24T11:10:22.504331Z  INFO wpctl succeeded, parsing rate via pw-dump...
2026-04-24T11:10:22.520337Z  INFO pw-dump output: [...]
2026-04-24T11:10:22.520464Z  INFO Successfully detected PipeWire with rate: 48000
2026-04-24T11:10:22.520630Z  INFO display detection (no wlr-randr resolution) display_server="Wayland"
2026-04-24T11:10:22.520647Z  INFO distro detection complete distro=Fedora version=43
2026-04-24T11:10:22.520651Z  INFO Checking for MangoHud...
2026-04-24T11:10:22.521873Z  INFO MangoHud detected and available
2026-04-24T11:10:22.521885Z  INFO Detecting gaming tools...
2026-04-24T11:10:22.522996Z  INFO Wine detected: wine-11.0 (Staging)
2026-04-24T11:10:22.537846Z  INFO Winetricks detected
2026-04-24T11:10:22.539541Z  INFO GameMode not available
2026-04-24T11:10:22.539748Z  INFO DXVK not available
2026-04-24T11:10:22.539859Z  INFO Proton compatibility tools directory detected
2026-04-24T11:10:23.246412Z  INFO Vulkan available
2026-04-24T11:10:23.246433Z  INFO hardware detection complete: 0/1/1
2026-04-24T11:10:23.247767Z DEBUG Checkpoint written phase=Detected path=...pending-0000000018a9467b14ffecf2/checkpoint.json
2026-04-24T11:10:23.247785Z  INFO Detection phase complete
2026-04-24T11:10:23.248009Z  INFO Starting planning phase
2026-04-24T11:10:23.248018Z  INFO Checking hard blocks...
2026-04-24T11:10:23.248021Z  INFO No hard blocks found
2026-04-24T11:10:23.248024Z  INFO Selecting translation layer...
2026-04-24T11:10:23.248029Z  INFO Translation layer: Dxvk
2026-04-24T11:10:23.248032Z  INFO Configuring DXVK async...
2026-04-24T11:10:23.248036Z  INFO DXVK async: true
2026-04-24T11:10:23.248039Z  INFO Selecting runner...
2026-04-24T11:10:23.248045Z  INFO Runner: ProtonGE 10.34.0
2026-04-24T11:10:23.248050Z  INFO Resolving runner action...
2026-04-24T11:10:23.248077Z  INFO Found runner in Steam compatibility tools directory event="runner_found_in_steam" path=/home/abdoufoundit/.steam/steam/compatibilitytools.d/GE-Proton10-34/proton
2026-04-24T11:10:23.248085Z  INFO Selecting sync mode...
2026-04-24T11:10:23.248088Z  INFO Sync: fsync=true, esync=false
2026-04-24T11:10:23.248092Z  INFO Selecting audio driver...
2026-04-24T11:10:23.248095Z  INFO Audio driver: Pulse
2026-04-24T11:10:23.248099Z  INFO Selecting prefix configuration...
2026-04-24T11:10:23.248104Z  INFO Prefix: Win64, Windows: Win10
2026-04-24T11:10:23.248108Z  INFO Resolving prefix action...
2026-04-24T11:10:23.248115Z  INFO Resolving tweaks...
2026-04-24T11:10:23.248122Z  INFO Applied 6 tweaks
2026-04-24T11:10:23.248125Z  INFO Computing required packages...
2026-04-24T11:10:23.248129Z  INFO Required packages: 1
2026-04-24T11:10:23.248132Z  INFO Building resolved environment...
2026-04-24T11:10:23.248139Z  INFO MangoHud auto-enabled (installed and detected)
2026-04-24T11:10:23.248143Z  INFO Plan complete: 8 decisions, 12 tweaks, 0 warnings
2026-04-24T11:10:23.248436Z DEBUG Checkpoint written phase=Planned path=.../cf155cab.../checkpoint.json
2026-04-24T11:10:23.248450Z  INFO Planning phase complete
2026-04-24T11:10:23.248456Z  INFO Starting confirmation phase
2026-04-24T11:10:25.033088Z DEBUG Checkpoint written phase=Confirmed path=.../cf155cab.../checkpoint.json
2026-04-24T11:10:25.033115Z  INFO User confirmed execution
2026-04-24T11:10:25.033119Z  INFO Starting execution phase
2026-04-24T11:10:25.159661Z  INFO All required packages already installed
2026-04-24T11:10:25.159685Z  INFO Runner already installed, skipping download
2026-04-24T11:10:25.159711Z  INFO Prefix already exists, verifying
2026-04-24T11:10:25.159726Z DEBUG Prefix structure verified
2026-04-24T11:10:25.159753Z  WARN Kernel parameter /proc/sys/kernel/sched_autogroup does not exist on this system - skipping tweak
2026-04-24T11:10:28.743961Z  INFO Sysctl kernel.split_lock_mitigate = 1 applied successfully
2026-04-24T11:10:31.703194Z  INFO Ulimit nofile = 524288 applied successfully
2026-04-24T11:10:43.704603Z ERROR Orchestrator failed event="run_failed" error=Failed to launch game at /home/abdoufoundit/Games/ECHO_BROKEN/Echo.exe: game exited during launch (code: Some(1))
```

---

## Known Issues Already Identified (do not re-report these, they're being tracked)

The following four issues in `promote_report.rs` are already known:
1. `Err(_e)` discards the actual TOML parse error message
2. The `--json` flag is parsed but never consulted — output is always JSON
3. `&exe_hash[7..9]` is a bare byte-slice index that will panic on malformed input
4. Missing tracing instrumentation in the promotion pipeline

---

## Your Review Task

Perform an exhaustive analysis of the entire `play` codebase. For each issue you find, provide:

- **Location**: module, function, and line context
- **Severity**: Critical (breaks launch or corrupts system) / High (wrong behavior) / Medium (reliability gap) / Low (quality/UX)
- **Root cause**: exactly what is wrong and why
- **Fix**: the specific code change needed

### Category 1 — Bugs Visible in the Log

Scrutinize the log line by line. The hardware values, the tweak values being applied, the warning messages, the timing between operations, the tweak count discrepancies, the tool detection vs. planning decisions — cross-reference every logged fact against what the correct behavior should be. There are multiple bugs hiding in plain sight in this one log.

### Category 2 — Launch Failure Root Cause

The game exits with code 1 at the end. The log gives you ~12 seconds of silence before the error and zero stderr output. Enumerate every plausible cause and the detection/mitigation for each:

- Proton environment variable correctness (`STEAM_COMPAT_DATA_PATH`, `STEAM_COMPAT_CLIENT_INSTALL_PATH`, `PROTON_LOG`, etc.)
- Whether the working directory passed to Proton is correct (should it be the game directory, the Proton directory, or something else?)
- Whether GE-Proton requires specific Steam client shims that aren't present outside of Steam
- Missing execute permissions on the `.exe` or on the `proton` binary itself
- The prefix being in a partially initialized state that looks "verified" but isn't actually ready
- DXVK availability vs. what GE-Proton expects
- Whether `WINEPREFIX` vs `STEAM_COMPAT_DATA_PATH` is the correct env var for GE-Proton (they are NOT the same thing — GE-Proton uses Steam's prefix layout, not a raw WINEPREFIX path)
- Missing 32-bit libraries even for 64-bit games
- Whether `ulimit raise` via `limits.d` actually takes effect for a child process spawned from the current shell session (hint: `/etc/security/limits.d` changes require PAM re-login; they do NOT affect already-running processes or their children)

### Category 3 — Tweak System Correctness

Review the entire tweak registry and application logic:

- For every Class B sysctl tweak, verify: the key name is correct, the value being written is correct (not inverted), the rollback entry is written before the write happens
- For every Class A env-var tweak, verify: the variable name is correct, the value is correct, it ends up in the actual subprocess environment
- For `ulimit_raise` specifically: does writing to `/etc/security/limits.d/` actually affect a process that is about to be spawned? Research and explain the PAM session lifecycle problem here.
- Is `split_lock_mitigate` applied before or after the rollback entry is written? What happens to the system if the process crashes between writing the sysctl and writing the rollback?
- Does the DXVK async tweak have any effect when DXVK is bundled inside GE-Proton rather than installed system-wide?
- Is there a race between tweak application timing and the process spawn?

### Category 4 — Hardware Detection Correctness

Audit every value returned by the detection phase:

- GPU VRAM: what unit does `nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits` return? What unit does the code store? Is there a unit conversion happening anywhere? What downstream decisions are affected by a wrong VRAM value?
- The `vm_max_map_count` formula is supposed to be hardware-proportional (RAM + VRAM tiers). Given the VRAM value logged, what tier would be computed? Is it the right one?
- Kernel futex2 detection: the code checks `has_futex2=true` for kernel 6.19. Is this check `kernel >= 5.16`, a feature flag read, or something else? Is it possible to get a false positive here?
- `is_laptop_gpu` and `is_laptop_cpu` fields: these gate thermal-risky tweaks. How are they determined? If a laptop GPU is misidentified as desktop, what dangerous tweaks get applied?
- Audio detection: PipeWire rate is `48000`. Is this being set into the Wine registry in the prefix before launch? Is the registry key path correct for the audio driver selected (`Pulse` vs `PipewireWine`)?

### Category 5 — Runner Resolution and Path Handling

- The recent fix appended `/proton` to the Steam compat tools path. What other runner sources exist (custom paths, Lutris-managed, system-installed), and does each one get the right binary path vs. directory path?
- What happens if the runner binary at the resolved path is not executable? Is there a chmod or a clear error?
- When a runner download is needed (not pre-installed), what is the end-to-end flow? Where is the GE-Proton release manifest fetched from? Is the SHA256 verified before the binary is made executable? What happens if the download is interrupted mid-write?
- GE-Proton unpacks as a tarball. After extraction, does the tool correctly set the runner path to the `proton` script inside the extracted directory?
- What if `~/.steam/steam/compatibilitytools.d/` does not exist (fresh system, Steam never run)? Is the directory created or does the scan silently return nothing?

### Category 6 — Prefix Lifecycle

- What constitutes a "verified" prefix? Does the current verification check anything meaningful (wineserver can start, system.reg exists, drive_c exists) or is it a superficial directory existence check?
- GE-Proton uses `STEAM_COMPAT_DATA_PATH=<prefix_dir>` where `<prefix_dir>/pfx` is the actual WINEPREFIX. Does the code set up this directory layout correctly, or does it point WINEPREFIX directly at the prefix root?
- If prefix initialization (first-run `wineboot`) times out or returns a non-zero exit code, what happens?
- Is the audio sample rate Wine registry write happening into the correct subkey path for the PulseAudio Wine driver? What is the exact registry path and value name?

### Category 7 — Permission and Security Edge Cases

- The game exe is at `/home/abdoufoundit/Games/ECHO_BROKEN/Echo.exe`. Is execute permission required on `.exe` files for Proton/Wine to run them? If not, is the `+x` check adding noise?
- `play-helper` is invoked via `pkexec`. What happens when the user cancels the polkit dialog? Does the orchestrator handle `pkexec` returning exit code 126 (authorization cancelled) vs. 127 (not found) vs. 1 (generic failure)?
- The polkit auth dialog taking 3+ seconds per tweak (visible in the log timing) means each Class B tweak fires a separate `pkexec` invocation. Is this true? If so, a game with 5 persistent tweaks will show 5 separate password prompts. What is the intended batching behavior?
- What happens to the rollback manifest if `pkexec` fails partway through a multi-tweak sequence — tweak 1 applied, tweak 2 failed? Is the partial rollback manifest valid and executable?

### Category 8 — Error Message Quality at the Failure Point

- The final error is `"game exited during launch (code: Some(1))"`. This tells the user nothing actionable. What should the error message say? What diagnostic information should be captured (Proton log path, Wine stderr, last N lines of prefix/logs/)?
- Is there a `PROTON_LOG` or `WINEDEBUG` variable being set that would produce a log file the user could inspect?
- Should `play` detect "exited within N seconds of launch" as a launch failure vs. "ran for M minutes then crashed" as a runtime crash, and present different diagnostics for each?

### Category 9 — Checkpoint and Resumption Correctness

- The first checkpoint is written with a `pending-` hash prefix (`pending-0000000018a9467b14ffecf2`) but the subsequent checkpoints use the real exe hash (`cf155cab...`). What happens on resume? Does the pending checkpoint get cleaned up? Can a stale pending checkpoint cause incorrect resumption behavior?
- If the tool is killed after tweaks are applied but before the checkpoint is written, what happens on the next run? Will it re-apply already-applied tweaks? Is that safe (idempotent)?

### Category 10 — Tool Detection vs. Runtime Behavior Mismatch

- Detection says `DXVK not available` but planning selects `TranslationLayer::Dxvk`. GE-Proton bundles its own DXVK internally. Does the planner correctly distinguish between "system DXVK" and "bundled DXVK inside runner"? Are DXVK-specific env vars (like `DXVK_STATE_CACHE_PATH`, `DXVK_ASYNC`) correctly set for bundled-DXVK scenarios?
- `GameMode not available` is detected. The plan still references MangoHud. Is MangoHud being injected via `LD_PRELOAD` or via the `mangohud` wrapper binary? Does it degrade gracefully if MangoHud is not actually in the `LD_LIBRARY_PATH` at launch time?
- The `--json` flag in `promote_report.rs` is dead code. Is there any other dead-flag-style issue in the main `play` CLI where a flag is parsed but silently ignored?

### Category 11 — Timing, Blocking, and UX

- From the log: tweak application spans from 11:10:25 to 11:10:31 (6 seconds for two tweaks + ulimit). Then 12 more seconds of silence before the error. This suggests the game process was alive for ~12 seconds before exiting with code 1. Is the orchestrator waiting for the game process to exit completely before reporting the error, or is it detecting "exit during launch" correctly?
- Are any of the pkexec invocations blocking the main thread such that the TUI/plan display is frozen while waiting for polkit auth?

---

## Output Format

Produce your findings in this structure:

**SECTION 1 — CONFIRMED BUGS** (things that are definitely wrong based on the log and architecture)
For each: Location | Severity | Root Cause | Exact Fix

**SECTION 2 — HIGH-PROBABILITY BUGS** (very likely wrong based on analysis, not directly visible in log)
For each: Location | Severity | Root Cause | Exact Fix

**SECTION 3 — MISSING BEHAVIORS** (things that should exist but don't)
For each: What's missing | Impact | Implementation approach

**SECTION 4 — PRIORITIZED FIX PLAN**
Order the fixes from "do this first or nothing else matters" down to "improve later." Group fixes that must be done atomically (e.g., if fix A creates a new code path that fix B depends on).

Be specific. If a fix requires changing a constant, name the constant. If it requires a new enum variant, show the variant. If it requires a new environment variable, name the variable and its value format. Vague recommendations ("improve error handling") are not acceptable — every fix must be actionable by a Rust developer who has never seen this codebase.
