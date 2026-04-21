use std::env;
use std::fs;
use std::process::ExitCode;

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
    write_to_file(&full_path, value)
}

fn write_file(path: &str, content: &str) -> ExitCode {
    write_to_file(path, content)
}

fn write_to_file(path: &str, content: &str) -> ExitCode {
    if let Err(e) = fs::write(path, content) {
        eprintln!("Failed to write to {}: {}", path, e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
