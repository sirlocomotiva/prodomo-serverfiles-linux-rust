//! The HWID multi-farm block (`ENABLE_MULTI_FARM_BLOCK`): `/multi_farm` and the
//! per-address registry (`CHARACTER_MANAGER::m_mapmultiFarm`, `G/char_manager.h:151`).
//!
//! The command toggles the caller's farm status with a 10 s protect (`do_multi_farm`,
//! `G/cmd_general.cpp:304-317`): too soon answers the slow line. The toggle runs
//! `CheckMultiFarmAccount` (`G/char_manager.cpp:1250-1305`): the address's gone rows
//! prune, the premium affect lends its type and time, two farming accounts refuse a
//! third by recursing with `false`, the row sets, the caller's status sets, and every
//! listed character hears `UpdateMultiFarmAffect` plus one `UpdateMultiFarmPlayer` per
//! farming row.
//!
//! The P2P fan-out has no Rewrite equivalent: one process holds every Channel
//! (ADR-0002), so the registry is already global. The status starts `false`
//! (`G/char.cpp:372`): a fresh character neither digs, fishes, nor drops until it
//! toggles on. `RemoveMultiFarm` needs no logout hook: legacy only calls it from P2P
//! input (`G/input_p2p.cpp:758`), and disconnects prune on the next check.
//!
//! `G/` = `server/server/game/`.

use common::vid::Vid;

use protocol::item_pos::ItemPos;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};
use crate::loading_phase::AFFECT_MULTI_FARM_PREMIUM;

use super::affect::Affect;

/// A `TMultiFarm` row (`C/tables.h:1726-1737`): one character's farm status on its
/// address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MultiFarmRow {
    /// `playerID`.
    pid: u32,
    /// `playerName`.
    name: String,
    /// `farmStatus`.
    farming: bool,
    /// `affectType`: the premium's apply value.
    affect_type: u8,
    /// `affectTime`: the `time(0)` the premium ends at, or 0 for none.
    affect_time: i32,
}

/// `do_multi_farm` (`G/cmd_general.cpp:304-317`): the caller's farm status toggles past
/// its 10 s protect, or the slow line answers.
pub(super) fn run_multi_farm(state: &mut GameState, vid: Vid, addr: &str) {
    let now = super::quests::quest_now();
    if let Some(until) = state.multi_protect_of(vid) {
        if until > now {
            let wait = until.saturating_sub(now);
            if let Some(mover) = state.own_mover(vid) {
                state.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_INFO,
                    b"@@(cmd_general)tradus:You need be slow! You can try after %d second.",
                    &[Arg::Int(i64::from(wait))],
                );
            }
            return;
        }
    }
    state.set_multi_protect(vid, now.saturating_add(10));
    let farming = !state.multi_status_of(vid);
    state.check_multi_farm_account(addr, vid, farming);
}

/// Whether the item at `at` of the character under `vid` is a multi-farm premium
/// (vnum 55610-55615, `G/char_item.cpp:3901-3906`).
#[must_use]
pub(super) fn premium_at(state: &GameState, vid: Vid, at: ItemPos) -> bool {
    state
        .characters
        .find_by_vid(vid)
        .ok()
        .and_then(|character| character.items().item_at(at))
        .is_some_and(|item| (55_610..=55_615).contains(&item.vnum))
}

/// `CG_ITEM_USE` of a multi-farm premium (`G/char_item.cpp:3901-3923`): an already-held
/// premium refuses with the line and the item kept; else `AddAffect(998, POINT_NONE,
/// value1, AFF_NONE, value0, 0, false, false)` lands, one item goes, the registry
/// re-checks the caller's own status, and the two added lines answer.
///
/// # Errors
///
/// As [`GameState::use_item`].
pub(super) fn use_premium(
    state: &mut GameState,
    vid: Vid,
    at: ItemPos,
    actor: Mover,
) -> Result<MovedItems, MoveItemRefused> {
    let looker = state.own_mover(vid);
    let held = state.affect_events.get(&vid).is_some_and(|event| {
        event
            .affects
            .iter()
            .any(|affect| affect.kind == AFFECT_MULTI_FARM_PREMIUM)
    });
    if held {
        if let Some(mover) = looker {
            state.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_INFO,
                b"Ai deja acest affect activ!",
                &[],
            );
        }
        return state.nothing_used(vid, actor);
    }
    let values = state
        .characters
        .find_by_vid(vid)
        .ok()
        .and_then(|character| character.items().item_at(at))
        .and_then(|item| state.protos.get(item.vnum))
        .map(|proto| proto.values);
    let Some(values) = values else {
        return state.nothing_used(vid, actor);
    };
    let affect = Affect {
        kind: AFFECT_MULTI_FARM_PREMIUM,
        apply_on: 0,
        value: values[1],
        flag: 0,
        remaining: values[0],
        sp_cost: 0,
    };
    state.add_pc_affect(vid, affect, false);
    let moved = state.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
        if rules.secured {
            return Err(world::character::MoveRefused::Secured);
        }
        world::character::use_premium(items, at, rules, gear)
    })?;
    if let Some(addr) = state.multi_addr_of(vid) {
        let farming = state.multi_status_of(vid);
        state.check_multi_farm_account(&addr, vid, farming);
    }
    if let Some(teller) = state.own_mover(vid) {
        state.tell(
            vid,
            teller,
            protocol::gc_chat::CHAT_TYPE_INFO,
            b"Affect succesfully added on your character!",
            &[],
        );
        state.tell(
            vid,
            teller,
            protocol::gc_chat::CHAT_TYPE_INFO,
            b"If you want use this affect this character need active drop status!",
            &[],
        );
    }
    Ok(moved)
}

impl super::GameState {
    /// The `multi-farm` protect's end for the character under `vid`, if it set one.
    pub(super) fn multi_protect_of(&self, vid: Vid) -> Option<u32> {
        self.multi_protect.get(&vid).copied()
    }

    /// Set the `multi-farm` protect's end for the character under `vid`.
    pub(super) fn set_multi_protect(&mut self, vid: Vid, until: u32) {
        self.multi_protect.insert(vid, until);
    }

    /// The host address the character under `vid` entered from, if it entered.
    pub(super) fn multi_addr_of(&self, vid: Vid) -> Option<String> {
        self.multi_addr.get(&vid).cloned()
    }

    /// `CHARACTER::GetMultiStatus` (`G/char.h:686`): absent is `false`.
    pub(super) fn multi_status_of(&self, vid: Vid) -> bool {
        self.multi_status.get(&vid).copied().unwrap_or(false)
    }

    /// `CHARACTER_MANAGER::CheckMultiFarmAccount` (`G/char_manager.cpp:1250-1305`) for
    /// the character under `vid` on `addr`, toggling to `farming`. The GG send has no
    /// Rewrite equivalent (the registry is already process-global); the rest is
    /// legacy's, including the recurse-with-`false` when two farm already.
    pub(super) fn check_multi_farm_account(&mut self, addr: &str, vid: Vid, farming: bool) {
        self.prune_multi_farm(addr);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let name = character.name().to_owned();
        let (affect_type, affect_time) = if farming {
            self.multi_premium_of(vid)
        } else {
            (0, 0)
        };
        if farming && self.multi_farm_count(addr) >= 2 {
            self.check_multi_farm_account(addr, vid, false);
            return;
        }
        self.set_multi_farm(addr, pid, &name, farming, affect_type, affect_time);
        self.multi_status.insert(vid, farming);
        self.send_multi_farm_updates(addr, vid);
    }

    /// `CHARACTER_MANAGER::CheckMultiFarmAccounts` (`G/char_manager.cpp:1149-1167`): drop
    /// the address's rows whose PID left the world, and the address with its last row.
    fn prune_multi_farm(&mut self, addr: &str) {
        let gone: Vec<u32> = self
            .multi_farm
            .get(addr)
            .map(|rows| {
                rows.iter()
                    .filter(|row| self.characters.find_by_pid(row.pid).is_err())
                    .map(|row| row.pid)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(rows) = self.multi_farm.get_mut(addr) {
            rows.retain(|row| !gone.contains(&row.pid));
        }
        if self.multi_farm.get(addr).is_some_and(Vec::is_empty) {
            self.multi_farm.remove(addr);
        }
    }

    /// The premium (`AFFECT_MULTI_FARM_PREMIUM`) of the character under `vid`: its apply
    /// value and the `time(0)` it ends at (`G/char_manager.cpp:1257-1258`), or zeros.
    fn multi_premium_of(&self, vid: Vid) -> (u8, i32) {
        let now = super::quests::quest_now();
        self.affect_events
            .get(&vid)
            .and_then(|event| {
                event
                    .affects
                    .iter()
                    .find(|affect| affect.kind == AFFECT_MULTI_FARM_PREMIUM)
            })
            .map_or((0, 0), |affect| {
                let value = u8::try_from(affect.value).unwrap_or(u8::MAX);
                let ends = now.saturating_add(u32::try_from(affect.remaining.max(0)).unwrap_or(0));
                (value, i32::try_from(ends).unwrap_or(i32::MAX))
            })
    }

    /// `CHARACTER_MANAGER::GetMultiFarmCount` (`G/char_manager.cpp:1223-1247`): the
    /// farming rows on `addr`, less the highest premium type while any premium runs,
    /// floored at 0.
    fn multi_farm_count(&self, addr: &str) -> i32 {
        let now = i64::from(super::quests::quest_now());
        let mut farming = 0;
        let mut premium_runs = false;
        let mut premium_top = 0;
        if let Some(rows) = self.multi_farm.get(addr) {
            for row in rows {
                if row.farming {
                    farming += 1;
                }
                if i64::from(row.affect_time) > now {
                    premium_runs = true;
                }
                premium_top = premium_top.max(row.affect_type);
            }
        }
        if premium_runs && premium_top > 0 {
            farming -= i32::from(premium_top);
        }
        farming.max(0)
    }

    /// `CHARACTER_MANAGER::SetMultiFarm` (`G/char_manager.cpp:1197-1221`): the row sets,
    /// or appends in join order.
    fn set_multi_farm(
        &mut self,
        addr: &str,
        pid: u32,
        name: &str,
        farming: bool,
        affect_type: u8,
        affect_time: i32,
    ) {
        let row = MultiFarmRow {
            pid,
            name: name.to_owned(),
            farming,
            affect_type,
            affect_time,
        };
        match self.multi_farm.get_mut(addr) {
            Some(rows) => match rows.iter_mut().find(|row| row.pid == pid) {
                Some(held) => *held = row,
                None => rows.push(row),
            },
            None => {
                self.multi_farm.insert(addr.to_owned(), vec![row]);
            }
        }
    }

    /// The fan-out half of `CheckMultiFarmAccount` (`G/char_manager.cpp:1290-1304`):
    /// every listed character hears `UpdateMultiFarmAffect` with its own status and
    /// whether it is the toggler, then one `UpdateMultiFarmPlayer` per farming row.
    fn send_multi_farm_updates(&self, addr: &str, toggler: Vid) {
        let Some(rows) = self.multi_farm.get(addr) else {
            return;
        };
        let rows = rows.clone();
        for row in &rows {
            let Some(vid) = self.vid_of_pid(row.pid) else {
                continue;
            };
            let Some(mover) = self.own_mover(vid) else {
                continue;
            };
            let status = i64::from(self.multi_status_of(vid));
            let is_toggler = i64::from(vid == toggler);
            self.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_COMMAND,
                b"UpdateMultiFarmAffect %d %d",
                &[Arg::Int(status), Arg::Int(is_toggler)],
            );
            for farming in rows.iter().filter(|row| row.farming) {
                self.tell(
                    vid,
                    mover,
                    protocol::gc_chat::CHAT_TYPE_COMMAND,
                    b"UpdateMultiFarmPlayer %s",
                    &[Arg::Text(farming.name.as_bytes())],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use common::vid::Vid;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::super::GameState;

    fn drained(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    #[test]
    fn a_toggle_reports_and_a_too_soon_toggle_answers_the_slow_line() {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let mut inbox = enter(&mut world, 7, (3200, 3200), 0);
        let _ = drained(&mut inbox);
        assert!(world.multi_status_of(Vid::new(7)), "the entrant farms");
        super::run_multi_farm(&mut world, Vid::new(7), "");
        let burst = drained(&mut inbox);
        assert_eq!(burst.len(), 1, "farm-off lists no row: {burst:02x?}");
        assert!(!world.multi_status_of(Vid::new(7)));
        super::run_multi_farm(&mut world, Vid::new(7), "");
        let slow = drained(&mut inbox);
        assert_eq!(slow.len(), 1, "the slow line: {slow:02x?}");
        assert!(slow[0].windows(9).any(|w| w == b"be slow! "), "{slow:02x?}");
        let _ = GameState::global_time;
    }
}
