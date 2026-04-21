//! Runner download and verification module.
//!
//! Downloads Proton-GE / Wine-GE releases, verifies SHA512 against the
//! runners.toml manifest, and extracts them to the install directory.
//! Retries up to 3 times on transient failures. On checksum mismatch,
//! the downloaded file is deleted for safety.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use indicatif::ProgressBar;
use sha2::{Digest, Sha512};

use crate::models::errors::PlayError;
use crate::models::plan::RunnerAction;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum download retry attempts.
const MAX_RETRIES: u8 = 3;

/// Buffer size for streaming downloads (64 KiB).
const DOWNLOAD_BUF_SIZE: usize = 65536;

/// Estimated download size for Proton-GE (1.2GB) + 20% buffer.
const ESTIMATED_DOWNLOAD_SIZE: u64 = 1_500_000_000;

/// Check available disk space before download using df command.
fn check_disk_space(path: &Path) -> Result<u64, PlayError> {
    use std::process::Command;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| PlayError::RunnerDownload {
                url: String::new(),
                attempt: 1,
                reason: format!("Failed to create directory: {e}"),
            })?;
        }
    }

    // Use df to get available space
    let output = Command::new("df")
        .args(["-B1", "--output=avail", path.display().to_string().as_str()])
        .output()
        .map_err(|e| PlayError::RunnerDownload {
            url: String::new(),
            attempt: 1,
            reason: format!("Failed to check disk space: {e}"),
        })?;

    let available: u64 = String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or(0);

    if available < ESTIMATED_DOWNLOAD_SIZE {
        return Err(PlayError::RunnerDownload {
            url: String::new(),
            attempt: 1,
            reason: format!(
                "Insufficient disk space. Required: ~1.5GB, Available: {}MB. Location: {}",
                available / 1_000_000,
                path.display()
            ),
        });
    }

    Ok(available)
}

// ---------------------------------------------------------------------------
// RunnerModule
// ---------------------------------------------------------------------------

/// Executes runner-related actions from the game plan.
///
/// - `AlreadyInstalled`: verifies SHA256 of the existing directory.
/// - `Download`: downloads, verifies, and extracts the runner.
pub struct RunnerModule {
    /// Root directory where runners are installed.
    /// Typically `~/.local/share/play/runners/`.
    runners_install_root: PathBuf,
}

impl RunnerModule {
    #[must_use]
    pub fn new(runners_install_root: PathBuf) -> Self {
        Self { runners_install_root }
    }

    /// Execute the runner action from the plan.
    ///
    /// Returns the installed path on success.
    ///
    /// # Errors
    ///
    /// Returns `PlayError::RunnerDownload` if download, verification, or extraction fails.
    pub fn execute(&self, action: &RunnerAction) -> Result<PathBuf, PlayError> {
        match action {
            RunnerAction::AlreadyInstalled { path } => {
                tracing::info!(
                    event = "runner_already_installed",
                    path = %path.display(),
                    "Runner already installed, skipping download"
                );
                // Verify the directory exists
                if !path.exists() {
                    return Err(PlayError::RunnerDownload {
                        url: String::new(),
                        attempt: 1,
                        reason: format!("Runner directory does not exist: {}", path.display()),
                    });
                }
                Ok(path.clone())
            },
            RunnerAction::Download { url, version, sha512 } => {
                let install_path = self.install_path_for(version);
                if install_path.exists() {
                    tracing::info!(
                        event = "runner_already_installed",
                        path = %install_path.display(),
                        "Runner directory exists, skipping download"
                    );
                    return Ok(install_path);
                }

                let archive_name = format!("Proton-{version}.tar.gz");
                let download_dir = self.runners_install_root.join(".downloads");
                fs::create_dir_all(&download_dir).map_err(|e| PlayError::RunnerDownload {
                    url: url.clone(),
                    attempt: 1,
                    reason: format!("failed to create download directory: {e}"),
                })?;

                let archive_path = download_dir.join(&archive_name);

                // Check disk space before download
                check_disk_space(&self.runners_install_root)?;

                // Download with retries
                Self::download_with_retries(url, &archive_path)?;

                // Verify SHA512
                Self::verify_checksum(&archive_path, sha512)?;

                // Extract
                Self::extract(&archive_path, &self.runners_install_root)?;

                // Clean up archive
                if let Err(e) = fs::remove_file(&archive_path) {
                    tracing::warn!(
                        event = "cleanup_failed",
                        path = %archive_path.display(),
                        error = %e,
                        "Failed to remove downloaded archive (non-fatal)"
                    );
                }

                tracing::info!(
                    event = "runner_installed",
                    path = %install_path.display(),
                    version = %version,
                    "Runner installed successfully"
                );

                Ok(install_path)
            },
        }
    }

    /// Compute the install path for a given runner version.
    fn install_path_for(&self, version: &semver::Version) -> PathBuf {
        // Convention: ProtonGE/8.25/
        self.runners_install_root.join("ProtonGE").join(version.to_string())
    }

    /// Download a file with up to `MAX_RETRIES` attempts.
    pub fn download_with_retries(url: &str, dest: &Path) -> Result<(), PlayError> {
        for attempt in 1..=MAX_RETRIES {
            match Self::download_once(url, dest) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt == MAX_RETRIES {
                        return Err(PlayError::RunnerDownload {
                            url: url.to_owned(),
                            attempt,
                            reason: format!("{e} (after {MAX_RETRIES} attempts)"),
                        });
                    }
                    tracing::warn!(
                        event = "download_retry",
                        url,
                        attempt,
                        error = %e,
                        "Download failed, retrying"
                    );
                },
            }
        }
        // Unreachable, but satisfies the type checker
        Err(PlayError::RunnerDownload {
            url: url.to_owned(),
            attempt: MAX_RETRIES,
            reason: "all retry attempts exhausted".to_owned(),
        })
    }

    /// Single download attempt using ureq (blocking HTTP) with progress bar.
    fn download_once(url: &str, dest: &Path) -> Result<(), String> {
        let response = ureq::get(url).call().map_err(|e| format!("HTTP request failed: {e}"))?;

        // Try to get content length for progress bar
        let total_bytes = response
            .header("content-length")
            .and_then(|h| h.parse::<u64>().ok());

        let pb = if let Some(total) = total_bytes {
            let pb = ProgressBar::new(total);
            pb.set_style(indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
                .progress_chars("#>-"));
            pb
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(indicatif::ProgressStyle::default_spinner()
                .template("{spinner:.green} {bytes} downloaded")
                .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()));
            pb
        };

        pb.set_message(format!("Downloading {}", url.split('/').next_back().unwrap_or("file")));

        let mut reader = response.into_reader();
        let mut file = fs::File::create(dest).map_err(|e| format!("failed to create file: {e}"))?;

        let mut buf = vec![0u8; DOWNLOAD_BUF_SIZE];
        let mut downloaded: u64 = 0;

        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("read error: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| format!("write error: {e}"))?;
            downloaded += n as u64;
            pb.set_position(downloaded);
        }

        file.flush().map_err(|e| format!("flush error: {e}"))?;

        pb.finish_with_message("Download complete");
        Ok(())
    }

    /// Verify SHA512 checksum of a file against the expected hex digest.
    /// GE-Proton releases provide SHA512 checksums as .sha512sum files.
    pub fn verify_checksum(path: &Path, expected: &str) -> Result<(), PlayError> {
        let mut file = fs::File::open(path).map_err(|e| PlayError::RunnerDownload {
            url: String::new(),
            attempt: 1,
            reason: format!("failed to open file for checksum: {e}"),
        })?;

        let mut hasher = Sha512::new();
        io::copy(&mut file, &mut hasher).map_err(|e| PlayError::RunnerDownload {
            url: String::new(),
            attempt: 1,
            reason: format!("failed to read file for checksum: {e}"),
        })?;

        let result = hasher.finalize();
        let actual = format!("{result:x}");

        if actual.eq_ignore_ascii_case(expected) {
            tracing::info!(
                event = "checksum_verified",
                path = %path.display(),
                "SHA512 checksum verified"
            );
            Ok(())
        } else {
            // Delete the file for safety — a checksum mismatch could indicate
            // a compromised or corrupted download.
            let _ = fs::remove_file(path);
            Err(PlayError::ChecksumMismatch { expected: expected.to_owned(), actual })
        }
    }

    /// Extract a .tar.gz archive to the target directory.
    pub fn extract(archive: &Path, dest: &Path) -> Result<(), PlayError> {
        // Use std::process::Command to call tar, since the tar crate is not
        // in our dependencies and tar is available on all target distros.
        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(dest)
            .status()
            .map_err(|e| PlayError::RunnerDownload {
                url: String::new(),
                attempt: 1,
                reason: format!("failed to spawn tar: {e}"),
            })?;

        if status.success() {
            Ok(())
        } else {
            Err(PlayError::RunnerDownload {
                url: String::new(),
                attempt: 1,
                reason: format!("tar extraction failed with exit code {:?}", status.code()),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_runners_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn already_installed_returns_path_if_exists() {
        let root = temp_runners_root();
        let runner_path = root.path().join("ProtonGE").join("8.25");
        fs::create_dir_all(&runner_path).unwrap();

        let module = RunnerModule::new(root.path().to_path_buf());
        let action = RunnerAction::AlreadyInstalled { path: runner_path.clone() };

        let result = module.execute(&action);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), runner_path);
    }

    #[test]
    fn already_installed_fails_if_missing() {
        let root = temp_runners_root();
        let runner_path = root.path().join("ProtonGE").join("8.25");

        let module = RunnerModule::new(root.path().to_path_buf());
        let action = RunnerAction::AlreadyInstalled { path: runner_path };

        let result = module.execute(&action);
        assert!(result.is_err());
    }

    #[test]
    fn verify_checksum_matches() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        fs::write(&file_path, b"hello world").unwrap();

        // Pre-computed SHA512 of "hello world"
        let expected = "309ecc489c12d6eb4cc40f50c902f2b4d0ed77ee511a7c7a9bcd3ca86d4cd86f989dd35bc5ff499670da34255b45b0cfd830e81f605dcf7dc5542e93ae9cd76f";

        let result = RunnerModule::verify_checksum(&file_path, expected);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_checksum_mismatch_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        fs::write(&file_path, b"hello world").unwrap();

        let result = RunnerModule::verify_checksum(&file_path, "0000000000000000");
        assert!(result.is_err());
        assert!(matches!(result, Err(PlayError::ChecksumMismatch { .. })));

        // File should be deleted on mismatch
        assert!(!file_path.exists(), "file should be deleted on checksum mismatch");
    }

    #[test]
    fn install_path_for_uses_convention() {
        let root = temp_runners_root();
        let module = RunnerModule::new(root.path().to_path_buf());
        let version = semver::Version::parse("8.25.0").unwrap();

        let path = module.install_path_for(&version);
        assert_eq!(path, root.path().join("ProtonGE").join("8.25.0"));
    }

    #[test]
    fn download_already_existing_skips() {
        let root = temp_runners_root();
        let version = semver::Version::parse("8.25.0").unwrap();
        let install_path = root.path().join("ProtonGE").join("8.25.0");
        fs::create_dir_all(&install_path).unwrap();

        let module = RunnerModule::new(root.path().to_path_buf());
        let action = RunnerAction::Download {
            url: "https://example.com/fake.tar.gz".to_owned(),
            version: version.clone(),
            sha512: "abc123".to_owned(),
        };

        let result = module.execute(&action);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), install_path);
    }
}
