# Helix GPU Testing Matrix — Execution Plan

*Validating Helix (play) across NVIDIA hardware/driver/distro combinations with Docker Desktop (free, local) + \~$50 cloud render credit. Written against current repo state: DXVK cache regression, hybrid PRIME-offload bug, AMD stubbed out.*

## 1. Split the matrix — don't spend GPU-hours on logic you can test for free

Two different test categories are getting conflated in "software and hardware combinations":

**Logic matrix** (distro, kernel version, package manager, DX-version routing, anti-cheat blocking, tweak constraint evaluation) — pure decision-engine logic. The architecture already mandates injectable `proc_root`/`sys_root` for exactly this reason. Test with fixture directories: zero GPU, zero cloud spend, runs in `cargo test` or the existing `ci.yml`/`cross-distro.yml`. If this isn't already a comprehensive fixture suite — kernel 5.15 vs 5.16 boundary, vm.max\_map\_count tiers, all 4 package managers, every `GameEngine` hint including GameMaker/RPGMaker's 32-bit + vcruntime special cases — build that out first. Highest-leverage, zero-cost testing not yet being done.

**Execution matrix** (does the game actually launch, render, and produce frames on real silicon with a real driver) — needs real GPUs. This is what the $50 is for.

Collapsing these wastes render credit re-proving things fixtures already cover for free.

## 2. What the cloud fleet physically cannot test

- **Kernel version variance** — containers share the host kernel. No GPU cloud provider gives you a <5.16 kernel box to exercise the esync fallback path for real; that stays a fixture test.
- **Hybrid PRIME-offload bug** — rented GPU boxes are single discrete-GPU; there's no iGPU to silently steal the render target. This bug only reproduces on the Intel+RTX 3050 Wayland box. Test it there, not in the cloud.
- **Distro diversity beyond package-manager logic** — a container gives userspace (apt vs dnf vs pacman), not a different kernel or init system. Fine for `PackageAgent` testing, not meaningful for anything kernel-gated.

Don't burn credits trying to fake these.

## 3. Phase 1 — Build one image, validate it free on Docker Desktop first

Bake a single image containing: Helix release binary + `play-helper`, Proton-GE + Wine-GE runners pre-downloaded and SHA256-verified (don't let pods re-download per spin-up — pure wasted rental time), DXVK's own test suite + VKD3D-Proton's own test suite (both open source, built specifically to exercise the exact D3D9/11/12→Vulkan paths Helix orchestrates — use these instead of baking real game binaries into a pushed image), Xvfb + minimal WM, MangoHud, and the harness script.

Validate structurally on Docker Desktop with no GPU attached: does it build, does the binary avoid panicking against the container's (host-shared) `/proc`, does `play-helper` correctly reject anything off its sysfs whitelist. Catches most integration bugs before any pod spin-up costs money. Then do one real pass with `--gpus all` / `--device /dev/dri` against the RTX 3050 locally — a second free data point and the only place the PRIME-offload bug is reachable.

Keep real game binaries out of anything pushed to a registry — licensing risk. DXVK/VKD3D test binaries are clean; mount any owned game at runtime via volume, never bake it in.

## 4. Phase 2 — Provider and real cost math

NVIDIA-only simplifies provider choice. Current marketplace rates (Vast.ai fluctuates):

| GPU gen | Provider | Rate |
| --- | --- | --- |
| RTX 3060 (Ampere, matches own tier) | Vast.ai interruptible | \~$0.03–0.10/hr |
| RTX 3090 (Ampere) | Vast.ai interruptible/on-demand | \~$0.09–0.30/hr |
| RTX 3090 | RunPod Community | \~$0.19–0.22/hr |
| RTX 4090 (Ada) | Vast.ai | \~$0.14–0.40/hr |
| RTX 4090 | RunPod Community | \~$0.34/hr |
| RTX A2000 (cheapest for pure launch/smoke checks) | RunPod Community | \~$0.12/hr |

Default: **Vast.ai interruptible**. Correctness smoke-testing is idempotent and short by design — Helix's own idempotency principle means a reclaimed pod costs a re-run, not a corrupted result — so interruptible pricing is a fit, not a risk. RunPod Community as fallback if Vast.ai availability is flaky that day; its custom-image UX is cleaner.

Real math: a 5-minute launch-test run on a $0.15/hr box costs \~$0.0125. A 300-run matrix (3 GPU gens × 2 driver branches × \~50 game/test-binary combos) at 5 min each is \~25 pod-hours ≈ **$3–5**, even at the pricier end. The $50 is not the constraint — pod cold-start/image-pull time is, which is exactly why the pre-baked image in Phase 1 matters more than the dollar figure.

## 5. Phase 3 — Matrix axes

1. **GPU generation**: Turing (GTX 1660/RTX 2060) · Ampere (RTX 3060/3090) · Ada (RTX 4090) — covers the own tier plus the realistic Linux-gamer install base, exercises `dx12_feature_level`/mesh-shader detection differences.
2. **Driver branch**: oldest-supported vs latest, per GPU gen.
3. **DX path**: D3D9 / D3D11 / D3D12 / native Vulkan — via DXVK/VKD3D-Proton's own test binaries, not full games.
4. **Engine diversity**: one light title per `GameEngine` hint actually detected — prioritize GameMaker and RPGMaker specifically, since those are flagged in the data model as needing the 32-bit + vcruntime-override special case; also one title with Bink/RAD video to exercise the WMF/ProtonGE runner-selection path end to end.
5. **Anti-cheat detection**: stays in the logic matrix (fixtures) — testing that the binary analyzer flags and blocks, not that the anti-cheat itself works.

## 6. Phase 4 — Harness and orchestration

Script against each provider's CLI/API (`vastai` CLI, RunPod's `runpod` Python SDK): spin pod with pinned image + GPU filter → pull (near-instant, pre-baked) → run harness (launch, hold N seconds, parse MangoHud log for FPS>0, check exit code/crash, run `play --undo`, diff `/proc/sys` values restored against rollback manifest) → write result → destroy pod immediately (per-second billing on both platforms — don't let idle pods eat budget).

Write results in the existing `report.schema.json` / `reports/{year}/{month}/{id}.toml` format instead of inventing a new one — this runs straight through the existing `promote_report.rs` pipeline (even with its currently tracked bugs) and produces real validated `play-db` entries as a byproduct of testing, not just pass/fail logs.

If this becomes a recurring CI thing rather than a one-off sweep, the upgrade path is registering a pod as a self-hosted GitHub Actions runner against the existing `ci.yml`/`cross-distro.yml`. Skip that for this first pass — a bespoke script is faster to stand up for a bounded $50 run.

## 7. Phase 5 — Runs to prioritize first (tied to known bugs, not generic coverage)

- **DXVK cache regression**: launch the same test binary/game twice in the same pod. Assert the 2nd launch's shader-compile stall is near-zero and `DXVK_STATE_CACHE_PATH` actually persists a non-empty cache file at `{prefix}/cache` between runs. This is the concrete regression test for the bug already tracked — should fail on current `main`, pass once `DXVK_STATE_CACHE_PATH` and the `STEAM_COMPAT_DATA_PATH`/`WINEPREFIX` confusion are fixed.
- **Prefix lifecycle**: hash or mtime the prefix directory across two consecutive launches of the same game — it shouldn't be re-initializing.
- **Rollback correctness**: cheap to check in every single pod run (apply tweaks → confirm → `--undo` → diff restored `/proc/sys` values against `rollback.toml`). Make this part of every harness pass, not a separate phase.

## 8. Execution checklist

1. Build + structurally validate image on Docker Desktop (no GPU) — free.
2. Validate once on the RTX 3050 box with real GPU passthrough — free, also the only PRIME-offload test.
3. Push image to GHCR/Docker Hub (no game binaries baked in).
4. Stand up the Vast.ai/RunPod driver script, dry-run against one cheap instance (RTX A2000 or similar) to confirm the full spin-up → harness → teardown loop before running the real matrix.
5. Run the logic-matrix fixture suite expansion in parallel — free, should happen regardless of cloud testing.
6. Run the prioritized Phase 5 regression checks first.
7. Run the full GPU-gen × driver × DX-path × engine matrix.
8. Feed passing results through `promote_report.rs` into `play-db`.
