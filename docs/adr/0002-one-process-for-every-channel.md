# One process hosts auth and every Channel

The Rewrite runs as a single process with an auth listener and one listener per Channel. Each
Channel is one world holding all of its maps. Every Channel is stepped on one game thread at 25
Pulses per second; network and database work run on async tasks that feed that thread through
queues, and Channels talk to each other only over an in-process message bus (whisper, shout,
messenger, guild). With no C++ peer to interoperate with (ADR-0001), the legacy DB-server process
and the per-Core P2P mesh would be pure overhead, and one game thread keeps the Legacy server's
single-threaded ordering.

## Considered options

- The legacy layout (DB server, auth, and several Cores per Channel) with the legacy DB-peer and P2P
  protocols. This was the direction before this decision.
- The legacy layout with redesigned internal protocols.

## Consequences

- The Shared Channel (legacy channel 99, `GUILD_WARP_WAR_CHANNEL`) is one more world in the same
  process, and each Channel's map set is configuration, like the union of its Cores' `MAP_ALLOW`
  lists in `legacy/config/ch*/core*/CONFIG`. The legacy map sets are kept, so the 8 maps only
  Channel 1 hosts stay unreachable from Channels 2-4.
- Leaving a Shared Channel map returns the player to the Channel they came from. Legacy sends them
  to Channel 1, because channel 99 learns only the Channel 1 and 99 map locations
  (`db/ClientManager.cpp:1298-1400`). This is a Divergence.
- A crash takes down every Channel, and the server cannot spread across machines. That is
  acceptable for a hobby server.
- Moving to one thread per Channel later is cheap, because Channels already talk only over the bus.
- Retires the DB-peer framing, peer policy, boot stream, `DbLink`, the setup and map-location
  exchange, the MySQL adapters, and the 32 missing GG records. The verified pure-logic rule modules
  (GM list rules, item-ID ranges, event, shop, item_attr, and banword parsing) move into the new
  process first. The retirement is recorded as its own ledger section.
- The account lookup, login-key registry, and character loading live in the same process as the
  Channels, which makes the owner-decision-1 split moot.

## Amendments

- **2026-10-14 (owner).** The deployment runs Channel 1 and the Shared Channel; Channels 2-4 are
  off and stay in the example configuration as commented tables.
- **2026-11-15 (direction review, ledger 679).** What was built differs from the wording above in
  two ways, and the wording gives way to the code:
  - Every Channel's maps, characters and timers live in one `GameState`, keyed by
    `(Channel, map)`, and one `CharacterManager` holds every character. A Channel is a key, not a
    world of its own.
  - The "bus" is `ChannelClients`, one registry of every connected client behind a mutex, which
    the game thread calls directly (the shout, a notice, a cross-Channel whisper, a close order).
    There is no message type between Channels.

  So the consequence "moving to one thread per Channel later is cheap" is withdrawn: it would
  now mean splitting `GameState`. One game thread is the design, not a first stage.
- **The Pulse budget is 40 ms and must be shown, not assumed.** Legacy runs one event queue; the
  Rewrite runs about 45 `run_*` passes every Pulse (`GameState::process_pulse`), each over its
  own collection. Measured on the release build at ledger 679 with Channel 1 and the Shared
  Channel (65,604 NPCs and monsters standing, no player): the game thread used 0.05 ms per Pulse
  and the process held 595 MB. No measurement exists with players in the world, where monsters
  wake, and the loop records no Pulse duration. Before the play test the loop records each
  Pulse's duration and logs an overrun, and a scripted-client load run shows the 99th percentile
  under 20 ms; if it does not, the per-Pulse passes move onto one due-time queue, as regen's
  already are.
- **A stalled game thread stops every Channel.** A panic on it is caught and ends the process
  with an error (`game_loop::run_guarded`); a hang is not caught. The known ways to hang it are
  a quest script that never returns (ADR-0006) and a 40 ms overrun that never recovers (the
  planner replays up to 750 missed Pulses). Each client's outbox is unbounded on purpose, so the
  game thread never waits for a slow client; a client that stops reading is closed once its
  outbox passes a cap, which does not exist yet.
