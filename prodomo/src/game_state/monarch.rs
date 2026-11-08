//! The monarch cells: `TMonarchInfo` (`C/tables.h:1574-1580`) minus the windate, with the
//! money lines, the identity lines and the powers that spend.
//!
//! The four treasury cells arrive from the store before the gate opens and live on the
//! world. `/temp` runs only in the test-server mode; a line without two words answers
//! `Usage: empire money`, and then the money moves when the cell stays within 0 to
//! 2000000000, as the DB server's guards do (`D/Monarch.cpp:122-147`), silently either
//! way. `/check_mmoney` with a word tells the cell, 0 for a slot past 3
//! (`CMonarch::GetMoney`, `G/monarch.cpp:175-182`); an empty line is silent.
//!
//! The four identity cells (`pid`, the stored name read back through `player`, as
//! `LoadMonarch` joins it) arrive the same way. `/setmonarch <name>` names the empire of
//! the name's account the monarch and refills the treasury from the monarch's gold
//! (`D/Monarch.cpp:208-246`); `/rmmonarch <name>` clears the caller's empire only when
//! the name is its monarch, else the empire's gold spenders keep their monarch.
//! `/minfo` tells the three names with the `[LS;560-563]` heads, the money only to a
//! caller who is its empire's monarch. `/mnotice` speaks to the caller's empire when the
//! caller is its monarch, else `#ERR_CODE: 1432030623`. `/mto`, `/mtr`, `/mtax` and
//! `/mmob` charge their price off the treasury after the monarch, cooldown, money and
//! reason checks. The cooldowns are the caller's own: 60 s for the warps, a week for
//! the tax, an hour for the summon (`CHARACTER::MC_*`, `G/char.h:1965-1971`).
//!
//! One legacy Defect is not reproduced. The game's send guard reads the empire through a
//! `BYTE` (`SendtoDBAddMoney(int, BYTE, LPCHARACTER)`, `G/monarch.cpp:118-132`) but sends
//! the full `int` on, and the DB server indexes `m_MonarchInfo.money` with it unchecked
//! (`D/Monarch.cpp:124`), so `/temp 5 1` writes past the array. The Rewrite refuses an
//! empire outside 0 to 3.
//!
//! Not ported: `/siege` and the `windate`; they wait for `sys.event.castle_monarch`.
//! `/elect` and `/rmcandidacy` are `unused`: the DB server's election tally reads
//! nothing (`CMonarch::ElectMonarch` fills `s` and deletes it, `D/Monarch.cpp:39-63`)
//! and the candidacy answers (`HEADER_DG_RMCANDIDACY`, `HEADER_DG_ELECT_MONARCH`,
//! `HEADER_DG_COME_TO_VOTE`) have no game-side case, so no client observes anything.

use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_INFO;
use tracing::warn;

use super::GameState;
use crate::chat_line::Arg;
use crate::command::two_arguments;
use crate::item_move::Mover;
use world::character::number;

/// The treasuries the store loaded, by empire slot.
pub type InstalledMonarchMoney = [i64; 4];

/// The monarch identities the store loaded: the pid and the name through `player`,
/// by empire slot. A pid of 0 is no monarch.
pub type InstalledMonarchIdentity = [(u32, Vec<u8>); 4];

/// What `/setmonarch` names: the empire, the name's pid, and the gold the treasury
/// refills from, which the store's candidate lookup read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetMonarchLanded {
    /// The name's account's empire.
    pub empire: u8,
    /// The name's pid.
    pub pid: u32,
    /// The refilled treasury.
    pub money: i64,
}

/// The powers with a cooldown (`CHARACTER::MONARCH_INDEX`, `G/char.h:1974-1981`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonarchPower {
    /// `MI_WARP`: `/mto`.
    Warp,
    /// `MI_TRANSFER`: `/mtr`.
    Transfer,
    /// `MI_TAX`: `/mtax`.
    Tax,
    /// `MI_SUMMON`: `/mmob`.
    Summon,
}

/// `____: %d __`: `/check_mmoney`'s format (`G/cmd_gm.cpp:4312`), byte-exact from the
/// frozen source, whose Korean the file holds as `U+FFFD` replacements.
const CHECK_MONEY_FORMAT: &[u8] = &[
    0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd, b':', b' ', b'%', b'd',
    b' ', 0xef, 0xbf, 0xbd, 0xef, 0xbf, 0xbd,
];
/// `c_apszEmpireNames` (`G/constants.cpp:1072-1078`): the info's empire word, by slot.
const EMPIRE_WORDS: [&[u8]; 4] = [b"[LS;1037]", b"[LS;793]", b"[LS;794]", b"[LS;795]"];
/// `cs_dwMonarchMobVnums` (`G/cmd_general.cpp:2836-2860`): the vnums `/mmob` summons.
const MONARCH_MOB_VNUMS: [u32; 20] = [
    191, 192, 193, 194, 391, 392, 393, 394, 491, 492, 493, 494, 591, 691, 791, 1304, 1901, 2091,
    2191, 2206,
];

impl GameState {
    /// Install the stored treasuries, once, before the gate opens.
    pub(super) fn install_monarch_money(&mut self, money: InstalledMonarchMoney) {
        self.monarch_money = money;
    }

    /// Install the stored identities, once, before the gate opens.
    pub(super) fn install_monarch_identity(&mut self, identity: InstalledMonarchIdentity) {
        let (pids, names): (Vec<u32>, Vec<Vec<u8>>) = identity.into_iter().unzip();
        self.monarch_pids = pids.try_into().unwrap_or([0; 4]);
        self.monarch_names = names.try_into().unwrap_or_default();
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
    /// `CMonarch::IsMonarch`: the slot's pid is the caller's.
    fn is_monarch(&self, pid: u32, empire: u8) -> bool {
        pid != 0
            && self
                .monarch_pids
                .get(usize::from(empire))
                .is_some_and(|cell| *cell == pid)
    }

    /// `ACMD(do_setmonarch)`: a missing word answers the usage; a name the store
    /// missed clears nothing and answers nothing, as legacy's failed `SetMonarch`
    /// does; else the empire names its monarch and refills from the gold. Answers
    /// what the line named for the store.
    pub(super) fn run_setmonarch(
        &mut self,
        vid: Vid,
        rest: &[u8],
        candidate: Option<(u8, u32, i64)>,
    ) -> Option<SetMonarchLanded> {
        let mover = self.own_mover(vid)?;
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: setmonarch <name>", &[]);
            return None;
        }
        let (empire, pid, gold) = candidate?;
        let slot = usize::from(empire);
        if slot >= self.monarch_pids.len() || slot == 0 {
            return None;
        }
        self.monarch_pids[slot] = pid;
        self.monarch_names[slot].clone_from(&word);
        let money = gold.clamp(0, 2_000_000_000);
        self.monarch_money[slot] = money;
        Some(SetMonarchLanded { empire, pid, money })
    }

    /// `ACMD(do_rmmonarch)`: a missing word answers the usage; the name clears the
    /// caller's empire only when it is its monarch. Answers the cleared empire for
    /// the store.
    pub(super) fn run_rmmonarch(&mut self, vid: Vid, rest: &[u8]) -> Option<u8> {
        let mover = self.own_mover(vid)?;
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: rmmonarch <name>", &[]);
            return None;
        }
        let empire = mover.empire;
        let slot = usize::from(empire);
        let cell = self.monarch_names.get(slot)?;
        if cell.is_empty() || !cell.eq_ignore_ascii_case(&word) {
            return None;
        }
        self.monarch_pids[slot] = 0;
        self.monarch_names[slot].clear();
        self.monarch_money[slot] = 0;
        Some(empire)
    }
    /// The cooldown's seconds (`CHARACTER::MONARCH_COOLTIME`, `G/char.h:1965-1971`).
    const fn power_seconds(power: MonarchPower) -> u64 {
        match power {
            MonarchPower::Warp | MonarchPower::Transfer => 60,
            MonarchPower::Tax => 60 * 60 * 24 * 7,
            MonarchPower::Summon => 60 * 60,
        }
    }

    /// The cooldown slot (`CHARACTER::MONARCH_INDEX`, `G/char.h:1974-1981`).
    const fn power_slot(power: MonarchPower) -> usize {
        match power {
            MonarchPower::Warp => 1,
            MonarchPower::Transfer => 2,
            MonarchPower::Tax => 3,
            MonarchPower::Summon => 4,
        }
    }

    /// `CHARACTER::IsMCOK` with `GetMCLTime`'s telling: `None` when the power may
    /// fire, else the seconds the `[LS;551;%d]` names.
    fn monarch_wait(&self, vid: Vid, power: MonarchPower) -> Option<u64> {
        let limit = Self::power_seconds(power) * u64::from(crate::save::PASSES_PER_SEC);
        let fired = self
            .monarch_cooldowns
            .get(&vid)
            .map(|set| set[Self::power_slot(power)]);
        // Wrapping, as legacy's `DWORD` math does: the seed sits a full limit in the
        // past even when that wraps below 0.
        let elapsed = self.last_pulse.wrapping_sub(fired.unwrap_or(0));
        if fired.is_none() || elapsed >= limit {
            return None;
        }
        let per = u64::from(crate::save::PASSES_PER_SEC);
        Some(limit / per - elapsed / per)
    }

    /// `CHARACTER::SetMC`: stamp the power's slot with this Pulse. A new set is
    /// seeded like `InitMC` (`G/char.cpp:8331-8345`): every slot a full limit in
    /// the past, so a power never fired reads as ready.
    fn stamp_monarch(&mut self, vid: Vid, power: MonarchPower) {
        let pulse = self.last_pulse;
        let slot = Self::power_slot(power);
        let per = u64::from(crate::save::PASSES_PER_SEC);
        self.monarch_cooldowns.entry(vid).or_insert_with(|| {
            let mut set = [pulse; 5];
            for (slot, seconds) in [(1, 60), (2, 60), (3, 60 * 60 * 24 * 7), (4, 60 * 60)] {
                set[slot] = pulse.wrapping_sub(seconds * per);
            }
            set
        })[slot] = pulse;
    }

    /// `IsMonarchWarpZone` (`G/monarch.cpp:255-272`): the battlefields and 351, 352,
    /// 208, 216 and 217 refuse, a private map by its base.
    fn is_monarch_warp_zone(map: i32) -> bool {
        let base = if map >= 10000 { map / 10000 } else { map };
        !matches!(base, 301 | 302 | 303 | 304 | 351 | 352 | 208 | 216 | 217)
    }

    /// `GetCenterPositionOfMap` (`G/sectree_manager.cpp:535-553`): the hosted
    /// region's centre, or `None` when the map has no region.
    fn monarch_map_center(&self, channel: u8, map: i32) -> Option<(i32, i32)> {
        let region = self.regens.region_of(channel, map)?;
        Some((
            region.sx + (region.ex - region.sx) / 2,
            region.sy + (region.ey - region.sy) / 2,
        ))
    }

    /// The VID of the online character the name names (`FindPC`,
    /// `G/char_manager.cpp:244-259`), or `None`.
    fn monarch_find(&self, name: &[u8]) -> Option<Vid> {
        let text = std::str::from_utf8(name).ok()?;
        self.characters
            .find_player_by_name(text)
            .ok()
            .map(world::character::Character::vid)
    }

    /// The spend's store row: the pid, the empty rows, and the slot with the landed
    /// cell.
    fn monarch_spend_rows(
        &self,
        vid: Vid,
        empire: u8,
        money: i64,
    ) -> Option<super::gm_lines::GmRows> {
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        Some(super::gm_lines::GmRows {
            pid,
            changes: Vec::new(),
            cleared_skills: Vec::new(),
            gold: 0,
            monarch: Some((empire, money)),
            points: None,
            quickslots: None,
        })
    }
    /// The price a warp or a summon charges before it fires.
    const fn power_price(power: MonarchPower) -> i64 {
        match power {
            MonarchPower::Warp | MonarchPower::Transfer => 10_000,
            MonarchPower::Tax => 0,
            MonarchPower::Summon => 5_000_000,
        }
    }

    /// The monarch, cooldown and money gates a power shares: the `[LS;1007]`,
    /// `[LS;551;%d]` and `[LS;552;%u;%u]`, in legacy's order. Answers the empire
    /// when the power may fire.
    fn monarch_power_gates(&self, vid: Vid, mover: Mover, power: MonarchPower) -> Option<u8> {
        let pid = self.characters.find_by_vid(vid).ok()?.player_id();
        let empire = mover.empire;
        if !self.is_monarch(pid, empire) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1007]", &[]);
            return None;
        }
        if let Some(wait) = self.monarch_wait(vid, power) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;551;%d]",
                &[Arg::Int(i64::try_from(wait).unwrap_or(i64::MAX))],
            );
            return None;
        }
        let price = Self::power_price(power);
        if i64::from(self.monarch_money_of(empire)) < price {
            let held = i64::from(self.monarch_money_of(empire));
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;552;%u;%u]",
                &[Arg::Int(held), Arg::Int(price)],
            );
            return None;
        }
        Some(empire)
    }

    /// Charge the price off the treasury and stamp the power. The gates held, so
    /// the cell stays in range.
    fn monarch_charge(&mut self, vid: Vid, empire: u8, power: MonarchPower) -> i64 {
        let slot = usize::from(empire);
        let money = self.monarch_money[slot] - Self::power_price(power);
        self.monarch_money[slot] = money;
        self.stamp_monarch(vid, power);
        money
    }

    /// `ACMD(do_monarch_warpto)`: the gates, then the name, then the target's
    /// empire and zone; the monarch warps to the target at 10000. A target on
    /// another Channel warps to that map's centre, as its position never
    /// crosses cores. Answers the spend for the store.
    pub(super) fn run_monarch_warp(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        let empire = self.monarch_power_gates(vid, mover, MonarchPower::Warp)?;
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;553]", &[]);
            return None;
        }
        let Some(target) = self.monarch_find(&word) else {
            let name = std::str::from_utf8(&word).unwrap_or("");
            let entry = self
                .clients
                .as_ref()
                .and_then(|clients| clients.entry_of(name));
            let Some((_, entry, _)) = entry else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"There is no one by that name",
                    &[],
                );
                return None;
            };
            if entry.empire != empire {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;554]", &[]);
                return None;
            }
            let (channel, _) = self.place_of(vid.raw())?;
            if entry.channel != channel {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;555;%d;%d]",
                    &[
                        Arg::Int(i64::from(entry.channel)),
                        Arg::Int(i64::from(channel)),
                    ],
                );
                return None;
            }
            if !Self::is_monarch_warp_zone(entry.map) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[#Unk]Cannot be summoned to that area.",
                    &[],
                );
                return None;
            }
            let Some(center) = self.monarch_map_center(entry.channel, entry.map) else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"Cannot find map (index %d)",
                    &[Arg::Int(i64::from(entry.map))],
                );
                return None;
            };
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;1010;%s]",
                &[Arg::Text(&word)],
            );
            self.order_warp(vid, center);
            let money = self.monarch_charge(vid, empire, MonarchPower::Warp);
            return self.monarch_spend_rows(vid, empire, money);
        };
        let target_mover = self.own_mover(target)?;
        if target_mover.empire != empire {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;554]", &[]);
            return None;
        }
        let (_, map) = self.place_of(target.raw())?;
        let spot = self.spot_of(target.raw())?;
        if !Self::is_monarch_warp_zone(map) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[#Unk]Can't go to that area",
                &[],
            );
            return None;
        }
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"[LS;1010;%s]",
            &[Arg::Text(&word)],
        );
        self.order_warp(vid, (spot.x, spot.y));
        self.stop(vid.raw());
        let money = self.monarch_charge(vid, empire, MonarchPower::Warp);
        self.monarch_spend_rows(vid, empire, money)
    }
    /// `ACMD(do_monarch_transfer)`: the empty word's `[LS;1006]` comes before the
    /// gates; then the monarch, cooldown and money gates, the self-summon's
    /// `[LS;1012]`, and the target's empire and zone. The target warps to the
    /// monarch at 10000. A cross-Channel target's position never crosses cores,
    /// so that branch only names the miss. Answers the spend for the store.
    pub(super) fn run_monarch_transfer(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1006]", &[]);
            return None;
        }
        let empire = self.monarch_power_gates(vid, mover, MonarchPower::Transfer)?;
        let Some(target) = self.monarch_find(&word) else {
            let name = std::str::from_utf8(&word).unwrap_or("");
            let entry = self
                .clients
                .as_ref()
                .and_then(|clients| clients.entry_of(name));
            let Some((_, entry, _)) = entry else {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1011]", &[]);
                return None;
            };
            if entry.empire != empire {
                self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1008]", &[]);
                return None;
            }
            let (channel, _) = self.place_of(vid.raw())?;
            if entry.channel != channel {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;1009;%s;%d;%d]",
                    &[
                        Arg::Text(&word),
                        Arg::Int(i64::from(entry.channel)),
                        Arg::Int(i64::from(channel)),
                    ],
                );
                return None;
            }
            if !Self::is_monarch_warp_zone(entry.map) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[#Unk]Can't go to that area",
                    &[],
                );
                return None;
            }
            let (_, map) = self.place_of(vid.raw())?;
            if !Self::is_monarch_warp_zone(map) {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[#Unk]Cannot be summoned to that area.",
                    &[],
                );
                return None;
            }
            // The GG transfer carries the monarch's position to the target's core;
            // with no such peer the line has nowhere to send it, so it stays silent
            // after the checks, as a GG send with no peer does.
            return None;
        };
        if target == vid {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1012]", &[]);
            return None;
        }
        let target_mover = self.own_mover(target)?;
        if target_mover.empire != empire {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;1008]", &[]);
            return None;
        }
        let (_, target_map) = self.place_of(target.raw())?;
        if !Self::is_monarch_warp_zone(target_map) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[#Unk]Can't go to that area",
                &[],
            );
            return None;
        }
        let (_, map) = self.place_of(vid.raw())?;
        if !Self::is_monarch_warp_zone(map) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[#Unk]Cannot be summoned to that area.",
                &[],
            );
            return None;
        }
        let spot = self.spot_of(vid.raw())?;
        self.order_warp(target, (spot.x, spot.y));
        let money = self.monarch_charge(vid, empire, MonarchPower::Transfer);
        self.monarch_spend_rows(vid, empire, money)
    }
    /// `ACMD(do_monarch_info)`: the `[LS;560]` head for a monarch and `[LS;563]`
    /// for anyone else, then the three empires' `[LS;561;%s;%s;%lld]` or
    /// `[LS;562;%s;%s]`: the money only to a caller who is its empire's monarch.
    pub(super) fn run_monarch_info(&self, vid: Vid, mover: Mover) {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        let own = self.is_monarch(pid, mover.empire);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            if own { b"[LS;560]" } else { b"[LS;563]" },
            &[],
        );
        for empire in 1..=3u8 {
            let slot = usize::from(empire);
            let name: &[u8] = &self.monarch_names[slot];
            let nation = self.locale.find(EMPIRE_WORDS[slot], mover.language);
            if own && empire == mover.empire {
                let money = self.monarch_money[slot];
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;561;%s;%s;%lld]",
                    &[Arg::Text(nation), Arg::Text(name), Arg::Int(money)],
                );
            } else {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"[LS;562;%s;%s]",
                    &[Arg::Text(nation), Arg::Text(name)],
                );
            }
        }
    }
    /// `ACMD(do_monarch_tax)`: the empty word's usage, the `[LS;564]` for a
    /// non-monarch, the `[LS;565]` for a tax outside 1 to 50 — which legacy
    /// tells and then ignores, so the flag takes the tax anyway — and the
    /// `[LS;375;%d]` with the two broadcasts. Stamps the week's cooldown.
    /// Nothing answers for the store: the flag is memory-only.
    pub(super) fn run_monarch_tax(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Usage: monarch_tax <1-50>",
                &[],
            );
            return None;
        }
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if !self.is_monarch(pid, mover.empire) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;564]", &[]);
            return None;
        }
        let tax = super::gm_lines::int_word(&word).unwrap_or(0);
        if !(1..=50).contains(&tax) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;565]", &[]);
        }
        self.set_event_flag(b"trade_tax", tax);
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"[LS;375;%d]",
            &[Arg::Int(i64::from(tax))],
        );
        let first = format!("In the name of the monarch, the tax has been changed to {tax}%");
        self.broadcast_notice_text(first.as_bytes());
        let second = format!(
            "In the future, {tax}% of the transaction amount will go into the national treasury."
        );
        self.broadcast_notice_text(second.as_bytes());
        self.stamp_monarch(vid, MonarchPower::Tax);
        None
    }

    /// `ACMD(do_monarch_notice)`: the monarch's whole line after the word goes
    /// to its empire as a notice; anyone else hears `#ERR_CODE: 1432030623`.
    pub(super) fn run_monarch_notice(&self, vid: Vid, rest: &[u8], mover: Mover) {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if !self.is_monarch(pid, mover.empire) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"#ERR_CODE: 1432030623", &[]);
            return;
        }
        let mut vids: Vec<Vid> = self.bodies.keys().copied().collect();
        vids.sort_by_key(|entry| entry.raw());
        for heard in vids {
            let Some(listen) = self.own_mover(heard) else {
                continue;
            };
            if listen.empire == mover.empire {
                self.tell(
                    heard,
                    listen,
                    protocol::gc_chat::CHAT_TYPE_NOTICE,
                    b"%s",
                    &[Arg::Text(rest)],
                );
            }
        }
    }
    /// `ACMD(do_monarch_mob)`: the `[LS;564]` for a non-monarch, the usage for
    /// the empty word, the cooldown's `[LS;551;%d]`, the money's `[LS;552;%u;%u]`,
    /// the `[LS;567]` for a name off the summon list — a vnum missing its proto
    /// reads as 0, which the list never holds — and the summon at 5000000 with
    /// the aggro bit and the stone's spawn motion. Answers the spend for the store.
    pub(super) fn run_monarch_mob(
        &mut self,
        vid: Vid,
        rest: &[u8],
        mover: Mover,
    ) -> Option<super::gm_lines::GmRows> {
        let pid = self
            .characters
            .find_by_vid(vid)
            .map_or(0, world::character::Character::player_id);
        if !self.is_monarch(pid, mover.empire) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;564]", &[]);
            return None;
        }
        let (word, _) = crate::command::one_argument(rest);
        if word.is_empty() {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Usage: mmob <mob name>", &[]);
            return None;
        }
        if let Some(wait) = self.monarch_wait(vid, MonarchPower::Summon) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;551;%d]",
                &[Arg::Int(i64::try_from(wait).unwrap_or(i64::MAX))],
            );
            return None;
        }
        let held = i64::from(self.monarch_money_of(mover.empire));
        if held < Self::power_price(MonarchPower::Summon) {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"[LS;552;%u;%u]",
                &[
                    Arg::Int(held),
                    Arg::Int(Self::power_price(MonarchPower::Summon)),
                ],
            );
            return None;
        }
        let respawner = self.regens.respawner()?;
        let (vnum, stone) = if word[0].is_ascii_digit() {
            let vnum = Self::monarch_vnum(&word);
            match respawner.mob(vnum) {
                Some(row) => (vnum, row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE),
                None => (0, false),
            }
        } else {
            match respawner.mob_by_name(&word) {
                Some(row) => (
                    row.vnum,
                    row.mob_type == gamedata::mob_proto::CHAR_TYPE_STONE,
                ),
                None => (0, false),
            }
        };
        if !MONARCH_MOB_VNUMS.contains(&vnum) {
            self.tell(vid, mover, CHAT_TYPE_INFO, b"[LS;567]", &[]);
            return None;
        }
        let place = self.place_of(vid.raw())?;
        let spot = self.spot_of(vid.raw())?;
        let pulse = self.last_pulse;
        let (low_x, low_y) = (
            spot.x.saturating_sub(number(&mut self.dice, 200, 750)),
            spot.y.saturating_sub(number(&mut self.dice, 200, 750)),
        );
        let (high_x, high_y) = (
            spot.x.saturating_add(number(&mut self.dice, 200, 750)),
            spot.y.saturating_add(number(&mut self.dice, 200, 750)),
        );
        let (Some(region), Some(cells), Some(respawner)) = (
            self.regens.region_of(place.0, place.1),
            self.regens.cells_of(place.1),
            self.regens.respawner_mut(),
        ) else {
            return None;
        };
        match respawner.spawn_mob_line(
            (&region, &cells),
            vnum,
            (low_x, low_y, high_x, high_y),
            stone,
            &mut self.dice,
        ) {
            Ok(Some(mut stood)) => {
                stood.ai_flags |= gamedata::mob_proto::AIFLAG_AGGRESSIVE;
                self.stand_runtime(place, vec![stood], pulse);
            }
            Ok(None) => return None,
            Err(error) => {
                warn!(?place, %error, "a mmob line outnumbered the VIDs left");
                return None;
            }
        }
        let money = self.monarch_charge(vid, mover.empire, MonarchPower::Summon);
        self.monarch_spend_rows(vid, mover.empire, money)
    }

    /// `str_to_number` on a `DWORD`: the leading digits, garbage answering 0.
    fn monarch_vnum(word: &[u8]) -> u32 {
        let mut value: u32 = 0;
        for byte in word {
            if !byte.is_ascii_digit() {
                break;
            }
            value = value
                .saturating_mul(10)
                .saturating_add(u32::from(byte - b'0'));
        }
        value
    }
}
