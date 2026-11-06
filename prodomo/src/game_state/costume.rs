//! The hair and costume lines: `/hair` and `/costume` (`ACMD(do_hair)`,
//! `ACMD(do_costume)`, `G/cmd_general.cpp:2957-3208`).
//!
//! Both lines share `FN_point_string`'s table (`:2960-3047`): the `%`-format of one
//! applied point, looked up in the caller's language before formatting, as
//! [`chat_packet`](crate::chat_line::chat_packet) does for every line.
//!
//! Four legacy shapes survive verbatim in the table. `POINT_MOV_SPEED` takes `%d` where
//! its siblings take `+%d`, `POINT_CASTING_SPEED` takes `-%d`, `POINT_MALL_EXPBONUS`
//! reads `POINT_MALL_DEFBONUS`, and `POINT_MALL_DEFBONUS` is the four U+FFFD
//! replacement characters the checked-in source holds: the file was saved in a legacy
//! code page and its Korean reads as mojibake. The two `%.1f` formats
//! (`POINT_MALL_ITEMBONUS`, `POINT_MALL_GOLDBONUS`) are a Defect: the call sites pass
//! the attribute's `short` value, promoted to `int`, where `%f` expects a `double`,
//! so legacy reads the value and whatever follows it as a floating point number and
//! the line is unstable. The Rewrite renders those two as the integer over 100 with
//! two decimals, which is what the percentage the point holds means
//! (`G/char_battle.cpp:582-583` divides it by 100).
//!
//! `ENABLE_MAGIC_REDUCTION_SYSTEM` is undefined on this tree, so the
//! `POINT_RESIST_MAGIC_REDUCTION` arm does not compile and is not in the table.

use common::point_slot::{
    POINT_ATTBONUS_ANIMAL, POINT_ATTBONUS_ASSASSIN, POINT_ATTBONUS_BOSS, POINT_ATTBONUS_DEVIL,
    POINT_ATTBONUS_HUMAN, POINT_ATTBONUS_METIN, POINT_ATTBONUS_MILGYO, POINT_ATTBONUS_MONSTER,
    POINT_ATTBONUS_ORC, POINT_ATTBONUS_SHAMAN, POINT_ATTBONUS_SURA, POINT_ATTBONUS_UNDEAD,
    POINT_ATTBONUS_WARRIOR, POINT_ATT_GRADE_BONUS, POINT_ATT_SPEED, POINT_BLOCK,
    POINT_CASTING_SPEED, POINT_CRITICAL_PCT, POINT_DAMAGE_SP_RECOVER, POINT_DEF_GRADE_BONUS,
    POINT_DODGE, POINT_DX, POINT_EXP_DOUBLE_BONUS, POINT_GOLD_DOUBLE_BONUS, POINT_HP_REGEN,
    POINT_HT, POINT_IQ, POINT_ITEM_DROP_BONUS, POINT_KILL_HP_RECOVERY, POINT_KILL_SP_RECOVER,
    POINT_MAGIC_ATT_GRADE, POINT_MAGIC_DEF_GRADE, POINT_MALL_ATTBONUS, POINT_MALL_DEFBONUS,
    POINT_MALL_EXPBONUS, POINT_MALL_GOLDBONUS, POINT_MALL_ITEMBONUS, POINT_MANA_BURN_PCT,
    POINT_MAX_HP, POINT_MAX_HP_PCT, POINT_MAX_SP, POINT_MAX_SP_PCT, POINT_MAX_STAMINA,
    POINT_MOV_SPEED, POINT_NORMAL_HIT_DAMAGE_BONUS, POINT_NORMAL_HIT_DEFEND_BONUS,
    POINT_PENETRATE_PCT, POINT_POISON_PCT, POINT_POISON_REDUCE, POINT_POTION_BONUS,
    POINT_REFLECT_CURSE, POINT_REFLECT_MELEE, POINT_RESIST_ASSASSIN, POINT_RESIST_BELL,
    POINT_RESIST_BOW, POINT_RESIST_CRITICAL, POINT_RESIST_DAGGER, POINT_RESIST_DARK,
    POINT_RESIST_EARTH, POINT_RESIST_ELEC, POINT_RESIST_FAN, POINT_RESIST_FIRE, POINT_RESIST_ICE,
    POINT_RESIST_MAGIC, POINT_RESIST_PENETRATE, POINT_RESIST_SHAMAN, POINT_RESIST_SURA,
    POINT_RESIST_SWORD, POINT_RESIST_TWOHAND, POINT_RESIST_WARRIOR, POINT_RESIST_WIND,
    POINT_SKILL_DAMAGE_BONUS, POINT_SKILL_DEFEND_BONUS, POINT_SLOW_PCT, POINT_SP_REGEN, POINT_ST,
    POINT_STEAL_HP, POINT_STEAL_SP, POINT_STUN_PCT,
};
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// One row of `FN_point_string` (`G/cmd_general.cpp:2960-3047`): the applied point and
/// its `%`-format, in legacy's `switch` order.
const POINT_FORMATS: &[(usize, &[u8])] = &[
    (POINT_MAX_HP, b"POINT_MAX_HP +%d"),
    (POINT_MAX_SP, b"POINT_MAX_SP +%d"),
    (POINT_HT, b"POINT_HT +%d"),
    (POINT_IQ, b"POINT_IQ +%d"),
    (POINT_ST, b"POINT_ST +%d"),
    (POINT_DX, b"POINT_DX +%d"),
    (POINT_ATT_SPEED, b"POINT_ATT_SPEED +%d"),
    (POINT_MOV_SPEED, b"POINT_MOV_SPEED %d"),
    (POINT_CASTING_SPEED, b"POINT_CASTING_SPEED -%d"),
    (POINT_HP_REGEN, b"POINT_HP_REGEN +%d"),
    (POINT_SP_REGEN, b"POINT_SP_REGEN +%d"),
    (POINT_POISON_PCT, b"POINT_POISON_PCT %d"),
    (POINT_STUN_PCT, b"POINT_STUN_PCT +%d"),
    (POINT_SLOW_PCT, b"POINT_SLOW_PCT +%d"),
    (POINT_CRITICAL_PCT, b"POINT_CRITICAL_PCT %d%%"),
    (POINT_RESIST_CRITICAL, b"POINT_RESIST_CRITICAL %d%%"),
    (POINT_PENETRATE_PCT, b"POINT_PENETRATE_PCT %d%%"),
    (POINT_RESIST_PENETRATE, b"POINT_RESIST_PENETRATE %d%%"),
    (POINT_ATTBONUS_HUMAN, b"POINT_ATTBONUS_HUMAN +%d%%"),
    (POINT_ATTBONUS_ANIMAL, b"POINT_ATTBONUS_ANIMAL +%d%%"),
    (POINT_ATTBONUS_ORC, b"POINT_ATTBONUS_ORC +%d%%"),
    (POINT_ATTBONUS_MILGYO, b"POINT_ATTBONUS_MILGYO +%d%%"),
    (POINT_ATTBONUS_UNDEAD, b"POINT_ATTBONUS_UNDEAD +%d%%"),
    (POINT_ATTBONUS_DEVIL, b"POINT_ATTBONUS_DEVIL +%d%%"),
    (POINT_STEAL_HP, b"POINT_STEAL_HP %d%%"),
    (POINT_STEAL_SP, b"POINT_STEAL_SP %d%%"),
    (POINT_MANA_BURN_PCT, b"POINT_MANA_BURN_PCT %d%%"),
    (POINT_DAMAGE_SP_RECOVER, b"POINT_DAMAGE_SP_RECOVER %d%%"),
    (POINT_BLOCK, b"POINT_BLOCK %d%%"),
    (POINT_DODGE, b"POINT_DODGE %d%%"),
    (POINT_RESIST_SWORD, b"POINT_RESIST_SWORD %d%%"),
    (POINT_RESIST_TWOHAND, b"POINT_RESIST_TWOHAND %d%%"),
    (POINT_RESIST_DAGGER, b"POINT_RESIST_DAGGER %d%%"),
    (POINT_RESIST_BELL, b"POINT_RESIST_BELL %d%%"),
    (POINT_RESIST_FAN, b"POINT_RESIST_FAN %d%%"),
    (POINT_RESIST_BOW, b"POINT_RESIST_BOW %d%%"),
    (POINT_RESIST_FIRE, b"POINT_RESIST_FIRE %d%%"),
    (POINT_RESIST_ELEC, b"POINT_RESIST_ELEC %d%%"),
    (POINT_RESIST_MAGIC, b"POINT_RESIST_MAGIC %d%%"),
    (POINT_RESIST_WIND, b"POINT_RESIST_WIND %d%%"),
    (POINT_RESIST_ICE, b"POINT_RESIST_ICE %d%%"),
    (POINT_RESIST_EARTH, b"POINT_RESIST_EARTH %d%%"),
    (POINT_RESIST_DARK, b"POINT_RESIST_DARK %d%%"),
    (POINT_REFLECT_MELEE, b"POINT_REFLECT_MELEE %d%%"),
    (POINT_REFLECT_CURSE, b"POINT_REFLECT_CURSE %d%%"),
    (POINT_POISON_REDUCE, b"POINT_POISON_REDUCE %d%%"),
    (POINT_KILL_SP_RECOVER, b"POINT_KILL_SP_RECOVER %d%%"),
    (POINT_EXP_DOUBLE_BONUS, b"POINT_EXP_DOUBLE_BONUS %d%%"),
    (POINT_GOLD_DOUBLE_BONUS, b"POINT_GOLD_DOUBLE_BONUS %d%%"),
    (POINT_ITEM_DROP_BONUS, b"POINT_ITEM_DROP_BONUS %d%%"),
    (POINT_POTION_BONUS, b"POINT_POTION_BONUS %d%%"),
    (POINT_KILL_HP_RECOVERY, b"POINT_KILL_HP_RECOVERY %d%%"),
    (POINT_ATT_GRADE_BONUS, b"POINT_ATT_GRADE_BONUS +%d"),
    (POINT_DEF_GRADE_BONUS, b"POINT_DEF_GRADE_BONUS +%d"),
    (POINT_MAGIC_ATT_GRADE, b"POINT_MAGIC_ATT_GRADE +%d"),
    (POINT_MAGIC_DEF_GRADE, b"POINT_MAGIC_DEF_GRADE +%d"),
    (POINT_MAX_STAMINA, b"POINT_MAX_STAMINA +%d"),
    (POINT_ATTBONUS_WARRIOR, b"POINT_ATTBONUS_WARRIOR +%d%%"),
    (POINT_ATTBONUS_ASSASSIN, b"POINT_ATTBONUS_ASSASSIN +%d%%"),
    (POINT_ATTBONUS_SURA, b"POINT_ATTBONUS_SURA +%d%%"),
    (POINT_ATTBONUS_SHAMAN, b"POINT_ATTBONUS_SHAMAN +%d%%"),
    (POINT_ATTBONUS_MONSTER, b"POINT_ATTBONUS_MONSTER +%d%%"),
    (POINT_MALL_ATTBONUS, b"POINT_MALL_ATTBONUS +%d%%"),
    (POINT_MALL_DEFBONUS, "���� +%d%%".as_bytes()),
    (POINT_MALL_EXPBONUS, b"POINT_MALL_DEFBONUS %d%%"),
    (POINT_MALL_ITEMBONUS, b"POINT_MALL_ITEMBONUS %d.%02d"),
    (POINT_MALL_GOLDBONUS, b"POINT_MALL_GOLDBONUS %d.%02d"),
    (POINT_MAX_HP_PCT, b"POINT_MAX_HP_PCT +%d%%"),
    (POINT_MAX_SP_PCT, b"POINT_MAX_SP_PCT +%d%%"),
    (POINT_SKILL_DAMAGE_BONUS, b"POINT_SKILL_DAMAGE_BONUS %d%%"),
    (
        POINT_NORMAL_HIT_DAMAGE_BONUS,
        b"POINT_NORMAL_HIT_DAMAGE_BONUS %d%%",
    ),
    (POINT_SKILL_DEFEND_BONUS, b"POINT_SKILL_DEFEND_BONUS %d%%"),
    (
        POINT_NORMAL_HIT_DEFEND_BONUS,
        b"POINT_NORMAL_HIT_DEFEND_BONUS %d%%",
    ),
    (POINT_RESIST_WARRIOR, b"POINT_RESIST_WARRIOR %d%%"),
    (POINT_RESIST_ASSASSIN, b"POINT_RESIST_ASSASSIN %d%%"),
    (POINT_RESIST_SURA, b"POINT_RESIST_SURA %d%%"),
    (POINT_RESIST_SHAMAN, b"POINT_RESIST_SHAMAN %d%%"),
    (POINT_ATTBONUS_METIN, b"POINT_ATTBONUS_METIN +%d%%"),
    (POINT_ATTBONUS_BOSS, b"POINT_ATTBONUS_BOSS +%d%%"),
];

/// `FN_point_string` (`G/cmd_general.cpp:2960-3047`): the `%`-format of the applied
/// point `on`, or `None` past the table's end (`default: return NULL`).
fn point_format(on: u8) -> Option<&'static [u8]> {
    POINT_FORMATS
        .iter()
        .find(|(point, _)| *point == usize::from(on))
        .map(|(_, format)| *format)
}

/// Whether `format` is one of the two `%.1f` rows, which the Rewrite renders as the
/// integer over 100: see the module documentation.
fn is_float_format(format: &[u8]) -> bool {
    format == b"POINT_MALL_ITEMBONUS %d.%02d" || format == b"POINT_MALL_GOLDBONUS %d.%02d"
}

/// The chat arguments of `value` under `format`: the whole and the hundredths for the
/// two float rows, the value alone otherwise.
fn point_args(format: &[u8], value: i32) -> [Arg<'static>; 2] {
    if !is_float_format(format) {
        return [Arg::Int(i64::from(value)), Arg::Int(0)];
    }
    let whole = value / 100;
    let cents = (value % 100).abs();
    [Arg::Int(i64::from(whole)), Arg::Int(i64::from(cents))]
}

impl GameState {
    /// Runs one `/hair` line (`do_hair`, `G/cmd_general.cpp:3200-3208`): the caller's hair
    /// affect as one line, `FN_hair_affect_string`'s text (`:3052-3087`).
    ///
    /// No hair affect, or a `hair.limit_time` flag below the world clock, runs nothing:
    /// both return `false` and `do_hair` returns. Else the line is the affect's point
    /// format with its value, then `[LS;651;year;month;day]` of the flag's date. A
    /// point past the table's end would crash legacy (`snprintf` of `NULL`); the
    /// Rewrite tells only the date, since the affect 514 row no live path grants can
    /// carry any point. Nothing is stored: the line is a tell, not state.
    pub(super) fn run_hair(&mut self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some(affect) = self.affect_events.get(&vid).and_then(|event| {
            event
                .affects
                .iter()
                .find(|affect| affect.kind == crate::loading_phase::AFFECT_HAIR)
                .copied()
        }) else {
            return;
        };
        let flag = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| {
                self.quests.as_ref().and_then(|quests| {
                    quests
                        .manager()
                        .quest_flag(character.player_id(), b"hair.limit_time")
                })
            })
            .unwrap_or(0);
        let now = Self::global_time();
        if flag < 0 || u32::try_from(flag).map_or(true, |expire| expire < now) {
            return;
        }
        let expire = u32::try_from(flag).unwrap_or(0);
        let date = civil_date(expire);
        match point_format(affect.apply_on) {
            Some(format) if !is_float_format(format) => {
                self.tell_hair_format(vid, mover, format, affect.value, date);
            }
            Some(format) => {
                self.tell_hair_float(vid, mover, format, affect.value, date);
            }
            None => {
                let (year, month, day) = date;
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;651;%d;%d;%d]",
                    &[
                        Arg::Int(i64::from(year)),
                        Arg::Int(i64::from(month)),
                        Arg::Int(i64::from(day)),
                    ],
                );
            }
        }
    }
    /// Runs one `/costume` line (`do_costume`, `G/cmd_general.cpp:3089-3198`) and answers
    /// with the moved step: the status lines are told, and the named piece comes off
    /// through [`GameState::use_item`], so the records ride and the rows are stored as a
    /// click's are.
    ///
    /// The six pieces are told in legacy's order — hair, body, mount, sash, weapon, aura
    /// (`:3114-3195`) — then the first word's first byte takes one off: `h`, `b`, `m`,
    /// `s`, `w`, or `a`. Any other word, or the piece missing, tells only. A piece names
    /// its cell through [`wear_pos`](world::character::wear_pos), the flat cell a wear
    /// cell holds.
    /// `COSTUME status:` is told even with nothing worn, and a worn piece names its
    /// locale name (`CItem::GetName`, `G/item.cpp:2994-2997`). Only the hair
    /// (`:3119-3128`) and the sash (`:3155-3171`) name their attributes: each attribute
    /// up to the first zero type (`CItem::GetAttributeCount`,
    /// `G/item_attribute.cpp:287-298`) with a type past 0 tells its point format, and
    /// the sash skips a point past the table's end (`pAttrName == NULL`, `:3162-3163`).
    /// The hair loop has no such guard: a point past the table's end would crash legacy
    /// (`snprintf` of `NULL`), so the Rewrite skips it too, since no hair-costume
    /// attribute of the shipped Game data names one. Nothing else is stored: the status
    /// is tells, not state.
    pub(super) fn run_costume(
        &mut self,
        vid: Vid,
        rest: &[u8],
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        use common::enums::EWearPositions;
        use common::item_slots::EWindows;
        let Some(mover) = self.own_mover(vid) else {
            return self.use_item(
                vid,
                protocol::item_pos::ItemPos::new(EWindows::Inventory as u8, 0),
                actor,
                &crate::game_loop_messages::QuestAccount::default(),
            );
        };
        self.tell(vid, mover, CHAT_TYPE_INFO, b"COSTUME status:", &[]);
        let pieces: [(EWearPositions, &[u8], u8, bool); 6] = [
            (EWearPositions::CostumeHair, b"  HAIR : %s", b'h', true),
            (EWearPositions::CostumeBody, b"  BODY : %s", b'b', false),
            (EWearPositions::CostumeMount, b"  MOUNT : %s", b'm', false),
            (EWearPositions::CostumeSash, b"  SASH: %s", b's', true),
            (EWearPositions::CostumeWeapon, b"  WEAPON : %s", b'w', false),
            (EWearPositions::CostumeAura, b"  AURA : %s", b'a', false),
        ];
        let (word, _) = crate::command::one_argument(rest);
        let asked = word.first().copied();
        let mut asked_at = None;
        for (wear, label, letter, with_attrs) in pieces {
            let at = world::character::wear_pos(wear as u16);
            let Some(item) = self
                .characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| character.items().item_at(at).cloned())
            else {
                continue;
            };
            let name = self
                .protos
                .get(item.vnum)
                .map(|proto| proto.locale_name.clone())
                .unwrap_or_default();
            self.tell(vid, mover, CHAT_TYPE_INFO, label, &[Arg::Text(&name)]);
            if with_attrs {
                self.tell_costume_attrs(vid, mover, &item.attributes);
            }
            if asked == Some(letter) {
                asked_at = Some(at);
            }
        }
        let Some(at) = asked_at else {
            return self.empty_costume_step(vid, actor);
        };
        self.use_item(
            vid,
            at,
            actor,
            &crate::game_loop_messages::QuestAccount::default(),
        )
    }

    /// The status chat lines of one costume piece's attributes: each attribute up to the
    /// first zero type with a type past 0 tells its point format with its value. A
    /// point past the table's end is skipped, as the sash's `NULL` guard does.
    fn tell_costume_attrs(
        &self,
        vid: Vid,
        mover: Mover,
        attrs: &[protocol::gc_item_window::ItemAttribute],
    ) {
        for attr in attrs {
            if attr.b_type == 0 {
                break;
            }
            let Some(format) = point_format(attr.b_type) else {
                continue;
            };
            let mut line = Vec::with_capacity(format.len() + 5);
            line.extend_from_slice(b"     ");
            line.extend_from_slice(format);
            if is_float_format(format) {
                let [whole, cents] = point_args(format, i32::from(attr.s_value));
                self.tell(vid, mover, CHAT_TYPE_INFO, &line, &[whole, cents]);
            } else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    &line,
                    &[Arg::Int(i64::from(attr.s_value))],
                );
            }
        }
    }

    /// The moved step of a `/costume` line that took nothing off: the empty step with
    /// the caller's points, as `run_use_item_line`'s miss answers its silence.
    fn empty_costume_step(&self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let mut empty = MovedItems::new(
            character.player_id(),
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Used,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        empty.points = character.points().cloned();
        Ok(empty)
    }

    /// The `/hair` line of an integer point format: the formatted point, then the
    /// `[LS;651]` date, in one tell.
    fn tell_hair_format(
        &self,
        vid: Vid,
        mover: Mover,
        format: &[u8],
        value: i32,
        date: (i32, u32, u32),
    ) {
        let (year, month, day) = date;
        let mut line = Vec::with_capacity(format.len() + 24);
        line.extend_from_slice(format);
        line.extend_from_slice(b"[LS;651;%d;%d;%d]");
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            &line,
            &[
                Arg::Int(i64::from(value)),
                Arg::Int(i64::from(year)),
                Arg::Int(i64::from(month)),
                Arg::Int(i64::from(day)),
            ],
        );
    }

    /// The `/hair` line of a float point format: the whole and hundredths, then the
    /// `[LS;651]` date, in one tell.
    fn tell_hair_float(
        &self,
        vid: Vid,
        mover: Mover,
        format: &[u8],
        value: i32,
        date: (i32, u32, u32),
    ) {
        let (year, month, day) = date;
        let [whole, cents] = point_args(format, value);
        let mut line = Vec::with_capacity(format.len() + 24);
        line.extend_from_slice(format);
        line.extend_from_slice(b"[LS;651;%d;%d;%d]");
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            &line,
            &[
                whole,
                cents,
                Arg::Int(i64::from(year)),
                Arg::Int(i64::from(month)),
                Arg::Int(i64::from(day)),
            ],
        );
    }
}

/// The civil date of `stamp` seconds after the Unix epoch: the year, the 1-based
/// month, and the day (`localtime_r`'s `tm_year + 1900`, `tm_mon + 1`, `tm_mday`,
/// `G/cmd_general.cpp:3077-3083`). The server clock runs in UTC, so the date is UTC.
fn civil_date(stamp: u32) -> (i32, u32, u32) {
    let days = i64::from(stamp) / 86_400;
    civil_from_days(days)
}

/// The civil date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`): the
/// year, the 1-based month, and the day. The shift moves the epoch to 0000-03-01,
/// which the algorithm counts from.
pub(super) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = u32::try_from(shifted - era * 146_097).unwrap_or(u32::MAX);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = i64::from(year_of_era) + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (i32::try_from(year).unwrap_or(i32::MAX), month, day)
}

#[cfg(test)]
mod tests {
    use super::{civil_date, is_float_format, point_args, point_format, POINT_FORMATS};
    use crate::chat_line::Arg;
    use common::point_slot::{POINT_ATTBONUS_BOSS, POINT_MALL_GOLDBONUS, POINT_MALL_ITEMBONUS};

    #[test]
    fn the_table_holds_every_arm_in_legacy_order() {
        assert_eq!(POINT_FORMATS.len(), 79);
        assert_eq!(POINT_FORMATS.first().map(|(point, _)| *point), Some(6));
        assert_eq!(
            POINT_FORMATS.last().map(|(point, _)| *point),
            Some(POINT_ATTBONUS_BOSS)
        );
        assert!(point_format(6).is_some());
        assert!(point_format(0).is_none());
    }

    #[test]
    fn the_float_rows_render_over_one_hundred() {
        assert!(is_float_format(point_format(117).unwrap_or(b"")));
        assert!(is_float_format(point_format(118).unwrap_or(b"")));
        let _ = (POINT_MALL_ITEMBONUS, POINT_MALL_GOLDBONUS);
        assert_eq!(
            point_args(b"POINT_MALL_ITEMBONUS %d.%02d", 250),
            [Arg::Int(2), Arg::Int(50)]
        );
        assert_eq!(
            point_args(b"POINT_MAX_HP +%d", 250),
            [Arg::Int(250), Arg::Int(0)]
        );
    }

    #[test]
    fn the_epoch_and_a_leap_day_render() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(1_582_934_400), (2020, 2, 29));
        assert_eq!(civil_date(1_700_000_000), (2023, 11, 14));
    }

    #[test]
    fn hair_is_silent_without_an_affect_or_past_its_flag() {
        use crate::game_state::fixtures::{a_world, enter_on, TestClock, PLACE};
        use crate::game_state::GameState;
        use crate::loading_phase::AFFECT_HAIR;
        use common::vid::Vid;
        use quest::manager::Manager;
        let clock = TestClock::default();
        let mut world: GameState = a_world(&clock);
        let mut inbox = enter_on(&mut world, PLACE, 7, (0, 0), 12_345);
        let quest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager = Manager::load(&quest_dir).unwrap_or_else(|error| panic!("{error}"));
        world.quests = Some(crate::game_state::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            gamedata::mob_locale_names::MobNamesByLanguage::default(),
        ));
        world.run_hair(Vid::new(7));
        assert!(inbox.try_recv().is_err(), "no affect, no line");
        world.add_affect(
            Vid::new(7),
            crate::game_state::affect::Affect {
                kind: AFFECT_HAIR,
                apply_on: 6,
                value: 500,
                flag: 0,
                remaining: 3600,
                sp_cost: 0,
            },
        );
        world.run_hair(Vid::new(7));
        assert!(inbox.try_recv().is_err(), "no flag, no line");
        let pid = world
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the entrant stands")
            .player_id();
        world
            .quests
            .as_mut()
            .expect("the quests load")
            .manager_mut()
            .set_quest_flag(pid, b"hair.limit_time", 1);
        world.run_hair(Vid::new(7));
        assert!(inbox.try_recv().is_err(), "a spent flag, no line");
    }
}
