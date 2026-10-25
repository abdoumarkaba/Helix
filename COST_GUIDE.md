# Claude Code Cost & Performance Guide
## for the `play` project

---

## How Claude Code Charges You

Claude Code charges per token — input tokens (what you send, including all context)
and output tokens (what Claude responds). The most important cost lever is: **input tokens**.

### Prompt Caching (your biggest savings tool)

Claude Code automatically sends `CLAUDE.md` as part of every request's system prompt.
Because CLAUDE.md doesn't change between requests, Anthropic's API caches it.
After the first request in a session, that cached content costs **~10× less** than normal input.

**Implication:** A detailed CLAUDE.md is essentially free after the first request. This is
why we made it ~600 lines. More context = better responses at near-zero marginal cost.

**Cache TTL:** 5 minutes of inactivity resets the cache. If you pause for lunch and
come back, the first request after your break re-pays the CLAUDE.md loading cost once.

**How to maximize cache hits:**
- Don't edit CLAUDE.md mid-session (edits invalidate the cache)
- Keep your sessions continuous rather than starting/stopping frequently
- Long-running `claude` sessions (the interactive REPL) amortize the load cost best

---

## Model Selection Strategy

### Sonnet (default — `claude-sonnet-4-6`)
Use for: architecture decisions, implementing new modules, complex logic, code review,
anything where correctness or design judgment matters.

**Use Sonnet when:** writing the DetectionModule, implementing the DecisionEngine,
designing the TweakRegistry, implementing Drop guards, anything in this checklist:
- New module scaffolding
- Antipattern review
- Rollback logic
- Error variant design
- Type system decisions

### Haiku (`claude-haiku-4-5-20251001`) — ~20× cheaper than Sonnet
Use for: mechanical, repetitive, or clearly-scoped tasks where you know exactly
what you want and just need it typed out.

**Use Haiku when:**
- Adding a field to an existing struct (you know the type, just need the boilerplate)
- Renaming a variable or function across multiple files
- Writing a new test that follows an existing pattern
- Generating a play-db entry TOML (you have all the data, just need it formatted)
- Explaining what a short function does
- Asking "is this valid TOML?"

**How to switch in a session:**
```bash
cch  # alias for: claude --model claude-haiku-4-5-20251001
```
Or mid-session in the Claude Code REPL, just type the alias on a new line.

---

## Context Window Management

Claude Code has a finite context window. As a session grows, two things happen:
1. Cost increases (more input tokens per request)
2. Quality may drop (older context gets compressed or dropped)

### The `/compact` Command
When your session has gotten long (lots of back-and-forth, many file reads),
type `/compact` in Claude Code. This compresses the conversation history into
a dense summary while keeping the most important context.

**When to use `/compact`:**
- Before switching to a completely new subtask in the same session
- When you notice Claude starting to lose track of earlier decisions
- After completing a major feature (the history is now "done", summarize it)
- When the session has gone on for >30 minutes of active work

**Tip:** After compacting, re-state your current goal explicitly:
"I just compacted context. I'm now working on implementing HardwareDetector.
The module contract is defined in src/detection/mod.rs. Continue from there."

### Starting Fresh vs. Continuing
For completely different tasks (e.g., switching from implementing detection to
writing CI config), starting a new `claude` session is often cheaper and cleaner
than continuing — CLAUDE.md re-loads cheaply (cached), and there's no stale context.

---

## Custom Commands as Cost Reducers

The `.claude/commands/*.md` files act as compressed prompt templates. Instead of
typing 200 tokens of instructions every time you want a code review, `/review <file>`
expands to the full review checklist automatically.

**Cost math example:**
- Without `/review`: "Please review src/detection/mod.rs for correctness, check for
  antipatterns, verify error handling..." = ~150 tokens repeated every review
- With `/review src/detection/mod.rs`: the command is ~30 tokens but expands Claude's
  attention to the full checklist = same quality, fewer tokens you need to type

The commands also improve **quality**, not just cost, because they ensure Claude checks
every item consistently rather than relying on your memory to specify everything.

---

## Practical Session Patterns

### Starting a new feature session
```
cd ~/play
claude                          # starts with cached CLAUDE.md context
/plan implement HardwareDetector for NVIDIA GPU detection
```
Cost: 1 CLAUDE.md cache load + your prompt. Then iterative work at low per-request cost.

### Mechanical task (use Haiku)
```
cch                             # start with Haiku model
Add a `display_server: DisplayServer` field to DisplayProfile struct, update all match arms
```
Cost: fraction of Sonnet for a task that doesn't need deep reasoning.

### Full CI check before committing
```bash
ci-check    # alias: cargo fmt --check && cargo clippy -- -D warnings && cargo nextest run
```
This is free — no Claude tokens, just your local Rust toolchain.

### Code review loop
```
/review src/planning/decision_engine.rs
# Read the review, fix the issues it found
/review src/planning/decision_engine.rs    # re-review after fixes
```

---

## Spending Mental Model

Think of Claude Code token costs like a consultant's hourly rate. Sonnet is your
senior architect: use them for hard problems. Haiku is your junior who can format
code and fill in boilerplate: use them for anything mechanical.

The CLAUDE.md + custom commands setup essentially pre-briefs both consultants on the
project once, so they never charge you to re-learn the architecture.

A typical productive day on this project might look like:
- 2-3 Sonnet sessions (architecture + implementation decisions) = main cost
- 5-10 Haiku micro-tasks (test boilerplate, struct fields, DB entries) = small cost
- Multiple local `ci-check` runs = free

**Bottom line:** CLAUDE.md context loading is amortized to near-zero by caching.
Model selection is your primary cost lever. Use Haiku by default for small tasks,
Sonnet when you'd actually want to think carefully yourself.
