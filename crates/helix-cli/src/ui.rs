#![allow(dead_code)]

//! Small presentation helpers for the Helix CLI.
//!
//! Keeps styling consistent across commands. All helpers are pure (no I/O)
//! except `spinner()` which returns a configured progress bar.

use std::time::Duration;

use console::{measure_text_width, style, Term};
use indicatif::{ProgressBar, ProgressStyle};

/// Accent color used for brand and section titles.
pub const ACCENT: &str = "magenta";

/// Render the brand wordmark: `helix ▸`.
pub fn brand() -> String {
    format!("{}", style("helix ▸").bold().magenta())
}

/// A thin dim horizontal rule capped at 72 columns.
pub fn rule() -> String {
    let width = Term::stdout().size().1 as usize;
    let width = if width == 0 { 72 } else { width.clamp(20, 72) };
    style("─".repeat(width)).dim().to_string()
}

/// A section header (accent, bold).
pub fn section(name: &str) -> String {
    format!("{}", style(name).bold().magenta())
}

/// A lightweight rounded card. Falls back to flat lines on narrow terminals.
pub fn card<T, L>(title: &str, lines: L) -> String
where
    T: AsRef<str>,
    L: AsRef<[T]>,
{
    let lines = lines.as_ref();
    let term_width = Term::stdout().size().1 as usize;
    let width = if term_width == 0 { 80 } else { term_width };

    // Narrow terminal fallback: flat lines.
    if width < 60 {
        let mut out = String::new();
        out.push_str(&format!("{}", style(title).bold().magenta()));
        out.push('\n');
        for line in lines {
            out.push_str(&format!("  {}\n", line.as_ref()));
        }
        return out.trim_end_matches('\n').to_string();
    }

    let w = width.min(72);
    let title_len = measure_text_width(title);
    let fill = if w >= title_len + 5 { w - title_len - 5 } else { 0 };

    let mut out = String::new();
    out.push_str(&format!("╭─ {} {}╮\n", style(title).bold().magenta(), "─".repeat(fill)));

    let inner = w.saturating_sub(4);
    for line in lines {
        let text = line.as_ref();
        let tw = measure_text_width(text);
        if tw <= inner {
            let pad = inner - tw;
            out.push_str(&format!("│ {}{} │\n", text, " ".repeat(pad)));
        } else {
            // Truncate to fit (best-effort, keeps box intact).
            let mut acc = String::new();
            for ch in text.chars() {
                let mut test = acc.clone();
                test.push(ch);
                if measure_text_width(&test) > inner.saturating_sub(1) {
                    break;
                }
                acc.push(ch);
            }
            acc.push('…');
            let tw2 = measure_text_width(&acc);
            let pad = inner.saturating_sub(tw2);
            out.push_str(&format!("│ {}{} │\n", acc, " ".repeat(pad)));
        }
    }

    out.push_str(&format!("╰{}╯", "─".repeat(w.saturating_sub(2))));
    out.trim_end_matches('\n').to_string()
}

/// Aligned key/value row. Label column is 14 chars.
pub fn kv(label: &str, value: impl std::fmt::Display) -> String {
    format!("  {:<14} {}", style(label).dim(), value)
}

/// Status line: success.
pub fn status_ok(msg: &str) -> String {
    format!("  {}  {}", style("✓").green().bold(), msg)
}

/// Status line: warning.
pub fn status_warn(msg: &str) -> String {
    format!("  {}  {}", style("⚠").yellow().bold(), msg)
}

/// Status line: error.
pub fn status_err(msg: &str) -> String {
    format!("  {}  {}", style("✗").red().bold(), msg)
}

/// Status line: info/progress.
pub fn status_info(msg: &str) -> String {
    format!("  {}  {}", style("›").magenta().bold(), msg)
}

// Backwards-friendly aliases matching the previous helper names.
pub fn ok(msg: &str) -> String {
    status_ok(msg)
}
pub fn warn(msg: &str) -> String {
    status_warn(msg)
}
pub fn err(msg: &str) -> String {
    status_err(msg)
}
pub fn info(msg: &str) -> String {
    status_info(msg)
}

/// A braille spinner styled with the accent color.
pub fn spinner() -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.magenta} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Spinner with an initial message.
pub fn spinner_msg(message: &str) -> ProgressBar {
    let pb = spinner();
    pb.set_message(message.to_string());
    pb
}

/// ProgressStyle variant for callers that manage their own bar.
pub fn spinner_style() -> ProgressStyle {
    ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠧⠇⠏")
        .template("{spinner:.magenta} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
}
