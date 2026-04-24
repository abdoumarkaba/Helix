use std::env;
use std::fs;
use std::process::ExitCode;

/// Whitelist of allowed sysfs paths for security.
/// Only these paths can be written to via play-helper.
const SYSFS_WHITELIST: &[&str] = &[
    // Transparent Huge Pages configuration
    "/sys/kernel/mm/transparent_hugepage/enabled",
    "/sys/kernel/mm/transparent_hugepage/defrag",
    // GPU power management (NVIDIA)
    "/sys/bus/pci/drivers/nvidia/bind",
    "/sys/bus/pci/drivers/nvidia/unbind",
    // CPU governor (handled via files in /sys/devices/system/cpu/)
    // Note: These are typically per-CPU, so we allow the pattern
];

/// Whitelist of allowed sysctl keys for security.
/// Only these keys can be written to via play-helper.
const SYSCTL_WHITELIST: &[&str] = &[
    "vm.max_map_count",
    "kernel.sched_autogroup",
    "kernel.split_lock_mitigate",
    "fs.file-max",
    "vm.swappiness",
    "vm.dirty_ratio",
    "vm.dirty_background_ratio",
];

/// Whitelist of allowed file paths for write-file command.
const FILE_WHITELIST: &[&str] = &[
    "/etc/security/limits.d/play.conf",
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: play-helper <subcommand> [args...]");
        eprintln!("Subcommands:");
        eprintln!("  sysctl-write <key> <value>   - Write to /proc/sys/<key>");
        eprintln!("  sysfs-write <path> <value>   - Write to /sys/<path>");
        eprintln!("  write-file <path> <content>  - Write content to file");
        return ExitCode::FAILURE;
    }

    match args[1].as_str() {
        "sysctl-write" => {
            if args.len() != 4 {
                eprintln!("Usage: play-helper sysctl-write <key> <value>");
                return ExitCode::FAILURE;
            }
            let key = &args[2];
            let value = &args[3];
            sysctl_write(key, value)
        },
        "sysfs-write" => {
            if args.len() != 4 {
                eprintln!("Usage: play-helper sysfs-write <path> <value>");
                return ExitCode::FAILURE;
            }
            let path = &args[2];
            let value = &args[3];
            sysfs_write(path, value)
        },
        "write-file" => {
            if args.len() != 4 {
                eprintln!("Usage: play-helper write-file <path> <content>");
                return ExitCode::FAILURE;
            }
            let path = &args[2];
            let content = &args[3];
            write_file(path, content)
        },
        _ => {
            eprintln!("Unknown subcommand: {}", args[1]);
            return ExitCode::FAILURE;
        },
    }
}

fn sysctl_write(key: &str, value: &str) -> ExitCode {
    // Security check: only allow whitelisted keys
    if !SYSCTL_WHITELIST.contains(&key) {
        eprintln!("Error: sysctl key '{}' is not in the whitelist", key);
        eprintln!("Allowed keys: {:?}", SYSCTL_WHITELIST);
        return ExitCode::FAILURE;
    }

    // Convert "vm.max_map_count" to "/proc/sys/vm/max_map_count"
    let path = format!("/proc/sys/{}", key.replace('.', "/"));
    write_to_file(&path, value)
}

fn sysfs_write(path: &str, value: &str) -> ExitCode {
    // Ensure path starts with /sys/
    let full_path = if path.starts_with("/sys/") {
        path.to_string()
    } else {
        format!("/sys/{}", path.trim_start_matches('/'))
    };

    // Security check: only allow whitelisted paths (or CPU governor patterns)
    let is_allowed = SYSFS_WHITELIST.contains(&full_path.as_str())
        || is_cpu_governor_path(&full_path);

    if !is_allowed {
        eprintln!("Error: sysfs path '{}' is not in the whitelist", full_path);
        eprintln!("Allowed paths: {:?}", SYSFS_WHITELIST);
        return ExitCode::FAILURE;
    }

    write_to_file(&full_path, value)
}

/// Check if path is a CPU governor path (allowed pattern for per-CPU control).
/// Format: /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
fn is_cpu_governor_path(path: &str) -> bool {
    path.contains("/sys/devices/system/cpu/")
        && path.contains("/cpufreq/scaling_governor")
}

fn write_file(path: &str, content: &str) -> ExitCode {
    // Security check: only allow whitelisted paths
    if !FILE_WHITELIST.contains(&path) {
        eprintln!("Error: file path '{}' is not in the whitelist", path);
        eprintln!("Allowed paths: {:?}", FILE_WHITELIST);
        return ExitCode::FAILURE;
    }

    write_to_file(path, content)
}

fn write_to_file(path: &str, content: &str) -> ExitCode {
    if let Err(e) = fs::write(path, content) {
        eprintln!("Failed to write to {}: {}", path, e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
