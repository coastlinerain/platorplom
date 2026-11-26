# Skill: Rust Netcode Patterns

## Overview
This skill documents common Rust patterns for game networking, specifically for the Rustvania project. Use this when working on client prediction, server reconciliation, interpolation, or packet serialization.

## Core Patterns

### 1. Client Prediction + Server Reconciliation
```rust
// Client side: predict locally, send inputs, reconcile on authoritative state
fn update(&mut self, dt: f32) {
    // 1. Sample inputs ONCE per frame (is_key_pressed is stateful!)
    let pressed_jump = is_key_pressed(KeyCode::Space);
    let pressed_shoot = is_key_pressed(KeyCode::Z);
    
    // 2. Local prediction with authoritative direction
    let local_input = PlayerInput {
        move_left: self.move_left,
        move_right: self.move_right,
        jump: pressed_jump,
        shoot: false, // bullet spawn is separate
    };
    self.player.update(dt, level, &local_input);
    
    // 3. Store edge keys for network layer
    self.pressed_jump = pressed_jump;
    self.pressed_shoot = pressed_shoot;
    
    // 4. Send inputs to server
    update_network(self);
}

// Network: sticky edges for unreliable transport
fn advance_sticky(sticky: u8, pressed: bool) -> (bool, u8) {
    let sticky = if pressed { 3 } else { sticky };
    let flag = sticky > 0;
    let next = if sticky > 0 { sticky - 1 } else { 0 };
    (flag, next)
}
```

### 2. Server Authoritative Movement
```rust
// Server: accumulate inputs, apply once per tick
pub fn apply_input(&mut self, id: u8, x: u8, y: u8, input: PlayerInput, seq: u16) {
    self.coords.insert(id, (x, y));
    let pending = self.pending.entry(id).or_default();
    
    // Held flags: latest wins
    pending.move_left = input.move_left;
    pending.move_right = input.move_right;
    
    // Edge flags: OR-accumulate (latched until consumed)
    pending.jump |= input.jump;
    pending.shoot |= input.shoot;
    
    // Track highest sequence number
    let current = self.max_seq.entry(id).or_insert(0);
    if game::is_newer_seq(seq, *current) {
        *current = seq;
    }
}

pub fn update(&mut self, dt: f32) -> Vec<BulletSpawnInfo> {
    // Clear latched edges after consuming
    pending.jump = false;
    if pending.shoot && cooldown == 0 {
        // fire
    }
    pending.shoot = false;
}
```

### 3. Reconciliation (Rewind + Replay)
```rust
pub fn reconcile(
    player: &mut Player,
    buffer: &mut InputRingBuffer,
    last_acked_seq: &mut u16,
    server_x: f32, server_y: f32, server_dir: f32,
    server_last_seq: u16,
    level: &Level,
) -> u16 {
    // 1. Stale check
    if !game::is_newer_seq(server_last_seq, *last_acked_seq) {
        return *last_acked_seq;
    }
    
    // 2. Update ack, drain acknowledged inputs
    *last_acked_seq = server_last_seq;
    buffer.drain_acknowledged(server_last_seq);
    
    // 3. Rewind to server position
    player.pos = Vec2::new(server_x, server_y);
    player.dir = server_dir;
    
    // 4. Replay unacknowledged inputs (movement only, shoot=false)
    let replay_dt = 1.0 / CLIENT_HZ as f32;
    for entry in buffer.replay_order() {
        let replay_input = PlayerInput {
            move_left: entry.move_left,
            move_right: entry.move_right,
            jump: entry.jump,
            shoot: false, // D7: never re-spawn bullets during replay
        };
        player.update(replay_dt, level, &replay_input);
    }
    
    *last_acked_seq
}
```

### 4. Interpolation (Remote Entities)
```rust
// Ring buffer for smooth remote entity rendering
struct RemoteInterp {
    interp_snapshots: [Option<(f32, f32, f32)>; 5], // x, y, dir
    timestamps: [Option<Instant>; 5],
    interp_len: u8,
}

impl RemoteInterp {
    fn push(&mut self, x: f32, y: f32, dir: f32) {
        if self.interp_len == 0 {
            self.interp_snapshots[0] = Some((x, y, dir));
            self.timestamps[0] = Some(Instant::now());
            self.interp_len = 1;
        } else if self.interp_len < 5 {
            self.interp_snapshots[self.interp_len as usize] = Some((x, y, dir));
            self.timestamps[self.interp_len as usize] = Some(Instant::now());
            self.interp_len += 1;
        } else {
            // Shift: [1]->[0], [2]->[1], [3]->[2], [4]->[3], new at [4]
            for i in 1..5 {
                self.interp_snapshots[i-1] = self.interp_snapshots[i];
                self.timestamps[i-1] = self.timestamps[i];
            }
            self.interp_snapshots[4] = Some((x, y, dir));
            self.timestamps[4] = Some(Instant::now());
        }
    }
    
    fn interpolate_at_tick(&self, target_tick: u64, last_known_server_tick: u64, last_known_arrival: Instant) -> Option<(f32, f32, f32)> {
        // Convert target tick to expected arrival time
        let tick_duration = Duration::from_millis(1000 / TICK_HZ as u64);
        let ticks_diff = target_tick as i64 - last_known_server_tick as i64;
        let target_arrival = last_known_arrival + tick_duration * ticks_diff as u32;
        
        // Find two bracketing snapshots
        // ... interpolation logic
    }
}
```

### 5. Sequence Number Handling (u16 Wraparound)
```rust
pub fn is_newer_seq(new: u16, old: u16) -> bool {
    const THRESHOLD: u16 = 32767;
    let diff = new.wrapping_sub(old);
    diff > 0 && diff < THRESHOLD
}
```

### 5. Packet Serialization (postcard)
```rust
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum GamePacket {
    JoinRequest,
    JoinResponse { assigned_id: u8 },
    PlayerPos {
        id: u8,
        x: f32, y: f32, dir: f32,
        level_x: u8, level_y: u8,
        last_seq: u16,
        server_tick: u64,
        server_time_ms: u64,
    },
    // ... other variants
}

// Round-trip test pattern
fn round_trip(pkt: GamePacket) {
    let bytes = postcard::to_allocvec(&pkt).unwrap();
    let decoded: GamePacket = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, pkt);
}
```

## Common Pitfalls

1. **`is_key_pressed` is stateful** — call ONCE per frame, store result
2. **`SystemTime` vs `Instant`** — never mix wall-clock and monotonic
3. **u16 sequence wraparound** — use `wrapping_sub` with threshold
4. **Sticky edges** — 3 frames per press for unreliable transport
5. **Reconciliation shoot** — always `false` in replay (D7)
6. **Room transitions** — client-side, server never teleports

## Key Constants (game::config)
```rust
pub const TICK_HZ: u32 = 60;           // Server tick rate
pub const CLIENT_HZ: u32 = 60;         // Client frame rate
pub const VELOCIDAD: f32 = 400.0;      // Player speed
pub const SALTO_FUERZA: f32 = -1000.0; // Jump force
pub const GRAVEDAD: f32 = 20.0;        // Gravity
pub const BULLET_SPEED: f32 = 400.0;
pub const INTERPOLATION_DELAY_TICKS: u8 = 5; // Jitter buffer delay
```

## File Locations
- Protocol: `protocol/src/lib.rs`
- Server: `server/src/world.rs`, `server/src/main.rs`
- Client: `client/src/network/world.rs`, `client/src/world/game.rs`, `client/src/world/lib.rs`
- Shared: `game/src/config.rs`, `game/src/entities/player.rs`, `game/src/entities/enemy.rs`
- E2E: `e2e/src/harness.rs`, `e2e/src/transport.rs`, `e2e/tests/`