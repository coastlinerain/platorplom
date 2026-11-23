# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-23

## Current status

| Phase | Status |
| --- | --- |
| Phase 0 — Workflow foundation | done |
| Phase 1 — Hardening | todo |
| Phase 2 — Feature loop | todo |
| Phase 3 — Continuous learning | ongoing |

## Phase 0 checklist

- [x] `PLAN.md` (roadmap)
- [x] `AGENTS.md` (memory & rules)
- [x] `README.md` (rewrite)
- [x] `CONTRIBUTING.md` (workflow)
- [x] `docs/progress.md` (this file)
- [x] `docs/lessons.md`
- [x] `docs/adr/` index + ADR-001 draft
- [x] `docs/specs/` index + template
- [x] `docs/plans/` index + template
- [x] `orchestrator` (default primary) + `spec-designer`/`planner`/`implementer`/`tester`/`reviewer`/`explainer` subagents
- [x] `opencode.json` (default agent + auto-load `CONTRIBUTING.md` + `docs/progress.md`)
- [x] Cargo workspace + shared `protocol/` crate
- [x] `rustfmt.toml` + clippy lints
- [x] Conventional commit template + pre-commit hooks
- [x] GitHub Actions CI (fmt, clippy, test)

## Phase 1 checklist

- [x] Extract shared `protocol/` crate; fix protocol drift (done in Phase 0)
- [ ] Protocol round-trip tests (postcard)
- [ ] Unit tests for `maps` parsing and `level` collision
- [ ] Remove dead code (`try_move`, `spawn_entities` behind `#[allow(dead_code)]`)
- [ ] Finalize ADR-001 networking model → netcode hardening

## Next actions (top = do first)

1. Add protocol round-trip tests (postcard).
2. Unit tests for `maps` parsing and `level` collision.
3. Remove dead code (`try_move`, `spawn_entities`).
4. Finalize ADR-001 networking model → netcode hardening.

## Blockers / notes

- (resolved) Protocol duplication/drift — fixed by the shared `protocol/` crate in Phase 0.
- Phase 0 `clippy -D warnings` surfaced ~24 pre-existing lints, fixed in Phase 0
  (empty branches, collapsible ifs, casts, unused imports/vars), plus the
  `world::world` → `world::game` module rename.
