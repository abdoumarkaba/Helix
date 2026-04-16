---
auto_execution_mode: 2
description: review play's code against project standards
---
# /review-play — Code Review Against Project Standards

Review the following: $ARGUMENTS

Perform a thorough code review. Be precise — quote the specific line or pattern
that is problematic, explain why it violates the standard, and show the correct version.

## Pass 1: Antipattern Scan

Check each antipattern from the master list:

- [ ] No string comparisons for distro/vendor detection — typed enums only
- [ ] No `.unwrap()` or `.expect()` outside `#[cfg(test)]` blocks
- [ ] All `/proc` and `/sys` paths use the injected `sys_root` parameter
- [ ] Rollback manifest written BEFORE any system state change (no exceptions)
- [ ] `play-helper` NOT called for read operations (only write-privileged ops)
- [ ] No blocking the main thread during downloads — indicatif + thread
- [ ] No silent fallbacks when critical components are missing — explicit plan warning
- [ ] No executable content in database entries
- [ ] No `shell=true` / `sh -c` subprocess spawning — `Command` with explicit args
- [ ] No GPU vendor conditionals in Orchestrator — in separate trait impls
- [ ] No hardcoded runner version string (e.g. `"GE-Proton9-27"` as a literal)
- [ ] No hardcoded `vm.max_map_count = 2097152` — use the hardware-proportional formula
- [ ] No hardcoded `nvidia-smi -lgc 1530,1530` — use `compute_nvidia_lock_clock()`
- [ ] AMD GPU paths return `TweakDecision::NotApplicable`, never error or panic
- [ ] No runtime network calls for decision-making

## Pass 2: Architecture Compliance

- [ ] Modules communicate only via typed structs through the Orchestrator
- [ ] No direct module-to-module calls
- [ ] The Orchestrator contains no tweak-specific logic
- [ ] Every tweak maps to a `TweakConstraint` struct entry
- [ ] `GameEnvironment` is the single source of truth — not bypassed

## Pass 3: Error Handling Quality

- [ ] All user-visible errors use typed `PlayError` variants with named fields
- [ ] Error messages carry enough context for the user to take action
- [ ] No swallowed errors (no `let _ = result` on important operations)
- [ ] `Drop` implementations log errors but never panic

## Pass 4: Security

- [ ] No path construction from game-provided data without sanitization
- [ ] No downloaded binary executed before SHA256 verification
- [ ] Every privileged operation goes through `play-helper`
- [ ] Audit log entry exists for every privileged operation

## Pass 5: Test Coverage

- [ ] Both happy path and all failure paths have tests
- [ ] Tests use injected `sys_root` (tempdir) — no real system access
- [ ] Mock `CommandRunner` used for subprocess calls in tests
- [ ] Idempotency tested (calling the same function twice produces the same result)

## Pass 6: Output Quality (Principle 6)

- [ ] Visual language consistent (`→` `⟳` `✓` `⊘` `⚠` `✗`)
- [ ] Plan output is understandable to a non-technical user
- [ ] Verbose output is sufficient for debugging without source access
- [ ] Every rationale string is a single, complete, human-readable sentence

## Summary

After all passes, provide:
1. A list of blockers (must fix before merge)
2. A list of suggestions (improvements, not blockers)
3. A confidence rating: Approved / Approved With Changes / Needs Rework
