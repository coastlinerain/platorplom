# Game State Machine

- Status: implemented
- Date: 2026-11-25
- Branch: `feat/game-state-machine`

## Summary

The client currently has no application-level state: `client/src/main.rs` hardcodes a
single connect-and-play path — it binds a socket, sends `JoinRequest`, immediately
constructs a `World`, and loops forever. There is no main menu, no pause, no
disconnect/quit handling, and no way to recover from a failed or lost connection.

This feature introduces a client-only **flat state machine** that owns the client's
lifecycle: `MainMenu → Connecting → Playing ⇄ Paused`, with `Disconnected` for
failure/loss and terminal `Quitting`. The pure transition core lives in the `game`
crate (RED-testable), and a new `client::app` runner owns the current state, the
socket, the `World`, input sampling, and per-state drawing.

The server is **unchanged**: it remains drop-in-join. No `GamePacket` variant is
added, and `ServerWorld` is untouched.

## Goals / non-goals

### Goals

- **G1**: A pure, total `transition(state, event) -> AppState` function in `game`,
  with exactly 16 valid edges and all other pairs as no-ops (state unchanged).
- **G2**: A client runner (`client::app`) that owns the socket lifecycle, `World`
  lifecycle, input sampling, and per-state drawing.
- **G3**: Lazy `World` construction — `World` is built only after `JoinResponse`.
- **G4**: Bind-per-join socket lifecycle — each `StartJoin`/`Retry` binds a fresh
  ephemeral socket; leaving `Playing`/`Paused`/`Connecting`/`Disconnected` drops it.
- **G5**: Connect timeout (5 s) and disconnect watchdog (1 s) fire `ConnectFailed`
  and `ConnectionLost` respectively.
- **G6**: `Paused` freezes simulation stepping and input sending but keeps receiving
  and processing server packets.
- **G7**: Quit / back-to-menu send `Leave` (new client behavior) when an id is
  assigned, then drop the socket.
- **G8**: Headless mode (`--headless`) bypasses the menu and walks
  `Connecting → Playing` through the same handshake helper.

### Non-goals

- **No server changes** — drop-in-join stays; no `ServerWorld` change; no protocol
  change (see *Network*).
- **No nested menus** — a state *stack* is deferred until nested menus (e.g.
  pause → settings → back) appear.
- **No real `Loading` state** — there is no asset pipeline (`Map::new` is instant,
  no textures), so `Connecting` is honest naming; a future `Loading` sub-state is a
  separate feature.
- **No server "lobby / game-not-started" phase** — that is a future protocol +
  `ServerWorld` change.
- **No headless window fix** — the `--headless` mode still runs inside the macroquad
  binary (known from Phase 2g; recorded as a follow-up, out of scope here).

## Behavior

### States

A flat enum (not a stack), with a single terminal state:

| State | Meaning |
| --- | --- |
| `MainMenu` | Idle, no socket, no `World`. |
| `Connecting` | Socket bound, `JoinRequest` sent, awaiting `JoinResponse`. |
| `Playing` | `World` live; full sim + network + draw. |
| `Paused` | `World` alive but frozen (sim + input send), still receiving. |
| `Disconnected` | Connection failed/lost; socket dropped. |
| `Quitting` | Terminal; the app loop exits. |

`MainMenu` is the initial state.

### Events

```
StartJoin, Joined, ConnectFailed, ConnectionLost,
PauseRequested, ResumeRequested, QuitToMenu, Cancel,
Retry, BackToMenu, Quit
```

`Joined` is a unit variant: it carries **no** payload. The assigned player id is
carried by the handshake result (see *Data model*) and stored on the runner, not
baked into the event, so `transition` remains a pure 6×11 total function.

### Transition table (the pure core)

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

**No-op rule (normative):** any `(state, event)` pair not in this table returns the
input `state` unchanged. Equivalently, the valid events per state are:

- `MainMenu`: `StartJoin`, `Quit`
- `Connecting`: `Joined`, `ConnectFailed`, `Cancel`
- `Playing`: `PauseRequested`, `QuitToMenu`, `ConnectionLost`, `Quit`
- `Paused`: `ResumeRequested`, `QuitToMenu`, `ConnectionLost`, `Quit`
- `Disconnected`: `Retry`, `BackToMenu`, `Quit`
- `Quitting`: *(none)*

`Quitting` is terminal: **no event leaves it** (all 11 events are no-ops). The
6×11 = 66 grid minus the 16 valid edges leaves 50 no-op pairs, which must all be
exhaustively tested.

### Per-state behavior and lifecycle side effects

Side effects run on **state entry/exit** (driven by the runner's `dispatch`), not
inside the pure `transition` function.

#### `MainMenu`

- Holds **no socket** and **no `World`**.
- Draws the menu.
- `StartJoin`: bind a fresh ephemeral socket (`0.0.0.0:0`), spawn the laminar
  poller thread, wrap in `Box<dyn PacketIO>`, send `JoinRequest` (via the existing
  `send_join_req`), record `connect_deadline = now + CONNECT_TIMEOUT_MS`, then enter
  `Connecting`. (In production the socket is laminar; tests inject a loopback
  transport through the `TransportFactory` seam.)
- `Quit`: enter `Quitting`.

#### `Connecting`

- Holds `Box<dyn PacketIO>` **standalone** (no `World` yet).
- Each frame, call the handshake helper `poll_join_response(io)`:
  - `HandshakeResult::Joined { assigned_id }` → store `assigned_id`, construct
    `World::new(Map::new(), io)`, set `world.id = Some(assigned_id)` and
    `world.player.id = assigned_id`, then fire `Joined` → `Playing`.
  - `HandshakeResult::Pending` and `now >= connect_deadline` → fire
    `ConnectFailed` → `Disconnected` (drop the socket).
  - `HandshakeResult::Pending` and `now < connect_deadline` → stay.
- `Cancel`: drop the socket (no id assigned, so **no `Leave`**), enter `MainMenu`.
- `ConnectFailed`: drop the socket, enter `Disconnected`.

#### `Playing`

- Runs the `FixedTimestep` accumulator: each render frame accumulates `frame_dt`,
  clamps to `MAX_ACCUMULATED_FRAMES` steps, runs 0..=N `world.update(tick_dt)` steps,
  and draws with `timestep.alpha()` (unchanged from Phase 2g).
- Each `world.update` receives+processes packets and sends one `PlayerInput`
  (send-once-per-tick, as established in Phase 2g).
- Watchdog: if no `PlayerPos` has been received for `DISCONNECT_TIMEOUT_MS`, fire
  `ConnectionLost` (see *Disconnect watchdog*).
- `PauseRequested`: enter `Paused`.
- `QuitToMenu`: send `Leave { id }` if an id is assigned, then **drop `World` and
  its socket**, enter `MainMenu`.
- `ConnectionLost`: **drop `World` and socket without sending `Leave`** (the
  connection is already dead), enter `Disconnected`.
- `Quit`: send `Leave { id }` if an id is assigned, drop `World` + socket, enter
  `Quitting`.

#### `Paused`

- `World` and socket stay alive, but:
  - **No simulation stepping** (the `FixedTimestep` accumulator is not advanced and
    `world.update` is not called).
  - **No input sending** (no `PlayerInput` is sent).
  - **Keep receiving + processing packets** (reconciliation, remote-player
    interpolation, `Leave`, `EnemyStateResponse`, etc. still apply to the `World`).
- Draws the frozen world with a `PAUSED` overlay.
- Watchdog still runs (see *Disconnect watchdog*).
- `ResumeRequested`: enter `Playing`.
- `QuitToMenu` / `ConnectionLost` / `Quit`: same side effects as from `Playing`.

#### `Disconnected`

- Holds no socket and no `World` (they were dropped on the transition into this
  state).
- Draws a disconnect screen.
- `Retry`: re-run the same bind-and-join steps as `StartJoin` (fresh socket, fresh
  `JoinRequest`, fresh `connect_deadline`), enter `Connecting`.
- `BackToMenu`: enter `MainMenu` (no cleanup — already disconnected).
- `Quit`: enter `Quitting`.

#### `Quitting`

- Terminal. The runner exposes `is_quitting() -> bool`; the main loop breaks and the
  process exits.

### Handshake helper

A small, non-blocking helper in `client::network` that the `Connecting` state uses
and that the headless path reuses (so both GUI and headless share the exact same
join logic):

```rust
pub enum HandshakeResult {
    Joined { assigned_id: u8 },
    Pending,
}

/// Drain pending packets looking for `JoinResponse`. Returns `Joined` with the
/// assigned id on the first `JoinResponse` (discarding any packets that precede
/// it), or `Pending` if the queue empties without one. Non-blocking (`try_recv`).
pub fn poll_join_response(io: &mut dyn PacketIO) -> HandshakeResult;
```

The timeout decision is the **caller's** responsibility: the runner compares
`Instant::now()` against `connect_deadline` and fires `ConnectFailed` when
`poll_join_response` keeps returning `Pending` past the deadline. Keeping the
timeout out of the helper makes it deterministically testable with a virtual clock.

### Connect timeout

`CONNECT_TIMEOUT_MS = 5000`. Recorded when entering `Connecting`
(`connect_deadline = now + CONNECT_TIMEOUT_MS`). Expiry fires `ConnectFailed`.

### Disconnect watchdog

`DISCONNECT_TIMEOUT_MS = 1000`. While in `Playing` or `Paused`, if no `PlayerPos`
packet (of **any** player id — the server broadcasts every player's position every
tick, so any `PlayerPos` proves the link is alive) has been processed for
`DISCONNECT_TIMEOUT_MS`, fire `ConnectionLost`.

Mechanics (testable with a virtual clock):

- `World` gains a liveness flag `received_player_pos: bool` (default `false`), set
  to `true` by `handle_packet` in the `PlayerPos` arm (both own and remote).
- The runner keeps `last_player_pos_deadline: Option<Instant>`. On entering
  `Playing`/`Paused` it initializes `last_player_pos_deadline = now + DISCONNECT_TIMEOUT_MS`
  (grace period). Each frame, after stepping/draining, it reads the flag: if set,
  it resets `last_player_pos_deadline = now + DISCONNECT_TIMEOUT_MS` and clears the
  flag; if `now >= last_player_pos_deadline`, it fires `ConnectionLost`.

The runner takes `now: Instant` as a parameter (production passes `Instant::now()`;
tests pass a virtual clock), so the watchdog is deterministic.

### Quit / back-to-menu `Leave` semantics

- `Leave { id }` is sent **only** when an id is assigned (i.e. from `Playing` or
  `Paused`, never from `Connecting`/`Disconnected`/`MainMenu`).
- The socket is dropped after sending `Leave`.
- `ConnectionLost` and `ConnectFailed` do **not** send `Leave` (the link is already
  gone).
- Today the client **never** sends `Leave` — this is new client behavior; the server
  already handles `Leave` (see *Network*).

### Headless (`--headless`)

- Uses the **same** `App` runner and handshake helper as the GUI.
- On startup, immediately dispatches `StartJoin` (never sits in `MainMenu`), walks
  `Connecting → Playing` via `poll_join_response`, and never enters `Paused`
  (pause input is ignored / not mapped).
- On reaching `Disconnected` (via `ConnectFailed` or `ConnectionLost`), or when the
  `--duration` elapses, it dispatches `Quit` and terminates.
- The existing hybrid sleep + busy-wait pacing (`HEADLESS_BUSY_WAIT_MS`) remains the
  headless frame driver; within `Playing` the `FixedTimestep` accumulator drives
  simulation.
- **e2e tests that spawn the headless binary must also spawn a live server.**
  Headless now genuinely connects (G8), so it must reach `Playing` against a real
  server bound to `127.0.0.1:12345`. The e2e crate provides a shared "spawn server +
  headless client" helper; the server process is killed when the test ends. Tests
  that spawn the headless client against a dead port (or assume local-only looping)
  will now fail and must be updated (see *Files to modify*).

### Timing

- Only `Playing` runs the `FixedTimestep` accumulator (`game::fixed_timestep`).
- `MainMenu`, `Connecting`, `Disconnected`, and `Paused` render free-run (no
  fixed-step simulation; `Paused` draws the frozen world).

### Edge cases (explicit)

1. **No-op events.** Every event not in the table is a no-op (state unchanged).
2. **`Quitting` terminal.** No event leaves `Quitting`.
3. **Double `StartJoin` impossible.** After `StartJoin` the state is `Connecting`,
   where `StartJoin` is a no-op — so no double socket bind.
4. **`Cancel` during `Connecting`.** Drops the socket with no `Leave` (no id yet).
5. **`Quit` during `Connecting` is a no-op** (not in the table). The user must
   `Cancel` → `MainMenu` → `Quit`. Recorded as a follow-up; see *Watch-outs*.
6. **`ConnectionLost` during `Connecting` is a no-op** — the only failure path in
   `Connecting` is `ConnectFailed` (the connect timeout), because there is no id and
   no `PlayerPos` stream yet.
7. **`ResumeRequested` outside `Paused` is a no-op** (e.g. pressing resume in
   `Playing` does nothing).
8. **`Retry` only in `Disconnected`.** `Retry` rebinds a fresh socket (never reuses
   a stale one).
9. **Grace period.** The disconnect watchdog starts on entering `Playing`/`Paused`,
   so a freshly joined client is not immediately dropped before the first `PlayerPos`
   arrives.
10. **`Leave` when id absent.** `QuitToMenu`/`Quit` from a state with no id send no
    `Leave` (defensive; cannot normally happen from `Playing`/`Paused`).

## Data model

### `game` crate — pure core

New module `game/src/state_machine.rs` (add `pub mod state_machine;` to
`game/src/lib.rs`):

```rust
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

`game/src/config.rs` — two new named constants:

```rust
/// Client connect timeout: give up waiting for `JoinResponse` after this long.
pub const CONNECT_TIMEOUT_MS: u64 = 5000;

/// Disconnect watchdog: fire `ConnectionLost` if no `PlayerPos` for this long.
pub const DISCONNECT_TIMEOUT_MS: u64 = 1000;
```

### `client` crate — handshake + runner

New module `client/src/network/handshake.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeResult {
    Joined { assigned_id: u8 },
    Pending,
}

pub fn poll_join_response(io: &mut dyn PacketIO) -> HandshakeResult;
```

`client/src/network/world.rs` — split `update_network` so `Paused` can receive
without sending, and expose the liveness flag:

```rust
/// Receive half: drain + handle_packet, then run the (currently inert)
/// staleness check. Public so `Paused` can receive/process packets without
/// stepping the sim or sending input.
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
(own-player and remote-player branches both count as liveness).

`client/src/world/game.rs` — add one field to `World`:

```rust
/// Set true by `handle_packet` when any `PlayerPos` is received; consumed by the
/// app runner's disconnect watchdog (cleared each frame after the check).
pub received_player_pos: bool,  // default false
```

New module `client/src/app.rs` (add `pub mod app;` to `client/src/lib.rs`):

```rust
use std::net::SocketAddr;   // (not required by App; addr is captured by the factory)
use std::time::Instant;

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
    /// Inject the current window viewport once per render frame (mirrors
    /// `set_game_input`). Forwards to `world.camera.set_viewport(width, height)`
    /// when in `Playing`/`Paused`; no-op otherwise (no `World` yet in menu/connecting/
    /// disconnected). The binary `main.rs` calls this each frame BEFORE `draw`.
    pub fn set_viewport(&mut self, width: f32, height: f32);
    /// Per-frame update: handshake polling, watchdog, accumulator (Playing only).
    pub fn update(&mut self, now: Instant);
    /// Per-state draw (menu / connecting / world / paused overlay / disconnected).
    pub fn draw(&self);
    pub fn is_quitting(&self) -> bool;
}
```

The `App` owns: current `AppState`, `Option<Box<dyn PacketIO>>` (held standalone
in `Connecting` only; moved into `World::new` on `Joined`), `Option<World>` (held
in `Playing`/`Paused`, and it owns the socket via `World.network`), `Option<u8>`
assigned id, `FixedTimestep`, `connect_deadline`, `last_player_pos_deadline`, the
`TransportFactory`, and last-frame time for `frame_dt`. The io and `World` are
mutually exclusive across states.

**Input boundary (matches Phase 2g precedent):** the binary `main.rs` samples
macroquad keys once per frame, maps UI keys to `AppEvent`s, calls
`app.dispatch(event, now)`, calls `app.set_game_input(...)` for move/jump/shoot, and
calls `app.set_viewport(screen_width(), screen_height())` each frame BEFORE `draw`
(this is the context-injection surface for `GameCamera`, alongside
`set_game_input`). The `App::update`/`dispatch` themselves are macroquad-free
(except `draw`), so the handshake/watchdog/lifecycle logic runs under `#[cfg(test)]`
and in the e2e crate.

### `client` binary (`main.rs`)

Rewritten as a thin loop over the `App`: parse args, build the laminar
`TransportFactory` (and the headless variant), construct `App::new(factory, headless)`,
then loop `{ sample keys → dispatch/set_input → app.set_viewport(screen_width(),
screen_height()) → update(Instant::now()) → draw → break on is_quitting() →
next_frame().await }`. The headless driver reuses the same `App` with the existing
sleep + busy-wait pacing.

## Network

- **No protocol change.** `protocol::GamePacket` is unchanged; no new variants.
- **No server change.** `ServerWorld` and `server/src/main.rs` are untouched.
- **New client behavior:** the client now sends `GamePacket::Leave { id }` on
  `Quit`/`QuitToMenu` (previously it never sent `Leave`). The server already handles
  `Leave` (removes the player and broadcasts to remaining clients).
- **Watch-out (latent server bug, out of scope):** the server `Leave` handler
  (`server/src/main.rs`, `GamePacket::Leave` arm) trusts the packet `id` for
  `world.remove(id)` and `broadcast_leave(...)` without validating that
  `id == addrs[addr]`. A malicious/buggy client could remove another player. This
  feature neither introduces nor fixes it, but it is recorded here for a future
  hardening pass.

## Acceptance criteria

| ID | Criterion | Test type |
| --- | --- | --- |
| AC1 | `game::state_machine` exposes `AppState` (6 variants), `AppEvent` (11 variants), and `transition(state, event) -> AppState`. | Unit (compile) |
| AC2 | `transition` implements exactly the 16 valid edges in the table. | Unit (grid) |
| AC3 | All 50 invalid `(state, event)` pairs return the input state unchanged. | Unit (grid) |
| AC4 | `Quitting` is terminal (no event changes it). | Unit (grid) |
| AC5 | `game::config` defines `CONNECT_TIMEOUT_MS = 5000` and `DISCONNECT_TIMEOUT_MS = 1000`. | Unit (compile) |
| AC6 | `client::network::handshake::poll_join_response` returns `Joined{assigned_id}` on `JoinResponse` and `Pending` otherwise, without blocking. | Unit |
| AC7 | e2e: `StartJoin` sends `JoinRequest`; on `JoinResponse` the app reaches `Playing` with `world.id == assigned_id`. | e2e |
| AC8 | e2e: no `JoinResponse` before `CONNECT_TIMEOUT_MS` fires `ConnectFailed` → `Disconnected`. | e2e |
| AC9 | e2e: no `PlayerPos` for `DISCONNECT_TIMEOUT_MS` fires `ConnectionLost` → `Disconnected`; a `PlayerPos` within the window resets the deadline. | e2e |
| AC10 | e2e: `Quit` (and `QuitToMenu`) from `Playing` send exactly one `Leave { id }`; `Quit` with no id assigned sends no `Leave`. | e2e |
| AC11 | `Paused` keeps receiving/processing packets but performs no `world.update` and sends no `PlayerInput`. | e2e / code review |
| AC12 | Only `Playing` advances the `FixedTimestep` accumulator. | code review |
| AC13 | GUI binary is a thin loop over `App`; headless reuses the same handshake + state machine and never sits in `MainMenu`/`Paused`. | code review |
| AC14 | No changes to `protocol/` or `server/`. | diff review |
| AC15 | `cargo test` green across all crates; `cargo fmt --check` and `cargo clippy -- -D warnings` clean. | CI |

## Test cases (tester writes these RED first)

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

The pure `transition` tests (1–3) live in the `game` crate (`#[cfg(test)]`). The
e2e tests (4–6) live in the `e2e` crate, driving the `App` through a
`TransportFactory` that returns a `LoopbackTransport` and using `VirtualClock` for
time.

**Headless e2e note:** tests that exercise the actual `--headless` binary (e.g.
`e2e/tests/headless_timing.rs`, `e2e/tests/packet_log_rate.rs`) must spawn a live
server binary alongside the headless client (server binds `127.0.0.1:12345`), so the
client genuinely reaches `Playing`. Use a shared helper in the e2e crate that starts
the server, waits for it to be ready, spawns the headless client, and kills the
server on test teardown. This is separate from the in-process `TestHarness` used by
tests 4–6 (which drive `App` directly over `LoopbackTransport` + `VirtualClock` and
does not need a real server).

## Design decisions (settled by the owner — do not re-litigate)

- **D1 — Client-only scope.** Server stays drop-in-join; no protocol or
  `ServerWorld` change.
- **D2 — Flat enum, not a stack.** A stack is only justified once nested menus
  (pause → settings → back) appear.
- **D3 — Lazy `World` + bind-per-join.** `World` is constructed only on `Joined`;
  each `StartJoin`/`Retry` binds a fresh ephemeral socket that is moved into
  `World::new`; leaving `Playing` drops it.
- **D4 — `Paused` freezes sim + input, keeps receive.** Packets are still processed
  while paused (reconciliation, remote interpolation), but no stepping and no send.
- **D5 — Timeouts as named constants.** `CONNECT_TIMEOUT_MS = 5000`,
  `DISCONNECT_TIMEOUT_MS = 1000` in `game::config`.
- **D6 — `transition` is total.** Invalid transitions are no-ops (state unchanged),
  never panics, never returns a sentinel/`Option`.

## Files to modify / create

| File | Change |
| --- | --- |
| `game/src/state_machine.rs` | NEW — `AppState`, `AppEvent`, `transition` + unit tests |
| `game/src/lib.rs` | Add `pub mod state_machine;` |
| `game/src/config.rs` | Add `CONNECT_TIMEOUT_MS`, `DISCONNECT_TIMEOUT_MS` |
| `client/src/network/handshake.rs` | NEW — `HandshakeResult`, `poll_join_response` |
| `client/src/network/world.rs` | Split `update_network` → `recv_and_process` + `send_input`; set `received_player_pos` in `handle_packet` `PlayerPos` arm |
| `client/src/world/game.rs` | Add `World.received_player_pos: bool` (default `false`) |
| `client/src/app.rs` | NEW — `App` runner, `TransportFactory`, lifecycle side effects, `set_viewport` |
| `client/src/lib.rs` | Add `pub mod app;`, `pub mod network::handshake;` re-exports |
| `client/src/main.rs` | Rewrite as a thin loop over `App` (GUI + headless); call `app.set_viewport(...)` before `app.draw()` |
| `e2e/src/…` / new tests | Handshake, watchdog, quit-`Leave` e2e tests; a shared "spawn server + headless client" helper; update `headless_timing.rs` and `packet_log_rate.rs` to spawn a live server |
| `protocol/`, `server/` | **No changes** |

## Watch-outs / follow-ups (recorded, not in scope)

1. **Server `Leave` trusts the packet id** for `world.remove(id)` +
   `broadcast_leave` without validating `id == addrs[addr]` — a latent bug that a
   malicious client could exploit to remove another player. Separate hardening
   phase.
2. **No asset pipeline today** (`Map::new` instant, no textures) — `Connecting` is
   honest naming; a future real `Loading` state would be a sub-state of
   `Connecting`.
3. **Flat enum over stack** — revisit only when nested menus appear.
4. **Server "lobby / game-not-started"** phase is a separate future phase (protocol
   + `ServerWorld` change).
5. **`Connecting` has no `Quit` edge** — per the settled table, a user must
   `Cancel` → `MainMenu` → `Quit` to exit while connecting. Possible UX follow-up
   (add `Connecting → Quit`) if the owner wants it; not part of this spec.
6. **Headless still opens a macroquad window** (Phase 2g follow-up) — out of scope;
   a future phase may extract a truly window-less binary.
7. **Remote `Nemesis`/enemy interpolation while paused** — packets are processed,
   so remote entities continue to update their interpolation snapshots while paused;
   rendering stays frozen because the accumulator is not advanced. Accepted.
