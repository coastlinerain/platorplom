# Windowless headless binary

- Status: implemented
- Date: 2026-11-25
- Branch: `refactor/windowless-headless`

## Summary

Split the single `metroidvania-rust` client binary into two feature-gated
binaries: the existing **GUI** binary (`#[macroquad::main]`) and a new
**windowless headless** binary with a plain `fn main()`. Today the headless path
lives behind a `--headless` flag *inside* the GUI binary's `main()`, which is
too late: `#[macroquad::main]` opens a window before `main()` runs, so headless
CI still needs Xvfb and local `cargo test` flashes blank windows. After this
refactor the headless binary never initializes macroquad, so no window exists to
satisfy.

This is a pure build/bin-split refactor: no protocol, server, simulation,
state-machine, or `App`-API changes.

## Goals / non-goals

**Goals**

- Two binaries: `metroidvania-rust` (GUI, `required-features = ["gui"]`) and
  `rustvania-headless` (windowless, `required-features = ["headless"]`).
- The headless binary's entry point is a plain `fn main()` — no
  `#[macroquad::main]`, no window, no render calls.
- Shared transport code moves from `main.rs` into the lib so both binaries use
  one implementation.
- e2e subprocess tests run the new windowless binary.
- CI drops Xvfb and runs `cargo test --workspace` directly.

**Non-goals**

- No change to `protocol/` or `server/` (no diff expected).
- No change to game/simulation logic, the state machine, or the `App` API
  (`App::new`, `update`, `dispatch`, `set_game_input`, `set_viewport`, `draw`).
- No new game features.
- Macroquad stays an unconditional dependency of the lib (the headless binary
  links it but never initializes it — linking alone opens no window).

## Behavior

### Build / feature surface

`client/Cargo.toml` keeps the existing `[lib]` and gains:

```toml
[features]
gui = []
headless = []
default = ["gui"]

[[bin]]
name = "metroidvania-rust"
path = "src/main.rs"
required-features = ["gui"]

[[bin]]
name = "rustvania-headless"
path = "src/main_headless.rs"
required-features = ["headless"]
```

The explicit `[[bin]]` entries replace Cargo's implicit auto-detection of
`src/main.rs`. The two features are pure markers used only for
`required-features`; they do not gate any dependency or lib item.

### GUI binary (`src/main.rs`)

Keeps `#[macroquad::main("Rustvania")]`, `sample_ui_event`, and the GUI loop
(sample UI event → `set_game_input` → `set_viewport` → `update` → `draw` →
`next_frame`). It:

- drops the `--headless` flag branch and `--duration` parsing (headless-only
  concerns);
- keeps `--server=ADDR` parsing (default `127.0.0.1:12345`, `SocketAddr`);
- calls `make_transport_factory(server_addr, /*headless=*/ false)` and
  `App::new(factory, /*headless=*/ false)`;
- removes its local definitions of `RealPacketIO`, `HeadlessPacketIO`,
  `make_transport_factory`, and `run_headless` (now in the lib / headless
  binary).

### Headless binary (`src/main_headless.rs`)

A plain synchronous `fn main()` (no async, no macroquad). It:

- parses `--duration=N` (seconds, default `0` = run until quit/SIGINT) and
  `--server=ADDR` (default `127.0.0.1:12345`, `SocketAddr`) with the same
  semantics as today — it does **not** accept `--headless` (headlessness is
  inherent to this binary);
- builds `let factory = make_transport_factory(server_addr, true);` and
  `let mut app = App::new(factory, true);`
- runs the fixed-timestep headless loop via a private `run_headless(&mut app,
  duration_secs)` function moved verbatim from `main.rs` (hybrid sleep +
  busy-wait, quit on `Quitting`/`Disconnected`, tick cap from `duration_secs`).
  The moved function keeps its existing behavior and prints.

It never calls `app.draw()`, `clear_background`, `next_frame`, `screen_width`,
or `screen_height`, and has no `#[macroquad::main]`, so no window is created.

### Shared transport (`client/src/network/transport.rs`)

Move `RealPacketIO`, `HeadlessPacketIO`, and `make_transport_factory` from
`main.rs` into a new `client/src/network/transport.rs`, registered as
`pub mod transport;` in `client/src/network/mod.rs`. `make_transport_factory`
is `pub` and re-exported from `client/src/lib.rs`. The two `PacketIO` structs
stay module-private (only the factory constructs them). Inside the lib, the
`log_packet` calls may use the `crate::network::logging::log_packet` path.

`run_headless` is **not** placed in the lib: it is only used by the headless
binary, so it lives as a private `fn` in `src/main_headless.rs`.

### e2e invocation

`e2e/src/process.rs::run_headless_client` changes the client spawn to the new
binary:

```text
cargo run -p metroidvania-rust --bin rustvania-headless --features headless -- --duration=N --server=127.0.0.1:12345
```

(the `--headless` flag is removed; `--bin` and `--features headless` are added).
`RUSTVANIA_PACKET_LOG` handling, `SERVER_PORT_LOCK`, and `ServerGuard` are
unchanged.

### CI

`.github/workflows/ci.yml`:

- removes the "Install Xvfb" step;
- runs `cargo test --workspace` directly (no `xvfb-run`);
- changes the clippy step to
  `cargo clippy --all-targets --all-features --workspace -- -D warnings` so the
  headless binary is also linted (plain `--all-targets` under default features
  would skip it, since `rustvania-headless` requires `headless`).

### Edge cases

- `cargo test --workspace` with default features compiles but never *runs* the
  GUI binary, so no window opens; the headless binary is built and run only by
  the e2e subprocess (with `--features headless`), and it is windowless.
- `cargo run --bin rustvania-headless` without `--features headless` fails with
  Cargo's "required feature not enabled" message — expected, not an error to fix.
- `cargo run --bin metroidvania-rust` (default features) still opens the window.
- `--duration=0` in the headless binary means "run until quit/SIGINT" (unchanged
  semantics).

## Data model

No new game types. Build-surface changes only:

| Item | Today | After |
| --- | --- | --- |
| `RealPacketIO` | private in `main.rs` | module-private in `client/src/network/transport.rs` |
| `HeadlessPacketIO` | private in `main.rs` | module-private in `client/src/network/transport.rs` |
| `make_transport_factory` | private `fn` in `main.rs` | `pub` in `transport.rs`, re-exported from `lib.rs` |
| `run_headless` | private `fn` in `main.rs` | private `fn` in `src/main_headless.rs` |
| `sample_ui_event` | private `fn` in `main.rs` | unchanged, stays in `main.rs` |
| `App` / `TransportFactory` / `PacketIO` | lib | unchanged |

## Network

No packets, protocol types, or wire format change. The transport code is moved,
not modified: `JoinRequest`/`Hit` still send reliable-ordered, all other packets
unreliable, and the ephemeral-port bind behavior is unchanged.

## Acceptance criteria

- `client/src/main_headless.rs` contains no `#[macroquad::main]` (grep-able) and
  its entry point is a plain `fn main()`.
- `client/Cargo.toml` declares `gui`/`headless` features, `default = ["gui"]`,
  and two `[[bin]]` entries with the correct `required-features`.
- `make_transport_factory` (and the two I/O structs) live in
  `client/src/network/transport.rs`, are no longer *defined* in `main.rs` (the
  GUI binary imports and calls `make_transport_factory` from the lib), and
  `make_transport_factory` is re-exported from `client/src/lib.rs`.
- `e2e/src/process.rs` spawns `rustvania-headless` with `--features headless`
  and no `--headless` flag.
- `cargo test --workspace` passes **without Xvfb** (the `headless_timing` and
  `packet_log_rate` subprocess tests run the windowless binary).
- `cargo run --bin metroidvania-rust` still opens the window and plays as before.
- `cargo clippy --all-targets --all-features --workspace -- -D warnings` is clean
  (both binaries covered).
- `cargo fmt --check --all` is clean.
- No diff in `protocol/` or `server/`.

## Follow-up

- **Collapse `RealPacketIO`/`HeadlessPacketIO`** (QA MINOR): the two structs are
  now byte-for-byte identical and co-located in `transport.rs`; the old "kept
  separate so each factory closure references a concrete type" rationale no
  longer holds (there is a single `make_transport_factory`). Collapsing them also
  makes the transport-level `headless` bool redundant — decide whether that flag
  survives at the transport layer or headlessness is solely an `App` concern.
- **`.unwrap()` in library code** (QA NIT): `transport.rs` now lives in the lib,
  so AGENTS.md's "prefer Result/Option over unwrap" binds more tightly. Convert
  `postcard::to_allocvec(...).unwrap()` and `laminar::Socket::bind(...).unwrap()`
  to `.expect("...")` with a message, and consider propagating a `Result` from
  `make_transport_factory` for `SocketAddr::parse`.
