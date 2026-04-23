//! CLI entry point for the `play` Linux gaming orchestrator.
//!
//! Handles argument parsing, tracing setup, plan display, user confirmation,
//! and post-session metrics/reporting workflows.

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use serde_json::Value as JsonValue;
use console::{style, Term};
use dirs::data_dir;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{error, info, warn};
use tracing_subscriber::prelude::*;

use play_core::models::environment::GameEnvironment;
use play_core::models::errors::PlayError;
use play_core::models::metrics::SessionMetrics;
use play_core::modules::detection::RealCommandRunner;
use play_core::modules::validation::FpsMetrics;
use play_core::orchestrator::{Orchestrator, OrchestratorPhase};

// ---------------------------------------------------------------------------
// CLI Arguments
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "play")]
#[command(about = "Linux gaming orchestrator — zero-config Windows game launcher")]
#[command(version)]
struct CliArgs {
    /// Path to game executable
    exe_path: Option<PathBuf>,

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
}

// ---------------------------------------------------------------------------
// Tracing Setup
// ---------------------------------------------------------------------------

fn get_log_dir() -> PathBuf {
    data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("play")
        .join("logs")
}

fn setup_tracing(_verbose: bool, exe_path: Option<&Path>) -> PathBuf {
    let log_dir = get_log_dir();

    // Create log directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("Warning: Failed to create log directory: {}", e);
    }

    // Generate per-run log filename: play-{timestamp}-{game_name}.log
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let game_name = exe_path
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let log_filename = format!("play-{}-{}.log", timestamp, game_name);
    let log_path = log_dir.join(&log_filename);

    // Create single file appender (non-rotating) for this run
    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::NEVER,
        &log_dir,
        format!("play-{}-{}", timestamp, game_name),
    );
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Keep the guard alive for the duration of the program
    Box::leak(Box::new(_guard));

    // Always verbose for now - show everything to console
    let filter = "play_core=debug,play_cli=debug,info";

    // Set up console output in addition to file
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(false);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking).with_ansi(false).with_target(false))
        .with(console_layer)
        .with(tracing_subscriber::EnvFilter::new(filter))
        .init();

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
    rollback_entries: &[play_core::orchestrator::RollbackEntry],
) {
    eprintln!("\n  {} {}", style("Error:").red().bold(), error);

    // Extract error-specific context
    let error_context = match error {
        PlayError::RunnerDownload { url, attempt, reason } => {
            Some(format!(
                "  URL: {}\n  Attempt: {}/3\n  Reason: {}",
                url, attempt, reason
            ))
        },
        PlayError::PackageInstall { pm, package, .. } => {
            Some(format!("  Package manager: {}\n  Package: {}", pm, package))
        },
        PlayError::GameCrash { seconds, .. } => {
            Some(format!("  Crashed after: {} seconds", seconds))
        },
        PlayError::GameLaunchFailed { exe_path, reason } => {
            Some(format!("  Executable: {}\n  Reason: {}", exe_path.display(), reason))
        },
        _ => None,
    };

    if let Some(ctx) = error_context {
        eprintln!("{}", ctx);
    }

    // Show rollback information if any
    if !rollback_entries.is_empty() {
        eprintln!("\n  {}", style("Rollback completed:").yellow().bold());
        for entry in rollback_entries {
            eprintln!("    · Restored {}: {}", entry.key, entry.previous_value);
        }
    }

    // Provide next steps based on error type
    eprintln!("\n  {}", style("Next steps:").cyan().bold());

    match error {
        PlayError::RunnerDownload { .. } => {
            eprintln!("    1. Check your internet connection");
            eprintln!("    2. Verify the runner URL is accessible");
            eprintln!(
                "    3. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!(
                "    4. Try again: {}",
                style(format!("play {}", exe_path.display())).cyan()
            );
            eprintln!(
                "    5. Or rollback: {}",
                style(format!("play --undo {}", exe_path.display())).cyan()
            );
        },
        PlayError::PackageInstall { .. } => {
            eprintln!("    1. Check if package manager is working");
            eprintln!("    2. Try installing manually with your package manager");
            eprintln!(
                "    3. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!(
                "    4. Try again: {}",
                style(format!("play {}", exe_path.display())).cyan()
            );
            eprintln!(
                "    5. Or rollback: {}",
                style(format!("play --undo {}", exe_path.display())).cyan()
            );
        },
        PlayError::GameCrash { .. } => {
            eprintln!(
                "    1. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!(
                "    2. Run {} to file a crash report",
                style(format!("play --report {}", exe_path.display())).cyan()
            );
            eprintln!("    3. Try running with different compatibility settings");
        },
        PlayError::ValidationFailed { .. } => {
            eprintln!("    1. Check if game process is still running");
            eprintln!("    2. Verify GPU drivers are installed and working");
            eprintln!(
                "    3. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!(
                "    4. Run {} to file a report",
                style(format!("play --report {}", exe_path.display())).cyan()
            );
        },
        PlayError::NoRunnerAvailable { .. } => {
            eprintln!("    1. Check that ProtonGE runners are installed");
            eprintln!("    2. Install required tools (pciutils, vulkan-tools)");
            eprintln!(
                "    3. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!("    4. Try a different game or check runner compatibility");
        },
        PlayError::UnsupportedDistro { .. } => {
            eprintln!("    1. Check if your distro is supported");
            eprintln!("    2. Try running on a supported distro (Ubuntu 22.04+, Fedora 38+, Arch, Debian 12+)");
            eprintln!(
                "    3. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
        },
        PlayError::InsufficientVram { .. } => {
            eprintln!("    1. Close other applications to free VRAM");
            eprintln!("    2. Lower game graphics settings");
            eprintln!("    3. Consider upgrading your GPU");
            eprintln!(
                "    4. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
        },
        _ => {
            eprintln!(
                "    1. Check the log file for details: {}",
                style(log_path.display()).cyan()
            );
            eprintln!(
                "    2. Try again: {}",
                style(format!("play {}", exe_path.display())).cyan()
            );
            eprintln!(
                "    3. Or rollback: {}",
                style(format!("play --undo {}", exe_path.display())).cyan()
            );
        },
    }

    // Log file path was already printed at startup
}

/// Show log file location at end of run
#[allow(dead_code)]
fn show_log_path(_log_path: &Path) {
    // Log path is now shown at startup, but keep this for potential future use
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
    let games_root = data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("play")
        .join("games");

    let matching_checkpoint = find_checkpoint_by_exe_path(&games_root, exe_path)?;

    let Some((_state_root, _checkpoint)) = matching_checkpoint else {
        return Err(PlayError::ValidationFailed {
            reason: format!("No previous session found for {}", exe_path.display()),
        });
    };

    let cmd_runner = Box::new(RealCommandRunner);
    let mut orchestrator = create_orchestrator(cmd_runner);

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

    Ok(None)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_runners_root() -> PathBuf {
    data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("runners")
}

fn get_prefix_root() -> PathBuf {
    data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("prefixes")
}

fn create_orchestrator(
    cmd_runner: Box<dyn play_core::modules::detection::CommandRunner>,
) -> Orchestrator {
    // Pass the games root directly - orchestrator will create temp dir then rename to SHA256
    let games_root =
        data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("games");

    Orchestrator::new(games_root, get_runners_root(), get_prefix_root(), cmd_runner)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = CliArgs::parse();

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
            }
        };
        let mut cmd = CliArgs::command();
        generate(shell, &mut cmd, "play", &mut std::io::stdout());
        return;
    }

    if args.about {
        println!("{}", style("play").bold().cyan());
        println!("  Linux gaming orchestrator — zero-config Windows game launcher");
        println!();
        println!("  Version: {}", env!("CARGO_PKG_VERSION"));
        println!();
        return;
    }

    // Validate exe_path for remaining commands (needed for log filename)
    let exe_path = match args.exe_path {
        Some(path) => path,
        None => {
            eprintln!(
                "{} Run with a .exe file to optimize your system and launch the game",
                style("Usage:").green().bold()
            );
            eprintln!("  {} play <game.exe>", style("Example:").dim());
            eprintln!("  {} play --help for more options", style("Or:").dim());
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

    // Handle report command
    if args.report {
        let games_root = data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("play")
            .join("games");

        let matching = find_checkpoint_by_exe_path(&games_root, &exe_path)
            .unwrap_or(None);

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

    // Main game launch flow
    let term = Term::stdout();
    let _ = term.write_line(&format!(
        "\n  {} {}\n",
        style("▶").green().bold(),
        style("Analyzing game...").dim()
    ));

    // Create orchestrator
    let cmd_runner = Box::new(RealCommandRunner);
    let mut orchestrator = create_orchestrator(cmd_runner);

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

    // Run orchestrator lifecycle with progress indicator
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message("Initializing play session...");
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

    let result = orchestrator.run(&exe_path);

    pb.finish_with_message("Session complete");

    match result {
        Ok(()) => {
            let phase = orchestrator.phase();

            if phase == OrchestratorPhase::Validated {
                if let Some(pid) = orchestrator.running_game_pid() {
                    println!(
                        "\n  {} Game launched successfully (PID: {})",
                        style("Game launched successfully").green().bold(),
                        pid
                    );
                    println!(
                        "  {}",
                        style("Press Ctrl+C to stop monitoring (game continues in background)").dim()
                    );
                } else {
                    println!(
                        "\n  {} Game session completed successfully",
                        style("Game session completed successfully").green().bold()
                    );
                }

                // Write metrics on successful completion
                if let Some(env) = orchestrator.env() {
                    // In a full implementation, we'd get actual duration and FPS from execution
                    // For now, write placeholder metrics
                    let games_root = data_dir()
                        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                        .join("play")
                        .join("games");
                    if let Ok(Some((state_root, _))) = find_checkpoint_by_exe_path(&games_root, &exe_path) {
                        if let Err(e) = write_metrics(
                            &state_root,
                            env,
                            0, // Would come from actual session timing
                            Some(0),
                            false,
                            None, // Would come from ValidationModule
                        ) {
                            warn!(event = "metrics_write_failed", error = %e, "Failed to write metrics");
                        }
                    }
                }
            } else if phase == OrchestratorPhase::Planned {
                // User aborted during confirmation
                println!("\n  {} {}", style("⊘").dim(), style("Aborted by user").dim());
            }

            std::process::exit(0);
        },
        Err(e) => {
            error!(event = "run_failed", error = %e, "Orchestrator failed");

            eprintln!(
                "\n  {}",
                style("Game session failed").red().bold()
            );

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
