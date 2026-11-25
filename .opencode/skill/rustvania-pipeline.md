# Skill: Rustvania Pipeline Workflow

## Overview
This skill enforces the Rustvania development pipeline. Every feature must go through the complete pipeline — no shortcuts, no bypassing stages.

## Pipeline Stages

```
1. SPEC      → spec-designer  → docs/specs/<feature>.md
2. PLAN      → planner        → docs/plans/<feature>.md  
3. REVIEW    → reviewer       → validate plan (plan mode)
4. TESTS     → tester         → write RED tests
5. IMPLEMENT → implementer    → make GREEN, fill Explain blocks
6. REVIEW    → reviewer       → code review (code mode)
7. QA        → qa             → advisory pass
8. MERGE     → orchestrator   → push branch, open PR, owner validates
```

## Orchestrator Responsibilities

### Before Each Stage
- [ ] Verify previous stage completed successfully
- [ ] Launch correct subagent with full context
- [ ] Wait for completion before next stage

### Validation Gates (MUST ask owner)
- [ ] Choosing workflow (spec vs fix)
- [ ] Creating branch
- [ ] Pushing to remote
- [ ] Opening PR
- [ ] Merging anything
- [ ] Deleting/renaming files not explicitly requested
- [ ] Architectural decisions affecting multiple crates
- [ ] Changing agent config (`.opencode/agents/` or `opencode.json`)

### After Each Phase
- [ ] Update `docs/progress.md` (status, next actions, blockers)
- [ ] Update `docs/lessons.md` (new Rust concepts from Explain blocks)
- [ ] Update plan status (draft → implemented)
- [ ] Update spec status (draft → implemented)

## Branch Strategy
- Branch name: `<type>/<spec-slug>` (e.g., `feat/netcode-sync`, `fix/room-transition`)
- Branched from `dev`
- Conventional commits with `## Explain` block
- Push and PR only after owner validation
- Owner merges — agent NEVER merges

## Commit Message Format
```
type(scope): subject

## What
<what changed>

## Why
<reason>

## Rust concepts
<features used>

## Watch out
<gotchas/trade-offs>
```

## Quality Gates (must pass before merge)
- `cargo build --workspace`
- `cargo test --workspace` (all pass)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check --all`

## Subagent Usage
- **spec-designer**: Writes specs from requirements
- **planner**: Converts spec to checkable implementation plan
- **reviewer**: Validates plans (plan mode) and code (code mode)
- **tester**: Writes failing tests from plan
- **implementer**: Makes tests pass, fills Explain blocks
- **qa**: Final advisory review
- **explainer**: Learning aid, no edits

## Anti-Patterns (DO NOT)
- ❌ Implement without spec
- ❌ Skip plan review
- ❌ Implement before tests (except infrastructure-first)
- ❌ Commit without Explain block
- ❌ Push without owner validation
- ❌ Merge own PR
- ❌ Bypass pipeline for "small fixes" (use proper fix workflow)

## Fix Workflow (for small issues)
```
1. Analyze → is it really a small fix?
2. If yes: implement directly with Explain block
3. If no: full pipeline
```

## Documentation Maintenance
After each phase completion:
- `docs/progress.md`: Update phase status, next actions, blockers
- `docs/lessons.md`: Add Rust concepts from Explain blocks
- `docs/plans/<feature>.md`: Status → implemented, fill Explain fields
- `docs/specs/<feature>.md`: Status → implemented

## Emergency Procedures
- If reviewer finds blocker: send back to planner with concrete feedback
- If tests fail in implementer: fix immediately, don't proceed
- If pipeline confused: stop, re-read AGENTS.md, ask owner