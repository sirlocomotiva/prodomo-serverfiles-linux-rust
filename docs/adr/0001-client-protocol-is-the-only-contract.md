# The Reference client protocol is the only compatibility contract

There is no production deployment and no data to migrate, and the Rewrite replaces the Legacy
server all at once, so nothing but the Reference client ever talks to it. Only the client wire
protocol (every CG and GC record, the framing, and the TEA boundaries) and the legacy Game data file
formats must stay byte-compatible. The SQL schema, the DB-peer protocol, the game-to-game (P2P)
protocol, the process layout, and the configuration are ours to redesign.

## Consequences

- Supersedes owner decision 1 in `docs/STATUS.md` (the auth split), whose stated reason was keeping
  a Rust `db-server` usable by an unmodified C++ game server, and every `AGENTS.md` rule that
  requires DB-peer or GG wire compatibility.
- The Reference client source is not available to the Rewrite. `server/server/game/packet.h` and
  the server's send sites are the only reference for record widths; the owner calibrates them
  against the Reference client in the final play test.
