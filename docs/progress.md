# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-23

## Current status

| Phase | Status |
| --- | --- |
| Phase 0 — Workflow foundation | in progress |
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

- [ ] Extract shared `protocol/` crate; fix protocol drift
- [ ] Protocol round-trip tests (postcard)
- [ ] Unit tests for `maps` parsing and `level` collision
- [ ] Clean dead code / empty branches
- [ ] Finalize ADR-001 networking model → netcode hardening

## Next actions (top = do first)

1. Create Cargo workspace + `protocol/` crate.
2. Move `GamePacket` into `protocol/` and fix field-type drift.
3. Add protocol round-trip tests.
4. Add `rustfmt.toml` + clippy lints.
5. Add CI workflow.

## Blockers / notes

- Protocol is currently duplicated in `server/src/game_packet.rs` and
  `client/src/network/client.rs` and has already drifted (see ADR-001).
