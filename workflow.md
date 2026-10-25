# Helix — Pro Finish Workflow

Operating instructions for the coding agent ("beam") closing the Pro Finish
polish backlog (`00-overview.md` … `05-roadmap.md`, 28 items) on the Helix
repository (formerly `play`). This file is the execution contract: work the
items in the order given, do not skip the gate, do not change behavior the
backlog did not ask you to change.

"Desired logic" = the architecture fixed in `PLAY_MASTER_PROMPT.md` /
`play_blueprint.docx` / `tweaks.json`. §1 below extracts the parts of that
architecture a bug-fix pass is most likely to accidentally violate. Treat §1
as a checklist to re-run mentally before every commit, not just at the start.

---

## 0. Scope

- In scope: items 1–23 (must all be closed; this is the certified release).
- Phase 9 (items 24–28) is explicitly "nice-to-have / future" per
  `00-overview.md` — attempt only after 1–23 are green, and never let it
  block or delay them.
- Out of scope / not specified here: `00-overview.md` also notes "many files
  need removal, cost guide, workflows, etc for final release." None of that
  is broken down into files/fix/verify the way items 1–28 are. Do not
  improvise these — flag them back to the project owner as undefined work
  rather than guessing at a license-cost guide or a cleanup list. See
  Appendix A.

---

## 1. Non-negotiable invariants

Pulled from the master prompt / blueprint. A fix that violates one of these
is a wrong fix even if it closes the ticket.

1. **Plan → Confirm → Execute** is never skipped, except read-only
   detection/analysis (no confirmation needed) and `--yes` (skips the
   prompt but must still log loudly that it did).
2. **Idempotency** — running `helix game.exe` twice gives identical results;
   an operation checks current state before acting.
3. **Rollback-before-change** — every persistent (Class B) write is recorded
   in the rollback manifest *before* the system value changes. No
   exceptions, no reordering for convenience.
4. **Session-scoped (Class A/C) changes restore via Rust `Drop` guards**,
   unconditionally, including on panic.
5. **Every error is a typed `PlayError` variant with context.** Never
   `anyhow`, never a bare `String`, never silently swallowed.
6. **No `.unwrap()` / `.expect()` outside `#[cfg(test)]`.**
7. **No `shell=true` / `sh -c "..."` string interpolation** — `Command` with
   explicit args only.
8. **Tweak-as-data** — the Orchestrator and `SystemAgent` never contain
   tweak-specific branches. New/changed tweak logic goes into a `Tweak`
   struct in the registry (`tweaks/*.rs`), not into an agent.
9. **GPU-vendor logic lives behind a trait/impl block**, never an inline
   `if vendor == Amd` in a shared function.
10. **All `/proc`, `/sys` reads/writes go through an injectable root**
    (`proc_root` / `sys_root`) so they stay testable.
11. **Typed enums, not string matching**, for distro/vendor detection.
12. **`play-db` entries are pure data.** No executable fields, ever —
    nothing in this backlog should add one.
13. **`play-helper`'s whitelist is the sole source of truth** for what the
    privileged binary will touch. The only acceptable change to it in this
    backlog is the name-string fix in Item 2 — not loosening what it will
    write to.

---

## 2. Operating procedure

1. Work items **in the exact phase order in §3**. One item per commit.
2. Per item: re-read the current file(s) → implement the minimal fix
   described → run the item's own verification → run the **global gate**
   (below) → commit with the message given → only then move on.
3. If a fix genuinely requires touching a file not listed for that item,
   that's fine — compilation/tests may force it — but say so in the commit
   body. Don't widen scope beyond what's needed to compile and pass tests.
4. Never combine two numbered items into one commit, even trivially related
   ones (e.g. items 4 and 23, already merged into one card below, are the
   one explicit exception — the backlog itself treats them as the same fix).
5. If an item's instructions conflict with §1, §1 wins. Note the conflict in
   the commit body instead of silently picking one.
6. If the gate goes red and you can't make it green in a reasonable number
   of attempts, revert the working-tree changes for that item only
   (`git checkout -- <paths>` / `git reset --hard` to the last good commit),
   log why, and stop — don't proceed to the next item on a red base.
7. Don't rewrite or delete a test to make it pass. A red test after your fix
   means the fix is wrong or incomplete.

**Global gate** (run after every item, before committing):
```
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```
Toolchain is pinned at MSRV 1.75 — run the gate on that toolchain, not
whatever is locally newest.

**Pre-flight** (once, before Item 1):
```
git status           # must be clean
git checkout -b polish/pro-finish
cargo test --workspace   # record the baseline pass count
```

**Commit message format:** `<type>(<scope>): <summary> [helix-polish #<N>]`
where `<type>` is `fix`, `feat`, `refactor`, `chore`, `docs`, or `test` (given
per item below).

---

## ⚠ Path note (read before Phase 3)

Items 1–10 below are written against the repo's **current** paths
(`crates/play-core`, `crates/play-cli`, `crates/play-helper`), because that
is what's on disk before Phase 3 runs. Item 11 (Phase 3) renames those
directories to `crates/helix-*`. **From Phase 4 onward, every
`crates/play-*` path in this document means `crates/helix-*`.** Do the
rename in its own phase, in full, with the gate green, before starting
Phase 4 — don't rename incrementally alongside other fixes.

---

## 3. Execution order

### Phase 1 — Stop the silent failures
*Rationale (source): "make the tool not look broken."*

#### Item 1 — `--yes` parsed but never wired
- **Type/scope:** `fix(cli)`
- **Files:** `crates/play-cli/src/main.rs`
- **Problem:** `args.yes` is parsed but unused; `dialoguer::Confirm` still
  prompts even with `--yes`.
- **Fix:** when `args.yes` is set, call
  `orchestrator.set_confirm_callback(Box::new(|_| true))` before `run()`.
- **Verify:** `helix --yes game.exe` runs non-interactively to completion.
- **Preserve:** §1.1 — the skip must still be logged prominently (it's an
  explicit documented exception to Plan→Confirm→Execute, not a silent one).

#### Item 2 — Helper whitelist mismatch (silent failure every run)
- **Type/scope:** `fix(helper)`
- **Files:** `crates/play-core/src/execution/system.rs` (writes
  `/etc/security/limits.d/helix.conf`), `crates/play-helper/src/main.rs`
  (`FILE_WHITELIST` allows only `play.conf`)
- **Problem:** name mismatch between what's written and what the helper
  permits → the helper silently refuses the write on every run.
- **Fix:** align the names. Prefer `helix.conf` (matches what's already
  being written); update the whitelist entry and any docs referencing
  `play.conf`.
- **Verify:** the ulimit tweak succeeds (helper exits 0) and the file
  exists at the expected path.
- **Preserve:** §1.13 — only the name string changes; the whitelist keeps
  refusing every other path.

---

### Phase 2 — Rollback, error integrity, prefix verification
*Rationale (source): rollback manifest and error propagation are the trust
layer under the Plan→Confirm→Execute contract.*

#### Item 3 — Rollback manifest incomplete
- **Type/scope:** `fix(orchestrator)`
- **Files:** `crates/play-core/src/orchestrator.rs`
  (`record_rollback_entries`)
- **Problem:** `SplitLockMitigate`, `UlimitNofile`, and
  `NvidiaPersistenceMode` are applied but never recorded for rollback.
- **Fix:** add arms for `SystemTweak::SplitLockMitigate`,
  `SystemTweak::UlimitNofile`, `SystemTweak::NvidiaPersistenceMode`, each
  with its correct restore method.
- **Verify:** rollback after a Class B/C failure restores all three values.
- **Preserve:** §1.3 — the new entries must still be written *before* the
  corresponding value changes, same as the existing arms.

#### Item 4 / 23 — `final_error()` discards the real error
- **Type/scope:** `fix(errors)`
- **Files:** `crates/play-core/src/orchestrator.rs` (`final_error`),
  `crates/play-cli/src/main.rs` (`display_error_with_context`)
- **Problem:** `final_error()` replaces the originating `PlayError` with a
  generic `OrchestratorFailed`, so the CLI can never show the real cause.
- **Fix:** store the originating `PlayError` on the orchestrator (or return
  it directly) instead of replacing it. Propagate it unchanged through
  `run()` to the CLI. Remove `final_error()`'s generic-replacement behavior
  entirely — item 23 is this same fix, not a second one.
- **Verify:** a failing run prints the original error message and variant,
  not a generic wrapper.
- **Preserve:** §1.5 — every `PlayError` keeps its context all the way to
  the terminal.

#### Item 5 — Prefix verification layout (Wine-runner path)
> ⚠ **Not in the source priority list.** `00-overview.md`'s 9-step order
> enumerates every other correctness item (1, 2, 3, 4, 23) but omits 5. It's
> placed here because it's a correctness bug in the same `execution/` family
> as items 3–4 and there's no reason to ship it broken. Confirm this slot is
> acceptable, or move it, before relying on this document as final.
- **Type/scope:** `fix(prefix)`
- **Files:** `crates/play-core/src/execution/prefix.rs`
- **Problem:** `AlreadyExists`/`Create` verify `pfx/` when present, but the
  plain-Wine (non-Proton) path stores the prefix at `path` itself — so
  verification looks in the wrong place for WineGE/standalone-Wine runners.
- **Fix:** determine the Wine-prefix location by checking for `system.reg`
  (check `pfx/` first, then the base path); verify whichever is found. Add
  an integration test for a WineGE runner prefix.
- **Verify:** `cargo test -p play-core execution::prefix` (package name
  becomes `helix-core` after Phase 3 — run against whatever it's called at
  the time you execute this item).

---

### Phase 3 — Naming migration (`play` → `helix`)
*Rationale (source): "biggest 'amateurish' signal."*

#### Item 11 — Finish naming migration
- **Type/scope:** `refactor(naming)`
- **Files:** paths in `crates/play-cli/src/main.rs`,
  `crates/play-core/src/*`, `README`, `CLAUDE.md`; data dir
  `~/.local/share/play/` → `~/.local/share/helix/`. Crates:
  `play-core`/`play-cli`/`play-helper`/`play-db-tools` → `helix-*`
  (Cargo.toml package names + workspace members + all `use play_core::...`
  / `use play_cli::...` imports).
- **Problem:** the CLI already presents itself as `helix` in places (see
  items 1, 2's verify text) but the crate directories, package names,
  imports, and on-disk data directory are still `play-*`. This half-migrated
  state is the most visible unfinished signal in the repo.
- **Fix:**
  1. `git mv crates/play-core crates/helix-core` (same for `-cli`,
     `-helper`, `-db-tools`).
  2. Update each crate's `Cargo.toml` `name` and the workspace
     `members` list.
  3. Update every `use play_core::…` / `use play_cli::…` /
     `use play_helper::…` import across the workspace.
  4. Update `README` and `CLAUDE.md` references.
  5. Change the data-directory constant to `~/.local/share/helix/`, and add
     a one-time fallback: if `~/.local/share/helix/` doesn't exist yet but
     `~/.local/share/play/` does, **read** from the legacy path (don't
     delete or silently drop it) for this one release.
- **Verify:** `grep -rn "play" crates/` returns only the legacy-compat
  fallback code (clearly marked as a migration shim) and genuinely
  unrelated matches (e.g. "display" — check manually, don't just count
  lines); the built binary and `--help` output say `helix` everywhere a
  tool name appears.
- **Preserve:** §1.2 (idempotency) and state continuity — existing users'
  `~/.local/share/play/` state must still be *readable* after this change,
  not orphaned.
- **After this item:** re-run the full gate once more (every path in the
  workspace changed). Only proceed to Phase 4 once it's green.

---

### Phase 4 — Lint baseline
*Rationale (source): make CI meaningful.*

#### Item 6 — Clippy red on modern toolchains
- **Type/scope:** `chore(lint)`
- **Files:** many — missing `# Errors` doc sections on `Result`-returning
  functions; one unnecessary `as u64` cast in `detection.rs`.
- **Fix:** add `# Errors` doc sections (or
  `#[allow(clippy::missing_errors_doc)]` only where a doc section would add
  nothing — justify each such allow in a comment), and remove the redundant
  cast in `detection.rs`. **No behavior changes in this item** — doc
  comments and the one cast, nothing else.
- **Verify:** `cargo clippy --workspace -- -D warnings` passes on the pinned
  MSRV 1.75 toolchain.

---

### Phase 5 — Steam integration
*Rationale (source): "the feature that makes it actually useful."*

#### Item 12 — Steam launch integration (`helix %command%`)
- **Type/scope:** `feat(cli)`
- **Files:** `crates/helix-cli/src/main.rs`,
  `crates/helix-core/src/orchestrator.rs`
- **Fix:** accept an optional exe path via Steam's `%command%` launch-option
  placeholder; detect the Steam app id from env/args and set
  `GameIdentity.steam_app_id`.
- **Verify:** `helix %command%` works correctly when set as a Steam launch
  option.
- **Preserve:** §1.1 (Plan→Confirm→Execute still applies on a Steam
  launch — this item adds an entry point, not a bypass).

---

### Phase 6 — Licensing & contribution scaffold

#### Item 18 — LICENSE file
- **Type/scope:** `chore(license)`
- **Fix:** add `LICENSE` (GPL-3.0 full text) at the repo root.
- **Verify:** `cargo build` metadata and the README's license link resolve.

#### Item 19 — CONTRIBUTING.md + play-db scaffold
- **Type/scope:** `docs(contributing)`
- **Fix:** add `CONTRIBUTING.md` (dev setup, commit style, PR checklist).
  Scaffold/document `play-db/` (`runners.toml`, `entries/`; `validate.py`
  already exists — document it, don't rewrite it).
- **Verify:** a new contributor can build and test from the docs alone.
- **Open question:** `play-db` is a separate repo per the blueprint and is
  *not* one of the four crates renamed in Item 11. Flag to the project
  owner whether it should become `helix-db` for naming consistency — don't
  rename it unilaterally under this item.

#### Item 20 — CI real-hardware coverage
- **Type/scope:** `chore(ci)`
- **Files:** `.github/workflows/ci.yml`
- **Fix:** add a self-hosted or manually-triggered workflow that runs
  `--detect-only` on a GPU-equipped runner; keep the existing container job
  for compile/test.
- **Verify:** both workflow paths are green.

---

### Phase 7 — Detection completeness
*Rationale (source): needed for the "zero-config" promise.*

#### Item 7 — `nvidia_vbios_max_clock_mhz` always null
- **Type/scope:** `feat(detection)`
- **Files:** `crates/helix-core/src/modules/detection.rs` (GPU enrichment)
- **Fix:** query
  `nvidia-smi --query-gpu=clocks.max.graphics --format=csv,noheader,nounits`
  (or VBIOS via `nvidia-smi -q -d CLOCK`); parse into
  `GpuProfile.nvidia_vbios_max_clock_mhz`.
- **Verify:** on NVIDIA hardware, `--detect-only` reports a non-null MHz
  value and the clock-lock tweak appears in the plan.
- **Preserve:** the clock-lock value itself stays **computed** (95% of
  `vbios_max`, rounded to the nearest 15 MHz step, per the blueprint's §6
  formula) — this item only has to make the *input* non-null; don't
  hardcode a literal clock anywhere.

#### Item 8 — `vulkan_version` and `dx12_feature_level` always null
- **Type/scope:** `feat(detection)`
- **Files:** `crates/helix-core/src/modules/detection.rs`
- **Fix:** parse `vulkaninfo --summary` for `apiVersion`; set
  `dx12_feature_level` from a D3D12 feature-level probe, or default to
  `"12_1"` for NVIDIA when no probe is available.
- **Verify:** the plan sets `VKD3D_FEATURE_LEVEL` when VKD3D-Proton is
  selected as the translation layer.

#### Item 9 — Laptop GPU detection hardcoded
- **Type/scope:** `fix(detection)`
- **Files:** `crates/helix-core/src/modules/detection.rs`
- **Fix:** infer `is_laptop_gpu` from the model name (containing "Laptop",
  "Mobile") or chassis type; cross-check against `is_laptop_cpu`.
- **Verify:** on an RTX 3050 Laptop machine, detection reports
  `is_laptop_gpu: true`.

#### Item 10 — Shallow tool detection
- **Type/scope:** `feat(detection)`
- **Files:** `crates/helix-core/src/modules/detection.rs`
- **Fix:** `gamemode_installed` should check for the running `gamemoded`
  daemon (not just the binary); `dxvk_installed` should check for
  `dxvk.conf` / Mesa DXVK libraries; record versions for both.
- **Verify:** `--detect-only`'s `gaming_tools` output accurately reflects
  installed state.

---

### Phase 8 — Remaining quality/UX

#### Item 13 — Clean quit/stop command
- **Type/scope:** `feat(cli)`
- **Files:** `crates/helix-cli/src/main.rs`,
  `crates/helix-core/src/execution/launch.rs`
- **Fix:** add `--stop` (or signal handling) that terminates the tracked
  game PID and runs post-exit hooks.
- **Verify:** the game process exits cleanly; guards restore.
- **Preserve:** §1.4 — `ActiveGuards`' drop sequence must still run
  unconditionally on this path.

#### Item 14 — Cache/checkpoint dir names include game name
- **Type/scope:** `feat(state)`
- **Files:** `crates/helix-core/src/orchestrator.rs` (state dir naming),
  `plan_builder.rs` (cache dir)
- **Fix:** name directories `{sanitized_name}-{hash[:8]}` instead of a bare
  hash.
- **Verify:** directories are human-identifiable.
- **Preserve:** `exe_hash` stays the internal lookup key — only the
  on-disk label changes. Existing bare-hash directories from before this
  change must still resolve (read fallback) so in-progress checkpoints
  aren't orphaned.

#### Item 15 — Persistent tweak state
- **Type/scope:** `feat(state)`
- **Files:** `crates/helix-core/src/execution/system.rs`, new state file
  under the data dir
- **Fix:** persist applied Class B values; skip re-applying (and
  re-prompting) when the system is already at the recorded target.
- **Verify:** a second launch of the same game does not re-prompt for
  Class B tweaks.
- **Preserve:** §1.2 — this item *is* idempotency for Class B tweaks. If a
  value has drifted from the recorded target since last run, it must
  re-prompt, not be silently skipped.

#### Item 16 — GUI progress for long downloads
- **Type/scope:** `feat(cli)`
- **Files:** `crates/helix-cli/src/main.rs`
- **Fix:** optional `zenity`/`kdialog` progress dialog; fall back to the
  existing spinner when neither is available.
- **Verify:** manual smoke test on a large runner download.

#### Item 17 — Richer `--dry-run` / plan display
- **Type/scope:** `feat(cli)`
- **Files:** `crates/helix-cli/src/main.rs` (`interactive_confirm` /
  dry-run path)
- **Fix:** show game name, runner + version, download size, tweak count,
  and warnings.
- **Verify:** `helix --dry-run game.exe` prints a complete summary.
- **Preserve:** the Plan Display Format conventions from the blueprint's
  §12 — `→ ⟳ ✓ ⊘ ⚠ ✗` symbols, and a rationale line under each decision.
  This item adds information to that format; it doesn't replace it.

#### Item 21 — PE fixtures validity
- **Type/scope:** `test(fixtures)`
- **Files:** `crates/helix-core/tests/fixtures/binaries/build_fixtures.rs`
- **Fix — two options, pick one and say which in the commit:**
  - (a) align the generated PE layout with `build_minimal_pe32` (non-zero
    IAT/ILT) so `goblin` parses it, or
  - (b) **recommended** — delete the generated `.exe` fixtures and rely
    solely on the in-memory builders already used by `pe.rs`'s own test
    suite (`build_minimal_pe32`), which already proves out D3D12/D3D11/
    D3D9/Vulkan/OpenGL/Unknown, EasyAntiCheat, BattlEye, Unity, Source, and
    Bink-video cases. Preferred because it removes a second, unverified
    PE-construction code path instead of debugging it.
- **Verify:** `analyze_binary` succeeds on every fixture that remains.
- **Preserve:** the fixture-coverage requirement from the blueprint's §14
  (Priority 3) — D3D9 (32-bit), D3D11 (64-bit), D3D12, Vulkan,
  EasyAntiCheat, Bink/RAD video, and an unrecognized-imports case must all
  still exist as fixtures/tests after this change, whichever option you
  pick.

#### Item 22 — End-to-end happy-path test
- **Type/scope:** `test(integration)`
- **Files:** `crates/helix-core/tests/` (new integration test)
- **Fix:** mock the runner/prefix/system agents to drive the orchestrator
  from detection through validation; assert the checkpoint phase reaches
  `Validated`.
- **Verify:** `cargo test --workspace` includes and passes the new test.

---

### Phase 9 — Roadmap (optional, time-permitting, non-blocking)

Do not start these until items 1–23 are all green. None of these have a
"Files" / "Fix" spec as precise as the items above — treat each as a
research/spike task, not a drop-in patch, and write up a short design note
before touching code.

24. **Frame-drop investigation** — profile with MangoHud logs; distinguish
    shader-compile stalls from scheduler issues.
25. **First-run shader-compile UX** — detect a missing DXVK cache; print an
    estimated wait and a progress hint.
26. **Wayland "not responding" handling** — raise the compositor timeout
    guidance, or auto-set `WAYLAND_DEBUG`.
27. **Persistent play-db integration** — re-enable `fetch_database()` in
    `orchestrator.rs`; wire `DbEntry` into `PlanBuilder`.
28. **Metrics aggregation across sessions** — extend `--review-mangohud`
    with a trend view (avg FPS over the last N runs per game).

---

## 4. Definition of Done

- [ ] 1 — `--yes` wired
- [ ] 2 — helper whitelist name aligned
- [ ] 3 — rollback manifest covers split_lock/ulimit/nvidia_persistence
- [ ] 4/23 — real `PlayError` propagates to the CLI
- [ ] 5 — prefix verification checks the right path for Wine-only runners
- [ ] 11 — crates, imports, data dir, docs all say `helix`; legacy path
      still readable
- [ ] 6 — `cargo clippy --workspace -- -D warnings` is clean
- [ ] 12 — `helix %command%` works as a Steam launch option
- [ ] 18 — LICENSE present
- [ ] 19 — CONTRIBUTING.md present; play-db scaffold documented
- [ ] 20 — CI has both a container job and a GPU-hardware job
- [ ] 7 — NVIDIA VBIOS max clock detected
- [ ] 8 — Vulkan version / DX12 feature level detected
- [ ] 9 — laptop GPU correctly inferred
- [ ] 10 — gamemode/DXVK tool detection checks real state, not just a binary
- [ ] 13 — `--stop` terminates cleanly and guards restore
- [ ] 14 — state dirs are human-readable; old bare-hash dirs still resolve
- [ ] 15 — Class B tweaks don't re-prompt when already applied
- [ ] 16 — GUI download progress with spinner fallback
- [ ] 17 — `--dry-run` shows a complete plan summary
- [ ] 21 — every PE fixture parses successfully
- [ ] 22 — end-to-end happy-path integration test passes
- [ ] (optional) 24–28 — roadmap items, as time permits

All boxes above item 24 checked + global gate green on the final commit =
Pro Finish release candidate.

---

## Appendix A — Noted but not specified

From `00-overview.md`, not broken into a Files/Fix/Verify item anywhere in
the source backlog, and therefore **not** attempted by this workflow:

- "Cost guide" for the final release.
- Unspecified "workflows" beyond the CI workflow in Item 20.
- A "many files need removal" cleanup pass — no file list given.

Surface these back to the project owner as open questions before declaring
the release certified; don't infer scope for them from the rest of the
backlog.
