//! The NPC shops: a click on a keeper opens its shop's window, and the window buys, sells and
//! closes (`CShopManager`, `G/shop_manager.cpp`, and `CShop`, `G/shop.cpp`).
//!
//! The world keeps which keeper each character is browsing, as legacy's `CHARACTER` keeps
//! `m_pkShop` and `m_pkChrShopOwner`. The storage and the gold change in
//! [`world::character::buy_item`] and [`world::character::sell_item`]; this is the manager
//! around them, which finds the keeper, measures the distance and chooses the answer.
//!
//! # The order
//!
//! A click (`CHARACTER::OnClick`, `G/char.cpp:6181-6352`, then `StartShopping`,
//! `G/shop_manager.cpp:115-161`) is ignored when the VID names no NPC and when the character trades
//! (`G/char.cpp:6210-6217`). Then the NPC's quests run (`CQuestManager::Click`,
//! `G/char.cpp:6333-6338`), and a quest that takes the click answers with its dialog and opens no
//! shop. It is ignored when the NPC's click trigger is not the shop's and when the character
//! already browses that keeper. Then an open safebox refuses it with `[LS;876]`
//! (`G/shop_manager.cpp:124-127`), and it is ignored when the keeper is `SHOP_MAX_DISTANCE` or
//! farther, when no shop names the keeper's vnum, and when the character browses another keeper.
//! Otherwise the window opens with each price tripled for a stranger unless `disable_shop_price_3x`
//! is set.
//!
//! A buy or a sale is ignored with no keeper, and refused with `[LS;877]` from more than 2000
//! away (`G/shop_manager.cpp:385-454`, `:456-594`). A buy of a position past the window answers
//! `SHOP_SUBHEADER_GC_INVALID_POS`; the rest are the world's. Closing answers
//! `SHOP_SUBHEADER_GC_END` when a window is open and nothing when none is.
//!
//! A buy that passes the distance check and a close of an open window stamp `SetMyShopTime`, the
//! portal guard a warp NPC's `IsHack` reads (`G/shop_manager.cpp:377`, `:439`). A sale does not,
//! as in legacy.
//!
//! # Divergences
//!
//! - **A keeper on another map.** `CHARACTER_MANAGER::Find` looks a VID up across every map
//!   of the process and `DISTANCE_APPROX` ignores the map, so a client naming a keeper on
//!   another map that stands at the same coordinates opens its shop. Only a modified client
//!   names one: a Defect. The Rewrite looks on the character's own Channel and map.
//! - **A second window.** `CShop::AddGuest` refuses a character that already browses, but
//!   `StartShopping` then makes the new keeper its shop owner anyway, so the next buy measures
//!   the distance to one keeper and buys from the other's shop. A refused open changes nothing
//!   here.
//! - **The tripled price.** A stranger is shown each price tripled, but `CShop::Buy` charges
//!   the price untripled (the tripling is commented out, `G/shop.cpp:658-659`). The Rewrite
//!   charges the price it showed.
//! - **Unported gates.** Of the other-window check (`[LS;876]`) only the safebox refuses: a trade
//!   never reaches it because `OnClick` has already ignored the click, and the personal shop, the
//!   cube and the aura window are not in the Rewrite. `IsSecured`, `CanHandleItem`, a locked item
//!   and the buy-and-sell throttle belong to systems the Rewrite does not have: none of them is
//!   open, secured, locked or set, so none refuses.

use std::sync::Arc;

use common::item_slots::usable_inventory_cells;
use common::vid::Vid;
use gamedata::item_proto::ItemProtos;
use gamedata::locale_string::LocaleStrings;
use gamedata::npc_shop::{NpcShop, NpcShops, ShopSlot};
use protocol::cg_shop::CgShop;
use protocol::gc_chat::CHAT_TYPE_INFO;
use protocol::gc_shop::{
    GcShop, GcShopItem, GcShopStart, SHOP_HOST_ITEM_MAX_NUM, SHOP_SUBHEADER_GC_END,
    SHOP_SUBHEADER_GC_INVALID_POS,
};
use world::character::{
    buy_item, sell_item, sync_quickslots, Character, MoveDone, MoveRules, ShopRefused,
};
use world::npc::Npc;

use super::GameState;
use crate::chat_line::chat_packet;
use crate::game_loop_messages::GroundPlace;
use crate::item_move::{gold_delta, MoveItemRefused, MovedItems, Mover};
use crate::sync_position::distance_approx;

/// `SHOP_MAX_DISTANCE` (`G/shop.h:6`): `StartShopping` opens no window from this far or farther.
const SHOP_MAX_DISTANCE: i32 = 1000;

/// The distance past which `CShopManager::Buy` and `Sell` refuse (`G/shop_manager.cpp:405`,
/// `:482`).
const SHOP_TRADE_DISTANCE: i32 = 2000;

/// `ON_CLICK_SHOP`: the click trigger `OnClickShop` (`G/trigger.cpp:21-25`).
const ON_CLICK_SHOP: u8 = 1;

/// `[LS;877]`: the keeper stands too far to trade with.
const TOO_FAR_NOTICE: &str = "[LS;877]";

/// The keeper a character browses: `m_pkChrShopOwner` and its `m_pkShop` together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Browsing {
    /// The keeper's VID.
    keeper: u32,
    /// The keeper's vnum, which names its shop.
    npc_vnum: u32,
    /// Where the keeper stands; an NPC never moves.
    x: i32,
    /// Where the keeper stands.
    y: i32,
    /// Whether the window showed each price tripled.
    tripled: bool,
}

/// One shop request of a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopStep {
    /// `CG_ON_CLICK` on `target`, which opens a keeper's shop.
    Click {
        /// The VID the client clicked.
        target: u32,
    },
    /// `SHOP_SUBHEADER_CG_BUY`: buy the window's slot `pos`.
    Buy {
        /// The window slot.
        pos: u8,
    },
    /// `SHOP_SUBHEADER_CG_SELL` or `SELL2`: sell `count` of the item in inventory cell `cell`,
    /// where 0 is all of it.
    Sell {
        /// The inventory cell.
        cell: u16,
        /// How many, where 0 is the whole stack.
        count: u16,
    },
    /// `SHOP_SUBHEADER_CG_END`: close the window.
    End,
}

impl ShopStep {
    /// The step a `CG_SHOP` asks for, as `CInputMain::Shop` reads it
    /// (`G/input_main.cpp:1241-1303`): `SELL` is `Sell(ch, pos)`, whose count defaults to 0, the
    /// whole stack. `None` for a subheader legacy logs and ignores.
    #[must_use]
    pub fn requested(record: CgShop) -> Option<Self> {
        match record {
            CgShop::End => Some(Self::End),
            CgShop::Buy { pos, .. } => Some(Self::Buy { pos }),
            CgShop::Sell { cell } => Some(Self::Sell {
                cell: u16::from(cell),
                count: 0,
            }),
            CgShop::Sell2 { slot, count, .. } => Some(Self::Sell {
                cell: u16::from(slot),
                count,
            }),
            CgShop::Unknown { .. } => None,
        }
    }
}

/// Why a shop step changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopDeclined {
    /// No NPC on the character's map has the clicked VID.
    NoSuchNpc {
        /// The VID the client clicked.
        target: u32,
    },
    /// The character trades.
    Trading,
    /// A quest took the click, and its dialog is the answer (`G/char.cpp:6333-6338`).
    Quest,
    /// The NPC's click trigger is not the shop's.
    NotAShop {
        /// Its `ON_CLICK_*`.
        on_click: u8,
    },
    /// The character already browses that keeper.
    AlreadyBrowsing,
    /// The character has its safebox open (`[LS;876]`, `shop_manager.cpp:124`).
    SafeboxOpen,
    /// The keeper stands `SHOP_MAX_DISTANCE` or farther.
    OutOfReach {
        /// `DISTANCE_APPROX` to the keeper.
        distance: i32,
    },
    /// No shop names the keeper's vnum.
    NoShop {
        /// The keeper's vnum.
        vnum: u32,
    },
    /// The character browses another keeper.
    BrowsingAnother {
        /// That keeper's VID.
        keeper: u32,
    },
    /// The character browses no keeper.
    NotBrowsing,
    /// The keeper stands more than 2000 away.
    TooFar {
        /// `DISTANCE_APPROX` to the keeper.
        distance: i32,
    },
    /// The slot is past the window.
    InvalidPos(u8),
    /// The world refused the buy or the sale.
    Refused(ShopRefused),
}

/// What a shop step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopAnswer {
    /// The window opened or closed; the records are sent and nothing is stored.
    Sent(Vec<Vec<u8>>),
    /// Nothing changed; the records, if any, are how legacy answers it.
    Declined {
        /// Why.
        reason: ShopDeclined,
        /// The records the client is sent.
        records: Vec<Vec<u8>>,
    },
    /// A buy or a sale, stored and sent as a move is, with the gold it left.
    Moved(MovedItems),
}

impl ShopAnswer {
    /// A decline legacy answers with nothing.
    const fn silent(reason: ShopDeclined) -> Self {
        Self::Declined {
            reason,
            records: Vec::new(),
        }
    }
}

impl GameState {
    /// Share the NPC shops boot laid out. Without them no keeper has a shop.
    #[must_use]
    pub fn with_npc_shops(mut self, shops: Arc<NpcShops>) -> Self {
        self.shops = shops;
        self
    }

    /// Set `g_bEmpireShopPriceTripleDisable`, the configured `game.disable_shop_price_3x`.
    ///
    /// Until this is called a stranger's prices are tripled, as legacy's compiled-in `false`
    /// has them.
    #[must_use]
    pub const fn with_shop_price_3x_disabled(mut self, disabled: bool) -> Self {
        self.shop_price_3x_disabled = disabled;
        self
    }

    /// Run one shop step for the character online under `vid`, standing at `place`.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn shop(
        &mut self,
        vid: Vid,
        step: ShopStep,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<ShopAnswer, MoveItemRefused> {
        if self.characters.find_by_vid(vid).is_err() {
            return Err(MoveItemRefused::NoSuchCharacter { vid });
        }
        match step {
            ShopStep::Click { target } => Ok(self.open_shop(vid, target, place, mover)),
            ShopStep::Buy { pos } => self.buy(vid, pos, place, mover),
            ShopStep::Sell { cell, count } => self.sell(vid, cell, count, place, mover),
            ShopStep::End => Ok(self.close_shop(vid)),
        }
    }

    /// Stop whatever the character under `vid` browses, as its departure does.
    pub(super) fn stop_browsing(&mut self, vid: Vid) {
        let _browsing = self.browsing.remove(&vid);
    }

    /// `CHARACTER::OnClick` on an NPC: the quests first, then its click trigger.
    fn open_shop(&mut self, vid: Vid, target: u32, place: GroundPlace, mover: Mover) -> ShopAnswer {
        let Some(race) = self.npc_at(place, target).map(|npc| npc.race) else {
            return ShopAnswer::silent(ShopDeclined::NoSuchNpc { target });
        };
        if self.trading.contains_key(&vid) {
            return ShopAnswer::silent(ShopDeclined::Trading);
        }
        let (taken, mut records) = self.quest_click(vid, (target, race), place, mover);
        if taken {
            return ShopAnswer::Declined {
                reason: ShopDeclined::Quest,
                records,
            };
        }
        match self.open_window(vid, target, place, mover) {
            ShopAnswer::Sent(sent) => {
                records.extend(sent);
                ShopAnswer::Sent(records)
            }
            ShopAnswer::Declined {
                reason,
                records: declined,
            } => {
                records.extend(declined);
                ShopAnswer::Declined { reason, records }
            }
            sold @ ShopAnswer::Moved(_) => sold,
        }
    }

    /// The NPC under `target` on the map at `place`.
    fn npc_at(&self, place: GroundPlace, target: u32) -> Option<&Npc> {
        self.npcs
            .get(&(place.channel, place.map))
            .and_then(|map| map.npcs.iter().find(|npc| npc.vid == target))
    }

    /// `OnClickShop`, then `StartShopping` and `CShop::AddGuest`.
    fn open_window(
        &mut self,
        vid: Vid,
        target: u32,
        place: GroundPlace,
        mover: Mover,
    ) -> ShopAnswer {
        let Some(npc) = self.npc_at(place, target) else {
            return ShopAnswer::silent(ShopDeclined::NoSuchNpc { target });
        };
        if npc.on_click != ON_CLICK_SHOP {
            return ShopAnswer::silent(ShopDeclined::NotAShop {
                on_click: npc.on_click,
            });
        }
        let open = self.browsing.get(&vid).copied();
        if open.is_some_and(|open| open.keeper == target) {
            return ShopAnswer::silent(ShopDeclined::AlreadyBrowsing);
        }
        if self.safebox_open(vid) {
            return ShopAnswer::Declined {
                reason: ShopDeclined::SafeboxOpen,
                records: vec![notice(SAFEBOX_OPEN_NOTICE, mover, &self.locale)],
            };
        }
        let distance =
            distance_approx(place.x.saturating_sub(npc.x), place.y.saturating_sub(npc.y));
        if distance >= SHOP_MAX_DISTANCE {
            return ShopAnswer::silent(ShopDeclined::OutOfReach { distance });
        }
        let Some(shop) = self.shops.for_npc(npc.vnum) else {
            return ShopAnswer::silent(ShopDeclined::NoShop { vnum: npc.vnum });
        };
        if let Some(open) = open {
            return ShopAnswer::silent(ShopDeclined::BrowsingAnother {
                keeper: open.keeper,
            });
        }
        let browsing = Browsing {
            keeper: target,
            npc_vnum: npc.vnum,
            x: npc.x,
            y: npc.y,
            tripled: !self.shop_price_3x_disabled && mover.empire != npc.empire,
        };
        let window = shop_window(target, shop, browsing.tripled);
        let _replaced = self.browsing.insert(vid, browsing);
        ShopAnswer::Sent(vec![window])
    }

    /// `CShopManager::Buy` and `CShop::Buy` for an NPC's shop.
    fn buy(
        &mut self,
        vid: Vid,
        pos: u8,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<ShopAnswer, MoveItemRefused> {
        let browsing = match self.trading(vid, place, mover) {
            Ok(browsing) => browsing,
            Err(declined) => return Ok(declined),
        };
        self.set_shop_time(vid);
        let slot = usize::from(pos);
        if slot >= SHOP_HOST_ITEM_MAX_NUM {
            return Ok(ShopAnswer::Declined {
                reason: ShopDeclined::InvalidPos(pos),
                records: vec![GcShop::new(SHOP_SUBHEADER_GC_INVALID_POS).encode()],
            });
        }
        // An empty slot costs 0, which `CShop::Buy` answers as too little gold.
        let mut offer = self
            .shops
            .for_npc(browsing.npc_vnum)
            .and_then(|shop| shop.slots[slot])
            .unwrap_or(ShopSlot {
                vnum: 0,
                count: 0,
                price: 0,
            });
        offer.price = shown_price(offer.price, browsing.tripled);
        let questing = self.quest_running(vid);
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let Some(ids) = self.item_ids.as_mut() else {
            return Ok(refusal(ShopRefused::IdsExhausted, mover, &self.locale));
        };
        // A bought item goes whole to the base inventory, never to the belt's.
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: usable_inventory_cells(character.inven_point()),
            belt_grade: None,
            questing,
        };
        let gold = character.gold();
        match buy_item(
            character.items_mut(),
            ids,
            gold,
            offer,
            &self.protos,
            &rules,
        ) {
            Ok((done, left)) => Ok(ShopAnswer::Moved(paid(
                character,
                done,
                left,
                mover,
                &self.protos,
                &self.locale,
            ))),
            Err(refused) => Ok(refusal(refused, mover, &self.locale)),
        }
    }

    /// `CShopManager::Sell` to an NPC's shop.
    fn sell(
        &mut self,
        vid: Vid,
        cell: u16,
        count: u16,
        place: GroundPlace,
        mover: Mover,
    ) -> Result<ShopAnswer, MoveItemRefused> {
        if let Err(declined) = self.trading(vid, place, mover) {
            return Ok(declined);
        }
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let gold = character.gold();
        match sell_item(character.items_mut(), gold, cell, count, &self.protos) {
            Ok((done, held)) => Ok(ShopAnswer::Moved(paid(
                character,
                done,
                held,
                mover,
                &self.protos,
                &self.locale,
            ))),
            Err(refused) => Ok(refusal(refused, mover, &self.locale)),
        }
    }

    /// The keeper the character under `vid` trades with, or the answer when it has none in
    /// reach.
    fn trading(&self, vid: Vid, place: GroundPlace, mover: Mover) -> Result<Browsing, ShopAnswer> {
        let Some(browsing) = self.browsing.get(&vid).copied() else {
            return Err(ShopAnswer::silent(ShopDeclined::NotBrowsing));
        };
        let distance = distance_approx(
            place.x.saturating_sub(browsing.x),
            place.y.saturating_sub(browsing.y),
        );
        if distance > SHOP_TRADE_DISTANCE {
            return Err(ShopAnswer::Declined {
                reason: ShopDeclined::TooFar { distance },
                records: vec![notice(TOO_FAR_NOTICE, mover, &self.locale)],
            });
        }
        Ok(browsing)
    }

    /// `StopShopping` and `CShop::RemoveGuest`.
    fn close_shop(&mut self, vid: Vid) -> ShopAnswer {
        if self.browsing.remove(&vid).is_some() {
            self.set_shop_time(vid);
            ShopAnswer::Sent(vec![GcShop::new(SHOP_SUBHEADER_GC_END).encode()])
        } else {
            ShopAnswer::silent(ShopDeclined::NotBrowsing)
        }
    }
}

/// The price the window shows, tripled for a stranger.
const fn shown_price(price: u64, tripled: bool) -> u64 {
    if tripled {
        price.saturating_mul(3)
    } else {
        price
    }
}

/// `SHOP_SUBHEADER_GC_START` with each slot of `shop` for the keeper `keeper`.
fn shop_window(keeper: u32, shop: &NpcShop, tripled: bool) -> Vec<u8> {
    let mut items = [GcShopItem::default(); SHOP_HOST_ITEM_MAX_NUM];
    for (item, slot) in items.iter_mut().zip(&shop.slots) {
        if let Some(slot) = slot {
            *item = GcShopItem::new(slot.vnum, shown_price(slot.price, tripled), slot.count);
        }
    }
    GcShopStart::new(keeper, items).encode()
}

/// The world's refusal, with the `GC_SHOP` answer a buy gives or the line a sale gives.
fn refusal(refused: ShopRefused, mover: Mover, locale: &LocaleStrings) -> ShopAnswer {
    let mut records = Vec::new();
    if let Some(subheader) = refused.subheader() {
        records.push(GcShop::new(subheader).encode());
    }
    if let Some(text) = refused.notice() {
        records.push(notice(text, mover, locale));
    }
    ShopAnswer::Declined {
        reason: ShopDeclined::Refused(refused),
        records,
    }
}

/// A shop opened while the safebox is open (`shop_manager.cpp:124-127`).
pub const SAFEBOX_OPEN_NOTICE: &str = "[LS;876]";

/// A `CHAT_TYPE_INFO` line to the mover.
pub(super) fn notice(text: &str, mover: Mover, locale: &LocaleStrings) -> Vec<u8> {
    chat_packet(
        mover.recipient(locale),
        CHAT_TYPE_INFO,
        text.as_bytes(),
        &[],
    )
}

/// The move a buy or a sale made, with the character's gold set to `gold`.
///
/// Neither changes a point, so the move carries none. The move carries the change to the gold,
/// which its Transfer adds to the stored gold.
fn paid(
    character: &mut Character,
    mut done: MoveDone,
    gold: u64,
    mover: Mover,
    protos: &ItemProtos,
    locale: &LocaleStrings,
) -> MovedItems {
    let change = gold_delta(character.gold(), gold);
    character.set_gold(gold);
    let (items, slots) = character.items_and_quickslots_mut();
    sync_quickslots(&mut done, slots, items);
    let quickslots = slots.clone();
    let owner_id = character.player_id();
    let mut step = MovedItems::new(owner_id, character.vid().raw(), done, mover, protos, locale);
    step.quickslots = Some(quickslots);
    step.gold = Some(change);
    step
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::item_slots::EWindows;
    use db::items::RowChange;
    use gamedata::npc_shop::shops_from_dump;
    use protocol::gc_actors::GcCharacterGoldChange;
    use protocol::gc_item_window::HEADER_GC_ITEM_SET;
    use protocol::gc_script::GcScript;
    use protocol::gc_shop::GC_SHOP_START_WIRE_SIZE;
    use protocol::item_pos::ItemPos;
    use tokio::sync::oneshot;
    use world::character::{MoveKind, Quickslot, Quickslots, Side, GOLD_MAX_MAX};
    use world::item::{Item, ItemIdRange, ITEM_FLAG_STACKABLE};
    use world::npc::{MapNpcs, Npc};

    use crate::client_registry::ClientOutbox;
    use crate::game_loop_messages::GameCommand;
    use crate::game_state::SafeboxStep;

    const SHOPPER: Vid = Vid::new(7);
    /// 9007, whose shop sells weapons.
    const WEAPONS: u32 = 0x8000_0001;
    /// 9004, whose shop sells fireworks 100 at a time.
    const FIREWORKS: u32 = 0x8000_0002;
    /// 20086, which a shop names but whose click is a talk.
    const TALKER: u32 = 0x8000_0003;
    /// 9002, whose click is a shop's but whom no shop names.
    const BARE: u32 = 0x8000_0004;
    /// 9007 on map 42.
    const ELSEWHERE: u32 = 0x8000_0005;
    /// 9007 on Channel 2.
    const OTHER_CHANNEL: u32 = 0x8000_0006;
    /// A VID no NPC has.
    const NOBODY: u32 = 0x8000_0099;
    /// Where every keeper stands.
    const SPOT: i32 = 10_000;
    /// The id of the first item the shopper holds, past every id a buy here allocates.
    const HELD: u32 = 5_000_000;
    /// The small red potion, stackable, whose `shop_buy_price` is 30.
    const POTION: u32 = 27_001;

    fn keeper(vid: u32, vnum: u32, on_click: u8) -> Npc {
        Npc {
            vid,
            vnum,
            race: u16::try_from(vnum).unwrap(),
            char_type: 1,
            on_click,
            x: SPOT,
            y: SPOT,
            z: 0,
            rotation: 0,
            empire: 1,
            moving_speed: 0,
            attack_speed: 0,
            name: Vec::new(),
        }
    }

    fn standing(npcs: Vec<Npc>) -> Arc<MapNpcs> {
        Arc::new(MapNpcs {
            npcs,
            positions: Vec::new(),
        })
    }

    /// The owner's protos and shops, keepers of empire 1 on Channel 1's map 41 and elsewhere,
    /// and a shopper online at VID 7 holding `gold` and `items`.
    fn a_market(gold: u64, items: &[(ItemPos, Item)], ids: bool) -> GameState {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy");
        let protos = ItemProtos::load(&root.join("gamedata/proto")).unwrap();
        let dump = std::fs::read(root.join("sql/gamedata/player.sql")).unwrap();
        let shops = NpcShops::lay_out(&shops_from_dump(&dump).unwrap(), &protos);
        let mut state = GameState::new(protos).with_npc_shops(Arc::new(shops));
        if ids {
            let _ids = state
                .install_item_ids(ItemIdRange::new(1, 1_000_000, 1).unwrap())
                .unwrap();
        }
        let here = vec![
            keeper(WEAPONS, 9007, 1),
            keeper(FIREWORKS, 9004, 1),
            keeper(TALKER, 20086, 2),
            keeper(BARE, 9002, 1),
        ];
        let _here = state.npcs.insert((1, 41), standing(here));
        let there = standing(vec![keeper(ELSEWHERE, 9007, 1)]);
        let _there = state.npcs.insert((1, 42), there);
        let other = standing(vec![keeper(OTHER_CHANNEL, 9007, 1)]);
        let _other = state.npcs.insert((2, 41), other);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .enter_world_with_items(SHOPPER, 7, "Shaman", items, ClientOutbox::new(tx))
            .unwrap();
        state
            .characters
            .find_by_vid_mut(SHOPPER)
            .unwrap()
            .set_gold(gold);
        state
    }

    /// `dx` east of the keepers on `channel`'s map `map`.
    const fn on(channel: u8, map: i32, dx: i32) -> GroundPlace {
        GroundPlace {
            channel,
            map,
            x: SPOT + dx,
            y: SPOT,
        }
    }

    /// `dx` east of the keepers on Channel 1's map 41.
    const fn at(dx: i32) -> GroundPlace {
        on(1, 41, dx)
    }

    /// `dy` south of the keepers on Channel 1's map 41.
    const fn below(dy: i32) -> GroundPlace {
        GroundPlace {
            channel: 1,
            map: 41,
            x: SPOT,
            y: SPOT + dy,
        }
    }

    const fn of(empire: u8) -> Mover {
        Mover {
            recently_fought: false,
            empire,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
        }
    }

    fn step(state: &mut GameState, step: ShopStep, place: GroundPlace) -> ShopAnswer {
        state.shop(SHOPPER, step, place, of(1)).unwrap()
    }

    fn click(state: &mut GameState, target: u32, place: GroundPlace) -> ShopAnswer {
        step(state, ShopStep::Click { target }, place)
    }

    fn buy(state: &mut GameState, pos: u8) -> ShopAnswer {
        step(state, ShopStep::Buy { pos }, at(0))
    }

    fn sell(state: &mut GameState, cell: u16, count: u16) -> ShopAnswer {
        step(state, ShopStep::Sell { cell, count }, at(0))
    }

    fn window(answer: ShopAnswer) -> GcShopStart {
        match answer {
            ShopAnswer::Sent(records) => {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].len(), GC_SHOP_START_WIRE_SIZE);
                GcShopStart::decode(&records[0]).unwrap()
            }
            other => panic!("the window did not open: {other:?}"),
        }
    }

    fn open(state: &mut GameState, keeper: u32) -> GcShopStart {
        window(click(state, keeper, at(0)))
    }

    fn moved(answer: ShopAnswer) -> MovedItems {
        match answer {
            ShopAnswer::Moved(moved) => moved,
            other => panic!("nothing moved: {other:?}"),
        }
    }

    /// A refused buy, answered with `GC_SHOP` and `subheader`.
    fn answered(refused: ShopRefused, subheader: u8) -> ShopAnswer {
        ShopAnswer::Declined {
            reason: ShopDeclined::Refused(refused),
            records: vec![GcShop::new(subheader).encode()],
        }
    }

    /// A `CHAT_TYPE_INFO` line of `text` to the shopper.
    fn line(state: &GameState, text: &[u8]) -> Vec<u8> {
        chat_packet(of(1).recipient(&state.locale), CHAT_TYPE_INFO, text, &[])
    }

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(EWindows::Inventory as u8, cell)
    }

    fn potions(id: u32, count: u16) -> Item {
        let mut item = Item::new(id, POTION);
        item.set_size(1).unwrap();
        item.flags = ITEM_FLAG_STACKABLE;
        item.count = count;
        item
    }

    fn shopper(state: &mut GameState) -> &mut Character {
        state.characters.find_by_vid_mut(SHOPPER).unwrap()
    }

    fn gold(state: &GameState) -> u64 {
        state.characters.find_by_vid(SHOPPER).unwrap().gold()
    }

    /// The vnum and count in inventory cell `cell`.
    fn held(state: &GameState, cell: u16) -> Option<(u32, u16)> {
        let character = state.characters.find_by_vid(SHOPPER).unwrap();
        let item = character.items().item_at(inventory(cell))?;
        Some((item.vnum, item.count))
    }

    fn gold_change(value: u64, amount: i64) -> Vec<u8> {
        let mut frame = Vec::new();
        GcCharacterGoldChange::new(7, amount, value).encode_into(&mut frame);
        frame
    }

    #[test]
    fn each_shop_record_asks_for_its_step() {
        assert_eq!(ShopStep::requested(CgShop::End), Some(ShopStep::End));
        assert_eq!(
            ShopStep::requested(CgShop::Buy { count: 7, pos: 3 }),
            Some(ShopStep::Buy { pos: 3 }),
            "the count is not read"
        );
        assert_eq!(
            ShopStep::requested(CgShop::Sell { cell: 200 }),
            Some(ShopStep::Sell {
                cell: 200,
                count: 0
            }),
            "SELL sells the whole stack"
        );
        assert_eq!(
            ShopStep::requested(CgShop::Sell2 {
                slot: 201,
                padding: 0xcc,
                count: 0x0102
            }),
            Some(ShopStep::Sell {
                cell: 201,
                count: 0x0102
            })
        );
        assert_eq!(ShopStep::requested(CgShop::Unknown { subheader: 9 }), None);
    }

    #[test]
    fn a_click_on_a_keeper_in_reach_opens_its_shop_window() {
        let mut state = a_market(0, &[], true);
        assert_eq!(distance_approx(1040, 0), 999);
        let weapons = window(click(&mut state, WEAPONS, at(1040)));
        assert_eq!(weapons.owner_vid, WEAPONS);
        assert_eq!(weapons.items[0], GcShopItem::new(3100, 110_000, 1));
        assert_eq!(weapons.items[1], GcShopItem::new(5020, 3000, 1));
        assert_eq!(weapons.items[5], GcShopItem::default(), "3100's own cell");
        assert_eq!(weapons.items[13], GcShopItem::new(7100, 110_000, 1));
        assert_eq!(weapons.items[39], GcShopItem::default());
        assert!(state.browsing.contains_key(&SHOPPER));
        let mut south = a_market(0, &[], true);
        assert_eq!(window(click(&mut south, WEAPONS, below(-1040))), weapons);
        let fireworks = open(&mut a_market(0, &[], true), FIREWORKS);
        assert_eq!(fireworks.owner_vid, FIREWORKS);
        assert_eq!(fireworks.items[5], GcShopItem::new(50105, 500_000, 100));
    }

    #[test]
    fn a_keeper_a_thousand_away_or_farther_opens_nothing() {
        let mut state = a_market(10_000, &[], true);
        assert_eq!(distance_approx(1041, 0), 1000);
        let edge = |x| GroundPlace { x, ..at(0) };
        for place in [
            at(1041),
            at(-1041),
            below(1041),
            below(-1041),
            edge(i32::MAX),
            edge(i32::MIN),
        ] {
            let distance =
                distance_approx(place.x.saturating_sub(SPOT), place.y.saturating_sub(SPOT));
            assert_eq!(
                click(&mut state, WEAPONS, place),
                ShopAnswer::silent(ShopDeclined::OutOfReach { distance }),
                "{place:?}"
            );
            assert!(distance >= 1000);
        }
        assert!(state.browsing.is_empty());
        assert_eq!(
            buy(&mut state, 1),
            ShopAnswer::silent(ShopDeclined::NotBrowsing)
        );
    }

    #[test]
    fn a_stranger_is_shown_and_charged_each_price_tripled_unless_that_is_disabled() {
        for (disabled, empire, price) in [
            (false, 1, 3000),
            (false, 2, 9000),
            (false, 3, 9000),
            (true, 1, 3000),
            (true, 2, 3000),
        ] {
            let mut state = a_market(9000, &[], true).with_shop_price_3x_disabled(disabled);
            let click = ShopStep::Click { target: WEAPONS };
            let opened = state.shop(SHOPPER, click, at(0), of(empire)).unwrap();
            let shown = window(opened);
            assert_eq!(shown.items[1].price, price, "{disabled} {empire}");
            assert_eq!(shown.items[0].price, price / 3000 * 110_000);
            let paid = ShopStep::Buy { pos: 1 };
            let bought = moved(state.shop(SHOPPER, paid, at(0), of(empire)).unwrap());
            assert_eq!(bought.gold, Some(-i64::try_from(price).unwrap()));
            assert_eq!(
                gold(&state),
                9000 - price,
                "the price shown is the price paid"
            );
        }
        let mut short = a_market(8999, &[], true);
        let click = ShopStep::Click { target: WEAPONS };
        let _window = window(short.shop(SHOPPER, click, at(0), of(2)).unwrap());
        let paid = ShopStep::Buy { pos: 1 };
        assert_eq!(
            short.shop(SHOPPER, paid, at(0), of(2)).unwrap(),
            answered(ShopRefused::NotEnoughMoney, 5)
        );
        assert_eq!((gold(&short), held(&short, 0)), (8999, None));
    }

    #[test]
    fn only_a_keeper_on_the_characters_own_channel_and_map_answers() {
        let mut state = a_market(0, &[], true);
        for target in [NOBODY, ELSEWHERE, OTHER_CHANNEL] {
            assert_eq!(
                click(&mut state, target, at(0)),
                ShopAnswer::silent(ShopDeclined::NoSuchNpc { target })
            );
        }
        let elsewhere = step(
            &mut state,
            ShopStep::Click { target: WEAPONS },
            on(1, 42, 0),
        );
        assert_eq!(
            elsewhere,
            ShopAnswer::silent(ShopDeclined::NoSuchNpc { target: WEAPONS })
        );
        assert!(state.browsing.is_empty());
        let there = step(
            &mut state,
            ShopStep::Click { target: ELSEWHERE },
            on(1, 42, 0),
        );
        assert_eq!(window(there).owner_vid, ELSEWHERE);
        let mut other = a_market(0, &[], true);
        let click = ShopStep::Click {
            target: OTHER_CHANNEL,
        };
        let answer = step(&mut other, click, on(2, 41, 0));
        assert_eq!(window(answer).owner_vid, OTHER_CHANNEL);
    }

    #[test]
    fn a_click_is_checked_in_legacys_order() {
        let far = at(5000);
        let out_of_reach = ShopAnswer::silent(ShopDeclined::OutOfReach {
            distance: distance_approx(5000, 0),
        });
        let mut state = a_market(10_000, &[], true);
        // The trigger comes before the distance, and the distance before the shop.
        let talk = ShopAnswer::silent(ShopDeclined::NotAShop { on_click: 2 });
        assert_eq!(click(&mut state, TALKER, far), talk);
        assert_eq!(click(&mut state, BARE, far), out_of_reach);
        let bare = ShopAnswer::silent(ShopDeclined::NoShop { vnum: 9002 });
        assert_eq!(click(&mut state, BARE, at(0)), bare);
        let _window = open(&mut state, WEAPONS);
        // The keeper browsed comes before the distance, and every other refusal before the
        // other keeper.
        let again = ShopAnswer::silent(ShopDeclined::AlreadyBrowsing);
        assert_eq!(click(&mut state, WEAPONS, far), again);
        assert_eq!(click(&mut state, WEAPONS, at(0)), again);
        assert_eq!(click(&mut state, TALKER, at(0)), talk);
        assert_eq!(click(&mut state, FIREWORKS, far), out_of_reach);
        assert_eq!(click(&mut state, BARE, at(0)), bare);
        let another = ShopAnswer::silent(ShopDeclined::BrowsingAnother { keeper: WEAPONS });
        assert_eq!(click(&mut state, FIREWORKS, at(0)), another);
        // A refused open leaves the window as it was.
        let bought = moved(buy(&mut state, 1));
        assert_eq!(bought.gold, Some(-3000));
        assert_eq!(held(&state, 0), Some((5020, 1)));
    }

    /// Open the shopper's safebox, or its mall, with nothing in it.
    fn open_store(state: &mut GameState, mall: bool) {
        let account = 3;
        let items = Vec::new();
        let steps = if mall {
            [
                SafeboxStep::BeginMall,
                SafeboxStep::OpenMall { account, items },
            ]
        } else {
            [SafeboxStep::BeginOpen, SafeboxStep::Open { account, items }]
        };
        for step in steps {
            let _answer = state.safebox(SHOPPER, step, of(1)).unwrap();
        }
    }

    #[test]
    fn an_open_safebox_refuses_a_shop_before_its_distance_and_the_mall_does_not() {
        let mut state = a_market(10_000, &[], true);
        open_store(&mut state, false);
        let refused = ShopAnswer::Declined {
            reason: ShopDeclined::SafeboxOpen,
            records: vec![line(&state, SAFEBOX_OPEN_NOTICE.as_bytes())],
        };
        assert_eq!(click(&mut state, WEAPONS, at(5000)), refused);
        assert_eq!(click(&mut state, WEAPONS, at(0)), refused);
        let talk = ShopAnswer::silent(ShopDeclined::NotAShop { on_click: 2 });
        assert_eq!(
            click(&mut state, TALKER, at(0)),
            talk,
            "the trigger comes first"
        );
        assert!(state.browsing.is_empty());
        let _closed = state.safebox(SHOPPER, SafeboxStep::Close, of(1)).unwrap();
        open_store(&mut state, true);
        let _window = open(&mut state, WEAPONS);
    }

    #[test]
    fn a_buy_pays_the_price_and_takes_the_first_free_cell() {
        let mut state = a_market(10_000, &[(inventory(0), potions(HELD, 5))], true);
        let _window = open(&mut state, WEAPONS);
        let bought = moved(buy(&mut state, 1));
        assert_eq!(bought.kind, MoveKind::Bought);
        assert_eq!(bought.owner_id, 7);
        assert_eq!(bought.records.len(), 2);
        assert_eq!(bought.records[0], gold_change(7000, 0));
        assert_eq!(bought.records[1][0], HEADER_GC_ITEM_SET);
        let [RowChange::Created(row)] = bought.changes.as_slice() else {
            panic!("the buy made no item: {:?}", bought.changes);
        };
        assert_eq!(
            (row.window_type, row.pos, row.vnum, row.count),
            (EWindows::Inventory as u8, 1, 5020, 1)
        );
        assert!(bought.around.is_empty());
        assert_eq!(bought.points, None);
        assert_eq!(bought.quickslots, Some(Quickslots::default()));
        assert_eq!(bought.gold, Some(-3000));
        assert_eq!(gold(&state), 7000);
        assert_eq!(held(&state, 0), Some((POTION, 5)));
        assert_eq!(held(&state, 1), Some((5020, 1)));
        let again = moved(buy(&mut state, 1));
        assert_eq!(again.records[0], gold_change(4000, 0));
        assert_eq!(held(&state, 2), Some((5020, 1)));
    }

    #[test]
    fn a_buy_from_more_than_two_thousand_away_is_refused_with_a_line() {
        let stock = [(inventory(3), potions(HELD, 200))];
        let mut state = a_market(10_000, &stock, true);
        let _window = open(&mut state, WEAPONS);
        assert_eq!(distance_approx(2083, 0), 2001);
        let too_far = ShopAnswer::Declined {
            reason: ShopDeclined::TooFar { distance: 2001 },
            records: vec![line(&state, b"[LS;877]")],
        };
        for place in [at(2083), below(-2083)] {
            let bought = step(&mut state, ShopStep::Buy { pos: 1 }, place);
            assert_eq!(bought, too_far);
            let sold = step(&mut state, ShopStep::Sell { cell: 3, count: 0 }, place);
            assert_eq!(sold, too_far);
        }
        assert_eq!(held(&state, 0), None);
        assert_eq!(
            (gold(&state), held(&state, 3)),
            (10_000, Some((POTION, 200)))
        );
        assert_eq!(distance_approx(2082, 0), 2000);
        let _bought = moved(step(&mut state, ShopStep::Buy { pos: 1 }, at(2082)));
        let _sold = moved(step(
            &mut state,
            ShopStep::Sell { cell: 3, count: 1 },
            below(2082),
        ));
        assert_eq!(held(&state, 0), Some((5020, 1)));
        assert_eq!(held(&state, 3), Some((POTION, 199)));
    }

    #[test]
    fn without_a_window_a_buy_or_a_sale_is_ignored() {
        let mut state = a_market(10_000, &[(inventory(3), potions(HELD, 200))], true);
        let requests = [
            ShopStep::Buy { pos: 1 },
            ShopStep::Buy { pos: 40 },
            ShopStep::Sell { cell: 3, count: 0 },
        ];
        for request in requests {
            assert_eq!(
                step(&mut state, request, at(0)),
                ShopAnswer::silent(ShopDeclined::NotBrowsing)
            );
        }
        assert_eq!(held(&state, 0), None);
        assert_eq!(
            (gold(&state), held(&state, 3)),
            (10_000, Some((POTION, 200)))
        );
    }

    #[test]
    fn a_buy_past_the_window_or_of_an_empty_slot_is_refused() {
        let mut state = a_market(1_000_000, &[], true);
        let _window = open(&mut state, WEAPONS);
        for pos in [40, 41, 255] {
            assert_eq!(
                buy(&mut state, pos),
                ShopAnswer::Declined {
                    reason: ShopDeclined::InvalidPos(pos),
                    records: vec![vec![38, 4, 0, 8]],
                }
            );
        }
        // An empty slot costs nothing, which is too little.
        for pos in [5, 14, 39] {
            let refused = answered(ShopRefused::NotEnoughMoney, 5);
            assert_eq!(buy(&mut state, pos), refused);
        }
        assert_eq!((gold(&state), held(&state, 0)), (1_000_000, None));
    }

    #[test]
    fn a_buy_needs_the_whole_price() {
        let mut state = a_market(2999, &[], true);
        let _window = open(&mut state, WEAPONS);
        let refused = answered(ShopRefused::NotEnoughMoney, 5);
        assert_eq!(buy(&mut state, 1), refused);
        assert_eq!((gold(&state), held(&state, 0)), (2999, None));
        shopper(&mut state).set_gold(3000);
        assert_eq!(moved(buy(&mut state, 1)).gold, Some(-3000));
        assert_eq!((gold(&state), held(&state, 0)), (0, Some((5020, 1))));
    }

    #[test]
    fn a_buy_needs_a_free_cell_of_the_unlocked_inventory() {
        let full: Vec<_> = (0..90)
            .map(|cell| (inventory(cell), potions(HELD + u32::from(cell), 1)))
            .collect();
        let mut state = a_market(10_000, &full, true);
        let _window = open(&mut state, WEAPONS);
        let refused = answered(ShopRefused::InventoryFull, 7);
        assert_eq!(buy(&mut state, 1), refused);
        assert_eq!((gold(&state), held(&state, 90)), (10_000, None));
        // Another row unlocked holds it.
        shopper(&mut state).set_inven_point(1);
        let bought = moved(buy(&mut state, 1));
        assert_eq!(bought.gold, Some(-3000));
        assert_eq!(held(&state, 90), Some((5020, 1)));
    }

    #[test]
    fn without_item_ids_a_buy_is_sold_out() {
        let mut state = a_market(10_000, &[], false);
        let _window = open(&mut state, WEAPONS);
        let refused = answered(ShopRefused::IdsExhausted, 9);
        assert_eq!(buy(&mut state, 1), refused);
        assert_eq!((gold(&state), held(&state, 0)), (10_000, None));
    }

    #[test]
    fn a_stack_bought_is_cut_to_the_count_limit() {
        for (limit, count) in [(200, 100), (100, 100), (40, 40)] {
            let mut state = a_market(500_000, &[], true).with_item_count_limit(limit);
            let _window = open(&mut state, FIREWORKS);
            let _bought = moved(buy(&mut state, 0));
            assert_eq!(held(&state, 0), Some((50_100, count)), "{limit}");
            assert_eq!(
                gold(&state),
                0,
                "the stack costs the same however big it is"
            );
        }
    }

    #[test]
    fn a_sale_pays_the_price_less_the_tax() {
        let mut state = a_market(1000, &[(inventory(3), potions(HELD, 200))], true);
        let mut slots = Quickslots::default();
        assert!(slots.set(0, Quickslot { kind: 1, pos: 3 }, &mut Vec::new()));
        shopper(&mut state).set_quickslots(slots.clone());
        let _window = open(&mut state, WEAPONS);
        // 50 potions of 30 are 1500, a fifth of it 300, and 3 per cent of that 9.
        let sold = moved(sell(&mut state, 3, 50));
        assert_eq!(sold.kind, MoveKind::Sold);
        assert_eq!(sold.records.len(), 3);
        assert_eq!(sold.records[0], line(&state, b"[LS;881;3]"));
        assert_eq!(sold.records[2], gold_change(1291, 291));
        let kept = RowChange::Count {
            id: HELD,
            count: 150,
        };
        assert_eq!(sold.changes, vec![kept]);
        assert_eq!(sold.gold, Some(291));
        assert_eq!(sold.quickslots.as_ref(), Some(&slots));
        assert_eq!((gold(&state), held(&state, 3)), (1291, Some((POTION, 150))));
        // More than the stack holds sells the stack, and its quickslot goes with it.
        let rest = moved(sell(&mut state, 3, 151));
        assert_eq!(rest.changes, vec![RowChange::Destroyed { id: HELD }]);
        assert_eq!(rest.records.first(), sold.records.first());
        assert!(rest.records.contains(&vec![29, 0]), "{:?}", rest.records);
        assert_eq!(rest.records.last(), Some(&gold_change(2164, 873)));
        assert_eq!(rest.gold, Some(873));
        assert_eq!(rest.quickslots, Some(Quickslots::default()));
        let character = state.characters.find_by_vid(SHOPPER).unwrap();
        assert_eq!(character.quickslots(), &Quickslots::default());
        assert_eq!((gold(&state), held(&state, 3)), (2164, None));
    }

    #[test]
    fn a_count_of_nothing_sells_the_whole_stack() {
        let mut state = a_market(0, &[(inventory(3), potions(HELD, 200))], true);
        let _window = open(&mut state, WEAPONS);
        let sold = moved(sell(&mut state, 3, 0));
        assert_eq!(sold.gold, Some(1164));
        assert_eq!(gold_delta(1164, 0), -1164);
        assert_eq!(gold_delta(0, u64::MAX), i64::MAX);
        assert_eq!(gold_delta(u64::MAX, 0), i64::MIN);
        assert_eq!((gold(&state), held(&state, 3)), (1164, None));
    }

    #[test]
    fn a_sale_of_nothing_of_a_worn_item_or_past_the_gold_cap_is_refused() {
        let mut armour = Item::new(HELD + 1, 11_901);
        armour.set_size(2).unwrap();
        let items = [(inventory(3), potions(HELD, 200)), (inventory(180), armour)];
        let mut state = a_market(GOLD_MAX_MAX - 291, &items, true);
        let _window = open(&mut state, WEAPONS);
        let quiet = |refused| ShopAnswer::silent(ShopDeclined::Refused(refused));
        assert_eq!(sell(&mut state, 50, 0), quiet(ShopRefused::Empty));
        let worn = ShopAnswer::Declined {
            reason: ShopDeclined::Refused(ShopRefused::Worn),
            records: vec![line(&state, b"[LS;1059]")],
        };
        assert_eq!(sell(&mut state, 180, 0), worn);
        let overflow = ShopRefused::GoldOverflow {
            gold: GOLD_MAX_MAX - 291,
            price: 291,
        };
        assert_eq!(sell(&mut state, 3, 50), quiet(overflow));
        assert_eq!(gold(&state), GOLD_MAX_MAX - 291);
        assert_eq!(held(&state, 3), Some((POTION, 200)));
        assert_eq!(held(&state, 180), Some((11_901, 1)));
        shopper(&mut state).set_gold(GOLD_MAX_MAX - 292);
        let _sold = moved(sell(&mut state, 3, 50));
        assert_eq!(gold(&state), GOLD_MAX_MAX - 1);
    }

    #[test]
    fn closing_the_window_ends_the_browsing() {
        let mut state = a_market(10_000, &[], true);
        let none = ShopAnswer::silent(ShopDeclined::NotBrowsing);
        assert_eq!(step(&mut state, ShopStep::End, at(0)), none);
        let _window = open(&mut state, WEAPONS);
        let closed = step(&mut state, ShopStep::End, at(5000));
        assert_eq!(closed, ShopAnswer::Sent(vec![vec![38, 4, 0, 1]]));
        assert_eq!(buy(&mut state, 1), none);
        assert_eq!(step(&mut state, ShopStep::End, at(0)), none);
        assert_eq!(open(&mut state, FIREWORKS).owner_vid, FIREWORKS);
    }

    /// `CHARACTER::OnClick` asks the quests first (`G/char.cpp:6181-6352`): a keeper whose quest
    /// takes the click opens no shop, and while that script waits the click is no quest's and
    /// the shop opens. The shop reads the keeper's vnum and the quest its race, and no keeper of
    /// the owner's has a quest, so a 9007 of the OX manager's race stands in for one.
    #[test]
    fn a_keeper_whose_quest_takes_the_click_opens_no_shop() {
        /// 9007 of the OX manager's race.
        const SCRIPTED: u32 = 0x8000_0007;
        let mut state = a_market(0, &[], true);
        let scripted = Npc {
            race: 20_011,
            ..keeper(SCRIPTED, 9007, 1)
        };
        let _here = state
            .npcs
            .insert((1, 41), standing(vec![keeper(WEAPONS, 9007, 1), scripted]));
        state.start_a_quest(SHOPPER);
        state.end_the_quest(SHOPPER);
        let mut menu = Vec::new();
        GcScript::new(1, b"[QUESTION 1;OX Contest |2;Inchide]".to_vec())
            .encode_into(&mut menu)
            .unwrap();
        let declined = ShopAnswer::Declined {
            reason: ShopDeclined::Quest,
            records: vec![menu],
        };
        assert_eq!(click(&mut state, SCRIPTED, at(0)), declined);
        assert!(state.browsing.is_empty());
        assert_eq!(
            window(click(&mut state, SCRIPTED, at(0))).owner_vid,
            SCRIPTED
        );
        assert!(state.browsing.contains_key(&SHOPPER));
    }

    #[test]
    fn leaving_the_world_ends_the_browsing() {
        let mut state = a_market(10_000, &[], true);
        let _window = open(&mut state, WEAPONS);
        let _departed = state.leave_world(SHOPPER).unwrap();
        assert!(state.browsing.is_empty());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        state
            .enter_world(SHOPPER, 7, "Shaman", ClientOutbox::new(tx))
            .unwrap();
        shopper(&mut state).set_gold(10_000);
        let none = ShopAnswer::silent(ShopDeclined::NotBrowsing);
        assert_eq!(buy(&mut state, 1), none);
    }

    #[test]
    fn a_step_for_nobody_online_is_refused() {
        let mut state = a_market(0, &[], true);
        let nobody = Vid::new(8);
        for request in [
            ShopStep::Click { target: WEAPONS },
            ShopStep::Buy { pos: 1 },
            ShopStep::Sell { cell: 0, count: 0 },
            ShopStep::End,
        ] {
            assert_eq!(
                state.shop(nobody, request, at(0), of(1)),
                Err(MoveItemRefused::NoSuchCharacter { vid: nobody })
            );
        }
        assert!(state.browsing.is_empty());
    }

    #[test]
    fn a_click_while_the_character_trades_is_ignored() {
        let mut state = a_market(0, &[], true);
        let _trading = state.trading.insert(SHOPPER, (7, Side::Asked));
        assert_eq!(
            click(&mut state, NOBODY, at(0)),
            ShopAnswer::silent(ShopDeclined::NoSuchNpc { target: NOBODY }),
            "the NPC is looked up first"
        );
        for keeper in [WEAPONS, TALKER] {
            assert_eq!(
                click(&mut state, keeper, at(0)),
                ShopAnswer::silent(ShopDeclined::Trading)
            );
        }
        assert!(state.browsing.is_empty());
        let _ended = state.trading.remove(&SHOPPER);
        assert_eq!(open(&mut state, WEAPONS).owner_vid, WEAPONS);
    }

    #[test]
    fn the_game_thread_runs_a_shop_step_and_answers_it() {
        let mut state = a_market(0, &[], true);
        let (reply, answer) = oneshot::channel();
        state.apply(GameCommand::Shop {
            vid: SHOPPER,
            step: ShopStep::Click { target: WEAPONS },
            place: at(0),
            mover: of(1),
            reply,
        });
        let opened = answer.blocking_recv().unwrap().unwrap();
        assert_eq!(window(opened).owner_vid, WEAPONS);
        let (reply, answer) = oneshot::channel();
        drop(answer);
        state.apply(GameCommand::Shop {
            vid: SHOPPER,
            step: ShopStep::End,
            place: at(0),
            mover: of(1),
            reply,
        });
        assert!(
            state.browsing.is_empty(),
            "the step runs though nobody is left to answer"
        );
    }
}
