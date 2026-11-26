# Skill: Issue-Driven Development Workflow

## Overview
This workflow handles issues (bugs, tech debt, clippy warnings, refactors) that emerge during development. It complements the feature pipeline by providing a structured process for unplanned work.

## When to Use
- Clippy warnings blocking CI
- Test failures from regressions
- Bugs discovered during testing
- Tech debt identified during implementation
- Any "small fix" that benefits from tracking

## Issue Lifecycle

```
DISCOVER → ANALYZE → PLAN → REVIEW → IMPLEMENT → VERIFY → CLOSE
```

### 1. DISCOVER
Create an issue entry in `docs/issues/<date>-<slug>.md`:
```markdown
# Issue: <title>
- **Type**: bug | clippy | refactor | test | tech-debt
- **Severity**: blocker | major | minor | nit
- **Found during**: <phase/activity>
- **Description**: <what's wrong>
- **Files affected**: <list>
- **Reproduction**: <steps if applicable>
```

### 2. ANALYZE
- Root cause analysis
- Impact assessment (what breaks if not fixed)
- Fix complexity estimate (S/M/L)

### 3. PLAN
Write a minimal fix plan in the issue file:
```markdown
## Fix Plan
- [ ] Step 1: <specific action>
- [ ] Step 2: <specific action>
- [ ] Tests to add/update: <list>
- [ ] Verification: `cargo test`, `cargo clippy`, `cargo fmt`
```

### 4. REVIEW (Optional for minor/nit)
- For blocker/major: ask owner or reviewer subagent
- For minor/nit: self-review sufficient

### 5. IMPLEMENT
- Make changes with conventional commits
- Each commit has `## Explain` block
- Update issue checkboxes

### 6. VERIFY
- Run quality gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check --all`
- All must pass

### 7. CLOSE
- Mark issue status: `closed`
- Update `docs/progress.md` if significant
- Add to `docs/lessons.md` if new Rust concept

## Integration with Feature Pipeline

| Scenario | Process |
|----------|---------|
| Blocker found during feature work | Pause feature, create issue, fix via this workflow, resume feature |
| Minor clippy warning during implementer | Fix inline with Explain block (no separate issue needed) |
| Test failure in CI | Create issue, analyze, fix via this workflow |
| Architectural concern | Create issue, may become new feature spec |

## Quick Fix Path (for trivial fixes)
If fix is < 5 lines and obvious:
1. Fix directly
2. Commit with Explain block
3. Run quality gates
4. No issue file needed

## Example: Clippy Warning Fix

### Discover
```markdown
# Issue: 2026-11-25-clippy-collapsible-match
- **Type**: clippy
- **Severity**: minor
- **Found during**: Phase 2f implementation
- **Description**: `clippy::collapsible_match` in client/src/network/world.rs:318
- **Files affected**: client/src/network/world.rs
```

### Plan
```markdown
## Fix Plan
- [ ] Collapse `GamePacket::Hit { entity_id } => { if ... }` into `GamePacket::Hit { entity_id } if Some(entity_id) == world.id =>`
- [ ] Run `cargo clippy` to verify
```

### Implement
```bash
# Fix applied
git commit -m "fix(client): collapse match arm for Hit packet

## What
Changed `GamePacket::Hit { entity_id } => { if Some(entity_id) == world.id { ... } }`
to `GamePacket::Hit { entity_id } if Some(entity_id) == world.id => { ... }`

## Why
Clippy warning: collapsible_match. The guard pattern is cleaner.

## Rust concepts
Match guards (`if` in match arms), pattern matching

## Watch out
Guard must be exhaustive for the variant (it is - only handles own player hit)
```

### Verify
```bash
cargo clippy --workspace --all-targets -- -D warnings  # passes
cargo test --workspace  # passes
```

### Close
Update issue status to `closed`.

## Issue File Template
```markdown
# Issue: <slug>
- **Status**: open | in-progress | review | closed
- **Type**: bug | clippy | refactor | test | tech-debt
- **Severity**: blocker | major | minor | nit
- **Created**: <date>
- **Closed**: <date>

## Description
<details>

## Root Cause
<analysis>

## Fix Plan
- [ ] Step 1
- [ ] Step 2

## Verification
- [ ] cargo test
- [ ] cargo clippy
- [ ] cargo fmt

## Commits
- <commit hash>: <subject>

## Lessons Learned
<if any>
```