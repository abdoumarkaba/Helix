# 02 — Detection gaps

Needed for the "zero-config" promise.

## 7. `nvidia_vbios_max_clock_mhz` always null
- Files: `crates/play-core/src/modules/detection.rs` (GPU enrichment)
- Fix: query `nvidia-smi --query-gpu=clocks.max.graphics --format=csv,noheader,nounits` (or VBIOS via `nvidia-smi -q -d CLOCK`); parse into `GpuProfile.nvidia_vbios_max_clock_mhz`.
- Verify: on NVIDIA hardware, `--detect-only` reports a non-null MHz value and the clock-lock tweak appears in the plan.

## 8. `vulkan_version` and `dx12_feature_level` always null
- Files: `crates/play-core/src/modules/detection.rs`
- Fix: parse `vulkaninfo --summary` for `apiVersion`; set `dx12_feature_level` from D3D12 feature-level probe or default "12_1" for NVIDIA.
- Verify: plan sets `VKD3D_FEATURE_LEVEL` when VKD3D-Proton is selected.

## 9. Laptop GPU detection hardcoded
- Files: `crates/play-core/src/modules/detection.rs`
- Fix: infer `is_laptop_gpu` from model name ("Laptop", "Mobile") or chassis type; cross-check `is_laptop_cpu`.
- Verify: on this machine (RTX 3050 Laptop) detection reports `is_laptop_gpu: true`.

## 10. Shallow tool detection
- Files: `crates/play-core/src/modules/detection.rs`
- Fix: `gamemode_installed` should check `gamemoded` daemon; `dxvk_installed` should check for `dxvk.conf`/Mesa DXVK libs; record versions.
- Verify: `--detect-only` `gaming_tools` reflects installed state accurately.
