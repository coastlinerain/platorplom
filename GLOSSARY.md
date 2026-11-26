# GLOSSARY

Domain vocabulary for Rustvania. Terms only — no implementation details, no
config constants. When a term is resolved, capture it here; challenge fuzzy
language against this file (see `AGENTS.md`).

## Time & simulation

- **server tick** — one authoritative simulation step on the server (fixed rate).
- **client tick** — one client simulation step (fixed rate, driven by the
  accumulator in the render loop).
- **frame** — one rendered image (variable rate, driven by the display).

## Netcode

- **authoritative server** — the server is the single source of truth for game
  state; clients never dictate state, only send input.
- **prediction** — the client simulates its own player immediately from local
  input, ahead of server confirmation, to hide round-trip latency.
- **reconciliation** — the client corrects its predicted state when a
  server-authoritative snapshot arrives, replaying un-acked inputs.
- **interpolation** — rendering a remote player between its two most recent
  server snapshots to smooth out the fixed-tick cadence.
