# Game State Machine — Implementation Plan

- Status: draft
- Spec: `docs/specs/game-state-machine.md`
- ADR: `docs/adr/0002-game-state-machine.md` (Accepted)
- Date: 2026-11-25
- Branch: `feat/game-state-machine`

## Overview

The client today has no application-level lifecycle: `client/src/main.rs` hardcodes a
single connect-and-play path (bind socket → `send_join_req` → build `World` → loop
forever). There is no menu, no pause, no disconnect/quit handling, and the client
never sends `Leave`.

This feature adds a **client-only flat state machine** that owns the client lifecycle:

- A **pure, total** `transition(state, event) -> AppState` core in the `game` crate
  (exhaustively unit-testable: 16 valid edges, 50 no-ops, terminal `Quitting`).
- A **runner** (`client::app::App`) that owns the current state, the socket
  (`Option<Box<dyn PacketIO>>`), the `World`, input sampling, the `FixedTimestep`
  accumulator, and per-state drawing.
- A **handshake helper** (`client::network::handshake`) shared by the GUI and headless
  paths, so both use the exact same join logic.
- A **receive/send split** in `update_network` so `Paused` can receive/process packets
  without stepping or sending, plus a `received_player_pos` liveness flag for the
  disconnect watchdog.
- A **thin `main.rs` rewrite** over `App`, with `--headless` reusing the same state
  machine.

States: `MainMenu → Connecting → Playing ⇄ Paused`, with `Disconnected` for
failure/loss and terminal `Quitting`. `MainMenu` is the initial state.

### Explicit non-goals (honor these — no exceptions)

- **NO changes to `protocol/`** — `GamePacket` is unchanged; no new variants.
- **NO changes to `server/`** — `ServerWorld` and `server/src/main.rs` are untouched;
  the server remains drop-in-join. The client now *sends* `Leave` (new client
  behavior), but the server already handles `Leave`.
- No nested menus (state stack deferred), no real `Loading` state, no server lobby
  phase, no headless-window fix (Phase 2g follow-up, out of scope).

These are gated by **AC14** (diff review): the code review must confirm `protocol/`
and `server/` show zero changes in the final diff.

## Public API (targets for the tester)

Signatures are carried **verbatim** from the spec's "Data model" section. Do not
invent additional public API; the tester and implementer both target exactly these.

### `game` crate — pure core

```rust
// game/src/state_machine.rs  (NEW; wire `pub mod state_machine;` into game/src/lib.rs)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    MainMenu,
    Connecting,
    Playing,
    Paused,
    Disconnected,
    Quitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppEvent {
    StartJoin,
    Joined,
    ConnectFailed,
    ConnectionLost,
    PauseRequested,
    ResumeRequested,
    QuitToMenu,
    Cancel,
    Retry,
    BackToMenu,
    Quit,
}

/// Total transition function. An event not valid for `state` returns `state`
/// unchanged (no-op). `Quitting` is terminal.
pub fn transition(state: AppState, event: AppEvent) -> AppState;
```

```rust
// game/src/config.rs  (add two named constants)

/// Client connect timeout: give up waiting for `JoinResponse` after this long.
pub const CONNECT_TIMEOUT_MS: u64 = 5000;

/// Disconnect watchdog: fire `ConnectionLost` if no `PlayerPos` for this long.
pub const DISCONNECT_TIMEOUT_MS: u64 = 1000;
```

### `client` crate — handshake + receive/send split + liveness flag

```rust
// client/src/network/handshake.rs  (NEW; wire `pub mod handshake;` into network/mod.rs)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeResult {
    Joined { assigned_id: u8 },
    Pending,
}

/// Drain pending packets looking for `JoinResponse`. Returns `Joined` with the
/// assigned id on the first `JoinResponse` (discarding any packets that precede
/// it), or `Pending` if the queue empties without one. Non-blocking (`try_recv`).
pub fn poll_join_response(io: &mut dyn PacketIO) -> HandshakeResult;
```

```rust
// client/src/network/world.rs  (split update_network)

/// Receive half: drain + handle_packet + the (currently inert) staleness check.
/// Public so `Paused` can receive/process packets without stepping the sim or
/// sending input.
pub fn recv_and_process(world: &mut World);

/// Send half: send one `PlayerInput` if `world.id.is_some()`.
pub fn send_input(world: &mut World);

/// Full tick: receive/process then send. Kept for `World::update` (Playing).
pub fn update_network(world: &mut World) {
    recv_and_process(world);
    send_input(world);
}
```

`handle_packet` sets `world.received_player_pos = true` in its `PlayerPos` arm
(own-player **and** remote-player branches both count as liveness).

```rust
// client/src/world/game.rs  (add one field to World)

pub struct World {
    // ... existing fields ...
    /// Set true by `handle_packet` when any `PlayerPos` is received; consumed by the
    /// app runner's disconnect watchdog (cleared each frame after the check).
    pub received_player_pos: bool,  // default false
}
```

### `client` crate — runner

```rust
// client/src/app.rs  (NEW; wire `pub mod app;` into client/src/lib.rs)

use std::time::Instant;
// (server addr is captured by the factory closure, not held by App)

/// Produces a fresh `Box<dyn PacketIO>` for one join attempt. Production binds a
/// laminar socket on `0.0.0.0:0` and spawns the poller; tests return a loopback
/// transport. Called once per `StartJoin`/`Retry` so each join uses a new socket.
pub type TransportFactory = Box<dyn Fn() -> Box<dyn PacketIO>>;

pub struct App { /* private fields */ }

impl App {
    /// `factory` creates a socket per join; `headless` skips the menu and auto-walks
    /// Connecting → Playing.
    pub fn new(factory: TransportFactory, headless: bool) -> Self;
    pub fn state(&self) -> AppState;
    /// Apply an event; runs enter/exit side effects (bind, `JoinRequest`, `Leave`,
    /// `World` drop, deadline init). `now` is needed to set timeouts.
    pub fn dispatch(&mut self, event: AppEvent, now: Instant);
    /// Inject per-frame gameplay input (sampled by the binary main; see note).
    pub fn set_game_input(&mut self, left: bool, right: bool, jump: bool, shoot: bool);
    /// Forward the render viewport to the live `World`'s camera. No-op unless
    /// `Playing`/`Paused`.
    pub fn set_viewport(&mut self, width: f32, height: f32);
    /// Per-frame update: handshake polling, watchdog, accumulator (Playing only).
    pub fn update(&mut self, now: Instant);
    /// Per-state draw (menu / connecting / world / paused overlay / disconnected).
    pub fn draw(&self);
    pub fn is_quitting(&self) -> bool;
}
```

`App` owns: current `AppState`, `Option<Box<dyn PacketIO>>` (mutually exclusive
with `World`: `Some` standalone only in `Connecting`; in `Playing`/`Paused` the socket
is moved into `World`), `Option<World>` (`Some` only in `Playing`/`Paused`, owning the
socket), `Option<u8>` assigned id, `FixedTimestep`, `connect_deadline`,
`last_player_pos_deadline`, the `TransportFactory`, and last-frame time for `frame_dt`.

**Input boundary (matches Phase 2g precedent):** the binary `main.rs` samples
macroquad keys once per frame, maps UI keys to `AppEvent`s, calls
`app.dispatch(event, now)`, and calls `app.set_game_input(...)` for move/jump/shoot.
`App::update`/`dispatch` are macroquad-free (except `draw`), so the handshake/watchdog/
lifecycle logic runs under `#[cfg(test)]` and in the e2e crate.

## Test cases (tester writes these RED first)

> These are the spec's six test cases, reproduced verbatim so the tester can work
> directly from the plan. Cases 1–3 are `game`-crate unit tests; cases 4–6 are `e2e`
> integration tests. There is one additional `game`-crate unit test (the config
> constants, AC5) and one `client` unit test (the handshake helper, AC6) called out
> below the numbered list.

1. **Transition edges** — for each of the 16 table rows, assert
   `transition(from, event) == to` (all 6 `AppState` variants × the listed events).
2. **No-op grid** — exhaustive over the 6×11 = 66 `(state, event)` pairs; assert the
   50 non-table pairs return the input state unchanged (and that the 16 table pairs
   do *not*). Loop over all 6 states × 11 events programmatically.
3. **`Quitting` terminal** — for every event, `transition(Quitting, ev) == Quitting`.
4. **Handshake e2e** — drive the app (or `poll_join_response` directly over a
   `LoopbackTransport`): send `JoinRequest`, inject `JoinResponse{assigned_id}`,
   assert `Joined`/`Playing`; and with no `JoinResponse`, advance a virtual clock
   past `CONNECT_TIMEOUT_MS`, assert `ConnectFailed`/`Disconnected`.
5. **Disconnect watchdog e2e** — reach `Playing`, then advance a virtual clock past
   `DISCONNECT_TIMEOUT_MS` with no `PlayerPos`, assert `ConnectionLost`; and a
   separate case that injects a `PlayerPos` before the deadline and asserts the
   watchdog resets (no `ConnectionLost`).
6. **Quit `Leave` e2e** — from `Playing`, dispatch `Quit`, assert exactly one
   `Leave { id }` was sent (with the assigned id) and the socket/`World` are dropped;
   and a case with no id assigned asserts **no** `Leave` is sent.

Additional tester-owned cases:

- **Config constants (AC5)** — in `game/src/config.rs`'s existing `#[cfg(test)]`
  module, `assert_eq!(CONNECT_TIMEOUT_MS, 5000)` and
  `assert_eq!(DISCONNECT_TIMEOUT_MS, 1000)`.
- **Handshake helper unit (AC6)** — lives in the `client` crate (`handshake.rs`
  `#[cfg(test)]`) and uses a **private local test double** (a small `DummyPacketIO`
  with an in-memory packet queue, like the existing one in `client/src/world/game.rs`
  tests) — *not* `LoopbackTransport`, which lives in the `e2e` crate. With a
  `JoinResponse { assigned_id }` queued it returns
  `HandshakeResult::Joined { assigned_id }` (discarding packets queued before it);
  with an empty queue it returns `HandshakeResult::Pending`; never blocks.

### The 16 valid edges (the exact table under test)

| State | Event | Next |
| --- | --- | --- |
| `MainMenu` | `StartJoin` | `Connecting` |
| `MainMenu` | `Quit` | `Quitting` |
| `Connecting` | `Joined` | `Playing` |
| `Connecting` | `ConnectFailed` | `Disconnected` |
| `Connecting` | `Cancel` | `MainMenu` |
| `Playing` | `PauseRequested` | `Paused` |
| `Playing` | `QuitToMenu` | `MainMenu` |
| `Playing` | `ConnectionLost` | `Disconnected` |
| `Playing` | `Quit` | `Quitting` |
| `Paused` | `ResumeRequested` | `Playing` |
| `Paused` | `QuitToMenu` | `MainMenu` |
| `Paused` | `ConnectionLost` | `Disconnected` |
| `Paused` | `Quit` | `Quitting` |
| `Disconnected` | `Retry` | `Connecting` |
| `Disconnected` | `BackToMenu` | `MainMenu` |
| `Disconnected` | `Quit` | `Quitting` |

Everything else (the other 50 of the 66 `(state, event)` pairs, including all 11
events from `Quitting`) is a no-op returning the input state.

## RED-tests-first mapping

The Standard pipeline requires the tester to write failing tests **before** the
implementer writes the corresponding code. The mapping is:

| RED test(s) | Location | Must land RED **before** the implementer starts |
| --- | --- | --- |
| TC1 (16 edges), TC2 (no-op grid), TC3 (`Quitting` terminal) | `game/src/state_machine.rs` `#[cfg(test)]` (or `game` tests) | Box 1 |
| AC5 config-constant asserts | `game/src/config.rs` `#[cfg(test)]` | Box 1 |
| AC6 handshake-helper unit | `client/src/network/handshake.rs` `#[cfg(test)]` (or `client` tests) | Box 2 |
| TC4 (handshake e2e), TC5 (watchdog e2e), TC6 (quit-`Leave` e2e) | `e2e/tests/state_machine.rs` (new) | Box 3 |

The `transition` unit tests (TC1–TC3) and the config test will fail to **compile**
(no `game::state_machine` module / constants yet). The handshake unit test fails to
compile (no `handshake` module). The e2e tests fail to compile (no `metroidvania_rust::app`).
All of these are legitimate RED states. The implementer turns them green in Boxes 1–3
(in order).

**Verified by code/diff review only (no RED test):**

- **AC11** — `Paused` keeps receiving/processing packets but performs no `world.update`
  and sends no `PlayerInput`. *Partially* e2e-testable (assert no `PlayerInput` is
  emitted over the loopback channel while `Paused`, and that an injected packet is
  still processed); the "no accumulator advance" half is code review (see Box 5.6).
- **AC12** — Only `Playing` advances the `FixedTimestep` accumulator. Code review.
- **AC13** — GUI binary is a thin loop over `App`; headless reuses the same handshake
  + state machine and never sits in `MainMenu`/`Paused`. Code review.
- **AC14** — No changes to `protocol/` or `server/`. Diff review.
- **AC15** — `cargo test` green across all crates; `cargo fmt --check` and
  `cargo clippy -- -D warnings` clean. CI gate (Box 5.7).

## Ordering & dependencies

Execution order (RED first, then green in dependency order):

1. **Tester (RED):** TC1–TC3 + AC5 config test (`game` crate); AC6 handshake unit
   (`client` crate); TC4–TC6 e2e (`e2e` crate). All fail to compile/run.
2. **Implementer — Box 1** (pure core + config) → turns TC1–TC3 + AC5 green.
3. **Implementer — Box 2** (handshake + split + liveness) → turns AC6 green; keeps
   all existing regression tests green (the `update_network` refactor is behavior-
   preserving).
4. **Implementer — Box 3** (`App` + `TransportFactory`) → makes the e2e TC4–TC6
   compile and pass (the `App` they target now exists).
5. **Implementer — Box 4** (`main.rs` rewrite) → no new RED tests, but must keep the
   full suite green (the e2e tests exercise the same `App`/handshake the binary now
   uses). Satisfies AC13 by review.
6. **Reviewer (code + diff):** AC11/AC12/AC13 by code review, AC14 by diff review.
7. **QA / CI:** AC15 — `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings`.

**Dependency notes:**

- Box 1 has **no** dependencies (pure, new module).
- Box 2 has no dependency on Box 1. The receive/send split references the
  pre-existing `game::config::STALENESS_THRESHOLD_TICKS` (the staleness check that
  moves into `recv_and_process`), not the two new constants — those are consumed by
  the Box 3 runner. The handshake helper needs only `PacketIO`/`GamePacket` (already
  present). Box 2 also depends on the `World.received_player_pos` field (same box)
  for `handle_packet`.
- Box 3 depends on Box 1 (`AppState`/`AppEvent`/`transition` + config constants) and
  Box 2 (`poll_join_response`, `recv_and_process`/`send_input`, `received_player_pos`).
- Box 4 depends on Box 3 (`App`) and reuses Box 2's handshake helper.
- Box 5 (tests) is written by the tester **first**; it is numbered last to reflect
  "everything is green here," but its RED versions gate Boxes 1–3 as shown above.

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Box 1 — Pure core + config (`game` crate)

- [ ] 1.1 Create `game/src/state_machine.rs` with `AppState` (6 variants),
      `AppEvent` (11 variants), and the total `transition(state, event) -> AppState`
      implementing the 16-edge table, the no-op default, and terminal `Quitting`.
- [ ] 1.2 Add `CONNECT_TIMEOUT_MS: u64 = 5000` and `DISCONNECT_TIMEOUT_MS: u64 = 1000`
      to `game/src/config.rs` (with the doc comments from the spec).
- [ ] 1.3 Wire `pub mod state_machine;` into `game/src/lib.rs`.

- **Acceptance**: `cargo build -p game` succeeds; `AppState` has exactly the 6
  variants, `AppEvent` exactly the 11 variants; `transition` is a total `fn` (no
  `panic!`, no `Option`/sentinel return). Config constants exist with the exact
  values.
- **Test**: RED `game`-crate unit tests TC1 (16 edges), TC2 (50 no-ops), TC3
  (`Quitting` terminal), plus the AC5 config-constant asserts. Green when the
  transition table + constants are implemented correctly.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 2 — Handshake helper + receive/send split + liveness flag (`client` crate)

- [ ] 2.1 Create `client/src/network/handshake.rs` with `HandshakeResult`
      (`Joined { assigned_id: u8 }` | `Pending`) and
      `poll_join_response(io: &mut dyn PacketIO) -> HandshakeResult`
      (non-blocking, drains `try_recv`, returns `Joined` on the first
      `JoinResponse`, discarding anything queued before it).
- [ ] 2.2 Wire `pub mod handshake;` into `client/src/network/mod.rs`.
- [ ] 2.3 Split `update_network` into `recv_and_process(world)` + `send_input(world)`;
      keep `update_network` as `{ recv_and_process(world); send_input(world); }`.
      The existing staleness-check block (the `if world.id.is_some() { … }` clock
      comparison) moves into `recv_and_process` (it is a receive-side concern,
      post-drain), so `update_network`'s observable behavior is unchanged.
- [ ] 2.4 Add `pub received_player_pos: bool` to `World` (default `false` in
      `World::new`), and set `world.received_player_pos = true` in `handle_packet`'s
      `PlayerPos` arm for **both** the own-player and remote-player branches.

- **Acceptance**: `cargo build -p metroidvania-rust` succeeds; `poll_join_response`
  returns `Joined`/`Pending` per spec and never blocks; `recv_and_process` +
  `send_input` reproduce `update_network`'s behavior exactly (existing network tests
  stay green); `received_player_pos` flips to `true` on any `PlayerPos`.
- **Test**: RED handshake-helper unit test (AC6). Regression: all existing
  `client`/`e2e` tests remain green (the split is behavior-preserving).
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 3 — `App` runner + `TransportFactory` (`client` crate)

- [ ] 3.1 Create `client/src/app.rs` defining `TransportFactory`
      (`Box<dyn Fn() -> Box<dyn PacketIO>>`) and the `App` struct with the exact
      method set from the Public API section (private fields as listed in the spec:
      state, `Option<Box<dyn PacketIO>>` (`Some` standalone only in `Connecting`),
      `Option<World>` (`Some` only in `Playing`/`Paused`, owning the socket),
      `Option<u8>` id, `FixedTimestep`, `connect_deadline`,
      `last_player_pos_deadline`, the factory, last-frame time).
- [ ] 3.2 Implement `dispatch` enter/exit side effects per the spec's "Per-state
      behavior" section: `StartJoin`/`Retry` bind a fresh socket via `factory()`,
      `send_join_req`, record `connect_deadline`; `Joined` stores `assigned_id`,
      builds `World::new(Map::new(), io)`, sets `world.id`/`world.player.id`; `Cancel`
      drops the socket with **no** `Leave`; `QuitToMenu`/`Quit` send `Leave { id }`
      iff an id is assigned, then drop `World` + socket; `ConnectionLost`/
      `ConnectFailed` drop without sending `Leave`.
- [ ] 3.3 Implement `update(now)`: in `Connecting`, call `poll_join_response` and
      apply the timeout; in `Playing`, advance the `FixedTimestep` accumulator
      (frame_dt from `now` vs last-frame time), run 0..=N `world.update(tick_dt)`,
      and run the disconnect watchdog; in `Paused`, call `recv_and_process` only (no
      stepping, no send) and still run the watchdog; initialize
      `last_player_pos_deadline = now + DISCONNECT_TIMEOUT_MS` on entering
      `Playing`/`Paused` (grace period), then each frame read/clear
      `world.received_player_pos` to reset the deadline, or fire `ConnectionLost`
      when `now >= deadline`. **`Joined` handoff:** `update` stashes `assigned_id`
      from a `poll_join_response() == Joined` result, then calls
      `self.dispatch(AppEvent::Joined, now)`; `dispatch` constructs `World::new(...)`
      on entry to `Playing` using the stashed id — the `World` is built exactly once
      (in `dispatch`), never in `update`.
- [ ] 3.4 Implement `draw()` per state (menu / connecting / world-with-alpha /
      paused overlay / disconnected), `state()`, `is_quitting()`, `set_game_input`.
- [ ] 3.5 Wire `pub mod app;` into `client/src/lib.rs` (and any ergonomic
      re-exports the e2e crate needs).
- [ ] 3.6 Implement `set_viewport(width: f32, height: f32)`: forwards to
      `world.camera.set_viewport(width, height)` when in `Playing`/`Paused`; no-op
      otherwise.

- **Acceptance**: `cargo build -p metroidvania-rust` succeeds; the `App` owns state,
  socket, `World`, id, clock and factory exactly as specified; `dispatch`/`update`
  are macroquad-free (only `draw` touches macroquad).
- **Test**: RED e2e tests TC4 (handshake), TC5 (watchdog), TC6 (quit-`Leave`) — they
  compile and pass once the `App` exists and behaves per spec.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 4 — `main.rs` rewrite (thin loop over `App`)

- [ ] 4.1 Rewrite the GUI path as a thin loop over `App`: parse args, build a
      laminar `TransportFactory` (closure capturing `server_addr` returning a fresh
      `RealPacketIO` per call), `App::new(factory, headless)`, then loop
      `{ sample keys → dispatch(event, now) / set_game_input →
      set_viewport(screen_width(), screen_height()) → update(Instant::now()) →
      draw() → break on is_quitting() → next_frame().await }`. Map UI keys to
      `AppEvent`s (Enter/Space → `StartJoin` on the menu, Esc → `Cancel`/`QuitToMenu`
     /`BackToMenu` per state, P → `PauseRequested`/`ResumeRequested`, Q → `Quit`).
- [ ] 4.2 Rewrite `run_headless` to reuse the same `App` + handshake helper: dispatch
      `StartJoin` immediately (never sit in `MainMenu`), never map `PauseRequested`,
      and dispatch `Quit` on `Disconnected` or when `--duration` elapses. Preserve the
      existing hybrid sleep + busy-wait pacing (`HEADLESS_BUSY_WAIT_MS`) as the frame
      driver.
- [ ] 4.3 Keep the existing `RealPacketIO`/`HeadlessPacketIO` wrappers (move them
      into the `TransportFactory` construction, or keep as-is and call from the
      closures). **Preserve the `"Headless client finished after {} ticks"` stdout
      line** — `e2e/tests/headless_timing.rs` still parses it, but now runs the
      client against a live server (Box 5.8), so headless genuinely reaches
      `Playing` and runs the full duration.

- **Acceptance**: binary compiles and runs; GUI and headless share the same `App`,
  handshake helper, and state machine; headless never sits in `MainMenu`/`Paused`
  and quits on `Disconnected`/duration. Headless now **requires a live server** to
  reach `Playing` — a dead port fires `ConnectFailed` (5 s) instead of running the
  tick loop. No new RED tests; full suite stays green.
- **Test**: Code review (AC13). Regression: the e2e binary-spawning tests
  (`e2e/tests/headless_timing.rs`, `e2e/tests/packet_log_rate.rs`) stay green **after
  being updated to spawn a live server** (Box 5.8); the new e2e `state_machine.rs`
  tests also stay green.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

### Box 5 — Tests (tester-owned; written RED before Boxes 1–3)

- [ ] 5.1 `game` crate: `transition` unit tests TC1 (16 edges), TC2 (50 no-ops via a
      6×11 programmatic loop), TC3 (`Quitting` terminal), plus the AC5 config-constant
      asserts in `game/src/config.rs`.
- [ ] 5.2 `client` crate (`handshake.rs` `#[cfg(test)]`): handshake-helper unit test
      AC6 (`poll_join_response` over a **private local `DummyPacketIO` test double** —
      `Joined` on `JoinResponse`, `Pending` otherwise, no block). Do **not** use
      `LoopbackTransport` (it lives in the `e2e` crate).
- [ ] 5.3 `e2e/tests/state_machine.rs`: handshake e2e (TC4 / AC7+AC8) — `StartJoin`
      sends `JoinRequest` (observable on the loopback server-side channel); inject
      `JoinResponse` → state `Playing` and `world.id == assigned_id`; and the
      no-response case advancing `VirtualClock` past `CONNECT_TIMEOUT_MS` →
      `ConnectFailed` → `Disconnected`.
- [ ] 5.4 `e2e/tests/state_machine.rs`: disconnect watchdog e2e (TC5 / AC9) — reach
      `Playing`, advance past `DISCONNECT_TIMEOUT_MS` with no `PlayerPos` →
      `ConnectionLost`; and a reset case injecting a `PlayerPos` before the deadline
      (assert no `ConnectionLost`).
- [ ] 5.5 `e2e/tests/state_machine.rs`: quit-`Leave` e2e (TC6 / AC10) — from
      `Playing`, dispatch `Quit`, assert exactly one `Leave { id }` with the assigned
      id on the server-side channel and `World`/socket dropped; and a no-id case
      asserting no `Leave` is sent.
- [ ] 5.6 `e2e/tests/state_machine.rs` (optional, AC11 partial): from `Playing`,
      dispatch `PauseRequested` → `Paused`; assert no `PlayerInput` is emitted over
      the loopback channel while paused, and that an injected packet (e.g. `Leave`)
      is still processed. The "accumulator not advanced" half is code review.
- [ ] 5.7 Final quality gate (AC15): `cargo test` green in every crate,
      `cargo fmt --check`, `cargo clippy -- -D warnings` clean.
- [ ] 5.8 e2e process helpers (shared "spawn server + client"): add a shared helper
      (e.g. `e2e/src/process.rs`, re-exported from `e2e::`) that spawns the server
      binary (`cargo run -p metroidvania-server`, binds `127.0.0.1:12345`), spawns
      the headless client binary (`--headless --duration=N --server=127.0.0.1:12345`),
      returns the client stdout/elapsed, and kills the server when the test ends
      (guard the handle with `Drop`). Refactor `e2e/tests/headless_timing.rs`
      (currently points at a dead port `127.0.0.1:9`, which now fires `ConnectFailed`
      at 5 s) and `e2e/tests/packet_log_rate.rs` (already spawns a server inline) to
      use it. The `"finished after N ticks"` assertion now runs against a live server.

- **Acceptance**: All tests compile and pass (green) after Boxes 1–4 are implemented;
  they are RED before. AC11/AC12/AC13 (review) and AC14 (diff review) verified by the
  reviewer, not by these tests.
- **Test**: self (this is the test box). Each sub-box names its own AC.
- **Explain**
  - What:
  - Why:
  - Rust concepts:
  - Watch out:

## Notes for implementer & tester

- **Observability of `world.id == assigned_id` (AC7/TC4):** `App` has no `world()`
  accessor in the spec's public API, so the e2e test asserts `state() == Playing`
  and then proves the id flowed into the world **via the observable `PlayerInput`
  packet**: after a `Playing` tick, `send_input` reads `world.id` and emits
  `PlayerInput { id, .. }` on the loopback server-side channel; assert that packet's
  `id == assigned_id`. This is a faithful observable of `world.id` without inventing
  a new accessor.
- **`App::update` drives `World::update`** in `Playing`. `World::update` is already
  macroquad-free (Phase 2g Box 2a injected input + viewport); it does use
  `macroquad::rand::gen_range` for enemy AI, which is a window-free PRNG re-export.
  If any e2e failure surfaces as a missing macroquad context, run that test under
  Xvfb (already configured in CI) rather than adding a window.
- **`update_network` refactor (Box 2.3) is behavior-preserving.** The staleness block
  must stay inside `recv_and_process` so `update_network`'s order of operations
  (drain → staleness → send) is unchanged. Existing reconciliation/multiplayer e2e
  tests are the regression net.
- **`poll_join_response` discards pre-`JoinResponse` packets** (per the spec's
  doc-comment) but must not consume the `JoinResponse` itself past returning
  `Joined` — it returns on the first `JoinResponse`.
- **`Transition` is total — no `unreachable!()`/`panic!()`.** Use a `match` with the
  16 edges and a wildcard `_ => state` arm (or equivalent), plus an explicit
  `Quitting => Quitting` arm. The no-op grid test (TC2) will fail if any edge is
  missing or any no-op mutates state.
- **Watchdog grace period (edge case 9):** initialize `last_player_pos_deadline` on
  entering `Playing`/`Paused` so a freshly joined client is not dropped before the
  first `PlayerPos` arrives.
- **`Leave` is sent only when an id is assigned** (edge case 10), and never on
  `ConnectFailed`/`ConnectionLost`/`Cancel`. TC6 covers both the id-assigned and
  no-id cases.
- **Naming:** use English identifiers/comments; use the named constants
  (`CONNECT_TIMEOUT_MS`, `DISCONNECT_TIMEOUT_MS`, `MAX_ACCUMULATED_FRAMES`, `TICK_HZ`)
  — no magic numbers. Conventional commits scoped `game` / `client` / `e2e` / `docs`.
- **Production `TransportFactory` binds `0.0.0.0:0` and must `.unwrap()`** — the
  factory type returns `Box<dyn PacketIO>` (no `Result`), matching the existing
  binary's `Socket::bind(...).unwrap()`. Deliberate exception to the AGENTS.md
  preference for `Result`/`?` (an ephemeral-port bind is effectively infallible);
  flagged here because the convention prefers `Result`.
- **Do not touch `protocol/` or `server/`** — the diff-review gate (AC14) fails the
  whole feature if either shows changes.

## Acceptance-criteria → verification map

| AC | Criterion | Verified by |
| --- | --- | --- |
| AC1 | `game::state_machine` exposes `AppState` (6), `AppEvent` (11), `transition` | compile (TC1–TC3 reference them) |
| AC2 | 16 valid edges | RED TC1 |
| AC3 | 50 no-op pairs unchanged | RED TC2 |
| AC4 | `Quitting` terminal | RED TC3 |
| AC5 | `CONNECT_TIMEOUT_MS = 5000`, `DISCONNECT_TIMEOUT_MS = 1000` | RED config asserts |
| AC6 | `poll_join_response` `Joined`/`Pending`, non-blocking | RED handshake unit |
| AC7 | `StartJoin` → `JoinRequest`; `JoinResponse` → `Playing`, `world.id == assigned_id` | RED TC4 |
| AC8 | connect timeout → `ConnectFailed` → `Disconnected` | RED TC4 |
| AC9 | disconnect watchdog fires + resets | RED TC5 |
| AC10 | `Quit`/`QuitToMenu` send one `Leave`; no-id sends none | RED TC6 |
| AC11 | `Paused` receives, no `update`, no `PlayerInput` | e2e 5.6 (partial) + code review |
| AC12 | only `Playing` advances accumulator | code review |
| AC13 | GUI thin loop; headless reuses state machine | code review |
| AC14 | no `protocol/`/`server/` changes | diff review |
| AC15 | `cargo test` / `fmt --check` / `clippy -D warnings` | CI (Box 5.7) |

## Open questions

None. The spec is owner-approved and settled; ADR-0002 records the decisions. The
spec's own watch-outs (server `Leave` trusts packet `id`, no asset pipeline, flat
enum over stack, no lobby phase, `Connecting` has no `Quit` edge, headless window)
are all recorded follow-ups **out of scope** for this plan.
