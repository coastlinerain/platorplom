# platorplom (Rustvania) — Development Plan

> Stable roadmap for turning this learning project into a serious, AI-assisted
> codebase. The **working memory** lives in [`docs/progress.md`](docs/progress.md);
> this file is the higher-level plan.

## Context

- 2D multiplayer metroidvania in Rust (server + client).
- Goal: learn Rust **deeply** and master AI-assisted development with OpenCode.
- Working style: **AI writes, I review and learn.** Full-ceremony process.

## Strategy

Spec-Driven Development + Plan Mode, with TDD/BDD layered on pure logic,
ADRs for decisions, and an "explain as you go" rule so every change is a lesson.

| Layer | Purpose |
| --- | --- |
| `AGENTS.md` | Always-loaded project memory & rules |
| `docs/specs/` | Feature specs (behavior + edge cases) |
| `docs/adr/` | Architecture decision records |
| `docs/progress.md` | Working memory / status tracker |
| `docs/lessons.md` | Rust learning journal |
| Tests | Unit tests for pure logic; BDD for player-visible features |

## Phases

### Phase 0 — Workflow foundation
1. `AGENTS.md` (the memory system).
2. Cargo workspace + shared `protocol/` crate.
3. `rustfmt.toml`, clippy lints, conventional commits, pre-commit hooks.
4. CI (GitHub Actions): `fmt`, `clippy`, `test`.
5. Docs skeleton (this file, specs, ADRs, README, progress).

### Phase 1 — Hardening (refactor before features)
1. Extract a shared `protocol/` crate; fix the protocol drift bugs.
2. Protocol round-trip tests (postcard serialize → deserialize).
3. Unit tests for `maps` parsing and `level` collision.
4. Clean dead code and empty branches.
5. ADR-001 networking model → netcode hardening follow-up.

### Phase 2 — Feature loop (per feature)
1. Write the spec.
2. BDD acceptance scenarios (cucumber).
3. TDD failing tests for the logic.
4. Plan-mode approval.
5. Implement + `## Explain` block.
6. Review + ADR (if a decision emerged).
7. Conventional commit + CI green.

### Phase 3 — Continuous learning
- Maintain `docs/lessons.md`.
- Periodic whole-codebase idiomatic-Rust review.

## First actions

The live, ordered list (with status) is in [`docs/progress.md`](docs/progress.md).
