# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-25 (Phase 2f COMPLETE - all quality gates green)

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
- [x] Phase 2b — server-authoritative movement (server tick loop, inputs)
- [x] Phase 2c — client prediction + reconciliation
- [x] Phase 2d — e2e testing infrastructure (all 41 e2e tests pass, ~116 total)
- [x] Phase 2e — server-authoritative hit detection & bullet spawn (all 145 tests pass)

## Phase 2e checklist
- [x] Server-authoritative bullet spawn (`BulletSpawn` packet to ALL clients)
- [x] Server-authoritative enemy death (`EnemyDied` packet + per-room enemy state)
- [x] Room load sync (`EnemyStateRequest`/`Response`)
- [x] Fixed double JoinRequest bug
- [x] Fixed multi-fire bug (server cooldown)
- [x] Fixed enemy death sync after room transition (pending_enemy_deaths queue)
- [x] Fixed bullet sticky reset on room transition

## Phase 2f checklist
- [x] Spec written (`docs/specs/phase-2f-netcode-sync.md`)
- [x] Plan review passed
- [x] Clock sync implementation (`server_tick`, `server_time_ms` in `PlayerPos`)
- [x] Jitter buffer + frame interpolation (N=5, tick-based)
- [x] Packet logging (JSONL, `RUSTVANIA_PACKET_LOG`)
- [x] Headless client (`--headless --duration=N --server=ADDR`)
- [x] All 172+ tests pass, quality gates green

## Next actions (top = do first)

1. **Phase 2g planning** — define next feature scope (enemy AI server-authoritative, room transition server-authoritative, etc.)
2. **Launch spec-designer** for Phase 2g
3. **Full pipeline** for Phase 2g

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
- (2b complete) Server-authoritative movement landed on `feat/server-authoritative-movement`
  (4 commits, 40 tests, all gates green). Client is a dumb terminal for movement
  (latent until 2c). Room-entry glitch accepted (client-side transitions briefly
  overwritten by authoritative `PlayerPos`). Enemy RNG stays client-side for now
  (the server does not simulate enemies — scope boundary D6).
- (2d infrastructure built) e2e testing framework landed on `feat/e2e-testing`
  (9 commits, 75 tests, all gates green). Client and server crates are split into
  lib + thin binary. `World` uses `Box<dyn PacketIO>` for swappable transport.
  `TestHarness` provides a deterministic in-memory simulation loop. The tester
  now writes e2e integration tests (Boxes 9–14).
- (2e complete) Server-authoritative hit detection landed on `feat/netcode-sync`
  (145 tests, all gates green). Server validates hits, tracks enemy death state,
  broadcasts `BulletSpawn` to ALL clients and `EnemyDied` to room clients.
- **Process violation resolved**: Phase 2f was properly implemented through spec→plan→review→test→implement pipeline. All boxes checked, Explain blocks filled, 172 tests pass, quality gates green.