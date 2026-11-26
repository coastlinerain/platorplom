# Phase 2f — Netcode Synchronization & Frame Alignment

- Status: implemented
- Date: 2026-11-25
- Branch: `feat/phase-2f-netcode-sync`

## Summary

Phase 2e made hit detection and bullet spawn server-authoritative. However, the client and server frames are not synchronized:
- Server runs at 60Hz tick loop, broadcasts `PlayerPos` with position
- Client runs at 60Hz render loop, predicts locally, reconciles on `PlayerPos`
- No clock synchronization: client doesn't know server's tick or time
- No jitter buffer: variable latency causes stutter
- No frame interpolation: remote entities snap between snapshots
- No RTT measurement or clock offset calculation

This phase adds:
1. **Clock synchronization**: Server broadcasts `server_tick` and wall-clock `server_time_ms` with every `PlayerPos`. Client computes clock offset and RTT.
2. **Jitter buffer**: Client stores last N=5 server snapshots per remote player, interpolates for smooth rendering.
3. **Frame interpolation**: Remote entities (nemesis) rendered with tick-based interpolation between buffered snapshots.
4. **Packet logging**: Structured JSONL logging for debugging packet flow.
5. **Headless client**: Single binary with `--headless` flag that runs game loop without window for CI testing.

## Goals / non-goals

### Goals
- **G1**: Client knows server time within ~1 frame accuracy (clock offset).
- **G2**: Remote players render smoothly despite network jitter (jitter buffer + interpolation).
- **G3**: Structured packet logs enable debugging desync/latency issues.
- **G4**: Headless client runs in CI for e2e tests without display.
- **G5**: Clock sync resets cleanly on join, death, room transition.
- **G6**: All existing tests pass (145+ tests), quality gates green.

### Non-Goals (Future Phases)
- Server-authoritative enemy AI movement.
- Server-side bullet physics (beyond spawn + raycast).
- Lag compensation for PvP (beyond existing reconciliation).
- Dynamic tick rate adjustment.
- Client-side prediction for enemies/bullets.

## Behavior

### Clock Synchronization (D1) — **Option A: Wall-Clock Time**
**Mechanism**: NTP-style offset on each `PlayerPos` using wall-clock time.
- Server includes `server_tick: u64` and `server_time_ms: u64` in every `PlayerPos` broadcast.
  - `server_tick`: Monotonic server tick counter (increments each server tick).
  - `server_time_ms`: Wall-clock time at broadcast = `SystemTime::now().duration_since(UNIX_EPOCH).as_millis() as u64`.
- Client on receiving `PlayerPos` (for own player):
  - `local_time_ms = SystemTime::now().duration_since(UNIX_EPOCH).as_millis() as i64`
  - `clock_offset_ms = server_time_ms as i64 - local_time_ms`
  - `server_tick_offset = server_tick as i64 - client_tick as i64`
- `client_tick` increments once per client frame (`World::update`).
- Clock offset is a running estimate (updated every frame); no smoothing/filtering in this phase.
- **Rationale**: Simple, uses existing broadcast, no extra packets. Server is authoritative time source. Both sides use wall-clock (`SystemTime`), so subtraction is meaningful. Server must have roughly synchronized wall-clock (NTP).

> **Note on ADR-001 discrepancy**: ADR-001 line 20 states "Fixed tick rate: 20 Hz" but `game/src/config.rs` defines `TICK_HZ = 60`. This spec assumes 60Hz (matching code). ADR-001 should be updated to reflect 60Hz.

### Jitter Buffer (D2, D7) — **N=5, Tick-Based**
**Structure**: Fixed N=5 ring buffer per remote player (keep latest 5).
- Extend `Nemesis.interp_snapshots` from `[Option<(f32,f32,f32)>; 2]` to `[Option<(f32,f32,f32)>; 5]`.
- `interp_len: u8` tracks valid count (0..5).
- `push_interp_snapshot`: on 6th push, shift `[1]→[0], [2]→[1], [3]→[2], [4]→[3]`, new at `[4]` (keep latest 5).
- `RemoteInterp.timestamps` mirrors: `[Option<Instant>; 5]` (arrival times, monotonic).
- **Eviction**: Keep latest N=5, drop older (FIFO ring).
- **Stale guard**: If newest snapshot > 200ms old, snap to newest (existing behavior preserved).

**Render interpolation (tick-based)**:
- Client renders at `client_tick` (local frame).
- Server tick estimate: `server_tick_estimate = client_tick + server_tick_offset` (using `server_tick_offset` from clock sync).
- Target render tick = `server_tick_estimate - INTERPOLATION_DELAY_TICKS` (where `INTERPOLATION_DELAY_TICKS = 5` from `game::config`).
- Target arrival time estimate: convert target tick to expected arrival `Instant` using `last_server_tick` arrival time as anchor.
- Find two snapshots whose arrival times bracket the target arrival time, lerp position, snap direction to newer.
- If only 1 snapshot: snap to it. If 0: don't render (or snap to `nemesis.pos` fallback).
- **Rationale**: N=5 gives 4 intervals = 67ms span at 60Hz. With `INTERPOLATION_DELAY_TICKS=5` (83ms), we have ~17ms headroom for jitter absorption. This is standard for 60Hz games.

### Frame Interpolation (D3)
**Target**: Remote players only (nemesis).
- Local player: prediction + reconciliation (unchanged).
- Bullets: server-authoritative spawn + client physics (unchanged).
- Enemies: client-side simulation (unchanged, D6 scope boundary).
- `RemoteInterp.interpolate_at_tick` computes interpolated position for a target render tick.
- `World.draw()` uses interpolated position for nemesis in current room.

### Packet Logging (D4)
**Format**: JSONL to stdout (one JSON object per line).
- Fields per entry:
  - `ts_ms`: local timestamp (ms since epoch)
  - `dir`: `"sent"` | `"recv"`
  - `type`: packet variant name (e.g., `"PlayerPos"`, `"PlayerInput"`)
  - `id`: player/entity id (if applicable)
  - `seq`: sequence number (if applicable)
  - `server_tick`: from packet (if applicable)
  - `server_time_ms`: from packet (if applicable)
  - `key_fields`: abbreviated summary (e.g., `{"x":100,"y":200}`)
- Enabled via `RUSTVANIA_PACKET_LOG=1` env var (opt-in, zero cost when off).
- **Rationale**: JSONL streams, parsable, no external deps. Stdout allows piping to file or log aggregator.

### Headless Client (D5) — **Single Binary with `--headless` Flag**
**Architecture**: Single binary entry point (`main.rs`) with conditional headless mode.
- `#[macroquad::main("Rustvania")] async fn main()` checks `std::env::args().any(|a| a == "--headless")`.
- If `--headless`: skips window init/render loop, runs fixed ticks, logs packets, exits after `--duration` (seconds) or SIGINT.
- If normal: runs full macroquad window/render loop.
- Uses `LoopbackTransport` or real `PacketIO` (configurable via args).
- **Rationale**: Clean CI integration, no display needed, shares all game logic. Single binary avoids code duplication. `macroquad::main` supports conditional window init via the return value pattern.

### Clock Sync Reset Triggers (D6)
Reset `client_tick`, `server_tick_offset`, `clock_offset_ms`, `last_server_tick`, `last_server_time_ms`, `last_acked_server_tick` on:
1. **Join**: `JoinResponse` received (already implemented).
2. **Death/Respawn**: `Hit` packet received for own player (`entity_id == world.id`) → respawn logic.
3. **Room Transition**: `load_current_level` called (client-side transition).
4. **Manual**: Debug key (e.g., `KeyCode::F11`) for testing.

**Rationale**: These are discontinuities where server time reference or player position teleports.

### Integration with Reconciliation (D8)
- `last_seq` remains primary for input buffer draining (input-sequence based).
- `server_tick` + `clock_offset_ms` used for:
  - Render-time server position estimation (interpolation target).
  - Staleness heuristic: if `server_tick - last_acked_server_tick > STALENESS_THRESHOLD_TICKS` (e.g., 5), force reconcile.
- No change to `prediction::reconcile` signature; it still takes `last_seq`.
- `World` tracks `last_acked_server_tick: u64` updated on each own-player `PlayerPos`.

### New Constant: `INTERPOLATION_DELAY_TICKS`
- Defined in `game/src/config.rs`: `pub const INTERPOLATION_DELAY_TICKS: u8 = 5;`
- Used by client for interpolation target calculation.
- **Remove `INTERPOLATION_DELAY_MS`** — no longer used.

## Data model

### Protocol changes (`protocol/src/lib.rs`)
```rust
// PlayerPos already has these fields (added in Phase 2e prep):
PlayerPos {
    id: u8,
    x: f32,
    y: f32,
    dir: f32,
    level_x: u8,
    level_y: u8,
    last_seq: u16,
    server_tick: u64,      // NEW: server tick counter
    server_time_ms: u64,   // NEW: server wall-clock ms since UNIX_EPOCH
}
```

### Server (`server/src/world.rs`)
```rust
pub struct ServerWorld {
    // ... existing fields ...
    current_tick: u64,           // increments each tick
    start_time: std::time::Instant, // server start time (for uptime if needed)
}

impl ServerWorld {
    pub fn current_tick(&self) -> u64 { self.current_tick }
    pub fn wall_time_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
```

### Client — World (`client/src/world/game.rs`)
```rust
pub struct World {
    // ... existing fields ...
    // Clock sync
    pub client_tick: u64,
    pub server_tick_offset: i64,    // server_tick - client_tick
    pub clock_offset_ms: i64,       // server_time_ms - local_time_ms
    pub last_server_tick: u64,
    pub last_server_time_ms: u64,
    pub last_acked_server_tick: u64, // for reconciliation staleness
    // Jitter buffer config (uses game::config::INTERPOLATION_DELAY_TICKS)
}
```

### Client — Nemesis (`game/src/entities/nemesis.rs`)
```rust
pub struct Nemesis {
    // ... existing fields ...
    /// Ring of 5 position snapshots: oldest at index 0, newest at index 4
    pub interp_snapshots: [Option<(f32, f32, f32)>; 5], // was [..; 2]
    pub interp_len: u8, // 0..5
}

impl Nemesis {
    pub fn push_interp_snapshot(&mut self, x: f32, y: f32, dir: f32) {
        // On 6th push: shift [1]→[0], [2]→[1], [3]→[2], [4]→[3], new at [4]
    }
}
```

### Client — RemoteInterp (`client/src/network/interpolation.rs`)
```rust
pub struct RemoteInterp {
    pub timestamps: [Option<Instant>; 5], // mirrors Nemesis.interp_snapshots (arrival times)
}

impl RemoteInterp {
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32) {
        nemesis.push_interp_snapshot(x, y, dir);
        let now = Instant::now();
        // Shift timestamps to match nemesis eviction (FIFO ring of 5)
        if nemesis.interp_len == 5 {
            for i in 0..4 {
                self.timestamps[i] = self.timestamps[i+1];
            }
            self.timestamps[4] = Some(now);
        } else {
            for i in (1..nemesis.interp_len as usize).rev() {
                self.timestamps[i] = self.timestamps[i-1];
            }
            self.timestamps[nemesis.interp_len as usize] = Some(now);
        }
    }

    /// Interpolate at a specific target render tick.
    /// `target_tick`: the server tick we want to render for (server_tick_estimate - INTERPOLATION_DELAY_TICKS).
    /// `last_known_server_tick`: the server_tick from the most recent PlayerPos.
    /// `last_known_arrival`: the Instant when that PlayerPos arrived.
    pub fn interpolate_at_tick(
        &self,
        nemesis: &Nemesis,
        target_tick: u64,
        last_known_server_tick: u64,
        last_known_arrival: Instant,
    ) -> Option<(f32, f32, f32)> {
        // Convert target_tick to expected arrival time:
        //   target_arrival = last_known_arrival + (target_tick - last_known_server_tick) * TICK_MS
        // where TICK_MS = 1000 / TICK_HZ (approx 16.67ms at 60Hz).
        // Find two snapshots whose arrival times bracket target_arrival, lerp position, snap dir to newer.
        // Stale guard: if newest timestamp > 200ms before target_arrival, snap to newest.
        // If only 1 snapshot: snap to it. If 0: None.
    }

    // Legacy interpolate() delegates to interpolate_at_tick with current estimates.
}
```

### Packet Logging (`client/src/network/logging.rs` — new module)
```rust
use protocol::GamePacket;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn log_packet(dir: &str, packet: &GamePacket) {
    if std::env::var("RUSTVANIA_PACKET_LOG").is_err() { return; }
    let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let entry = serde_json::json!({
        "ts_ms": ts_ms,
        "dir": dir,
        "type": packet_type_name(packet),
        "id": packet_id(packet),
        "seq": packet_seq(packet),
        "server_tick": packet_server_tick(packet),
        "server_time_ms": packet_server_time_ms(packet),
        "key_fields": packet_key_fields(packet),
    });
    println!("{}", entry);
}

fn packet_type_name(p: &GamePacket) -> &'static str { ... }
fn packet_id(p: &GamePacket) -> Option<u8> { ... }
fn packet_seq(p: &GamePacket) -> Option<u16> { ... }
fn packet_server_tick(p: &GamePacket) -> Option<u64> { ... }
fn packet_server_time_ms(p: &GamePacket) -> Option<u64> { ... }
fn packet_key_fields(p: &GamePacket) -> serde_json::Value { ... }
```

### Headless Client (`client/src/main.rs` — modified entry point)
```rust
#[macroquad::main("Rustvania")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let duration_secs = args.iter()
        .find(|a| a.starts_with("--duration="))
        .and_then(|a| a.split('=').nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0); // 0 = run until SIGINT

    if headless {
        run_headless(duration_secs).await;
        return;
    }

    // ... normal macroquad window init + render loop ...
}

async fn run_headless(duration_secs: u64) {
    let server_addr = std::env::args()
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| "127.0.0.1:12345".to_string())
        .parse()
        .unwrap();

    let mut socket = laminar::Socket::bind("0.0.0.0:0").unwrap();
    let sender = socket.get_packet_sender();
    let receiver = socket.get_event_receiver();
    std::thread::spawn(move || socket.start_polling());

    let io = HeadlessPacketIO { sender, receiver, server_addr };
    let mut boxed: Box<dyn PacketIO> = Box::new(io);
    send_join_req(&mut *boxed);

    let world_data = Map::new();
    let mut game_world = World::new(world_data, boxed);

    let tick_dt = 1.0 / game::config::TICK_HZ as f32;
    let max_ticks = if duration_secs > 0 {
        (duration_secs as f32 * game::config::TICK_HZ as f32) as u64
    } else {
        u64::MAX
    };

    for _ in 0..max_ticks {
        game_world.update(tick_dt);
        // No draw, no next_frame().await
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}
```

## Network

### Packets changed
- `PlayerPos`: already has `server_tick`, `server_time_ms` (added in Phase 2e prep, confirmed in `protocol/src/lib.rs`).
- No new packet types.

### Server broadcast (already in `server/src/main.rs` lines 147-159, verify uses wall-clock)
```rust
GamePacket::PlayerPos {
    id: player_id,
    x: player.pos.x,
    y: player.pos.y,
    dir: player.dir,
    level_x: lx,
    level_y: ly,
    last_seq,
    server_tick: world.current_tick(),
    server_time_ms: world.wall_time_ms(), // MUST be wall-clock (SystemTime), not Instant
}
```

### Client receive (already in `client/src/network/world.rs` lines 87-104, verify uses SystemTime)
```rust
GamePacket::PlayerPos {
    id, x, y, dir, level_x, level_y, last_seq,
    server_tick, server_time_ms,
} => {
    if Some(id) == world.id {
        let local_time_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        world.clock_offset_ms = server_time_ms as i64 - local_time_ms;
        world.server_tick_offset = server_tick as i64 - world.client_tick as i64;
        world.last_server_tick = server_tick;
        world.last_server_time_ms = server_time_ms;
        world.last_acked_server_tick = server_tick; // for staleness check
        // ... reconciliation ...
    }
}
```

### Client receive: Hit packet (death clock reset)
```rust
GamePacket::Hit { entity_id, .. } => {
    if Some(entity_id) == world.id {
        // Reset clock sync on death
        world.client_tick = 0;
        world.server_tick_offset = 0;
        world.clock_offset_ms = 0;
        world.last_server_tick = 0;
        world.last_server_time_ms = 0;
        world.last_acked_server_tick = 0;
        // ... existing respawn logic ...
    }
}
```

## Acceptance criteria

| ID | Criterion | Test Type |
|----|-----------|-----------|
| AC1 | `PlayerPos` includes `server_tick: u64` and `server_time_ms: u64` (wall-clock) | Unit (protocol round-trip) |
| AC2 | `ServerWorld` increments `current_tick` each tick, `wall_time_ms()` returns `SystemTime` | Unit (ServerWorld::update) |
| AC3 | Client computes `clock_offset_ms = server_time_ms - local_time_ms` on each own-player `PlayerPos` | Unit (handle_packet) |
| AC4 | Client tracks `server_tick_offset = server_tick - client_tick` | Unit (handle_packet) |
| AC5 | `Nemesis.interp_snapshots` holds 5 entries; `push_interp_snapshot` evicts oldest on 6th push | Unit (Nemesis) |
| AC6 | `RemoteInterp.timestamps` mirrors 5 entries; `push` shifts correctly | Unit (RemoteInterp) |
| AC7 | `RemoteInterp.interpolate_at_tick` returns lerped position between two snapshots bracketing target tick | Unit (RemoteInterp) |
| AC8 | Stale guard: newest snapshot > 200ms old → snap to newest (returns Some) | Unit (RemoteInterp) |
| AC9 | Remote players rendered with interpolation in `World.draw()` (uses `interpolate_at_tick`) | Integration (e2e) |
| AC10 | Packet logging emits JSONL to stdout when `RUSTVANIA_PACKET_LOG=1` | Unit (logging) |
| AC11 | Headless client runs with `--headless`, game loop without window, exits after `--duration` or SIGINT | Integration (CI) |
| AC12 | Clock sync resets on: JoinResponse, Hit (death), room transition, F11 key | Unit (handle_packet, World::update) |
| AC13 | Reconciliation still uses `last_seq` for input buffer; `server_tick` supplements staleness check (>5 ticks) | Unit (prediction::reconcile) |
| AC14 | All existing tests pass (145+ tests) | Regression (cargo test) |
| AC15 | Quality gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` | CI |

## Design Decisions (D1-D8) with Rationale

### D1: Clock Sync Mechanism — **Option A (Wall-Clock NTP-style on each PlayerPos)**
- **Chosen**: Client computes `clock_offset_ms = server_time_ms - local_time_ms` on every own-player `PlayerPos`. Server sends wall-clock `server_time_ms`.
- **Rejected**: Option B (monotonic `Instant` + connect anchor) — requires tracking connect time, more complex. Option C (`server_tick` only) — loses absolute time, drift accumulates.
- **Rationale**: Server is authoritative time source; broadcast already exists; simple; ~1 frame accuracy sufficient. Both sides use `SystemTime` so subtraction is meaningful. Server must run NTP.

### D2: Jitter Buffer Size — **Fixed N=5**
- **Chosen**: Ring buffer of 5 snapshots per remote player.
- **Rejected**: N=2/3 — too little headroom (3 entries = 33ms span, 50ms delay = zero headroom). Dynamic/adaptive — over-engineering.
- **Rationale**: 5 snapshots = 4 intervals = 67ms span at 60Hz. With `INTERPOLATION_DELAY_TICKS=5` (83ms), ~17ms headroom for jitter. Standard for 60Hz games.

### D3: Interpolation Target — **Remote players only (nemesis), tick-based**
- **Chosen**: Only `Nemesis` (other players) interpolated using tick-based target.
- **Rejected**: Local player — breaks prediction/reconciliation. All entities — enemies/bullets not authoritative. Wall-clock target — domain mismatch with `Instant` timestamps.
- **Rationale**: Remote players are the only entities with authoritative server positions arriving at variable latency. Tick-based domain matches `server_tick` from packets and `client_tick` locally.

### D4: Packet Logging Format — **JSONL to stdout (opt-in via env var)**
- **Chosen**: One JSON per line, printed to stdout when `RUSTVANIA_PACKET_LOG=1`.
- **Rejected**: Binary format — not human-readable. Separate files — adds file I/O. Always-on — performance cost.
- **Rationale**: Zero cost when disabled; streamable; parsable with `jq`; works in CI logs.

### D5: Headless Client Architecture — **Single binary with `--headless` flag**
- **Chosen**: Conditional in `main.rs` via `#[macroquad::main]`, checks `--headless` flag.
- **Rejected**: Separate binary `main_headless.rs` — duplicates entry logic, Cargo `[[bin]]` complexity.
- **Rationale**: Single binary, no code duplication. `macroquad::main` supports conditional window init (return early before window creation). Clean CI: `cargo run -- --headless --duration=10`.

### D6: Clock Sync Reset Triggers — **Join + Death + Room Transition + Manual**
- **Chosen**: Reset on `JoinResponse`, `Hit` (own player), `load_current_level`, `KeyCode::F11`.
- **Rejected**: Only join — misses respawn/transition discontinuities. Every frame — defeats purpose.
- **Rationale**: These are the exact moments where server time reference or player position teleports.

### D7: Jitter Buffer Eviction Policy — **Keep latest N=5, drop older (FIFO ring)**
- **Chosen**: On 6th push, shift `[1]→[0], [2]→[1], [3]→[2], [4]→[3]`, new at `[4]`.
- **Rejected**: Drop if >200ms old — handled by stale guard at render time.
- **Rationale**: Simple ring buffer; always has most recent 5 snapshots for interpolation.

### D8: Integration with Reconciliation — **Keep last_seq primary, server_tick supplements**
- **Chosen**: `last_seq` drives input buffer draining. `server_tick` + `clock_offset_ms` used for render-time estimation and staleness heuristic (`>5 ticks`).
- **Rejected**: Replace `last_seq` with `server_tick` — `last_seq` is input-sequence precise; `server_tick` is wall-clock.
- **Rationale**: Input buffer indexed by `seq`; changing would break reconciliation. `server_tick` adds wall-clock context for interpolation and staleness detection.

## Files to Modify / Create

| File | Change |
|------|--------|
| `protocol/src/lib.rs` | Confirm `PlayerPos` has `server_tick`, `server_time_ms` (already done) |
| `server/src/world.rs` | `current_tick`, `start_time` exist; add `wall_time_ms()` using `SystemTime` |
| `server/src/main.rs` | Broadcast uses `world.wall_time_ms()` for `server_time_ms` (verify) |
| `game/src/config.rs` | Add `pub const INTERPOLATION_DELAY_TICKS: u8 = 5;` |
| `game/src/entities/nemesis.rs` | `interp_snapshots: [Option<...>; 5]`, `push_interp_snapshot` for 5-entry ring |
| `client/src/network/interpolation.rs` | `RemoteInterp.timestamps: [Option<Instant>; 5]`, `interpolate_at_tick(target_tick, ...)` |
| `client/src/network/world.rs` | Clock sync on `PlayerPos` (wall-clock), reset on death/transition, log packets, staleness check |
| `client/src/network/logging.rs` | **New**: JSONL packet logging module with `packet_key_fields` for all variants |
| `client/src/world/game.rs` | `last_acked_server_tick`, use `interpolate_at_tick` in `draw()`, `client_tick++` in `update()` |
| `client/src/world/lib.rs` | Reset clock sync in `load_current_level`, `handle_death` (Hit packet) |
| `client/src/main.rs` | Conditional `--headless` flag, call `log_packet` on send/recv (when enabled) |
| `e2e/src/harness.rs` | Update `TestHarness` to send `server_tick`/`server_time_ms` (verify), verify interpolation |
| `e2e/src/transport.rs` | No change needed (loopback already carries all fields) |

## Test Cases (for planner)

### Unit Tests
1. `Nemesis::push_interp_snapshot` — 6 pushes keeps latest 5, evicts oldest.
2. `RemoteInterp::push` — timestamps mirror nemesis eviction exactly (5-entry ring).
3. `RemoteInterp::interpolate_at_tick` — 2 snapshots, target at 50% of interval → lerp 0.5.
4. `RemoteInterp::interpolate_at_tick` — target before oldest → snap to oldest.
5. `RemoteInterp::interpolate_at_tick` — target after newest → snap to newest (clamp).
6. `RemoteInterp::interpolate_at_tick` — stale guard (>200ms) → snap to newest.
7. `logging::log_packet` — emits valid JSONL with correct fields for each packet type.
8. `logging::packet_key_fields` — returns correct abbreviated summary for all packet variants.
9. `handle_packet` — on own `PlayerPos`, updates `clock_offset_ms`, `server_tick_offset` using `SystemTime`.
10. `handle_packet` — on `Hit` for own player, resets clock sync fields.
11. `load_current_level` — resets clock sync fields.
12. `World::update` — increments `client_tick` each frame.
13. `World::update` — staleness check: if `server_tick - last_acked_server_tick > 5`, force reconcile flag.

### Integration (e2e) Tests
14. Two clients, one moves: remote nemesis renders interpolated (not snapping).
15. Simulated jitter (delay packets 1-2 ticks): remote nemesis still smooth.
16. Clock sync: after 100 ticks, `clock_offset_ms` stable within ±5ms.
17. Death/respawn: clock sync resets, new offset converges.
18. Room transition: clock sync resets, remote players in new room interpolate.
19. Headless client runs with `--headless --duration=10`, exits 0.
20. Packet logging: `RUSTVANIA_PACKET_LOG=1` produces valid JSONL for sent/recv packets.

## Explain Block (for implementer commits)

Each implementation commit must include:
```
## What
<what changed>

## Why
<reason>

## Rust concepts
<ownership, borrows, lifetimes, traits, generics, Result/Option, pattern matching, channels, derive, etc.>

## Watch out
<gotchas, trade-offs, things to verify>
```