# Test Fixtures — PE Binaries

Minimal PE fixture binaries used by `binary::pe` analysis tests.

## Fixtures
- `d3d9_32bit.exe` — D3D9 import
- `d3d11_64bit.exe` — D3D11 import
- `d3d12_64bit.exe` — D3D12 import
- `vulkan_64bit.exe` — Vulkan import
- `eac_present.exe` — EasyAntiCheat strings
- `bink_video.exe` — Bink video strings
- `no_imports.exe` — no recognizable imports

## Generation
Run the fixture builder:
```bash
cd crates/play-core
cargo run --bin build_fixtures -- tests/fixtures/binaries
```
Or compile directly:
```bash
rustc tests/fixtures/binaries/build_fixtures.rs -o build_fixtures && ./build_fixtures tests/fixtures/binaries
```

These are minimal, legally distributable PE files containing just enough structure
and import table entries for the analyzer to detect DirectX version, anti-cheat,
engine hints, and video codec signals.
