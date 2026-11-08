//! The monarch treasuries: `TMonarchInfo.money` (`C/tables.h:1574-1580`) and the two money
//! lines, `/temp` (`do_temp`, `G/cmd_gm.cpp:4368-4388`) and `/check_mmoney`
//! (`do_check_monarch_money`, `G/cmd_gm.cpp:4299-4313`).
//!
//! The four cells arrive from the store before the gate opens and live on the world. `/temp`
//! runs only in the test-server mode; a line without two words answers `Usage: empire
//! money`, and then the money moves when the cell stays within 0 to 2000000000, as the DB
//! server's guards do (`D/Monarch.cpp:122-147`), silently either way. `/check_mmoney` with
//! a word tells the cell, 0 for a slot past 3 (`CMonarch::GetMoney`, `G/monarch.cpp:175-182`);
//! an empty line is silent.
//!
//! One legacy Defect is not reproduced. The game's send guard reads the empire through a
//! `BYTE` (`SendtoDBAddMoney(int, BYTE, LPCHARACTER)`, `G/monarch.cpp:118-132`) but sends
//! the full `int` on, and the DB server indexes `m_MonarchInfo.money` with it unchecked
//! (`D/Monarch.cpp:124`), so `/temp 5 1` writes past the array. The Rewrite refuses an
//! empire outside 0 to 3.
//!
//! Not ported: the monarch identities, the elections, and every other monarch line; they
//! wait for `sys.event.castle_monarch`.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::two_arguments;
use crate::item_move::Mover;

/// The treasuries the store loaded, by empire slot.
pub type InstalledMonarchMoney = [i64; 4];

/// `____: %d __`: `/check_mmoney`'s format (`G/cmd_gm.cpp:4312`), byte-exact from the
/// frozen source, whose Korean the file holds as `U+FFFD` replacements.
const CHECK_MONEY_FORMAT: &[u8] = &[
    0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, b':', b' ', b'%', b'd',
    b' ', 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd,
];

impl GameState {
    /// Install the stored treasuries, once, before the gate opens.
    pub(super) fn install_monarch_money(&mut self, money: InstalledMonarchMoney) {
        self.monarch_money = money;
    }

    /// `CMonarch::GetMoney`: the cell as an `int`, 0 for a slot past 3.
    fn monarch_money_of(&self, empire: u8) -> i32 {
        self.monarch_money
            .get(usize::from(empire))
            .copied()
            .unwrap_or(0)
            .try_into()
            .unwrap_or(i32::MAX)
    }

    /// `ACMD(do_temp)`: outside the test-server mode, or with a word missing, the
    /// usage or nothing answers, as above; else the money moves when the cell stays in
    /// range. Answers the slot with the landed cell for the store, or `None` for a line
    /// that moves nothing.
    pub(super) fn run_temp(&mut self, vid: Vid, rest: &[u8]) -> Option<(u8, i64)> {
        if !self.test_server {
            return None;
        }
        let mover = self.own_mover(vid)?;
        let (first, second) = two_arguments(rest);
        if first.is_empty() || second.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: empire money", &[]);
            return None;
        }
        let empire = super::gm_lines::int_word(&first).unwrap_or(0);
        let money = i64::from(super::gm_lines::int_word(&second).unwrap_or(0));
        let Ok(slot) = u8::try_from(empire) else {
            return None;
        };
        let cell = self.monarch_money.get(usize::from(slot)).copied()?;
        let landed = cell + money;
        if !(0..=2_000_000_000).contains(&landed) {
            return None;
        }
        self.monarch_money[usize::from(slot)] = landed;
        Some((slot, landed))
    }

    /// `ACMD(do_check_monarch_money)`: an empty line is silent; else the caller hears
    /// the cell.
    pub(super) fn run_check_mmoney(&self, vid: Vid, rest: &[u8], mover: Mover) {
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            return;
        }
        let empire = super::gm_lines::int_word(&word).unwrap_or(0);
        let shown = u8::try_from(empire)
            .ok()
            .map_or(0, |slot| self.monarch_money_of(slot));
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            CHECK_MONEY_FORMAT,
            &[Arg::Int(i64::from(shown))],
        );
    }
}
