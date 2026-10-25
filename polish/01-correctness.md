# 01 — Correctness / latent bugs

Fix before calling the release done.

## 1. `--yes` parsed but never wired
- Files: `crates/play-cli/src/main.rs`
- Issue: `args.yes` is parsed but never used; `dialoguer::Confirm` still prompts.
- Fix: when `args.yes`, call `orchestrator.set_confirm_callback(Box::new(|_| true))` before `run()`.
- Verify: `helix --yes game.exe` runs non-interactively to completion.

## 2. Helper whitelist mismatch (silent failure every run)
- Files: `crates/play-core/src/execution/system.rs` (writes `/etc/security/limits.d/helix.conf`), `crates/play-helper/src/main.rs` (`FILE_WHITELIST` allows only `play.conf`)
- Fix: align names — either helper allows `helix.conf` or system writes `play.conf`. Prefer `helix.conf` and update whitelist + docs.
- Verify: ulimit tweak succeeds (helper exit 0) and file exists.

## 3. Rollback manifest incomplete
- Files: `crates/play-core/src/orchestrator.rs` (`record_rollback_entries`)
- Missing entries: `split_lock_mitigate`, `ulimit_nofile`, `nvidia_persistence_mode`.
- Fix: add arms for `SystemTweak::SplitLockMitigate`, `UlimitNofile`, `NvidiaPersistenceMode` with correct restore methods.
- Verify: rollback after a Class B/C failure restores all applied values.

## 4 / 23. `final_error()` discards the real error
- Files: `crates/play-core/src/orchestrator.rs` (`final_error`), `crates/play-cli/src/main.rs` (`display_error_with_context`)
- Fix: store the originating `PlayError` on the orchestrator (or return it directly) instead of replacing with generic `OrchestratorFailed`. Propagate through `run()`.
- Verify: failing run prints the original error message and type.

## 5. Prefix verification layout (Wine-runner path)
- Files: `crates/play-core/src/execution/prefix.rs`
- Issue: `AlreadyExists`/`Create` verify `pfx/` when present but the plain-Wine (non-Proton) path stores the prefix at `path` itself.
- Fix: determine wine-prefix location by checking for `system.reg` (pfx first, then base); verify that. Add an integration test for a WineGE runner prefix.
- Verify: `cargo test -p play-core execution::prefix`.

## 6. Clippy red on modern toolchains
- Files: many (missing `# Errors` doc sections on `Result`-returning fns; `detection.rs` unnecessary `as u64` cast).
- Fix: add `# Errors` sections (or `#[allow(clippy::missing_errors_doc)]` where appropriate) and remove the redundant cast.
- Verify: `cargo clippy --workspace -- -D warnings` passes on the pinned toolchain (MSRV 1.75).
