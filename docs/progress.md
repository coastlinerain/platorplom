# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-25 (Phase 2h game-state-machine COMPLETE — 18 RED tests green, all 5 boxes + e2e helper done; code review APPROVE, QA advisory done; subagent shell perms fixed + verified)

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

## Phase 2g checklist
- [x] Box 1 — `MAX_ACCUMULATED_FRAMES = 5`, `HEADLESS_BUSY_WAIT_MS = 1` (`game/src/config.rs`)
- [x] Box 2 — `prev_player_pos`/`prev_player_dir` fields on `World` (spawn-seeded)
- [x] Box 2a — decouple `World::update` from macroquad (input keys + camera viewport via `GameCamera::set_viewport`)
- [x] Box 3 — store prev player state before `player.update`
- [x] Box 4 — `interpolated_player_pos/dir(alpha)` + `draw(&self, alpha)`
- [x] Box 5 — accumulator loop in GUI `main.rs` (clamp to 5 steps, interpolation alpha)
- [x] Box 6 — headless hybrid sleep + busy-wait (`run_headless` non-async)
- [x] Box 7 — server catch-up loop verification comment (no code change)
- [x] Boxes 8–11 — unit tests (written by tester; now green)
- [x] Boxes 12–15 — integration tests (written by tester; now green)
- [x] Box 16 — fixed-rate input send verified (natural consequence of accumulator)
- [x] Box 17 — `cargo test` + `cargo fmt --check` + `cargo clippy -- -D warnings` (green)

## Phase 2h checklist — game state machine (`feat/game-state-machine`)
- [x] Spec written (`docs/specs/game-state-machine.md`), ADR-0002 accepted
- [x] Plan review passed (`docs/plans/game-state-machine.md`)
- [x] Box 1 — `game/src/state_machine.rs` (`AppState` ×6, `AppEvent` ×11, total `transition`), `CONNECT_TIMEOUT_MS`/`DISCONNECT_TIMEOUT_MS` in `game/src/config.rs`
- [x] Box 2 — `client/src/network/handshake.rs` (`poll_join_response`), `update_network` → `recv_and_process` + `send_input`, `World.received_player_pos` liveness flag
- [x] Box 3 — `client/src/app.rs` (`App` runner + `TransportFactory`), re-exported from `client/src/lib.rs`
- [x] Box 4 — `client/src/main.rs` thin loop over `App` (GUI + headless share handshake/state machine)
- [x] Box 5.8 — `e2e/src/process.rs` shared "spawn server + headless client" helper (port-serialized + `Drop` guard); `headless_timing.rs` + `packet_log_rate.rs` refactored to run against a live server
- [x] Code review (reviewer) — APPROVE (7 MINORs, triaged); AC14 diff review clean (`protocol/`/`server/` untouched)
- [x] QA + CI — AC15 gates green; QA advisory done (M2 socket leak + `.expect` restructure deferred as follow-ups)

## Next actions (top = do first)

1. **Push/PR (owner merges)** — Phase 2h game-state-machine is complete (all
   gates green, code review APPROVE + QA done). Push `feat/game-state-machine`
   + open PR to `dev`; owner merges.
2. **Phase 2h follow-ups (deferred)**:
   - M2 — laminar poller thread + UDP socket leak on Leave/Retry/quit (needs a
     transport `Drop` that drains-then-stops + joins, plus a real-socket check).
   - `.expect()` restructure in `App::enter_playing_from_join` (pass `assigned_id`
     in instead of stash + payload-less `Joined`, or make `Joined` payload-ful —
     a spec revision).
   - Rebind the world-dump debug key (P is now Pause) and/or re-add F11
     clock-reset if wanted.
   - Paused-screen remote-animation wording (remote snapshots keep updating while
     paused; "rendering stays frozen" is slightly imprecise).
3. **Phase 2g follow-ups (deferred)** — (b) feed the camera the interpolated
   render position (QA m5); (c) snap `prev_player_*` after reconciliation (QA n3).
4. **Phase 2g naming/cleanup** — rename `CLIENT_HZ` → `RENDER_HZ`.
5. **Separate task: English/Spanish cleanup** — `GRAVEDAD`/`SALTO_FUERZA`/
   `VELOCIDAD` and remaining Spanish comments → English.
6. **Follow-up: netcode smoothness review** — owner wants a seamless online
   experience; analyze interpolation/prediction smoothness (jitter buffer N,
   `INTERPOLATION_DELAY_TICKS`, remote dir/rotation interpolation) as a future
   phase.

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
- (2g implemented) Fixed-timestep spec + plan + RED tests → implementation complete.
  All 17 boxes (incl. Box 2a input decoupling) done; Explain fields filled. Grill
  with owner surfaced: (a) the accumulator's job is deterministic sim for
  prediction/reconciliation, NOT a render-floor guarantee — spec's "30 FPS floor"
  claim was mathematically wrong (clamp=5 keeps full 60Hz sim down to 12 FPS
  render) and has been removed; (b) `CLIENT_HZ` will rename to `RENDER_HZ`;
  (c) `GLOSSARY.md` created; (d) Spanish→English cleanup deferred. Owner's goal:
  seamless online experience → netcode-smoothness follow-up.
- **Implementer/tester shell denial (resolved)**: the `implementer` and `tester`
  subagents' `shell` tool returned `Permission denied: shell`, so neither could
  run `cargo test`/`fmt`/`clippy` or `git add`/`git commit`. The orchestrator
  took over verification + commit. This surfaced a cluster of tester-authored
  RED-test bugs that were never executed (all now fixed): `std::env::set_var`
  is `unsafe` in edition 2024; `gui_accumulator` off-by-one (`client_tick`
  reset by the deferred `JoinResponse`); `cargo run --bin` vs `-p` ambiguity
  when CWD is `e2e/`; one `clippy::collapsible_if`. The `implementer` also
  missed a macroquad-context dependency (`GameCamera::update` → `screen_width`)
  that Box 2a then covered. CI now runs tests under Xvfb because the "headless"
  client still opens a macroquad window.
- **Skills are advisory tools, not enforced steps (owner note)**: the subagent
  definitions list relevant skills (`tdd`, `codebase-design`, `domain-modeling`,
  `diagnosing-bugs`, `research`, `prototype`, `grilling`), but during Phase 2g no
  subagent actually invoked the `skill` tool — they worked directly from
  spec/plan/AGENTS.md. Owner's stance: do NOT force skill loading; keep skills as
  reachable tools each agent can invoke when effective. Some cases would have
  benefited from an explicit prompt (e.g. `grilling` for the design/decision
  stress-test that was instead done as a plain Q&A). Next phase: when a task
  matches a skill's purpose, prompt that subagent to reach for the skill rather
  than assuming it will happen automatically.
- (2h complete) Game-state-machine spec + plan + RED tests → implementation
  complete. Boxes 1–5.8 done (pure `transition` core, `poll_join_response`
  handshake, `update_network` receive/send split + `received_player_pos`
  liveness flag, `App` runner + `TransportFactory`, `main.rs` thin-loop rewrite,
  and the `e2e/src/process.rs` "spawn server + headless client" helper with
  port-serialization + `Drop` guard). Code review APPROVE (7 MINORs) + QA
  advisory done. `protocol/`/`server/` left untouched (AC14). Headless e2e tests
  refactored to spawn a live server (headless now genuinely connects).
  Post-review cleanups: deleted orphaned `client/src/hud.rs` +
  `client/src/network/client.rs` (dead code + Spanish comments) and the stray
  `.git_commit_msg`; re-added the "Connected!" print; added two `lessons.md` 2h
  entries. M2 (laminar socket/thread leak) and the `.expect` restructure are
  deferred follow-ups.
- **Subagent shell denial (root-caused + fixed)**: the implementer/tester (and
  reviewer/qa) shipped code they could not compile because the `shell * deny`
  allowlist made OpenCode's scanner deny *compound* commands (any non-allowlisted
  subcommand like `tail` in a pipe hit the deny, and "any deny denies the
  operation"). Fixed by switching all four agents to a denylist (`shell *` allow
  + deny on git push/rm/mv/branch/merge/reset/rebase/cherry-pick/revert/tag/
  clean; reviewer/qa additionally deny add/commit/checkout/stash to stay
  read-only). Verified: `cargo --version`, a compound `cargo test | tail`, and
  `rm` (correctly denied) all behave as expected.