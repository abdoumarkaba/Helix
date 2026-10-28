#![allow(
    clippy::pedantic,
    clippy::too_many_lines,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::unnecessary_sort_by,
    clippy::bool_comparison,
    clippy::single_match,
    clippy::match_result_ok,
    clippy::used_underscore_binding,
    clippy::uninlined_format_args
)]
//! CLI entry point for the `play` Linux gaming orchestrator.
//!
//! Handles argument parsing, tracing setup, plan display, user confirmation,
//! and post-session metrics/reporting workflows.

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use console::{style, Term};
use dirs::data_dir;

use serde_json::Value as JsonValue;
use tracing::{error, info, warn};
use tracing_subscriber::prelude::*;

use helix_core::execution::mangohud::MangoHudParser;
use helix_core::models::environment::GameEnvironment;
use helix_core::models::errors::PlayError;
use helix_core::models::metrics::SessionMetrics;
use helix_core::modules::detection::RealCommandRunner;
use helix_core::modules::validation::FpsMetrics;
use helix_core::orchestrator::{Orchestrator, OrchestratorPhase};

mod ui;

// ---------------------------------------------------------------------------
// CLI Arguments
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "helix")]
#[command(about = "Linux gaming orchestrator — zero-config Windows game launcher")]
#[command(version)]
struct CliArgs {
    /// Path to game executable
    exe_path: Option<PathBuf>,

    /// Additional arguments forwarded to the game
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    game_args: Vec<String>,

    /// Auto-confirm without interactive prompt
    #[arg(long)]
    yes: bool,

    /// Rollback previous session for this game
    #[arg(long)]
    undo: bool,

    /// Enable verbose tracing output
    #[arg(long, short)]
    verbose: bool,

    /// Plan only, do not execute
    #[arg(long)]
    dry_run: bool,

    /// Stop the tracked game process for this executable
    #[arg(long)]
    stop: bool,

    /// File a crash report for last session
    #[arg(long)]
    report: bool,

    /// Show about information
    #[arg(long)]
    about: bool,

    /// Show log file location
    #[arg(long)]
    log_path: bool,

    /// Ignore checkpoint and start fresh session
    #[arg(long)]
    force_fresh: bool,

    /// Generate shell completions (bash, zsh, fish)
    #[arg(long, value_name = "SHELL")]
    generate_completion: Option<String>,

    /// Review MangoHud logs from previous game runs
    #[arg(long)]
    review_mangohud: bool,

    /// Show cross-session FPS trend view (use with --review-mangohud)
    #[arg(long)]
    trend: bool,

    /// Run hardware detection only and exit (no planning or execution)
    #[arg(long)]
    detect_only: bool,

    /// Enable Wayland debug logging for the spawned game
    #[arg(long)]
    wayland_debug: bool,
}

// ---------------------------------------------------------------------------
// Tracing Setup
// ---------------------------------------------------------------------------

fn helix_data_dir() -> PathBuf {
    data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("helix")
}

#[allow(dead_code)]
fn legacy_data_dir() -> PathBuf {
    data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play")
}

fn get_log_dir() -> PathBuf {
    helix_data_dir().join("logs")
}

fn setup_tracing(verbose: bool, exe_path: Option<&Path>) -> PathBuf {
    let log_dir = get_log_dir();

    // Create log directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("Warning: Failed to create log directory: {}", e);
    }

    // Generate per-run log filename: play-{timestamp}-{game_name}.log
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let game_name =
        exe_path.and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("unknown");
    let log_filename = format!("helix-{}-{}.log", timestamp, game_name);
    let log_path = log_dir.join(&log_filename);

    // Create single file appender (non-rotating) for this run
    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::NEVER,
        &log_dir,
        format!("helix-{}-{}", timestamp, game_name),
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Keep the guard alive for the duration of the program
    Box::leak(Box::new(_guard));

    let file_filter = tracing_subscriber::EnvFilter::new("helix_core=debug,helix_cli=debug,info");
    let console_filter = if verbose {
        tracing_subscriber::EnvFilter::new("helix_core=debug,helix_cli=debug,info")
    } else {
        tracing_subscriber::EnvFilter::new("warn")
    };

    // Set up console output in addition to file
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(false)
        .with_filter(console_filter);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false)
        .with_filter(file_filter);

    tracing_subscriber::registry().with(file_layer).with(console_layer).init();

    // Print the log file path at startup so user knows where to find it
    eprintln!("{} {}", style("Log file:").dim(), log_path.display());

    log_path
}

// ---------------------------------------------------------------------------
// Error Display
// ---------------------------------------------------------------------------

/// Display error with full context including rollback information and next steps.
fn display_error_with_context(
    error: &PlayError,
    exe_path: &Path,
    log_path: &Path,
    rollback_entries: &[helix_core::orchestrator::RollbackEntry],
) {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{}", error));

    // Extract error-specific context
    let error_context = match error {
        PlayError::RunnerDownload { url, attempt, reason } => Some(vec![
            ui::kv("URL", url),
            ui::kv("Attempt", format!("{}/3", attempt)),
            ui::kv("Reason", reason),
        ]),
        PlayError::PackageInstall { pm, package, .. } => {
            Some(vec![ui::kv("Package manager", pm), ui::kv("Package", package)])
        },
        PlayError::GameCrash { seconds, .. } => {
            Some(vec![ui::kv("Crashed after", format!("{} seconds", seconds))])
        },
        PlayError::GameLaunchFailed { exe_path, reason } => {
            Some(vec![ui::kv("Executable", exe_path.display()), ui::kv("Reason", reason)])
        },
        _ => None,
    };

    if let Some(ctx) = error_context {
        for c in ctx {
            lines.push(c);
        }
    }

    // Show rollback information if any
    if !rollback_entries.is_empty() {
        lines.push(ui::status_warn("Rollback completed:"));
        for entry in rollback_entries {
            lines.push(format!("    · Restored {}: {}", entry.key, entry.previous_value));
        }
    }

    // Provide next steps based on error type
    lines.push(ui::section("Next steps"));
    let steps: Vec<String> = match error {
        PlayError::RunnerDownload { .. } => vec![
            "Check your internet connection".to_string(),
            "Verify the runner URL is accessible".to_string(),
            format!("Check the log file for details: {}", log_path.display()),
            format!("Try again: play {}", exe_path.display()),
            format!("Or rollback: play --undo {}", exe_path.display()),
        ],
        PlayError::PackageInstall { .. } => vec![
            "Check if package manager is working".to_string(),
            "Try installing manually with your package manager".to_string(),
            format!("Check the log file for details: {}", log_path.display()),
            format!("Try again: play {}", exe_path.display()),
            format!("Or rollback: play --undo {}", exe_path.display()),
        ],
        PlayError::GameCrash { .. } => vec![
            format!("Check the log file for details: {}", log_path.display()),
            format!("Run play --report {} to file a crash report", exe_path.display()),
            "Try running with different compatibility settings".to_string(),
        ],
        PlayError::ValidationFailed { .. } => vec![
            "Check if game process is still running".to_string(),
            "Verify GPU drivers are installed and working".to_string(),
            format!("Check the log file for details: {}", log_path.display()),
            format!("Run play --report {} to file a report", exe_path.display()),
        ],
        PlayError::NoRunnerAvailable { .. } => vec![
            "Check that ProtonGE runners are installed".to_string(),
            "Install required tools (pciutils, vulkan-tools)".to_string(),
            format!("Check the log file for details: {}", log_path.display()),
            "Try a different game or check runner compatibility".to_string(),
        ],
        PlayError::UnsupportedDistro { .. } => vec![
            "Check if your distro is supported".to_string(),
            "Try running on a supported distro (Ubuntu 22.04+, Fedora 38+, Arch, Debian 12+)"
                .to_string(),
            format!("Check the log file for details: {}", log_path.display()),
        ],
        PlayError::InsufficientVram { .. } => vec![
            "Close other applications to free VRAM".to_string(),
            "Lower game graphics settings".to_string(),
            "Consider upgrading your GPU".to_string(),
            format!("Check the log file for details: {}", log_path.display()),
        ],
        _ => vec![
            format!("Check the log file for details: {}", log_path.display()),
            format!("Try again: play {}", exe_path.display()),
            format!("Or rollback: play --undo {}", exe_path.display()),
        ],
    };

    for (i, s) in steps.iter().enumerate() {
        lines.push(format!("    {}. {}", i + 1, s));
    }

    eprintln!();
    eprintln!("{}", ui::card("ERROR", &lines));
}

/// Show log file location at end of run
#[allow(dead_code)]
fn show_log_path(_log_path: &Path) {
    // Log path is now shown at startup, but keep this for potential future use
}

// ---------------------------------------------------------------------------
// Review MangoHud Logs
// ---------------------------------------------------------------------------

fn review_mangohud_logs(show_trend: bool) -> Result<(), PlayError> {
    let log_dir = helix_data_dir().join("logs");

    if !log_dir.exists() {
        eprintln!(
            "{} No MangoHud logs directory found: {}",
            style("Error:").red().bold(),
            log_dir.display()
        );
        eprintln!("  Run a game with MangoHud enabled to generate logs");
        return Ok(());
    }

    // Find all MangoHud CSV files
    let entries = std::fs::read_dir(&log_dir).map_err(|e| PlayError::ValidationFailed {
        reason: format!("Failed to read log directory: {e}"),
    })?;

    let mut log_files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("csv"))
        .filter(|e| e.file_name().to_string_lossy().starts_with("mangohud_"))
        .map(|e| e.path())
        .collect();

    if log_files.is_empty() {
        eprintln!(
            "{} No MangoHud log files found in: {}",
            style("Info:").cyan(),
            log_dir.display()
        );
        eprintln!("  Run a game with MangoHud enabled to generate logs");
        return Ok(());
    }

    // Sort by modification time (newest first)
    log_files.sort_by(|a, b| {
        let a_time =
            std::fs::metadata(a).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        let b_time =
            std::fs::metadata(b).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        b_time.cmp(&a_time)
    });

    println!("{} Found {} MangoHud log file(s)", ui::section("MangoHud Logs:"), log_files.len());
    println!();

    let mut trend_summaries: Vec<helix_core::models::metrics::SessionSummary> = Vec::new();

    for (idx, log_path) in log_files.iter().enumerate() {
        let file_name = log_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");

        // Get file metadata
        let metadata = std::fs::metadata(log_path).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to read log metadata: {e}"),
        })?;
        let modified = metadata.modified().ok().map(|t| {
            chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()
        });
        let size = metadata.len();

        println!("  {}. {}", style(idx + 1).cyan(), style(file_name).bold());
        if let Some(mod_str) = modified {
            println!("     Modified: {}", mod_str);
        }
        println!("     Size: {} bytes", size);

        // Try to parse and show summary
        match MangoHudParser::parse_csv(log_path, file_name.to_string()) {
            Ok(session) => {
                if let Some(ref fps_stats) = session.fps_stats {
                    println!(
                        "     {} FPS: avg={:.1}, min={:.1}, max={:.1}, 1% low={:.1}",
                        style("✓").green(),
                        fps_stats.avg,
                        fps_stats.min,
                        fps_stats.max,
                        fps_stats.percentile_1
                    );
                }
                println!("     Samples: {}", session.samples.len());

                trend_summaries.push(helix_core::models::metrics::SessionSummary {
                    game: session.game_name.clone(),
                    started_at: session.started_at,
                    avg_fps: session.fps_stats.clone().map(|s| s.avg).unwrap_or(0.0),
                    p1_low: session.fps_stats.clone().map(|s| s.percentile_1).unwrap_or(0.0),
                });
            },
            Err(e) => {
                println!("     {} Failed to parse: {}", style("⚠").yellow(), e);
            },
        }
        println!();
    }

    if show_trend && !trend_summaries.is_empty() {
        let trends = helix_core::models::metrics::aggregate_trends(&trend_summaries, 5);
        println!("\n{}", ui::section("FPS Trends (last 5 runs per game)"));
        for t in trends {
            let delta =
                t.delta_pct.map(|d| format!("{:+.1}%", d)).unwrap_or_else(|| "n/a".to_string());
            println!(
                "  {} — runs: {}, avg FPS (last 5): {:.1}, delta: {}",
                style(&t.game).bold(),
                t.runs,
                t.trend_fps,
                delta
            );
        }
        println!();
    }

    // Ask which log to review in detail
    if log_files.len() == 1 {
        review_single_log(&log_files[0])?;
    } else {
        eprintln!(
            "{} Enter log number to review (1-{}), or 0 to exit: ",
            style("Select:").cyan(),
            log_files.len()
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();

        if let Ok(num) = input.trim().parse::<usize>() {
            if num == 0 {
                println!("{} Exited", style("Info:").cyan());
                return Ok(());
            }
            if num >= 1 && num <= log_files.len() {
                review_single_log(&log_files[num - 1])?;
            } else {
                eprintln!("{} Invalid selection", style("Error:").red().bold());
            }
        }
    }

    Ok(())
}

fn review_single_log(log_path: &Path) -> Result<(), PlayError> {
    let file_name = log_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");

    println!("\n{} {}", ui::section("Reviewing"), style(file_name).bold());
    println!("{}", style("─".repeat(60)).dim());

    let session = MangoHudParser::parse_csv(log_path, file_name.to_string())?;

    println!("\n{}", ui::section("Session Summary"));
    println!("  Game: {}", session.game_name);
    println!("  Started: {}", session.started_at.format("%Y-%m-%d %H:%M:%S UTC"));
    if let Some(ended) = session.ended_at {
        let duration = (ended - session.started_at).num_seconds();
        println!("  Duration: {} seconds", duration);
    }
    println!("  Samples: {}", session.samples.len());

    // Print FPS statistics
    if let Some(ref fps) = session.fps_stats {
        println!("\n{}", ui::section("FPS Statistics"));
        println!("  Average: {:.2}", fps.avg);
        println!("  Minimum: {:.2}", fps.min);
        println!("  Maximum: {:.2}", fps.max);
        println!("  1% Low: {:.2}", fps.percentile_1);
        println!("  0.1% Low: {:.2}", fps.percentile_0_1);
    }

    // Print frametime statistics
    if let Some(ref ft) = session.frametime_stats {
        println!("\n{}", ui::section("Frametime Statistics"));
        println!("  Average: {:.2} ms", ft.avg);
        println!("  Minimum: {:.2} ms", ft.min);
        println!("  Maximum: {:.2} ms", ft.max);
        println!("  1% Low: {:.2} ms", ft.percentile_1);
        println!("  0.1% Low: {:.2} ms", ft.percentile_0_1);
    }

    // Print CPU statistics
    if let Some(ref cpu) = session.cpu_stats {
        println!("\n{}", ui::section("CPU Load Statistics"));
        println!("  Average: {:.1}%", cpu.avg);
        println!("  Minimum: {:.1}%", cpu.min);
        println!("  Maximum: {:.1}%", cpu.max);
    }

    // Print GPU statistics
    if let Some(ref gpu) = session.gpu_stats {
        println!("\n{}", ui::section("GPU Load Statistics"));
        println!("  Average: {:.1}%", gpu.avg);
        println!("  Minimum: {:.1}%", gpu.min);
        println!("  Maximum: {:.1}%", gpu.max);
    }

    // Print RAM statistics
    if let Some(ref ram) = session.ram_stats {
        println!("\n{}", ui::section("RAM Usage Statistics"));
        println!("  Average: {:.1} MiB", ram.avg);
        println!("  Minimum: {:.1} MiB", ram.min);
        println!("  Maximum: {:.1} MiB", ram.max);
    }

    // Frame-stall analysis (roadmap item 24)
    let stall_report = helix_core::execution::mangohud::analyze_session(&session);
    if !stall_report.stalls.is_empty() {
        println!("\n{}", ui::section("Frame-Stall Analysis"));
        println!(
            "  Median frametime: {:.2} ms, p99: {:.2} ms",
            stall_report.median_frametime_ms, stall_report.p99_frametime_ms
        );
        println!(
            "  Stalls: {} total (shader-compile: {}, scheduler: {}, unknown: {})",
            stall_report.stalls.len(),
            stall_report.shader_compile_count,
            stall_report.scheduler_count,
            stall_report.unknown_count
        );
        println!("  Top stalls:");
        for stall in stall_report.stalls.iter().take(5) {
            println!(
                "    t={}ms ft={:.1}ms cpu={:.0}% gpu={:.0}% {:?}",
                stall.time_ms, stall.frametime_ms, stall.cpu_load, stall.gpu_load, stall.kind
            );
        }
        if stall_report.shader_compile_count > 0 {
            println!(
                "  Hint: shader-compile stalls detected; DXVK cache should reduce them on later runs."
            );
        }
        if stall_report.scheduler_count > 0 {
            println!("  Hint: scheduler stalls detected; consider the sched_autogroup tweak.");
        }
    }

    // Print VRAM statistics
    if let Some(ref vram) = session.vram_stats {
        println!("\n{}", ui::section("VRAM Usage Statistics"));
        println!("  Average: {:.1} MiB", vram.avg);
        println!("  Minimum: {:.1} MiB", vram.min);
        println!("  Maximum: {:.1} MiB", vram.max);
    }

    // Show sample data (first and last 5)
    if session.samples.len() > 10 {
        println!("\n{}", ui::section("Sample Data (first 5)"));
        for sample in session.samples.iter().take(5) {
            println!("  [{:6}ms] FPS: {:6.1} | CPU: {:5.1}% | GPU: {:5.1}% | RAM: {:6.0} MiB | VRAM: {:6.0} MiB",
                sample.time_ms,
                sample.fps,
                sample.cpu_load,
                sample.gpu_load,
                sample.ram_mib,
                sample.vram_mib
            );
        }

        println!("\n{}", ui::section("Sample Data (last 5)"));
        for sample in session.samples.iter().rev().take(5).rev() {
            println!("  [{:6}ms] FPS: {:6.1} | CPU: {:5.1}% | GPU: {:5.1}% | RAM: {:6.0} MiB | VRAM: {:6.0} MiB",
                sample.time_ms,
                sample.fps,
                sample.cpu_load,
                sample.gpu_load,
                sample.ram_mib,
                sample.vram_mib
            );
        }
    }

    println!("\n{}", style("─".repeat(60)).dim());
    Ok(())
}

// ---------------------------------------------------------------------------
// Write Metrics
// ---------------------------------------------------------------------------

fn write_metrics(
    state_root: &Path,
    env: &GameEnvironment,
    duration_seconds: u64,
    exit_code: Option<i32>,
    crashed: bool,
    fps_metrics: Option<FpsMetrics>,
) -> Result<(), PlayError> {
    let metrics_path = state_root.join("metrics.toml");

    let metrics = SessionMetrics::new(
        env.identity.exe_hash.clone(),
        env.identity.exe_name.clone(),
        Utc::now(),
        env.hardware.gpu.vendor,
        env.runner.runner_type,
    )
    .finalize(duration_seconds, exit_code, crashed, fps_metrics);

    metrics.write_to_file(&metrics_path)?;
    info!(event = "metrics_written", path = %metrics_path.display(), "Session metrics saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Undo / Rollback
// ---------------------------------------------------------------------------

fn undo_game(exe_path: &Path) -> Result<(), PlayError> {
    // Scan for checkpoint matching this exe_path
    let games_root = helix_data_dir().join("games");

    let matching_checkpoint = find_checkpoint_by_exe_path(&games_root, exe_path)?;

    let Some((state_root, _checkpoint)) = matching_checkpoint else {
        return Err(PlayError::ValidationFailed {
            reason: format!("No previous session found for {}", exe_path.display()),
        });
    };

    let cmd_runner = Box::new(RealCommandRunner);
    let mut orchestrator = Orchestrator::with_roots(
        PathBuf::from("/proc"),
        PathBuf::from("/sys"),
        PathBuf::from("/etc"),
        state_root,
        get_runners_root(),
        get_prefix_root(),
        cmd_runner,
    );

    // Try to recover from checkpoint
    match orchestrator.recover() {
        Ok(true) => {
            info!(event = "undo_recovered", phase = ?orchestrator.phase(), "Recovered checkpoint for undo");
            orchestrator.abort()
        },
        Ok(false) => {
            Err(PlayError::ValidationFailed { reason: "No checkpoint found to undo".to_string() })
        },
        Err(e) => Err(e),
    }
}

/// Scan games directory for a checkpoint matching the given exe_path.
fn find_checkpoint_by_exe_path(
    games_root: &Path,
    exe_path: &Path,
) -> Result<Option<(PathBuf, JsonValue)>, PlayError> {
    if !games_root.exists() {
        return Ok(None);
    }

    let entries = std::fs::read_dir(games_root).map_err(|e| PlayError::ValidationFailed {
        reason: format!("Failed to read games directory: {e}"),
    })?;

    for entry in entries.flatten() {
        let state_root = entry.path();
        let checkpoint_path = state_root.join("checkpoint.json");

        if checkpoint_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&checkpoint_path) {
                if let Ok(json) = serde_json::from_str::<JsonValue>(&contents) {
                    let stored_path = json
                        .get("env")
                        .and_then(|e| e.get("identity"))
                        .and_then(|i| i.get("exe_path"))
                        .and_then(|p| p.as_str());
                    if stored_path == Some(exe_path.to_string_lossy().as_ref()) {
                        return Ok(Some((state_root, json)));
                    }
                }
            }
        }
    }

    // One-release fallback to legacy play/ data dir.
    let legacy_root = legacy_data_dir().join("games");
    if games_root != legacy_root && legacy_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&legacy_root) {
            for entry in entries.flatten() {
                let state_root = entry.path();
                let checkpoint_path = state_root.join("checkpoint.json");
                if checkpoint_path.exists() {
                    if let Ok(contents) = std::fs::read_to_string(&checkpoint_path) {
                        if let Ok(json) = serde_json::from_str::<JsonValue>(&contents) {
                            let stored_path = json
                                .get("env")
                                .and_then(|e| e.get("identity"))
                                .and_then(|i| i.get("exe_path"))
                                .and_then(|p| p.as_str());
                            if stored_path == Some(exe_path.to_string_lossy().as_ref()) {
                                return Ok(Some((state_root, json)));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_runners_root() -> PathBuf {
    helix_data_dir().join("runners")
}

fn get_prefix_root() -> PathBuf {
    helix_data_dir().join("prefixes")
}

fn create_orchestrator(
    cmd_runner: Box<dyn helix_core::modules::detection::CommandRunner>,
    exe_path: &Path,
) -> Orchestrator {
    let games_root = helix_data_dir().join("games");

    // Check if there's an existing checkpoint for this exe
    if let Ok(Some((state_root, _checkpoint))) = find_checkpoint_by_exe_path(&games_root, exe_path)
    {
        // Use existing state directory for recovery
        return Orchestrator::with_roots(
            PathBuf::from("/proc"),
            PathBuf::from("/sys"),
            PathBuf::from("/etc"),
            state_root,
            get_runners_root(),
            get_prefix_root(),
            cmd_runner,
        );
    }

    // No existing checkpoint - use temp directory (will be renamed after detection)
    Orchestrator::new(games_root, get_runners_root(), get_prefix_root(), cmd_runner)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = CliArgs::parse();

    // --detect-only: run hardware detection only, output JSON, exit
    if args.detect_only {
        let cmd_runner = Box::new(helix_core::modules::detection::RealCommandRunner);
        match helix_core::modules::detection::detect_hardware(
            std::path::Path::new("/proc"),
            std::path::Path::new("/sys"),
            std::path::Path::new("/etc"),
            cmd_runner.as_ref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        ) {
            Ok(result) => {
                match serde_json::to_string_pretty(&result.hw) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("Failed to serialize detection result: {}", e),
                }
                std::process::exit(0);
            },
            Err(e) => {
                eprintln!("Detection failed: {}", e);
                std::process::exit(1);
            },
        }
    }

    // Handle utility commands that don't need an exe_path (before setting up tracing)
    if args.log_path {
        let log_dir = get_log_dir();
        println!("{}", log_dir.display());
        return;
    }

    if let Some(shell_name) = args.generate_completion {
        let shell = match shell_name.as_str() {
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "fish" => Shell::Fish,
            _ => {
                eprintln!("Error: Unsupported shell '{}'. Supported: bash, zsh, fish", shell_name);
                std::process::exit(1);
            },
        };
        let mut cmd = CliArgs::command();
        generate(shell, &mut cmd, "helix", &mut std::io::stdout());
        return;
    }

    if args.about {
        println!("{}", ui::brand());
        println!("  Linux gaming orchestrator — zero-config Windows game launcher");
        println!("{}", ui::kv("Version", env!("CARGO_PKG_VERSION")));
        println!();
        return;
    }

    // Handle MangoHud review command (doesn't require exe_path)
    if args.review_mangohud {
        match review_mangohud_logs(args.trend) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("{} {}", style("Error:").red().bold(), e);
                std::process::exit(1);
            },
        }
    }

    // Validate exe_path for remaining commands (needed for log filename)
    let exe_path = match args.exe_path {
        Some(path) => path,
        None => {
            eprintln!(
                "{} Run with a .exe file to optimize your system and launch the game",
                ui::section("Usage:")
            );
            eprintln!("  {} helix <game.exe>", style("Example:").dim());
            eprintln!("  {} helix --help for more options", style("Or:").dim());
            std::process::exit(1);
        },
    };

    // Now setup tracing with exe_path for per-run log file naming
    let log_path = setup_tracing(args.verbose, Some(&exe_path));

    // Validate executable exists
    if !exe_path.exists() {
        eprintln!("{} Executable not found: {}", style("Error:").red().bold(), exe_path.display());
        std::process::exit(1);
    }

    // Handle undo command
    if args.undo {
        println!("{} Rolling back previous session...", style("Undo:").yellow().bold());
        match undo_game(&exe_path) {
            Ok(()) => {
                println!("{}", style("✓ Rollback complete").green());
                return;
            },
            Err(e) => {
                eprintln!("{} {}", style("Rollback failed:").red().bold(), e);
                std::process::exit(1);
            },
        }
    }

    // Handle stop command
    if args.stop {
        let pid_path = helix_data_dir().join("last_game.pid");
        let pid =
            std::fs::read_to_string(&pid_path).ok().and_then(|s| s.trim().parse::<u32>().ok());
        match pid {
            Some(pid) => {
                println!("{} Stopping game (PID {})...", style("Stop:").yellow().bold(), pid);
                // Send SIGTERM
                let _ =
                    std::process::Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
                // Give it a moment
                std::thread::sleep(std::time::Duration::from_secs(2));
                // Check if still alive
                if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
                    let _ = std::process::Command::new("kill")
                        .arg("-KILL")
                        .arg(pid.to_string())
                        .status();
                }
                let _ = std::fs::remove_file(&pid_path);
                println!("{}", style("✓ Stop complete").green());
                return;
            },
            None => {
                eprintln!("{} No tracked game PID found", style("Error:").red().bold());
                std::process::exit(1);
            },
        }
    }

    // Handle report command
    if args.report {
        let games_root = data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("helix")
            .join("games");

        let matching = find_checkpoint_by_exe_path(&games_root, &exe_path).unwrap_or(None);

        let Some((state_root, _)) = matching else {
            eprintln!("{} No previous session found for this game", style("Error:").red().bold());
            std::process::exit(1);
        };

        let report_dir = state_root.join("reports");

        if !report_dir.exists() {
            eprintln!("{} No crash reports found for this game", style("Error:").red().bold());
            std::process::exit(1);
        }

        println!("{} Crash reports available in: {}", style("Info:").cyan(), report_dir.display());
        println!("  (Reporting functionality to be implemented)");
        return;
    }

    // Handle --dry-run: plan only, no execution
    if args.dry_run {
        let cmd_runner = Box::new(helix_core::modules::detection::RealCommandRunner);
        let mut orchestrator = create_orchestrator(cmd_runner, &exe_path);
        orchestrator.set_launch_args(args.game_args.clone());
        match orchestrator.plan_only(&exe_path) {
            Ok(plan) => {
                let applied = plan
                    .tweaks
                    .iter()
                    .filter(|t| {
                        matches!(t.decision, helix_core::models::plan::TweakDecision::Apply(_))
                    })
                    .count();

                let mut lines: Vec<String> = Vec::new();
                lines.push(ui::kv("Game", &plan.env.identity.exe_name));
                lines.push(ui::kv("Path", &plan.env.identity.exe_path.display().to_string()));
                lines.push(ui::kv("Hash", &plan.env.identity.exe_hash));
                lines.push(ui::kv(
                    "Runner",
                    &format!("{:?} {}", plan.env.runner.runner_type, plan.env.runner.version),
                ));

                match &plan.runner_action {
                    helix_core::models::plan::RunnerAction::AlreadyInstalled { path } => {
                        lines.push(ui::kv(
                            "Runner action",
                            &format!("already installed ({})", path.display()),
                        ));
                    },
                    helix_core::models::plan::RunnerAction::Download { version, .. } => {
                        lines.push(ui::kv("Runner action", &format!("download {}", version)));
                        lines.push(ui::kv("Download size", "~1.5 GB"));
                    },
                }

                match &plan.prefix_action {
                    helix_core::models::plan::PrefixAction::AlreadyExists { path } => {
                        lines.push(ui::kv(
                            "Prefix",
                            &format!("already exists ({})", path.display()),
                        ));
                    },
                    helix_core::models::plan::PrefixAction::Create { path, .. } => {
                        lines.push(ui::kv("Prefix", &format!("create ({})", path.display())));
                    },
                }

                lines.push(ui::kv("Tweaks", &applied.to_string()));
                for t in &plan.tweaks {
                    if let helix_core::models::plan::TweakDecision::Apply(ref st) = t.decision {
                        lines.push(format!("    · {:?}: {:?}", t.id, st));
                    }
                }

                if !plan.required_packages.is_empty() {
                    lines.push(ui::kv("Packages", ""));
                    for p in &plan.required_packages {
                        lines.push(format!("    - {} ({})", p.name, p.reason));
                    }
                }

                for w in &plan.warnings {
                    lines.push(ui::status_warn(&w.message));
                    if let Some(h) = &w.install_hint {
                        lines.push(format!("      Hint: {}", h));
                    }
                }

                if !plan.hard_blocks.is_empty() {
                    lines.push(ui::status_err("HARD BLOCKS"));
                    for b in &plan.hard_blocks {
                        lines.push(format!("    - {}", b));
                    }
                }

                println!();
                println!("{}", ui::card("PLAN", &lines));
                println!();
                std::process::exit(0);
            },
            Err(e) => {
                eprintln!("{} {}", style("Error:").red().bold(), e);
                std::process::exit(1);
            },
        }
    }

    // Main game launch flow
    let term = Term::stdout();
    println!();
    println!("{}", ui::status_info("Analyzing game…"));
    let _ = term;

    // Create orchestrator (will use existing checkpoint path if found)
    let cmd_runner = Box::new(RealCommandRunner);
    let mut orchestrator = create_orchestrator(cmd_runner, &exe_path);
    orchestrator.set_launch_args(args.game_args.clone());

    // Check for checkpoint recovery (unless --force-fresh is set)
    if !args.force_fresh {
        match orchestrator.recover() {
            Ok(true) => {
                let phase = orchestrator.phase();
                if phase.can_resume() {
                    warn!(event = "recovered_from_checkpoint", phase = ?phase, "Resuming from previous session");
                    let _ = term.write_line(&format!(
                        "  {} Resuming from previous session (phase: {:?})\n",
                        style("Info:").cyan(),
                        phase
                    ));
                } else {
                    let _ = term.write_line(&format!(
                        "  {} Previous session found but cannot resume (phase: {:?})\n",
                        style("Warning:").yellow(),
                        phase
                    ));
                }
            },
            Ok(false) => {
                info!(event = "fresh_start", "Starting fresh session");
            },
            Err(e) => {
                warn!(event = "recovery_failed", error = %e, "Failed to recover checkpoint");
            },
        }
    } else {
        info!(event = "force_fresh", "Starting fresh session (--force-fresh)");
    }

    // Check for fast launch from validated checkpoint
    if orchestrator.can_fast_launch() {
        let game_name = exe_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| exe_path.display().to_string());

        match orchestrator.fast_launch() {
            Ok(pid) => {
                println!(
                    "{}",
                    ui::status_info(&format!("Fast launch: {} (PID {})", game_name, pid))
                );
                std::process::exit(0);
            },
            Err(e) => {
                eprintln!("\n  {} Fast launch failed: {}", style("Warning:").yellow(), e);
                eprintln!("  Falling back to full setup...\n");
            },
        }
    }

    // Run orchestrator lifecycle with progress indicator
    let pb = ui::spinner();
    pb.set_message("Initializing play session...".to_string());
    // keep tick rate consistent with previous
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Set progress callback to update progress bar
    let progress_bar = pb.clone();
    orchestrator.set_progress_callback(move |msg: String| {
        progress_bar.set_message(msg);
    });

    // Set pre/post confirmation callbacks to pause/resume progress bar
    let progress_bar_pause = pb.clone();
    orchestrator.set_pre_confirm_callback(Box::new(move || {
        progress_bar_pause.disable_steady_tick();
    }));

    let progress_bar_resume = pb.clone();
    orchestrator.set_post_confirm_callback(Box::new(move || {
        progress_bar_resume.enable_steady_tick(std::time::Duration::from_millis(100));
    }));

    if args.yes {
        orchestrator.set_confirm_callback(Box::new(|_| true));
    }

    if args.wayland_debug || args.verbose {
        std::env::set_var("WAYLAND_DEBUG", "1");
    }

    let result = orchestrator.run(&exe_path);

    pb.finish_with_message("Session complete");

    match result {
        Ok(()) => {
            let phase = orchestrator.phase();

            if phase == OrchestratorPhase::Validated {
                if let Some(pid) = orchestrator.running_game_pid() {
                    println!();
                    println!(
                        "{}",
                        ui::status_ok(&format!("Game launched successfully (PID: {})", pid))
                    );
                    println!(
                        "  {}",
                        style("Press Ctrl+C to stop monitoring (game continues in background)")
                            .dim()
                    );
                } else {
                    println!();
                    println!("{}", ui::status_ok("Game session completed successfully"));
                }

                // Write metrics on successful completion
                if let Some(env) = orchestrator.env() {
                    // In a full implementation, we'd get actual duration and FPS from execution
                    // For now, write placeholder metrics
                    let games_root = data_dir()
                        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                        .join("helix")
                        .join("games");
                    if let Ok(Some((state_root, _))) =
                        find_checkpoint_by_exe_path(&games_root, &exe_path)
                    {
                        let fps_metrics = env
                            .launch
                            .env
                            .get("MANGOHUD_CONFIGFILE")
                            .map(PathBuf::from)
                            .filter(|p| p.exists())
                            .and_then(|p| {
                                MangoHudParser::parse_csv(&p, env.identity.exe_name.clone()).ok()
                            })
                            .and_then(|session| {
                                session.fps_stats.map(|s| FpsMetrics {
                                    avg_fps: s.avg,
                                    min_fps: s.min,
                                    max_fps: s.max,
                                    percentile_1_low: s.percentile_1,
                                })
                            });

                        if let Err(e) = write_metrics(
                            &state_root,
                            env,
                            0, // Would come from actual session timing
                            Some(0),
                            false,
                            fps_metrics,
                        ) {
                            warn!(event = "metrics_write_failed", error = %e, "Failed to write metrics");
                        }
                    }
                }
            } else if phase == OrchestratorPhase::Planned {
                // User aborted during confirmation
                println!();
                println!("{}", ui::status_info("Aborted by user"));
            }

            std::process::exit(0);
        },
        Err(e) => {
            error!(event = "run_failed", error = %e, "Orchestrator failed");

            // Display error with full context including rollback info
            display_error_with_context(&e, &exe_path, &log_path, orchestrator.rollback_manifest());

            std::process::exit(1);
        },
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_args_parsing() {
        // Test with exe_path
        let args = vec!["play", "/path/to/game.exe"];
        let parsed = CliArgs::parse_from(args);
        assert_eq!(parsed.exe_path, Some(PathBuf::from("/path/to/game.exe")));
        assert!(!parsed.yes);

        // Test with flags
        let args = vec!["play", "--yes", "game.exe"];
        let parsed = CliArgs::parse_from(args);
        assert!(parsed.yes);
    }
}
