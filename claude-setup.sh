#!/usr/bin/env bash
# =============================================================================
# play — Claude Code Dev Environment Setup
# Run from ~/play directory: bash scripts/claude-setup.sh
# Takes ~10-20 minutes on first run (Rust tool compilation)
# =============================================================================

set -euo pipefail

PLAY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PLAY_DIR"

# Colors
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

step()  { echo -e "\n${CYAN}${BOLD}▶ $1${NC}"; }
ok()    { echo -e "  ${GREEN}✓${NC} $1"; }
warn()  { echo -e "  ${YELLOW}⚠${NC} $1"; }
info()  { echo -e "  ${BLUE}→${NC} $1"; }
fail()  { echo -e "  ${RED}✗${NC} $1"; exit 1; }

echo -e "${BOLD}"
echo "  ┌─────────────────────────────────────────────┐"
echo "  │   play — Claude Code dev environment setup  │"
echo "  │   Fedora 43 · Rust 2021 · Claude Code 2.x   │"
echo "  └─────────────────────────────────────────────┘"
echo -e "${NC}"

# =============================================================================
# 1. VERIFY PREREQUISITES
# =============================================================================
step "Verifying prerequisites"

command -v rustup >/dev/null 2>&1 || fail "rustup not found. Install from https://rustup.rs"
command -v cargo  >/dev/null 2>&1 || fail "cargo not found. Check your rustup installation."
command -v git    >/dev/null 2>&1 || fail "git not found. Install with: sudo dnf install git"
command -v claude >/dev/null 2>&1 || warn "claude (Claude Code) not found in PATH. Install from https://claude.ai/code"

RUST_VERSION=$(rustc --version | awk '{print $2}')
info "Rust version: $RUST_VERSION"
info "Claude Code: $(claude --version 2>/dev/null || echo 'not found')"

# Check MSRV: 1.75
MAJOR=$(echo "$RUST_VERSION" | cut -d. -f1)
MINOR=$(echo "$RUST_VERSION" | cut -d. -f2)
if [[ "$MAJOR" -lt 1 ]] || [[ "$MAJOR" -eq 1 && "$MINOR" -lt 75 ]]; then
    warn "Rust $RUST_VERSION < MSRV 1.75. Updating..."
    rustup update stable
fi
ok "Rust version OK"

# =============================================================================
# 2. RUST TOOLCHAIN COMPONENTS
# =============================================================================
step "Installing Rust toolchain components"

rustup component add rustfmt clippy rust-analyzer rust-src 2>/dev/null || true
ok "rustfmt, clippy, rust-analyzer, rust-src added"

# Cross-compilation targets for CI matrix distros
# (comment out if disk space is tight — only needed for cross-distro CI)
# rustup target add x86_64-unknown-linux-musl 2>/dev/null || true
# ok "musl target added (for static binary CI builds)"

# =============================================================================
# 3. CARGO TOOLS (critical for this project)
# =============================================================================
step "Installing cargo tools (this takes a while on first run)"

install_if_missing() {
    local bin="$1"; local crate="$2"; local extra="${3:-}"
    if ! command -v "$bin" >/dev/null 2>&1; then
        info "Installing $crate..."
        # shellcheck disable=SC2086
        cargo install "$crate" $extra --locked 2>/dev/null || \
        cargo install "$crate" $extra 2>/dev/null || \
        warn "Failed to install $crate — skipping (non-critical)"
    else
        ok "$bin already installed"
    fi
}

# nextest: much faster test runner, better output, shows failing tests clearly
install_if_missing cargo-nextest cargo-nextest

# cargo-watch: auto-rerun on file change (cargo watch -x test)
install_if_missing cargo-watch cargo-watch

# cargo-expand: expands proc macros — essential for debugging serde/thiserror derives
install_if_missing cargo-expand cargo-expand

# cargo-audit: check for known CVEs in dependencies
install_if_missing cargo-audit cargo-audit

# cargo-outdated: see which deps have new versions
install_if_missing cargo-outdated cargo-outdated

# cargo-deny: license and duplicate dependency checking
install_if_missing cargo-deny cargo-deny

# flamegraph: profiling (for performance debugging)
# install_if_missing flamegraph flamegraph  # uncomment if you want profiling

# tokei: code line counts (useful for tracking project size)
install_if_missing tokei tokei

ok "Cargo tools installed"

# =============================================================================
# 4. SYSTEM TOOLS (Fedora / dnf)
# =============================================================================
step "Checking system tools"

check_or_suggest() {
    local cmd="$1"; local pkg="$2"
    if command -v "$cmd" >/dev/null 2>&1; then
        ok "$cmd available"
    else
        warn "$cmd not found. Install with: sudo dnf install $pkg"
    fi
}

check_or_suggest rg      ripgrep       # fast code search
check_or_suggest fd      fd-find       # fast file find (used in watch scripts)
check_or_suggest jq      jq            # JSON parsing (for play-helper JSON interface)
check_or_suggest tree    tree          # directory visualization
check_or_suggest tokei   ""            # installed via cargo above

# These are needed by the tool itself at runtime (not just dev):
check_or_suggest wine        wine          # needed for testing Wine prefix creation
check_or_suggest nvidia-smi  ""            # ships with NVIDIA driver

# =============================================================================
# 5. CLAUDE CODE CONFIGURATION
# =============================================================================
step "Setting up Claude Code configuration"

mkdir -p .claude/commands

# settings.json — already created separately, copy if it doesn't exist
if [[ ! -f ".claude/settings.json" ]]; then
    warn ".claude/settings.json not found — copy it from the setup package"
else
    ok ".claude/settings.json present"
fi

# Verify custom commands are present
for cmd in plan tweak module review detect db-entry; do
    if [[ -f ".claude/commands/${cmd}.md" ]]; then
        ok "Custom command /${cmd} present"
    else
        warn ".claude/commands/${cmd}.md missing — copy it from the setup package"
    fi
done

# CLAUDE.md at project root
if [[ -f "CLAUDE.md" ]]; then
    ok "CLAUDE.md present ($(wc -l < CLAUDE.md) lines)"
else
    warn "CLAUDE.md not found at project root — copy it from the setup package"
fi

# .claudeignore — tell Claude Code which files to skip for context
cat > .claudeignore << 'EOF'
# Build artifacts — never include in context
target/
**/*.rlib
**/*.rmeta
**/*.d

# Large data files
play-db/
**/*.exe
tests/fixtures/binaries/*.exe

# IDE files
.idea/
.vscode/
**/*.swp

# Generated docs
docs/book/

# Lock files (include Cargo.lock but not others)
package-lock.json
yarn.lock

# Logs and state
**/*.log
**/*.crash
~/.local/share/play/
EOF
ok ".claudeignore created"

# =============================================================================
# 6. GIT CONFIGURATION
# =============================================================================
step "Configuring git hooks"

mkdir -p .git/hooks

# pre-commit: enforce fmt and clippy before any commit
cat > .git/hooks/pre-commit << 'HOOK'
#!/usr/bin/env bash
# play pre-commit hook: fmt check + clippy
# Installed by scripts/claude-setup.sh

set -euo pipefail

echo "→ Checking formatting..."
if ! cargo fmt -- --check 2>/dev/null; then
    echo "✗ Formatting issues found. Run: cargo fmt"
    echo "  Or stage formatted files: cargo fmt && git add -u"
    exit 1
fi
echo "✓ Format OK"

echo "→ Running clippy..."
if ! cargo clippy -- -D warnings 2>/dev/null; then
    echo "✗ Clippy warnings found. Fix them before committing."
    echo "  Hint: cargo clippy --fix for auto-fixable lints"
    exit 1
fi
echo "✓ Clippy OK"

echo "✓ Pre-commit checks passed"
HOOK
chmod +x .git/hooks/pre-commit
ok "pre-commit hook installed (fmt + clippy)"

# commit-msg: enforce conventional commit format
cat > .git/hooks/commit-msg << 'HOOK'
#!/usr/bin/env bash
# Enforce: feat(scope): description | fix(scope): description | etc.
MSG_FILE="$1"
MSG=$(cat "$MSG_FILE")
PATTERN='^(feat|fix|test|docs|refactor|perf|chore|ci|style)\([a-z_-]+\): .{10,}'
if ! echo "$MSG" | grep -qE "$PATTERN"; then
    echo "✗ Commit message doesn't match conventional format."
    echo "  Required: type(scope): description (min 10 chars)"
    echo "  Types: feat fix test docs refactor perf chore ci style"
    echo "  Example: feat(detection): implement NVIDIA GPU clock detection via nvidia-smi"
    exit 1
fi
HOOK
chmod +x .git/hooks/commit-msg
ok "commit-msg hook installed (conventional commits)"

# =============================================================================
# 7. CARGO CONFIGURATION
# =============================================================================
step "Writing Cargo configuration"

mkdir -p .cargo
cat > .cargo/config.toml << 'EOF'
[build]
# Fail on any warning in CI and locally. No warning debt.
rustflags = ["-D", "warnings"]

[alias]
# Quick commands used constantly during development
t   = "nextest run"           # cargo t — fast test runner
ta  = "nextest run --no-fail-fast"  # run all, don't stop on first failure
c   = "clippy -- -D warnings"      # cargo c — quick lint
w   = "watch -x 'nextest run'"     # cargo w — watch + test on save
b   = "build"
cb  = "check"                      # faster than build, just type-checks
ex  = "expand"                     # expand macros (requires cargo-expand)
doc = "doc --no-deps --open"       # open docs in browser

[profile.dev]
# Faster incremental builds during development
opt-level = 0
debug = true
incremental = true

[profile.release]
# Production binary: maximize performance
opt-level = 3
lto = "thin"
codegen-units = 1
strip = "symbols"

[profile.test]
# Tests: optimize enough to run fast, keep debug info
opt-level = 1
debug = true
EOF
ok ".cargo/config.toml written (aliases: t, ta, c, w, b, cb, ex, doc)"

# =============================================================================
# 8. RUSTFMT CONFIGURATION
# =============================================================================
step "Writing rustfmt configuration"

cat > rustfmt.toml << 'EOF'
edition = "2021"

# Max line width: 100 (wider than default 80, but not too wide for terminals)
max_width = 100

# Imports: merge, group, sort
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
reorder_imports = true

# Trailing commas in multi-line structures (makes diffs cleaner)
trailing_comma = "Vertical"

# Consistent match arm braces
match_arm_blocks = true

# Always use field init shorthand: MyStruct { field } not MyStruct { field: field }
use_field_init_shorthand = true

# Force where clauses to their own line when they get long
where_single_line = false
EOF
ok "rustfmt.toml written (100 cols, merged imports, trailing commas)"

# =============================================================================
# 9. CLIPPY CONFIGURATION
# =============================================================================
step "Writing clippy configuration"

cat > .clippy.toml << 'EOF'
# Clippy configuration for play

# Warn on large enum variants (catch accidental boxing issues)
enum-variant-size-threshold = 200

# Cognitive complexity limit (lower = stricter)
cognitive-complexity-threshold = 20

# Disallow type complexity beyond this threshold
type-complexity-threshold = 250
EOF

# Also write deny.toml for cargo-deny (license and security checks)
cat > deny.toml << 'EOF'
[licenses]
# Only allow licenses compatible with GPL-3.0
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
    "GPL-3.0",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "CC0-1.0",
    "Zlib",
]
unlicensed = "deny"
copyleft = "warn"

[advisories]
# Reject dependencies with known CVEs
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"
notice = "warn"

[bans]
# Prevent multiple versions of critical crates (causes binary bloat)
multiple-versions = "warn"
deny = [
    # Never use anyhow for user-facing errors (use typed PlayError)
    { name = "anyhow" },
    # Use indexmap not std HashMap where order matters
]
EOF
ok ".clippy.toml written, deny.toml written (license + security checks)"

# =============================================================================
# 10. SHELL ALIASES
# =============================================================================
step "Writing shell aliases"

ALIAS_FILE="$HOME/.bashrc.d/play-dev.sh"
mkdir -p "$HOME/.bashrc.d"

cat > "$ALIAS_FILE" << 'EOF'
# =============================================================================
# play — Development Aliases
# Source: scripts/claude-setup.sh
# =============================================================================

# Claude Code model shortcuts
# Use Sonnet (default) for complex work, Haiku for mechanical tasks
alias claude-sonnet='claude --model claude-sonnet-4-6'
alias claude-haiku='claude --model claude-haiku-4-5-20251001'

# Quick claude code shortcuts for this project
alias cc='claude'
alias cch='claude --model claude-haiku-4-5-20251001'    # cheap: formatting, renaming
alias ccs='claude --model claude-sonnet-4-6'             # balanced: default for play

# Cost-conscious one-liners:
# "Do this small mechanical thing cheaply"
alias cc-cheap='claude --model claude-haiku-4-5-20251001 --print'
# "One-shot print output without interactive session"  
alias cc-print='claude --print'

# Cargo shortcuts (complement .cargo/config.toml aliases)
alias ct='cargo nextest run'                    # fast test run
alias ctw='cargo watch -x "nextest run"'        # watch + test
alias cta='cargo nextest run --no-fail-fast'    # run all, show all failures
alias cc-lint='cargo clippy -- -D warnings'
alias cc-fmt='cargo fmt'
alias cc-check='cargo fmt --check && cargo clippy -- -D warnings'
alias cc-audit='cargo audit'

# Run all checks (what CI runs)
alias ci-check='cargo fmt --check && cargo clippy -- -D warnings && cargo nextest run'

# Open docs for a crate
docs() { cargo doc --no-deps --open --package "${1:-play}" 2>/dev/null; }

# Quick context: show what's changed since last commit
alias whatchanged='git diff HEAD --stat && echo "" && git log --oneline -5'

# Show the play crate structure
alias playtree='tree src -I "*.rs.bk" --dirsfirst'

EOF

# Ensure .bashrc.d is sourced from .bashrc
if ! grep -q "bashrc.d" "$HOME/.bashrc" 2>/dev/null; then
    cat >> "$HOME/.bashrc" << 'EOF'

# Source modular bash config files
if [ -d "$HOME/.bashrc.d" ]; then
    for f in "$HOME/.bashrc.d"/*.sh; do
        [ -r "$f" ] && source "$f"
    done
fi
EOF
    ok "Added .bashrc.d sourcing to ~/.bashrc"
fi

ok "Shell aliases written to $ALIAS_FILE"
info "Run: source ~/.bashrc  (or open a new terminal)"

# =============================================================================
# 11. AUDIT LOG DIRECTORY
# =============================================================================
step "Creating runtime state directories"

mkdir -p "$HOME/.local/share/play/"{states,prefixes,runners,cache,logs}
info "Created: ~/.local/share/play/{states,prefixes,runners,cache,logs}"
ok "Runtime directories ready"

# =============================================================================
# 12. VERIFY EVERYTHING
# =============================================================================
step "Final verification"

echo ""
echo -e "  ${BOLD}Rust toolchain:${NC}"
rustc --version | sed 's/^/    /'
cargo --version | sed 's/^/    /'

echo ""
echo -e "  ${BOLD}Cargo tools:${NC}"
for tool in cargo-nextest cargo-watch cargo-audit cargo-deny tokei; do
    if cargo "$tool" --version >/dev/null 2>&1 || command -v "${tool#cargo-}" >/dev/null 2>&1; then
        echo -e "    ${GREEN}✓${NC} $tool"
    else
        echo -e "    ${YELLOW}⚠${NC} $tool (not installed — non-critical)"
    fi
done

echo ""
echo -e "  ${BOLD}Git hooks:${NC}"
for hook in pre-commit commit-msg; do
    [[ -x ".git/hooks/$hook" ]] && echo -e "    ${GREEN}✓${NC} $hook" || echo -e "    ${RED}✗${NC} $hook missing"
done

echo ""
echo -e "  ${BOLD}Config files:${NC}"
for f in .cargo/config.toml rustfmt.toml .clippy.toml deny.toml .claudeignore CLAUDE.md .claude/settings.json; do
    [[ -f "$f" ]] && echo -e "    ${GREEN}✓${NC} $f" || echo -e "    ${YELLOW}⚠${NC} $f (not found)"
done

echo ""
echo -e "  ${BOLD}Custom commands:${NC}"
for cmd in plan tweak module review detect db-entry; do
    f=".claude/commands/${cmd}.md"
    [[ -f "$f" ]] && echo -e "    ${GREEN}✓${NC} /$cmd" || echo -e "    ${YELLOW}⚠${NC} /$cmd missing"
done

# =============================================================================
# DONE
# =============================================================================
echo ""
echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}${BOLD}  Setup complete.${NC}"
echo -e "${GREEN}${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo "  Next steps:"
echo ""
echo "  1. source ~/.bashrc"
echo "     (load new aliases into this terminal)"
echo ""
echo "  2. cd ~/play && claude"
echo "     (start Claude Code — CLAUDE.md auto-loads as cached context)"
echo ""
echo "  3. In Claude Code, type: /plan implement the Workspace Cargo.toml"
echo "     (first task from the checklist)"
echo ""
echo "  Model selection guide:"
echo "    claude              → Sonnet (default, use for everything complex)"
echo "    cch                 → Haiku (cheap, use for: rename, format, boilerplate)"
echo "    claude --print '…'  → non-interactive, pipe output to a file"
echo ""
echo "  Key cargo aliases (from .cargo/config.toml):"
echo "    cargo t             → nextest run (fast tests)"
echo "    cargo c             → clippy -D warnings"
echo "    cargo w             → watch -x 'nextest run'"
echo "    ci-check            → full CI check suite"
echo ""
