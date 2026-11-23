# AGENTS.md — platorplom (Rustvania)

Project guidance and memory. Read this first, and keep it updated. This file is
auto-loaded by OpenCode, so it is the single source of truth for how to work here.

## What this is

A 2D multiplayer metroidvania in Rust, built to learn Rust and to learn
AI-assisted development with OpenCode. GPL-3.0.

Layout:
- `server/` — `metroidvania-server`: UDP networking (laminar) + postcard serialization.
- `client/` — `metroidvania-rust`: macroquad rendering + the same networking stack.
- `docs/` — specs, ADRs, progress, lessons (see pointer map below).

## Team & pipeline

The `orchestrator` (the default primary agent) manages a team of specialized
subagents. For a **new feature** it runs them in order, waiting for each to
finish before the next:

1. `spec-designer` → writes `docs/specs/<feature>.md`
2. `planner` → writes `docs/plans/<feature>.md` (checkable boxes, public API
   signatures, test cases)
3. `reviewer` (plan mode) → verifies the plan is complete and feasible; send it
   back to `planner` if not
4. `tester` → writes the failing tests from the plan's test cases
5. `implementer` → implements to pass the tests, fills each box's Explain notes
6. `reviewer` (code mode) → reviews the diff for correctness and regressions
7. `qa` → final advisory pass on process health, Rust learning value, and game quality

Small fixes are done directly by the `orchestrator`.

## Workflow rules (apply every time)

1. **Feature vs fix.** A new feature goes through the pipeline above; a small
   fix is done directly. When in doubt, use the pipeline.
2. **Test the logic.** Pure logic (protocol, maps, collision) gets unit tests.
   Rendering/input loops do not. BDD (cucumber) for player-visible behavior.
3. **Record decisions.** Non-obvious choices go in `docs/adr/` as an ADR.
4. **Explain every change.** Every change ships with an `## Explain` block
   (see below). The owner is learning Rust — do not skip this.
5. **Update memory.** At the end of every task, update `docs/progress.md`
   (status + next actions) and, if a new Rust concept appeared, `docs/lessons.md`.
6. **Branch per feature, merge on validation.** Work happens on a branch named
   `<type>/<spec-slug>` branched from `dev`; commits carry the `## Explain` block
   in their body; nothing is merged until the owner validates.

## The Explain block

For every code change, include in the commit body or review summary:

```text
## What
<what changed>

## Why
<reason>

## Rust concepts
<which Rust features are used — e.g. ownership, borrows, lifetimes,
 traits, generics, Result/Option, pattern matching, channels, derive>

## Watch out
<gotchas, trade-offs, or things to verify>
```

Keep it short but specific. If the owner asks "why did you do it this way?",
the Explain block should already answer it.

## Code conventions

- Identifiers, comments, and docs in **English** (migrating away from the
  existing Spanish comments/prints).
- `cargo fmt` clean; `clippy` with no warnings.
- Prefer `Result`/`Option` and `?` over `unwrap`/`expect` in non-example code.
- Conventional commits: `type(scope): subject`, e.g. `feat(protocol)`, `test(client)`,
  `chore(ci)`, `docs`. Scopes: `protocol`, `server`, `client`, `workspace`, `docs`,
  `ci`, `agents`.
- No magic numbers where a named constant (`config.rs`) already belongs.

## Testing

- `cargo test` must pass in every crate.
- Protocol: round-trip serialize/deserialize tests.
- Maps: parsing tests. Collision: `solid_in_rect`, `grounded`, `colision` tests.

## Memory / pointer map

- `docs/progress.md` — **live status tracker** (update after every task).
- `docs/lessons.md` — Rust concepts learned (update as you go).
- `docs/specs/` — feature specs. `docs/adr/` — decision records.
- `CONTRIBUTING.md` — the full ceremony (commits, review, CI).
- `PLAN.md` — the stable roadmap.

## Team

- `orchestrator` (primary) — decides feature vs fix and runs the pipeline.
- `spec-designer` — writes feature specs.
- `planner` — turns a spec into a checkable implementation plan.
- `tester` — writes and runs tests for pure logic.
- `implementer` — implements a plan, filling per-box Explain notes.
- `reviewer` — reviews plans (completeness/feasibility) and code
  (correctness/regressions); read-only + read-only git.
- `qa` — audits process health, Rust learning value, and game quality
  (advisory, read-only).
- `explainer` — explains code/decisions without editing (learning aid).
