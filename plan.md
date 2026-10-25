# /plan — Feature Planning Command

Before implementing: $ARGUMENTS

Produce a structured written plan. Do NOT write any code yet.

## Step 1: Identify Affected Modules
List every module in the pipeline that this change touches (DetectionModule, PlanningModule,
ExecutionModules, Orchestrator, play-helper, play-db). For each, describe what input it
receives and what output it produces for this feature.

## Step 2: New Types Required
List every new struct, enum, or type alias this feature needs. Show the Rust definition
with doc comments. Verify all enums are exhaustive — no catch-all string arms.

## Step 3: Error Cases
List every way this can fail. For each, identify the `PlayError` variant (existing or new).
Show the error message the user would see.

## Step 4: Eight Principles Checklist
For each of the 8 inviolable principles, explicitly state whether this plan satisfies it:
- Plan→Confirm→Execute
- Idempotency
- Rollback guarantee
- Fail loudly
- Every decision explainable in one sentence
- Output as first-class interface
- Security by default
- Tweak is data, not code

If any principle is violated, STOP and redesign that part of the plan.

## Step 5: Antipattern Check
Scan the plan against every item in the ANTIPATTERNS list in CLAUDE.md.
Call out any risk explicitly.

## Step 6: Test Plan
List the unit tests, integration tests, and property-based tests this feature requires.
Include tests for the failure paths, not just the happy path.

## Step 7: Implementation Order
List the files to create/modify in dependency order (types first, then logic, then tests).
Show file paths, not just module names.

Only after this plan is reviewed and approved should implementation begin.
