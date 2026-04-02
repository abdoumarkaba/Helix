# /detect — Implement Detection Logic

Implement detection for: $ARGUMENTS

Detection is the foundation. Everything downstream (planning, execution) depends on
getting this right. Detection must be read-only, never require root, and produce
fully-typed results. Uncertainty is allowed; incorrect detection is not.

## Detection Principles

Every detector must satisfy:
1. **Read-only** — no writes, no subprocess side effects
2. **Unprivileged** — read /proc /sys directly as the current user
3. **Injectable** — accepts `sys_root: &Path` so tests can use tempdir fixtures
4. **Typed output** — returns a typed enum or struct, never a String that callers must re-parse
5. **Uncertainty expressed** — use `Option<T>` when detection can legitimately fail
6. **Fails loudly** — detection failure for a required field is `PlayError::HardwareDetection { component }`

## Step 1: Define the Output Type

What typed Rust value does this detection produce? Start there, not with the implementation.
Make sure the type is an enum (not a String) when there are a known set of valid values.

## Step 2: Identify the Source

Where does this information live? Priority order:
- `/proc/cpuinfo`, `/proc/meminfo` — CPU, memory
- `/sys/class/drm/*/` — GPU, display
- `nvidia-smi --query-gpu=... --format=csv,noheader` — NVIDIA-specific
- `vulkaninfo`, `glxinfo` — Vulkan/OpenGL capability
- `pactl info`, `pw-dump` — audio server
- PE binary import table — DirectX version, anti-cheat, engine

## Step 3: Parse, Don't Grep

Parse structured output into typed values. Never do `output.contains("nvidia")` and
return a string. Parse once, store typed, match on enum everywhere downstream.

```rust
// WRONG: string comparison downstream
let vendor = if nvidia_smi_output.contains("NVIDIA") { "nvidia".to_string() } else { "unknown".to_string() };

// RIGHT: parse to typed enum once
let vendor: GpuVendor = parse_gpu_vendor(&nvidia_smi_output)?;
// All downstream code: match vendor { GpuVendor::NVIDIA => ..., GpuVendor::AMD => ... }
```

## Step 4: Testability Setup

Create a minimal sysfs/procfs fixture in a tempdir that exercises this detection.
The test must pass with NO real system access — it reads only from the tempdir.

```rust
#[test]
fn test_detect_nvidia_gpu() {
    let tmp = tempdir().unwrap();
    // Create fake /sys/class/drm/card0/device/vendor with content "0x10de"
    // Create fake /sys/class/drm/card0/device/uevent with DRIVER=nvidia
    let result = detect_gpu(&tmp.path()).unwrap();
    assert_eq!(result.vendor, GpuVendor::NVIDIA);
}
```

## Step 5: Graceful Degradation

Not all detections are required. Decide:
- Required (failure = PlayError): GPU vendor, DX version, kernel version
- Optional (failure = None): VBIOS max clock, Vulkan version, game engine hint

For optional fields, return `Option<T>`, never error. Document why the field is optional
with a comment.

## Step 6: Integration with HardwareProfile / GameIdentity

Show exactly how the detected value maps into `GameEnvironment`. Which field?
Which struct? If a new field is needed, add it to the appropriate type and update
all match arms that need to be exhaustive.

Generate the complete implementation with tests.
