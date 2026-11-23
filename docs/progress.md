# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-23

## Current status

| Phase | Status |
| --- | --- |
| Phase 0 — Workflow foundation | done |
| Phase 1 — Hardening | done |
| Phase 2 — Feature loop | in progress |
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
- [x] Protocol round-trip tests (postcard)
- [x] Unit tests for `maps` parsing and `level` collision
- [x] Remove dead code (`try_move`, `spawn_entities`)
- [x] Finalize ADR-001 networking model → server-authoritative (Accepted)
- [x] Phase 2a — extract `game` core crate (ADR-001 step 1)
- [ ] Phase 2b — server-authoritative movement (server tick loop, inputs)
- [ ] Phase 2c — client prediction + reconciliation

## Next actions (top = do first)

1. Phase 2b — server-authoritative movement (server tick loop @ 20 Hz `TICK_HZ`,
   client sends inputs instead of positions).
2. Phase 2c — client prediction + reconciliation.
3. Consider client lib/bin split to enable `client/tests/` integration tests.

## Blockers / notes

- (resolved) Protocol duplication/drift — fixed by the shared `protocol/` crate in Phase 0.
- Phase 0 `clippy -D warnings` surfaced ~24 pre-existing lints, fixed in Phase 0
  (empty branches, collapsible ifs, casts, unused imports/vars), plus the
  `world::world` → `world::game` module rename.
- Phase 1 QA flagged: `PotionSpawn` currently spawns a green `Enemy` (no `Potion`
  type yet) — latent, but now locked by test; client is a binary crate so tests
  are inline `#[cfg(test)]` (a lib/bin split would enable `client/tests/`).
- (follow-up) `Potion` is intended to be a collectible entity, not an enemy. The
  current `PotionSpawn → green Enemy` is a placeholder; a future phase should
  introduce a real `Potion` collectible and retire `EnemyKind::Potion`.
