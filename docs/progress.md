# Progress tracker

Working memory for the project. **Update this at the end of every task.**

> Last updated: 2026-11-26 (reconciliation-replay-dt COMPLETE — replay at TICK_HZ + CLIENT_HZ→RENDER_HZ rename; spec 4 of netcode-hardening roadmap)

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

## Windowless headless binary checklist — refactor (`refactor/windowless-headless`)
- [x] Spec + plan written (`docs/specs/windowless-headless.md`, `docs/plans/windowless-headless.md`)
- [x] Box 1 — move `RealPacketIO`/`HeadlessPacketIO`/`make_transport_factory` into `client/src/network/transport.rs` (structs module-private, `pub fn` factory, re-exported from `lib.rs`)
- [x] Box 2 — `[features] gui/headless/default=["gui"]` + two `[[bin]]` entries; new windowless `client/src/main_headless.rs` (plain `fn main()`, private `run_headless` moved verbatim)
- [x] Box 3 — trim `main.rs` to GUI-only; rewire e2e spawn to `--bin rustvania-headless --features headless`; drop Xvfb from CI; `README.md` → `cargo run --bin metroidvania-rust`
- [x] `cargo test --workspace` green WITHOUT Xvfb (194 passed, 8 ignored); `headless_timing` + `packet_log_rate` run the windowless binary
- [x] `cargo clippy --all-targets --all-features --workspace -- -D warnings` clean; `cargo fmt --check --all` clean
- [ ] Manual smoke: `cargo run --bin metroidvania-rust` opens the window (needs a display; binary compiles, not automatable here)

## Remove PvE enemies checklist — refactor (`refactor/remove-pve-enemies`)
- [x] Spec written (`docs/specs/remove-pve-enemies.md`)
- [x] Plan review passed (SEND-BACK → 2 MAJOR + 4 MINOR fixed)
- [x] PvE `Enemy` machinery removed end-to-end (protocol/server/game/client/e2e)
- [x] `enemy_bullets` → `remote_bullets` (+ `update_remote_bullets_position`) rename
- [x] `enemy.rs`, `room_load.rs`, `enemy_death.rs` deleted
- [x] Code review APPROVE-WITH-MINOR (5 doc/process MINORs fixed)
- [x] 178 passed / 0 failed / 3 ignored; fmt + clippy clean; grep clean

## Authoritative player death checklist — feature (`feat/authoritative-player-death`)
- [x] Spec written (`docs/specs/authoritative-player-death.md`)
- [x] Plan review passed (SEND-BACK → MAJOR harness order + 5 MINOR fixed)
- [x] 21 RED tests written by tester (`d857f39`)
- [x] Boxes 1–6 implemented (protocol rename, solid_between, raycast/respawn, broadcast, client respawn, e2e)
- [x] Code review APPROVE-WITH-MINOR; QA done (2 MAJOR + minors, all follow-ups)
- [x] 198 passed / 0 failed / 3 ignored; fmt + clippy clean; grep clean
- [x] ADR-0003 accepted

## Authoritative room membership checklist — feature (`feat/authoritative-room-membership`)
- [x] Spec written; plan approved (2 review passes)
- [x] RED tests (G/P/S/C/E), then 7-box implementer, code review, QA
- [x] QA fix batch: grounded re-arm (M1), bounds guard (m1), cooldown 12→6 (m5), reject rate-limit (m2)
- [x] 247 passed / 3 ignored; ADR-0004; M2 latency limitation recorded as issue

## Reconciliation replay-dt fix — fix (`fix/reconciliation-replay-dt`)
- [x] `reconcile` replays at `TICK_HZ` (was `CLIENT_HZ`) — `84785a3`
- [x] `CLIENT_HZ` → `RENDER_HZ` rename (kept, documented-as-unused) — `a6628a8`
- [x] 198 passed / 3 ignored (dev base); fmt + clippy clean

## Next actions (top = do first)

1. **Merge spec 3 → dev, then rebase spec 4** — `fix/reconciliation-replay-dt` is
   branched from `dev` (spec-2 base) and touches the same memory docs as the
   unmerged spec 3 branch. Merge spec 3 first, then rebase spec 4 onto dev and
   resolve the roadmap/progress/lessons overlap. See roadmap spec 4.

1. **Merge spec 1 → dev, then rebase spec 2** — `feat/authoritative-player-death`
   is stacked on `refactor/remove-pve-enemies` (PR #8, not yet in `dev`). Merge
   PR #8 first, then `git rebase --onto dev refactor/remove-pve-enemies
   feat/authoritative-player-death`, then open PR #9. (QA min-6; owner decides
   merge-first vs stacked-PR.)
2. **Spec 3 — `feat/authoritative-room-membership`** (next, top priority) —
   door-aware `RequestRoomChange`, drop `PlayerInput.level_x/level_y`. QA added a
   victim-side `coords`-spoof acceptance criterion (a cheater can dodge death by
   claiming an empty room). See roadmap spec 3.
3. **Spec 2 QA follow-ups (deferred, record for owner)**:
   - Spawn-kill loop: global `(0,0)` spawn + no i-frames + 83ms cooldown lets an
     attacker kill a respawning player repeatedly. Cheapest fix: `invuln_until_tick`
     map in `raycast_player`; real fix is room-local/checkpoint spawns.
   - `dir.signum()` normalization in `raycast_player` (the "only the sign matters"
     invariant is implicit while `|dir| == 1.0`).
   - `PlayerDied` drops the victim's in-flight bullets on every observer (remote
     `Nemesis` removal detaches bullet ownership); decouple bullet retention from
     `Nemesis` presence, or accept + comment.
   - Respawn-tick phantom-hit window: the victim's already-in-flight `PlayerInput`
     re-claims the old room for 1–2 ticks; ignore `coords` writes on the respawn
     tick (or document in `respawn`).
4. **Phase 2h follow-ups (deferred)**:
   - M2 — laminar poller thread + UDP socket leak on Leave/Retry/quit (needs a
     transport `Drop` that drains-then-stops + joins, plus a real-socket check).
   - `.expect()` restructure in `App::enter_playing_from_join` (pass `assigned_id`
     in instead of stash + payload-less `Joined`, or make `Joined` payload-ful —
     a spec revision).
   - Rebind the world-dump debug key (P is now Pause) and/or re-add F11
     clock-reset if wanted.
   - Paused-screen remote-animation wording (remote snapshots keep updating while
     paused; "rendering stays frozen" is slightly imprecise).
5. **Phase 2g follow-ups (deferred)** — (b) feed the camera the interpolated
   render position (QA m5); (c) snap `prev_player_*` after reconciliation (QA n3).
6. **Phase 2g naming/cleanup** — rename `CLIENT_HZ` → `RENDER_HZ`.
7. **Separate task: English/Spanish cleanup** — `GRAVEDAD`/`SALTO_FUERZA`/
   `VELOCIDAD` and remaining Spanish comments → English.
8. **Follow-up: netcode smoothness review** — owner wants a seamless online
   experience; analyze interpolation/prediction smoothness (jitter buffer N,
   `INTERPOLATION_DELAY_TICKS`, remote dir/rotation interpolation) as a future
   phase.
9. **Windowless-headless QA follow-ups (deferred, non-blocking)**:
   - Collapse `RealPacketIO`/`HeadlessPacketIO` (now byte-identical in
     `transport.rs`; decide whether the transport-level `headless` bool survives).
   - Convert `.unwrap()` in lib code (`transport.rs` bind/`to_allocvec`,
     `SocketAddr::parse`) to `.expect(...)` with a message / `Result`.
   - T13 manual GUI smoke — run `cargo run --bin metroidvania-rust` once (needs a
     display) to confirm the split didn't break the window path.

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
- (windowless-headless complete) Windowless-headless bin-split refactor landed on
  `refactor/windowless-headless` (3 commits, 3 boxes, all gates green). Transport
  moved into the lib (`client/src/network/transport.rs`, re-exported); two
  feature-gated binaries (`metroidvania-rust` GUI + `rustvania-headless` plain
  `fn main()`); e2e spawn rewired to `--bin rustvania-headless --features
  headless`; Xvfb dropped from CI; `README.md` → `cargo run --bin
  metroidvania-rust`. `cargo test --workspace` passes WITHOUT Xvfb (194 passed,
  8 ignored). `protocol/`/`server/` untouched. Minor plan inconsistencies noted:
  (a) Box 1's "add `use crate::network::logging::log_packet`" would be an unused
  import if call sites stayed fully-qualified — resolved by importing the short
  name and calling `log_packet(...)`; (b) the plan's T4 grep expects no
  `make_transport_factory` in `main.rs`, but Box 3 *requires* the lib import +
  call there — the real intent ("no local *definitions*") is satisfied (all four
  definitions are gone; only the call site remains). Manual smoke T13
  (`cargo run --bin metroidvania-rust` opens a window) is deferred to the owner
  (needs a display); the binary compiles under `--all-targets --all-features`.
- (remove-pve-enemies complete) Spec 1 landed on `refactor/remove-pve-enemies`
  (code `c58724e`, docs `a6b7b61`, deletions `5842b9f`). PvE `Enemy` machinery
  removed end-to-end (protocol `EnemyDied`/`EnemyState*`; server `EnemyState`/
  `EnemyDiedInfo`/`enemies`/`raycast_hit`; client `World.enemies` + RNG loop +
  draw; e2e PvE tests). `enemy_bullets` → `remote_bullets`. 178 passed / 3
  ignored; fmt + clippy clean; grep clean. Code review APPROVE-WITH-MINOR (5 doc
  MINORs fixed). "Shoot hits nothing" is accepted until spec 2.
- **Token optimization (owner-driven)**: all 7 subagents pinned to
  `opencode-go/deepseek-v4.1-flash` (standard variant); `docs/progress.md` removed
  from auto-load; pre-commit quiet on success + skips `cargo test` on docs-only
  commits. Commits `64470e5` + `00215d2` live on the remove-pve-enemies branch —
  branch-scope decision pending (Next actions #1).
- (authoritative-player-death complete) Spec 2 landed on `feat/authoritative-player-death`
  (7 commits, 6 boxes + fmt). Server is now the sole authority on PvP death:
  `Hit` → `PlayerDied`, `raycast_player` (horizontal, same-room, self-excluded,
  wall-occluded via `Level::solid_between`, closest-hit-wins), `respawn` to the
  global `(0,0)`/`(100,100)` spawn, `update -> (Vec<BulletSpawnInfo>,
  Vec<PlayerHitInfo>)`, impact-x `BulletSpawn`, and client respawns only on
  `PlayerDied` (no self-report). 21 tests (1 protocol + 5 solid_between + 10
  server + 3 client + 2 e2e); 198 passed / 3 ignored; fmt + clippy clean; grep
  clean. Code review APPROVE-WITH-MINOR; QA found 2 MAJOR (spawn-kill loop; victim
  `coords`-spoof cheat) — both recorded as follow-ups, not blockers. ADR-0003
  accepted (hitscan, impact-x bullet, global respawn, ignored inbound, no room
  filter). Branch is stacked on `refactor/remove-pve-enemies` (PR #8) — merge order
  pending (Next actions #1).