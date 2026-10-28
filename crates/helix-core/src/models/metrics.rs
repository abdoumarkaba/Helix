#![allow(clippy::pedantic)]

//! Session metrics for post-game analysis.
//!
//! Records FPS data, session duration, and crash status for each game run.
//! Written to `~/.local/share/play/games/{hash}/metrics.toml` after clean exit.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::environment::{GpuVendor, RunnerType};
use crate::models::errors::PlayError;
use crate::modules::validation::FpsMetrics;

// ---------------------------------------------------------------------------
// SessionMetrics
// ---------------------------------------------------------------------------

/// Post-session metrics recorded after game exit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetrics {
    /// SHA256 hash of the game executable
    pub exe_hash: String,
    /// Name of the executable file
    pub exe_name: String,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// Session duration in seconds
    pub duration_seconds: u64,
    /// Process exit code (None if killed/unknown)
    pub exit_code: Option<i32>,
    /// True if process exited abnormally
    pub crashed: bool,
    /// FPS metrics from MangoHud (if enabled)
    pub fps_metrics: Option<FpsMetrics>,
    /// GPU vendor used
    pub gpu_vendor: GpuVendor,
    /// Runner type used
    pub runner_type: RunnerType,
}

impl SessionMetrics {
    /// Create a new SessionMetrics with basic session info.
    pub fn new(
        exe_hash: String,
        exe_name: String,
        started_at: DateTime<Utc>,
        gpu_vendor: GpuVendor,
        runner_type: RunnerType,
    ) -> Self {
        Self {
            exe_hash,
            exe_name,
            started_at,
            duration_seconds: 0,
            exit_code: None,
            crashed: false,
            fps_metrics: None,
            gpu_vendor,
            runner_type,
        }
    }

    /// Finalize metrics with session outcome.
    pub fn finalize(
        mut self,
        duration_seconds: u64,
        exit_code: Option<i32>,
        crashed: bool,
        fps_metrics: Option<FpsMetrics>,
    ) -> Self {
        self.duration_seconds = duration_seconds;
        self.exit_code = exit_code;
        self.crashed = crashed;
        self.fps_metrics = fps_metrics;
        self
    }

    /// Write metrics to a TOML file.
    pub fn write_to_file(&self, path: &Path) -> Result<(), PlayError> {
        let content = toml::to_string_pretty(self).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to serialize metrics: {e}"),
        })?;
        std::fs::write(path, content).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to write metrics file: {e}"),
        })
    }

    /// Load metrics from a TOML file.
    pub fn load_from_file(path: &Path) -> Result<Self, PlayError> {
        let content = std::fs::read_to_string(path).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to read metrics file: {e}"),
        })?;
        toml::from_str(&content).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to parse metrics file: {e}"),
        })
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_session_metrics_creation() {
        let started = Utc::now();
        let metrics = SessionMetrics::new(
            "abc123".to_string(),
            "game.exe".to_string(),
            started,
            GpuVendor::NVIDIA,
            RunnerType::ProtonGE,
        );

        assert_eq!(metrics.exe_hash, "abc123");
        assert_eq!(metrics.exe_name, "game.exe");
        assert_eq!(metrics.duration_seconds, 0);
        assert!(!metrics.crashed);
        assert!(metrics.fps_metrics.is_none());
    }

    #[test]
    fn test_session_metrics_finalize() {
        let started = Utc::now();
        let metrics = SessionMetrics::new(
            "abc123".to_string(),
            "game.exe".to_string(),
            started,
            GpuVendor::NVIDIA,
            RunnerType::ProtonGE,
        )
        .finalize(
            3600,
            Some(0),
            false,
            Some(FpsMetrics {
                avg_fps: 60.0,
                min_fps: 30.0,
                max_fps: 120.0,
                percentile_1_low: 45.0,
            }),
        );

        assert_eq!(metrics.duration_seconds, 3600);
        assert_eq!(metrics.exit_code, Some(0));
        assert!(!metrics.crashed);
        assert!(metrics.fps_metrics.is_some());
        let fps = metrics.fps_metrics.unwrap();
        assert!((fps.avg_fps - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_session_metrics_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("metrics.toml");

        let started = Utc::now();
        let original = SessionMetrics::new(
            "def456".to_string(),
            "other.exe".to_string(),
            started,
            GpuVendor::AMD,
            RunnerType::WineGE,
        )
        .finalize(1800, Some(1), true, None);

        original.write_to_file(&path).unwrap();
        let loaded = SessionMetrics::load_from_file(&path).unwrap();

        assert_eq!(original.exe_hash, loaded.exe_hash);
        assert_eq!(original.exe_name, loaded.exe_name);
        assert_eq!(original.duration_seconds, loaded.duration_seconds);
        assert_eq!(original.exit_code, loaded.exit_code);
        assert_eq!(original.crashed, loaded.crashed);
        assert_eq!(original.gpu_vendor, loaded.gpu_vendor);
        assert_eq!(original.runner_type, loaded.runner_type);
    }

    #[test]
    fn test_load_missing_file() {
        let result = SessionMetrics::load_from_file(&PathBuf::from("/nonexistent/metrics.toml"));
        assert!(result.is_err());
    }
}

// ---------------------------------------------------------------------------
// Cross-session aggregation (roadmap item 28)
// ---------------------------------------------------------------------------

/// Minimal per-session summary used for trend aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub game: String,
    pub started_at: DateTime<Utc>,
    pub avg_fps: f32,
    pub p1_low: f32,
}

/// Aggregated trend for one game across multiple sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameTrend {
    pub game: String,
    pub runs: usize,
    pub trend_fps: f32,
    pub delta_pct: Option<f32>,
    pub points: Vec<SessionSummary>,
}

/// Aggregate per-session summaries into per-game trends.
///
/// `window` controls how many of the most recent runs feed `trend_fps`
/// (defaults to 5 when 0 is passed).
pub fn aggregate_trends(sessions: &[SessionSummary], window: usize) -> Vec<GameTrend> {
    use std::collections::BTreeMap;

    let window = if window == 0 { 5 } else { window };

    let mut groups: BTreeMap<String, Vec<SessionSummary>> = BTreeMap::new();
    for s in sessions {
        groups.entry(s.game.clone()).or_default().push(s.clone());
    }

    let mut trends = Vec::new();
    for (game, mut points) in groups {
        points.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        let runs = points.len();
        let recent: Vec<&SessionSummary> = points.iter().rev().take(window).collect();
        let trend_fps = if recent.is_empty() {
            0.0
        } else {
            recent.iter().map(|p| p.avg_fps).sum::<f32>() / recent.len() as f32
        };

        let delta_pct = if points.len() >= 2 {
            let first = points[points.len().saturating_sub(window)].avg_fps;
            let last = points[points.len() - 1].avg_fps;
            if first > 0.0 {
                Some((last - first) / first * 100.0)
            } else {
                None
            }
        } else {
            None
        };

        trends.push(GameTrend { game, runs, trend_fps, delta_pct, points });
    }

    trends
}

#[cfg(test)]
mod trend_tests {
    use super::*;

    #[test]
    fn empty_input_empty_output() {
        let t = aggregate_trends(&[], 5);
        assert!(t.is_empty());
    }

    #[test]
    fn single_run_delta_none() {
        let s = SessionSummary {
            game: "g".to_string(),
            started_at: Utc::now(),
            avg_fps: 60.0,
            p1_low: 45.0,
        };
        let t = aggregate_trends(&[s], 5);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].runs, 1);
        assert!(t[0].delta_pct.is_none());
    }

    #[test]
    fn windowed_mean_and_delta() {
        let base = Utc::now();
        let pts: Vec<SessionSummary> = (0..6)
            .map(|i| SessionSummary {
                game: "g".to_string(),
                started_at: base + chrono::Duration::seconds(i as i64),
                avg_fps: 50.0 + i as f32 * 2.0,
                p1_low: 40.0,
            })
            .collect();
        let t = aggregate_trends(&pts, 3);
        assert_eq!(t[0].runs, 6);
        // last 3: 56,58,60 -> mean 58
        assert!((t[0].trend_fps - 58.0).abs() < 0.01);
        assert!(t[0].delta_pct.is_some());
    }
}
