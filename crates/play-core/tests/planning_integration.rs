#![allow(clippy::pedantic)]
/// Integration tests for PlanningModule.
/// Uses fixture files from tests/fixtures/db/ — no real system access.
use std::path::PathBuf;

use play_core::models::environment::*;
use play_core::models::plan::*;
use play_core::modules::database_client::{FixtureReader, NoopDatabaseReader};
use play_core::modules::planning::PlanningModule;

fn fixture_db() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/db")
}

fn planning_module() -> PlanningModule {
    PlanningModule::new(
        fixture_db(),
        PathBuf::from("/tmp/play-test-runners"),
        PathBuf::from("/tmp/play-test-prefixes"),
    )
}

fn base_env(dx: DirectXVersion, pe_arch: PeArchitecture) -> GameEnvironment {
    GameEnvironment {
        identity: GameIdentity {
            exe_hash: "0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
            exe_name: "test_game.exe".to_owned(),
            steam_app_id: None,
            detected_name: None,
            dx_version: dx,
            pe_arch,
            anti_cheat: vec![],
            engine_hint: None,
            has_video_cutscenes: None,
        },
        hardware: HardwareProfile {
            gpu: GpuProfile {
                vendor: GpuVendor::NVIDIA,
                model: "RTX 3050 Laptop".to_owned(),
                vram_mb: 6144,
                driver_version: semver::Version::new(550, 0, 0),
                vulkan_version: Some(semver::Version::new(1, 3, 0)),
                driver_type: DriverType::NvidiaProprietary,
                features: GpuFeatureSet {
                    vulkan_1_2: true,
                    vulkan_1_3: true,
                    ray_tracing: false,
                    mesh_shaders: false,
                    resizable_bar: false,
                    dx12_feature_level: None,
                },
                is_laptop_gpu: true,
                nvidia_vbios_max_clock_mhz: Some(1777),
            },
            cpu: CpuProfile {
                vendor: CpuVendor::Intel,
                model: "i7-13650HX".to_owned(),
                arch: CpuArch::X86_64,
                physical_cores: 6,
                logical_cores: 12,
                base_freq_mhz: 2600,
                supports_avx2: true,
                supports_avx512: false,
                is_laptop_cpu: true,
            },
            memory: MemoryProfile { total_mb: 15657, available_mb: 10000, swap_total_mb: 8192 },
            kernel: KernelProfile {
                version: KernelVersion { major: 6, minor: 18, patch: 13 },
                has_futex2: true,
                has_fsync: true,
                vm_max_map_count: 65536,
                thp_mode: ThpMode::Madvise,
                split_lock_mitigate: true,
                sched_autogroup: false,
            },
            display: DisplayProfile {
                server: DisplayServer::Wayland,
                primary_res: (1920, 1080),
                refresh_hz: 60.0,
            },
            distro: DistroInfo {
                distro: Distro::Fedora,
                version_id: "43".to_owned(),
                pretty_name: "Fedora Linux 43".to_owned(),
            },
        },
        graphics: GraphicsConfig {
            translation_layer: TranslationLayer::Dxvk,
            dxvk_version: None,
            vkd3d_version: None,
            dxvk_config: DxvkConfig {
                async_compile: false,
                frame_limit: None,
                hud: DxvkHud::Off,
                state_cache: true,
                config_path: PathBuf::from("/tmp/dxvk.conf"),
            },
            mangohud: None,
            gamemode: false,
        },
        runner: RunnerConfig {
            runner_type: RunnerType::ProtonGE,
            version: semver::Version::new(0, 0, 0),
            install_path: PathBuf::from("/tmp"),
            verified: false,
        },
        audio: AudioConfig {
            backend: AudioBackend::PipeWire,
            server_rate: 48000,
            wine_driver: WineAudioDriver::Pulse,
            latency_ms: None,
        },
        prefix: PrefixConfig {
            path: PathBuf::from("/tmp/prefix"),
            arch: WineArch::Win64,
            windows_version: WindowsVersion::Win10,
            dll_overrides: vec![],
            env_vars: indexmap::IndexMap::new(),
            large_address_aware: false,
        },
        system: SystemTuning {
            vm_max_map_count: None,
            thp_mode: None,
            sched_autogroup: None,
            split_lock_mitigate: None,
            ulimit_nofile: None,
            cpu_governor: None,
            gpu_perf_mode: None,
            esync: false,
            fsync: false,
            gamemode: false,
            nvidia_clock_lock_mhz: None,
        },
        launch: LaunchConfig {
            exe_path: PathBuf::from("/tmp/test_game.exe"),
            working_dir: PathBuf::from("/tmp"),
            args: vec![],
            env: indexmap::IndexMap::new(),
            pre_launch: vec![],
            post_exit: vec![],
        },
        metadata: EnvironmentMetadata {
            schema_version: 1,
            created_at: chrono::Utc::now(),
            last_run: None,
            tool_version: semver::Version::new(0, 1, 0),
            resolution_source: ResolutionSource::FullyAutomatic,
            decisions: vec![],
        },
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[test]
fn full_plan_d3d9_pipewire_nvidia_laptop() {
    let env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    let module = planning_module();
    let db = FixtureReader::new(fixture_db());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert!(plan.hard_blocks.is_empty(), "unexpected hard blocks: {:?}", plan.hard_blocks);
    assert_eq!(plan.env.graphics.translation_layer, TranslationLayer::Dxvk);
    assert_eq!(plan.env.runner.runner_type, RunnerType::ProtonGE);
    assert!(plan.env.system.fsync, "fsync should be set on futex2 kernel");
    assert!(!plan.env.system.esync, "esync should not be set when fsync is active");
    assert!(!plan.env.metadata.decisions.is_empty(), "decisions must be recorded");
}

#[test]
fn full_plan_d3d12_selects_vkd3d() {
    let env = base_env(DirectXVersion::D3D12, PeArchitecture::X86_64);
    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert_eq!(plan.env.graphics.translation_layer, TranslationLayer::Vkd3dProton);
    assert!(!plan.db_hit);
}

#[test]
fn eac_hard_block_short_circuits() {
    let mut env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    env.identity.anti_cheat = vec![AntiCheat::EasyAntiCheat { linux_supported: false }];

    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert!(!plan.hard_blocks.is_empty(), "EAC hard block must be recorded");
    assert!(plan.hard_blocks[0].contains("EasyAntiCheat"));
}

#[test]
fn plan_is_deterministic() {
    let env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());

    let plan1 = module.plan_with_reader(&env, &db).unwrap();
    let plan2 = module.plan_with_reader(&env, &db).unwrap();

    // Compare key fields — GamePlan doesn't derive PartialEq (contains PathBuf etc.)
    assert_eq!(plan1.env.graphics.translation_layer, plan2.env.graphics.translation_layer);
    assert_eq!(plan1.env.runner.runner_type, plan2.env.runner.runner_type);
    assert_eq!(plan1.env.runner.version, plan2.env.runner.version);
    assert_eq!(plan1.env.system.fsync, plan2.env.system.fsync);
    assert_eq!(plan1.tweaks.len(), plan2.tweaks.len());
}

#[test]
fn laptop_gpu_has_no_class_c_tweaks_applied() {
    // Dev machine is a laptop — Class C tweaks must be NotApplicable.
    let env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    assert!(env.hardware.gpu.is_laptop_gpu);

    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    for tweak in &plan.tweaks {
        if tweak.class == TweakClass::C {
            assert!(
                matches!(tweak.decision, TweakDecision::NotApplicable { .. }),
                "Class C tweak {:?} should be NotApplicable on laptop GPU",
                tweak.id
            );
        }
    }
}

#[test]
fn db_entry_dll_overrides_merged() {
    let mut env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    env.identity.exe_hash =
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_owned();

    let module = planning_module();
    let db = FixtureReader::new(fixture_db());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert!(plan.db_hit, "DB hit expected for fixture hash");
    assert!(
        plan.env.prefix.dll_overrides.iter().any(|o| o.dll == "d3d9"),
        "fixture DLL override (d3d9) should be in plan"
    );
}

#[test]
fn decisions_are_non_empty() {
    let env = base_env(DirectXVersion::D3D11, PeArchitecture::X86_64);
    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert!(
        plan.env.metadata.decisions.len() >= 6,
        "expected at least 6 recorded decisions, got {}",
        plan.env.metadata.decisions.len()
    );
}

#[test]
fn mangohud_warning_present_when_not_configured() {
    let env = base_env(DirectXVersion::D3D9, PeArchitecture::X86_64);
    // env.graphics.mangohud is None by default.
    let module = planning_module();
    let db = NoopDatabaseReader::with_manifest(load_fixture_manifest());
    let plan = module.plan_with_reader(&env, &db).unwrap();

    assert!(
        plan.warnings.iter().any(|w| w.message.contains("MangoHud")),
        "MangoHud warning expected"
    );
    assert!(
        plan.warnings.iter().any(|w| w.install_hint.is_some()),
        "MangoHud warning should include install hint"
    );
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn load_fixture_manifest() -> play_core::models::plan::RunnersManifest {
    let path = fixture_db().join("runners.toml");
    let raw = std::fs::read_to_string(path).unwrap();
    toml::from_str(&raw).unwrap()
}
