//! The GM inspection lines: `/state` and `/inventory` (`ACMD(do_state)`,
//! `ACMD(do_inventory)`, `G/cmd_gm.cpp:1017-1202`, `G/cmd_general.cpp:3210-3250`).
//!
//! Both lines only tell. `/state` names a target — no word is the caller, `#vid`
//! is `CHARACTER_MANAGER::Find(strtoul)` (a player or a standing NPC), a name is
//! `FindByCharacterName` (`strcmp`, case-sensitive) — and dumps its state line,
//! its coordinate line, its pools, its grades, its resistances, its mall and bonus
//! lines, its immunities, and its live privs. `/inventory` lists the caller's own
//! cells from an index for a count, `"<NONE>"` where no item stands.
//!
//! Four legacy shapes survive. The state line's `Shop` and `Exchange` bits read the
//! *caller*, not the target (`ch->GetShop()`, `:1057-1061`). The coordinate line's
//! map tail runs only with a sectree (`pSec`, `:1074-1080`); without one the line
//! ends after the `/100` pair. The priv loop's gate is `GetPriv` (the max over the
//! all-empires row, the target's row, the guild row and the character row) but the
//! empire line tells only the target's own row (`GetPrivByEmpire(tch->GetEmpire())`,
//! `:1191`), so a grant on the all-empires row alone passes the gate and tells
//! nothing. And `POINT_MALL_ITEMBONUS` and `POINT_MALL_GOLDBONUS` are divided by 10
//! for the `ITEMx`/`GOLDx` words (`:1127-1128`).
//!
//! A monster target reads what a monster holds: its name, level, pools and
//! coordinates are real, and every grade, resistance, bonus and immunity reads 0,
//! as its `GetPoint` does (the mob table holds no SP, so `SP` is `0/0`). The port
//! holds no guild or character privs (`priv_guild` is dead, no caller grants a
//! character priv), so the guild and player arms of the priv loop always read 0
//! and only the empire arm tells.

use common::point_slot::{
    POINT_ATTBONUS_ANIMAL, POINT_ATTBONUS_ASSASSIN, POINT_ATTBONUS_BOSS, POINT_ATTBONUS_DESERT,
    POINT_ATTBONUS_DEVIL, POINT_ATTBONUS_FIRE, POINT_ATTBONUS_HUMAN, POINT_ATTBONUS_ICE,
    POINT_ATTBONUS_INSECT, POINT_ATTBONUS_METIN, POINT_ATTBONUS_MILGYO, POINT_ATTBONUS_MONSTER,
    POINT_ATTBONUS_ORC, POINT_ATTBONUS_SHAMAN, POINT_ATTBONUS_SURA, POINT_ATTBONUS_TREE,
    POINT_ATTBONUS_UNDEAD, POINT_ATTBONUS_WARRIOR, POINT_ATT_BONUS, POINT_ATT_GRADE,
    POINT_ATT_SPEED, POINT_BLOCK, POINT_CRITICAL_PCT, POINT_DEF_BONUS, POINT_DEF_GRADE,
    POINT_DODGE, POINT_HP, POINT_IMMUNE_FALL, POINT_IMMUNE_SLOW, POINT_IMMUNE_STUN,
    POINT_MAGIC_ATT_GRADE, POINT_MAGIC_DEF_GRADE, POINT_MALL_ATTBONUS, POINT_MALL_DEFBONUS,
    POINT_MALL_EXPBONUS, POINT_MALL_GOLDBONUS, POINT_MALL_ITEMBONUS, POINT_MAX_HP,
    POINT_NORMAL_HIT_DAMAGE_BONUS, POINT_NORMAL_HIT_DEFEND_BONUS, POINT_PENETRATE_PCT,
    POINT_RESIST_ASSASSIN, POINT_RESIST_BELL, POINT_RESIST_BOW, POINT_RESIST_CRITICAL,
    POINT_RESIST_DAGGER, POINT_RESIST_DARK, POINT_RESIST_EARTH, POINT_RESIST_ELEC,
    POINT_RESIST_FAN, POINT_RESIST_FIRE, POINT_RESIST_ICE, POINT_RESIST_MAGIC,
    POINT_RESIST_PENETRATE, POINT_RESIST_SHAMAN, POINT_RESIST_SURA, POINT_RESIST_SWORD,
    POINT_RESIST_TWOHAND, POINT_RESIST_WARRIOR, POINT_RESIST_WIND, POINT_SKILL_DAMAGE_BONUS,
    POINT_SKILL_DEFEND_BONUS,
};
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::{one_argument, two_arguments};
use crate::item_move::Mover;
use common::item_slots::EWindows;

/// `INVENTORY_MAX_NUM` (180): the `/inventory` dump stops here.
const DUMP_MAX: i32 = 180;

/// `MAX_PRIV_NUM` (`common/length.h:816`): the priv loop runs types `0..5`.
const MAX_PRIV_NUM: usize = 5;

/// `c_apszPrivNames` (`G/constants.cpp:1080-1087`): the priv word, by type.
const PRIV_NAMES: [&[u8]; MAX_PRIV_NUM] =
    [b"", b"[LS;1038]", b"[LS;1039]", b"[LS;1031]", b"[LS;1040]"];

/// What `/state` reads of its target: a player, whose points answer, or a standing
/// NPC, whose grades and resistances read 0. Both carry the Channel and map they
/// stand on, for the coordinate line's region.
enum StateTarget {
    /// A player under `vid`, on `channel`/`map` at `x`/`y`.
    Player {
        vid: Vid,
        channel: u8,
        map: i32,
        x: i32,
        y: i32,
    },
    /// A standing NPC under `vid`, on `channel`/`map` at `x`/`y`.
    Npc {
        vid: u32,
        channel: u8,
        map: i32,
        x: i32,
        y: i32,
    },
}

impl StateTarget {
    /// The Channel, map and point the target stands on.
    fn place(&self) -> (u8, i32, i32, i32) {
        match *self {
            StateTarget::Player {
                channel, map, x, y, ..
            }
            | StateTarget::Npc {
                channel, map, x, y, ..
            } => (channel, map, x, y),
        }
    }
}

/// `strtoul(word, NULL, 10)` as the `#vid` arm reads it: the digits' value, or 0
/// with no digits. A value past `u32` clamps, as the 32-bit `long` does.
fn vid_number(word: &[u8]) -> u32 {
    let mut value: u64 = 0;
    let mut any = false;
    for &byte in word {
        let digit = byte.wrapping_sub(b'0');
        if digit >= 10 {
            break;
        }
        any = true;
        value = value.saturating_mul(10).saturating_add(u64::from(digit));
    }
    if !any {
        return 0;
    }
    u32::try_from(value.min(u64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// `str_to_number(int)` (`common/utils.h:44-50`) as the `/inventory` counts read
/// it: the sign and digits' value, saturating past `i32`'s ends, 0 with no
/// digits.
fn dump_number(word: &[u8]) -> i32 {
    let (negative, digits) = match word.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, word),
    };
    let mut value: i64 = 0;
    let mut any = false;
    for &byte in digits {
        let digit = byte.wrapping_sub(b'0');
        if digit >= 10 {
            break;
        }
        any = true;
        value = value.saturating_mul(10).saturating_add(i64::from(digit));
    }
    if !any {
        return 0;
    }
    let signed = if negative { -value } else { value };
    i32::try_from(signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
}

impl GameState {
    /// Runs one `/state` line (`do_state`, `G/cmd_gm.cpp:1017-1202`): the target's
    /// dump, told to the caller. Nothing is stored: the dump is tells, not state.
    pub(super) fn run_state(&mut self, vid: Vid, rest: &[u8]) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (word, _) = one_argument(rest);
        let Some(target) = self.state_target(vid, &word) else {
            return;
        };
        self.tell_state_head(vid, mover, vid, &target);
        self.tell_state_coordinate(vid, mover, &target);
        self.tell_state_pools(vid, mover, &target);
        self.tell_state_resists(vid, mover, &target);
        self.tell_state_bonus(vid, mover, &target);
        self.tell_state_privs(vid, mover, &target);
    }

    /// The `/state` target: no word is the caller, `#vid` is the player or NPC
    /// under the VID, a name is the online player of the exact name (`strcmp`). A
    /// VID of 0 finds nothing, as `Find(0)` does.
    fn state_target(&self, vid: Vid, word: &[u8]) -> Option<StateTarget> {
        if word.is_empty() {
            let (channel, map) = self.place_of(vid.raw())?;
            let spot = self.spot_of(vid.raw())?;
            return Some(StateTarget::Player {
                vid,
                channel,
                map,
                x: spot.x,
                y: spot.y,
            });
        }
        if let Some(number) = word.strip_prefix(b"#") {
            let found = vid_number(number);
            if found == 0 {
                return None;
            }
            if self.characters.find_by_vid(Vid::new(found)).is_ok() {
                let (channel, map) = self.place_of(found)?;
                let spot = self.spot_of(found)?;
                return Some(StateTarget::Player {
                    vid: Vid::new(found),
                    channel,
                    map,
                    x: spot.x,
                    y: spot.y,
                });
            }
            let npc = self.npc_by_vid(found)?;
            let (x, y) = (npc.x, npc.y);
            let (channel, map, _) = self.npc_of.get(&found).copied()?;
            return Some(StateTarget::Npc {
                vid: found,
                channel,
                map,
                x,
                y,
            });
        }
        let name = std::str::from_utf8(word).ok()?;
        let (_, found) = self
            .characters
            .player_names()
            .into_iter()
            .find(|(shown, _)| *shown == name)?;
        let (channel, map) = self.place_of(found.raw())?;
        let spot = self.spot_of(found.raw())?;
        Some(StateTarget::Player {
            vid: found,
            channel,
            map,
            x: spot.x,
            y: spot.y,
        })
    }

    /// The state line: `"<name>'s State: "` with Battle, Dead or Standing, then the
    /// *caller's* `, Shop` and `, Exchange` bits (`ch->`, not `tch->`).
    fn tell_state_head(&self, vid: Vid, mover: Mover, caller: Vid, target: &StateTarget) {
        let mut line = Vec::new();
        line.extend_from_slice(&self.state_name(target));
        line.extend_from_slice(b"'s State: ");
        line.extend_from_slice(if self.state_fighting(target) {
            b"Battle"
        } else if self.state_dead(target) {
            b"Dead"
        } else {
            b"Standing"
        });
        if self.browsing.contains_key(&caller) {
            line.extend_from_slice(b", Shop");
        }
        if self.trading.contains_key(&caller) {
            line.extend_from_slice(b", Exchange");
        }
        self.tell(vid, mover, CHAT_TYPE_INFO, b"%s", &[Arg::Text(&line)]);
    }

    /// The coordinate line: `Coordinate XxY (XxY)` in cells and `/100`, with the
    /// `MapIndex/Attribute/Local` tail where the target stands on a sectree's cells.
    /// The base is the region's `sx`/`sy` (`iBaseX`/`iBaseY`).
    fn tell_state_coordinate(&self, vid: Vid, mover: Mover, target: &StateTarget) {
        let (channel, map, x, y) = target.place();
        let mut line = format!("Coordinate {x}x{y} ({}x{})", x / 100, y / 100).into_bytes();
        let cells = self.regens.cells_of(map);
        let region = self.regens.region_of(channel, map);
        if let (Some(cells), Some(region)) = (cells, region) {
            if let Some(attr) = cells.attribute(x, y) {
                line.extend_from_slice(
                    format!(
                        " MapIndex {map} Attribute {:08X} Local Position ({} x {})",
                        attr,
                        (x - region.sx) / 100,
                        (y - region.sy) / 100
                    )
                    .as_bytes(),
                );
            }
        }
        self.tell(vid, mover, CHAT_TYPE_INFO, b"%s", &[Arg::Text(&line)]);
    }

    /// The pools and grades: `LEV`, `HP`, `SP`, `ATT` and `DEF`.
    fn tell_state_pools(&self, vid: Vid, mover: Mover, target: &StateTarget) {
        let point = |kind: usize| self.state_point(target, kind);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"LEV %d",
            &[Arg::Int(i64::from(self.state_level(target)))],
        );
        let (hit, hit_max, mana, mana_max) = self.state_pools(target);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"HP %d/%d",
            &[Arg::Int(i64::from(hit)), Arg::Int(i64::from(hit_max))],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"SP %d/%d",
            &[Arg::Int(i64::from(mana)), Arg::Int(i64::from(mana_max))],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"ATT %d MAGIC_ATT %d SPD %d CRIT %d%% PENE %d%% ATT_BONUS %d%%",
            &[
                Arg::Int(i64::from(point(POINT_ATT_GRADE))),
                Arg::Int(i64::from(point(POINT_MAGIC_ATT_GRADE))),
                Arg::Int(i64::from(point(POINT_ATT_SPEED))),
                Arg::Int(i64::from(point(POINT_CRITICAL_PCT))),
                Arg::Int(i64::from(point(POINT_PENETRATE_PCT))),
                Arg::Int(i64::from(point(POINT_ATT_BONUS))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"DEF %d MAGIC_DEF %d BLOCK %d%% DODGE %d%% DEF_BONUS %d%%",
            &[
                Arg::Int(i64::from(point(POINT_DEF_GRADE))),
                Arg::Int(i64::from(point(POINT_MAGIC_DEF_GRADE))),
                Arg::Int(i64::from(point(POINT_BLOCK))),
                Arg::Int(i64::from(point(POINT_DODGE))),
                Arg::Int(i64::from(point(POINT_DEF_BONUS))),
            ],
        );
    }

    /// The resistances: `RESISTANCES:` with the job, weapon, element and spare rows.
    fn tell_state_resists(&self, vid: Vid, mover: Mover, target: &StateTarget) {
        let point = |kind: usize| self.state_point(target, kind);
        self.tell(vid, mover, CHAT_TYPE_INFO, b"RESISTANCES:", &[]);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   WARR:%3d%% ASAS:%3d%% SURA:%3d%% SHAM:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_RESIST_WARRIOR))),
                Arg::Int(i64::from(point(POINT_RESIST_ASSASSIN))),
                Arg::Int(i64::from(point(POINT_RESIST_SURA))),
                Arg::Int(i64::from(point(POINT_RESIST_SHAMAN))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   SWORD:%3d%% THSWORD:%3d%% DAGGER:%3d%% BELL:%3d%% FAN:%3d%% BOW:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_RESIST_SWORD))),
                Arg::Int(i64::from(point(POINT_RESIST_TWOHAND))),
                Arg::Int(i64::from(point(POINT_RESIST_DAGGER))),
                Arg::Int(i64::from(point(POINT_RESIST_BELL))),
                Arg::Int(i64::from(point(POINT_RESIST_FAN))),
                Arg::Int(i64::from(point(POINT_RESIST_BOW))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   FIRE:%3d%% ELEC:%3d%% MAGIC:%3d%% WIND:%3d%% CRIT:%3d%% PENE:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_RESIST_FIRE))),
                Arg::Int(i64::from(point(POINT_RESIST_ELEC))),
                Arg::Int(i64::from(point(POINT_RESIST_MAGIC))),
                Arg::Int(i64::from(point(POINT_RESIST_WIND))),
                Arg::Int(i64::from(point(POINT_RESIST_CRITICAL))),
                Arg::Int(i64::from(point(POINT_RESIST_PENETRATE))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   ICE:%3d%% EARTH:%3d%% DARK:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_RESIST_ICE))),
                Arg::Int(i64::from(point(POINT_RESIST_EARTH))),
                Arg::Int(i64::from(point(POINT_RESIST_DARK))),
            ],
        );
    }

    /// The mall, bonus and immune lines: `MALL:`, `BONUS:` and `IMMUNE:`. The mall's
    /// `ITEMx` and `GOLDx` words divide their points by 10.
    fn tell_state_bonus(&self, vid: Vid, mover: Mover, target: &StateTarget) {
        let point = |kind: usize| self.state_point(target, kind);
        self.tell(vid, mover, CHAT_TYPE_INFO, b"MALL:", &[]);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   ATT:%3d%% DEF:%3d%% EXP:%3d%% ITEMx%d GOLDx%d",
            &[
                Arg::Int(i64::from(point(POINT_MALL_ATTBONUS))),
                Arg::Int(i64::from(point(POINT_MALL_DEFBONUS))),
                Arg::Int(i64::from(point(POINT_MALL_EXPBONUS))),
                Arg::Int(i64::from(point(POINT_MALL_ITEMBONUS) / 10)),
                Arg::Int(i64::from(point(POINT_MALL_GOLDBONUS) / 10)),
            ],
        );
        self.tell(vid, mover, CHAT_TYPE_INFO, b"BONUS:", &[]);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   SKILL:%3d%% NORMAL:%3d%% SKILL_DEF:%3d%% NORMAL_DEF:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_SKILL_DAMAGE_BONUS))),
                Arg::Int(i64::from(point(POINT_NORMAL_HIT_DAMAGE_BONUS))),
                Arg::Int(i64::from(point(POINT_SKILL_DEFEND_BONUS))),
                Arg::Int(i64::from(point(POINT_NORMAL_HIT_DEFEND_BONUS))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   HUMAN:%3d%% ANIMAL:%3d%% ORC:%3d%% MILGYO:%3d%% UNDEAD:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_ATTBONUS_HUMAN))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_ANIMAL))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_ORC))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_MILGYO))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_UNDEAD))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   DEVIL:%3d%% INSECT:%3d%% FIRE:%3d%% ICE:%3d%% DESERT:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_ATTBONUS_DEVIL))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_INSECT))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_FIRE))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_ICE))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_DESERT))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   TREE:%3d%% MONSTER:%3d%% METIN:%3d%% BOSS:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_ATTBONUS_TREE))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_MONSTER))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_METIN))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_BOSS))),
            ],
        );
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   WARR:%3d%% ASAS:%3d%% SURA:%3d%% SHAM:%3d%%",
            &[
                Arg::Int(i64::from(point(POINT_ATTBONUS_WARRIOR))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_ASSASSIN))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_SURA))),
                Arg::Int(i64::from(point(POINT_ATTBONUS_SHAMAN))),
            ],
        );
        self.tell(vid, mover, CHAT_TYPE_INFO, b"IMMUNE:", &[]);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"   STUN:%d SLOW:%d FALL:%d",
            &[
                Arg::Int(i64::from(point(POINT_IMMUNE_STUN))),
                Arg::Int(i64::from(point(POINT_IMMUNE_SLOW))),
                Arg::Int(i64::from(point(POINT_IMMUNE_FALL))),
            ],
        );
    }

    /// The live privs: for each type with a nonzero `GetPriv`, the nonzero arms'
    /// lines. Only the empire arm reads nonzero: the port holds no guild or
    /// character privs.
    fn tell_state_privs(&self, vid: Vid, mover: Mover, target: &StateTarget) {
        let empire = self.state_empire(target);
        for priv_type in 0..MAX_PRIV_NUM {
            let priv_type = u8::try_from(priv_type).unwrap_or(u8::MAX);
            if self.state_priv(empire, priv_type) == 0 {
                continue;
            }
            let own = self.empire_priv_live(empire, priv_type);
            if own != 0 {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"%s for empire : %d",
                    &[
                        Arg::Text(PRIV_NAMES[usize::from(priv_type)]),
                        Arg::Int(i64::from(own)),
                    ],
                );
            }
        }
    }

    /// `CPrivManager::GetPriv` of the target (`G/priv_manager.cpp:221-240`): the max
    /// over the character row (0, ungranted), the all-empires row, the empire's row
    /// and the guild row (0, ungranted).
    fn state_priv(&self, empire: u8, priv_type: u8) -> i32 {
        self.empire_priv_live(0, priv_type)
            .max(self.empire_priv_live(empire, priv_type))
    }

    /// `GetPrivByEmpire` (`:242-250`): the live row's value, or 0 past the table or
    /// past its end Pulse.
    fn empire_priv_live(&self, empire: u8, priv_type: u8) -> i32 {
        let (priv_type, empire) = (usize::from(priv_type), usize::from(empire));
        let Some(row) = self
            .empire_privs
            .get(priv_type)
            .and_then(|row| row.get(empire))
        else {
            return 0;
        };
        if self.last_pulse >= row.ends {
            0
        } else {
            row.value
        }
    }

    /// The target's shown name: the player's Name, or the NPC's locale name.
    fn state_name(&self, target: &StateTarget) -> Vec<u8> {
        match *target {
            StateTarget::Player { vid, .. } => self
                .characters
                .find_by_vid(vid)
                .map_or(Vec::new(), |character| character.name().as_bytes().to_vec()),
            StateTarget::Npc { vid, .. } => self
                .npc_by_vid(vid)
                .map_or(Vec::new(), |npc| npc.name.clone()),
        }
    }

    /// Whether the target stands fighting: a player's `POSITION_FIGHTING`, or an
    /// NPC's held victim.
    fn state_fighting(&self, target: &StateTarget) -> bool {
        match *target {
            StateTarget::Player { vid, .. } => {
                self.bodies.get(&vid).is_some_and(|body| body.fighting)
            }
            StateTarget::Npc { vid, .. } => self
                .minds
                .get(vid)
                .is_some_and(|mind| mind.victim.is_some() || mind.victim_npc.is_some()),
        }
    }

    /// Whether the target is dead: a player's death, or an NPC's empty pools.
    fn state_dead(&self, target: &StateTarget) -> bool {
        match *target {
            StateTarget::Player { vid, .. } => self.is_dead(vid.raw()),
            StateTarget::Npc { vid, .. } => self.npc_by_vid(vid).is_some_and(|npc| npc.hp <= 0),
        }
    }

    /// The target's level: a player's point, or the NPC's table level.
    fn state_level(&self, target: &StateTarget) -> i32 {
        match *target {
            StateTarget::Player { vid, .. } => self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .map_or(0, |points| i32::from(points.level())),
            StateTarget::Npc { vid, .. } => self
                .npc_by_vid(vid)
                .map_or(0, |npc| i32::from(npc.fight.level)),
        }
    }

    /// The target's empire: a player's card, or the NPC's proto empire.
    fn state_empire(&self, target: &StateTarget) -> u8 {
        match *target {
            StateTarget::Player { vid, .. } => {
                self.bodies.get(&vid).map_or(0, |body| body.card.empire)
            }
            StateTarget::Npc { vid, .. } => self.npc_by_vid(vid).map_or(0, |npc| npc.empire),
        }
    }

    /// `GetHP`, `GetMaxHP`, `GetSP` and `GetMaxSP` of the target (`G/char.h:791-807`):
    /// a player's pools, or an NPC's hit points with no spell pool (the mob table
    /// holds no SP, so `SP` is `0/0`).
    fn state_pools(&self, target: &StateTarget) -> (i32, i32, i32, i32) {
        match *target {
            StateTarget::Player { vid, .. } => self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .map_or((0, 0, 0, 0), |points| {
                    (points.hp(), points.max_hp(), points.sp(), points.max_sp())
                }),
            StateTarget::Npc { vid, .. } => self
                .npc_by_vid(vid)
                .map_or((0, 0, 0, 0), |npc| (npc.hp, npc.max_hp, 0, 0)),
        }
    }

    /// `GetPoint(kind)` of the target: a player's point, or an NPC's pools for the
    /// two HP kinds and 0 elsewhere (the mob table holds no SP).
    fn state_point(&self, target: &StateTarget, kind: usize) -> i32 {
        match *target {
            StateTarget::Player { vid, .. } => self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.points())
                .map_or(0, |points| points.get_point(kind)),
            StateTarget::Npc { vid, .. } => {
                let Some(npc) = self.npc_by_vid(vid) else {
                    return 0;
                };
                match kind {
                    POINT_HP => npc.hp,
                    POINT_MAX_HP => npc.max_hp,
                    _ => 0,
                }
            }
        }
    }

    /// Runs one `/inventory` line (`do_inventory`, `G/cmd_general.cpp:3210-3250`):
    /// the caller's own cells from an index for a count. Nothing is stored.
    pub(super) fn run_inventory(&mut self, vid: Vid, rest: &[u8]) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let (first, second) = two_arguments(rest);
        if first.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: inventory <start_index> <count>",
                &[],
            );
            return;
        }
        let (mut index, count) = if second.is_empty() {
            (0, dump_number(&first))
        } else {
            (
                dump_number(&first).min(DUMP_MAX),
                dump_number(&second).min(DUMP_MAX),
            )
        };
        for _ in 0..count.max(0) {
            if index >= DUMP_MAX {
                break;
            }
            let name = self.inventory_name(vid, index);
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"inventory [%d] = %s",
                &[Arg::Int(i64::from(index)), Arg::Text(&name)],
            );
            index += 1;
        }
    }

    /// The `/inventory` name of the caller's cell: the item's locale name, or
    /// `"<NONE>"` where no item stands. A negative index wraps to its `WORD`
    /// cell, as the `int` to `WORD` conversion does; a wrapped cell past the
    /// slots holds nothing.
    fn inventory_name(&self, vid: Vid, index: i32) -> Vec<u8> {
        let cell = u16::try_from(index.rem_euclid(65536)).unwrap_or(0);
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| {
                character.items().item_at(protocol::item_pos::ItemPos::new(
                    EWindows::Inventory as u8,
                    cell,
                ))
            })
            .map_or(b"<NONE>".to_vec(), |item| {
                self.protos
                    .get(item.vnum)
                    .map(|proto| proto.locale_name.clone())
                    .unwrap_or_default()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{dump_number, vid_number, DUMP_MAX};
    use crate::game_state::fixtures::{a_world, enter_on, TestClock, PLACE};
    use crate::game_state::GameState;
    use common::vid::Vid;

    #[test]
    fn the_vid_reads_digits_and_clamps() {
        assert_eq!(vid_number(b"7"), 7);
        assert_eq!(vid_number(b"12ab"), 12);
        assert_eq!(vid_number(b""), 0);
        assert_eq!(vid_number(b"ab"), 0);
        assert_eq!(vid_number(b"99999999999"), u32::MAX);
    }

    #[test]
    fn the_dump_counts_read_signs_and_saturate() {
        assert_eq!(dump_number(b"12"), 12);
        assert_eq!(dump_number(b"-5"), -5);
        assert_eq!(dump_number(b"+7x"), 7);
        assert_eq!(dump_number(b""), 0);
        assert_eq!(dump_number(b"99999999999"), i32::MAX);
        assert_eq!(dump_number(b"-99999999999"), i32::MIN);
        assert_eq!(DUMP_MAX, 180);
    }

    #[test]
    fn a_priv_on_the_all_empires_row_passes_the_gate_and_tells_nothing() {
        let clock = TestClock::default();
        let mut world: GameState = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 7, (0, 0), 12_345);
        // A grant on the all-empires row: the gate passes, but the empire line
        // tells only the target's own row, so nothing rides.
        world.give_empire_priv(0, 1, 50, u64::MAX);
        while inbox.try_recv().is_ok() {}
        world.run_state(Vid::new(7), b"");
        let mut lines = Vec::new();
        while let Ok(record) = inbox.try_recv() {
            if record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value() {
                lines.push(record);
            }
        }
        // Head, coordinate, 5 pools, 5 resists, 10 mall/bonus/immune: no priv line.
        assert_eq!(lines.len(), 2 + 5 + 5 + 10);
        // The target's own row tells.
        world.give_empire_priv(1, 1, 30, u64::MAX);
        world.run_state(Vid::new(7), b"");
        let mut privs = 0;
        while let Ok(record) = inbox.try_recv() {
            if record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value() {
                privs += 1;
            }
        }
        assert_eq!(privs, 2 + 5 + 5 + 10 + 1);
    }

    #[test]
    fn an_unknown_name_and_a_zero_vid_are_silent() {
        let clock = TestClock::default();
        let mut world: GameState = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 7, (0, 0), 12_345);
        world.run_state(Vid::new(7), b"Nobody");
        world.run_state(Vid::new(7), b"#0");
        world.run_state(Vid::new(7), b"#99999");
        assert!(inbox.try_recv().is_err(), "no target, no lines");
    }

    #[test]
    fn the_inventory_tells_its_usage_and_its_cells() {
        let clock = TestClock::default();
        let mut world: GameState = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 7, (0, 0), 12_345);
        world.run_inventory(Vid::new(7), b"");
        let usage = inbox.try_recv().expect("the usage rides");
        assert_eq!(usage[0], protocol::gc_inventory::HEADER_GC_CHAT.value());
        world.run_inventory(Vid::new(7), b"1");
        let cell = inbox.try_recv().expect("one cell rides");
        assert_eq!(cell[0], protocol::gc_inventory::HEADER_GC_CHAT.value());
        assert!(inbox.try_recv().is_err(), "one cell, one line");
        world.run_inventory(Vid::new(7), b"0 0");
        assert!(inbox.try_recv().is_err(), "a zero count, no lines");
        world.run_inventory(Vid::new(7), b"180 5");
        assert!(inbox.try_recv().is_err(), "past the end, no lines");
    }
}
