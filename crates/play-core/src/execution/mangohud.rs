//! MangoHud integration for capturing and analyzing game performance metrics.
//!
//! This module provides:
//! - Configuration of MangoHud for detailed logging
//! - Parsing of MangoHud log files (CSV format)
//! - Structured access to all MangoHud metrics
//! - Utilities for reviewing and analyzing performance data

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::errors::PlayError;

// ---------------------------------------------------------------------------
// MangoHud Metrics Data Structures
// ---------------------------------------------------------------------------

/// Complete MangoHud metrics from a single frame/sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangoHudSample {
    /// Frame timestamp (relative to session start, in milliseconds)
    pub time_ms: u64,
    /// Frames per second
    pub fps: f32,
    /// Frame time in milliseconds
    pub frametime_ms: f32,
    /// CPU usage percentage (0-100)
    pub cpu_load: f32,
    /// GPU usage percentage (0-100)
    pub gpu_load: f32,
    /// RAM usage in MiB
    pub ram_mib: f32,
    /// VRAM usage in MiB
    pub vram_mib: f32,
    /// CPU temperature in Celsius
    pub cpu_temp: Option<f32>,
    /// GPU temperature in Celsius
    pub gpu_temp: Option<f32>,
    /// GPU fan speed percentage (0-100)
    pub gpu_fan: Option<f32>,
    /// GPU power usage in Watts
    pub gpu_power: Option<f32>,
}

impl Default for MangoHudSample {
    fn default() -> Self {
        Self {
            time_ms: 0,
            fps: 0.0,
            frametime_ms: 0.0,
            cpu_load: 0.0,
            gpu_load: 0.0,
            ram_mib: 0.0,
            vram_mib: 0.0,
            cpu_temp: None,
            gpu_temp: None,
            gpu_fan: None,
            gpu_power: None,
        }
    }
}

/// Aggregated statistics for a metric over a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricStats {
    /// Average value
    pub avg: f32,
    /// Minimum value
    pub min: f32,
    /// Maximum value
    pub max: f32,
    /// 1st percentile (1% low)
    pub percentile_1: f32,
    /// 0.1th percentile (0.1% low - more sensitive to stutters)
    pub percentile_0_1: f32,
}

/// Complete MangoHud session data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangoHudSession {
    /// Session identifier (timestamp-based)
    pub session_id: String,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// When the session ended (None if still running)
    pub ended_at: Option<DateTime<Utc>>,
    /// Game executable name
    pub game_name: String,
    /// All captured samples
    pub samples: Vec<MangoHudSample>,
    /// Aggregated FPS statistics
    pub fps_stats: Option<MetricStats>,
    /// Aggregated frametime statistics
    pub frametime_stats: Option<MetricStats>,
    /// Aggregated CPU load statistics
    pub cpu_stats: Option<MetricStats>,
    /// Aggregated GPU load statistics
    pub gpu_stats: Option<MetricStats>,
    /// Aggregated RAM usage statistics
    pub ram_stats: Option<MetricStats>,
    /// Aggregated VRAM usage statistics
    pub vram_stats: Option<MetricStats>,
}

impl MangoHudSession {
    /// Create a new MangoHud session.
    pub fn new(game_name: String) -> Self {
        let session_id = format!("mangohud_{}", Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        Self {
            session_id,
            started_at: Utc::now(),
            ended_at: None,
            game_name,
            samples: Vec::new(),
            fps_stats: None,
            frametime_stats: None,
            cpu_stats: None,
            gpu_stats: None,
            ram_stats: None,
            vram_stats: None,
        }
    }

    /// Add a sample to the session.
    pub fn add_sample(&mut self, sample: MangoHudSample) {
        self.samples.push(sample);
    }

    /// Finalize the session (calculate statistics).
    pub fn finalize(&mut self) {
        if self.samples.is_empty() {
            return;
        }

        self.fps_stats = Some(Self::calculate_stats(&self.samples, |s| s.fps));
        self.frametime_stats = Some(Self::calculate_stats(&self.samples, |s| s.frametime_ms));
        self.cpu_stats = Some(Self::calculate_stats(&self.samples, |s| s.cpu_load));
        self.gpu_stats = Some(Self::calculate_stats(&self.samples, |s| s.gpu_load));
        self.ram_stats = Some(Self::calculate_stats(&self.samples, |s| s.ram_mib));
        self.vram_stats = Some(Self::calculate_stats(&self.samples, |s| s.vram_mib));

        self.ended_at = Some(Utc::now());
    }

    /// Calculate statistics for a metric extracted from samples.
    fn calculate_stats<F>(samples: &[MangoHudSample], extractor: F) -> MetricStats
    where
        F: Fn(&MangoHudSample) -> f32,
    {
        let mut values: Vec<f32> = samples.iter().map(&extractor).filter(|&v| v.is_finite()).collect();

        if values.is_empty() {
            return MetricStats {
                avg: 0.0,
                min: 0.0,
                max: 0.0,
                percentile_1: 0.0,
                percentile_0_1: 0.0,
            };
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min = values[0];
        let max = values[values.len() - 1];
        let avg = values.iter().sum::<f32>() / values.len() as f32;

        let percentile_1_idx = (values.len() as f32 * 0.01).max(1.0) as usize;
        let percentile_0_1_idx = (values.len() as f32 * 0.001).max(1.0) as usize;

        let percentile_1 = values.get(percentile_1_idx).copied().unwrap_or(min);
        let percentile_0_1 = values.get(percentile_0_1_idx).copied().unwrap_or(min);

        MetricStats {
            avg,
            min,
            max,
            percentile_1,
            percentile_0_1,
        }
    }

    /// Write session to a JSON file.
    pub fn write_to_file(&self, path: &Path) -> Result<(), PlayError> {
        let content = serde_json::to_string_pretty(self).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to serialize MangoHud session: {e}"),
        })?;
        fs::write(path, content).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to write MangoHud session file: {e}"),
        })
    }

    /// Load session from a JSON file.
    pub fn load_from_file(path: &Path) -> Result<Self, PlayError> {
        let content = fs::read_to_string(path).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to read MangoHud session file: {e}"),
        })?;
        serde_json::from_str(&content).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to parse MangoHud session file: {e}"),
        })
    }
}

// ---------------------------------------------------------------------------
// MangoHud Log Parser
// ---------------------------------------------------------------------------

/// Parser for MangoHud CSV log files.
///
/// Expected format (comma-separated, header line):
/// ```text
/// fps,frametime,cpu,gpu_load,ram,vram,cpu_temp,gpu_temp,gpu_fan,gpu_power
/// 60.0,16.67,45.2,78.3,4096,2048,65.0,72.0,50.0,180.0
/// ...
/// ```
pub struct MangoHudParser;

impl MangoHudParser {
    /// Parse a MangoHud CSV log file into a MangoHudSession.
    pub fn parse_csv(path: &Path, game_name: String) -> Result<MangoHudSession, PlayError> {
        let content = fs::read_to_string(path).map_err(|e| PlayError::ValidationFailed {
            reason: format!("Failed to read MangoHud log: {e}"),
        })?;

        let mut session = MangoHudSession::new(game_name);
        let mut time_ms = 0u64;
        const SAMPLE_INTERVAL_MS: u64 = 100; // Assume 10Hz sampling

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Skip header line
            if line_num == 0 && line.contains("fps") {
                continue;
            }

            if let Ok(sample) = Self::parse_line(line, time_ms) {
                session.add_sample(sample);
                time_ms += SAMPLE_INTERVAL_MS;
            }
        }

        if session.samples.is_empty() {
            return Err(PlayError::ValidationFailed {
                reason: "No valid samples found in MangoHud log".to_string(),
            });
        }

        session.finalize();
        Ok(session)
    }

    /// Parse a single CSV line into a MangoHudSample.
    fn parse_line(line: &str, time_ms: u64) -> Result<MangoHudSample, PlayError> {
        let parts: Vec<&str> = line.split(',').collect();

        if parts.len() < 2 {
            return Err(PlayError::ValidationFailed {
                reason: format!("Invalid MangoHud log line: not enough fields: {line}"),
            });
        }

        let parse_field = |idx: usize, default: f32| -> f32 {
            parts
                .get(idx)
                .and_then(|s| s.trim().parse::<f32>().ok())
                .filter(|&v| v.is_finite())
                .unwrap_or(default)
        };

        let parse_optional = |idx: usize| -> Option<f32> {
            parts.get(idx).and_then(|s| s.trim().parse::<f32>().ok()).filter(|&v| v.is_finite() && v > 0.0)
        };

        Ok(MangoHudSample {
            time_ms,
            fps: parse_field(0, 0.0),
            frametime_ms: parse_field(1, 0.0),
            cpu_load: parse_field(2, 0.0),
            gpu_load: parse_field(3, 0.0),
            ram_mib: parse_field(4, 0.0),
            vram_mib: parse_field(5, 0.0),
            cpu_temp: parse_optional(6),
            gpu_temp: parse_optional(7),
            gpu_fan: parse_optional(8),
            gpu_power: parse_optional(9),
        })
    }
}

// ---------------------------------------------------------------------------
// MangoHud Configuration
// ---------------------------------------------------------------------------

/// Configuration for MangoHud logging.
#[derive(Debug, Clone)]
pub struct MangoHudConfig {
    /// Path to log file
    pub log_path: PathBuf,
    /// Whether to enable logging
    pub enabled: bool,
    /// Log format (csv, json)
    pub format: LogFormat,
}

/// Log format for MangoHud output.
#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    /// Comma-separated values
    Csv,
    /// JSON (not yet supported by MangoHud directly)
    Json,
}

impl MangoHudConfig {
    /// Create a new MangoHud config with default settings.
    pub fn new(log_dir: &Path, _game_name: &str) -> Self {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let log_path = log_dir.join(format!("mangohud_{}.csv", timestamp));

        Self {
            log_path,
            enabled: true,
            format: LogFormat::Csv,
        }
    }

    /// Get the environment variables needed to configure MangoHud.
    ///
    /// Returns a map of environment variable names to values.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return vec![];
        }

        let mut vars = vec![
            ("MANGOHUD".to_string(), "1".to_string()),
            ("MANGOHUD_CONFIGFILE".to_string(), self.log_path.display().to_string()),
        ];

        // Configure MangoHud to log all metrics
        vars.push(("MANGOHUD_LOG".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_LOG_FILE".to_string(), self.log_path.display().to_string()));

        // Enable all metrics
        vars.push(("MANGOHUD_CPU".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_GPU".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_RAM".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_VRAM".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_TEMP".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_FPS".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_FRAMETIME".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_FAN".to_string(), "1".to_string()));
        vars.push(("MANGOHUD_POWER".to_string(), "1".to_string()));

        vars
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_csv_valid() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("mangohud.csv");

        let log_content = r#"fps,frametime,cpu,gpu_load,ram,vram,cpu_temp,gpu_temp,gpu_fan,gpu_power
60.0,16.67,45.2,78.3,4096,2048,65.0,72.0,50.0,180.0
59.5,16.81,46.1,79.1,4100,2050,66.0,73.0,51.0,182.0
61.2,16.34,44.8,77.5,4092,2045,64.5,71.5,49.5,178.0"#;

        fs::write(&log_path, log_content).unwrap();

        let session = MangoHudParser::parse_csv(&log_path, "test_game".to_string()).unwrap();

        assert_eq!(session.samples.len(), 3);
        assert_eq!(session.game_name, "test_game");
        assert!(session.fps_stats.is_some());

        let fps_stats = session.fps_stats.unwrap();
        assert!((fps_stats.avg - 60.23).abs() < 0.1);
        assert!((fps_stats.min - 59.5).abs() < 0.1);
        assert!((fps_stats.max - 61.2).abs() < 0.1);
    }

    #[test]
    fn test_parse_csv_empty() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("mangohud.csv");

        fs::write(&log_path, "").unwrap();

        let result = MangoHudParser::parse_csv(&log_path, "test_game".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_session_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("session.json");

        let mut session = MangoHudSession::new("test_game".to_string());
        session.add_sample(MangoHudSample {
            time_ms: 0,
            fps: 60.0,
            frametime_ms: 16.67,
            cpu_load: 50.0,
            gpu_load: 75.0,
            ram_mib: 4096.0,
            vram_mib: 2048.0,
            cpu_temp: Some(65.0),
            gpu_temp: Some(70.0),
            gpu_fan: Some(50.0),
            gpu_power: Some(180.0),
        });
        session.finalize();

        session.write_to_file(&session_path).unwrap();
        let loaded = MangoHudSession::load_from_file(&session_path).unwrap();

        assert_eq!(session.session_id, loaded.session_id);
        assert_eq!(session.game_name, loaded.game_name);
        assert_eq!(session.samples.len(), loaded.samples.len());
    }

    #[test]
    fn test_mangohud_config_env_vars() {
        let temp_dir = TempDir::new().unwrap();
        let config = MangoHudConfig::new(temp_dir.path(), "test_game");

        let vars = config.env_vars();
        assert!(!vars.is_empty());

        // Check that MANGOHUD=1 is set
        assert!(vars.iter().any(|(k, v)| k == "MANGOHUD" && v == "1"));

        // Check that log file path is set
        assert!(vars.iter().any(|(k, _)| k == "MANGOHUD_LOG_FILE"));
    }
}
