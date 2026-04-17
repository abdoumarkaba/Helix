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
