# Skill: Documentation Maintenance for Rustvania

## Overview
This skill defines how to keep `docs/progress.md`, `docs/lessons.md`, specs, and plans in sync with implementation. Use this when the orchestrator needs to update documentation after a phase completes.

## When to Update Docs

### After Each Pipeline Stage
| Stage | Files to Update |
|-------|-----------------|
| Spec approved | `docs/specs/<feature>.md` status → `implemented` |
| Plan approved | `docs/plans/<feature>.md` status → `implemented` |
| Tests written | Plan checkboxes `[x]` for test boxes |
| Implementation done | All plan boxes `[x]`, fill Explain blocks |
| Code review passed | `docs/progress.md` phase status, next actions |
| QA passed | `docs/lessons.md` new Rust concepts |
| Merge | Branch status in progress.md |

### Spec/Plan Sync Checklist (Run After Each Phase)
- [ ] Spec status: `implemented`
- [ ] Plan status: `implemented`
- [ ] All plan boxes: `[x] done`
- [ ] All Explain blocks: filled from commit messages
- [ ] Progress.md: phase status, next actions, blockers
- [ ] Lessons.md: new Rust concepts from Explain blocks
- [ ] ADR updated if architectural decision made
- [ ] Branch pushed, PR opened, owner validation requested

## File Formats

### progress.md
```markdown
## Current status
| Phase | Status |
|-------|--------|
| Phase 2f | in progress |

## Phase 2f checklist
- [x] Spec written
- [x] Plan review passed
- [ ] Tests written (RED)
- [ ] Implementation (GREEN)
- [ ] Code review
- [ ] QA

## Next actions
1. Implementer: make tests GREEN
2. Reviewer: code review
...

## Blockers / notes
- ...
```

### lessons.md
```markdown
## Rust Concepts Learned

### Phase 2f: Netcode Synchronization
- **SystemTime vs Instant**: Wall-clock for NTP-style sync, monotonic for interpolation
- **Ring buffer eviction**: Shift `[1]→[0]..[n-1]→[n-2]`, new at `[n-1]`
- **Tick-based interpolation**: Convert server tick delta to Instant using anchor
- **Feature-gated binaries**: `required-features` in Cargo.toml for headless/GUI
```

### Spec status field
```yaml
status: draft | implemented
```

### Plan checkboxes
```markdown
- [x] Box A1: Add INTERPOLATION_DELAY_TICKS constant
  - Explain: [filled after commit]
```

## Automation Options

### Option 1: General Subagent (Recommended)
Launch `general` subagent with prompt:
```
Update docs/progress.md and docs/lessons.md for Phase 2f completion.
Read recent commits for Explain blocks.
Update spec/plan status to implemented.
Mark all plan boxes as done.
```

### Option 2: Orchestrator Direct
Do it yourself in-session after each phase (simpler, no context switch).

### Option 3: Automation (Scheduled)
Create automation that runs after git push:
```javascript
// Not needed for now - manual is fine
```

## Quick Commands

### Update progress.md phase status
```bash
# After plan approval
sed -i 's/| Phase 2f .*/| Phase 2f | in progress |/' docs/progress.md
```

### Update spec status
```bash
sed -i 's/status: draft/status: implemented/' docs/specs/phase-2f-netcode-sync.md
```

### Update plan status + checkboxes
```bash
sed -i 's/status: draft/status: implemented/' docs/plans/phase-2f-netcode-sync.md
# Checkboxes updated manually per box
```

## Integration with Pipeline

Add to orchestrator's post-phase routine:
```markdown
After each phase:
1. Run spec/plan sync checklist
2. Launch general subagent for doc updates (or do inline)
3. Verify with `cargo test` still passes
4. Commit docs with Explain block
```

## Example: Phase 2f Completion

When all tests pass and code review approves:
1. `docs/specs/phase-2f-netcode-sync.md`: status → implemented
2. `docs/plans/phase-2f-netcode-sync.md`: status → implemented, all 24 boxes `[x]`
3. `docs/progress.md`: Phase 2f → done, Phase 2g next actions
4. `docs/lessons.md`: Add SystemTime/Instant, ring buffer, tick interpolation, feature-gated binaries
5. Commit: `docs(phase-2f): update progress, lessons, spec, plan status`