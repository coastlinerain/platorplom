# Single interpolation path — Implementation Plan

- Status: implemented (2026-11-26; 241 passed / 3 ignored workspace-wide, i.e.
  baseline 247 − 6; `interpolate_at_tick` is the only interpolation entry point)
- Spec: `docs/specs/netcode-hardening-roadmap.md` (spec 5)
- Date: 2026-11-26
- Branch: `refactor/single-interpolation-path` (already checked out, branched from
  `dev`; do not create a branch)
- Roadmap: spec 5 of `docs/specs/netcode-hardening-roadmap.md`
- Pipeline: **single box** — the implementer writes the code **and** the test
  edits in one commit (or a small related set) → code review → qa. There is no
  separate tester step and no intentional RED window.

> **Implementation note (2026-11-26).** The owner revised two points of the draft
> while briefing the implementer, so the code differs from the text below in
> exactly two places:
>
> 1. `interpolate_evicts_old_snapshots` was **renamed** to
>    `push_evicts_old_snapshots` (body and doc comment unchanged) instead of being
>    deleted, keeping the only direct test of the 5-entry ring eviction. Net test
>    change is therefore **−6** (six deleted, one renamed), not −7.
> 2. `client/src/render.rs:70` **was** updated (`RemoteInterp::interpolate` →
>    `RemoteInterp::interpolate_at_tick`) instead of being left as a docs-cleanup
>    follow-up. It is a doc-comment-only edit.
>
> Everything else (deleted methods, deleted D1–D6, untouched K1–K4/`push`/
> `set_anchor`/`new`/`Default`/fields, no other file touched) matches this plan.

## Overview

`RemoteInterp` currently exposes two interpolation APIs:

1. the tick-based `interpolate_at_tick(nemesis, target_tick,
   last_known_server_tick, last_known_arrival)`, which `World::draw` uses, and
2. the legacy wall-clock `interpolate` / `interpolate_at`, whose only callers are
   the file's own `#[cfg(test)]` module.

This change deletes the legacy pair **and every test that targets it**, leaving
exactly one interpolation path. Nothing else moves: `interpolate_at_tick`,
`push`, `set_anchor`, the `timestamps` ring, and the `last_known_*` anchor fields
are unchanged, and no production call site changes (there is only one, in
`client/src/world/game.rs`).

This is a **pure deletion task**. No new logic and no new tests are written; the
"test work" is deleting the six legacy tests that call the wall-clock path (and
renaming the seventh), and the verification is "the existing suite stays green,
the four tick tests still pass, and the grep acceptance is clean".

**Baseline:** before editing, record the `cargo test --workspace` result on the
branch tip (247 passed / 3 ignored). After the box the suite must be green with
exactly **6 fewer tests** (all in `client`'s lib target). Any *failure* is a
regression from this change.

## Non-goals (hard constraints — do NOT do these)

- Do **not** change the body, signature, semantics, or name of
  `interpolate_at_tick`.
- Do **not** touch `push`, `set_anchor`, `RemoteInterp::new`, `Default`, the
  `timestamps: [Option<Instant>; 5]` field, `last_known_server_tick`, or
  `last_known_arrival`.
- Do **not** delete the four `interpolate_at_tick_*` tests, the `make_nemesis`
  helper, or the test module's `use` lines.
- Do **not** add new interpolation tests, and do not "improve" the stale-guard
  threshold. This is a deletion refactor.
- Do **not** touch `client/src/render.rs` except for the stale doc comment at
  line 70 naming `RemoteInterp::interpolate`; the owner approved that one-line
  doc fix (no code change) as part of this box. (The draft deferred it as a later
  docs cleanup.)
- Do **not** touch `client/src/world/game.rs` or `client/src/network/world.rs`
  — both already use only `interpolate_at_tick` / `push` / `set_anchor`.
- Do **not** touch `protocol/`, `server/`, `game/`, or `e2e/`.
- Do **not** reformat the whole file; keep the diff to the deleted ranges plus
  the section header (see Box 1) so review stays mechanical.
- Do **not** remove `use std::time::Instant;` — the struct fields, `new`,
  `push`, `set_anchor`, and `interpolate_at_tick` all still need it.
- Do **not** remove `Duration` / `Instant` from the test module's `use` — the
  four surviving tick tests use both.
- Do **not** add `#[allow(dead_code)]` or otherwise keep the legacy methods
  "just in case". The point of the spec is that the path is gone.

## Public API (exact shape after the box)

```rust
// client/src/network/interpolation.rs

pub struct RemoteInterp {
    pub timestamps: [Option<Instant>; 5],
    pub last_known_server_tick: u64,
    pub last_known_arrival: Instant,
}

impl RemoteInterp {
    pub fn new() -> Self;
    pub fn push(&mut self, nemesis: &mut Nemesis, x: f32, y: f32, dir: f32);
    pub fn set_anchor(&mut self, server_tick: u64);

    /// KEPT — the single interpolation path.
    pub fn interpolate_at_tick(
        &self,
        nemesis: &Nemesis,
        target_tick: u64,
        last_known_server_tick: u64,
        last_known_arrival: Instant,
    ) -> Option<(f32, f32, f32)>;

    // DELETED — must not exist anywhere:
    // pub fn interpolate(&self, nemesis: &Nemesis) -> Option<(f32, f32, f32)>
    // pub fn interpolate_at(&self, nemesis: &Nemesis, now: Instant)
    //     -> Option<(f32, f32, f32)>
}

impl Default for RemoteInterp {
    fn default() -> Self; // unchanged, delegates to new()
}
```

Re-export note (the public surface is narrower than it looks):

- `client/src/network/mod.rs` declares `pub mod interpolation;` and
  `client/src/lib.rs` declares `pub mod network;`, so the type is *reachable* as
  `metroidvania_rust::network::interpolation::RemoteInterp`.
- It is **not re-exported at the crate root**. `client/src/lib.rs` exports
  `App`, `TransportFactory`, `PacketIO`, `make_transport_factory`,
  `handle_packet`, `send_join_req`, `update_network`, `World`, and
  `request_room_change` — no `RemoteInterp`.
- No crate outside `client` mentions `RemoteInterp` (verified by grep). Its only
  non-test uses are the `World::other_interp` field and the sole production call
  in `World::draw`.
- Consequence: deleting the two methods is a breaking change only for the
  in-file test module. There is no wire, protocol, or crate-root surface change.

Sole production caller (must remain untouched):

```rust
// client/src/world/game.rs — World::draw
interp.interpolate_at_tick(
    nemesis,
    target_tick,
    interp.last_known_server_tick,
    interp.last_known_arrival,
)
```

## Test cases

No new tests are written (nothing to make RED). The implementer **deletes six**
tests from `client/src/network/interpolation.rs`'s `#[cfg(test)] mod tests`,
**renames one**, and keeps four.

### Deleted (D1–D6)

| # | Test (pre-edit line) | Why it dies |
| --- | --- | --- |
| D1 | `interpolate_empty_returns_none` (257) | calls `interp.interpolate_at(&nemesis, now)` |
| D2 | `interpolate_single_snapshot_returns_it` (269) | calls `interp.interpolate_at(&nemesis, now)` |
| D3 | `interpolate_two_snapshots_lerps_midpoint` (287) | calls `interp.interpolate_at(&nemesis, now)` |
| D4 | `interpolate_clamps_to_most_recent` (319) | calls `interp.interpolate_at(&nemesis, now)` |
| D5 | `interpolate_stale_guard_snaps_to_latest` (353) | calls `interp.interpolate_at(&nemesis, now)` |
| D6 | `interpolate_stale_guard_boundary` (377) | calls `interp.interpolate_at(&nemesis, now)` twice |

Each deletion removes the whole `///` doc comment + `#[test]` + function body.
Do **not** delete `use super::RemoteInterp;`, `use
game::entities::nemesis::Nemesis;`, `use std::time::{Duration, Instant};`, or the
`make_nemesis` helper — the surviving tests use all of them.

### Renamed (R1)

| # | Test (pre-edit line) | Change |
| --- | --- | --- |
| R1 | `interpolate_evicts_old_snapshots` (420) | renamed to `push_evicts_old_snapshots`; body and doc comment unchanged. It never called the legacy API, so it survives — under a name that matches the `push` API it exercises. |

The draft deleted this test (giving up the only direct coverage of `push`'s
5-entry ring eviction); the owner reversed that decision, so it stays. The
`Group D` divider above it becomes `// Push / ring-buffer tests`.

### Kept verbatim (K1–K4 — the tick-based path)

| # | Test (pre-edit line) | Covers |
| --- | --- | --- |
| K1 | `interpolate_at_tick_lerps_midpoint` (454) | target at the oldest tick; position from the bracket; `dir` snaps to newer |
| K2 | `interpolate_at_tick_before_oldest_snaps_to_oldest` (491) | clamp before the buffered range |
| K3 | `interpolate_at_tick_after_newest_snaps_to_newest` (526) | clamp after the buffered range |
| K4 | `interpolate_at_tick_stale_guard_snaps_to_newest` (561) | >200 ms stale guard |

Do not edit their bodies. `interpolate_at_tick` is now the only interpolation
behavior under test, and these four tests are the whole regression net for it.

### Section header (comments only)

- The now-empty `// Group D — Entity interpolation tests (updated for N=5)`
  divider above D1 was deleted; the header above R1 now reads
  `// Push / ring-buffer tests`.
- Keep the `// New tests for interpolate_at_tick (tick-based interpolation)`
  divider above K1; it is still accurate.

## Implementation checklist

Each box is a small, independently verifiable step. The `implementer` fills the
`Explain` fields.

### Box 1 — Delete the legacy wall-clock path and its legacy tests

- [x] done
- **Files**: `client/src/network/interpolation.rs`, plus the one-line doc fix in
  `client/src/render.rs` (owner-approved; see the implementation note).
- **Change** (line numbers are pre-edit anchors; work **bottom-up** so they stay
  valid):

  1. Delete the test block **first**, lines **250–415** (i.e. the blank line
     after `make_nemesis`, the `// ------` / `// Group D — Entity interpolation
     tests (updated for N=5)` / `// ------` divider, and D1–D6 through their doc
     comments), leaving one blank line before the `/// AC 17` doc comment of R1.
  2. Rename `interpolate_evicts_old_snapshots` → `push_evicts_old_snapshots`
     and replace the deleted divider with `// Push / ring-buffer tests`.
     Do not touch K1–K4 below it.
  3. Delete the legacy methods **and** their doc comments, lines **169–227**
     (the blank line after `interpolate_at_tick` plus):
     - `/// Legacy interpolation method (wall-clock based)...` +
       `pub fn interpolate(&self, nemesis: &Nemesis)` (170–176);
     - `/// Same as \`interpolate\` but accepts an explicit \`now\`...` +
       `pub fn interpolate_at(&self, nemesis: &Nemesis, now: Instant)`
       (178–227), including the `match` on `interp_snapshots[0..2]`, the
       `elapsed_ms` `Vec` collection, the stale guard, and the lerp.
  3. Leave the closing `}` of `impl RemoteInterp` (228) and everything before
     line 170 (`new`, `push`, `set_anchor`, `interpolate_at_tick`) untouched.
  4. Do both deletions in the **same edit/commit**. Removing the methods while
     the legacy tests still call `interpolate_at` leaves the crate uncompilable;
     there is no intentional RED window here. (Bottom-up ordering just keeps the
     line anchors correct while editing; it is not a commitment boundary.)
  5. In `client/src/render.rs:70`, change `RemoteInterp::interpolate` to
     `RemoteInterp::interpolate_at_tick` in the `draw_nemesis_at` doc comment.
     No code change.
  6. Do not modify `client/src/world/game.rs`, `client/src/network/world.rs`, or
     any other file.

- **Acceptance**:
  1. `cargo test -p metroidvania-rust` green.
  2. The client lib test list contains the four `interpolate_at_tick_*` tests,
     `push_evicts_old_snapshots`, and none of the six deleted names:

     ```bash
     cargo test -p metroidvania-rust --lib -- --list \
       | grep -c 'network::interpolation::tests::interpolate_at_tick_'
     # → 4
     ```

     ```bash
     cargo test -p metroidvania-rust --lib -- --list \
       | grep -E 'tests::interpolate_(empty|single_snapshot|two_snapshots|clamps|stale_guard|evicts)'
     # → no output
     ```

  3. `cargo test --workspace` green at `baseline - 6` (no failures; server,
     protocol, game, and e2e untouched).
  4. `cargo fmt --check --all` clean.
  5. `cargo clippy --all-targets --all-features --workspace -- -D warnings`
     clean. Nothing should become unused: `Instant` is still used by the
     struct / `new` / `push` / `set_anchor` / `interpolate_at_tick`, and
     `Duration` + `Instant` are still used by K1–K4 and R1.
  6. Grep acceptance — anchor on `(`, and use word boundaries so
     `interpolate_at_tick` is not a false positive:

     ```bash
     grep -nE 'fn interpolate\(|fn interpolate_at\(|\.interpolate\(|\.interpolate_at\(' \
       client/src/network/interpolation.rs
     # → no output
     ```

     ```bash
     grep -nwE 'interpolate|interpolate_at' client/src/network/interpolation.rs
     # → no output (-w means interpolate_at_tick never matches)
     ```

     ```bash
     grep -RInE '\.interpolate(_at)?\(' client server protocol game e2e
     # → no output (no remaining caller anywhere in the workspace)
     ```

     ```bash
     grep -n 'interpolate_at_tick' client/src/network/interpolation.rs \
       client/src/world/game.rs client/src/render.rs
     # → present in all three (definition, sole caller, doc comment)
     ```

     Prose may still say "interpolation"/"Interpolate" (e.g. the
     `interpolate_at_tick` doc comment); only the removed *method names* must be
     gone. `render.rs` now names `interpolate_at_tick`, so it also passes the
     `\.interpolate(_at)?\(` grep.

- **Test**: K1–K4 and R1 must pass; D1–D6 must be gone. No new tests. (This box's
  "test work" is deletion, per the single-box pipeline.)

- **Explain**
  - What: Deleted `RemoteInterp::interpolate` and `RemoteInterp::interpolate_at`
    (with their doc comments) and the six tests that only exercised the
    wall-clock path, plus the `Group D` divider; renamed the seventh to
    `push_evicts_old_snapshots` under a new `// Push / ring-buffer tests` header;
    fixed the stale `render.rs` doc comment. `interpolate_at_tick` and its four
    tests are byte-for-byte unchanged.
  - Why: two interpolation paths mean two sets of semantics (wall-clock
    arrival-time lerp vs. tick-based target). Production already chose the tick
    path, so the other was dead weight that could drift or be mistakenly used.
  - Rust concepts: inherent-method deletion from an `impl` block; why an unused
    `pub` method is *not* flagged by `dead_code` (it is reachable through a
    public module path, so the compiler assumes external use) and must be removed
    deliberately; `Instant`/`Duration` ownership staying with the surviving API;
    `#[cfg(test)] mod tests` as an ordinary child module that must be edited in
    the same commit as the API it calls.
  - Watch out: the deletion is atomic (no compiling intermediate state), and the
    eviction test was *kept* (renamed) by owner decision rather than deleted.

- **Watch out (implementation hints, not the Explain field)**:
  - `interpolate` only delegated to `interpolate_at`; deleting `interpolate_at`
    makes `interpolate` uncompilable, so neither can be left behind.
  - Keep `set_anchor` — it is called by `client/src/network/world.rs` after each
    `PlayerPos` and is unrelated to the legacy pair.
  - Do not "helpfully" delete the tick-based section divider or the
    `// New tests for interpolate_at_tick` header.
  - The `render.rs:70` comment was fixed (owner-approved); it is no longer a
    follow-up.

## Definition of Done

- [x] `cargo test --workspace` green; test count is exactly `baseline - 6`
      (241 passed / 3 ignored).
- [x] `cargo fmt --check --all` clean.
- [x] `cargo clippy --all-targets --all-features --workspace -- -D warnings`
      clean.
- [x] `client/src/network/interpolation.rs` contains no `fn interpolate(` /
      `fn interpolate_at(` / `.interpolate(` / `.interpolate_at(` (the
      `interpolate_at_tick` family is untouched and allowed).
- [x] The six legacy tests are gone, the seventh is renamed to
      `push_evicts_old_snapshots`, and the four `interpolate_at_tick_*` tests
      pass unchanged.
- [x] No `.interpolate(` / `.interpolate_at(` callers remain anywhere in the
      workspace.
- [x] The diff touches only `client/src/network/interpolation.rs` (deleting the
      two method definitions, the six legacy tests, the stale `Group D` divider,
      and renaming R1) plus the one-line doc comment in `client/src/render.rs`.
- [x] `RemoteInterp::interpolate_at_tick` is unchanged and remains the only
      interpolation entry point used by `World::draw`.
- [x] One commit (or a small related set) on
      `refactor/single-interpolation-path`:
      `refactor(client): remove legacy wall-clock interpolation`, with the
      `## Explain` block (What / Why / Rust concepts / Watch out) in the body.
- [ ] Owner validates; the agent does **not** merge.

## Ripple surfaces

| Surface | Site | Impact |
| --- | --- | --- |
| `client/src/network/interpolation.rs` | methods 170–227 | deleted |
| `client/src/network/interpolation.rs` tests | D1–D6 (251–415) + `Group D` divider | deleted |
| `client/src/network/interpolation.rs` tests | R1 (420) | renamed to `push_evicts_old_snapshots` |
| `client/src/network/interpolation.rs` tests | K1–K4, `make_nemesis`, `use` lines | unchanged |
| `client/src/world/game.rs` | `World::draw` (~246), `other_interp` field | unchanged — already `interpolate_at_tick` |
| `client/src/network/world.rs` | `push` / `set_anchor` calls (~234–236) | unchanged |
| `client/src/render.rs` | line 70 doc comment | **changed** — now names `interpolate_at_tick` (doc only) |
| `client/src/lib.rs`, `client/src/network/mod.rs` | module wiring | unchanged (no re-export existed) |
| `protocol/`, `server/`, `game/`, `e2e/` | — | untouched |

## Open questions

Resolved during planning (revised at implementation — see the implementation note):

1. **`interpolate_evicts_old_snapshots` does not call the deleted API** — it
   only exercises the surviving `push`. Draft decision was to delete it anyway;
   the owner reversed that at implementation time: **keep it, renamed to
   `push_evicts_old_snapshots`**. It is the only direct test of the 5-entry ring
   eviction and timestamp shift, so coverage is preserved.
2. **`client/src/render.rs:70` said `RemoteInterp::interpolate`** — the draft
   deferred this; the owner approved fixing it in this box (doc comment only),
   so no stale API name remains anywhere in the workspace.

Deferred (do **not** change here):

- `push` still writes `last_known_server_tick = nemesis.interp_len as u64` with
  a `// placeholder, will be set by caller` comment, immediately overwritten by
  `set_anchor`. Left as-is; a future cleanup, not this spec.
- `World::draw` passes `interp.last_known_server_tick` /
  `interp.last_known_arrival` even though they are fields of `interp`. The
  signature is intentional (testability); do not "simplify" it.
