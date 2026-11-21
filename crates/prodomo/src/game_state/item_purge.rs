//! The GM's item lines: `/slow` and `/ipurge` (`ACMD(do_slow)`, `do_item_purge`,
//! `G/cmd_gm.cpp:92-100`, `:868-1016`).
//!
//! Both run as item steps through [`GameState::run_item_step`], so the quickslots sync,
//! the points and the look are stored, and the moved rows reach the store. `slow` needs
//! no gear: armour goes into cells, never on. `ipurge` needs it: worn pieces leave
//! through the unequip point path, the points recompute twice, and the weapon part
//! resets with the look.

use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use world::character::{book_give, purge_all, slow_give, MoveDone, MoveKind};

use super::GameState;
use crate::command::one_argument;
use crate::game_state::skill_admin::str_to_dword;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

impl GameState {
    /// Run `/slow` for the character online under `vid`: ten fresh 12019s, numbered in
    /// socket 4, or nothing when nobody is online there.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn slow(&mut self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |items, ids, rules, _gear, protos| {
            Ok(slow_give(items, ids, rules.usable_cells, protos))
        })
    }

    /// Run `/book` for the character online under `vid`: one fresh 50300 with the named
    /// skill's vnum in socket 0, or "There is no such a skill" when the word names none.
    /// The word is a vnum when it starts with a digit (`str_to_dword`), else a
    /// case-insensitive skill name (`CSkillManager::Get`); an empty word names nothing.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn book(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let (word, _) = one_argument(rest);
        let Some(skill) = self.book_skill(&word) else {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"There is no such a skill", &[]);
            return self.run_item_step(vid, mover, |_items, _ids, _rules, _gear, _protos| {
                Ok(MoveDone {
                    kind: MoveKind::Given,
                    records: Vec::new(),
                    changes: Vec::new(),
                })
            });
        };
        self.run_item_step(vid, mover, |items, ids, rules, _gear, protos| {
            Ok(book_give(items, ids, rules.usable_cells, protos, skill))
        })
    }

    /// `CSkillManager::Get` for `/book`'s word: the vnum when it starts with an ASCII
    /// digit (`isnhdigit` for ASCII, `str_to_dword` mod 2^32), else the name's lowest
    /// vnum. A vnum no row carries, or a name none bears, is no skill. Unlike
    /// `/setskill` there is no 254 cap: `Get(vnum)` reads the map whole.
    fn book_skill(&self, word: &[u8]) -> Option<u32> {
        if word.first().is_some_and(u8::is_ascii_digit) {
            let vnum = str_to_dword(word);
            return self.skill_names.contains(vnum).then_some(vnum);
        }
        self.skill_names.by_name(word)
    }

    /// Run `/console` for the character online under `vid`: the `ConsoleEnable`
    /// command line.
    pub(super) fn run_console(&self, vid: Vid) {
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        self.tell(vid, mover, CHAT_TYPE_COMMAND, b"ConsoleEnable", &[]);
    }

    /// Run `/ipurge` for the character online under `vid`: every flat and dragon soul
    /// cell cleared, the points recomputed either side, the weapon part reset with the
    /// look — or nothing when nobody is online there.
    ///
    /// # Errors
    ///
    /// As [`GameState::move_item`].
    pub fn item_purge(&mut self, vid: Vid, actor: Mover) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |items, _ids, _rules, gear, _protos| {
            let Some(gear) = gear else {
                return Err(world::character::MoveRefused::NotPorted(
                    world::character::Unported::Equipment,
                ));
            };
            Ok(purge_all(items, gear))
        })
    }
}
