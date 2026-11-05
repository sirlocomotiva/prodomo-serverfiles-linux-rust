//! Emotions: `/emotion_allow` and the forty `/do_emotion` lines (`G/cmd_emotion.cpp`).
//!
//! [`GameState::run_emotion`] is `ACMD(do_emotion)` (`:153-296`): the ride gate, the table
//! row the entry names, the mask gate, the woman gate, the named victim in view, the NEED
//! gates, the victim gates (a PC, not itself, not riding, 10..500 away, the other sex, the
//! consent pair unless married), the consent insert, and the `CHAT_TYPE_COMMAND` line
//! `"<to_client> <vid> <victim>"` to the view (`PacketAround`). [`GameState::allow_emotion`]
//! is `ACMD(do_emotion_allow)` (`:112-128`): the arena gate, then the `(VID, number)` consent
//! insert.
//!
//! What legacy reads that this build does not track: the arena (no character is on one),
//! riding (no character rides), and marriage (no character is married). The wedding map
//! (81) and the two mask uniques (71011, 71033) are read for real; the owner's config sets
//! `disable_emotion_mask`, so the mask gate passes by default. `NEED_TARGET` never appears
//! in the table, the disarm flags are never checked, and the consent set is never cleared:
//! all three are legacy bugs this build keeps, because no honest client can tell them from
//! the table.

use std::collections::BTreeSet;

use common::vid::Vid;

use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};

use crate::chat_line::chat_packet;
use crate::game_state::view::EntityKey;
use crate::game_state::GameState;
use crate::item_move::Mover;
use crate::sync_position::distance_approx;
use common::constants::CHAT_MAX_LEN;
use common::enums::EWearPositions;
use world::character::{is_female, Equipment};
/// `NEED_TARGET` (`G/cmd_emotion.cpp:13`): no row sets it, so the check it arms never fires.
const NEED_TARGET: u32 = 1 << 0;
/// `NEED_PC` (`:14`): the victim must be a PC, and the consent pair or marriage gates it.
const NEED_PC: u32 = 1 << 1;
/// `WOMAN_ONLY` (`:15`): a male caller hears `[LS;425]`.
const WOMAN_ONLY: u32 = 1 << 2;
/// `OTHER_SEX_ONLY` (`:16`): a victim of the caller's sex hears `[LS;470]`.
const OTHER_SEX_ONLY: u32 = 1 << 3;

/// `WEDDING_MAP_INDEX` (`G/wedding.h:8`): its maps skip the mask gate.
const WEDDING_MAP_INDEX: i32 = 81;
/// `UNIQUE_ITEM_EMOTION_MASK` (`G/unique_item.h:50`): worn, it passes the mask gate.
const EMOTION_MASK: u32 = 71_011;
/// `UNIQUE_ITEM_EMOTION_MASK2` (`:51`): worn, it passes the mask gate.
const EMOTION_MASK2: u32 = 71_033;

/// One row of `emotion_types` (`G/cmd_emotion.cpp:25-71`): the client command and its flags.
/// The Korean command column is legacy bytes the `cmd_info` table never registers, so it is
/// not ported: the walk names rows by their `command_to_client`, which is every registered
/// entry.
const EMOTIONS: &[(&str, u32)] = &[
    ("french_kiss", NEED_PC | OTHER_SEX_ONLY),
    ("kiss", NEED_PC | OTHER_SEX_ONLY),
    ("slap", NEED_PC),
    ("clap", 0),
    ("cheer1", 0),
    ("cheer2", 0),
    ("dance1", 0),
    ("dance2", 0),
    ("dance3", 0),
    ("dance4", 0),
    ("dance5", 0),
    ("dance6", 0),
    ("congratulation", 0),
    ("forgive", 0),
    ("angry", 0),
    ("attractive", 0),
    ("sad", 0),
    ("shy", 0),
    ("cheerup", 0),
    ("banter", 0),
    ("joy", 0),
    ("pushup", 0),
    ("dance7", 0),
    ("exercise", 0),
    ("doze", 0),
    ("selfie", 0),
    ("cherging", 0),
    ("nosay", 0),
    ("weather_1", 0),
    ("weather_2", 0),
    ("weather_3", 0),
    ("hungry", 0),
    ("siren", 0),
    ("letter", 0),
    ("call", 0),
    ("celebration", 0),
    ("alcohol", 0),
    ("busy", 0),
    ("whirl", 0),
];

/// `(unsigned int)strtoul(word)` (`common/utils.h:52-58`): the VID an `/emotion_allow` word
/// names. Leading spaces are already gone, an optional sign leads, digits run to the first
/// non-digit, and only the low 32 bits survive; a word with no digits reads as zero, and a
/// magnitude past `u64::MAX` clamps there first.
fn parse_allow_number(word: &[u8]) -> u32 {
    let (negative, digits) = match word.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, word),
    };
    let mut value: u64 = 0;
    let mut any = false;
    for &byte in digits {
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
    let low = u32::try_from(value & 0xffff_ffff).expect("a masked word fits");
    if negative {
        low.wrapping_neg()
    } else {
        low
    }
}

impl GameState {
    /// Share `disable_emotion_mask` (`G/config.cpp:1059-1072`): the owner's `CONFIG` sets
    /// `EMOTION_MASK_REQUIRE: 0`, so emotions need no mask.
    ///
    /// Until this is called the mask is required, as legacy's compiled-in `false` has it.
    #[must_use]
    pub const fn with_emotion_mask_disabled(mut self, disabled: bool) -> Self {
        self.emotion_mask_disabled = disabled;
        self
    }

    /// `g_bDisableEmotionMask`.
    const fn emotion_mask_disabled(&self) -> bool {
        self.emotion_mask_disabled
    }

    /// `ACMD(do_emotion_allow)`: the arena gate, then the consent insert.
    ///
    /// No character of this build is on an arena (`sys.event.arena` is missing), so the
    /// gate always passes; the insert is `(caller's VID, strtoul(first word))`, and an
    /// empty line inserts nothing.
    pub(super) fn allow_emotion(&mut self, vid: Vid, rest: &[u8]) {
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            return;
        }
        let target = parse_allow_number(&word);
        self.emotion_consents.insert((vid.raw(), target));
    }

    /// `ACMD(do_emotion)` for the entry named `command`: the ride gate, the row, the mask
    /// gate, the woman gate, the victim in view, the NEED gates, the victim gates, the
    /// consent insert, and the command line to the view.
    ///
    /// No character of this build rides (`sys.char.horse` is missing) or is married
    /// (`sys.marriage` is missing), so the ride gates always pass and the marriage branch
    /// always refuses `[LS;481]` without the consent pair. Returns the number the answer
    /// line carries for the victim: its VID, or 0 with none.
    pub(super) fn run_emotion(
        &mut self,
        vid: Vid,
        command: &str,
        rest: &[u8],
        mover: Mover,
    ) -> u32 {
        let Some((_, flags)) = EMOTIONS.iter().find(|(name, _)| *name == command) else {
            return 0;
        };
        let flags = *flags;
        if !self.can_emote(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;414]", &[]);
            return 0;
        }
        if flags & WOMAN_ONLY != 0 && !self.is_female(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;425]", &[]);
            return 0;
        }
        let (word, _) = crate::command::one_argument(rest);
        let victim = if word.is_empty() {
            None
        } else {
            self.find_in_view(vid, &word, flags & NEED_PC != 0)
        };
        // `IS_SET(flag, NEED_TARGET | NEED_PC)`: `IS_SET` is a nonzero `&`, so either bit
        // arms it — but the refusal only tells `[LS;436]` and runs on.
        if flags & (NEED_TARGET | NEED_PC) != 0 && victim.is_none() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;436]", &[]);
        }
        let Some(victim) = victim else {
            self.send_emotion(vid, command, 0, mover);
            return 0;
        };
        if !self.is_player(victim) || victim == vid.raw() {
            return 0;
        }
        let Some(answer) = self.check_emotion_target(vid, victim, flags, mover) else {
            return 0;
        };
        if flags & NEED_PC != 0 {
            self.emotion_consents.insert((vid.raw(), victim));
        }
        self.send_emotion(vid, command, victim, mover);
        answer
    }

    /// `CHARACTER_CanEmotion` (`G/cmd_emotion.cpp:133-151`): the config switch, the wedding
    /// map, and the two worn masks.
    fn can_emote(&self, vid: Vid) -> bool {
        if self.emotion_mask_disabled() {
            return true;
        }
        if self
            .place_of(vid.raw())
            .is_some_and(|(_, map)| map == WEDDING_MAP_INDEX || map / 10_000 == WEDDING_MAP_INDEX)
        {
            return true;
        }
        self.wears_emotion_mask(vid)
    }

    /// Whether the character under `vid` wears an emotion mask unique in either unique
    /// cell (`IsEquipUniqueItem`, `G/char_item.cpp:9519-9540`).
    fn wears_emotion_mask(&self, vid: Vid) -> bool {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return false;
        };
        let equipment = Equipment::of(character.items(), &self.protos);
        [EWearPositions::Unique1, EWearPositions::Unique2]
            .into_iter()
            .filter_map(|wear| equipment.wear(wear as u16))
            .any(|worn| worn.item.vnum == EMOTION_MASK || worn.item.vnum == EMOTION_MASK2)
    }

    /// `GET_SEX(ch) != SEX_MALE`: races 1, 3, 4 and 6 are female.
    fn is_female(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(world::character::Character::points)
            .is_some_and(|points| is_female(points.race()))
    }

    /// Whether `victim` is a player character rather than an NPC entity.
    fn is_player(&self, victim: u32) -> bool {
        self.characters
            .find_by_vid(Vid::new(victim))
            .is_ok_and(|character| character.kind() == world::character::CharacterKind::Player)
    }

    /// `FindCharacterInView` (`G/char.cpp:1415-1434`): the first entity the caller sees
    /// whose name matches without regard to case, NPCs skipped when `pc_only`.
    fn find_in_view(&self, vid: Vid, name: &[u8], pc_only: bool) -> Option<u32> {
        let me = EntityKey::Character(vid.raw());
        let place = self.place_of(vid.raw())?;
        let index = self.maps.get(&place)?;
        let viewers = index.viewers(me)?;
        for viewer in viewers {
            match viewer {
                EntityKey::Character(other) => {
                    let Ok(character) = self.characters.find_by_vid(Vid::new(other)) else {
                        continue;
                    };
                    if character.name().as_bytes().eq_ignore_ascii_case(name) {
                        return Some(other);
                    }
                }
                EntityKey::Npc(other) => {
                    if pc_only {
                        continue;
                    }
                    if self
                        .npc_name(place, other)
                        .is_some_and(|npc| npc.eq_ignore_ascii_case(name))
                    {
                        return Some(other);
                    }
                }
                EntityKey::Ground(_) => {}
            }
        }
        None
    }

    /// The mob name of the NPC under `vid` on `place`, if it still stands there.
    fn npc_name(&self, place: (u8, i32), vid: u32) -> Option<&[u8]> {
        self.npcs
            .get(&place)?
            .npcs
            .iter()
            .find(|npc| npc.vid == vid)
            .map(|npc| npc.name.as_slice())
    }

    /// The victim gates: the 10..500 distance, the other sex, and the consent pair or
    /// marriage. Tells the refusal and returns `None` when one fires; returns the victim
    /// VID otherwise.
    fn check_emotion_target(&self, vid: Vid, victim: u32, flags: u32, mover: Mover) -> Option<u32> {
        let here = self.spot_of(vid.raw())?;
        let there = self.spot_of(victim).or_else(|| {
            let place = self.place_of(vid.raw())?;
            self.maps.get(&place)?.spot(EntityKey::Npc(victim))
        })?;
        let distance = distance_approx(here.x - there.x, here.y - there.y);
        if distance < 10 {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;448]", &[]);
            return None;
        }
        if distance > 500 {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;459]", &[]);
            return None;
        }
        if flags & OTHER_SEX_ONLY != 0 && self.is_female(vid) == self.is_female(Vid::new(victim)) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;470]", &[]);
            return None;
        }
        if flags & NEED_PC != 0 && !self.emotion_consents.contains(&(victim, vid.raw())) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;481]", &[]);
            return None;
        }
        Some(victim)
    }

    /// The `CHAT_TYPE_COMMAND` line `"<to_client> <vid> <victim>"` to the caller's view
    /// (`PacketAround`). The empire rides from the caller's mover, as its own line does.
    fn send_emotion(&self, vid: Vid, command: &str, victim: u32, mover: Mover) {
        let text = format!("{command} {} {victim}", vid.raw());
        let mut bytes = text.into_bytes();
        bytes.truncate(CHAT_MAX_LEN as usize);
        let record = chat_packet(
            mover.recipient(&self.locale),
            CHAT_TYPE_COMMAND,
            &bytes,
            &[],
        );
        self.packet_around(EntityKey::Character(vid.raw()), &record, None);
    }
}

/// The consent pairs `s_emotion_set` holds (`G/cmd_emotion.cpp:110`).
pub(super) type EmotionConsents = BTreeSet<(u32, u32)>;

#[cfg(test)]
mod tests {
    use super::parse_allow_number;

    /// `strtoul`'s walk: the sign, the digit run, the low word, and the no-digit zero.
    #[test]
    fn the_allow_number_reads_strtoul_low_word() {
        assert_eq!(parse_allow_number(b"123"), 123);
        assert_eq!(parse_allow_number(b"+123abc"), 123);
        assert_eq!(parse_allow_number(b"  123"), 0);
        assert_eq!(parse_allow_number(b"-1"), u32::MAX);
        assert_eq!(parse_allow_number(b"4294967296"), 0);
        assert_eq!(parse_allow_number(b"no digits"), 0);
        assert_eq!(parse_allow_number(b""), 0);
    }

    /// The table holds every registered `do_emotion` entry, and only those.
    #[test]
    fn the_table_names_every_emotion_entry() {
        let names: Vec<&str> = super::EMOTIONS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names.len(), 39);
        assert!(names.contains(&"kiss"));
        assert!(names.contains(&"whirl"));
        assert!(!names.contains(&"emotion_allow"));
        assert!(!names.contains(&"dungeon_info"));
    }
}
