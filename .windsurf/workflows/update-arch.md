---
description: Update arch.md to reflect current project state after major changes
---

# Update Architecture Documentation

This workflow updates `arch.md` to reflect the current state of the project after major code changes. This ensures any LLM can read the architecture document and be up-to-date.

## When to Use

Run this workflow after:
- Adding new major features (e.g., checkpoint system, fast-launch mode)
- Fixing significant bugs that change architecture
- Refactoring core modules
- Adding new crates or major reorganization
- Changes to the system tweak architecture
- Changes to the orchestrator phase flow

## Steps

1. **Review recent changes**
   - Check git log for recent commits on main branch
   - Identify which modules were modified
   - Note any new features or architectural changes

2. **Update "Completed" section**
   - Add newly completed features to the list
   - Remove items that are no longer relevant
   - Keep the list concise and focused on major milestones

3. **Add to "Recent Fixes" section**
   - For each significant bug fix, add an entry with:
     - Clear title describing the issue
     - Root cause explanation
     - Fix applied (what code was changed)
     - Status (Fixed/Open)
   - Keep the most recent 5-10 fixes
   - Archive older fixes to a separate file if needed

4. **Update "Known Issues" section**
   - Add new issues discovered during development
   - Update status of existing issues (Open → Fixed → Archived)
   - Include symptoms, root cause (if known), and potential fixes
   - Remove issues that are no longer relevant

5. **Review and verify**
   - Read through the entire document
   - Ensure all sections are consistent
   - Check that code references (file names, function names) are accurate
   - Verify the document reflects the actual codebase state

6. **Commit the changes**
   - Commit with message: "docs: update arch.md for [brief description of changes]"
   - Example: "docs: update arch.md for fast-launch feature implementation"

## Guidelines

- **Be specific**: Reference actual file names, function names, and module paths
- **Keep it current**: Archive old fixes and issues to keep the document readable
- **Focus on architecture**: This is not a changelog - focus on structural changes, not minor bug fixes
- **Link to code**: When possible, reference specific files and line numbers
- **Maintain structure**: Keep the existing document structure intact

## Example Entry

```
**7. Fast-launch checkpoint optimization**

**Root Cause:**
Orchestrator created a temporary state directory before detection, then renamed to SHA256 hash after detection. The CLI's `create_orchestrator()` always used the temp path, so `recover()` couldn't find existing checkpoints.

**Fix Applied:**
- Modified `create_orchestrator()` to search for existing checkpoint by exe_path before creating orchestrator
- If checkpoint found, use that state root directly via `Orchestrator::with_roots()`
- Added `Planned` to `can_resume()` phases (previously only `Confirmed` and `Executed`)

**Status:**
Fixed. Fast-launch now correctly finds and uses existing validated checkpoints.
```
