# Windowless headless binary — Implementation Plan

- Status: implemented
- Spec: `docs/specs/windowless-headless.md`
- Date: 2026-11-25
- Branch: `refactor/windowless-headless`
- Tests: 194 passed (8 ignored) — `cargo test --workspace` green without Xvfb

## Overview

Split the single `metroidvania-rust` client binary into two feature-gated
binaries so the headless CI path stops initializing macroquad (and therefore
stops needing Xvfb). This is a **pure build/bin-split refactor**: no protocol,
server, simulation, state-machine, or `App`-API changes.

- **Box 1** moves the shared transport code (`RealPacketIO`, `HeadlessPacketIO`,
  `make_transport_factory`) out of `main.rs` into the lib (`client/src/network/transport.rs`).
- **Box 2** adds the Cargo `[features]` + two `[[bin]]` entries and creates the
  new windowless `src/main_headless.rs` (plain `fn main()`, private `run_headless`
  moved verbatim).
- **Box 3** trims `main.rs` to GUI-only, rewires the e2e spawn to the new binary,
  and drops Xvfb from CI.

There is **no new pure-logic unit test** in this refactor. Verification is the
existing e2e suite (`headless_timing`, `packet_log_rate`) passing **without
Xvfb**, plus the grep-able acceptance criteria at the end of this plan. The
tester's job here is to confirm the e2e suite still exercises the new binary and
to run the acceptance greps; no new `#[test]` is written.

## Public API (targets for the tester)

```rust
// client/src/network/transport.rs  (new module)
// module-private (NOT pub): the two PacketIO structs
struct RealPacketIO { /* sender, receiver, server_addr — moved verbatim */ }
struct HeadlessPacketIO { /* sender, receiver, server_addr — moved verbatim */ }

// pub: the only public item in the module; re-exported from lib.rs
pub fn make_transport_factory(server_addr: SocketAddr, headless: bool)
    -> TransportFactory
//   where TransportFactory = Box<dyn Fn() -> Box<dyn PacketIO>>  (unchanged, app.rs)
```

```rust
// client/src/lib.rs — add one re-export (alongside the existing `pub use network::packet_io::PacketIO;`)
pub use network::transport::make_transport_factory;
```

```rust
// client/src/main_headless.rs  (new binary)
fn main()                                       // plain fn main(), NOT async, NO #[macroquad::main]
fn run_headless(app: &mut App, duration_secs: u64)  // private, moved verbatim from main.rs
```

```toml
# client/Cargo.toml — build surface (exact)
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

Unchanged (must not change): `App`, `TransportFactory`, `PacketIO`, `World`,
`game::state_machine::{AppEvent, AppState, transition}`, all of `protocol/` and
`server/`, and `game::config::{TICK_HZ, HEADLESS_BUSY_WAIT_MS}` (still consumed
by the moved `run_headless`).

## Test cases (tester writes these first)

No new `#[test]` functions. The tester confirms the **existing** e2e suite still
passes with the new binary and runs these grep-based checks against the final tree:

| # | Verification (grep / run) | Expected |
| --- | --- | --- |
| T1 | `grep -R '#\[macroquad::main\]' client/src/main_headless.rs` | no match |
| T2 | `grep -n '^fn main' client/src/main_headless.rs` | exactly `fn main()`, no `async`, no attribute above it |
| T3 | `grep -n '\[features\]\|gui =\|headless =\|default =\|required-features\|\[\[bin\]\]' client/Cargo.toml` | both features + `default = ["gui"]` + two bins with correct `required-features` |
| T4 | `grep -n 'make_transport_factory\|RealPacketIO\|HeadlessPacketIO' client/src/main.rs` | no match (all moved out) |
| T5 | `grep -n 'pub fn make_transport_factory' client/src/network/transport.rs` | present |
| T6 | `grep -n 'make_transport_factory' client/src/lib.rs` | present (re-export) |
| T7 | `grep -n '\-\-headless' e2e/src/process.rs` | no match (and `--bin rustvania-headless` + `--features headless` present) |
| T8 | `grep -n 'xvfb\|Xvfb' .github/workflows/ci.yml` | no match |
| T9 | `cargo test --workspace` (no Xvfb / no `xvfb-run`) | passes; `headless_timing` + `packet_log_rate` green |
| T10 | `cargo clippy --all-targets --all-features --workspace -- -D warnings` | clean |
| T11 | `cargo fmt --check --all` | clean |
| T12 | `git diff --stat protocol/ server/` | empty |
| T13 | manual smoke: `cargo run --bin metroidvania-rust` | GUI window opens + plays (not CI-automatable) |

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Box 1 — Move transport code into the lib

- [x] done
- **Files**: create `client/src/network/transport.rs`; edit `client/src/network/mod.rs`; edit `client/src/lib.rs`.
- **Change**:
  - Move `RealPacketIO`, `HeadlessPacketIO`, and `make_transport_factory`
    **verbatim** from `client/src/main.rs` (lines 9–105) into the new
    `client/src/network/transport.rs`. The two structs stay **module-private**;
    `make_transport_factory` becomes `pub fn make_transport_factory(server_addr: SocketAddr, headless: bool) -> TransportFactory`.
  - Rewrite the four `log_packet` call sites inside `transport.rs` from
    `metroidvania_rust::network::logging::log_packet(...)` to
    `crate::network::logging::log_packet(...)` (they are now inside the lib).
  - Add the needed `use` lines (`crate::app::TransportFactory`,
    `crate::network::packet_io::PacketIO`, `crate::network::logging::log_packet`,
    `protocol::GamePacket`, `std::net::SocketAddr`).
  - Register `pub mod transport;` in `client/src/network/mod.rs`.
  - Add `pub use network::transport::make_transport_factory;` to `client/src/lib.rs`.
- **Note**: `main.rs` is **not** touched in this box — its local copies of the
  transport still exist and compile (they are private, so no name clash with the
  new lib items). Box 3 removes them.
- **Acceptance**: `cargo check -p metroidvania-rust` passes; `grep` confirms
  `pub fn make_transport_factory` in `transport.rs`, the module is registered,
  and the re-export is in `lib.rs` (T5, T6).
- **Test**: T5, T6 (grep) + `cargo check -p metroidvania-rust` (still compiles with the lib items added and `main.rs` unchanged).
- **Explain**
  - What: Created `client/src/network/transport.rs` and moved `RealPacketIO`,
    `HeadlessPacketIO`, and `make_transport_factory` there from `main.rs`. The two
    structs stay module-private; `make_transport_factory` is now
    `pub fn ... -> TransportFactory`. The four `log_packet` call sites now use
    `crate::network::logging::log_packet` (imported as the short name
    `log_packet`). Registered `pub mod transport;` in `network/mod.rs` and added
    `pub use network::transport::make_transport_factory;` to `lib.rs`. `main.rs`
    was left untouched (its private copies still compile).
  - Why: Both binaries need one shared transport implementation, and only the
    lib is linkable from both `main.rs` (GUI) and the future `main_headless.rs`.
    Moving it now (before the bin split) lets `cargo check` prove the lib items
    compile without touching `main.rs`.
  - Rust concepts: module privacy (module-private structs vs. a `pub` fn exposed
    only via the factory), re-exports (`pub use`), trait objects
    (`Box<dyn PacketIO>` / `TransportFactory = Box<dyn Fn() -> Box<dyn PacketIO>>`),
    and `use` paths (`crate::`-relative vs. the previous external
    `metroidvania_rust::` path now that the code is inside the lib).
  - Watch out: the plan's "add `use crate::network::logging::log_packet`" line
    would be an *unused* import if the call sites stayed fully-qualified
    (`crate::network::logging::log_packet(...)`). To keep `clippy -D warnings`
    clean I imported the short name and called `log_packet(...)`; likewise
    `protocol::GamePacket` is imported and used as the short name `GamePacket`.
    This is a deliberate minor deviation from the literal "rewrite to
    `crate::network::logging::log_packet(...)`" wording, not from the plan's
    intent (internal paths, no unused imports).

### Box 2 — Cargo feature/bin split + the windowless headless binary

- [x] done
- **Files**: edit `client/Cargo.toml`; create `client/src/main_headless.rs`.
- **Change**:
  - Add the exact `[features]` and two `[[bin]]` blocks from the "Public API"
    section to `client/Cargo.toml` (the explicit `[[bin]]` entries replace Cargo's
    implicit auto-detection of `src/main.rs`). `macroquad` stays an unconditional
    dependency — the two features are pure markers used only for `required-features`.
  - Create `client/src/main_headless.rs`:
    - **plain `fn main()`** — no `#[macroquad::main]`, no `macroquad` import, no
      `async`.
    - Parse `--duration=N` (default `0` = run until quit/SIGINT) and
      `--server=ADDR` (default `127.0.0.1:12345`, parsed as `SocketAddr`), same
      semantics as today. It does **not** accept `--headless`.
    - `let factory = make_transport_factory(server_addr, true);` and
      `let mut app = App::new(factory, true);` (importing `App` from
      `metroidvania_rust::app` and `make_transport_factory` from the lib).
    - Move `run_headless(&mut app, duration_secs)` **verbatim** from `main.rs`
      (lines 164–218, including its `/// Headless driver: …` doc comment) as a
      private `fn` in this file, keeping its behavior and prints. Carry over its imports (`game::config::{HEADLESS_BUSY_WAIT_MS, TICK_HZ}`,
      `game::state_machine::{AppEvent, AppState}`, `std::time::{Duration, Instant}`).
    - `main()` calls `run_headless(&mut app, duration_secs);` — never `app.draw()`,
      `clear_background`, `next_frame`, `screen_width`, or `screen_height`.
- **Note**: at the end of this box both binaries compile (`main.rs` still has its
  own private transport + `--headless` flag; Box 3 removes them).
- **Acceptance**: `cargo check --all-targets --all-features -p metroidvania-rust`
  builds both bins; `grep` finds no `#[macroquad::main]` in `main_headless.rs`
  and its entry point is a plain `fn main()` (T1, T2, T3).
- **Test**: T1, T2, T3 (grep) + `cargo check --all-targets --all-features -p metroidvania-rust`.
- **Explain**
  - What: Added `[features] gui/headless/default=["gui"]` plus two explicit
    `[[bin]]` entries to `client/Cargo.toml`, and created
    `client/src/main_headless.rs` with a plain `fn main()` (no `#[macroquad::main]`,
    no `macroquad` import, no `async`). It parses `--duration=N`/`--server=ADDR`
    exactly like the old headless path, builds `make_transport_factory(server_addr,
    true)` + `App::new(factory, true)`, and calls a private `run_headless` moved
    verbatim from `main.rs` (including its `/// Headless driver:` doc comment).
  - Why: The whole point of the refactor — `#[macroquad::main]` opens a window
    before `main()` runs, so headlessness has to live in a *separate* binary that
    never links a macroquad entry point. The explicit `[[bin]]` entries (with
    `required-features`) replace Cargo's implicit `src/main.rs` auto-detection and
    gate each binary on its marker feature.
  - Rust concepts: Cargo features as pure build-surface markers (no dependency
    gating), multiple `[[bin]]` targets in one package, and a plain synchronous
    `fn main()` as an alternative entry point to the macroquad async macro.
  - Watch out: `main_headless.rs` must not reference any macroquad item — it
    relies entirely on the macroquad-free `App::update`/`dispatch`/`state`/
    `is_quitting` API. `main.rs` still compiles in this box (its private transport
    + `--headless` flag remain); Box 3 removes them. `--duration=0` still means
    "run until quit/SIGINT" (unchanged semantics).

### Box 3 — Trim `main.rs` to GUI-only, rewire e2e, drop Xvfb from CI

- [x] done
- **Files**: edit `client/src/main.rs`; edit `e2e/src/process.rs`; edit `.github/workflows/ci.yml`; edit `README.md`.
- **Change**:
  - `client/src/main.rs`:
    - Keep `#[macroquad::main("Rustvania")]`, `sample_ui_event`, and the GUI loop
      (sample UI event → `set_game_input` → `set_viewport` → `update` → `draw` →
      `next_frame`).
    - Drop `--headless`/`--duration` parsing, `run_headless`, and the local
      `RealPacketIO`/`HeadlessPacketIO`/`make_transport_factory`.
    - Parse only `--server=ADDR` (default `127.0.0.1:12345`).
    - Build `let factory = make_transport_factory(server_addr, false);` and
      `App::new(factory, false)`, importing `make_transport_factory` from the lib.
    - Remove now-unused imports (`PacketIO`, `game::config::*`, `Duration` if unused;
      keep `Instant`, `AppState`, `AppEvent`, `App`, `SocketAddr`, and the
      macroquad prelude). **Do NOT keep `TransportFactory`** — after trimming,
      `main.rs` no longer names that type (it only writes
      `let factory = make_transport_factory(server_addr, false)` and
      `App::new(factory, false)`), so importing it would trip
      `clippy::unused_imports` under `-D warnings`. Ensure `cargo clippy` stays
      clean (no unused-import warnings).
  - `e2e/src/process.rs::run_headless_client`:
    - Change the client spawn to
      `cargo run -p metroidvania-rust --bin rustvania-headless --features headless -- --duration=N --server=127.0.0.1:12345`
      (drop `--headless`; add `--bin rustvania-headless` and `--features headless`).
    - Keep `RUSTVANIA_PACKET_LOG`, `SERVER_PORT_LOCK`, and `ServerGuard` unchanged.
    - Update the doc comment that references the old `--headless` subprocess path.
  - `.github/workflows/ci.yml`:
    - Remove the "Install Xvfb" step and the `xvfb-run` wrapper; run
      `cargo test --workspace` directly.
    - Change the clippy step to
      `cargo clippy --all-targets --all-features --workspace -- -D warnings`
      (plain `--all-targets` under default features would skip `rustvania-headless`
      since it requires the `headless` feature).
  - `README.md`:
    - Change `cd client && cargo run` (line 24) to
      `cd client && cargo run --bin metroidvania-rust` — with two `[[bin]]`
      entries and no default binary, bare `cargo run` now fails with "could not
      determine which binary to run".
- **Acceptance**: full quality gate green **without Xvfb** — `cargo test --workspace`
  passes, clippy/fmt clean, and greps T4/T7/T8 confirm `main.rs` has no transport
  leftovers, the e2e spawn uses the new binary with no `--headless`, and CI has no
  Xvfb.
- **Test**: T4, T7, T8 (grep) + `cargo test --workspace` (T9, no Xvfb) + `cargo clippy --all-targets --all-features --workspace -- -D warnings` (T10) + `cargo fmt --check --all` (T11).
- **Explain**
  - What: Rewrote `client/src/main.rs` to GUI-only — kept `#[macroquad::main]`,
    `sample_ui_event`, and the render loop, dropped the `--headless`/`--duration`
    parsing, `run_headless`, and the local `RealPacketIO`/`HeadlessPacketIO`/
    `make_transport_factory`. It now parses only `--server=ADDR`, imports
    `make_transport_factory` from the lib, and builds
    `App::new(make_transport_factory(server_addr, false), false)`. Rewired
    `e2e/src/process.rs::run_headless_client` to spawn
    `cargo run -p metroidvania-rust --bin rustvania-headless --features headless
    -- --duration=N --server=127.0.0.1:12345` (dropped `--headless`) and updated
    its doc comment. Removed the Xvfb install + `xvfb-run` wrapper from CI and
    changed clippy to `--all-targets --all-features --workspace`. Updated
    `README.md` to `cargo run --bin metroidvania-rust`.
  - Why: After the bin split, `main.rs` is the GUI-only entry point and must not
    carry headless-only machinery (or the now-dead `--headless` flag, whose
    removal makes `RealPacketIO` etc. unreachable from `main.rs`). The e2e
    subprocess must target the new windowless binary so tests run without a
    display; CI drops Xvfb because nothing opens a window anymore; bare
    `cargo run` is now ambiguous with two bins, hence `--bin`.
  - Rust concepts: import hygiene under `clippy::unused_imports -D warnings`
    (`TransportFactory`/`PacketIO`/`game::config::*`/`Duration` are gone because
    they're no longer named), path-based re-export usage
    (`metroidvania_rust::make_transport_factory`), and Cargo's `--bin`/`--features`
    CLI selection vs. `required-features` gating.
  - Watch out: the plan's T4 grep pattern
    (`make_transport_factory\|RealPacketIO\|HeadlessPacketIO` in `main.rs`) is
    too broad — `make_transport_factory` legitimately remains in `main.rs` as the
    lib import + call (Box 3 requires it), so T4 does not match "no match"
    literally. What is actually gone (and verified) is every *definition*:
    `struct RealPacketIO`, `struct HeadlessPacketIO`, `fn make_transport_factory`,
    and `fn run_headless` have no match. The two I/O structs are entirely absent.
    I judged "definitions moved out" (the real intent) satisfied and left the
    call site, exactly as Box 3 specifies.

## Definition of done

Acceptance criteria from `docs/specs/windowless-headless.md`, verbatim:

- [x] `client/src/main_headless.rs` contains no `#[macroquad::main]` (grep-able) and its entry point is a plain `fn main()`.
- [x] `client/Cargo.toml` declares `gui`/`headless` features, `default = ["gui"]`, and two `[[bin]]` entries with the correct `required-features`.
- [x] `make_transport_factory` (and the two I/O structs) live in `client/src/network/transport.rs`, are no longer in `main.rs`, and `make_transport_factory` is re-exported from `client/src/lib.rs`.
- [x] `e2e/src/process.rs` spawns `rustvania-headless` with `--features headless` and no `--headless` flag.
- [x] `cargo test --workspace` passes **without Xvfb** (the `headless_timing` and `packet_log_rate` subprocess tests run the windowless binary).
- [ ] `cargo run --bin metroidvania-rust` still opens the window and plays as before.
- [x] `cargo clippy --all-targets --all-features --workspace -- -D warnings` is clean (both binaries covered).
- [x] `cargo fmt --check --all` is clean.
- [x] No diff in `protocol/` or `server/`.

## Open questions

None — the spec resolves the prior "single binary vs separate binary" decision
(reversed from the Phase 2f ADR) and specifies the build surface, file layout,
e2e invocation, and CI changes exactly.
