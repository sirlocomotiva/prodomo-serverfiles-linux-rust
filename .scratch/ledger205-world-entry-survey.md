# Ledger 205 survey: how a live client reaches the game thread's world

Read-only survey, 2026-09-27. No source under `server/` or `legacy/` was modified.

## The question

Ledgers 201-204 put a `GameState` on a dedicated game thread and made a grant
cross into it. What is still missing is the other direction: `serve` never puts a
**live client's character** into that world. A grant can be issued to a character
that only a test created. No operator and no client can reach one.

## What `serve` does today

`prodomo/src/main.rs`, at the end of the `enter_game` handler:

- `held.avatar` is built from the store row.
- `held.save` is armed.
- `context.clients.join(ClientEntry { channel, map, name, vid })` takes a lease in
  the in-process `ChannelClients` registry.
- `context.positions.track(lease.id(), Player, vid, x, y)` records a position.

The code says so itself, twice:

> `DESC::SetPlayer` also puts the character in the world, which is what a
> sync-position claim later looks it up in. The table stands in for the world.

and, on disconnect:

> Releasing the lease takes the client out of the Channel set, but the position
> table stands in for the world, so it has to be told as well.

So the client set and the position table are **stand-ins for the world**. They
were written before the world existed on a thread. The world now exists and
nothing uses it for a real client.

## What legacy does

`server/server/game/input_db.cpp`, `PlayerLoad`:

```cpp
LPCHARACTER ch = CHARACTER_MANAGER::instance().CreateCharacter(pTab->name, pTab->id);
ch->BindDesc(d);
ch->SetPlayerProto(pTab);
ch->SetEmpire(d->GetEmpire());
d->BindCharacter(ch);
```

`CHARACTER_MANAGER::CreateCharacter` (`char_manager.cpp`):

```cpp
DWORD dwVID = AllocVID();
ch->Create(name, dwVID, dwPID ? true : false);
m_map_pkChrByVID.insert(std::make_pair(dwVID, ch));
if (dwPID) { m_map_pkPCChr.insert(...); m_map_pkChrByPID.insert(...); }
```

`CHARACTER::Create` (`char.cpp`):

```cpp
static int s_crc = 172814;
snprintf(crc_string, sizeof(crc_string), "%s%p%d", c_pszName, this, ++s_crc);
m_vid = VID(vid, GetCRC32(crc_string, strlen(crc_string)));
```

## The VID finding, verified

`vid.h` is a two-field class:

```cpp
class VID {
    DWORD m_id;    // the allocated counter value
    DWORD m_crc;   // CRC32 of name + this-pointer + a per-process counter
    operator DWORD() const { return m_id; }
};
```

Only `m_id` is ever put on the wire, because the conversion to `DWORD` returns it.
`m_crc` is a **server-local** anti-alias: two characters in the same process
share an `m_id` only if a stale reference outlived the object it named, and the
CRC tells the two apart. It is not a client-visible id and must not be ported as
one.

The consequence: **a player's wire VID is a process-lifetime counter from
`CHARACTER_MANAGER::AllocVID()` (`++m_iVIDCount`), not the store's `player.id`.**
The store id is the PID, which is a separate field (`m_dwPlayerID`, set by
`SetPlayerProto`).

Verification, with controls:

- Positive control: `CreateCharacter(pTab->name, pTab->id)` at `input_db.cpp`
  shows the store id arriving as the **third** parameter (`dwPID`), and
  `Create`'s second parameter (`vid`) is the counter. The two are distinct.
- Negative control: `SpawnMob` and `SpawnMobRandomPosition` call
  `CreateCharacter(name)` with no PID at all and still get a counter VID, so the
  counter is not a player-id alias.
- Second negative control: `CPrivateShopManager::CreatePrivateShop` and
  `CObject` in `building.cpp` call their own `AllocVID()` counters, so "allocate
  a counter VID" is the general rule, not something special to players.

**The Rewrite currently has this wrong.** `main.rs:character_vid` returns
`character.id`, the store's player id, and the enter-game burst and every
`GC_ENTITY` put that on the wire. `world::CharacterManager::create_player` also
allocates a counter starting at 1, which is right in shape but is a *second*
counter that the live path does not share.

That is a real, already-shipped divergence in a live record (`GC_ENTITY`, header
0x32, at enter-game). It is not a Defect to reproduce and not parity: the client
only requires that VIDs be distinct and stable within a session, and the Rewrite
satisfies that by accident of using a stable store id. It is more useful than a
counter, because it is stable across relog, and that is a deliberate Divergence
to record, not a bug to chase. But it must be **named**, because the world is
about to allocate a second, different VID for the same character, and two VIDs
for one character is a genuine defect.

## What the slice has to settle

1. **One VID per character.** The live descriptor, the world, the client set, and
   the position table must all use the same number. The world must be told the
   store id, not left to invent one.
2. **Join and leave are commands.** A client enters the world when it enters the
   game and leaves when the descriptor ends. Both cross the thread, both answer.
3. **The world must be able to write to a descriptor.** The world has no socket.
   It needs a sender the descriptor drains, which is the same shape as the
   existing `Lease` outbox.
4. **Order.** The character must be in the world before any record can be
   addressed to it, and out of the world before its row is written for the last
   time, or a save can race a destroyed character.
