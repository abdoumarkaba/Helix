---
agent: devin-local
session: speckled-muenster
created: 2026-10-25T12:36:06Z
---
# Complete Helix Project - Add detect-only Flag and Test Fixtures

Add --detect-only CLI flag to helix binary, create test fixtures directory, update CI workflow, and refresh CLAUDE.md status.

# Project Completion Plan for Helix

## Current Status Assessment

### Completed Components ✓
- **Hardware Detection** - Full implementation with 14+ tests
- **Binary Analysis (PE parsing)** - Full implementation with 17+ tests  
- **Planning Module** - DecisionEngine, TweakRegistry, PlanBuilder all implemented with tests
- **Execution Modules**:
  - LaunchModule - process spawning with stability checks
  - RunnerModule - download, SHA512 verification, extraction
  - PrefixModule - wineboot, prefix creation, Windows version config
  - SystemModule - tweak application via helix-helper
  - MangoHudModule - metrics parsing and analysis
- **Orchestrator** - full state machine with checkpoints and rollback
- **CLI (play-cli)** - argument parsing, error display, metrics, plan confirmation
- **play-helper** - privileged operations with security whitelists
- **CI Workflows** - ci.yml and cross-distro.yml exist
- **Build Configuration** - workspace Cargo.toml, .cargo/config.toml, rustfmt.toml
- **DESIGN.md** - comprehensive design document exists

### Test Results
- 152 unit tests passing in play-core
- 8 integration tests passing
- 1 test passing in play-helper
- **Total: 161 tests passing**

### Workspace Structure
- 4 crates: play-cli (binary: helix), play-core, play-helper (binary: helix-helper), play-db-tools
- **No play-detect-only binary exists** - CI references non-existent binary

---

## Remaining Work Items

### 1. Add --detect-only CLI Flag
**Status:** ✅ Completed - flag implemented and tested  
**Impact:** CI workflow integration detection smoke test fails  
**Action:** 
- Add `--detect-only` flag to helix CLI in play-cli/src/main.rs
- When enabled, run hardware detection only and output results
- Skip planning, confirmation, and execution phases
- Output detection results in structured format (JSON or human-readable)
- Update CI workflow to use: `cargo run --bin helix -- --detect-only`

### 2. Test Fixtures Directory
**Status:** ✅ Completed - directory + README + generator present  
**Impact:** Binary analysis tests rely on PE fixtures  
**Action:** 
- Create `tests/fixtures/binaries/` directory
- Add minimal PE fixture files:
  - d3d9.exe (D3D9 import)
  - d3d11.exe (D3D11 import)
  - d3d12.exe (D3D12 import)
  - vulkan.exe (Vulkan import)
  - eac.exe (EasyAntiCheat strings)
  - bink.exe (Bink video strings)
  - no_imports.exe (no recognizable imports)
- Document fixture creation process

### 3. CI Workflow Update
**Status:** ✅ Completed - uses helix --detect-only  
**Impact:** CI smoke test integration  
**Action:** 
- Update ci.yml line 37 from `cargo run --bin play-detect-only` to `cargo run --bin helix -- --detect-only`
- Verify the command syntax is correct

### 4. Documentation Updates
**Status:** ✅ Completed - CLAUDE.md refreshed  
**Impact:** Current status tracking  
**Action:** Update CLAUDE.md "Current Status" section to reflect completed work

---

## Implementation Steps

### Step 1: Add --detect-only Flag to helix CLI
- Add `#[arg(long)] detect_only: bool` to CliArgs struct
- In main(), check if detect_only is true
- If true, run detection only:
  - Create orchestrator
  - Call detect() method
  - Output hardware profile in JSON format
  - Exit without planning/execution
- Test locally with: `cargo run --bin helix -- --detect-only`

### Step 2: Create Test Fixtures
- Create directory structure: `tests/fixtures/binaries/`
- Create README.md documenting fixture creation
- Add stub/dummy files or documentation for PE binary creation
- Note: Actual PE binaries may need external tooling - focus on infrastructure first

### Step 3: Update CI Workflow
- Edit `.github/workflows/ci.yml` line 37
- Change to: `run: ~/.cargo/bin/cargo run --bin helix -- --detect-only`
- Or use: `run: ~/.cargo/bin/helix --detect-only` if installed

### Step 4: Update CLAUDE.md
- Move all completed items to "Completed ✓" section
- Update "Before First Commit" to reflect actual remaining work
- Remove completed items from "Not Yet Started"

### Step 5: Final Verification
- Run full test suite: `cargo test --workspace`
- Run lint checks: `cargo fmt --check && cargo clippy -- -D warnings`
- Verify all crates build: `cargo build --workspace --release`
- Test --detect-only flag locally
- Check CI workflow syntax

---

## Files to Create/Modify

### New Files
- `tests/fixtures/binaries/README.md` - fixture documentation

### Files to Modify
- `crates/play-cli/src/main.rs` - add --detect-only flag and detection-only logic
- `.github/workflows/ci.yml` - fix play-detect-only reference
- `CLAUDE.md` - update current status
- `tests/fixtures/binaries/` - create directory structure

---

## Verification Checklist

- [x] --detect-only flag added to helix CLI
- [x] --detect-only outputs hardware detection results
- [x] Test fixtures directory structure created
- [x] CI workflow uses helix --detect-only
- [x] CLAUDE.md status is accurate
- [ ] All tests pass (161+ tests)
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo build --workspace --release` succeeds
- [x] `cargo run --bin helix -- --detect-only` works locally

---

## Risks and Considerations

### --detect-only Implementation
- Need to ensure detection module can be called without exe_path
- Current detection requires exe_path for binary analysis
- May need to split detection into hardware-only + binary analysis
- Decision: For CI smoke test, hardware-only detection is sufficient
- Can pass a dummy exe_path or make it optional for detect-only mode

### PE Binary Fixtures
- Creating valid PE binaries requires external tooling or manual hex editing
- May need to use existing Wine test binaries or create stub PE files
- Alternative: Mock the binary analysis for tests (already done to some extent)
- For now: Create directory structure and documentation, actual binaries can be added later

---

## Priority Order

1. **High Priority** (blocks CI):
   - Add --detect-only flag to helix CLI
   - Update CI workflow

2. **Medium Priority** (test infrastructure):
   - Test fixtures directory and documentation

3. **Low Priority** (documentation):
   - CLAUDE.md update

---

## Estimated Effort

- --detect-only flag implementation: 30 minutes
- Test fixtures: 20 minutes (documentation only)
- CI update: 5 minutes
- CLAUDE.md update: 5 minutes
- Final verification: 10 minutes

**Total: ~70 minutes**
