//! Shell completion generation for play CLI.

use clap::CommandFactory;
use clap_complete::{generate, Shell};

/// Generate shell completions for the given shell.
pub fn generate_completion(shell: Shell, cmd: &clap::Command) -> String {
    let mut buffer = Vec::new();
    generate(shell, cmd, "play", &mut buffer);
    String::from_utf8(buffer).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_completion() {
        let cmd = crate::CliArgs::command();
        let completion = generate_completion(Shell::Bash, &cmd);
        assert!(!completion.is_empty());
        assert!(completion.contains("play"));
    }

    #[test]
    fn test_zsh_completion() {
        let cmd = crate::CliArgs::command();
        let completion = generate_completion(Shell::Zsh, &cmd);
        assert!(!completion.is_empty());
    }

    #[test]
    fn test_fish_completion() {
        let cmd = crate::CliArgs::command();
        let completion = generate_completion(Shell::Fish, &cmd);
        assert!(!completion.is_empty());
    }
}
