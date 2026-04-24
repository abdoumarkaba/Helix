#![allow(clippy::pedantic)]

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use semver::Version;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level: GameEnvironment
// ---------------------------------------------------------------------------

/// The complete, versioned description of a game's runtime environment.
/// This is what gets serialized to state files and what agents pass between them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEnvironment {
    pub identity: GameIdentity,
    pub hardware: HardwareProfile,
    pub graphics: GraphicsConfig,
    pub runner: RunnerConfig,
    pub audio: AudioConfig,
    pub prefix: PrefixConfig,
    pub system: SystemTuning,
    pub launch: LaunchConfig,
    pub metadata: EnvironmentMetadata,
}

// ---------------------------------------------------------------------------
// GameIdentity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameIdentity {
    /// SHA256, primary key everywhere
    pub exe_hash: String,
    /// Full path to the game executable
    pub exe_path: PathBuf,
    pub exe_name: String,
    pub steam_app_id: Option<u32>,
    pub detected_name: Option<String>,
    /// From PE import table, static
    pub dx_version: DirectXVersion,
    /// x86 | x86_64
    pub pe_arch: PeArchitecture,
    pub anti_cheat: Vec<AntiCheat>,
    pub engine_hint: Option<GameEngine>,
    /// WMF requirement signal
    pub has_video_cutscenes: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectXVersion {
    D3D8,
    D3D9,
    D3D10,
    D3D11,
    D3D12,
    Vulkan,
    OpenGL,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeArchitecture {
    X86,
    #[serde(rename = "x86_64")]
    X86_64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AntiCheat {
    EasyAntiCheat { linux_supported: bool },
    BattlEye { linux_supported: bool },
    Denuvo,
    VMProtect,
    GameGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameEngine {
    UnrealEngine4,
    UnrealEngine5,
    Unity,
    Source,
    Source2,
    /// Capcom, very specific behavior
    REEngine,
    /// id Software
    IDAEngine,
    Unknown,
}

// ---------------------------------------------------------------------------
// HardwareProfile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub gpu: GpuProfile,
    pub cpu: CpuProfile,
    pub memory: MemoryProfile,
    pub kernel: KernelProfile,
    pub display: DisplayProfile,
    pub distro: DistroInfo,
    pub gaming_tools: GamingToolsProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProfile {
    pub vendor: GpuVendor,
    pub model: String,
    pub vram_mb: u32,
    pub driver_version: Version,
    pub vulkan_version: Option<Version>,
    /// Mesa | NvidiaProprietary | IntelANV
    pub driver_type: DriverType,
    pub features: GpuFeatureSet,
    /// Gates thermal-risky tweaks
    pub is_laptop_gpu: bool,
    /// NVIDIA VBIOS max clock in MHz (for clock lock formula)
    pub nvidia_vbios_max_clock_mhz: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuVendor {
    AMD,
    NVIDIA,
    Intel,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverType {
    Mesa,
    NvidiaProprietary,
    IntelANV,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuFeatureSet {
    pub vulkan_1_2: bool,
    pub vulkan_1_3: bool,
    pub ray_tracing: bool,
    pub mesh_shaders: bool,
    pub resizable_bar: bool,
    /// "12_0" | "12_1" | "12_2"
    pub dx12_feature_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuProfile {
    pub vendor: CpuVendor,
    pub model: String,
    pub arch: CpuArch,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub base_freq_mhz: u32,
    pub supports_avx2: bool,
    pub supports_avx512: bool,
    pub is_laptop_cpu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuVendor {
    Intel,
    AMD,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProfile {
    pub total_mb: u64,
    pub available_mb: u64,
    pub swap_total_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelProfile {
    pub version: KernelVersion,
    /// fsync support, kernel >= 5.16
    pub has_futex2: bool,
    /// proton fsync patchset
    pub has_fsync: bool,
    /// current value, needs >= 2_097_152
    pub vm_max_map_count: u64,
    /// always | madvise | never
    pub thp_mode: ThpMode,
    pub split_lock_mitigate: bool,
    pub sched_autogroup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThpMode {
    Always,
    Madvise,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub server: DisplayServer,
    pub primary_res: (u32, u32),
    pub refresh_hz: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayServer {
    X11,
    Wayland,
    Unknown,
}

// ---------------------------------------------------------------------------
// DistroInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistroInfo {
    pub distro: Distro,
    pub version_id: String,
    pub pretty_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Distro {
    Ubuntu,
    Fedora,
    Arch,
    Debian,
    OpenSUSE,
    Unknown,
}

// ---------------------------------------------------------------------------
// GamingToolsProfile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamingToolsProfile {
    pub wine_installed: bool,
    pub wine_version: Option<String>,
    pub winetricks_installed: bool,
    pub gamemode_installed: bool,
    pub dxvk_installed: bool,
    pub proton_available: bool,
    pub vulkan_available: bool,
}

// ---------------------------------------------------------------------------
// GraphicsConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicsConfig {
    pub translation_layer: TranslationLayer,
    pub dxvk_version: Option<Version>,
    pub vkd3d_version: Option<Version>,
    pub dxvk_config: DxvkConfig,
    pub mangohud: Option<MangoHudConfig>,
    pub gamemode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslationLayer {
    /// D3D9/10/11 -> Vulkan
    Dxvk,
    /// D3D12 -> Vulkan
    Vkd3dProton,
    /// OpenGL passthrough
    WineOpenGL,
    /// Vulkan or native Linux
    Native,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DxvkConfig {
    /// Disable if VAC/EAC present
    pub async_compile: bool,
    pub frame_limit: Option<u32>,
    pub hud: DxvkHud,
    /// Always true
    pub state_cache: bool,
    /// Per-game dxvk.conf
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DxvkHud {
    Off,
    Fps,
    Full,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangoHudConfig {
    pub enabled: bool,
    pub config_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// RunnerConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub runner_type: RunnerType,
    pub version: Version,
    pub install_path: PathBuf,
    /// SHA256 verified on install
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunnerType {
    ProtonOfficial,
    ProtonGE,
    WineGE,
    WineStaging,
    SodaWine,
}

// ---------------------------------------------------------------------------
// AudioConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    /// Detected from server
    pub server_rate: u32,
    /// Must match backend
    pub wine_driver: WineAudioDriver,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioBackend {
    PipeWire,
    PulseAudio,
    ALSA,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WineAudioDriver {
    Pulse,
    Alsa,
}

// ---------------------------------------------------------------------------
// PrefixConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixConfig {
    /// ~/.local/share/play/prefixes/{hash}
    pub path: PathBuf,
    /// Win32 | Win64
    pub arch: WineArch,
    /// Reported to game
    pub windows_version: WindowsVersion,
    pub dll_overrides: Vec<DllOverride>,
    /// Ordered for determinism
    pub env_vars: IndexMap<String, String>,
    /// For 32-bit games with >2GB assets
    pub large_address_aware: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WineArch {
    Win32,
    Win64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowsVersion {
    WinXP,
    Win7,
    Win8,
    Win10,
    Win11,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DllOverride {
    pub dll: String,
    /// e.g. "native,builtin"
    pub mode: String,
}

// ---------------------------------------------------------------------------
// SystemTuning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTuning {
    pub vm_max_map_count: Option<u64>,
    pub thp_mode: Option<ThpMode>,
    pub sched_autogroup: Option<bool>,
    pub split_lock_mitigate: Option<bool>,
    pub ulimit_nofile: Option<u64>,
    /// Session-scoped
    pub cpu_governor: Option<CpuGovernor>,
    /// Session-scoped
    pub gpu_perf_mode: Option<GpuPerfMode>,
    pub esync: bool,
    pub fsync: bool,
    pub gamemode: bool,
    /// NVIDIA clock lock value in MHz (95% of VBIOS max, rounded to 15 MHz step).
    /// Stored here for audit trail; applied via play-helper.
    pub nvidia_clock_lock_mhz: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuGovernor {
    Performance,
    Schedutil,
    Powersave,
    Ondemand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuPerfMode {
    Auto,
    High,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuArch {
    X86,
    X86_64,
}

// ---------------------------------------------------------------------------
// LaunchConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchConfig {
    pub exe_path: PathBuf,
    pub working_dir: PathBuf,
    pub args: Vec<String>,
    pub env: IndexMap<String, String>,
    pub pre_launch: Vec<HookCommand>,
    pub post_exit: Vec<HookCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Fully resolved command to launch a game.
/// Never contains shell strings — only explicit arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: IndexMap<String, String>,
    pub working_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// EnvironmentMetadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentMetadata {
    /// Bump on breaking changes, never reset
    pub schema_version: u32,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub tool_version: Version,
    pub resolution_source: ResolutionSource,
    /// Audit trail
    pub decisions: Vec<ResolutionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolutionSource {
    FullyAutomatic,
    DatabaseAssisted { entry_id: String, confidence: f32 },
    UserOverridden { fields: Vec<String> },
    Hybrid { db_fields: Vec<String>, heuristic_fields: Vec<String> },
}

/// Every decision the Planning Agent makes is recorded here.
/// This is what users see in verbose mode and what goes into bug reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionDecision {
    /// e.g. "runner.runner_type"
    pub field: String,
    /// e.g. "ProtonGE"
    pub chosen: String,
    /// e.g. "Game uses WMF video; GE includes media patches"
    pub reason: String,
    pub confidence: Confidence,
    pub source: DecisionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Inferred,
}

// ---------------------------------------------------------------------------
// Steam Installation
// ---------------------------------------------------------------------------

/// Detected Steam installation information.
#[derive(Debug, Clone)]
pub struct SteamInstallation {
    /// Path to Steam installation directory
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Batched System Tweak Commands
// ---------------------------------------------------------------------------

/// Batched system tweak command for play-helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum BatchedTweakCommand {
    /// Write sysctl value: (key, value)
    #[serde(rename = "sysctl_write")]
    SysctlWrite(String, String),
    /// Write sysfs value: (path, value)
    #[serde(rename = "sysfs_write")]
    SysfsWrite(String, String),
    /// Write ulimit: value
    #[serde(rename = "ulimit_nofile")]
    UlimitNofile(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionSource {
    Database,
    Heuristic,
    HardwareDetection,
    UserOverride,
}
