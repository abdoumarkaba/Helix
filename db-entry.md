# /db-entry — Generate play-db Entry

Generate a play-db entry for: $ARGUMENTS

play-db entries are pure data — no executable content, ever. They describe
what configuration produces a good result for a specific game on specific hardware.
All fields are validated by `tools/validate.py` in CI.

## Step 1: Required Information

To generate an entry, I need:
- `exe_hash` (SHA256 of the game executable)
- `exe_name` (filename of the .exe)
- `steam_app_id` (if on Steam — check SteamDB)
- `canonical_name` (the game's full name)
- What DirectX version does the game use? (D3D9/D3D11/D3D12/Vulkan)
- Any known anti-cheat? (EasyAntiCheat, BattlEye, Denuvo, etc.)
- Minimum GE-Proton version known to work?
- Any specific environment variables required?
- Any DLL overrides needed in the Wine prefix?
- Tested result: Broken / Degraded / Playable / Optimal?

If this information is not provided in $ARGUMENTS, ask for it before generating.

## Step 2: Generate the TOML Entry

Follow the canonical schema exactly. The entry goes at:
`entries/{exe_hash[0:2]}/{exe_hash}/default.toml`

```toml
[entry]
id             = "{generate-a-uuid-v4}"
schema_version = 1

[entry.identity]
exe_hash       = "sha256:{hash}"
exe_name       = "{name}.exe"
steam_app_id   = {app_id}
canonical_name = "{Full Game Name}"

[entry.conditions]
# All conditions must match for entry to apply. Missing fields match ANY value.
dx_version         = "{D3D9|D3D11|D3D12|Vulkan}"
gpu_vendor         = ["NVIDIA"]   # v1 entries are NVIDIA-only; AMD is Phase 2
gpu_vulkan_min     = "1.3"
kernel_min         = "6.0"
runner_type        = "ProtonGE"
runner_version_min = "{minimum_version}"
anti_cheat         = []

[entry.outcome]
result       = "{Broken|Degraded|Playable|Optimal}"
# confidence is COMPUTED by tools/compute_confidence.py — never manually set
confidence   = 0.0
degradations = []
blockers     = []

[entry.configuration]

[entry.configuration.runner]
type        = "ProtonGE"
version_min = "{minimum_version}"

[entry.configuration.graphics]
translation_layer = "{Dxvk|Vkd3dProton|WineOpenGL|Native}"
dxvk_async        = {true|false}

[entry.configuration.system]
vm_max_map_count = {hardware_proportional_value}
fsync            = true

[entry.configuration.prefix]
windows_version = "{Win7|Win10}"
dll_overrides   = []

[entry.configuration.env_vars]
# Only include env vars that are non-default and required for this game

[entry.metadata]
created           = "{today's date YYYY-MM-DD}"
last_validated    = "{today's date YYYY-MM-DD}"
contributor_count = 1
report_ids        = []
maintainer_notes  = ""
```

## Step 3: Validation Check

After generating the entry, verify:
- `confidence` is 0.0 (will be computed by CI — never manually assign a real value)
- No executable content in any field (especially env_vars and dll_overrides)
- `runner_version_min` is a real version from `runners.toml`, not a placeholder
- `vm.max_map_count` uses the hardware-proportional value for the hardware class
  that the game was tested on, not a hardcoded 2097152
- All conditions are typed correctly (dx_version matches DirectXVersion enum)

## Step 4: Companion nvidia.toml (if needed)

If the game has NVIDIA-specific configuration different from the default:
generate `entries/{hash[0:2]}/{hash}/nvidia.toml` with only the overriding fields.
