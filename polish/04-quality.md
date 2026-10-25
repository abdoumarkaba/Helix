# 04 — Quality / engineering

## 18. LICENSE file
- Add `LICENSE` (GPL-3.0 full text) at repo root.
- Verify: `cargo build` metadata + README link resolve.

## 19. CONTRIBUTING.md + play-db scaffold
- Add `CONTRIBUTING.md` (dev setup, commit style, PR checklist).
- Scaffold `play-db/` (runners.toml, entries/, validate.py already present — document).
- Verify: a new contributor can build + test from docs alone.

## 20. CI real-hardware coverage
- Files: `.github/workflows/ci.yml`
- Fix: add a self-hosted or manual workflow that runs `--detect-only` on a GPU runner; keep container job for compile/test.
- Verify: workflow green on both paths.

## 21. PE fixtures validity
- Files: `crates/play-core/tests/fixtures/binaries/build_fixtures.rs`
- Fix: align generated PE layout with `build_minimal_pe32` (non-zero IAT/ILT) so goblin parses; or delete generated `.exe` and rely on in-memory builders (already used by tests).
- Verify: `analyze_binary` succeeds on every fixture file.

## 22. End-to-end happy-path test
- Files: `crates/play-core/tests/` (new integration test)
- Fix: mock runner/prefix/system to drive orchestrator from detection through validation; assert checkpoint phase `Validated`.
- Verify: `cargo test --workspace` includes the new test.

## 23. Error propagation (see 01 #4)
- Ensure `PlayError` flows unchanged from module to CLI; remove `final_error()` generic replacement.
