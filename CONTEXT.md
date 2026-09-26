# Prodomo server rewrite

A Linux-native Rust rewrite of the Prodomo Metin2 game and database servers. It exists so that the
unmodified Prodomo 5.4 client can be pointed at the new server and play exactly as it does against
the legacy server.

## Scope

**Legacy server**:
The frozen C++ Prodomo server source under `server/server`, taken as functional. It is the
behavioural reference and is not modified.
_Avoid_: old server, C++ server (when meaning the reference), original

**Reference client**:
The Prodomo 5.4 Windows client built for the Legacy server. It is the only external consumer the
Rewrite must satisfy, and it is not modified.
_Avoid_: the client (unqualified), official client, vanilla client

**Rewrite**:
The Rust servers in this workspace that replace the Legacy server all at once, with a fresh store
and no mixed deployment.
_Avoid_: port, new server, Rust server (when meaning the whole system)

## Parity

**Parity**:
The Rewrite behaves as the Legacy server does in every way the Reference client can observe, apart
from Defects. Internal mechanisms (storage, process layout, inter-server messages) are not part of
Parity.
_Avoid_: 1-1, compatibility, bug-for-bug

**Quirk**:
Legacy behaviour that an unmodified Reference client can reach in normal play and that the Rewrite
reproduces exactly, even when it looks odd.
_Avoid_: bug (when meaning a Quirk), feature

**Defect**:
Legacy behaviour the Rewrite never reproduces: crashes, memory corruption, data loss, item
duplication, injection, and anything reachable only by a modified client. A bug an honest client
can trigger is a Defect unless the owner names it a Quirk.
_Avoid_: legacy bug, parity bug

**Divergence**:
A deliberate, recorded difference between Rewrite and Legacy server behaviour, most often a fixed
Defect.
_Avoid_: deviation, fix (when unrecorded)

**Parity inventory**:
The list of every Legacy server system and handler, taken from the source, each with its porting
status. It is what "everything" means.
_Avoid_: checklist, TODO list, feature list

**Game data**:
The owner's legacy data set (protos, maps, quests, locale strings, drop and shop tables, and the
tables legacy keeps in SQL), taken unchanged in its legacy formats.
_Avoid_: share, assets, content, fixtures (fixtures are synthetic test stand-ins)

## World

**Channel**:
One of several independent, numbered copies of a configured set of maps that a player picks between
at login; players on different Channels cannot see each other. Channels need not host the same maps.
_Avoid_: server, realm, shard

**Shared Channel**:
Channel 99, which hosts maps that exist once for everyone (dungeons, event and war maps). Players
from every Channel reach them by Warp and meet there, and leave back to the Channel they came from;
it is never picked at login.
_Avoid_: ch99, dungeon channel, war channel

**Core**:
A Legacy server game process that hosts some of one Channel's maps. The Rewrite has no Cores; a
Channel is never split.
_Avoid_: using it to mean Channel

**Warp**:
Moving a character to another map or Channel in a way that makes the Reference client reconnect
and show a loading screen.
_Avoid_: teleport (a same-map move with no reconnect)

**Pulse**:
One step of the game loop; there are 25 per second, and timers, cooldowns, and regeneration are
counted in Pulses.
_Avoid_: tick, pass, frame

**Locale**:
The service region setting that selects name rules, the text code page, and Game data paths.
Prodomo runs the `europe` Locale; no other Locale is ported.
_Avoid_: language (a per-player choice within the Locale), country

## Players and items

**Name**:
A character's name: ASCII letters and digits only, and unique regardless of case.
_Avoid_: nickname, login (the account's credential)

**Free text**:
Anything a player types that is not a Name, such as chat, whispers, shop signs, guild notices, and
offline messages. It is kept and relayed as the exact bytes the client sent.
_Avoid_: string, message (when meaning the stored form)

**Transfer**:
Any exchange that moves items or gold between two owners, such as a trade, a shop sale, the
safebox, the item shop, or an offline message. It happens completely or not at all.
_Avoid_: exchange (the trade window specifically), transaction

**Operator**:
The person who runs the Rewrite and manages accounts, GMs, and item-shop currency from outside the
game.
_Avoid_: admin, GM (an in-game role)
