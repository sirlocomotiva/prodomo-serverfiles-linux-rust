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
