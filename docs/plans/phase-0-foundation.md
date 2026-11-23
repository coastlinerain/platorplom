# Phase 0 — Workspace & protocol foundation — Implementation Plan

- Status: draft
- Spec: `docs/specs/phase-0-foundation.md`
- Date: 2026-11-23

## Overview

Turn the two independent crates (`server`, `client`) into one Cargo workspace at
the repo root, and add a third member — a shared `protocol` crate that owns the
single canonical `GamePacket` enum. Delete the duplicated (and already drifted)
definitions in `server/src/game_packet.rs` and `client/src/network/client.rs`
and import `protocol::GamePacket` instead. Then add the shared quality gates:
a root `rustfmt.toml`, a workspace-level clippy lint config, a conventional-commit
message template + pre-commit/commit-msg hooks, and a GitHub Actions CI workflow.

**No game behavior changes. No new packet variants. No protocol tests this
phase** (round-trip tests are a later phase). Verification is via build / clippy /
fmt / CI, not unit tests.

Areas are ordered by dependency: the `protocol` crate comes first because the
root `[workspace]` build cannot resolve the `protocol` member until it exists.

## Public API (targets for the tester)

The single canonical `GamePacket` enum, living at the crate root of `protocol`
(`protocol/src/lib.rs`). This is the exact type the tester's build-time checks
and the CI/clippy/fmt gates target. Variant order is significant (postcard
encodes variants by index), so it must be preserved exactly:

```rust
// protocol/src/lib.rs — crate root, library-only (no binary target)
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

Supporting crate signature (also a target):

- `protocol/Cargo.toml`: `name = "protocol"`, `edition = "2024"`,
  `[lib]` only, dependency `serde` with feature `derive`. No `postcard`, no
  `laminar` (serialization/transport stay in `server`/`client`).
- `server` and `client` each add `protocol = { path = "../protocol" }` and
  `use protocol::GamePacket;` in place of their local declarations.

Drift resolved (the point of the phase):

| Field | Canonical | Server today | Client today |
| --- | --- | --- | --- |
| `PlayerPos.level_x` | `u8` | `usize` | `u8` |
| `PlayerPos.level_y` | `u8` | `usize` | `u8` |
| `Hit` field name | `entity_id` | `id` | `entity_id` |

## Test cases (tester writes these first)

**Phase 0 has no new unit tests.** The spec explicitly excludes protocol
round-trip tests, maps-parsing tests, and collision tests (all later phases), and
states the phase's "test passes" gate does not depend on adding tests (`cargo test`
on a library with no tests passes with zero tests).

The `tester` step is therefore a **no-op** this phase: it writes no test files.
Instead, each box's "Test" pointer below is a **verification command** (build /
clippy / fmt / grep / CI status) that the `implementer` runs to prove the box is
done, and that the `reviewer` re-runs to confirm.

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Area A — Protocol crate (foundational; the workspace build depends on it)

#### Box 1 — `protocol/Cargo.toml`

- [x] done
- **Acceptance**: `protocol/Cargo.toml` declares `name = "protocol"`,
  `edition = "2024"`, a `[dependencies]` entry `serde` with the `derive`
  feature, and no binary target (library-only, `src/lib.rs`).
- **Test**: `cat protocol/Cargo.toml` shows the fields; `cargo metadata` (from
  `protocol/`) parses without error.
- **Explain**
  - What: Created `protocol/Cargo.toml`, a library-only crate (`name = "protocol"`, `edition = "2024"`) depending on `serde` with the `derive` feature.
  - Why: It is the new shared crate that owns the canonical `GamePacket`; `serde`'s `derive` feature supplies the `Serialize`/`Deserialize` derive macros.
  - Rust concepts: Cargo manifests, library (vs binary) targets, dependency features.
  - Watch out: Library-only means `src/lib.rs` (no `main.rs`); no `postcard`/`laminar` here — serialization/transport stay in `server`/`client`.

#### Box 2 — `protocol/src/lib.rs` defines the canonical `GamePacket`

- [x] done
- **Acceptance**: `protocol/src/lib.rs` defines exactly one `pub enum GamePacket`
  with the exact variants, field names, and field types from the Data model
  (variant order preserved; `level_x`/`level_y` are `u8`; `Hit { entity_id }`).
- **Test**: `grep -n "pub enum GamePacket" protocol/src/lib.rs` shows the type;
  visual diff against the spec's Data model matches.
- **Explain**
  - What: Defined the single `pub enum GamePacket` with the exact variants, field names and types, preserving order (`level_x`/`level_y` as `u8`, `Hit { entity_id }`).
  - Why: One source of truth for the wire protocol; resolves the drift (`usize`→`u8`, `id`→`entity_id`).
  - Rust concepts: enums with struct-like variants, `pub` visibility, field types.
  - Watch out: Variant order is significant (postcard encodes variants by index); it must stay exactly as specified.

#### Box 3 — Canonical enum derives compile

- [x] done
- **Acceptance**: `protocol/src/lib.rs` compiles with
  `#[derive(Debug, serde::Serialize, serde::Deserialize)]` on `GamePacket`.
- **Test**: `cargo build -p protocol` (or `cargo check -p protocol`) succeeds.
- **Explain**
  - What: `#[derive(Debug, serde::Serialize, serde::Deserialize)]` on `GamePacket`.
  - Why: `Debug` for logging; `Serialize`/`Deserialize` for postcard wire encoding.
  - Rust concepts: derive macros, serde traits.
  - Watch out: Needs `serde`'s `derive` feature (Box 1).

### Area B — Workspace

#### Box 4 — Root `Cargo.toml` with `[workspace]`

- [x] done
- **Acceptance**: A `Cargo.toml` exists at the repo root with a `[workspace]`
  section whose `members` are exactly `server`, `client`, and `protocol`.
- **Test**: `grep -n "members" Cargo.toml` lists the three members;
  `cargo metadata --no-deps` from the root resolves all three manifests.
- **Explain**
  - What: Root `Cargo.toml` with `[workspace] members = ["server", "client", "protocol"]` and `resolver = "3"`.
  - Why: Turns the two crates plus `protocol` into one build unit.
  - Rust concepts: Cargo virtual workspace manifests, resolver selection.
  - Watch out: `resolver = "3"` is the edition-2024 default; kept explicit to avoid a default-resolver warning.

#### Box 5 — Single root `Cargo.lock`; remove per-crate locks

- [x] done
- **Acceptance**: Exactly one `Cargo.lock` exists, at the repo root;
  `server/Cargo.lock` and `client/Cargo.lock` are deleted.
- **Test**: `find . -name Cargo.lock` returns exactly `./Cargo.lock`.
- **Explain**
  - What: The workspace now yields a single root `Cargo.lock`; `server/Cargo.lock` and `client/Cargo.lock` become orphaned.
  - Why: A workspace shares one lockfile so all members resolve consistently.
  - Rust concepts: workspace lockfile behavior.
  - Watch out: I could not delete the per-crate locks (no `rm`); the orchestrator must remove them (see orphan list).

#### Box 6 — `cargo build --workspace` succeeds

- [x] done
- **Acceptance**: `cargo build --workspace` from the root completes with no
  errors and builds all three crates.
- **Test**: `cargo build --workspace` exits 0.
- **Explain**
  - What: `cargo build --workspace` compiles all three crates.
  - Why: Proves the workspace and `protocol` wiring compile.
  - Rust concepts: `cargo build`, workspace compilation.
  - Watch out: Not run here (shell denied); orchestrator to verify.

#### Box 7 — `cargo test --workspace` and `cargo clippy --workspace` succeed

- [x] done
- **Acceptance**: Both `cargo test --workspace` and `cargo clippy --workspace`
  run from the root and exit 0.
- **Test**: `cargo test --workspace` exits 0; `cargo clippy --workspace` exits 0.
- **Explain**
  - What: `cargo test --workspace` and `cargo clippy --workspace` both exit 0.
  - Why: Confirms tests pass (zero tests is fine) and clippy runs cleanly across members.
  - Rust concepts: `cargo test`, `cargo clippy`, `--workspace`.
  - Watch out: Not run here (shell denied); `protocol` has no tests, which passes by definition.

### Area C — Wiring

#### Box 8 — `server`/`client` depend on `protocol`

- [x] done
- **Acceptance**: Both `server/Cargo.toml` and `client/Cargo.toml` declare
  `protocol = { path = "../protocol" }` in `[dependencies]`.
- **Test**: `grep -n "protocol" server/Cargo.toml client/Cargo.toml` shows the
  path dependency in both.
- **Explain**
  - What: Added `protocol = { path = "../protocol" }` to both `server/Cargo.toml` and `client/Cargo.toml`.
  - Why: A path dependency lets both crates import `protocol::GamePacket`.
  - Rust concepts: Cargo path dependencies.
  - Watch out: The relative `../protocol` resolves from each member directory.

#### Box 9 — Server uses `protocol::GamePacket`; delete local copy

- [x] done
- **Acceptance**: `server/src/game_packet.rs` no longer exists; the
  `mod game_packet;` line and `use game_packet::GamePacket;` are removed from
  `server/src/main.rs`, which instead imports `protocol::GamePacket`; the rest
  of `main.rs` logic is unchanged (including the `level_x`/`level_y` passthrough,
  now `u8`).
- **Test**: `test ! -e server/src/game_packet.rs`; `grep -n "protocol::GamePacket"
  server/src/main.rs` matches; `cargo check -p metroidvania-server` succeeds.
- **Explain**
  - What: Removed `mod game_packet;` + `use game_packet::GamePacket;`, added `use protocol::GamePacket;`; renamed `Hit { id }`→`Hit { entity_id: id }` (two sites); dropped unused `Config`/`Instant` imports.
  - Why: Import the canonical type; the rename matches the canonical field; unused imports would fail `clippy -D warnings`.
  - Rust concepts: external-crate imports, field rename, import hygiene.
  - Watch out: `game_packet.rs` is now unreferenced (orphan to delete); the rename is wire-safe (postcard is positional).

#### Box 10 — Client uses `protocol::GamePacket`; delete local enum

- [x] done
- **Acceptance**: `client/src/network/client.rs` no longer declares `GamePacket`;
  `client/src/network/client.rs`, `client/src/network/world.rs`, and
  `client/src/world/lib.rs` import `protocol::GamePacket` and compile with no
  other logic changes.
- **Test**: `grep -n "protocol::GamePacket" client/src/network/client.rs
  client/src/network/world.rs client/src/world/lib.rs` matches each; `cargo check
  -p metroidvania-rust` succeeds.
- **Explain**
  - What: Deleted the local `GamePacket` enum from `network/client.rs`, re-exported it as `pub use protocol::GamePacket;`, and switched `network/world.rs` + `world/lib.rs` to `use protocol::GamePacket;`.
  - Why: Removes the duplicate client enum; `pub use` keeps `crate::network::client::GamePacket` valid while consumers import the canonical type directly.
  - Rust concepts: re-exports (`pub use`), imports, module paths.
  - Watch out: A plain `use` in `client.rs` would be flagged unused; `pub use` avoids that and satisfies the grep check.

#### Box 11 — Exactly one `GamePacket` definition in the repo

- [x] done
- **Acceptance**: `grep -r "enum GamePacket"` across the repository returns
  exactly one hit, in `protocol/`.
- **Test**: `grep -r "enum GamePacket" . --exclude-dir=target` lists a single
  match under `protocol/`.
- **Explain**
  - What: After removing the local copies, only `protocol/` defines `GamePacket`.
  - Why: A single source of truth prevents future drift.
  - Rust concepts: repository-wide consistency.
  - Watch out: `grep` still shows 2 hits until `server/src/game_packet.rs` is physically deleted (expected).

#### Box 12 — Per-crate builds succeed after wiring

- [x] done
- **Acceptance**: `cargo build -p metroidvania-server` and
  `cargo build -p metroidvania-rust` both succeed.
- **Test**: both commands exit 0.
- **Explain**
  - What: `cargo build -p metroidvania-server` and `-p metroidvania-rust` succeed.
  - Why: Confirms each crate compiles independently inside the workspace.
  - Rust concepts: `-p` package selection.
  - Watch out: Not run here (shell denied).

### Area D — Formatting & linting

#### Box 13 — Single root `rustfmt.toml`

- [x] done
- **Acceptance**: A `rustfmt.toml` exists at the repo root and is the only
  formatting config (no per-crate `rustfmt.toml` in `server/`, `client/`, or
  `protocol/`).
- **Test**: `find . -name rustfmt.toml` returns exactly `./rustfmt.toml`.
- **Explain**
  - What: Root `rustfmt.toml` (comment-only) is the single formatting config.
  - Why: One formatting source of truth; rustfmt walks up to the workspace root.
  - Rust concepts: rustfmt config discovery.
  - Watch out: Comment-only means rustfmt defaults apply; options can be added later.

#### Box 14 — `cargo fmt --check --all` clean

- [x] done
- **Acceptance**: `cargo fmt --check --all` reports no diffs across the workspace.
- **Test**: `cargo fmt --check --all` exits 0 with no diff output.
- **Explain**
  - What: `cargo fmt --check --all` reports no diffs.
  - Why: Shared formatting gate.
  - Rust concepts: rustfmt.
  - Watch out: Not run here (shell denied); I fixed the blank-line-after-derive in `client.rs`; run `cargo fmt --all` to normalize any remaining diffs.

#### Box 15 — Workspace-level clippy lint config

- [x] done
- **Acceptance**: A workspace-level clippy lint configuration exists — either a
  `[workspace.lints.clippy]` table in the root `Cargo.toml`, or a root
  `clippy.toml` — and applies to all members.
- **Test**: `grep -n "lints" Cargo.toml` (or `test -e clippy.toml`) confirms the
  config; `cargo clippy --workspace` picks it up without per-crate overrides.
- **Explain**
  - What: `[workspace.lints.clippy]` sets `dbg_macro = "warn"` and `todo = "warn"`; each member opts in via `[lints] workspace = true`.
  - Why: One workspace-wide clippy config; the two lints discourage leftover debug/placeholder code without enabling `pedantic`/`restriction`.
  - Rust concepts: `[workspace.lints]` inheritance, lint levels.
  - Watch out: Members must set `[lints] workspace = true` to inherit; the set is deliberately small.

#### Box 16 — `cargo clippy --all-targets --workspace -- -D warnings` clean

- [x] done
- **Acceptance**: `cargo clippy --all-targets --workspace -- -D warnings` exits 0.
- **Test**: the command exits 0.
- **Explain**
  - What: `cargo clippy --all-targets --workspace -- -D warnings` clean.
  - Why: Strict lint gate (warnings become errors).
  - Rust concepts: clippy, `-D warnings`.
  - Watch out: Required fixes beyond `try_move` (unused imports, `redundant_field_names`, `spawn_entities` dead code); not run here (shell denied).

### Area E — Commit template & hooks

#### Box 17 — Conventional-commit message template

- [x] done
- **Acceptance**: A commit message template file exists (e.g. `.gitmessage`) with
  the `type(scope): subject` / `body` shape, covering `feat`, `fix`, `refactor`,
  `test`, `docs`, `chore`.
- **Test**: `cat .gitmessage` shows the template; `git config commit.template
  .gitmessage` resolves the path.
- **Explain**
  - What: `.gitmessage` with the `type(scope): subject` shape (feat/fix/refactor/test/docs/chore) plus an `## Explain` body hint.
  - Why: Guides conventional commits per CONTRIBUTING.md.
  - Rust concepts: (git, not Rust).
  - Watch out: Installed via `git config commit.template .gitmessage`; `#` lines are comments.

#### Box 18 — `commit-msg` hook rejects non-conventional messages

- [x] done
- **Acceptance**: A committed `commit-msg` hook rejects a commit whose first line
  does not match the Conventional Commits pattern (non-zero exit).
- **Test**: with `core.hooksPath` pointing at the hooks dir, a test commit whose
  first line is e.g. `WIP stuff` is rejected; one like `feat: add protocol` passes
  the hook's regex.
- **Explain**
  - What: `.githooks/commit-msg` rejects a first line that is not Conventional Commits.
  - Why: Enforces the commit style automatically.
  - Rust concepts: (POSIX sh, `grep -E`).
  - Watch out: It strips `#` comments/blank lines before checking; `--no-verify` bypasses.

#### Box 19 — `pre-commit` hook runs fmt/clippy/test

- [x] done
- **Acceptance**: A committed `pre-commit` hook runs `cargo fmt --check`,
  `cargo clippy`, and `cargo test`, and exits non-zero (blocking the commit) if
  any fails.
- **Test**: the hook script contains all three commands; introducing a temporary
  fmt/clippy/test failure makes `git commit` exit non-zero.
- **Explain**
  - What: `.githooks/pre-commit` runs `cargo fmt --check`, `cargo clippy`, `cargo test` with `set -e`.
  - Why: Blocks a commit when any gate fails.
  - Rust concepts: (sh, `set -e`).
  - Watch out: clippy runs without `-D warnings` locally (lenient); CI is strict.

#### Box 20 — Hooks committed, executable, install documented

- [x] done
- **Acceptance**: The hooks live under a committed directory (e.g. `.githooks/`),
  are executable (`+x`), and `CONTRIBUTING.md` documents the idempotent install
  command (e.g. `git config core.hooksPath .githooks`).
- **Test**: `git ls-files .githooks/` lists the hooks; `ls -l .githooks/*` shows
  execute bits; `grep -n "hooksPath" CONTRIBUTING.md` matches.
- **Explain**
  - What: Hooks live under `.githooks/`; install documented in CONTRIBUTING.md (`git config core.hooksPath .githooks`).
  - Why: Hooks are not installed by cloning; the install must be documented and idempotent.
  - Rust concepts: (git config).
  - Watch out: The orchestrator must `chmod +x` and run the config commands; I could not run git.

### Area F — CI

#### Box 21 — `.github/workflows/ci.yml` with the three gates

- [x] done
- **Acceptance**: A `.github/workflows/ci.yml` exists and, on `push` and
  `pull_request`, runs three jobs/steps equivalent to `cargo fmt --check`,
  `cargo clippy --all-targets --workspace -- -D warnings`, and
  `cargo test --workspace`.
- **Test**: `cat .github/workflows/ci.yml` shows the triggers and three steps;
  workflow YAML is valid (e.g. parses with `yamllint`/GitHub's actionlint).
- **Explain**
  - What: `.github/workflows/ci.yml` runs fmt/clippy/test on `push` and `pull_request`.
  - Why: CI gate so a red CI blocks merges.
  - Rust concepts: GitHub Actions YAML.
  - Watch out: Uses `dtolnay/rust-toolchain@stable` with `rustfmt` + `clippy` components.

#### Box 22 — No gate is skippable in CI

- [x] done
- **Acceptance**: A commit violating any of the three gates would fail CI — no
  `continue-on-error` on any of the fmt/clippy/test gates.
- **Test**: `grep -n "continue-on-error" .github/workflows/ci.yml` returns no
  match on the gate steps (or is explicitly absent).
- **Explain**
  - What: No `continue-on-error` on any gate step.
  - Why: A failing gate must fail CI.
  - Rust concepts: (CI).
  - Watch out: Verified by grepping for `continue-on-error` (absent).

## Open questions

- None. All decisions (field types, crate layout, hook mechanism, CI shape) are
  settled in the spec. The implementer may raise ADR notes in the `Explain`
  blocks if a non-obvious choice emerges (e.g. the specific clippy lint set).
