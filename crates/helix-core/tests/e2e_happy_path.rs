use helix_core::binary::pe::tests::build_minimal_pe32;
use helix_core::modules::detection::CommandRunner;
use helix_core::orchestrator::{Orchestrator, OrchestratorPhase};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[derive(Clone)]
struct MockRunner {
    responses: HashMap<String, Result<String, String>>,
}

impl CommandRunner for MockRunner {
    fn run_command(&self, program: &str, _args: &[&str]) -> Result<String, String> {
        self.responses.get(program).cloned().unwrap_or(Err("not mocked".into()))
    }
    fn clone_boxed(&self) -> Box<dyn CommandRunner> {
        Box::new(self.clone())
    }
}

#[test]
fn e2e_happy_path() {
    let tmp = TempDir::new().unwrap();
    let proc_root = tmp.path().join("proc");
    let sys_root = tmp.path().join("sys");
    let etc_root = tmp.path().join("etc");
    let state_root = tmp.path().join("state");
    let runners_root = tmp.path().join("runners");
    let prefix_root = tmp.path().join("prefixes");

    fs::create_dir_all(&proc_root).unwrap();
    fs::create_dir_all(&sys_root).unwrap();
    fs::create_dir_all(&etc_root).unwrap();
    fs::create_dir_all(&state_root).unwrap();
    fs::create_dir_all(&runners_root).unwrap();
    fs::create_dir_all(&prefix_root).unwrap();

    fs::write(proc_root.join("version"), "Linux version 6.18.13-200.fc43.x86_64").unwrap();
    let cpuinfo = fs::read_to_string("tests/fixtures/proc_cpuinfo_i7_13650hx").unwrap_or_default();
    if !cpuinfo.is_empty() {
        fs::write(proc_root.join("cpuinfo"), cpuinfo).unwrap();
    }
    fs::write(
        proc_root.join("meminfo"),
        "MemTotal: 16384000 kB\nMemAvailable: 8192000 kB\nSwapTotal: 4096000 kB\n",
    )
    .unwrap();
    fs::create_dir_all(proc_root.join("sys/vm")).ok();
    fs::write(proc_root.join("sys/vm/max_map_count"), "65536\n").ok();
    fs::create_dir_all(proc_root.join("sys/kernel")).ok();
    fs::write(proc_root.join("sys/kernel/split_lock_mitigate"), "1\n").ok();

    fs::create_dir_all(sys_root.join("class/power_supply/BAT0")).unwrap();
    fs::write(sys_root.join("class/power_supply/BAT0/present"), "1\n").unwrap();
    fs::create_dir_all(sys_root.join("class/dmi/id")).unwrap();
    fs::write(sys_root.join("class/dmi/id/chassis_type"), "10\n").unwrap();
    fs::create_dir_all(sys_root.join("kernel/mm/transparent_hugepage")).unwrap();
    fs::write(sys_root.join("kernel/mm/transparent_hugepage/enabled"), "always [madvise] never\n")
        .unwrap();

    fs::write(
        etc_root.join("os-release"),
        "ID=fedora\nVERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43\"\n",
    )
    .unwrap();

    let mut responses = HashMap::new();
    responses.insert("pkexec".to_string(), Ok(String::new()));
    responses.insert("wine".to_string(), Ok("wine-9.0".to_string()));
    responses.insert("gamemoded".to_string(), Ok("gamemoded 1.8".to_string()));
    responses.insert("vulkaninfo".to_string(), Ok("apiVersion = 1.3.0".to_string()));
    let mock = MockRunner { responses };

    let pe_bytes = build_minimal_pe32(&["KERNEL32.dll"]);
    let exe_path = tmp.path().join("game.exe");
    fs::write(&exe_path, &pe_bytes).unwrap();

    // runner must be an executable file at the install path
    let runner_path = runners_root.join("ProtonCachyOS").join("11.0.0");
    if let Some(parent) = runner_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&runner_path, "#!/bin/sh\nsleep 30\n").unwrap();
    let mut perms = fs::metadata(&runner_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&runner_path, perms).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(&pe_bytes);
    let hash = format!("{:x}", hasher.finalize());
    let prefix_dir = prefix_root.join(&hash);
    fs::create_dir_all(&prefix_dir).unwrap();
    fs::create_dir_all(prefix_dir.join("drive_c")).unwrap();
    fs::write(prefix_dir.join("system.reg"), "").unwrap();

    let mut orch = Orchestrator::new_with_roots(
        proc_root.clone(),
        sys_root.clone(),
        etc_root.clone(),
        state_root.clone(),
        runners_root.clone(),
        prefix_root.clone(),
        Box::new(mock),
    );
    orch.set_confirm_callback(Box::new(|_| true));

    let res = orch.run(&exe_path);
    assert!(res.is_ok(), "run failed: {:?}", res);
    assert_eq!(orch.phase(), OrchestratorPhase::Validated);
}
