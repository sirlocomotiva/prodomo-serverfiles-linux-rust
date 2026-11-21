# First-Session Play-Test Script

This script is the owner's checklist for running the unmodified Prodomo 5.4 Reference client
against the Rust `prodomo` server. Every Parity inventory row is closed under the scripted client
(`crates/prodomo/tests/parity.rs`), and Packages A–C are done; this script verifies the remaining
client-only behaviours and establishes baseline parity with the real client binary.

## 0. Start the server and prepare test accounts

Follow [`README.md`](../README.md) ("With Compose" or "From the source tree"):

1. Set `[network].public_ip` in `prodomo.toml` to the IP the Reference client connects to.
2. Start PostgreSQL 18 and `prodomo` (`docker compose up -d --build`, `podman compose up -d --build`, or `cargo run --release -p prodomo -- --config prodomo.toml serve`).
   - Note: `docker-compose.yml` (formerly `compose.yaml`) was verified on Podman 5.4.2; a native Docker daemon run has not been executed on the development host. With Podman, never use `podman compose restart` (use `stop` then `up -d`).
3. Create two accounts and grant one character GM authority (`test_server` ships `false`, ADR-0007):
   ```bash
   prodomo account create alice --delete-code 1234567
   prodomo account create bob   --delete-code 1234567
   prodomo gm grant alice Admin implementor
   ```
4. Configure the client's `serverinfo.py` with `<public_ip>`:
   - Auth: `30001`
   - Channel 1: `30003` (mark server `MARKADDR`: `30003`; `30005` also accepts Channel 1 clients)
   - Shared Channel (`30019`): do not add to `CHANNEL_DICT`; `GC_WARP` directs the client there.
5. Tail the server log in a second terminal (`docker compose logs -f prodomo` or `tail -f log/*.log`).

---

## 1. Priority checks (Owner Questions 7 and 27)

Run these first; they test behaviours only the Reference client can confirm.

### 1.1 Question 7 — `GC_WARP` port reconnect and `CG_WARP`

- [ ] **Same-Channel cross-Core warp:** Log in on Channel 1 (`30003`). Walk into a map-1 warp NPC whose legacy target lived on Core 2 (for example `metin2_map_a1` -> `metin2_map_a3`).
  - Expected: `GC_WARP` names `<public_ip>:30003` (the port the client is already using on Channel 1), the client reconnects cleanly, loads the target map, and enters without a disconnect or stuck loading bar.
- [ ] **Channel 1 -> Shared Channel warp:** Warp to a Shared Channel map (for example `/warp` to map 103, 110, 111, or 113, or via NPC `20011` during OX).
  - Expected: `GC_WARP` names `<public_ip>:30019`, the client reconnects to `30019` and enters cleanly.
- [ ] **`CG_WARP` observation:** Check the server log during both warps to record whether the Reference client sends `CG_WARP` (`0x41`) and on which connection. The server consumes `CG_WARP` in `Game` without closing; confirm no `Closing session on refused client frame` line appears.
- [ ] **Return from Shared Channel:** Leave the Shared Channel map (exit portal or `/go`).
  - Expected: returns to the origin Channel (Channel 1, `<public_ip>:30003`, recorded Divergence).

### 1.2 Question 27 — Shared Channel routes and `event_panel`

- [ ] **OX event (map 113):** As `Admin`, enable `oxevent_status = 1` (or open via GM event controls), then talk to Uriel (`20011`) as a normal player (`bob`) and enter map 113. Verify quiz/exit flow.
- [ ] **Guild war maps (110, 111):** Declare a guild battle/flag war and enter via NPCs `11001`, `11003`, or `11005`.
- [ ] **Kingdom / siege war (map 103):** Trigger the kingdom/siege war button and verify transfer to map 103.
- [ ] **`event_panel` access check:** As a non-GM character (`bob`), inspect whether the client exposes `event_panel` and whether an ordinary player can toggle `oxevent_status`. Record the observation for owner question 27.

---

## 2. Core session walkthrough

### 2.1 Auth, select screen, and character lifecycle
- [ ] Channel status indicator (`CG_STATE_CHECKER` on `30003`) shows Channel 1 online.
- [ ] Wrong password shows the client's invalid-credentials message; correct password reaches empire select (first login) or character select.
- [ ] Choose empire, create `Admin` on `alice` and a mortal character on `bob`. Verify character list, appearance, stats, and level render with no shifted fields (confirms `GC_LOGIN_SUCCESS` / `GC_PLAYER_CREATE_SUCCESS` wire widths).
- [ ] Test deleting a throwaway character with wrong delete code (refused) and right delete code `1234567` (deleted).

### 2.2 Entering the world, movement, and chat
- [ ] Enter world on both clients in the same empire/map. Both characters see each other, NPCs, and monsters.
- [ ] Walk, run, attack animation, and sync position; confirm no rubber-banding or entity desync.
- [ ] Test normal chat, shout (`!`), whisper between `alice` and `bob`, and quickslot add/swap/delete.

### 2.3 Combat, skills, death, and restart
- [ ] Pull and kill map-1 monsters; verify HP bar updates, damage numbers, XP gain, gold/item drops, and pickup.
- [ ] Learn a skill group, raise a skill, cast an active buff and attack skill; verify animations, SP cost, cooldown, and affect icons.
- [ ] Let a character die (or `/kill`): confirm `/restart_here` enforces the 9-second wait (shipped `test_server = false`) and `/restart_town` returns to town without XP loss.

### 2.4 Items, equipment, economy, and storage
- [ ] Equip, swap, and unequip weapon/armour/accessories; confirm visual model updates (`GC_CHARACTER_UPDATE`) and stat window updates (`GC_POINT_CHANGE`).
- [ ] Drop an item to the ground and pick it up on the second client after ownership expires (or own pickup).
- [ ] Buy from and sell to an NPC shop.
- [ ] Execute a 2-client direct trade (items + Yang) and confirm both inventories and Yang update atomically.
- [ ] Open a private shop on `bob` and buy from `alice`.
- [ ] Open safebox, check in and check out an item, and test smith refine (`+0` -> `+1`).

### 2.5 Quests, party, guild, and GM permissions
- [ ] Open a quest letter and an NPC quest dialog; advance through `say`/`select`/`confirm`.
- [ ] Form a party between `alice` and `bob`, confirm shared XP and party UI updates.
- [ ] Create a guild, upload a guild mark (`MARKADDR`), and confirm the mark displays on both clients after relog.
- [ ] Confirm `Admin` (granted `implementor`) can run `/item`, `/level`, `/m`, `/warp`, while `bob` (no grant) is refused as `GM_PLAYER`.

### 2.6 Persistence and clean shutdown
- [ ] Log out cleanly on both clients, relog, and verify position, level, Yang, inventory, equipment, quickslots, and skills persisted.
- [ ] Stop the server with `SIGTERM` / `podman compose stop` and confirm exit code `0` after the shutdown save log lines.

---

## 3. Reading disconnect and refused-frame logs

Whenever `prodomo` closes a client connection due to an unknown header, framing error, wrong-phase
packet, or malformed payload, it logs a structured `WARN` line containing the descriptor's phase,
hex header, length, and up to 64 bytes of the wire frame:

- Framing / unknown header error:
  ```text
  WARN prodomo: Client session stopped on frame error addr=... phase=Handshake header=0xfa buffered=4 bytes=fa 11 22 33 error=...
  ```
- Wrong-phase or malformed decoded record:
  ```text
  WARN prodomo: Closing session on refused client frame addr=... phase=Login header=0x10 len=4 bytes=10 02 01 07
  ```

Credential-bearing headers (`0x01` `CG_LOGIN`, `0x05` `CG_CHARACTER_DELETE`, `0x6d` `CG_LOGIN2`,
`0x6f` `CG_LOGIN3`, `0x90` `CG_INVENTORY_PROTECTED`) always redact their payload bytes as
`<redacted payload, N bytes>`.

For any unexpected disconnect or visual glitch during the play test, record:
1. The `WARN` line(s) from the server log (`phase`, `header`, `len`, `bytes`).
2. The action performed in the client right before the disconnect.
3. The matching lines from the client's `syserr.txt`.

Each confirmed issue becomes its own ledger section; any Parity row marked `unused` that the
Reference client actually reaches is reopened and ported.
