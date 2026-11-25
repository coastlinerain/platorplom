# 0002. Game state machine

- Status: Accepted
- Date: 2026-11-25

## Context

The client has no application-level lifecycle: `client/src/main.rs` hardcodes a
single connect-and-play path (bind socket → `JoinRequest` → build `World` → loop
forever). There is no main menu, pause, disconnect handling, or recovery from a
failed/lost connection, and the client never sends `Leave`. Adding any of these
piecemeal risks scattering state transitions across the render loop. Before adding
menus/pause/reconnect, the client needs a single, explicit state machine that owns
the socket and `World` lifecycles.

## Decision

**Introduce a client-only, flat game state machine.**

1. **Client-only scope.** The server stays drop-in-join — no `GamePacket` change and
   no `ServerWorld` change. The state machine lives entirely in the client; its pure
   core (`AppState`/`AppEvent`/`transition`) lives in the shared `game` crate for
   unit-testability, and the runner/dispatcher lives in a new `client::app` module.

2. **Flat enum over a stack.** Six states — `MainMenu`, `Connecting`, `Playing`,
   `Paused`, `Disconnected`, `Quitting` (terminal) — with a total
   `transition(state, event) -> AppState` function. Invalid `(state, event)` pairs
   are no-ops (state unchanged). A *stack* is deferred until nested menus (e.g.
   pause → settings → back) actually need it.

3. **Lazy `World` + bind-per-join socket lifecycle.** `World` is constructed only on
   `Joined` (after `JoinResponse`). Each `StartJoin`/`Retry` binds a fresh ephemeral
   socket (`0.0.0.0:0`), sends `JoinRequest`, awaits `JoinResponse` via a shared
   handshake helper, then moves the socket into `World::new`. Leaving `Playing`/
   `Paused`/`Connecting`/`Disconnected` drops the socket (and `World`, when present).
   `MainMenu` holds no socket.

4. **`Paused` freezes simulation + input send but keeps network receive.** Packets
   are still received and processed (reconciliation, remote-player interpolation,
   `Leave`) while paused, but no simulation steps run and no `PlayerInput` is sent.
   This requires splitting the client's `update_network` into a receive half and a
   send half.

5. **Timeouts as named constants.** `CONNECT_TIMEOUT_MS = 5000` (fires
   `ConnectFailed` if no `JoinResponse`) and `DISCONNECT_TIMEOUT_MS = 1000` (fires
   `ConnectionLost` if no `PlayerPos` while `Playing`/`Paused`), both in
   `game::config`.

## Consequences

- **Positive:** a single source of truth for client lifecycle; the pure transition
  core is exhaustively unit-testable; the socket/`World` lifecycles are explicit and
  leak-free (bind-per-join, drop-on-leave); pause and disconnect are now well-defined;
  quit finally sends `Leave` so the server and other clients learn about a voluntary
  exit promptly.
- **Negative:** the GUI `main.rs` must be rewritten as a thin loop over the runner;
  the receive/send split in `update_network` and the new `received_player_pos`
  liveness flag touch hot networking code (regression risk, mitigated by existing
  e2e coverage); the state machine adds a layer of indirection that the flat
  no-op table makes slightly verbose (66 pairs for 16 edges).
- **Deferred:** nested-menu stack, a real `Loading` sub-state, a server "lobby"
  phase, and fixing the server `Leave` handler's unchecked `id` (a pre-existing
  latent bug this feature records but does not fix).

## Open questions

- Whether `Connecting` should also accept `Quit` (today the settled table requires
  `Cancel` → `MainMenu` → `Quit` to exit while connecting). Deferred as a possible
  UX follow-up.
- Whether the disconnect watchdog should also pause the server's view of the player
  (no — server-side timeouts already exist via laminar `SocketEvent::Timeout`).
