# Phase 0 — Workspace & protocol foundation

- Status: draft
- Date: 2026-11-23
- Branch: `refactor/phase-0-foundation`

## Summary

Turn the two independent crates (`server` = `metroidvania-server`,
`client` = `metroidvania-rust`) into a single Cargo workspace and add a third
member, a shared `protocol` crate that owns the one canonical `GamePacket` enum.
This removes the duplication that has already drifted (`level_x: usize` vs
`u8`; `Hit { id }` vs `Hit { entity_id }`) and makes a single source of truth for
the wire protocol, consistent with ADR-001. The phase also adds the shared
quality gates that every later phase relies on: a workspace-level `rustfmt.toml`
and clippy lint config, a conventional-commit message template plus pre-commit
hooks (fmt/clippy/test), and a GitHub Actions CI workflow running the same three
checks.

No game behavior changes. No new packet variants. No protocol tests yet (that is
a later phase).

## Goals / non-goals

### Goals

1. A Cargo workspace at the repo root with members `server`, `client`, and a new
   `protocol` crate.
2. A shared `protocol` crate owning the single canonical `GamePacket` enum
   (`serde::Serialize` + `serde::Deserialize` + `Debug`), resolving the drift by
   choosing exactly one representation per field.
3. `server` and `client` depend on `protocol` and use its `GamePacket`; their
   duplicated definitions are deleted and imports updated.
4. Workspace-level `rustfmt.toml` and clippy lint configuration that apply to all
   three crates.
5. A conventional-commit message template and pre-commit hooks that run
   `cargo fmt --check`, `cargo clippy`, and `cargo test`.
6. A GitHub Actions CI workflow that runs `fmt`, `clippy`, and `test` on the
   workspace.

### Non-goals (explicitly out of scope)

- **Protocol round-trip serialize/deserialize tests** (postcard) — later phase.
- **Maps parsing tests** and **collision unit tests** (`solid_in_rect`,
  `grounded`, `try_move`) — later phase.
- **Dead-code / empty-branch cleanup** — *revised during implementation*: the
  clippy cleanup required by acceptance #16 (empty branches, collapsible `if`s,
  unnecessary casts, unused imports/vars) was pulled forward into Phase 0.
  Dead-code *removal* (unused functions `try_move`, `spawn_entities`, kept
  behind `#[allow(dead_code)]`) remains a later phase.
- **Netcode hardening** (fixed tick rate, reliable-vs-unreliable channel
  choices, interpolation, server-authoritative simulation, lag compensation) —
  ADR-001 decisions 2–3; later phase.
- Adding, removing, or reordering `GamePacket` variants beyond resolving the
  existing drift.
- Any change to game logic, rendering, or input handling.
- Changing the serialization format backend (postcard stays).

## Behavior

This is an infrastructure phase: there is **no player-visible behavior change**.
The game plays exactly as before; only the tooling and code layout change. The
observable behavior is for developers and CI, described below.

- **Single build unit.** `cargo build` from the repo root builds all three
  crates. `cargo build --workspace` and `cargo build -p <crate>` both work.
  A single root `Cargo.lock` is produced; the two per-crate `Cargo.lock` files
  (`server/Cargo.lock`, `client/Cargo.lock`) are superseded and removed.
- **One protocol definition.** There is exactly one `GamePacket` type in the
  repository, defined in `protocol/`. `server` and `client` reference it via a
  path dependency; neither re-declares it.
- **Formatting.** `cargo fmt --check` succeeds from the root with no diffs; the
  `rustfmt.toml` at the root is the only formatting config (no per-crate
  overrides).
- **Linting.** `cargo clippy --all-targets --workspace -- -D warnings` succeeds;
  the workspace-level clippy lint config is applied to all members.
- **Pre-commit gate.** A local commit is blocked (non-zero exit) if any of
  `cargo fmt --check`, `cargo clippy`, or `cargo test` fails. A commit message
  that does not match the Conventional Commits shape is rejected by the
  `commit-msg` hook (see Behavior edge cases).
- **CI.** On push and pull request, the CI workflow runs the same three gates;
  a red CI means the change is not mergeable.

### Edge cases

- **Wire-format change for `level_x`/`level_y`.** Narrowing `usize` → `u8`
  changes the number of bytes postcard emits for those two fields (`usize` is
  4 or 8 bytes depending on target; `u8` is 1). This is a breaking wire change
  but is acceptable pre-release and is the whole point of fixing the drift.
  Server and client change together in the same phase, so they remain
  interoperable with each other.
- **Field *rename* is not a wire change.** `Hit { id }` → `Hit { entity_id }`
  changes no bytes: postcard is positional, not name-based. Only the type change
  above affects the wire.
- **Pre-commit hooks are not installed by cloning.** Git hooks live in the local
  `.git` and are not version-controlled by default. The hooks must live in a
  committed directory (e.g. `.githooks/`) and be activated by a documented,
  idempotent command (e.g. `git config core.hooksPath .githooks`). The setup
  step must be recorded in `CONTRIBUTING.md`.
- **Hook failure blocks the commit, not the test.** `cargo test` inside the
  hook runs the workspace tests; if a test is slow, commits are slow. This is
  accepted for now; a later phase may split fast/slow hooks.
- **Empty workspace members.** `protocol` has no binary target; it is a library
  (`src/lib.rs`). `cargo test` on a library with no tests still passes (zero
  tests), so the phase's "test passes" gate does not depend on adding tests.
- **Lint denial in CI vs local.** Locally `clippy` may warn; CI treats warnings
  as errors (`-D warnings`). The acceptance bar is CI-green, not warning-free
  on every local run.

## Data model

### Canonical `GamePacket` (in `protocol/src/lib.rs`)

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum GamePacket {
    JoinRequest,
    JoinResponse { assigned_id: u8 },
    PlayerPos { id: u8, x: f32, y: f32, dir: f32, level_x: u8, level_y: u8 },
    Action { id: u8, kind: String, dir: f32 },
    Leave { id: u8 },
    Hit { entity_id: u8 },
}
```

The variant set, order, and every field type below are the single canonical
choice. Field-by-field decisions and rationale:

| Field | Canonical type | Server copy today | Client copy today | Decision & rationale |
| --- | --- | --- | --- | --- |
| `PlayerPos.level_x` | `u8` | `usize` | `u8` | **`u8`.** `usize` is platform-width (4 or 8 bytes) and non-portable on the wire; room/level indices never approach 255. The client already casts `as u8` on send and `as usize` on receive, so choosing `u8` keeps the client code path identical and only removes an unnecessary widening on the server. |
| `PlayerPos.level_y` | `u8` | `usize` | `u8` | **`u8`.** Same reasoning as `level_x`. |
| `Hit` field name | `entity_id` | `id` | `entity_id` | **`entity_id`.** It names *what* the id refers to — the entity that was hit/died — matching the codebase's `Entity` trait and the fact that `Hit` can later cover non-player entities. `id` alone is ambiguous among the many `id` fields. Renaming is wire-safe (postcard is positional). |
| `PlayerPos.id` | `u8` | `u8` | `u8` | Already agreed; no drift. `u8` suffices for ≤255 concurrent players. |
| `JoinResponse.assigned_id` | `u8` | `u8` | `u8` | Already agreed; no drift. |
| `Action.id` / `Leave.id` | `u8` | `u8` | `u8` | Already agreed; no drift. |
| `x`, `y`, `dir` | `f32` | `f32` | `f32` | Already agreed; no drift. |
| `Action.kind` | `String` | `String` | `String` | Already agreed; no drift. |

### New crate: `protocol`

- Package name `protocol`, `edition = "2024"`, library-only (`src/lib.rs`).
- Dependencies: `serde` with the `derive` feature (required for the derives).
  No `postcard`, no `laminar` — serialization/transport stay in `server` and
  `client`, which already depend on `postcard`.
- Exports `pub enum GamePacket` at the crate root. No other types in this phase.

### `server` and `client` changes

- Both `Cargo.toml` files add `protocol = { path = "../protocol" }`.
- Delete `server/src/game_packet.rs` and its `mod game_packet;` + `use
  game_packet::GamePacket;` in `server/src/main.rs`; import `protocol::GamePacket`
  instead.
- Delete the `GamePacket` enum from `client/src/network/client.rs`; import
  `protocol::GamePacket` where it is used (`client/src/network/client.rs`,
  `client/src/network/world.rs`, `client/src/world/lib.rs`).
- No other type or logic changes.

## Network (if any)

The wire protocol is unchanged in *meaning*; only two mechanical changes occur:

1. **`PlayerPos.level_x` / `level_y` become `u8`** (was `usize` on the server,
   `u8` on the client). This changes the byte size of those two fields on the
   wire (a breaking change, accepted pre-release). Both sides change together,
   so server↔client stays interoperable.
2. **`Hit` field renamed `id` → `entity_id`.** No wire change: postcard encodes
   enum variants by index and struct fields by position, never by name.

No packet variants are added, removed, or reordered; variant order must stay
exactly as listed in the Data model so existing postcard-encoded payloads keep
the same variant indices.

## Acceptance criteria

Each item is a checkable statement of "done". Not an implementation plan.

### Workspace

1. A `Cargo.toml` exists at the repo root with a `[workspace]` whose members are
   exactly `server`, `client`, and `protocol`.
2. `cargo build --workspace` from the root succeeds with no errors.
3. Exactly one `Cargo.lock` exists (at the root); `server/Cargo.lock` and
   `client/Cargo.lock` are gone.
4. `cargo test --workspace` and `cargo clippy --workspace` both run from the
   root and succeed.

### Protocol crate

5. `protocol/Cargo.toml` declares `name = "protocol"`, `edition = "2024"`, and a
   `serde` dependency with the `derive` feature.
6. `protocol/src/lib.rs` defines exactly one `GamePacket` enum with the exact
   variants and field types from the Data model section.
7. `grep -r "enum GamePacket"` across the repository returns exactly one hit, in
   `protocol/`.
8. The canonical enum compiles with `derive(Debug, serde::Serialize,
   serde::Deserialize)`.

### Wiring

9. `server/Cargo.toml` and `client/Cargo.toml` each declare
   `protocol = { path = "../protocol" }`.
10. `server/src/game_packet.rs` no longer exists; `server/src/main.rs` imports
    `protocol::GamePacket`.
11. `client/src/network/client.rs` no longer declares `GamePacket`; the client
    files that use it (`network/world.rs`, `world/lib.rs`) import
    `protocol::GamePacket` and compile. (`network/client.rs` no longer
    references the type at all.)
12. Both `cargo build -p metroidvania-server` and
    `cargo build -p metroidvania-rust` succeed after the wiring.

### Formatting & linting

13. A `rustfmt.toml` exists at the repo root and is the only formatting config in
    the repo (no per-crate `rustfmt.toml`).
14. `cargo fmt --check --all` reports no diffs.
15. A workspace-level clippy lint configuration exists (in the root `Cargo.toml`
    `[workspace.lints]`, or a root `clippy.toml` applied to all members).
16. `cargo clippy --all-targets --workspace -- -D warnings` exits 0.

### Commit template & hooks

17. A conventional-commit message template file exists (e.g. `.gitmessage`) with
    the `type(scope): subject` / `body` shape covering `feat`, `fix`, `refactor`,
    `test`, `docs`, `chore`.
18. A committed `commit-msg` hook rejects a commit whose first line does not match
    the Conventional Commits pattern.
19. A committed `pre-commit` hook runs `cargo fmt --check`, `cargo clippy`, and
    `cargo test`, and exits non-zero (blocking the commit) if any fails.
20. The hooks live under a committed directory (e.g. `.githooks/`), are
    executable, and `CONTRIBUTING.md` documents the idempotent install command
    (e.g. `git config core.hooksPath .githooks`).

### CI

21. A `.github/workflows/ci.yml` exists and, on push and pull_request, runs three
    jobs/steps equivalent to `cargo fmt --check`, `cargo clippy
    --all-targets --workspace -- -D warnings`, and `cargo test --workspace`.
22. A commit violating any of those three gates would fail CI (no `continue-on-error`
    on the gates).
