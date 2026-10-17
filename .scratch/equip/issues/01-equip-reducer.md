# 01. The equip reducer and its wiring (ledger 215)

Status: ready-for-agent

Ledger 214 landed the groundwork (deferrable cell key, `RowChange::Sockets`, `world::character::dice`,
`Equipment::part_change`, `removal_applies`, `refine_element_type`, `Points::compute_battle_points_with`,
`set_part`, `race`, `conqueror_level`, `part_base`). This ticket is the reducer that uses them, and its wiring
into `CG_ITEM_MOVE`. `CG_ITEM_USE` on an equippable item follows in the next ledger. Every flow below was
read from the legacy source (`G/` is `server/server/game/`); re-check a line before citing it.

## Design

- `world/src/character/equip.rs`. `Gear<'a> { points: &mut Points, protos: &ItemProtos, dice: &mut dyn Dice,
  recently_fought: bool }`. `move_item` gains `gear: Option<&mut Gear>`. With `None`, a move onto or off a wear
  cell stays `NotPorted(Unported::Equipment)`, so the existing tests pass `None`.
- `MoveDone.records: Vec<MoveRecord>`, where `MoveRecord` is `Item(ItemRecord)`, `Point(PointRecord)`,
  `Look(CharacterLook)`, `Effect(u8)` or `Notice(&'static str)`. Tests read the item records through a helper
  `item_records(&done)`.
- `CharacterLook` holds the parts, the moving and attack speeds (low byte of `limit_point`), the level, the
  conqueror level and the refine element type. prodomo encodes it as `GcCharacterUpdate::new` (vid,
  `LANGUAGE_EUROPE`).
- Move `WornSystem` and `worn_system_not_ported` from `prodomo/src/item_load.rs` into world, and refuse an
  equip onto a wear cell with `Unported::Worn(system)` **before any change**. `WEARABLE_UNIQUE` is
  `Worn(Unique)`.
- `ItemChange::Sockets{id, sockets}` maps to `RowChange::Sockets`. A worn item's stored position is
  `(Equipment, 180 + w)`; `stored_row_position` already stores it at `(2, w)`.
- prodomo: the world character holds its `Points`; `GameState` owns a `Pcg32` seeded from `RandomState` (a
  Divergence). `Held` records `selected_at` (legacy `m_dwLastSkillTime` is set at creation) and `last_attack`
  (set by an accepted `CG_MOVE` with func 2 or 3, `G/input_main.cpp:1839`). `recently_fought` is either one
  within 1500 ms. Look, Effect and the points records also go to the viewers (`broadcast_excluding`).

## Legacy flows

**CanEquipNow**
1. A job anti-flag → false, with no notice.
2. The limits, in order: LEVEL `[LS;462]`; CHAMPION "Nivelul tau campion este prea mic pentru a putea purta
   acest item!"; STR `[LS;463]`; INT `[LS;464]`; DEX `[LS;465]`; CON `[LS;466]`. All are read with `GetPoint`.
3. `IsSecured` is unreachable.

**CanUnequipNow**
1. A belt while any item is in cells 274..290 → "[1095]You can only discard the belt when there are no longer
   any items in its inventory.".
2. `IRREMOVABLE` → false, silently.
3. No empty cell (`GetEmptyInventoryEx`) → "[1130]There isn't enough space in your inventory.".

**FindEquipCell**
1. `(wearflag == 0 || TOTEM)` and the type is not one of COSTUME, DS, SPECIAL_DS, RING or BELT → -1.
2. DS → 64 + subtype.
3. TALISMAN → 30..35 by subtype.
4. COSTUME: BODY 19, HAIR 20, MOUNT 21, WEAPON 22, SASH 23, AURA 24, PET 28.
5. RING → Ring2 if Ring1 is worn, else Ring1. BELT → 27.
6. The wear flags in order: BODY 0, HEAD 1, FOOTS 2, WRIST 3, WEAPON 4, SHIELD 10, NECK 5, EAR 6, ARROW 9,
   GLOVE 36.
7. UNIQUE → U2 if U1 is worn, else U1. ABILITY → the first free cell of 11..18.

**EquipItem**
1. `!IsEquipable` (COSTUME, ARMOR, WEAPON, ROD, PICK, UNIQUE, DS, SPECIAL_DS, RING, BELT, TALISMAN),
   `!CanEquipNow` or `FindEquipCell < 0` → false.
2. The wedding checks:
   - "Non puoi usare un Costume con uno Smoking o Abito da Sposa." for a costume body onto a wedding armour
     (11901-11904);
   - "Devi rimuovere il Costume per usarlo." for a wedding armour while a costume body is worn.
3. The sex check → `[LS;1005]`.
4. At any cell but the arrow, `recently_fought` → `[LS;451]`.
5. At the weapon cell:
   - a costume weapon that cannot be taken off (for an ITEM_WEAPON, only when its `value3` differs from the
     weapon's subtype) → "You cannot unequip the costume weapon because there is not enough space".
6. A costume weapon onto a missing, non-weapon or mismatched weapon (`item.value3 != weapon.subtype`) →
   "You cannot equip the costume weapon, because you have the wrong weapon equipped".
7. If the cell holds an item that is not `IRREMOVABLE`:
   - a `WEARABLE_ABILITY` item → false;
   - otherwise `SwapItem(item.cell, 180 + cell)`.
8. Otherwise `EquipTo(cell)`.
9. Once the item is worn, the effects:
   - 71135 → 21, 71136 → 22, 71143 → 23, 71145 → 24;
   - otherwise a costume sash gives 26, unless a wedding armour is worn.

**UnequipItem**
1. At the weapon cell, a worn costume weapon that cannot be taken off → the notice above, and false.
2. `!CanUnequipNow` → false.
3. `Unequip`, then `AddToCharacter` at the first empty cell. For a costume sash with socket 0 at 0, that rolls
   `value0`: 2 → 5, 3 → 10, 4 → `number(11, 19)`, anything else → 1. The item set record has highlight 0.
4. Then `CheckMaximumPoints`.

**MoveItem**
- From a wear cell:
  1. `!CanUnequipNow` → false.
  2. At the weapon cell, a costume weapon that cannot be taken off → the notice above.
  3. A destination that is not an empty grid → `return UnequipItem(item)`.
- To a wear cell: an occupied destination → `[LS;1092]`; otherwise `EquipItem(item, dest - 180)`.
- Anything else uses the generic path. A worn item that is moved whole, or merged down to 0, is taken off
  (`Unequip` effects), with no `CheckMaximumPoints` and no sash roll.

**SwapItem**
1. Refuse DS cells, the same cell twice, and two wear cells.
2. The carried item must fit the worn item's grid.
3. For a DS or belt item: `CanUnequipNow` on the worn item and `CanEquipNow` on the carried one.
4. The target must equal `FindEquipCell(carried)`.
5. Take the worn item off, `EquipTo` the carried one, and `AddToCharacter` the worn one at the carried
   item's old cell.
6. Legacy removes the worn item before its later checks fail; the Rewrite checks first (a Defect not
   reproduced).

**EquipTo**
1. An occupied cell → false.
2. Clear the carried cell.
3. Set it at `(INV, 180 + w)` with highlight 0.
4. Run the applies, then the part (with the Look for the costume arms).
5. A set item runs `ComputePoints`; anything else runs the battle points. The Look follows either way.

**Unequip** runs `removal_applies`, then the remove-case part (+Look), clears `(INV, 180 + w)`, and then
runs the set points or the battle points, with the Look.

**Unreachable here, to record:** riding and polymorph, the unique unstack, the NUNTA duplicates, the SIG
effect, REAL_TIME_FIRST_USE, IsRideItem, the mount and pet summon, the aura window, IsSecured,
IsExchanging, the quickslot sync.

## Done when

1. A scripted-client scenario in `prodomo/tests/parity.rs` equips, swaps and unequips.
   - It waits more than 1.5 s after the select before equipping.
   - It checks the item records, the points records, `GC_CHARACTER_UPDATE`, the effect and the stored
     rows.
2. There is a mutation sweep.
3. Every gate passes, with and without `DATABASE_URL`.
4. There is a ledger section, and `sys.item.core` is updated.
