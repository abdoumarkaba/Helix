#![allow(clippy::pedantic)]
//! Hardcoded runner manifest for local functionality.
//!
//! This module provides a default set of ProtonGE runners without requiring
//! an external play-db repository. The SHA512 checksums are placeholders that
//! will be verified during download.

use crate::models::environment::RunnerType;
use crate::models::plan::{RunnerRelease, RunnersManifest};

/// Get the default runners manifest for local use.
///
/// Returns a manifest with recent stable ProtonGE releases.
/// SHA512 checksums are placeholders and will be verified during download.
pub fn get_default_manifest() -> RunnersManifest {
    RunnersManifest {
        runners: vec![
            RunnerRelease {
                runner_type: RunnerType::ProtonCachyOS,
                version: semver::Version::new(11, 0, 0),
                url: "https://github.com/CachyOS/proton-cachyos/releases/download/cachyos-11.0-20260429-slr/proton-cachyos-11.0-20260429-slr-x86_64.tar.gz".to_string(),
                sha512: "PLACEHOLDER_PROTON_CACHYOS_11_0_SHA512".to_string(),
            },
            RunnerRelease {
                runner_type: RunnerType::ProtonGE,
                version: semver::Version::new(9, 27, 0),
                url: "https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton9-27/GE-Proton9-27.tar.gz".to_string(),
                sha512: "PLACEHOLDER_GE_PROTON_9_27_SHA512".to_string(),
            },
            RunnerRelease {
                runner_type: RunnerType::ProtonGE,
                version: semver::Version::new(11, 11, 0),
                url: "https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-11/GE-Proton11-11-x86_64.tar.gz".to_string(),
                sha512: "45721a7b625b2404d8896bb5e2a8b87fc9623a394873a6dd6b900c839681d5c01ec694ca00d003fc9c733c5f6c5648af8171985656f9fa47e20999fbebdbe235".to_string(),
            },
        ],
    }
}
