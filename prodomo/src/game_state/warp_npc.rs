//! The warp NPCs: `warp_npc_event` and `FuncCheckWarp` (`G/char.cpp:7893-8015`), with the
//! `IsHack` check they run on each player (`G/char.cpp:8226-8293`).
//!
//! # The event
//!
//! `SetProto` starts the event for every `CHAR_TYPE_WARP` and `CHAR_TYPE_GOTO` NPC
//! (`StartWarpNPCEvent`, `G/char.cpp:8017-8030`). It fires 12 Pulses (`passes_per_sec / 2`) after
//! the NPC is created and every 12 after that, whether anybody stands near or not. Each fire
//! parses the NPC's name as `" %s %ld %ld "` and multiplies the two numbers by 100; a goto NPC
//! adds its map's base. Then each player around the NPC is checked, in this order:
//!
//! 1. it stands within 300 (`DISTANCE_APPROX`);
//! 2. it has the NPC's empire, when both have one;
//! 3. `IsHack` passes it;
//! 4. `CanHandleItem(false, true)` passes it.
//!
//! A player that passes is sent through `WarpSet` by a warp NPC, and shown at the target on its
//! own map and stopped by a goto NPC. The world owns no descriptor, so a warp is a
//! [`ClientOrder`] that the player's descriptor runs on its next turn; a goto moves the body in
//! the world at once, as legacy's `Show` and `Stop` do.
//!
//! # `IsHack`
//!
//! With its defaults (`G/char.h:1958`), `IsHack` refuses, in this order and each with its line:
//!
//! 1. a safebox loaded or closed within `g_nPortalLimitTime` (10 s, 250 Pulses): `[LS;850;%d]`;
//! 2. an open trade, shop window or safebox: `[LS;851]`;
//! 3. a trade started or completed within 250 Pulses: `[LS;852;%d]`;
//! 4. a shop bought from or closed within 250 Pulses: `[LS;852;%d]`.
//!
//! # Divergences
//!
//! - **The phase.** Legacy fires each NPC's event 12 Pulses after that NPC was created. Every warp
//!   NPC the Rewrite has stands up at boot, before the first Pulse, so every event fires on the
//!   Pulses that are a multiple of 12.
//! - **The order of the players.** Legacy walks the sectrees around the NPC's in `Build` order,
//!   each an `unordered_set` hashed by pointer, so the order inside a sectree is unspecified. The
//!   Rewrite walks each sectree in VID order (V1). Only the order of the lines and orders
//!   differs; each player is judged alone.
//! - **One warp per Pulse.** A player that `WarpSet` accepts leaves its sectree, so no other NPC
//!   finds it on the same Pulse. The descriptor runs the warp after the Pulse, so the Rewrite
//!   checks a player that one NPC has warped against no other NPC on that Pulse, whatever the
//!   warp then does. A goto moves the body at once, so a later NPC of the same Pulse judges it
//!   again at its new spot, as legacy's later event would.
//! - **An invalid name.** Legacy re-parses the name on every fire and, for a name that does not
//!   parse, draws `number(1, 100)` and logs when it is below 5. The Rewrite parses the name
//!   once, when the NPC stands up, and logs an invalid name once. A number whose target
//!   does not fit 32 bits is invalid too, where legacy's 32-bit `long` clamps the number and
//!   wraps the product; no name in the data is near.
//! - **The start of the process.** Legacy's timers start at 0 (`G/char.cpp:278`), so for the first
//!   10 seconds after boot every player is refused with `[LS;850;10]` (a Defect). A timer that
//!   was never set refuses nothing in the Rewrite.
//!
//! # Not ported
//!
//! - The `[TestOnly]Pulse %d LoadTime %d PASS %d` line `IsHack` adds under `test_server`, left to
//!   the `test_server` audit the owner decides on, with the rest of `test_server`'s effects.
//! - The refine timer (`[LS;437;%d]`), the personal shop, the cube and the aura window: the
//!   Rewrite has no refine, no personal shop, no cube and no aura window, so none refuses.
//! - `CanHandleItem(false, true)` (`G/char_item.cpp:213-247`): every window and state it reads
//!   (the personal shop, the refine, the cube, the dragon soul refine, `IsWarping`, and under the
//!   owner's `__SASH_SYSTEM__`, `__AURA_SYSTEM__` and `__CHANGELOOK_SYSTEM__` the sash
//!   combination and absorption, the aura refine window and its opener, and the change look) is
//!   one the Rewrite does not have, so it passes every player. Each port of one gates the warp
//!   NPC here.

use std::collections::HashSet;

use common::vid::Vid;
use gamedata::map_atlas::MapRegion;
use gamedata::mob_proto::{CHAR_TYPE_GOTO, CHAR_TYPE_WARP};
use protocol::gc_chat::CHAT_TYPE_INFO;
use tracing::warn;
use world::npc::Npc;

use super::safebox::LOAD_WAIT_PULSES;
use super::view::EntityKey;
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::client_registry::ClientOrder;
use crate::item_move::Mover;
use crate::sync_position::distance_approx;
use crate::warp::WARP_LOCATION_SCALE;

/// `passes_per_sec / 2`: the Pulses between two fires of `warp_npc_event` (`G/char.cpp:8013`).
pub const WARP_NPC_PULSES: u64 = 12;

/// `FuncCheckWarp`'s reach (`G/char.cpp:7953-7956`).
pub const WARP_NPC_REACH: i32 = 300;

/// `g_nPortalLimitTime` (`G/char_item.cpp:7166`): the seconds `IsHack` names.
pub const PORTAL_LIMIT_SECONDS: i64 = 10;

/// `IsHack`'s first refusal: a safebox loaded or closed within the portal limit.
pub const SAFEBOX_HACK_NOTICE: &[u8] = b"[LS;850;%d]";

/// `IsHack`'s second refusal: an open trade, shop window or safebox.
pub const WINDOW_HACK_NOTICE: &[u8] = b"[LS;851]";

/// `IsHack`'s third and fourth refusals: a trade, or a shop, within the portal limit.
pub const RECENT_HACK_NOTICE: &[u8] = b"[LS;852;%d]";

/// The `z` `CHARACTER::Show` is given when the caller names none: the default `LONG_MAX` of
/// its declaration (`G/char.h:879`), which is `i32::MAX` on the 32-bit legacy target. A goto
/// NPC names none (`G/char.cpp:7971`), so the insert record the client then receives carries
/// it.
pub const SHOW_Z: i32 = i32::MAX;

/// What a warp or goto NPC does to a player that passes `FuncCheckWarp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpcOrder {
    /// `WarpSet(x, y)` from a warp NPC (`G/char.cpp:7967-7968`), which the player's descriptor
    /// runs as a [`ClientOrder::Warp`].
    Warp {
        /// The target x.
        x: i32,
        /// The target y.
        y: i32,
    },
    /// `Show(GetMapIndex(), x, y)` then `Stop()` from a goto NPC (`G/char.cpp:7971-7972`),
    /// which the world runs.
    Goto {
        /// The target x, the map's base included.
        x: i32,
        /// The target y, the map's base included.
        y: i32,
    },
}

/// A warp or goto NPC whose name parsed, and what it does to a player that passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WarpNpc {
    /// Its VID, for the log.
    vid: u32,
    /// Its x.
    x: i32,
    /// Its y.
    y: i32,
    /// `m_bEmpire`: its own empire, 0 for none.
    empire: u8,
    /// The `WarpSet` or `Show` a player that passes is sent.
    order: NpcOrder,
}

/// The pulses `IsHack` measures the portal limit from, for one character.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PortalTimes {
    /// `m_iExchangeTime`: a trade started, or both sides accepted.
    exchange: Option<u64>,
    /// `m_iMyShopTime`: a shop bought from, or its window closed.
    shop: Option<u64>,
}

/// `sscanf(name, " %s %ld %ld ", ...)` as glibc reads it: whether it assigns all three, and the
/// two numbers when it does.
///
/// The name ends at its first NUL, as `GetName()` is a C string. `%s` takes any run of
/// characters that are not white space, and each `%ld` an optional sign and at least one
/// decimal digit; the white space before each is skipped. A number too large for 64 bits does
/// not parse. glibc's `%ld` clamps a number too large for the legacy build's 32-bit `long` to
/// `LONG_MAX` instead; the target of either is past what [`warp_npc_order`] accepts.
#[must_use]
pub fn parse_warp_name(name: &[u8]) -> Option<(i64, i64)> {
    let name = name.split(|&byte| byte == 0).next().unwrap_or_default();
    let mut rest = skip_space(name);
    let token = rest.iter().take_while(|byte| !is_space(**byte)).count();
    if token == 0 {
        return None;
    }
    rest = rest.get(token..).unwrap_or_default();
    let (x, rest) = scan_long(rest)?;
    let (y, _) = scan_long(rest)?;
    Some((x, y))
}

/// The white space of `isspace` in the C locale.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// The input after its leading white space.
fn skip_space(input: &[u8]) -> &[u8] {
    let skipped = input.iter().take_while(|byte| is_space(**byte)).count();
    input.get(skipped..).unwrap_or_default()
}

/// One `%ld`: white space, an optional sign, then decimal digits.
fn scan_long(input: &[u8]) -> Option<(i64, &[u8])> {
    let input = skip_space(input);
    let (negative, digits) = match input.first() {
        Some(b'-') => (true, input.get(1..).unwrap_or_default()),
        Some(b'+') => (false, input.get(1..).unwrap_or_default()),
        _ => (false, input),
    };
    let count = digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if count == 0 {
        return None;
    }
    let mut value: i64 = 0;
    for digit in digits.get(..count).unwrap_or_default() {
        value = value
            .checked_mul(10)?
            .checked_add(i64::from(digit - b'0'))?;
    }
    let value = if negative {
        value.checked_neg()?
    } else {
        value
    };
    Some((value, digits.get(count..).unwrap_or_default()))
}

/// The target `FuncCheckWarp` computes for one NPC standing on `region`, or `None` when its
/// name does not parse or the target does not fit.
///
/// A warp NPC's target is the name's numbers times 100. A goto NPC adds `iBaseX` and `iBaseY`,
/// the map's base, which the region's left and top edges are.
#[must_use]
pub fn warp_npc_order(npc: &Npc, region: &MapRegion) -> Option<NpcOrder> {
    let (x, y) = parse_warp_name(&npc.name)?;
    let scale = i64::from(WARP_LOCATION_SCALE);
    let x = i32::try_from(x.checked_mul(scale)?).ok()?;
    let y = i32::try_from(y.checked_mul(scale)?).ok()?;
    match npc.char_type {
        CHAR_TYPE_WARP => Some(NpcOrder::Warp { x, y }),
        CHAR_TYPE_GOTO => Some(NpcOrder::Goto {
            x: x.checked_add(region.sx)?,
            y: y.checked_add(region.sy)?,
        }),
        _ => None,
    }
}

/// The warp and goto NPCs among `npcs`, each with its order. A name that does not parse is
/// logged once and left out.
pub(super) fn warp_npcs(npcs: &[Npc], region: &MapRegion) -> Vec<WarpNpc> {
    npcs.iter()
        .filter(|npc| matches!(npc.char_type, CHAR_TYPE_WARP | CHAR_TYPE_GOTO))
        .filter_map(|npc| {
            let Some(order) = warp_npc_order(npc, region) else {
                warn!(
                    vnum = npc.vnum,
                    vid = npc.vid,
                    map = region.index,
                    name = %String::from_utf8_lossy(&npc.name),
                    "Warp NPC name wrong"
                );
                return None;
            };
            Some(WarpNpc {
                vid: npc.vid,
                x: npc.x,
                y: npc.y,
                empire: npc.empire,
                order,
            })
        })
        .collect()
}

/// `m_bEmpire && pkChr->GetEmpire() && m_bEmpire != pkChr->GetEmpire()`: the NPC refuses a
/// player of another empire when both have one.
#[must_use]
pub const fn empire_refuses(npc: u8, player: u8) -> bool {
    npc != 0 && player != 0 && npc != player
}

impl GameState {
    /// `SetExchangeTime` for the character under `vid`.
    pub(super) fn set_exchange_time(&mut self, vid: Vid) {
        self.portal_times.entry(vid).or_default().exchange = Some(self.last_pulse);
    }

    /// `SetMyShopTime` for the character under `vid`.
    pub(super) fn set_shop_time(&mut self, vid: Vid) {
        self.portal_times.entry(vid).or_default().shop = Some(self.last_pulse);
    }

    /// Forget the portal times of the character under `vid`, as its departure does.
    pub(super) fn forget_portal_times(&mut self, vid: Vid) {
        let _times = self.portal_times.remove(&vid);
    }

    /// `CHARACTER::IsHack()` with its defaults: the line it sends the character under `vid`
    /// when it refuses, or `None` when it passes.
    pub(super) fn is_hack(&self, vid: Vid, mover: Mover) -> Option<Vec<u8>> {
        let limit = [Arg::Int(PORTAL_LIMIT_SECONDS)];
        let line = |text: &[u8], args: &[Arg<'_>]| {
            chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, text, args)
        };
        if self.safebox_loaded_recently(vid) {
            return Some(line(SAFEBOX_HACK_NOTICE, &limit));
        }
        if self.trading.contains_key(&vid)
            || self.browsing.contains_key(&vid)
            || self.safebox_open(vid)
        {
            return Some(line(WINDOW_HACK_NOTICE, &[]));
        }
        let times = self.portal_times.get(&vid).copied().unwrap_or_default();
        let recent = |at: Option<u64>| {
            at.is_some_and(|at| self.last_pulse.saturating_sub(at) < LOAD_WAIT_PULSES)
        };
        if recent(times.exchange) || recent(times.shop) {
            return Some(line(RECENT_HACK_NOTICE, &limit));
        }
        None
    }

    /// Fire every warp NPC's event on the Pulses it fires on, and run `FuncCheckWarp` on each
    /// player around it (`G/char.cpp:7988-8014`). An NPC that stands in no sectree fires on
    /// nobody (`:7998-8002`).
    pub(super) fn run_warp_npcs(&mut self, pulse: u64) {
        if pulse == 0 || pulse % WARP_NPC_PULSES != 0 {
            return;
        }
        let Some(clients) = self.clients.clone() else {
            return;
        };
        let mut warped: HashSet<u32> = HashSet::new();
        let mut warps: Vec<(u8, u64, ClientOrder)> = Vec::new();
        let maps: Vec<(u8, i32)> = self.warp_npcs.keys().copied().collect();
        for (channel, map) in maps {
            let members = clients.members_on_map(channel, map);
            if members.is_empty() {
                continue;
            }
            let npcs = self
                .warp_npcs
                .get(&(channel, map))
                .cloned()
                .unwrap_or_default();
            for npc in npcs {
                // `ForEachAround` collects the entities before it calls the check on any.
                let around = self
                    .maps
                    .get(&(channel, map))
                    .map(|index| index.around_of(EntityKey::Npc(npc.vid)))
                    .unwrap_or_default();
                for key in around {
                    let EntityKey::Character(raw) = key else {
                        continue;
                    };
                    if warped.contains(&raw) {
                        continue;
                    }
                    let Some((id, entry)) = members.iter().find(|(_, entry)| entry.vid == raw)
                    else {
                        continue;
                    };
                    let vid = Vid::new(raw);
                    if self.characters.find_by_vid(vid).is_err() {
                        continue;
                    }
                    let Some(spot) = self.spot_of(raw) else {
                        continue;
                    };
                    let distance =
                        distance_approx(spot.x.saturating_sub(npc.x), spot.y.saturating_sub(npc.y));
                    if distance > WARP_NPC_REACH || empire_refuses(npc.empire, entry.empire) {
                        continue;
                    }
                    let mover = Mover {
                        recently_fought: false,
                        empire: entry.empire,
                        language: entry.language,
                        pk_mode: self.pk_mode_of(raw),
                        affect_flags: self.affect_flags_of(common::vid::Vid::new(raw)),
                    };
                    if let Some(line) = self.is_hack(vid, mover) {
                        let _sent = self.write_to_client(vid, line);
                        continue;
                    }
                    match npc.order {
                        NpcOrder::Warp { x, y } => {
                            let _first = warped.insert(raw);
                            warps.push((channel, *id, ClientOrder::Warp { x, y }));
                        }
                        NpcOrder::Goto { x, y } => {
                            // `Show` changes nothing when no sectree holds the target; `Stop`
                            // runs either way.
                            let _shown = self.show_body(raw, (x, y, SHOW_Z), None);
                            self.stop(raw);
                        }
                    }
                }
            }
        }
        for (channel, id, order) in warps {
            if !clients.order(channel, id, order) {
                warn!(
                    channel,
                    id,
                    ?order,
                    "A warp NPC ordered a client that is gone"
                );
            }
        }
    }

    /// The warp and goto NPCs standing on one map of one Channel, with their orders.
    #[cfg(test)]
    pub(super) fn warp_npcs_on(&self, channel: u8, map: i32) -> Vec<NpcOrder> {
        self.warp_npcs
            .get(&(channel, map))
            .map(|npcs| npcs.iter().map(|npc| npc.order).collect())
            .unwrap_or_default()
    }

    /// The VIDs of the warp and goto NPCs standing on one map of one Channel.
    #[cfg(test)]
    pub(super) fn warp_npc_vids_on(&self, channel: u8, map: i32) -> Vec<u32> {
        self.warp_npcs
            .get(&(channel, map))
            .map(|npcs| npcs.iter().map(|npc| npc.vid).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use gamedata::mob_proto::CHAR_TYPE_NPC;

    use super::*;

    /// `metin2_map_monkey_dungeon2`'s region (`Setting.txt:8-9`), whose goto NPC 10601 is named
    /// `. 345 361` (`npc.txt:8`). The owner's map index lists no such map, so its index here is
    /// made up.
    fn a_region() -> MapRegion {
        MapRegion {
            index: 1_000,
            name: b"metin2_map_monkey_dungeon2".to_vec(),
            sx: 128_000,
            sy: 640_000,
            ex: 204_800,
            ey: 716_800,
            spawn: (0, 0),
            empire_spawns: None,
        }
    }

    fn an_npc(char_type: u8, name: &[u8]) -> Npc {
        Npc {
            vid: 0x8000_0101,
            vnum: 10_001,
            race: 10_001,
            char_type,
            on_click: 0,
            x: 450_100,
            y: 903_300,
            z: 0,
            rotation: 0,
            empire: 0,
            moving_speed: 0,
            attack_speed: 0,
            name: name.to_vec(),
        }
    }

    #[test]
    fn a_name_parses_as_a_token_then_two_decimal_longs() {
        assert_eq!(parse_warp_name(b"a3 4002 8995"), Some((4002, 8995)));
        assert_eq!(parse_warp_name(b". 345 361"), Some((345, 361)));
        assert_eq!(parse_warp_name(b"  \twarp\n-12 +34 rest"), Some((-12, 34)));
        assert_eq!(
            parse_warp_name(b"\x0ba\x0c1\r2"),
            Some((1, 2)),
            "the vertical tab, the form feed and the carriage return are white space"
        );
        assert_eq!(
            parse_warp_name(b"a 12x 34"),
            None,
            "the second number starts at the x"
        );
        assert_eq!(
            parse_warp_name(b"a 12 34x"),
            Some((12, 34)),
            "a tail is fine"
        );
    }

    #[test]
    fn a_name_without_three_fields_does_not_parse() {
        assert_eq!(parse_warp_name(b""), None);
        assert_eq!(parse_warp_name(b"   "), None);
        assert_eq!(parse_warp_name(b"Guard"), None);
        assert_eq!(parse_warp_name(b"a 345"), None);
        assert_eq!(
            parse_warp_name(b"345 361"),
            None,
            "the token takes the first"
        );
        assert_eq!(parse_warp_name(b"a b 1 2"), None);
        assert_eq!(parse_warp_name(b"a - 1"), None, "a sign needs a digit");
        assert_eq!(parse_warp_name(b"a 0x10 2"), None, "decimal only");
    }

    #[test]
    fn a_name_ends_at_its_first_nul_and_a_number_must_fit() {
        assert_eq!(parse_warp_name(b"a 1\0 2"), None);
        assert_eq!(
            parse_warp_name(b"a\0 1 2"),
            None,
            "the NUL ends the token and the name"
        );
        assert_eq!(parse_warp_name(b"a 1 2\0 junk"), Some((1, 2)));
        assert_eq!(
            parse_warp_name(b"a 9223372036854775807 1"),
            Some((i64::MAX, 1))
        );
        assert_eq!(parse_warp_name(b"a 9223372036854775808 1"), None);
        assert_eq!(
            parse_warp_name(b"a 18446744073709551621 1"),
            None,
            "2^64 + 5 does not wrap to 5"
        );
        assert_eq!(
            parse_warp_name(b"a -9223372036854775807 1"),
            Some((-i64::MAX, 1))
        );
    }

    #[test]
    fn a_warp_npc_sends_to_its_numbers_times_a_hundred() {
        let npc = an_npc(CHAR_TYPE_WARP, b"a3 4002 8995");
        assert_eq!(
            warp_npc_order(&npc, &a_region()),
            Some(NpcOrder::Warp {
                x: 400_200,
                y: 899_500
            })
        );
    }

    #[test]
    fn a_goto_npc_adds_its_maps_base() {
        let npc = an_npc(CHAR_TYPE_GOTO, b". 345 361");
        assert_eq!(
            warp_npc_order(&npc, &a_region()),
            Some(NpcOrder::Goto {
                x: 162_500,
                y: 676_100
            })
        );
    }

    #[test]
    fn an_npc_that_is_neither_or_a_target_that_does_not_fit_has_no_order() {
        let region = a_region();
        assert_eq!(
            warp_npc_order(&an_npc(CHAR_TYPE_NPC, b"a 1 2"), &region),
            None
        );
        let far = an_npc(CHAR_TYPE_WARP, b"a 21474837 1");
        assert_eq!(warp_npc_order(&far, &region), None, "past i32::MAX");
        let edge = an_npc(CHAR_TYPE_WARP, b"a 21474836 -21474836");
        assert_eq!(
            warp_npc_order(&edge, &region),
            Some(NpcOrder::Warp {
                x: 2_147_483_600,
                y: -2_147_483_600
            })
        );
        let past = an_npc(CHAR_TYPE_GOTO, b"a 21474836 1");
        assert_eq!(warp_npc_order(&past, &region), None, "the base overflows");
    }

    #[test]
    fn only_the_warp_and_goto_npcs_whose_names_parse_are_kept() {
        let npcs = [
            an_npc(CHAR_TYPE_NPC, b"a 1 2"),
            Npc {
                vid: 1,
                empire: 2,
                x: 5,
                y: 6,
                ..an_npc(CHAR_TYPE_WARP, b"a 1 2")
            },
            an_npc(CHAR_TYPE_WARP, b"Gatekeeper"),
            Npc {
                vid: 2,
                ..an_npc(CHAR_TYPE_GOTO, b". 3 4")
            },
        ];

        let kept = warp_npcs(&npcs, &a_region());

        assert_eq!(
            kept,
            vec![
                WarpNpc {
                    vid: 1,
                    x: 5,
                    y: 6,
                    empire: 2,
                    order: NpcOrder::Warp { x: 100, y: 200 },
                },
                WarpNpc {
                    vid: 2,
                    x: 450_100,
                    y: 903_300,
                    empire: 0,
                    order: NpcOrder::Goto {
                        x: 128_300,
                        y: 640_400
                    },
                },
            ]
        );
    }

    #[test]
    fn only_two_different_empires_refuse() {
        assert!(!empire_refuses(0, 0));
        assert!(!empire_refuses(0, 2));
        assert!(!empire_refuses(2, 0));
        assert!(!empire_refuses(2, 2));
        assert!(empire_refuses(1, 2));
        assert!(empire_refuses(3, 1));
    }
}
