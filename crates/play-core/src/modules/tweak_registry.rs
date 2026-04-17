#![allow(clippy::pedantic)]
/// The TweakRegistry: all system tweaks as compile-time static data.
///
/// DecisionEngine iterates this registry to produce TweakDecisions.
/// There is no match-on-tweak-name in business logic — the registry
/// encapsulates all applicability constraints here, as pure data.
use crate::models::environment::GpuVendor;
use crate::models::plan::{RiskLevel, TweakClass, TweakConstraint, TweakId};

/// Returns the complete static tweak registry.
/// Every known system tweak is represented exactly once.
pub fn all() -> &'static [TweakConstraint] {
    TWEAKS
}

static TWEAKS: &[TweakConstraint] = &[
    // -----------------------------------------------------------------------
    // Class A — session-scoped, Drop-restored, no confirmation needed
    // -----------------------------------------------------------------------
    TweakConstraint {
        id: TweakId::Fsync,
        class: TweakClass::A,
        min_ram_mb: None,
        kernel_version_min: Some((5, 16)), // has_futex2
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Low,
        rationale: "Kernel >= 5.16 has futex2 syscall; fsync outperforms esync for multi-threaded games.",
    },
    TweakConstraint {
        id: TweakId::Esync,
        class: TweakClass::A,
        min_ram_mb: None,
        kernel_version_min: None, // fallback when futex2 unavailable
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Low,
        rationale: "Esync reduces context switches for games with many synchronisation objects; requires ulimit nofile.",
    },
    TweakConstraint {
        id: TweakId::DxvkAsync,
        class: TweakClass::A,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Low,
        rationale: "Async shader compilation eliminates hitches during first-run traversal.",
    },
    TweakConstraint {
        id: TweakId::CpuGovernorPerformance,
        class: TweakClass::A,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: true, // Spec §10: "NOT laptop, NOT battery" — thermal risk on laptops
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Low,
        rationale: "Performance governor eliminates frequency scaling latency during gameplay; restored on exit. Desktop only — thermal risk on laptops.",
    },
    TweakConstraint {
        id: TweakId::GameMode,
        class: TweakClass::A,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Low,
        rationale: "GameMode applies CPU scheduler hints and inhibits power management during gameplay.",
    },

    // -----------------------------------------------------------------------
    // Class B — persistent, rollback manifest first, confirm once per game
    // -----------------------------------------------------------------------
    TweakConstraint {
        id: TweakId::VmMaxMapCount,
        class: TweakClass::B,
        min_ram_mb: None, // always beneficial; value is hardware-proportional
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: true, // sysctl resets on reboot
        risk_level: RiskLevel::Low,
        rationale: "Many modern games exceed the default map count (65536); hardware-proportional target avoids OOM.",
    },
    TweakConstraint {
        id: TweakId::ThpMadvise,
        class: TweakClass::B,
        min_ram_mb: Some(16_384), // Spec §10: "RAM >= 16GB"
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: true, // THP mode resets on reboot
        risk_level: RiskLevel::Low,
        rationale: "THP=madvise reduces TLB pressure for large game heaps without the system-wide cost of always.",
    },
    TweakConstraint {
        id: TweakId::SchedAutogroup,
        class: TweakClass::B,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: true, // sysctl resets on reboot
        risk_level: RiskLevel::Low,
        rationale: "Disabling autogroup isolates the game's scheduler group, preventing desktop processes from stealing time slices.",
    },
    TweakConstraint {
        id: TweakId::SplitLockMitigate,
        class: TweakClass::B,
        min_ram_mb: None,
        kernel_version_min: Some((5, 7)),
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None, // Spec §10: "x86 arch only" — split_lock is an x86-family concern (both X86 and X86_64). All supported distros are x86-family; set to None until ARM support is added.
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: true, // sysctl resets on reboot
        risk_level: RiskLevel::Low,
        rationale: "Disabling split-lock mitigation eliminates stalls from Windows game code using unaligned atomics.",
    },
    TweakConstraint {
        id: TweakId::UlimitNofile,
        class: TweakClass::B,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: None,
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: false,
        reversible: true,
        reboot_required: false,
        reboot_resets: true, // limits.d resets on reboot
        risk_level: RiskLevel::Low,
        rationale: "Esync requires a high file descriptor limit (524288); written to /etc/security/limits.d/.",
    },

    // -----------------------------------------------------------------------
    // Class C — NVIDIA desktop-only, explicit user consent required
    // -----------------------------------------------------------------------
    TweakConstraint {
        id: TweakId::NvidiaPersistenceMode,
        class: TweakClass::C,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: Some(GpuVendor::NVIDIA),
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: true, // is_laptop = true → NotApplicable
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Medium,
        rationale: "Persistence mode keeps the NVIDIA kernel module loaded, eliminating first-launch GPU init latency.",
    },
    TweakConstraint {
        id: TweakId::NvidiaClockLock,
        class: TweakClass::C,
        min_ram_mb: None,
        kernel_version_min: None,
        gpu_vendor_required: Some(GpuVendor::NVIDIA),
        gpu_vendor_exclusions: vec![],
        cpu_arch_required: None,
        requires_desktop: true, // thermal risk on laptops — never apply
        reversible: true,
        reboot_required: false,
        reboot_resets: false,
        risk_level: RiskLevel::Medium,
        rationale: "Clock lock at 95% of VBIOS max eliminates boost-induced frequency instability without thermal risk.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_all_tweak_ids() {
        use TweakId::*;
        let all_ids = [
            VmMaxMapCount,
            ThpMadvise,
            SchedAutogroup,
            SplitLockMitigate,
            UlimitNofile,
            CpuGovernorPerformance,
            NvidiaPersistenceMode,
            NvidiaClockLock,
            DxvkAsync,
            Fsync,
            Esync,
            GameMode,
        ];
        let registry = all();
        for id in all_ids {
            assert!(
                registry.iter().any(|c| c.id == id),
                "TweakId::{id:?} missing from registry"
            );
        }
    }

    #[test]
    fn class_c_tweaks_are_desktop_only() {
        for c in all() {
            if c.class == TweakClass::C {
                assert!(
                    c.requires_desktop,
                    "Class C tweak {:?} must have requires_desktop = true",
                    c.id
                );
                assert!(
                    c.gpu_vendor_required == Some(GpuVendor::NVIDIA),
                    "Class C tweak {:?} must require NVIDIA GPU in v1",
                    c.id
                );
            }
        }
    }

    #[test]
    fn all_tweaks_have_rationale() {
        for c in all() {
            assert!(
                !c.rationale.is_empty(),
                "TweakId::{:?} has empty rationale",
                c.id
            );
        }
    }
}
