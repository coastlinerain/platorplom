# Rustvania

A 2D multiplayer metroidvania written in Rust — a learning project for Rust and
for AI-assisted development with OpenCode.

## Structure

| Crate | Path | Role |
| --- | --- | --- |
| `metroidvania-server` | `server/` | UDP game server (laminar + postcard) |
| `metroidvania-rust` | `client/` | Game client (macroquad) |

## Running

### Server

```sh
cd server && cargo run
```

### Client

```sh
cd client && cargo run
```

## Status

Early-stage. See [docs/progress.md](docs/progress.md) for current status and
[docs/adr/](docs/adr/) for architectural decisions.

## Documentation

- [Development plan](PLAN.md)
- [Contributing / workflow](CONTRIBUTING.md)
- [Progress tracker](docs/progress.md)
- [Rust lessons](docs/lessons.md)
