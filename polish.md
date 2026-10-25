# Helix — "Pro Finish" Polish Backlog

Assessment of what remains for a certified, professional-grade release.
No code changes made for this document.

## Correctness / latent bugs (fix before calling it done)

1. `--yes` parsed but never wired — non-interactive launches still block on dialoguer. Orchestrator::set_confirm_callback exists; main.rs never calls it when args.yes.
2. Helper whitelist mismatch — system.rs writes /etc/security/limits.d/helix.conf, but helix-helper's FILE_WHITELIST only allows /etc/security/limits.d/play.conf. The ulimit tweak fails the whitelist check every run.
3. Rollback manifest incomplete — record_rollback_entries covers only vm.max_map_count, thp, sched_autogroup, nvidia_clock_lock. Missing: split_lock_mitigate, ulimit_nofile, nvidia_persistence_mode.
4. final_error() discards the real error — returns generic OrchestratorFailed { reason: "orchestrator failed" }, losing the actual cause in logs/UI.
5. Prefix verification layout — only partially fixed (pfx check added). Wine-runner (non-Proton) path still untested end-to-end.
6. Clippy red on modern toolchains — 41 errors (missing # Errors docs, unnecessary cast). CI on newer Rust fails outright.

## Detection gaps (needed for the "zero-config" promise)

7. nvidia_vbios_max_clock_mhz always null -> clock-lock tweak never applies (TODO: query nvidia-smi).
8. vulkan_version and dx12_feature_level always null -> VKD3D feature-level env var never set.
9. Laptop GPU detection hardcoded false in one path (TODO in detection.rs).
10. gamemode_installed / dxvk_installed detection is shallow (binary presence only).

## UX / product polish

11. Naming migration half-done — binary is helix, but paths are ~/.local/share/play/, crates are play-*, logs say play.
12. No Steam launch integration (helix %command%) — the headline use case.
13. No clean quit/stop command for a running game.
14. Cache/checkpoint dirs are pure hashes — impossible to debug by hand.
15. Password prompt every launch — persistent tweak state not implemented.
16. No GUI progress for long downloads.
17. --dry-run output and plan display could be richer (game name, runner version, estimated download).

## Quality / engineering

18. No LICENSE file though GPL-3.0 is declared.
19. No CONTRIBUTING.md; play-db scaffold missing.
20. CI doesn't exercise --detect-only against real hardware (containerized, no GPU).
21. PE fixtures are generated but invalid (goblin can't parse them due to zeroed IAT) — build_fixtures must match build_minimal_pe32 layout, or be dropped in favor of in-memory tests (already used).
22. Execution modules have mock-only tests — no end-to-end happy-path integration test.
23. final_error() / error display loses context; should propagate the original PlayError.

## Nice-to-have / roadmap

24. Occasional frame-drop investigation (profiling).
25. First-run shader-compile UX (progress hint).
26. Wayland "not responding" handling.
27. Persistent play-db integration (fetch currently disabled).
28. Metrics aggregation across sessions (--review-mangohud exists but no trend view).

## Suggested priority order

1. Fix silent bugs #2 (whitelist) and #1 (--yes).
2. Complete rollback manifest #3 and error propagation #4, #23.
3. Finish naming migration #11.
4. Make Clippy green #6.
5. Ship Steam integration #12.
6. Add LICENSE + CONTRIBUTING + play-db scaffold #18-20.
7. Detection enhancements #7-10.
8. Quality/UX items #13-17, #21-22.
9. Roadmap #24-28 as time permits.
