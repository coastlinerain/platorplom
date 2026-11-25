# Phase 2f — Netcode Synchronization & Validation Infrastructure

## Problem Statement

Despite Phase 2e fixes, manual testing reveals:
- **Server/client frame desync**: Packets arrive but aren't processed in sync with simulation frames
- **No timestamp-based reconciliation**: Sequence numbers alone don't handle variable latency/jitter
- **Invisible packet processing bugs**: Wireshark shows packets sent, but state doesn't update correctly
- **Validation gap**: E2E harness uses lossless in-memory channels — misses real UDP issues

## Root Cause Analysis

### Current Architecture Gaps
| Component | Current State | Missing |
|-----------|---------------|---------|
| **Time sync** | None (assumes 60Hz lockstep) | Clock synchronization, RTT measurement |
| **Frame timing** | Fixed `tick_dt = 1/60` | Jitter buffer, frame interpolation |
| **Packet timestamps** | None (only sequence numbers) | Server/client clock correlation |
| **Validation** | In-memory loopback only | Real UDP with chaos, visual debugging |

### Why E2E Tests Pass But Game Doesn't
| E2E Harness | Real Network |
|-------------|--------------|
| Zero latency | ~1-50ms RTT |
| Ordered delivery | Packet reordering |
| No loss | 0-5% packet loss |
| Deterministic ticks | Jitter (±2ms) |
| Single process | Multi-process isolation |

---

## Phase 2f Plan

### Stage 1: Time Synchronization & Frame Alignment (Week 1)

#### 1.1 Server Clock & RTT Measurement
- Add `ServerClock { start: Instant, tick: u64 }` to `ServerWorld`
- Include `server_tick: u64` and `server_time_ms: u64` in every `PlayerPos` broadcast
- Client measures RTT: `rtt = local_now - packet_sent_time` (using `Instant`)

#### 1.2 Client Clock Synchronization
- On `JoinResponse`, server sends `server_time_ms` + `server_tick`
- Client computes `clock_offset = server_time - local_time`
- Client maintains `synced_server_tick = local_tick + clock_offset`

#### 1.3 Jitter Buffer & Frame Interpolation
- Client maintains `Vec<ServerSnapshot>` (last 3-5 server states)
- Render uses interpolation between 2 most recent server snapshots
- Simulation advances at fixed 60Hz; render at display refresh rate

### Stage 2: Packet Processing Validation (Week 2)

#### 2.1 Structured Packet Logging
```rust
// Both client & server
struct PacketLogEntry {
    timestamp_ms: u64,
    direction: Sent | Received,
    packet_type: String,
    seq: u16,
    payload_size: usize,
    // key fields for debugging
}
```

#### 2.2 Packet Processing Metrics
- Track: packets received/frame, packets processed/frame, queue depth
- Alert on: packets dropped, reordering detected, stale packets

#### 2.3 Wireshark Integration
- Export PCAP from logs for offline analysis
- Color-code by packet type (PlayerInput, PlayerPos, BulletSpawn, etc.)

### Stage 3: Headless Client & Chaos Testing (Week 3)

#### 3.1 Headless Client Binary
- `cargo build --bin metroidvania-headless`
- Runs game loop without macroquad window
- `--server-addr`, `--duration`, `--log-packets` flags
- Outputs JSONL packet log + state snapshots

#### 3.2 Chaos UDP Wrapper
```rust
struct ChaosSocket {
    inner: Socket,
    loss_rate: f32,      // 0.0-1.0
    latency_ms: Range,   // min-max added latency
    reorder_buffer: VecDeque<Packet>, // for reordering
    duplicate_rate: f32,
}
```

#### 3.3 CI Integration
```yaml
# .github/workflows/netcode-test.yml
- Run server + 2 headless clients
- With chaos: 2% loss, 10-50ms latency, 10% reorder
- Assert: no desync > 100ms, no state divergence > 5px
```

### Stage 4: Visual Debug Overlay (Week 4)

#### 4.1 Egui Debug Window (toggle with F3)
- Local: pos, vel, tick, input queue
- Server: last recv pos, server_tick, RTT, clock_offset
- Remote: interpolated pos, last snapshot age
- Packets: in/out queue, last 20 packets with colors

#### 4.2 State Divergence Detector
- Periodic hash of (all positions, enemy states, bullet states)
- Compare client vs server hash (via debug packet)
- Log divergence > threshold

---

## Merge Strategy

### Current Branch: `feat/e2e-testing`
Contains: Phase 2c bugfixes + Phase 2d e2e infrastructure + Phase 2e netcode correctness + recent fixes

### Merge Process
```bash
git checkout dev
git merge feat/e2e-testing --no-ff -m "Merge Phase 2c-2e: netcode fixes + e2e infrastructure"
git push origin dev
```

### Post-Merge: Create `feat/netcode-sync` Branch
```bash
git checkout -b feat/netcode-sync dev
```

---

## Acceptance Criteria for Phase 2f

| Criterion | Verification |
|-----------|--------------|
| RTT < 50ms measured correctly | Log shows RTT within 5ms of Wireshark |
| Clock offset < 5ms after 10s | Debug overlay shows offset stable |
| No desync > 50ms in chaos test | CI passes with 2% loss, 50ms latency |
| Packet processing visible | Debug overlay shows queue depth, processing rate |
| State hash matches server | Hash matches within 1 tick |
| Zero desync in 5-min manual test | Manual playtest with 2 clients |

---

## Dependencies

| Task | Blocks |
|------|--------|
| 1.1 Server clock | 1.2 Client sync, 1.3 Jitter buffer |
| 2.1 Packet logging | 2.2 Metrics, 2.3 Wireshark |
| 3.1 Headless client | 3.2 Chaos, 3.3 CI |
| 4.1 Debug overlay | 4.2 Divergence detector |

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Clock sync drift | Re-sync on every `PlayerPos` (every tick) |
| Jitter buffer adds latency | Cap at 2 frames (33ms max) |
| Headless client divergence | Run same binary, just no render |
| Chaos flakiness | Deterministic seed for CI runs |

---

## Next Immediate Action

1. **Merge `feat/e2e-testing` → `dev`** (this PR)
2. **Create `feat/netcode-sync` branch** from `dev`
3. **Start Stage 1.1**: Add `server_tick` + `server_time_ms` to `PlayerPos` packet