# Contributing / Workflow

Full-ceremony process for AI-assisted development of Rustvania.

## Branching

- `main` — stable.
- `dev` — integration.
- Feature work on short-lived branches: **one branch per spec**, named
  `<type>/<spec-slug>` (e.g. `refactor/phase-0-foundation`), branched from `dev`.

## Feature loop (pipeline)

The `orchestrator` primary agent runs this for each feature:

1. **Spec** — `spec-designer` writes `docs/specs/<feature>.md`.
2. **Plan** — `planner` writes `docs/plans/<feature>.md` (checkable boxes,
   public API signatures, test cases).
3. **Plan review** — `reviewer` (plan mode) checks completeness/feasibility;
   send the plan back to `planner` if it has real issues.
4. **Test first** — `tester` writes failing tests from the plan's test cases.
5. **Implement** — `implementer` makes the tests pass and fills each box's
   `Explain` notes (What/Why/Rust concepts/Watch out).
6. **Code review** — `reviewer` (code mode) reviews the diff; the owner reviews too.
7. **QA** — `qa` audits process health, Rust learning value, and game quality
   (advisory; escalate blocker/major findings to the owner).
8. **ADR** — record a decision if one emerged.
9. **Commit & PR** — commit in small steps with the `## Explain` block in each
   commit body (explanation commits are the norm); push the branch and open a
   pull request to `dev`. CI must pass.
10. **Merge** — only after the owner validates. The agent never merges on its own.

Small fixes are done directly by the `orchestrator`.

## Commits

Conventional commits with a scope: `type(scope): subject`, e.g. `feat(protocol)`,
`test(client)`, `chore(ci)`, `docs`.

- Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`.
- Scopes: `protocol`, `server`, `client`, `workspace`, `docs`, `ci`, `agents`.

## Quality gates

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

## Git hooks

The repo ships hooks in `.githooks/` (`pre-commit`: fmt/clippy/test; `commit-msg`:
Conventional Commits). They are not installed by cloning, so enable them once:

    git config core.hooksPath .githooks

and set the commit message template:

    git config commit.template .gitmessage

Both commands are idempotent and safe to re-run. (`git commit --no-verify`
bypasses the `pre-commit` and `commit-msg` hooks if you need to.)

## Explain rule

Every change includes a `## Explain` block (What / Why / Rust concepts / Watch out).
