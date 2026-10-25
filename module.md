# /module — Scaffold New Module

Create a new module: $ARGUMENTS

Every module in this codebase follows strict contracts. This command generates
the correct scaffolding so that architecture rules are never accidentally violated.

## Module Architecture Rules (enforce all of these)

1. **Single public entry function** — one function that takes typed input, returns typed output
2. **No direct I/O** — inject `sys_root: &Path` for all /proc /sys paths, `run_cmd: &dyn CommandRunner` for subprocesses
3. **No side effects outside the module's domain** — detection reads only, execution writes only what it's responsible for
4. **No inter-module calls** — the Orchestrator dispatches; modules never call each other
5. **Every error is typed** — `Result<Output, PlayError>`, no anyhow, no String errors
6. **Testable in isolation** — can be unit-tested with no system access

## Step 1: Define the Module Contract

```rust
// src/{module_name}/mod.rs

/// Input type: everything this module needs
pub struct {ModuleName}Input {
    // ...
    pub sys_root: PathBuf,                      // injected for /proc /sys access
    pub run_cmd: Box<dyn CommandRunner>,         // injected for subprocess calls
}

/// Output type: everything this module produces
pub struct {ModuleName}Output {
    // all fields typed and documented
}

/// Public entry point — the only thing the Orchestrator calls
pub fn run_{module_name}(input: {ModuleName}Input) -> Result<{ModuleName}Output, PlayError> {
    // ...
}
```

## Step 2: CommandRunner Trait (if needed)

```rust
// For testability: real impl spawns processes, mock impl returns fixtures
pub trait CommandRunner: Send + Sync {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput, PlayError>;
}

pub struct SystemCommandRunner;
impl CommandRunner for SystemCommandRunner { /* ... */ }

// In tests:
pub struct MockCommandRunner { pub responses: HashMap<String, CommandOutput> }
impl CommandRunner for MockCommandRunner { /* return fixture responses */ }
```

## Step 3: SysPath Helper

```rust
// Never hardcode /proc or /sys paths. Always use:
fn sysfs_path(root: &Path, relative: &str) -> PathBuf {
    root.join(relative.trim_start_matches('/'))
}
// In tests: pass a tempdir as sys_root
// In prod: pass PathBuf::from("/")
```

## Step 4: tracing Instrumentation

Add structured tracing at key decision points:
```rust
tracing::info!(event = "{module}_started", input_hash = %input.identity.exe_hash);
tracing::debug!(event = "{step}_completed", value = ?result);
tracing::warn!(event = "{condition}_not_met", detail = %reason);
```

## Step 5: Test Scaffolding

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture_input(tmp: &TempDir) -> {ModuleName}Input {
        // create minimal sysfs structure in tmp
        // provide mock CommandRunner with expected responses
        {ModuleName}Input {
            sys_root: tmp.path().to_owned(),
            run_cmd: Box::new(MockCommandRunner::default()),
        }
    }

    #[test]
    fn test_happy_path() { /* ... */ }

    #[test]
    fn test_missing_sysfs_entry_returns_typed_error() { /* ... */ }

    #[test]
    fn test_idempotent_when_called_twice() { /* ... */ }
}
```

Generate the complete file, not just the outline. Place it at `src/{module_name}/mod.rs`.
