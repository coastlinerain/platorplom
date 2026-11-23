# Contributing / Workflow

Full-ceremony process for AI-assisted development of Rustvania.

## Branching

- `main` — stable.
- `dev` — integration.
- Feature work on short-lived branches.

## Feature loop (pipeline)

The `orchestrator` primary agent runs this for each feature:

1. **Spec** — `spec-designer` writes `docs/specs/<feature>.md`.
2. **Plan** — `planner` writes `docs/plans/<feature>.md` (checkable boxes,
   public API signatures, test cases).
3. **Test first** — `tester` writes failing tests from the plan's test cases.
4. **Implement** — `implementer` makes the tests pass and fills each box's
   `Explain` notes (What/Why/Rust concepts/Watch out).
5. **Review** — `reviewer` reviews the diff; the owner reviews too.
6. **ADR** — record a decision if one emerged.
7. **Commit** — conventional commit; pre-commit hooks + CI must pass.

Small fixes are done directly by the `orchestrator`.

## Commits

Conventional commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`.

## Quality gates

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

## Explain rule

Every change includes a `## Explain` block (What / Why / Rust concepts / Watch out).
