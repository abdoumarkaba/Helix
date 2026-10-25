# 06 — User-facing output (terminal noise reduction)

## Problem
Current runs are effectively in debug mode: every `INFO`/`DEBUG` line from `play_core`
and `play_cli` is printed to stderr, so a normal launch dumps dozens of lines
(kernel tweaks, batched commands, stabilization checks, checkpoint events, etc.).
That is useful while developing, but it makes the tool look noisy and unfinished
for end users.

Debug detail should still be captured, but in the **log file**, not on the terminal.

## What a user should see (and why)

### On a fresh run
```
Log file: ~/.local/share/helix/logs/helix-20261025-190417-Balatro.log

▶ Analyzing game...

  Detected: NVIDIA RTX 3050 Laptop, Intel i7-13650HX, 16 GB RAM, Fedora 43

  Plan
  · Runner: Proton-GE 8-25
  · Prefix:  create new
  · Tweaks:  7 (vm.max_map_count, THP madvise, split-lock, ulimit, fsync, ...)
  · Warnings: MangoHud not installed

  Proceed? [y/N] y

  ✓ Packages already installed
  ↓ Downloading runner... (2.1 MB / 145 MB)
  ✓ Prefix created
  ✓ System tweaks applied
  ✓ Game launched (PID 164870)

  Press Ctrl+C to stop monitoring (game continues in background)
```

### On a validated (fast) run
```
Log file: ...

  ▶ Analyzing game...
  Fast launch: reusing saved configuration...
  ✓ Game launched (PID 164870)
  Press Ctrl+C to stop monitoring (game continues in background)
```

### On failure
```
  ✗ Game session failed

  Error: Runner download failed: ... (attempt 2/3): connection reset

  Next steps:
    1. Check your internet connection
    2. ...
  Log file: ...
```

### Why
- **Progress, not internals**: users care about phase transitions and outcomes, not
  individual sysctl writes or stabilization polling counts.
- **Errors must stand out**: hiding routine `INFO` lines makes real failures visible.
- **Trust**: a clean, short transcript reads as a finished product; a wall of debug
  text reads as a prototype.
- **Debuggability preserved**: the full trace still goes to the per-run log file,
  which `--log-path` / crash reports can reference.

## What to hide from the terminal by default
- `INFO`/`DEBUG` tracing lines from `play_core`/`play_cli`.
- Per-tweak application details (`Batched 2 system tweaks applied`, etc.).
- Stabilization polling (`stable_count=...`).
- Checkpoint/recovery event names.
- Internal phase-skip messages.

## What to keep on the terminal
- Log file path (once, at start).
- One-line phase progress (spinner or short status).
- Plan summary (game, runner, prefix action, tweak count, warnings).
- Confirmation prompt.
- Download progress (size/ETA) for long operations.
- Final success/failure + PID.
- Error context + next steps on failure.

## Implementation notes
- In `crates/play-cli/src/main.rs`, set the console tracing filter to `warn` (or `error`)
  instead of the current `"play_core=debug,play_cli=debug,info"`, while keeping the file
  appender at `debug`/`trace`.
- Keep `--verbose`/`-v` to restore the current noisy console output for developers.
- Route phase progress through the existing `set_progress_callback` / pre/post confirm
  callbacks rather than `tracing::info!`.
- Ensure `--dry-run` prints the full plan summary (already partially there) without
  debug noise.

## Verify
- `helix game.exe` on a fresh run prints ≤ ~15 terminal lines before the prompt.
- `helix game.exe` on a validated run prints ≤ ~6 lines.
- `helix --verbose game.exe` restores the current detailed output.
- The log file still contains the full debug trace.