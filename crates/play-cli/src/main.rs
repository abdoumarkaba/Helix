//! CLI entry point for the `play` Linux gaming orchestrator.
//!
//! Handles argument parsing, tracing setup, plan display, user confirmation,
//! and post-session metrics/reporting workflows.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use clap::Parser;
use serde_json::Value as JsonValue;
use console::{style, Term};
use dirs::data_dir;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{error, info, warn};

use play_core::models::environment::GameEnvironment;
use play_core::models::errors::PlayError;
use play_core::models::metrics::SessionMetrics;
use play_core::models::plan::GamePlan;
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

    /// Update play-db to latest
    #[arg(long)]
    update_db: bool,

    /// Enable verbose tracing output
    #[arg(long, short)]
    verbose: bool,

    /// Plan only, do not execute
    #[arg(long)]
    dry_run: bool,

    /// File a crash report for last session
    #[arg(long)]
    report: bool,
}

// ---------------------------------------------------------------------------
// Tracing Setup
// ---------------------------------------------------------------------------

fn setup_tracing(verbose: bool) {
    let filter = if verbose {
        "play_core=debug,play_cli=debug,info"
    } else {
        "play_core=info,play_cli=info,warn"
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .init();
}

// ---------------------------------------------------------------------------
// Plan Display
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn display_plan(plan: &GamePlan, term: &Term) {
    let _ = term.write_line(&format!("\n{}", style("═".repeat(60)).dim()));
    let _ = term.write_line(&format!("  {}", style("Game Plan").bold().cyan()));
    let _ = term.write_line(&format!("{}\n", style("═".repeat(60)).dim()));

    // Identity section
    let env = &plan.env;
    let _ = term.write_line(&format!(
        "  {}: {}",
        style("Game").bold(),
        style(&env.identity.exe_name).green()
    ));
    let _ = term.write_line(&format!(
        "  {}: {}",
        style("Hash").bold(),
        style(&env.identity.exe_hash[..16.min(env.identity.exe_hash.len())]).dim()
    ));
    let _ =
        term.write_line(&format!("  {}: {:?}", style("DirectX").bold(), env.identity.dx_version));

    // Hardware section
    let _ = term.write_line(&format!("\n  {}", style("Hardware").bold().underlined()));
    let _ = term.write_line(&format!(
        "    GPU: {:?} {} ({} MB)",
        env.hardware.gpu.vendor, env.hardware.gpu.model, env.hardware.gpu.vram_mb
    ));
    let _ = term.write_line(&format!(
        "    CPU: {} ({} cores)",
        env.hardware.cpu.model, env.hardware.cpu.logical_cores
    ));
    let _ = term.write_line(&format!("    RAM: {} MB", env.hardware.memory.total_mb));
    let _ = term
        .write_line(&format!("    Audio: {:?} @ {} Hz", env.audio.backend, env.audio.server_rate));

    // Runner section
    let _ = term.write_line(&format!("\n  {}", style("Runner").bold().underlined()));
    let _ =
        term.write_line(&format!("    Type: {:?} v{}", env.runner.runner_type, env.runner.version));
    let _ = term.write_line(&format!(
        "    Status: {}",
        match &plan.runner_action {
            play_core::models::plan::RunnerAction::AlreadyInstalled { .. } =>
                style("✓ Already installed").green(),
            play_core::models::plan::RunnerAction::Download { .. } =>
                style("⬇ Download required").yellow(),
        }
    ));

    // Prefix section
    let _ = term.write_line(&format!("\n  {}", style("Prefix").bold().underlined()));
    let _ = term.write_line(&format!("    Arch: {:?}", env.prefix.arch));
    let _ = term.write_line(&format!("    Windows: {:?}", env.prefix.windows_version));
    let _ = term.write_line(&format!(
        "    Status: {}",
        match &plan.prefix_action {
            play_core::models::plan::PrefixAction::AlreadyExists { .. } =>
                style("✓ Exists").green(),
            play_core::models::plan::PrefixAction::Create { .. } => style("⚠ Create new").yellow(),
        }
    ));

    // Tweaks section
    if !plan.tweaks.is_empty() {
        let _ = term.write_line(&format!("\n  {}", style("System Tweaks").bold().underlined()));
        for tweak in &plan.tweaks {
            let status = match &tweak.decision {
                play_core::models::plan::TweakDecision::Apply(_) => style("⚡ Apply").yellow(),
                play_core::models::plan::TweakDecision::AlreadySatisfied => {
                    style("✓ Already set").green()
                },
                play_core::models::plan::TweakDecision::NotApplicable { .. } => {
                    style("⊘ Skip").dim()
                },
            };
            let _ = term.write_line(&format!("    {:<25} {}", format!("{:?}", tweak.id), status));
        }
    }

    // Warnings
    if !plan.warnings.is_empty() {
        let _ = term.write_line(&format!("\n  {}", style("Warnings").bold().underlined().yellow()));
        for warning in &plan.warnings {
            let _ = term.write_line(&format!("    ⚠ {}", warning.message));
        }
    }

    // Hard blocks (shouldn't reach here if present, but show them)
    if !plan.hard_blocks.is_empty() {
        let _ =
            term.write_line(&format!("\n  {}", style("BLOCKING ISSUES").bold().red().on_white()));
        for block in &plan.hard_blocks {
            let _ = term.write_line(&format!("    ✗ {}", block));
        }
    }

    // Required packages
    if !plan.required_packages.is_empty() {
        let _ = term.write_line(&format!("\n  {}", style("Required Packages").bold().underlined()));
        for pkg in &plan.required_packages {
            let _ = term.write_line(&format!("    ⬇ {} ({})", pkg.name, pkg.reason));
        }
    }

    let _ = term.write_line(&format!("\n{}", style("═".repeat(60)).dim()));
}

// ---------------------------------------------------------------------------
// User Confirmation
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn confirm_prompt(term: &Term, yes_flag: bool) -> bool {
    if yes_flag {
        let _ = term.write_line(&format!("  {} (--yes flag set)", style("Auto-confirming").dim()));
        return true;
    }

    let _ = term.write_line("");
    let _ = term.write_str(&format!("  {} [Y/n] ", style("Proceed?").bold()));
    let _ = term.flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    let trimmed = input.trim().to_lowercase();
    trimmed == "y" || trimmed == "yes" || trimmed.is_empty()
}

// ---------------------------------------------------------------------------
// Update DB
// ---------------------------------------------------------------------------

fn update_db() -> Result<(), PlayError> {
    let db_dir =
        data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("db");

    info!(event = "update_db_start", path = %db_dir.display(), "Updating play-db");

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}").unwrap());
    pb.set_message("Updating play-db...");

    // Check if directory exists and has .git
    let git_dir = db_dir.join(".git");
    let result = if git_dir.exists() {
        // Pull latest
        Command::new("git")
            .args(["-C", &db_dir.to_string_lossy(), "pull", "origin", "main"])
            .output()
    } else {
        // Clone fresh
        std::fs::create_dir_all(&db_dir).map_err(|e| PlayError::DatabaseUpdateFailed {
            reason: format!("Failed to create db directory: {e}"),
        })?;
        Command::new("git")
            .args(["clone", "https://github.com/abdoumarkt/play-db.git", &db_dir.to_string_lossy()])
            .output()
    };

    pb.finish_and_clear();

    match result {
        Ok(output) => {
            if output.status.success() {
                info!(event = "update_db_complete", "play-db updated successfully");
                println!("{}", style("✓ play-db updated successfully").green());
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(PlayError::DatabaseUpdateFailed {
                    reason: format!("git command failed: {stderr}"),
                })
            }
        },
        Err(e) => {
            Err(PlayError::DatabaseUpdateFailed { reason: format!("Failed to run git: {e}") })
        },
    }
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

fn get_db_root() -> PathBuf {
    data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("db")
}

fn create_orchestrator(
    cmd_runner: Box<dyn play_core::modules::detection::CommandRunner>,
) -> Orchestrator {
    // Pass the games root directly - orchestrator will create temp dir then rename to SHA256
    let games_root =
        data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("play").join("games");

    Orchestrator::new(
        games_root,
        get_runners_root(),
        get_prefix_root(),
        get_db_root(),
        cmd_runner,
    )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = CliArgs::parse();

    setup_tracing(args.verbose);

    // Handle utility commands that don't need an exe_path
    if args.update_db {
        if let Err(e) = update_db() {
            error!(event = "update_db_failed", error = %e, "Failed to update play-db");
            eprintln!("{} {}", style("Error:").red().bold(), e);
            std::process::exit(1);
        }
        return;
    }

    // Validate exe_path for remaining commands
    let exe_path = match args.exe_path {
        Some(path) => path,
        None => {
            eprintln!(
                "{} game.exe required (unless using --update-db)",
                style("Error:").red().bold()
            );
            std::process::exit(1);
        },
    };

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

    let cmd_runner = Box::new(RealCommandRunner);
    let mut orchestrator = create_orchestrator(cmd_runner);

    // Check for checkpoint recovery
    match orchestrator.recover() {
        Ok(true) => {
            warn!(event = "recovered_from_checkpoint", phase = ?orchestrator.phase(), "Resuming from previous session");
            let _ = term.write_line(&format!(
                "  {} Recovered previous session (phase: {:?})\n",
                style("⚠").yellow(),
                orchestrator.phase()
            ));
        },
        Ok(false) => {
            info!(event = "fresh_start", "Starting fresh session");
        },
        Err(e) => {
            warn!(event = "recovery_failed", error = %e, "Failed to recover checkpoint");
        },
    }

    // Run orchestrator lifecycle
    let result = orchestrator.run(&exe_path);

    match result {
        Ok(()) => {
            let phase = orchestrator.phase();

            if phase == OrchestratorPhase::Validated {
                println!(
                    "\n  {} {}",
                    style("✓").green().bold(),
                    style("Game session completed successfully").green()
                );

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

            // Check if it's a crash vs other error
            let is_crash = matches!(&e, PlayError::ValidationFailed { .. });

            eprintln!(
                "\n  {} {}",
                style("✗").red().bold(),
                style("Game session failed").red().bold()
            );
            eprintln!("  {} {}", style("Error:").red(), e);

            if is_crash {
                println!("\n  {}", style("This appears to be a game crash.").yellow());
                println!(
                    "  Run {} to file a report.",
                    style(format!("play --report {}", exe_path.display())).cyan()
                );
            }

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

        // Test utility flags without exe_path
        let args = vec!["play", "--update-db"];
        let parsed = CliArgs::parse_from(args);
        assert!(parsed.update_db);
        assert!(parsed.exe_path.is_none());
    }

}
