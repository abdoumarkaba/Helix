# /tweak — Add New Tweak to TweakRegistry

Add a new system optimization tweak: $ARGUMENTS

A tweak is DATA, not code. The Orchestrator iterates the TweakRegistry — it never
contains tweak-specific logic. Adding a new tweak is a one-file change.

## Step 1: Classify the Tweak
Determine which class this tweak belongs to:
- **Class A** — session-scoped, restored via Drop guard, no confirmation needed
- **Class B** — persistent, written to rollback manifest before applying, confirm once per game
- **Class C** — hardware-conditional (NVIDIA desktop only for v1), explicit user consent per invocation

## Step 2: Write the TweakConstraint Entry
```rust
TweakConstraint {
    tweak:               SystemTweak::/* name */,
    min_ram_mb:          None,          // or Some(16_384)
    kernel_version_min:  None,          // or Some(KernelVersion::new(5, 16, 0))
    gpu_vendor_required: None,          // None = applies to all vendors
    gpu_vendor_exclusions: vec![],
    cpu_arch_required:   None,
    requires_desktop:    false,         // true = skip on laptop GPUs silently
    reversible:          true,
    reboot_required:     false,
    reboot_resets:       false,         // true if sysctl resets on reboot anyway
    risk_level:          RiskLevel::Low,
    rationale:           "One sentence explaining why this tweak helps gaming.",
}
```

## Step 3: Application Method
Show the exact implementation — the env var to set, the sysfs path to write,
or the play-helper JSON command to issue.

If this is a session-scoped change (Class A/C): show the Drop guard implementation
with proper error logging (never panic in Drop).

If this is a persistent change (Class B): show the rollback manifest entry format
and the verification step that runs before applying.

## Step 4: Constraint Logic
Write the `evaluate_tweak()` call for this tweak. If constraints are not satisfied,
the tweak must be **invisible** in the plan output (not disabled, not warned about — invisible).

## Step 5: Plan Display Text
Write exactly what the user sees in the plan output for this tweak, in both the
normal case (will apply) and the skipped case (constraint not met, so nothing shown).

## Step 6: Tests
Write at minimum three tests:
1. Tweak applied correctly when constraints are satisfied
2. `TweakDecision::NotApplicable` returned when constraints are not satisfied
3. Drop guard restores previous value after the guard drops

Then add the entry to the TweakRegistry. Run `cargo test` to verify.
