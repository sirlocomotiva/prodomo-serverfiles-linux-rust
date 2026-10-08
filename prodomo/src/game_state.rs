//! The state the game thread owns: the characters, the item ids, and the item prototypes.
//!
//! ADR-0002 says one process hosts auth and every Channel, that each Channel is one
//! world, and that all worlds step on one game thread at 25 Pulses per second. The
//! thread existed before this module, but it owned nothing: `main.rs` started it
//! with `|_| {}`, so it ticked and discarded the count. This module is what the
//! thread holds.
//!
//! # Why the state is here and not in the accept loop
//!
//! A character is only touched by the game thread. The accept loop is Tokio-owned
//! and its descriptors are not reachable from the game thread yet, so nothing that
//! mutates a world may run there. Keeping the world in a value that the thread owns
//! outright, with no `Arc` and no lock, is what makes that checkable rather than a
//! convention: the only way to reach this state is a command, and commands are
//! drained between pulses.
//!
//! # What is deliberately absent
//!
//! There is no Channel map set. The NPCs the regen files stand up at boot are here, but they
//! never move or respawn; a click reaches their quests and then a keeper's shop, and a warp NPC
//! sends the players near it on. This holds the pieces the item path, the shops, the quests, the
//! warp NPCs and the map view need and nothing more, because each of the rest is a separate unit
//! with its own decision to record.
//! [`PulseProcessor::process_pulse`](crate::game_loop::PulseProcessor::process_pulse) steps
//! only the events those pieces have: the ground items, the recovery, the distant trades and the
//! warp NPCs.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::item_slots::usable_inventory_cells;
use gamedata::item_proto::ItemProtos;
use gamedata::locale_string::LocaleStrings;
use gamedata::map_atlas::MapRegion;
use gamedata::regen::RegenEntry;
use gamedata::server_attr::SectreeGrid;
use world::character::{
    drop_item, move_item, pickup_item, use_item, CharacterManager, CharacterManagerError, DropAt,
    Gear, GroundItem, MoveRefused, MoveRequest, MoveRules, Pcg32, Picker, Rejected, Side,
};
use world::item::{ItemIdRange, ItemIds};
use world::npc::{MapNpcs, NpcSpawner, NpcVidsExhausted};

use tokio::sync::{oneshot, watch};
use tracing::{debug, warn};

use crate::client_live::LiveClock;
use crate::client_registry::{ChannelClients, ClientOutbox};
use crate::game_loop::PulseProcessor;
use crate::game_loop_messages::{GameCommand, GroundPlace, Kept, Loaded, Shown};
use crate::item_grant::{grant_item, GrantOutcome, GrantRefusal, GrantRequest};
use crate::item_move::{belt_grade, move_facts, MoveItemRefused, MovedItems, Mover, StoreOrder};
use crate::quickslot::{QuickslotAnswer, QuickslotStep};
use crate::save::PASSES_PER_SEC;
use crate::sync_position::distance_approx;
use world::character::{add_from_client, sync_quickslots};

mod affect;
#[cfg(test)]
mod fixtures;
mod motion;
mod quests;
mod safebox;
mod shop;
mod sync;
mod trade;
mod view;
mod view_encode;
mod warp_npc;

pub use quests::{QuestStep, Quests};
pub use safebox::{
    SafeboxAnswer, SafeboxStep, ALREADY_OPEN_NOTICE, LOAD_WAIT_PULSES, MALL_WAIT_NOTICE,
    OTHER_WINDOW_NOTICE, SAFEBOX_WAIT_NOTICE, TRADE_WAIT_NOTICE, TRADE_WAIT_SECONDS,
};
pub use shop::{ShopAnswer, ShopDeclined, ShopStep, SAFEBOX_OPEN_NOTICE};
pub use trade::{
    TradeAnswer, TradeDeclined, TradeSettled, TradeStep, EXCHANGE_SUBHEADER_CG_ACCEPT,
    EXCHANGE_SUBHEADER_CG_CANCEL, EXCHANGE_SUBHEADER_CG_ELK_ADD, EXCHANGE_SUBHEADER_CG_ITEM_ADD,
    EXCHANGE_SUBHEADER_CG_ITEM_DEL, EXCHANGE_SUBHEADER_CG_START,
};

/// `PickupItem`'s `DistanceValid` bound (`G/item.cpp:593`), checked at `G/char_item.cpp:7982`.
const PICKUP_DISTANCE: i32 = 300;

/// An item lying on one map of one Channel.
#[derive(Debug)]
struct Lying {
    ground: GroundItem,
    channel: u8,
    map: i32,
    /// The pulse the destroy event fires on.
    expires: u64,
    /// Told once the drop that laid it has stored its rows, which each pick-up waits for.
    stored: watch::Receiver<bool>,
}

/// Counters the owning side can read while the game thread is running.
///
/// Shared rather than returned, because the state is **moved** into the thread and
/// cannot be read afterwards. That is the point of owning it: the only handle that
/// outlives the move is one the game thread agrees to update.
#[derive(Debug, Default)]
pub struct GameStateMetrics {
    pulses: AtomicU64,
}

impl GameStateMetrics {
    /// Pulses the game thread has stepped.
    ///
    /// Zero is the honest value before the thread is spawned, and it is what a
    /// caller sees if the thread never started. A test that waits for a non-zero
    /// value is waiting for the thread, not for the construction.
    pub fn pulses(&self) -> u64 {
        self.pulses.load(Ordering::SeqCst)
    }
}

/// An id range was installed over an allocator that is already installed.
///
/// Named rather than logged, because the consequence is a duplicate item id and no
/// store write will report it: the second allocator hands out the same numbers the
/// first one did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInstalled;

impl std::fmt::Display for AlreadyInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an item id allocator is already installed")
    }
}

impl std::error::Error for AlreadyInstalled {}

/// Why taking a granted item back out of the world did not succeed.
///
/// Every variant leaves the world unchanged, so a caller that gets one may retry or
/// report. [`RevokeRefused::Rejected`] is the one that matters most: it means the
/// storage named a cell and then refused to release it, which is a bug in the storage
/// rather than a state a caller can wait out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokeRefused {
    /// The character is not online, so the world holds nothing of theirs.
    NoSuchCharacter {
        /// The name the revoke named.
        name: String,
    },
    /// The world does not hold that item for that character.
    ///
    /// A bug rather than a state to recover from: the id came from a grant that mutated
    /// the world moments earlier, and only a pulse in between could have moved it.
    NotThere {
        /// The id the revoke named.
        id: u32,
    },
    /// The storage named a cell and would not release it.
    Rejected {
        /// The id the revoke named.
        id: u32,
        /// What the storage reported.
        error: Rejected,
    },
}

impl std::fmt::Display for RevokeRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchCharacter { name } => {
                write!(formatter, "no online character named {name}")
            }
            Self::NotThere { id } => write!(formatter, "the world does not hold item {id}"),
            Self::Rejected { id, error } => {
                write!(formatter, "item {id} could not be released: {error}")
            }
        }
    }
}

impl std::error::Error for RevokeRefused {}

/// Why the world refused to admit a live client's character.
///
/// Every variant leaves the world unchanged, so the descriptor can close without the
/// world and the client having disagreed about whether the character exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnterWorldRefused {
    /// Another character is already indexed under that VID.
    ///
    /// A bug rather than a state to recover from: a second client with the same wire
    /// VID would make every record the world sends go to whichever descriptor answered
    /// last, and a grant would land on the wrong player.
    DuplicateVid {
        /// The VID the client arrived with.
        vid: common::vid::Vid,
    },
    /// Another character is already indexed under that player id.
    DuplicatePlayerId {
        /// The player id the client arrived with.
        player_id: u32,
    },
    /// Another character already holds that Name, compared case-insensitively.
    DuplicateName {
        /// The Name the client arrived with.
        name: String,
    },
    /// The VID is zero, which is legacy's `VID::NULL`.
    ///
    /// Refused rather than stored, because a zero VID is what a record with no target
    /// carries, so a world that indexed it would answer a "broadcast to nobody" claim
    /// with a character.
    NullVid,
    /// The world could not be asked, because the manager refused for a reason the
    /// world has no name of its own for.
    ///
    /// Kept apart from the duplicates above on purpose. Those are states a caller
    /// could act on -- this name is taken, do not retry it -- and this one is a bug in
    /// the crossing. Reporting it as a name duplicate would send an operator looking
    /// at a player's Name when the real fault is in the world, and would hide the
    /// exhaustion case entirely, which is the one that needs attention.
    NotAdmitted {
        /// The manager's own reason, verbatim, so nothing is lost in translation.
        reason: String,
    },
    /// A loaded item would not go where the load placed it.
    ///
    /// The load placed the same items in the same order into an empty inventory, so
    /// this is a bug in the crossing rather than a state a player can reach. The
    /// character is taken back out, because a world holding it without that item
    /// would offer the item's cell to the next grant.
    ItemRefused {
        /// The item's id.
        id: u32,
        /// Why the inventory would not take it.
        reason: Rejected,
    },
}

impl std::fmt::Display for EnterWorldRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateVid { vid } => {
                write!(formatter, "the world already holds {vid}")
            }
            Self::DuplicatePlayerId { player_id } => {
                write!(formatter, "the world already holds player {player_id}")
            }
            Self::DuplicateName { name } => {
                write!(
                    formatter,
                    "the world already holds a character named {name}"
                )
            }
            Self::NullVid => {
                formatter.write_str("a character cannot enter the world with a null VID")
            }
            Self::NotAdmitted { reason } => {
                write!(formatter, "the world refused this character: {reason}")
            }
            Self::ItemRefused { id, reason } => {
                write!(formatter, "the world refused loaded item {id}: {reason}")
            }
        }
    }
}

impl EnterWorldRefused {
    /// Names the world-side reason in the caller's terms.
    ///
    /// The two null-VID checks are the same rule seen from two places, and both are
    /// kept: the world checks before it calls the manager so a refusal costs no
    /// counter value, and the manager checks because it is also reachable directly.
    /// Mapping them onto one variant means a caller cannot tell which check fired,
    /// which is fine, because the repair is the same in both cases.
    fn from_manager(error: CharacterManagerError, name: &str) -> Self {
        match error {
            CharacterManagerError::DuplicateVid(vid) => Self::DuplicateVid { vid },
            CharacterManagerError::DuplicatePlayerId(player_id) => {
                Self::DuplicatePlayerId { player_id }
            }
            // The manager's variant carries the name it refused. The caller's copy is
            // used instead so a caller that passed a different spelling sees its own,
            // which is what the log line will print.
            CharacterManagerError::DuplicatePlayerName(_) => Self::DuplicateName {
                name: name.to_owned(),
            },
            CharacterManagerError::NullVid => Self::NullVid,
            // The remaining variants are lookups and a counter exhaustion, none of
            // which `create_player_with_vid` can produce. They are carried through by
            // their own text rather than folded into a duplicate: `VidExhausted` in
            // particular is the one case here an operator has to act on, and calling
            // it a name duplicate would bury it and send the search to a player's
            // Name instead of to the world.
            other => Self::NotAdmitted {
                reason: other.to_string(),
            },
        }
    }
}

impl std::error::Error for EnterWorldRefused {}

/// A grant was asked for before the world had an allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoItemIds;

impl std::fmt::Display for NoItemIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the world has no item id allocator yet")
    }
}

impl std::error::Error for NoItemIds {}

/// The world-facing range for a range the store resolved.
///
/// The two range types are deliberately different: [`db::item_id_range::ItemIdRange`]
/// carries legacy's `dwMin`/`dwMax`/`dwUsableItemIDMin` and performs no checks, and
/// [`world::item::ItemIdRange`] refuses a range that would hand out id 0. The
/// conversion is here because `db` is below `world` and cannot see it, and a
/// `From` impl would be two foreign types. Both agree that `last` is never issued,
/// so the mapping is one to one.
///
/// # Errors
///
/// [`world::item::BadIdRange`] when `first_usable` is 0 or falls outside the span.
/// `db::items::resolve_item_id_range` cannot produce that for a span it accepted,
/// because it starts above the highest stored id, so this is a seam check rather than
/// an expected failure.
pub fn world_item_id_range(
    range: db::item_id_range::ItemIdRange,
) -> Result<world::item::ItemIdRange, world::item::BadIdRange> {
    world::item::ItemIdRange::new(range.min, range.max, range.usable_item_id_min)
}

/// The world state one game thread owns.
#[derive(Debug)]
pub struct GameState {
    characters: CharacterManager,
    item_ids: Option<ItemIds>,
    protos: Arc<ItemProtos>,
    /// The locale strings a chat line is looked up in.
    locale: Arc<LocaleStrings>,
    metrics: Arc<GameStateMetrics>,
    last_pulse: u64,
    /// Where each online character's records go, keyed by the VID it entered under.
    ///
    /// Separate from the [`CharacterManager`] on purpose. The manager owns gameplay
    /// state that only the game thread touches; this map owns a **sender**, which is
    /// the one thing here that is not world state and must not be walked as if it
    /// were. Keeping them apart also means a departed client is removed from one map
    /// and cannot leave a stale sender behind in the other.
    outboxes: HashMap<common::vid::Vid, ClientOutbox>,
    /// `g_bItemCountLimit`: the largest stack a merge may build.
    item_count_limit: u16,
    /// The draw a sash rolls its absorption from, and the draws the boot spawn places and turns
    /// the NPCs with.
    ///
    /// Legacy's `number()` draws from the process-wide `thecore_random`, which `srandom`
    /// seeds with the boot time; this one is seeded from the standard library's per-process
    /// hash keys. Either way a player cannot predict the draw, and the Rewrite does not
    /// reproduce legacy's sequence (a Divergence).
    dice: Pcg32,
    /// The items lying on the ground, keyed by their ground VID.
    ///
    /// Legacy holds them as `CItem` sectree entities, in memory only; a restart loses them,
    /// and so does this.
    ground: BTreeMap<u32, Lying>,
    /// The next ground VID, apart from the character VIDs as legacy's are. Legacy's
    /// `ITEM_MANAGER` counter numbers every item when it is created (`++m_dwVIDCount` in
    /// `ITEM_MANAGER::CreateItem`); a Divergence: this one numbers an item only when it is
    /// dropped.
    next_ground_vid: u32,
    /// `item_destroy_time_dropitem`, in pulses.
    drop_lifetime: u64,
    /// The Channel's client registry: who a trade or a warp NPC finds online.
    clients: Option<Arc<ChannelClients>>,
    /// The affect event of each character that has one (`m_pkAffectEvent`): the potion
    /// recovery, the stamina refill and the timed affects. See [`affect`].
    affect_events: HashMap<common::vid::Vid, affect::AffectEvent>,
    /// The number of the last affect event started, which orders two events due on one Pulse.
    affect_sequence: u64,
    /// The NPCs standing on each map of each Channel, keyed by (Channel, map index).
    ///
    /// Shared with the descriptor that shows them, because they never change once boot has
    /// stood them up: nothing moves, respawns or removes an NPC yet.
    npcs: BTreeMap<(u8, i32), Arc<MapNpcs>>,
    /// The NPC shops, by keeper vnum.
    shops: Arc<gamedata::npc_shop::NpcShops>,
    /// `g_bEmpireShopPriceTripleDisable`: a stranger's prices are not tripled.
    shop_price_3x_disabled: bool,
    /// The keeper each character browses, keyed by its VID.
    browsing: HashMap<common::vid::Vid, shop::Browsing>,
    /// The open trades, keyed by the VID of the character that started each.
    trades: BTreeMap<u32, trade::Deal>,
    /// The trade each trading character is in and its side of it, keyed by its VID.
    trading: HashMap<common::vid::Vid, (u32, Side)>,
    /// The safebox and mall of each character that has opened one, keyed by its VID.
    storages: HashMap<common::vid::Vid, safebox::Storage>,
    /// The quests boot loaded, when it did.
    quests: Option<Quests>,
    /// The warp and goto NPCs standing on each map of each Channel, keyed as `npcs` is.
    warp_npcs: BTreeMap<(u8, i32), Vec<warp_npc::WarpNpc>>,
    /// When each character last traded or shopped, keyed by its VID, which `IsHack` reads.
    portal_times: HashMap<common::vid::Vid, warp_npc::PortalTimes>,
    /// The sectrees, spots and views of each map of each Channel, keyed as `npcs` is.
    maps: BTreeMap<(u8, i32), view::MapIndex>,
    /// The body of every player standing on a map, keyed by its VID.
    bodies: HashMap<common::vid::Vid, motion::Body>,
    /// The players in the Move state, by VID: the ones each Pulse steps.
    movers: BTreeSet<u32>,
    /// `get_dword_time()`, shared with the connections in production.
    clock: motion::WorldClock,
    /// `VIEW_RANGE + VIEW_BONUS_RANGE`, the radius a view takes an entity in within.
    view_radius: i64,
    /// Where each NPC is listed: its map's key and its place in that map's list.
    npc_of: HashMap<u32, (u8, i32, usize)>,
}

/// An item the world has taken back, together with whose it was.
///
/// Both halves are needed together and neither can be recovered from the other. The
/// cell is the only way to find the row, and the owner is half of that row's key; a
/// caller that looked either one up again could get a different answer than the world
/// acted on, and would then delete a row for an item that is still somewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Released {
    /// The cell the item was in, which the world has just freed.
    pub pos: protocol::item_pos::ItemPos,
    /// The store `player.id` of the character it belonged to.
    ///
    /// The world admitted this character under that id, so it is the same number the
    /// grant's row carried and the same number the descriptor publishes as the VID.
    pub owner_id: u32,
}

impl GameState {
    /// Build a state from the Game data alone.
    ///
    /// `protos` is read before the thread starts, not inside it, so a missing or
    /// malformed proto file stops `serve` before any port opens. That ordering is
    /// the same one `load_atlas` already uses, and it is the Rewrite's own: legacy
    /// accepts clients before its DB boot finishes, which AGENTS.md records as a
    /// Defect.
    ///
    /// There is no id range and no way to give this constructor one, because the
    /// start id is `MAX(id)` over the item table and that table is not readable
    /// until the store has migrated -- which happens inside the accept loop, after
    /// the listeners are bound. The range arrives later as
    /// [`GameCommand::InstallItemIdRange`].
    ///
    /// The alternative was a range in this constructor, and there is no honest value
    /// to put there: any fixed start would either reissue an id a stored item holds
    /// or leave a gap. An `Option` that is honestly absent is the only state that
    /// does not invent one.
    ///
    /// The prototypes may arrive shared. The item load at character select reads the
    /// same table on the descriptor's task before the world is asked, and one table in
    /// two places cannot disagree about an item's size or flags.
    #[must_use]
    pub fn new(protos: impl Into<Arc<ItemProtos>>) -> Self {
        Self {
            characters: CharacterManager::new(),
            item_ids: None,
            protos: protos.into(),
            locale: Arc::default(),
            metrics: Arc::new(GameStateMetrics::default()),
            last_pulse: 0,
            outboxes: HashMap::new(),
            item_count_limit: common::item_slots::ITEM_COUNT_LIMIT,
            dice: Pcg32::new(RandomState::new().hash_one(0_u8), 0),
            ground: BTreeMap::new(),
            next_ground_vid: 1,
            drop_lifetime: drop_lifetime_pulses(DEFAULT_DROP_LIFETIME_SECS),
            clients: None,
            affect_events: HashMap::new(),
            affect_sequence: 0,
            npcs: BTreeMap::new(),
            shops: Arc::default(),
            shop_price_3x_disabled: false,
            browsing: HashMap::new(),
            trades: BTreeMap::new(),
            trading: HashMap::new(),
            storages: HashMap::new(),
            quests: None,
            warp_npcs: BTreeMap::new(),
            portal_times: HashMap::new(),
            maps: BTreeMap::new(),
            bodies: HashMap::new(),
            movers: BTreeSet::new(),
            clock: motion::WorldClock::default(),
            view_radius: view_radius_of(DEFAULT_VIEW_RANGE),
            npc_of: HashMap::new(),
        }
    }

    /// Read `get_dword_time()` from `clock`. Production passes the connections' clock, so the
    /// world and the connections share one epoch.
    #[must_use]
    pub fn with_clock(mut self, clock: Box<dyn LiveClock + Send>) -> Self {
        self.clock = motion::WorldClock::new(clock);
        self
    }

    /// Set `VIEW_RANGE`, the configured `game.view_range`; the view's radius is it plus
    /// `VIEW_BONUS_RANGE` (`G/entity_view.cpp:94`), in 64 bits so no range can overflow it.
    #[must_use]
    pub fn with_view_range(mut self, view_range: i32) -> Self {
        self.view_radius = view_radius_of(view_range);
        self
    }

    /// Hosts one map of one Channel over the sectrees `grid` builds (`SECTREE_MAP::Build`).
    /// A map hosted again keeps no spot or view; call this before anyone enters it.
    pub fn host_map(&mut self, channel: u8, map: i32, grid: SectreeGrid) {
        self.maps.insert((channel, map), view::MapIndex::new(grid));
    }

    /// Share the locale strings, which the chat lines a move sends are looked up in. Without
    /// them every line is sent as it is written.
    #[must_use]
    pub fn with_locale_strings(mut self, locale: Arc<LocaleStrings>) -> Self {
        self.locale = locale;
        self
    }

    /// Set `item_destroy_time_dropitem`, the seconds a dropped item lies before it is destroyed.
    #[must_use]
    pub fn with_drop_lifetime(mut self, seconds: i32) -> Self {
        self.drop_lifetime = drop_lifetime_pulses(seconds);
        self
    }

    /// Share the Channel's client registry, where a trade and a warp NPC find who is online.
    #[must_use]
    pub fn with_clients(mut self, clients: Arc<ChannelClients>) -> Self {
        self.clients = Some(clients);
        self
    }

    /// Set `g_bItemCountLimit`, the largest stack a merge may build.
    ///
    /// The configured `game.item_count_limit`, which `ServerConfig::validate` has already
    /// held to 1 through 5000. Until this is called the limit is legacy's compiled-in 5000
    /// (`G/config.cpp:41`).
    #[must_use]
    pub const fn with_item_count_limit(mut self, limit: u16) -> Self {
        self.item_count_limit = limit;
        self
    }

    /// Stand up the NPCs one map's regen entries name, on one Channel, drawing from this
    /// state's own dice as legacy's boot draws from `thecore_random`.
    ///
    /// Called once per map before the thread starts. A second call for the same map replaces
    /// its NPCs; their VIDs are not reused.
    ///
    /// # Errors
    ///
    /// Returns [`NpcVidsExhausted`] when the spawner has no VID left to give.
    pub fn spawn_npcs(
        &mut self,
        spawner: &mut NpcSpawner<'_>,
        channel: u8,
        region: &MapRegion,
        entries: &[RegenEntry],
    ) -> Result<(), NpcVidsExhausted> {
        let npcs = spawner.spawn_map(region, entries, &mut self.dice)?;
        self.stand_npcs(channel, region, npcs);
        Ok(())
    }

    /// Stands one map's NPCs up on one Channel: each enters the map's view where a sectree
    /// holds it, and the warp and goto NPCs among them start their event. The NPCs a map held
    /// before leave it.
    fn stand_npcs(&mut self, channel: u8, region: &MapRegion, npcs: MapNpcs) {
        let warps = warp_npc::warp_npcs(&npcs.npcs, region);
        let key = (channel, region.index);
        let index = self.maps.entry(key).or_insert_with(|| {
            SectreeGrid::of(region).map_or_else(view::MapIndex::treeless, view::MapIndex::new)
        });
        // Nobody stands on a map at boot, so the shows have no record to deliver.
        let mut effects = Vec::new();
        if let Some(replaced) = self.npcs.get(&key) {
            for npc in &replaced.npcs {
                index.remove(view::EntityKey::Npc(npc.vid), &mut effects);
                self.npc_of.remove(&npc.vid);
            }
        }
        for (at, npc) in npcs.npcs.iter().enumerate() {
            // `SpawnMob` places an NPC only where a sectree stands (`world/src/npc.rs:367`).
            let _shown = index.show(
                view::EntityKey::Npc(npc.vid),
                (npc.x, npc.y, npc.z),
                self.view_radius,
                &mut effects,
            );
            self.npc_of.insert(npc.vid, (channel, region.index, at));
        }
        self.npcs.insert(key, Arc::new(npcs));
        self.warp_npcs.insert(key, warps);
    }

    /// The NPCs standing on one map of one Channel, or none for a map boot stood none up on.
    #[must_use]
    pub fn npcs_on(&self, channel: u8, map: i32) -> Arc<MapNpcs> {
        self.npcs.get(&(channel, map)).cloned().unwrap_or_default()
    }

    /// A handle to the counters, which stays readable after the state is moved into
    /// the thread.
    #[must_use]
    pub fn metrics(&self) -> Arc<GameStateMetrics> {
        Arc::clone(&self.metrics)
    }

    /// The characters, for a caller that already holds `&mut GameState`.
    pub fn characters(&self) -> &CharacterManager {
        &self.characters
    }

    /// The characters, mutably.
    pub fn characters_mut(&mut self) -> &mut CharacterManager {
        &mut self.characters
    }

    /// The item id allocator, or `None` until one has been installed.
    ///
    /// `None` is a real state, not a placeholder: the start id is a fact about the
    /// stored items, and it is not known when this state is built.
    #[must_use]
    pub fn item_ids(&self) -> Option<&ItemIds> {
        self.item_ids.as_ref()
    }

    /// The item id allocator, mutably.
    pub fn item_ids_mut(&mut self) -> Option<&mut ItemIds> {
        self.item_ids.as_mut()
    }

    /// Install the allocator.
    ///
    /// Refuses a second install. One allocator has to serve the world's whole life:
    /// a second one starts again at the same `usable_item_id_min` and reissues ids
    /// that live items already hold, and there is no database write that would
    /// notice. An allocator that is installed over cannot be recovered, so this
    /// returns what the caller would lose rather than taking the first answer.
    ///
    /// # Errors
    ///
    /// [`AlreadyInstalled`] when an allocator is already installed. The existing one
    /// is left untouched, so a caller that ignores this error has lost nothing.
    ///
    /// # Panics
    ///
    /// Never. The borrow of the freshly installed value is taken from the same
    /// `Option` that was just assigned, so the `expect` cannot fail; it is written
    /// out because `Option::as_mut` says `None` is possible and here it is not.
    pub fn install_item_ids(
        &mut self,
        range: ItemIdRange,
    ) -> Result<&mut ItemIds, AlreadyInstalled> {
        if self.item_ids.is_some() {
            return Err(AlreadyInstalled);
        }
        self.item_ids = Some(ItemIds::new(range));
        Ok(self.item_ids.as_mut().expect("just installed"))
    }

    /// The item prototypes.
    pub fn protos(&self) -> &ItemProtos {
        &self.protos
    }

    /// The last pulse this state stepped.
    pub fn last_pulse(&self) -> u64 {
        self.last_pulse
    }
}

impl GameState {
    /// Apply one command to the world this thread owns.
    ///
    /// Split out from the trait impl so it can be called directly, which is what
    /// the tests do: a test that had to spawn a thread to learn whether a grant
    /// worked would be testing the scheduler, not the grant.
    pub fn apply(&mut self, command: GameCommand) {
        match command {
            GameCommand::ApplyAsyncCompletion { .. } => {
                // There is nothing to apply it to yet. A completion answers work
                // that game state asked for, and nothing has asked. Dropping it
                // here keeps the variant from being handled twice, in the loop and
                // here, which is how the two drift apart.
                debug_assert!(
                    false,
                    "a completion arrived for work this game state never started"
                );
            }
            GameCommand::GrantItem { request, reply } => {
                let answer = self.grant(&request);
                // A closed reply means the caller gave up, which it may legitimately
                // do when the descriptor it was writing to closed. The grant still
                // happened, and that is not a reason to undo it: the item is in the
                // world and the caller is the one that went away.
                if reply.send(answer).is_err() {
                    warn!(
                        target = ?request.target,
                        vnum = request.vnum,
                        "the item was granted but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::InstallItemIdRange { range, reply } => {
                let answer = self.install_item_ids(range);
                // A dropped install answer leaves the world without an allocator,
                // which is the state it started in, so the caller can ask again.
                if reply.send(answer.is_ok()).is_err() {
                    warn!(
                        first = range.first,
                        last = range.last,
                        "the item id range was installed but nobody was left to hear about it"
                    );
                }
            }
            GameCommand::RevokeGrant { target, id, reply } => {
                self.answer_revoke(&target, id, reply);
            }
            GameCommand::ReleaseItem { target, id, reply } => {
                self.answer_release(&target, id, reply);
            }
            GameCommand::EnterWorld {
                vid,
                player_id,
                name,
                items,
                loaded,
                outbox,
                reply,
            } => {
                let answer = self.enter_world_with_items(vid, player_id, &name, &items, outbox);
                let answer = answer.map(|()| self.shown_on_admit(vid, &name, loaded));
                // A dropped admit answer means the descriptor is already closing, and a
                // character nobody will play is not a state worth warning about: the
                // close path runs the leave, which finds nobody and says so at debug.
                if reply.send(answer).is_err() {
                    debug!(?vid, player_id, name = ?name.as_str(), "the world admitted a character nobody was left to hear about");
                }
            }
            GameCommand::DeliverRecord { vid, record, reply } => {
                // A `false` here means the character is not online or its descriptor
                // is gone. It is not logged as a warning: a record that could not be
                // delivered is a normal outcome for anything sent after a disconnect,
                // and the caller is told in the answer.
                if reply.send(self.write_to_client(vid, record)).is_err() {
                    debug!(
                        ?vid,
                        "the world wrote a record nobody was left to hear about"
                    );
                }
            }
            GameCommand::LeaveWorld { vid, reply } => self.answer_leave(vid, reply),
            GameCommand::Quickslot { vid, step, reply } => {
                if reply.send(self.quickslot(vid, step)).is_err() {
                    debug!(?vid, "nobody was left to hear a quickslot answer");
                }
            }
            GameCommand::KeptOf { vid, reply } => {
                let place = self.kept_place(vid);
                let kept = self.characters.find_by_vid(vid).ok().map(|character| Kept {
                    points: character.points().cloned(),
                    quickslots: character.quickslots().clone(),
                    place,
                });
                if reply.send(kept).is_err() {
                    debug!(?vid, "nobody was left to hear a character's points");
                }
            }
            command @ (GameCommand::MoveItem { .. }
            | GameCommand::UseItem { .. }
            | GameCommand::Shop { .. }
            | GameCommand::Trade { .. }
            | GameCommand::Safebox { .. }
            | GameCommand::Quest { .. }) => self.apply_item_step(command),
            command @ (GameCommand::DropItem { .. }
            | GameCommand::PickupItem { .. }
            | GameCommand::NpcsOn { .. }) => self.apply_ground(command),
            command @ (GameCommand::Move { .. }
            | GameCommand::SyncPosition { .. }
            | GameCommand::Relay { .. }) => self.apply_motion(command),
            GameCommand::Stop => {
                // The loop handles `Stop` itself, before a command ever reaches a
                // processor. Reaching here would mean the loop and the state
                // disagree about who owns shutdown, and quietly ignoring it would
                // hide exactly that.
                debug_assert!(false, "the game loop must handle Stop itself");
            }
        }
    }

    /// Take a granted item back and tell the caller whether it came out.
    fn answer_revoke(&mut self, target: &str, id: u32, reply: oneshot::Sender<bool>) {
        let removed = match self.revoke_grant(target, id) {
            Ok(_) => true,
            Err(error) => {
                // A revoke that could not happen is the state a player can end up
                // holding, so it is logged at warn with both ids rather than being
                // folded into the boolean the reply carries.
                warn!(target = ?target, id, %error, "a granted item could not be taken back");
                false
            }
        };
        // A dropped revoke answer leaves the world in the state the caller is
        // about to report as wrong, so the log line has to say which id and who
        // it belonged to: this is the state a player can be holding.
        if reply.send(removed).is_err() {
            warn!(
                target = ?target,
                id,
                "a granted item was taken back but nobody was left to hear about it"
            );
        }
    }

    /// Release an item from the world and tell the caller which cell it freed.
    fn answer_release(
        &mut self,
        target: &str,
        id: u32,
        reply: oneshot::Sender<Result<Released, RevokeRefused>>,
    ) {
        let answer = self.revoke_grant(target, id);
        // The same answer as the revoke, and for the same reason, but the
        // consequence is different: a refused release here means a row that is
        // about to be deleted is still held by the world, so it is logged with
        // both names rather than folded into a boolean the caller cannot act on.
        if let Err(error) = &answer {
            warn!(target = ?target, id, %error, "an item could not be released from the world");
        }
        if reply.send(answer).is_err() {
            warn!(
                target = ?target,
                id,
                "an item was released but nobody was left to hear about it"
            );
        }
    }

    /// Take a granted item back out of the world.
    ///
    /// This is the undo for the grant, and it exists because the grant mutates the world
    /// before its row can be written: the world is the only thing that can choose a cell,
    /// and the cell is part of the row. The id is enough to find the item again, because
    /// [`world::character::CharacterItems::release`] answers from the storage rather than being
    /// handed a cell the caller might have stale.
    ///
    /// # Errors
    ///
    /// [`RevokeRefused::NoSuchCharacter`] when the target is not online, and
    /// [`RevokeRefused::NotThere`] when the world does not hold the id, or
    /// [`RevokeRefused::Rejected`] when the storage named a cell it then would not
    /// release. None of the three changes anything.
    pub fn revoke_grant(&mut self, target: &str, id: u32) -> Result<Released, RevokeRefused> {
        let character = self.characters.find_player_mut(target).ok_or_else(|| {
            RevokeRefused::NoSuchCharacter {
                name: target.to_owned(),
            }
        })?;
        // Read before the release, not after: `release` takes `&mut self` on the
        // storage, and the owner is a property of the character, not of the item.
        let owner_id = character.player_id();
        let pos = character
            .items_mut()
            .release(id)
            .map_err(|error| RevokeRefused::Rejected { id, error })?;
        Ok(Released { pos, owner_id })
    }

    /// Puts a live client's character into the world under the VID it already uses.
    ///
    /// The VID is not allocated here and that is the whole point. Legacy allocates it
    /// (`CHARACTER_MANAGER::AllocVID`) and adds a CRC that never reaches the wire,
    /// but the Rewrite's live descriptor has been publishing the store's `player.id`
    /// as the VID since the enter-game burst was written, and the client has been
    /// treating it as that character's identity ever since. Allocating a second
    /// number here would put one character in the world under a VID nothing else
    /// knows, so a record the world sends would be addressed to a client that never
    /// asked for it. So the caller supplies the number and this refuses a clash.
    ///
    /// # Errors
    ///
    /// [`EnterWorldRefused::NullVid`], [`EnterWorldRefused::DuplicateVid`],
    /// [`EnterWorldRefused::DuplicatePlayerId`], or
    /// [`EnterWorldRefused::DuplicateName`]. Every one leaves the world unchanged, so
    /// the descriptor can close without the two sides disagreeing about who exists.
    pub fn enter_world(
        &mut self,
        vid: common::vid::Vid,
        player_id: u32,
        name: &str,
        outbox: ClientOutbox,
    ) -> Result<(), EnterWorldRefused> {
        self.enter_world_with_items(vid, player_id, name, &[], outbox)
    }

    /// Puts a live client's character into the world holding the items its load placed.
    ///
    /// The items are set in the order given, which is the order the load placed them in
    /// its own empty inventory, so each one lands where the client was told it is.
    ///
    /// # Errors
    ///
    /// The answer to an admitted character: its load's points, quickslots and gold are set on it,
    /// then its body is placed and, when the load showed it, the revive-invisible affect starts.
    /// The placement's records come first, and the update that starts the affect after them.
    fn shown_on_admit(&mut self, vid: common::vid::Vid, name: &str, loaded: Box<Loaded>) -> Shown {
        if let Some(character) = self.characters.find_player_mut(name) {
            character.set_points(loaded.points);
            character.set_quickslots(loaded.quickslots);
            character.set_gold(loaded.gold);
        }
        let (records, update) = match loaded.show {
            Some(show) => {
                let records = self.place_body(vid, show.place, show.card);
                (records, self.enter_revive_invisible(vid))
            }
            None => (Vec::new(), None),
        };
        Shown { records, update }
    }

    /// As [`GameState::enter_world`], and [`EnterWorldRefused::ItemRefused`] when an
    /// item would not go where the load put it. The character is taken back out in that
    /// case, so every refusal still leaves the world unchanged.
    ///
    /// # Errors
    ///
    /// Every [`EnterWorldRefused`] that the admission can raise: a null VID, a duplicate VID,
    /// player id or Name, and an item that would not go where the load put it.
    pub fn enter_world_with_items(
        &mut self,
        vid: common::vid::Vid,
        player_id: u32,
        name: &str,
        items: &[(protocol::item_pos::ItemPos, world::item::Item)],
        outbox: ClientOutbox,
    ) -> Result<(), EnterWorldRefused> {
        if vid.is_null() {
            return Err(EnterWorldRefused::NullVid);
        }
        // Checked before the create, because `CharacterManager::create_player` reports
        // a duplicate player id or name but allocates a **fresh** VID, so a duplicate
        // VID would otherwise slip past it and consume a counter value silently.
        if self.characters.find_by_vid(vid).is_ok() {
            return Err(EnterWorldRefused::DuplicateVid { vid });
        }
        // The manager is the authority on the indexes it owns. A clone of the outbox
        // is taken only after both sides agree the character is new, so a refusal
        // cannot leave a sender for a character that was never admitted.
        self.characters
            .create_player_with_vid(player_id, name, vid)
            .map_err(|error| EnterWorldRefused::from_manager(error, name))?;
        if let Err(refused) = self.place_loaded_items(name, items) {
            let _destroyed = self.characters.destroy(vid);
            return Err(refused);
        }
        let _previous = self.outboxes.insert(vid, outbox);
        Ok(())
    }

    /// Sets each loaded item into the character just created under `name`.
    fn place_loaded_items(
        &mut self,
        name: &str,
        items: &[(protocol::item_pos::ItemPos, world::item::Item)],
    ) -> Result<(), EnterWorldRefused> {
        let Some(character) = self.characters.find_player_mut(name) else {
            return Err(EnterWorldRefused::NotAdmitted {
                reason: format!("the character named {name} was created and then not found"),
            });
        };
        for (pos, item) in items {
            character.items_mut().set(*pos, item).map_err(|reason| {
                EnterWorldRefused::ItemRefused {
                    id: item.id,
                    reason,
                }
            })?;
        }
        Ok(())
    }

    /// Takes a live client's character out of the world.
    ///
    /// The sender is dropped **first**, so a record the world writes after this point
    /// finds no client rather than a client whose character no longer exists. That is
    /// the ordering the reverse case depends on too: a disconnect writes the row for
    /// the last time after this returns, and anything the world still held would be
    /// released by the destruction and the row would still name it.
    ///
    /// Answers the character's points and quickslots as the world last changed them, which
    /// the final save writes: the recovery event changes the points on the world's own pulse,
    /// after the descriptor's last step, and a trade the partner closes the quickslots. Its
    /// event ends with the character (`event_cancel` in `CHARACTER::Destroy`), and so does its
    /// trade, cancelled for both sides (`CHARACTER::Destroy`'s `Cancel`), and its running quest
    /// script (`CQuestManager::DisconnectPC`, `G/char.cpp:1802`). Then its body leaves its map
    /// and every player that saw it is sent its removal (`G/char.cpp:786-789`), and the answer
    /// carries where the body stood.
    ///
    /// Returns `None` when the world did not hold that character, which is reported
    /// rather than treated as fatal because a disconnect that finds nobody is
    /// already ending.
    pub fn leave_world(&mut self, vid: common::vid::Vid) -> Option<Kept> {
        self.cancel_trade(vid);
        let _outbox = self.outboxes.remove(&vid);
        let _event = self.affect_events.remove(&vid);
        self.stop_browsing(vid);
        self.close_storage(vid);
        self.end_quests(vid);
        self.forget_portal_times(vid);
        let place = self.remove_body(vid);
        let character = self.characters.find_by_vid(vid).ok()?;
        let kept = Kept {
            points: character.points().cloned(),
            quickslots: character.quickslots().clone(),
            place,
        };
        self.characters.destroy(vid).ok()?;
        Some(kept)
    }

    /// Whether a character is online under that VID.
    ///
    /// This is what decides whether a record the world is about to build has a
    /// client to go to, so it is deliberately a world question and not a registry
    /// one: the registry is keyed by lease and knows about broadcasts, while the
    /// world is keyed by the VID a grant and a record both use.
    #[must_use]
    pub fn is_online(&self, vid: common::vid::Vid) -> bool {
        self.characters.find_by_vid(vid).is_ok()
    }

    /// How many characters the world holds.
    ///
    /// Counts the characters, not the senders, so a test that joins and leaves sees
    /// the same number on both sides of the round trip.
    #[must_use]
    pub fn online_count(&self) -> usize {
        self.characters.len()
    }

    /// Writes one record to a character's client.
    ///
    /// The world owns no socket, so this is the only way a record the world produced
    /// reaches a client (ADR-0002). A `false` means the client is gone; the caller
    /// decides whether that matters, and for a grant it does not, because the row is
    /// already written and the item is in the world either way -- it is the item
    /// that must not be silently lost, not the notification.
    pub fn write_to_client(&self, vid: common::vid::Vid, record: Vec<u8>) -> bool {
        self.outboxes
            .get(&vid)
            .is_some_and(|outbox| outbox.send(record))
    }

    /// Run one `CG_ITEM_MOVE` for the character online under `vid`.
    ///
    /// The rules are the character's: its own `Inven_Point`, its worn belt, and the
    /// configured stack limit. A split takes its id from the world's allocator.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`, and
    /// [`MoveItemRefused::Refused`] with the world's reason otherwise. Neither changes
    /// anything.
    pub fn move_item(
        &mut self,
        vid: common::vid::Vid,
        request: MoveRequest,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        self.run_item_step(vid, actor, |items, ids, rules, gear, protos| {
            move_item(items, ids, request, rules, gear, |vnum| {
                move_facts(protos, vnum)
            })
        })
    }

    /// Run one client quickslot request for the character online under `vid`, answering the
    /// records for its client and the slots after it, or `None` when nobody is online under
    /// `vid`.
    pub fn quickslot(
        &mut self,
        vid: common::vid::Vid,
        step: QuickslotStep,
    ) -> Option<QuickslotAnswer> {
        let character = self.characters.find_by_vid_mut(vid).ok()?;
        let (items, slots) = character.items_and_quickslots_mut();
        let mut records = Vec::new();
        match step {
            QuickslotStep::Add { slot, quickslot } => {
                records = add_from_client(slots, items, &self.protos, slot, quickslot);
            }
            QuickslotStep::Del { slot } => {
                let _deleted = slots.delete(slot, &mut records);
            }
            QuickslotStep::Swap { slot, with } => {
                let _swapped = slots.swap(slot, with, &mut records);
            }
        }
        Some(QuickslotAnswer {
            records: crate::quickslot::encode(&records),
            quickslots: slots.clone(),
        })
    }

    /// Answer a `LeaveWorld`: the character leaves, and the descriptor is handed its points.
    fn answer_leave(&mut self, vid: common::vid::Vid, reply: oneshot::Sender<Option<Kept>>) {
        let removed = self.leave_world(vid);
        // A leave that finds nobody is a double leave, not a failure: the
        // descriptor is ending and the world already agrees the character is
        // gone. Logging it at warn would make an ordinary reconnect noisy.
        if removed.is_none() {
            debug!(
                ?vid,
                "the world was asked to remove a character it did not hold"
            );
        }
        if reply.send(removed).is_err() {
            debug!(
                ?vid,
                "the world removed a character nobody was left to hear about"
            );
        }
    }

    /// Run one `CG_ITEM_USE` for the character online under `vid`: an equippable item goes on
    /// or comes off, and a potion is drunk (`CHARACTER::UseItem`, `G/char_item.cpp:7168`).
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`].
    pub fn use_item(
        &mut self,
        vid: common::vid::Vid,
        at: protocol::item_pos::ItemPos,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let moved = self.run_item_step(vid, actor, |items, _ids, rules, gear, _protos| {
            use_item(items, at, rules, gear)
        })?;
        // `StartAffectEvent`: a potion that left recovery starts the event, one second out,
        // unless it already runs.
        if moved
            .points
            .as_ref()
            .is_some_and(world::character::is_recovering)
        {
            self.start_affect_event(vid);
        }
        Ok(moved)
    }

    /// Apply one of the steps a character takes with its own items or an NPC: a move, a use, a
    /// shop, trade or safebox step, or a quest step.
    fn apply_item_step(&mut self, command: GameCommand) {
        match command {
            GameCommand::MoveItem {
                vid,
                request,
                mover,
                reply,
            } => answer_item_step(vid, reply, self.move_item(vid, request, mover)),
            GameCommand::UseItem {
                vid,
                at,
                mover,
                reply,
            } => answer_item_step(vid, reply, self.use_item(vid, at, mover)),
            GameCommand::Shop {
                vid,
                step,
                place,
                mover,
                reply,
            } => {
                // As with a move, a dropped answer leaves the world ahead of the store.
                let place = self.live_place(vid, place);
                if reply.send(self.shop(vid, step, place, mover)).is_err() {
                    warn!(
                        ?vid,
                        ?step,
                        "a shop step ran and nobody was left to store it"
                    );
                }
            }
            GameCommand::Trade {
                vid,
                step,
                place,
                mover,
                reply,
            } => {
                // A dropped settlement leaves both characters' worlds ahead of the store.
                let place = self.live_place(vid, place);
                if reply.send(self.trade(vid, step, place, mover)).is_err() {
                    warn!(
                        ?vid,
                        ?step,
                        "a trade step ran and nobody was left to store it"
                    );
                }
            }
            GameCommand::Safebox {
                vid,
                step,
                mover,
                reply,
            } => {
                if reply.send(self.safebox(vid, step, mover)).is_err() {
                    warn!(?vid, "a safebox step ran and nobody was left to store it");
                }
            }
            GameCommand::Quest {
                vid,
                step,
                place,
                mover,
                reply,
            } => {
                // Nothing is stored; the client only misses the dialog.
                let place = self.live_place(vid, place);
                if reply.send(self.quest(vid, &step, place, mover)).is_err() {
                    debug!(
                        ?vid,
                        ?step,
                        "a quest step ran and nobody was left to see it"
                    );
                }
            }
            _ => debug_assert!(false, "only item steps reach apply_item_step"),
        }
    }

    /// Apply one of the ground commands: a drop, a pick-up, or the NPCs standing on a map.
    fn apply_ground(&mut self, command: GameCommand) {
        match command {
            GameCommand::DropItem {
                vid,
                at,
                count,
                place,
                mover,
                reply,
            } => {
                let place = self.live_place(vid, place);
                answer_item_step(vid, reply, self.drop_item(vid, at, count, place, mover));
            }
            GameCommand::PickupItem {
                vid,
                ground,
                place,
                mover,
                reply,
            } => {
                let place = self.live_place(vid, place);
                answer_item_step(vid, reply, self.pickup_item(vid, ground, place, mover));
            }
            GameCommand::NpcsOn {
                channel,
                map,
                reply,
            } => {
                if reply.send(self.npcs_on(channel, map)).is_err() {
                    debug!(channel, map, "nobody was left to hear the NPCs");
                }
            }
            _ => debug_assert!(false, "only ground commands reach apply_ground"),
        }
    }

    /// Run one `CG_ITEM_DROP` for the character online under `vid` (`CHARACTER::DropItem`,
    /// `G/char_item.cpp:7444`). The item lies where the character stands until it is picked
    /// up or its destroy event fires, and enters the view there (`CItem::AddToGround`,
    /// `G/item.cpp:549-584`).
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`]; [`MoveRefused::NoSectree`] when no sectree holds the point.
    pub fn drop_item(
        &mut self,
        vid: common::vid::Vid,
        at: protocol::item_pos::ItemPos,
        count: u16,
        place: GroundPlace,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let owner_id = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?
            .player_id();
        // `GetXYZ()`: the body's point, or the descriptor's with no body.
        let (x, y, z) = self
            .spot_of(vid.raw())
            .map_or((place.x, place.y, 0), |spot| (spot.x, spot.y, spot.z));
        let key = (place.channel, place.map);
        let to = DropAt {
            vid: self.next_ground_vid,
            x,
            y,
            owner_id,
            questing: self.quest_running(vid),
            lands: self
                .maps
                .get(&key)
                .and_then(|index| index.tree_at(x, y))
                .is_some(),
        };
        let mut dropped = None;
        let mut moved = self.run_item_step(vid, actor, |items, ids, _rules, _gear, _protos| {
            drop_item(items, ids, at, count, to).map(|(done, ground)| {
                dropped = Some(ground);
                done
            })
        })?;
        if let Some(ground) = dropped {
            let laid = ground.vid;
            let (order, stored) = StoreOrder::laid();
            moved.store_order = order;
            self.ground.insert(
                laid,
                Lying {
                    ground,
                    channel: place.channel,
                    map: place.map,
                    expires: self.last_pulse.saturating_add(self.drop_lifetime),
                    stored,
                },
            );
            self.next_ground_vid = self.next_ground_vid.wrapping_add(1).max(1);
            let radius = self.view_radius;
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&key) {
                index.add_to_ground(
                    view::EntityKey::Ground(laid),
                    (x, y, z),
                    radius,
                    &mut effects,
                );
            }
            let mut own = Vec::new();
            self.deliver(&effects, Some((vid.raw(), &mut own)));
            moved.place_ground(own);
        }
        Ok(moved)
    }

    /// Run one `CG_ITEM_PICKUP` for the character online under `vid` (`CHARACTER::PickupItem`,
    /// `G/char_item.cpp:7972`). An item picked up whole leaves the view
    /// (`CItem::RemoveFromGround`, `G/item.cpp:533-547`).
    ///
    /// # Errors
    ///
    /// As [`Self::move_item`]; [`MoveRefused::NotOnGround`] when nothing lies under that VID on
    /// the character's map, and [`MoveRefused::TooFar`] past `DistanceValid`.
    pub fn pickup_item(
        &mut self,
        vid: common::vid::Vid,
        ground: u32,
        place: GroundPlace,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let lying = self
            .ground
            .get(&ground)
            .filter(|lying| lying.channel == place.channel && lying.map == place.map)
            .ok_or(MoveItemRefused::Refused(MoveRefused::NotOnGround))?;
        let (dx, dy) = (
            lying.ground.x.saturating_sub(place.x),
            lying.ground.y.saturating_sub(place.y),
        );
        if distance_approx(dx, dy) > PICKUP_DISTANCE {
            return Err(MoveItemRefused::Refused(MoveRefused::TooFar));
        }
        let questing = self.quest_running(vid);
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let Some(mut lying) = self.ground.remove(&ground) else {
            return Err(MoveItemRefused::Refused(MoveRefused::NotOnGround));
        };
        // Its rows wait for the drop's, a remainder's pick-up's too.
        let order = StoreOrder::after(lying.stored.clone());
        let owner_id = character.player_id();
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: usable_inventory_cells(character.inven_point()),
            belt_grade: belt_grade(character.items(), &self.protos),
            questing,
        };
        let mut picker = Picker {
            owner_id,
            rules: &rules,
            protos: &self.protos,
            dice: &mut self.dice,
        };
        let done = pickup_item(character.items_mut(), &mut lying.ground, &mut picker);
        let mut own = Vec::new();
        if lying.ground.item.count > 0 {
            self.ground.insert(ground, lying);
        } else {
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&(lying.channel, lying.map)) {
                index.remove(view::EntityKey::Ground(ground), &mut effects);
            }
            self.deliver(&effects, Some((vid.raw(), &mut own)));
        }
        let done = done.map_err(MoveItemRefused::Refused)?;
        let mut moved =
            MovedItems::new(owner_id, vid.raw(), done, actor, &self.protos, &self.locale);
        moved.place_ground(own);
        moved.store_order = order;
        Ok(moved)
    }

    /// Fire every destroy event due by `pulse`: the item leaves the ground, and each player
    /// that sees it is sent `GC_ITEM_GROUND_DEL` (`CItem::DestroyEvent`,
    /// `item_destroy_time_dropitem`; `RemoveFromGround`, `G/item.cpp:533-547`).
    fn destroy_expired(&mut self, pulse: u64) {
        let due: Vec<u32> = self
            .ground
            .iter()
            .filter(|(_, lying)| lying.expires <= pulse)
            .map(|(vid, _)| *vid)
            .collect();
        for vid in due {
            let Some(lying) = self.ground.remove(&vid) else {
                continue;
            };
            let mut effects = Vec::new();
            if let Some(index) = self.maps.get_mut(&(lying.channel, lying.map)) {
                index.remove(view::EntityKey::Ground(vid), &mut effects);
            }
            self.deliver(&effects, None);
        }
    }

    /// Run one item step against the storage and gear of the character online under `vid`.
    fn run_item_step(
        &mut self,
        vid: common::vid::Vid,
        actor: Mover,
        step: impl FnOnce(
            &mut world::character::CharacterItems,
            Option<&mut ItemIds>,
            &MoveRules,
            Option<&mut Gear<'_>>,
            &ItemProtos,
        ) -> Result<world::character::MoveDone, world::character::MoveRefused>,
    ) -> Result<MovedItems, MoveItemRefused> {
        let questing = self.quest_running(vid);
        // `IsPC() && 1500 > now - GetLastAttackTime()` (`G/char_item.cpp:8416-8422`): the world
        // holds the attack stamp, and the descriptor the target selection. The world also holds
        // the PK mode a look carries.
        let actor = Mover {
            recently_fought: actor.recently_fought || self.attacked_recently(vid),
            pk_mode: self.pk_mode_of(vid.raw()),
            affect_flags: self.affect_flags_of(vid),
            ..actor
        };
        let character = self
            .characters
            .find_by_vid_mut(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let owner_id = character.player_id();
        let rules = MoveRules {
            count_limit: self.item_count_limit,
            usable_cells: usable_inventory_cells(character.inven_point()),
            belt_grade: belt_grade(character.items(), &self.protos),
            questing,
        };
        let protos = &self.protos;
        let (items, points) = character.items_and_points_mut();
        let mut gear = points.map(|points| Gear {
            points,
            protos,
            dice: &mut self.dice,
            recently_fought: actor.recently_fought,
        });
        let mut done = step(items, self.item_ids.as_mut(), &rules, gear.as_mut(), protos)
            .map_err(MoveItemRefused::Refused)?;
        let (items, slots) = character.items_and_quickslots_mut();
        sync_quickslots(&mut done, slots, items);
        let quickslots = slots.clone();
        let mut moved = MovedItems::new(owner_id, vid.raw(), done, actor, protos, &self.locale);
        moved.points = character.points().cloned();
        moved.quickslots = Some(quickslots);
        Ok(moved)
    }

    /// Run one grant against the world, taking the target's own `Inven_Point`.
    fn grant(&mut self, request: &GrantRequest) -> Result<GrantOutcome, GrantRefusal> {
        let Some(item_ids) = self.item_ids.as_mut() else {
            // Refused before anything is placed, so no id is burned and the world is
            // unchanged. A caller that retries after the install gets a fresh answer.
            return Err(GrantRefusal::NoAllocator);
        };
        let inven_point = self
            .characters
            .find_player_mut(&request.target)
            .map_or(0, |character| character.inven_point());
        grant_item(
            &mut self.characters,
            &self.protos,
            item_ids,
            request,
            inven_point,
        )
    }
}

impl PulseProcessor for GameState {
    fn apply_command(&mut self, command: GameCommand) {
        self.apply(command);
    }

    /// Step one pulse.
    ///
    /// Counts the Pulse, then runs the events that are due on it: the ground items whose
    /// lifetime ran out, the affect event of each character (the potion's recovery, the stamina
    /// refill and the revive timer, `run_affect_events`) and the warp NPCs. Last, as
    /// `CHARACTER_MANAGER::Update` comes after the heartbeat's events (`G/main.cpp:777-782`),
    /// each moving player steps. The count is also the only way a test can tell that the game
    /// thread owns this value.
    fn process_pulse(&mut self, pulse: u64) {
        self.last_pulse = pulse;
        self.metrics.pulses.store(pulse, Ordering::SeqCst);
        self.destroy_expired(pulse);
        self.run_affect_events(pulse);
        self.run_warp_npcs(pulse);
        self.step_motion(pulse);
    }
}

/// Legacy's `item_destroy_time_dropitem` default, in seconds (`G/config.cpp:56`).
const DEFAULT_DROP_LIFETIME_SECS: i32 = 300;

/// `VIEW_RANGE`'s default (`G/config.cpp:123`), which the configured `game.view_range` replaces.
const DEFAULT_VIEW_RANGE: i32 = 5000;

/// The view's radius for `view_range`: `VIEW_RANGE + VIEW_BONUS_RANGE` (`G/entity_view.cpp:94`).
fn view_radius_of(view_range: i32) -> i64 {
    i64::from(view_range) + view::VIEW_BONUS_RANGE
}

/// A lifetime in seconds as pulses, at least one.
fn drop_lifetime_pulses(seconds: i32) -> u64 {
    u64::try_from(seconds)
        .unwrap_or(0)
        .saturating_mul(u64::from(PASSES_PER_SEC))
        .max(1)
}

/// Hands an item step's answer back to the descriptor that asked for it.
fn answer_item_step(
    vid: common::vid::Vid,
    reply: tokio::sync::oneshot::Sender<Result<MovedItems, MoveItemRefused>>,
    answer: Result<MovedItems, MoveItemRefused>,
) {
    // A dropped answer means the descriptor is closing. The world has moved the item, and
    // the store has not, so the next login loads the old cell; that is the same outcome as
    // a write that failed, and it is logged.
    if reply.send(answer).is_err() {
        warn!(
            ?vid,
            "an item moved in the world and nobody was left to store it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use common::item_slots::{usable_inventory_cells, INVENTORY_MAX_EXTENDED, INVENTORY_MAX_NUM};
    use common::vid::Vid;
    use db::items::RowChange;
    use protocol::item_pos::ItemPos;
    use world::character::{Lookup, MoveRefused, Points};

    fn owners() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        ItemProtos::load(&dir).expect("the owner's item protos load")
    }

    fn a_range() -> ItemIdRange {
        ItemIdRange::new(1, 1_000_000, 1).expect("a range that can issue an id")
    }

    /// A state with an allocator installed, which is the shape every world test
    /// after the install wants.
    fn a_state() -> GameState {
        let mut state = GameState::new(owners());
        state
            .install_item_ids(a_range())
            .expect("the first install");
        state
    }

    /// A sender nobody is draining, which is what a world write to a departed
    /// client meets. Kept as a helper because three of the tests below need the
    /// closed case and building it by dropping the receiver is the only honest way
    /// to get one.
    fn a_dropped_outbox() -> ClientOutbox {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        drop(rx);
        ClientOutbox::new(tx)
    }

    /// A sender with a receiver the test keeps, so a write can be read back.
    fn a_live_outbox() -> (ClientOutbox, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        (ClientOutbox::new(tx), rx)
    }

    #[test]
    fn a_character_enters_the_world_under_the_vid_the_descriptor_is_using() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        // The VID here is the store's `player.id`, which is the number the
        // enter-game burst already put on the wire. A world that allocated its own
        // would hold the character under an identity the client has never seen.
        state
            .enter_world(Vid::new(4_242), 4_242, "Shaman", outbox)
            .expect("the character is admitted");
        assert_eq!(state.online_count(), 1);
        assert!(state.is_online(Vid::new(4_242)));
        let character = state
            .characters()
            .find_by_vid(Vid::new(4_242))
            .expect("the character is in the world");
        assert_eq!(character.player_id(), 4_242);
        assert_eq!(character.name(), "Shaman");
    }

    #[test]
    fn a_second_client_on_the_same_vid_is_refused_and_changes_nothing() {
        let mut state = a_state();
        let (outbox, _first) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (second_outbox, _second) = a_live_outbox();
        assert_eq!(
            state.enter_world(Vid::new(7), 8, "Warrior", second_outbox),
            Err(EnterWorldRefused::DuplicateVid { vid: Vid::new(7) })
        );
        // The refusal left one character, not two, and the one that is there is
        // still the first one. A world that admitted both would send every record
        // to whichever descriptor answered last.
        assert_eq!(state.online_count(), 1);
        let character = state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the first character is still the one there");
        assert_eq!(character.name(), "Shaman");
        assert_eq!(character.player_id(), 7);
    }

    #[test]
    fn a_duplicate_name_is_refused_case_insensitively_because_names_are_case_insensitive() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (other, _inbox) = a_live_outbox();
        // Legacy lowercases into its name index (`str_lower` in
        // `CHARACTER_MANAGER::CreateCharacter`), so a second login as `shaman` is the
        // same character as far as the world is concerned.
        assert_eq!(
            state.enter_world(Vid::new(8), 8, "sHaMaN", other),
            Err(EnterWorldRefused::DuplicateName {
                name: "sHaMaN".to_owned()
            })
        );
        assert_eq!(state.online_count(), 1);
    }

    #[test]
    fn a_null_vid_is_refused_because_it_is_what_a_record_with_no_target_carries() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        assert_eq!(
            state.enter_world(Vid::NULL, 1, "Shaman", outbox),
            Err(EnterWorldRefused::NullVid)
        );
        // Nothing was indexed, not even under the player id, so a later admission of
        // the same character is not blocked by a half-made one.
        assert_eq!(state.online_count(), 0);
        let (retry, _inbox) = a_live_outbox();
        assert!(state.enter_world(Vid::new(1), 1, "Shaman", retry).is_ok());
    }

    #[test]
    fn a_refused_admission_leaves_no_sender_behind_for_a_character_that_never_existed() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the first character is admitted");
        let (second, _inbox) = a_live_outbox();
        let _refused = state.enter_world(Vid::new(7), 8, "Warrior", second);
        // The refused sender was never inserted, so leaving the real character is the
        // only entry the map had and it leaves cleanly. A sender for a character that
        // was never admitted would keep a record addressable to a client the world
        // has no character for.
        assert!(state.leave_world(Vid::new(7)).is_some());
        assert_eq!(state.online_count(), 0);
    }

    /// An item of `size` cells, the way the load builds one before it is placed.
    fn a_loaded_item(id: u32, size: u8) -> world::item::Item {
        let mut item = world::item::Item::new(id, 19);
        item.set_size(size).expect("a positive footprint");
        item
    }

    #[test]
    fn the_admitted_character_holds_the_items_its_load_placed() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        let inventory = common::item_slots::EWindows::Inventory as u8;
        let items = vec![
            (ItemPos::new(inventory, 0), a_loaded_item(11, 2)),
            (ItemPos::new(inventory, 277), a_loaded_item(12, 1)),
        ];
        state
            .enter_world_with_items(Vid::new(7), 7, "Shaman", &items, outbox)
            .expect("the character is admitted with its items");
        let character = state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the character is in the world");
        assert_eq!(
            character.items().get(ItemPos::new(inventory, 0)),
            Lookup::Occupied(11)
        );
        assert_eq!(
            character.items().get(ItemPos::new(inventory, 277)),
            Lookup::Occupied(12)
        );
        // The next grant is offered the first cell the loaded items leave free. A world that
        // admitted the character empty would offer cell 0, which the client already shows
        // as taken and the store's cell key already holds.
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 19,
            count: None,
        };
        let outcome = state.grant(&request).expect("the grant is placed");
        assert_eq!(outcome.row.pos, 1);
    }

    #[test]
    fn an_item_the_inventory_refuses_takes_the_character_back_out() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        let inventory = common::item_slots::EWindows::Inventory as u8;
        let items = vec![
            (ItemPos::new(inventory, 0), a_loaded_item(11, 2)),
            (ItemPos::new(inventory, 0), a_loaded_item(12, 1)),
        ];
        assert_eq!(
            state.enter_world_with_items(Vid::new(7), 7, "Shaman", &items, outbox),
            Err(EnterWorldRefused::ItemRefused {
                id: 12,
                reason: Rejected::AlreadyOccupied {
                    window: inventory,
                    cell: 0,
                    present: 11,
                },
            })
        );
        // Nothing is left behind: no character, no sender, and no index under the VID or the
        // Name, so the same character can be admitted again.
        assert_eq!(state.online_count(), 0);
        assert_eq!(state.characters().len(), 0);
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        let (retry, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", retry)
            .expect("the same character is admitted afterwards");
    }

    #[test]
    fn the_world_writes_a_record_to_the_client_that_joined_under_that_vid() {
        let mut state = a_state();
        let (outbox, mut inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        assert!(state.write_to_client(Vid::new(7), vec![0x2B, 0x01, 0x02]));
        // Read it back, because a `true` from `write_to_client` only proves the
        // channel accepted the record; what the client sees is the point.
        assert_eq!(
            inbox.try_recv().expect("the record is queued"),
            vec![0x2B, 0x01, 0x02]
        );
    }

    #[test]
    fn a_write_to_a_departed_character_reports_failure_rather_than_queueing_nothing_silently() {
        let mut state = a_state();
        let (outbox, mut inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        state.leave_world(Vid::new(7));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        assert!(inbox.try_recv().is_err());
    }

    #[test]
    fn leaving_removes_the_sender_before_the_character_so_a_later_write_fails() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        assert!(state.leave_world(Vid::new(7)).is_some());
        // Both halves are gone together, which is what a grant checks before it
        // builds a record for a character.
        assert!(!state.is_online(Vid::new(7)));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
        assert_eq!(state.online_count(), 0);
    }

    #[test]
    fn leaving_a_character_the_world_never_had_reports_none_rather_than_panicking() {
        let mut state = a_state();
        // A descriptor that never entered the game still runs the close path, so a
        // double leave is reachable and must not be a panic.
        assert!(state.leave_world(Vid::new(999)).is_none());
    }

    #[test]
    fn a_character_in_the_world_is_reachable_by_the_name_a_grant_addresses() {
        // This is the whole reason a live client has to be in the world: a grant
        // addresses a character by Name, and before this the world held nothing a
        // Name could find.
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        let request = GrantRequest {
            target: "shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        // The grant is asked for by a differently-cased spelling, which the world's
        // index accepts, and it places rather than answering "no such character".
        assert!(state.grant(&request).is_ok());
    }

    #[test]
    fn a_write_to_a_dropped_client_is_reported_as_failed() {
        let mut state = a_state();
        state
            .enter_world(Vid::new(7), 7, "Shaman", a_dropped_outbox())
            .expect("the character is admitted even though its descriptor is gone");
        // The world holds the character, so a grant would still place an item; the
        // notification is what cannot be delivered. Reporting that separately is the
        // point: the item is safe, the client will see it at relog.
        assert!(state.is_online(Vid::new(7)));
        assert!(!state.write_to_client(Vid::new(7), vec![0x2B]));
    }

    #[test]
    fn a_new_state_starts_with_no_characters_and_no_allocator() {
        let state = GameState::new(owners());
        assert_eq!(state.characters().len(), 0);
        // No allocator, and not a placeholder one: the start id is a fact about the
        // stored items, and a state that is built before the store is ready does not
        // know it.
        assert!(state.item_ids().is_none());
        assert_eq!(state.protos().len(), 7_305);
        assert_eq!(state.last_pulse(), 0);
    }

    #[test]
    fn an_installed_allocator_starts_unissued_at_the_first_usable_id() {
        let state = a_state();
        let ids = state.item_ids().expect("the allocator was installed");
        assert_eq!(ids.issued(), 0);
        assert_eq!(ids.peek(), 1);
        assert_eq!(ids.range().first, 1);
        assert_eq!(ids.range().last, 1_000_000);
    }

    #[test]
    fn a_second_install_is_refused_rather_than_replacing_the_allocator() {
        // A second allocator starts again at the same first usable id and reissues
        // ids live items hold. Nothing in the store would notice, so the refusal is
        // the only thing standing between a bug and duplicate item ids.
        let mut state = a_state();
        let before = state.item_ids().expect("one allocator").peek();
        assert_eq!(state.install_item_ids(a_range()), Err(AlreadyInstalled));
        assert_eq!(
            state.item_ids().expect("still one allocator").peek(),
            before
        );
    }

    #[test]
    fn a_grant_is_refused_before_an_allocator_is_installed_and_changes_nothing() {
        let mut state = GameState::new(owners());
        state
            .characters_mut()
            .create_player(1, "Shaman")
            .expect("the character exists");
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        assert_eq!(state.grant(&request), Err(GrantRefusal::NoAllocator));
        // The refused grant placed nothing: no cell is taken, and no id is burned,
        // because the allocator is read before the placement search runs.
        let character = state
            .characters_mut()
            .find_player_mut("Shaman")
            .expect("the character");
        assert_eq!(character.items().len(), 0);
        // The control for that claim: the same world with an allocator hands the
        // first usable id out, so the ids above really were still unissued.
        state
            .install_item_ids(a_range())
            .expect("the first install");
        assert_eq!(state.item_ids().expect("the allocator").peek(), 1);
    }

    #[test]
    fn the_same_grant_succeeds_once_an_allocator_is_installed() {
        // The control for the test above, and the reason a retry is the answer:
        // the refusal above left the world in a state this one can act on.
        let mut state = GameState::new(owners());
        state
            .characters_mut()
            .create_player(1, "Shaman")
            .expect("the character exists");
        let request = GrantRequest {
            target: "Shaman".to_owned(),
            vnum: 30_000,
            count: None,
        };
        assert_eq!(state.grant(&request), Err(GrantRefusal::NoAllocator));
        state
            .install_item_ids(a_range())
            .expect("the first install");
        let outcome = state.grant(&request).expect("the grant happened");
        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .expect("the character")
                .items()
                .len(),
            1
        );
        // The id came from the allocator that was installed after the refusal, so
        // the retry really did use the new allocator.
        assert_eq!(outcome.row.id, 1);
        assert_eq!(state.item_ids().expect("the allocator").issued(), 1);
    }

    #[test]
    fn a_pulse_records_its_number_and_bumps_the_shared_counter() {
        let mut state = a_state();
        let metrics = state.metrics();
        assert_eq!(metrics.pulses(), 0, "nothing has stepped it yet");

        for pulse in 1..=5 {
            state.process_pulse(pulse);
        }
        assert_eq!(state.last_pulse(), 5);
        assert_eq!(metrics.pulses(), 5, "the handle sees the same state");
    }

    #[test]
    fn the_metrics_handle_survives_the_state_being_dropped() {
        // The state is moved into the game thread, so nothing can read it afterwards.
        // The handle is the only thing that outlives the move, and a test that
        // asserts on the handle after the drop is asserting it is a real handle.
        let metrics = {
            let mut state = a_state();
            let handle = state.metrics();
            state.process_pulse(9);
            handle
        };
        assert_eq!(metrics.pulses(), 9);
    }

    #[test]
    fn the_stored_number_is_the_pulse_just_stepped_and_not_a_count_of_pulses() {
        // The distinction matters because the two agree until a number is skipped or
        // repeated. An accumulating counter would pass a one-pulse test and then
        // report a different number from `GameLoopSummary::final_pulse`, which the
        // cross-thread test compares against. Both numbers are 7 after seven
        // consecutive pulses, and only this pair separates them.
        let mut state = a_state();
        for pulse in 1..=7 {
            state.process_pulse(pulse);
        }
        assert_eq!(state.last_pulse(), 7);
        assert_eq!(state.metrics().pulses(), 7);

        // A repeated pulse number, which the loop would not send, leaves the state
        // reporting that number and not a larger one. That is the whole content of
        // the property: the value is replaced, never added to.
        state.process_pulse(7);
        assert_eq!(state.last_pulse(), 7);
        assert_eq!(state.metrics().pulses(), 7);
    }

    #[test]
    fn the_state_is_send_because_moving_it_into_the_thread_is_the_whole_design() {
        // `spawn_game_loop` takes the processor by value and moves it onto a fresh
        // OS thread, so a `GameState` that is not `Send` would not compile at the
        // call site. Asserting it here means the failure names this module rather
        // than a caller's signature.
        fn assert_send<T: Send>() {}
        assert_send::<GameState>();
    }

    #[test]
    fn a_character_the_state_owns_is_reachable_by_name() {
        let mut state = a_state();
        assert!(
            state.characters().find_player_by_name("Shaman").is_err(),
            "an empty state resolves no name"
        );
        state
            .characters_mut()
            .create_player(7, "Shaman")
            .expect("the first player is free");
        assert!(
            state.characters().find_player_by_name("Shaman").is_ok(),
            "and resolves it once the character exists"
        );
    }

    #[test]
    fn the_pulse_period_is_the_legacy_forty_milliseconds() {
        // 25 Pulses per second is ADR-0002, and it is the number the loop already
        // runs at. This test exists so the state cannot be built against a different
        // rate later without this failing to be noticed.
        assert_eq!(crate::game_loop::PULSE_PERIOD, Duration::from_millis(40));
    }

    // ---- the crossing ----------------------------------------------------
    //
    // The tests below are the ones that matter for ledger 202. Each is written as
    // if the claim were false, so that a change that breaks the crossing has to
    // break a test rather than pass a weaker one.

    fn grant_for(target: &str, vnum: u32) -> GrantRequest {
        GrantRequest {
            target: target.to_owned(),
            vnum,
            count: None,
        }
    }

    /// A one-cell, non-custom vnum from the owner's table, so the grant lands in the
    /// base inventory and nothing else decides the cell.
    fn a_plain_vnum() -> u32 {
        let protos = owners();
        protos
            .rows()
            .iter()
            .find(|proto| {
                proto.size == 1
                    && (0..6).all(|category| {
                        !gamedata::item_custom_category::is_custom_category(proto, category)
                    })
            })
            .expect("the owner's table has a one-cell item outside every custom bank")
            .vnum
    }

    #[test]
    fn a_grant_command_reaches_the_world_and_answers_with_the_outcome() {
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        let command = GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        };

        state.apply(command);

        // The answer crossed, and it names the cell the world actually used.
        let outcome = answer
            .blocking_recv()
            .expect("the game thread answered")
            .expect("the grant happened");
        assert_eq!(
            outcome.record.cell.cell, 0,
            "the first free base cell is cell 0"
        );
        assert_eq!(outcome.record.vnum, vnum);
        assert_eq!(outcome.count, 1, "None means one");
        assert_eq!(outcome.bank, None, "and it went to the base inventory");
    }

    #[test]
    fn the_item_exists_in_the_world_and_not_only_in_the_answer() {
        // The answer is a copy. If the world were unchanged and only the answer
        // were right, a client would be told about an item that is not there. This
        // reads the world back through the same path the reducer used.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        });
        let outcome = answer.blocking_recv().unwrap().unwrap();

        let character = state
            .characters_mut()
            .find_player_mut("SHAMAN")
            .expect("name matching is case-insensitive");
        let items = character.items();
        assert_eq!(items.len(), 1, "the world holds the item");
        assert_eq!(
            items.get(outcome.record.cell),
            Lookup::Occupied(outcome.row.id),
            "the answer's id is the one sitting in the answer's cell"
        );
        assert_eq!(
            items.cell_of(outcome.row.id),
            Some(outcome.record.cell),
            "and the same id answers to the same cell from the other direction"
        );
        assert_eq!(outcome.row.vnum, vnum, "the same prototype");
        assert_eq!(
            outcome.row.owner_id,
            Some(character.player_id()),
            "owned by the target, and not on the ground"
        );
        assert_eq!(
            outcome.row.pos,
            u32::from(outcome.record.cell.cell),
            "one row, one cell"
        );
    }

    #[test]
    fn a_refusal_comes_back_as_a_refusal_and_leaves_the_world_alone() {
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let (reply, answer) = tokio::sync::oneshot::channel();

        state.apply(GameCommand::GrantItem {
            request: grant_for("Nobody", a_plain_vnum()),
            reply,
        });

        assert_eq!(
            answer.blocking_recv().unwrap(),
            Err(GrantRefusal::NoSuchCharacter {
                name: "Nobody".to_owned()
            }),
            "an unknown name is a refusal, not a missing answer"
        );
        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .unwrap()
                .items()
                .len(),
            0,
            "and nothing was placed anywhere"
        );
    }

    #[test]
    fn the_target_characters_own_inven_point_bounds_the_search() {
        // This is the reason `envanter` was added to `Character`. A grant has to be
        // placed against the target's own stat rather than against a number the
        // caller supplied, or an operator could hand out a cell the character
        // cannot draw. The test goes through the command, which is the only path
        // that has an `inven_point` to read at all.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();

        {
            let character = state.characters_mut().find_player_mut("Shaman").unwrap();
            assert_eq!(
                character.inven_point(),
                0,
                "a new character has the stat at 0, which buys the legacy 90 cells"
            );
        }

        // Fill those 90 cells through the command. The cell numbers are what make
        // this a test rather than a smoke test.
        for cell in 0..90u16 {
            let outcome = ask(&mut state, &grant_for("Shaman", vnum))
                .expect("a one-cell item fits while the stat is 0");
            assert_eq!(outcome.record.cell.cell, cell, "cells fill in order from 0");
        }
        assert_eq!(
            ask(&mut state, &grant_for("Shaman", vnum)),
            Err(GrantRefusal::NoRoom { size: 1 }),
            "the 91st cell is out of reach at stat 0"
        );

        // Raising the stat opens the rest of the base inventory, and the command
        // picks that up without the caller passing anything.
        state
            .characters_mut()
            .find_player_mut("Shaman")
            .unwrap()
            .set_inven_point(18);
        let outcome =
            ask(&mut state, &grant_for("Shaman", vnum)).expect("cell 90 is free at stat 18");
        assert_eq!(
            outcome.record.cell.cell, 90,
            "the raised stat opened the 91st cell"
        );
    }

    #[test]
    fn an_inven_point_above_the_base_inventory_is_clamped_on_write() {
        // Divergence 202.1. Legacy copies the stat out of the stored blob and never
        // clamps, and `usable_inventory_cells` is a bare sum, so a hand-edited row
        // can name a cell in the equipment window. The Rewrite refuses to store a
        // stat that would do that.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let character = state.characters_mut().find_player_mut("Shaman").unwrap();
        character.set_inven_point(u16::MAX);

        assert_eq!(
            character.inven_point(),
            INVENTORY_MAX_EXTENDED,
            "18 is the largest stat that still lands inside the base inventory"
        );
        assert_eq!(
            usable_inventory_cells(character.inven_point()),
            INVENTORY_MAX_NUM,
            "and at that stat the usable count is exactly the array length"
        );
        assert_eq!(INVENTORY_MAX_EXTENDED, 18, "the derived constant, pinned");
    }

    /// Sends one grant command to `state` on the calling thread and waits for it.
    ///
    /// Every crossing test goes through here so a change to the command shape breaks
    /// one place rather than six.
    fn ask(state: &mut GameState, request: &GrantRequest) -> Result<GrantOutcome, GrantRefusal> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::GrantItem {
            request: request.clone(),
            reply,
        });
        answer.blocking_recv().expect("the game thread answered")
    }

    #[test]
    fn a_dropped_reply_does_not_undo_the_grant() {
        // The caller is allowed to vanish: a descriptor can close while the world
        // is being changed. If that rolled the item back, a lost connection would
        // be a way to lose an item the operator was told was granted.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        drop(answer);

        state.apply(GameCommand::GrantItem {
            request: grant_for("Shaman", vnum),
            reply,
        });

        assert_eq!(
            state
                .characters_mut()
                .find_player_mut("Shaman")
                .unwrap()
                .items()
                .len(),
            1,
            "the item is still in the world; the listener went, not the item"
        );
    }

    #[test]
    fn apply_is_reachable_without_a_thread_so_a_grant_can_be_tested_directly() {
        // The method exists so the tests above do not have to spawn a thread. A
        // test that spawned one would be testing the scheduler, and a scheduler
        // bug would then be reported as a grant bug.
        let mut state = a_state();
        state
            .characters_mut()
            .create_player(11, "Shaman")
            .expect("the name is free");
        let vnum = a_plain_vnum();
        let (reply, answer) = tokio::sync::oneshot::channel();
        GameState::apply(
            &mut state,
            GameCommand::GrantItem {
                request: grant_for("Shaman", vnum),
                reply,
            },
        );
        assert!(answer.blocking_recv().unwrap().is_ok());
    }

    /// A character at VID 7 holding `items`, with no body, in a state whose allocator is
    /// installed or not and which hosts [`fixtures::PLACE`].
    fn a_holder(items: &[(ItemPos, world::item::Item)], ids: bool) -> GameState {
        let mut state = if ids {
            a_state()
        } else {
            GameState::new(owners())
        };
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world_with_items(Vid::new(7), 7, "Shaman", items, outbox)
            .expect("the character is admitted with its items");
        host_place(&mut state);
        state
    }

    /// Hosts [`fixtures::PLACE`] as a 10 by 10 sectree grid from the origin, so an item dropped
    /// there lands.
    fn host_place(state: &mut GameState) {
        let (channel, map) = fixtures::PLACE;
        state.host_map(
            channel,
            map,
            SectreeGrid {
                x: 0,
                y: 0,
                columns: 10,
                rows: 10,
            },
        );
    }

    /// A one-cell item of a real vnum, so the move finds its prototype.
    fn a_plain_item(id: u32, vnum: u32) -> world::item::Item {
        let mut item = world::item::Item::new(id, vnum);
        item.set_size(1).expect("a positive footprint");
        item
    }

    /// A stack of `count` small red potions, which the owner's table makes stackable.
    fn a_potion_stack(id: u32, count: u16) -> world::item::Item {
        let mut item = a_plain_item(id, 27_001);
        item.flags = world::item::ITEM_FLAG_STACKABLE;
        item.count = count;
        item
    }

    fn inventory(cell: u16) -> ItemPos {
        ItemPos::new(common::item_slots::EWindows::Inventory as u8, cell)
    }

    fn a_move(from: u16, to: u16, count: u16) -> MoveRequest {
        MoveRequest {
            from: inventory(from),
            to: inventory(to),
            count,
        }
    }

    fn a_mover() -> Mover {
        Mover {
            recently_fought: false,
            empire: 1,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PEACE,
            affect_flags: [0; 2],
        }
    }

    /// A look carries the PK mode of the body's card, not the descriptor's mover's
    /// (`UpdatePacket`, `G/char.cpp:1315`): the fixture body is level 10, in `PK_MODE_PROTECT`.
    #[test]
    fn a_look_carries_the_pk_mode_of_the_body() {
        let clock = fixtures::TestClock::default();
        let mut world = fixtures::a_world(&clock);
        let _viewer = fixtures::enter(&mut world, 8, (3300, 3200), 820);
        let card = world.bodies[&Vid::new(8)].card.clone();
        let (tx, _inbox) = tokio::sync::mpsc::unbounded_channel();
        let mut sword = world::item::Item::new(11, 10);
        sword.set_size(2).expect("a two-cell sword");
        world
            .enter_world_with_items(
                Vid::new(7),
                7,
                "Warrior",
                &[(inventory(0), sword)],
                ClientOutbox::new(tx),
            )
            .expect("the player enters");
        world
            .characters
            .find_by_vid_mut(Vid::new(7))
            .expect("the player is in the world")
            .set_points(Some(fixtures::points(820)));
        let (channel, map) = fixtures::PLACE;
        let place = crate::game_loop_messages::EnterPlace {
            channel,
            map,
            x: 3200,
            y: 3200,
            z: 0,
        };
        let _shown = world.place_body(Vid::new(7), place, card);
        let weapon = INVENTORY_MAX_NUM + common::enums::EWearPositions::Weapon as u16;
        let worn = world
            .move_item(Vid::new(7), a_move(0, weapon, 0), a_mover())
            .expect("the sword is worn");
        let look = worn
            .records
            .iter()
            .find(|frame| frame.first() == Some(&19))
            .expect("a GC_CHARACTER_UPDATE");
        assert_eq!(a_mover().pk_mode, crate::loading_phase::PK_MODE_PEACE);
        assert_eq!(look[42], crate::loading_phase::PK_MODE_PROTECT, "bPKMode");
    }

    fn held_at(state: &GameState, pos: ItemPos) -> Lookup {
        state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the character is in the world")
            .items()
            .get(pos)
    }

    fn encoded(record: world::character::ItemRecord) -> Vec<u8> {
        let mut frame = Vec::new();
        record.encode_into(&mut frame);
        frame
    }

    #[test]
    fn a_move_command_moves_the_item_and_answers_with_its_records_and_row() {
        let vnum = a_plain_vnum();
        let mut state = a_holder(&[(inventory(0), a_plain_item(11, vnum))], true);
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::MoveItem {
            vid: Vid::new(7),
            request: a_move(0, 5, 0),
            mover: a_mover(),
            reply,
        });
        let moved = answer
            .blocking_recv()
            .expect("the game thread answered")
            .expect("the move happened");
        assert_eq!(moved.kind, world::character::MoveKind::Moved);
        assert_eq!(
            moved.owner_id, 7,
            "the row owner is the store id, not the VID"
        );
        let stored = state
            .characters()
            .find_by_vid(Vid::new(7))
            .unwrap()
            .items()
            .item(11)
            .expect("the item is still held")
            .clone();
        assert_eq!(
            moved.records,
            vec![
                encoded(world::character::ItemRecord::Set(
                    world::item::gc_item_clear(inventory(0))
                )),
                encoded(world::character::ItemRecord::Set(
                    stored.gc_item_set(inventory(5), 0)
                )),
            ],
            "the old cell is cleared first, then the new one is set"
        );
        assert_eq!(
            moved.changes,
            vec![RowChange::Moved {
                id: 11,
                window_type: 1,
                pos: 5,
            }]
        );
        assert_eq!(held_at(&state, inventory(5)), Lookup::Occupied(11));
        assert_eq!(held_at(&state, inventory(0)), Lookup::Empty);
    }

    #[test]
    fn a_move_for_a_vid_nobody_holds_is_refused_by_name() {
        let mut state = a_holder(&[(inventory(0), a_plain_item(11, a_plain_vnum()))], true);
        assert_eq!(
            state.move_item(Vid::new(8), a_move(0, 5, 0), a_mover()),
            Err(MoveItemRefused::NoSuchCharacter { vid: Vid::new(8) })
        );
        assert_eq!(held_at(&state, inventory(0)), Lookup::Occupied(11));
    }

    #[test]
    fn a_refused_move_answers_why_and_leaves_the_world_alone() {
        let vnum = a_plain_vnum();
        let mut state = a_holder(
            &[
                (inventory(0), a_plain_item(11, vnum)),
                (inventory(1), a_plain_item(12, vnum)),
            ],
            true,
        );
        // Cell 90 is the first one a character with no extra pages has not unlocked.
        assert_eq!(
            state.move_item(Vid::new(7), a_move(0, 90, 0), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NoRoom))
        );
        assert_eq!(
            state.move_item(Vid::new(7), a_move(4, 5, 0), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::Empty))
        );
        assert_eq!(held_at(&state, inventory(0)), Lookup::Occupied(11));
        assert_eq!(held_at(&state, inventory(1)), Lookup::Occupied(12));
        assert_eq!(held_at(&state, inventory(90)), Lookup::Empty);
    }

    #[test]
    fn the_characters_unlocked_pages_bound_the_destination() {
        let mut state = a_holder(&[(inventory(0), a_plain_item(11, a_plain_vnum()))], true);
        state
            .characters_mut()
            .find_by_vid_mut(Vid::new(7))
            .unwrap()
            .set_inven_point(1);
        // One point unlocks one more row of five, so cell 94 is open and 95 is not.
        assert_eq!(
            state.move_item(Vid::new(7), a_move(0, 95, 0), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NoRoom))
        );
        assert!(state
            .move_item(Vid::new(7), a_move(0, 94, 0), a_mover())
            .is_ok());
        assert_eq!(held_at(&state, inventory(94)), Lookup::Occupied(11));
    }

    #[test]
    fn a_split_needs_the_allocator_and_stores_the_new_item_whole() {
        let items = [(inventory(0), a_potion_stack(11, 10))];
        let mut without = a_holder(&items, false);
        assert_eq!(
            without.move_item(Vid::new(7), a_move(0, 3, 4), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NoItemIds))
        );
        assert_eq!(held_at(&without, inventory(3)), Lookup::Empty);

        let mut state = a_holder(&items, true);
        let moved = state
            .move_item(Vid::new(7), a_move(0, 3, 4), a_mover())
            .expect("the split happens");
        assert_eq!(moved.kind, world::character::MoveKind::Split);
        let Lookup::Occupied(piece) = held_at(&state, inventory(3)) else {
            panic!("the new stack is at the destination");
        };
        assert_ne!(piece, 11, "the new stack has an id of its own");
        let [RowChange::Count { id: 11, count: 6 }, RowChange::Created(row)] =
            moved.changes.as_slice()
        else {
            panic!("the source count and then the new row: {:?}", moved.changes);
        };
        assert_eq!(row.id, piece);
        assert_eq!(row.count, 4);
        assert_eq!(row.vnum, 27_001);
        assert_eq!(row.owner_id, Some(7));
        assert_eq!((row.window_type, row.pos), (1, 3));
    }

    #[test]
    fn the_configured_count_limit_caps_a_merge() {
        let items = [
            (inventory(0), a_potion_stack(11, 8)),
            (inventory(1), a_potion_stack(12, 5)),
        ];
        let mut state = a_holder(&items, true).with_item_count_limit(10);
        let moved = state
            .move_item(Vid::new(7), a_move(0, 1, 0), a_mover())
            .expect("the merge happens");
        assert_eq!(moved.kind, world::character::MoveKind::Merged);
        assert_eq!(
            moved.changes,
            vec![
                RowChange::Count { id: 11, count: 3 },
                RowChange::Count { id: 12, count: 10 },
            ],
            "only five fit under a limit of ten"
        );

        // The default is the compiled-in 5000, so the same merge moves every potion.
        let mut state = a_holder(&items, true);
        let moved = state
            .move_item(Vid::new(7), a_move(0, 1, 0), a_mover())
            .expect("the merge happens");
        assert_eq!(
            moved.changes,
            vec![
                RowChange::Destroyed { id: 11 },
                RowChange::Count { id: 12, count: 13 },
            ]
        );
        assert_eq!(held_at(&state, inventory(0)), Lookup::Empty);
    }

    /// The owner's belt with the highest grade, which opens every belt cell.
    fn the_widest_belt() -> (u32, i32) {
        let protos = owners();
        let belt = gamedata::item_proto_value::type_value(b"ITEM_BELT").expect("a belt type");
        protos
            .rows()
            .iter()
            .filter(|proto| proto.item_type == belt)
            .map(|proto| (proto.vnum, proto.values[0]))
            .max_by_key(|&(_, grade)| grade)
            .expect("the owner's table has a belt")
    }

    #[test]
    fn a_belt_cell_opens_only_under_a_worn_belt_and_is_stored_in_the_belt_window() {
        let potion = [(inventory(0), a_potion_stack(11, 3))];
        let mut bare = a_holder(&potion, true);
        assert_eq!(
            bare.move_item(Vid::new(7), a_move(0, 274, 0), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NoRoom)),
            "no belt is worn, so no belt cell is open"
        );

        let (belt, grade) = the_widest_belt();
        assert!(grade > 0, "the widest belt opens at least one cell");
        let worn = inventory(crate::item_move::BELT_WEAR_CELL);
        let mut state = a_holder(&[potion[0].clone(), (worn, a_plain_item(20, belt))], true);
        let moved = state
            .move_item(Vid::new(7), a_move(0, 274, 0), a_mover())
            .expect("the worn belt opens its first cell");
        assert_eq!(
            moved.changes,
            vec![RowChange::Moved {
                id: 11,
                window_type: common::item_slots::EWindows::BeltInventory as u8,
                pos: 0,
            }]
        );
        assert_eq!(held_at(&state, inventory(274)), Lookup::Occupied(11));
    }

    #[test]
    fn metrics_start_at_zero_and_only_the_thread_advances_them() {
        // The negative control for "the game thread owns this state": before the
        // thread runs, the counter is zero, so a test that observes a non-zero value
        // has observed the thread and nothing else.
        let metrics = a_state().metrics();
        let deadline = Instant::now() + Duration::from_millis(50);
        while Instant::now() < deadline {
            assert_eq!(metrics.pulses(), 0, "no thread was started");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn a_place(map: i32, x: i32, y: i32) -> GroundPlace {
        GroundPlace {
            channel: 1,
            map,
            x,
            y,
        }
    }

    /// The ground VIDs lying on `place`, each with the point the map's view holds it at.
    fn lying_on(state: &GameState, place: (u8, i32)) -> Vec<(u32, i32, i32)> {
        state
            .ground
            .iter()
            .filter(|(_, lying)| (lying.channel, lying.map) == place)
            .map(|(&vid, _)| {
                let spot = state.maps[&place]
                    .spot(view::EntityKey::Ground(vid))
                    .expect("a lying item is in its map's view");
                (vid, spot.x, spot.y)
            })
            .collect()
    }

    fn drop_at(state: &mut GameState, cell: u16, count: u16) -> MovedItems {
        state
            .drop_item(
                Vid::new(7),
                inventory(cell),
                count,
                a_place(41, 500, 700),
                a_mover(),
            )
            .expect("the drop happened")
    }

    #[test]
    fn a_dropped_item_lies_on_its_map_until_it_is_picked_up_again() {
        let vnum = a_plain_vnum();
        let mut state = a_holder(&[(inventory(3), a_plain_item(11, vnum))], true);
        let dropped = drop_at(&mut state, 3, 0);
        assert_eq!(dropped.kind, world::character::MoveKind::Dropped);
        assert_eq!(held_at(&state, inventory(3)), Lookup::Empty);
        assert_eq!(lying_on(&state, (1, 41)), vec![(1, 500, 700)]);
        assert_eq!(state.ground[&1].ground.item.vnum, vnum);
        assert!(lying_on(&state, (1, 42)).is_empty(), "another map");
        assert!(lying_on(&state, (2, 41)).is_empty(), "another Channel");

        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::PickupItem {
            vid: Vid::new(7),
            ground: 1,
            place: a_place(41, 700, 700),
            mover: a_mover(),
            reply,
        });
        let picked = answer
            .blocking_recv()
            .expect("the game thread answered")
            .expect("the pick-up happened");
        assert_eq!(picked.kind, world::character::MoveKind::PickedUp);
        assert_eq!(held_at(&state, inventory(0)), Lookup::Occupied(11));
        assert!(lying_on(&state, (1, 41)).is_empty());
        assert!(
            state.maps[&(1, 41)]
                .spot(view::EntityKey::Ground(1))
                .is_none(),
            "the item left the view"
        );
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, a_place(41, 500, 700), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NotOnGround)),
            "the item is gone once picked up"
        );
    }

    #[test]
    fn a_pick_up_line_is_looked_up_in_the_pickers_language() {
        let german =
            gamedata::locale_string::LanguageTable::parse(b"\"[LS;444;%s]\";\"Aufgehoben: %s\";\n");
        let strings = LocaleStrings::default().with_table(5, german);
        let mut state = a_holder(&[(inventory(3), a_plain_item(11, a_plain_vnum()))], true)
            .with_locale_strings(Arc::new(strings));
        let says = |records: &[Vec<u8>], text: &[u8]| {
            records
                .iter()
                .any(|record| record.windows(text.len()).any(|part| part == text))
        };
        // Each drop takes the next ground VID.
        for (ground, language, text) in [(1, 5, &b"Aufgehoben: "[..]), (2, 1, &b"[LS;444;"[..])] {
            drop_at(&mut state, 3, 0);
            let mover = Mover {
                language,
                ..a_mover()
            };
            let picked = state
                .pickup_item(Vid::new(7), ground, a_place(41, 500, 700), mover)
                .expect("the pick-up happened");
            assert!(says(&picked.records, text), "language {language}");
            let _moved = state
                .move_item(Vid::new(7), a_move(0, 3, 0), a_mover())
                .expect("the item goes back to cell 3");
        }
    }

    #[test]
    fn each_drop_takes_the_next_ground_vid_and_a_refused_drop_takes_none() {
        let vnum = a_plain_vnum();
        let mut state = a_holder(
            &[
                (inventory(0), a_potion_stack(11, 5)),
                (inventory(1), a_plain_item(12, vnum)),
            ],
            true,
        );
        drop_at(&mut state, 0, 2);
        assert_eq!(
            state.drop_item(Vid::new(7), inventory(9), 0, a_place(41, 1, 1), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::Empty))
        );
        drop_at(&mut state, 1, 0);
        let vids: Vec<u32> = state.ground.keys().copied().collect();
        assert_eq!(vids, vec![1, 2]);
        assert_eq!(state.ground[&1].ground.item.count, 2);
        assert_eq!(state.ground[&2].ground.item.id, 12);
    }

    #[test]
    fn a_pick_up_from_another_map_or_past_the_approximate_three_hundred_is_refused_and_the_item_stays(
    ) {
        let mut state = a_holder(&[(inventory(0), a_plain_item(11, a_plain_vnum()))], true);
        drop_at(&mut state, 0, 0);
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, a_place(42, 500, 700), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NotOnGround))
        );
        let far = GroundPlace {
            channel: 2,
            ..a_place(41, 500, 700)
        };
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, far, a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NotOnGround)),
            "another Channel"
        );
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, a_place(41, 814, 700), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::TooFar)),
            "DISTANCE_APPROX puts 314 along one axis at 301"
        );
        assert_eq!(
            state.pickup_item(Vid::new(8), 1, a_place(41, 800, 700), a_mover()),
            Err(MoveItemRefused::NoSuchCharacter { vid: Vid::new(8) })
        );
        assert_eq!(lying_on(&state, (1, 41)).len(), 1);
        state
            .pickup_item(Vid::new(7), 1, a_place(41, 813, 700), a_mover())
            .expect("313 along one axis approximates to 300, which is in reach");
    }

    #[test]
    fn a_pick_up_that_finds_no_room_leaves_the_item_on_the_ground() {
        let vnum = a_plain_vnum();
        let items: Vec<(ItemPos, world::item::Item)> = (0..90)
            .map(|cell| (inventory(cell), a_plain_item(100 + u32::from(cell), vnum)))
            .collect();
        let mut state = a_holder(&items, true);
        drop_at(&mut state, 0, 0);
        state
            .characters_mut()
            .find_by_vid_mut(Vid::new(7))
            .unwrap()
            .items_mut()
            .set(inventory(0), &a_plain_item(99, vnum))
            .expect("the cell was freed by the drop");
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, a_place(41, 500, 700), a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::NoRoomToPickUp))
        );
        assert_eq!(lying_on(&state, (1, 41)), vec![(1, 500, 700)]);
    }

    /// A pick-up stores only after the drop that laid its item: it waits on the channel the
    /// drop tells once its rows are stored, as a remainder's pick-up does, and a pick-up whose
    /// drop ended unstored is told so.
    #[tokio::test]
    async fn a_pick_up_waits_for_the_store_of_the_drop_that_laid_its_item() {
        let vnum = a_plain_vnum();
        let items: Vec<(ItemPos, world::item::Item)> = [
            (inventory(0), a_potion_stack(11, 5)),
            (inventory(1), a_potion_stack(12, 8)),
        ]
        .into_iter()
        .chain((2..90).map(|cell| (inventory(cell), a_plain_item(100 + u32::from(cell), vnum))))
        .collect();
        let mut state = a_holder(&items, true).with_item_count_limit(10);
        let mut dropped = drop_at(&mut state, 0, 0);
        assert!(dropped.store_order.turn().await, "a drop waits on nothing");
        // With cell 0 taken again, the pick-up tops stack 12 up to ten and leaves three lying.
        state
            .characters_mut()
            .find_by_vid_mut(Vid::new(7))
            .unwrap()
            .items_mut()
            .set(inventory(0), &a_plain_item(99, vnum))
            .expect("the cell was freed by the drop");
        let mut picked = state
            .pickup_item(Vid::new(7), 1, a_place(41, 500, 700), a_mover())
            .expect("part is picked up");
        assert_eq!(picked.kind, world::character::MoveKind::Declined);
        assert_eq!(state.ground[&1].ground.item.count, 3);
        assert_eq!(
            picked.store_order,
            StoreOrder::after(state.ground[&1].stored.clone()),
            "the remainder keeps the drop's channel"
        );
        let early = tokio::time::timeout(Duration::from_millis(20), picked.store_order.turn());
        assert!(early.await.is_err(), "the drop has not stored");
        dropped.store_order.stored();
        assert!(picked.store_order.turn().await);
        assert!(*state.ground[&1].stored.borrow());

        // A drop that ends unstored tells its pick-up the rows never will be.
        let mut state = a_holder(&[(inventory(3), a_plain_item(11, vnum))], true);
        let dropped = drop_at(&mut state, 3, 0);
        let mut picked = state
            .pickup_item(Vid::new(7), 1, a_place(41, 500, 700), a_mover())
            .expect("the pick-up happened");
        drop(dropped);
        assert!(
            !picked.store_order.turn().await,
            "the drop was never stored"
        );
    }

    /// A character that leaves the world ends its waiting script (`CQuestManager::DisconnectPC`,
    /// `G/char.cpp:1802`).
    #[test]
    fn a_character_that_leaves_the_world_ends_its_waiting_script() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        state
            .enter_world(Vid::new(7), 7, "Shaman", outbox)
            .expect("the character is admitted");
        state.start_a_quest(Vid::new(7));
        let running = |state: &GameState| state.quests().expect("loaded").manager().is_running(7);
        assert!(running(&state));
        assert!(state.leave_world(Vid::new(7)).is_some());
        assert!(!running(&state));
    }

    /// While a script waits (`@fixme150`, `G/char_item.cpp:7350`, `:7479`, `:7987`) the
    /// character uses and drops nothing and picks up no quest item; once it ends, it does.
    #[test]
    fn a_character_whose_script_waits_uses_drops_and_picks_up_no_quest_item() {
        /// An `ITEM_QUEST` the owner's table lets be dropped.
        const LETTER: u32 = 25_104;
        let (mut state, _inbox) = a_hurt_drinker();
        host_place(&mut state);
        let items = state
            .characters_mut()
            .find_by_vid_mut(Vid::new(7))
            .unwrap()
            .items_mut();
        items.set(inventory(5), &a_plain_item(12, LETTER)).unwrap();
        let plain = a_plain_item(13, a_plain_vnum());
        items.set(inventory(6), &plain).unwrap();
        drop_at(&mut state, 5, 0);
        drop_at(&mut state, 6, 0);
        state.start_a_quest(Vid::new(7));
        let refused = |reason| Err(MoveItemRefused::Refused(reason));
        let used = state.use_item(Vid::new(7), inventory(0), a_mover());
        assert_eq!(
            used.map(|moved| moved.kind),
            refused(MoveRefused::UsedWhileQuesting)
        );
        let place = a_place(41, 500, 700);
        let dropped = state.drop_item(Vid::new(7), inventory(0), 0, place, a_mover());
        let kind = dropped.map(|moved| moved.kind);
        assert_eq!(kind, refused(MoveRefused::DroppedWhileQuesting));
        let picked = state.pickup_item(Vid::new(7), 1, place, a_mover());
        let kind = picked.map(|moved| moved.kind);
        assert_eq!(kind, refused(MoveRefused::PickedUpWhileQuesting));
        assert_eq!(lying_on(&state, (1, 41)).len(), 2, "the letter stays");
        let picked = state.pickup_item(Vid::new(7), 2, place, a_mover());
        assert_eq!(
            picked.map(|moved| moved.kind),
            Ok(world::character::MoveKind::PickedUp)
        );
        state.end_the_quest(Vid::new(7));
        let picked = state.pickup_item(Vid::new(7), 1, place, a_mover());
        assert_eq!(
            picked.map(|moved| moved.kind),
            Ok(world::character::MoveKind::PickedUp)
        );
        let used = state.use_item(Vid::new(7), inventory(0), a_mover());
        assert_eq!(
            used.map(|moved| moved.kind),
            Ok(world::character::MoveKind::Used)
        );
    }

    /// Puts a one-cell item of a real vnum in the inventory cell `cell` of the player `vid`,
    /// and answers its vnum.
    fn hold(state: &mut GameState, vid: u32, cell: u16) -> u32 {
        let vnum = a_plain_vnum();
        state
            .characters_mut()
            .find_by_vid_mut(Vid::new(vid))
            .expect("the player is in the world")
            .items_mut()
            .set(inventory(cell), &a_plain_item(11, vnum))
            .expect("the cell is free");
        vnum
    }

    fn drained(inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// 7 standing at (3200, 3200) and holding an item in cell 3, 8 in its view, 9 out of it;
    /// every inbox is drained.
    fn three_on_the_ground() -> (
        GameState,
        [tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>; 3],
    ) {
        let clock = fixtures::TestClock::default();
        let mut state = fixtures::a_world(&clock);
        let mut inboxes = [
            fixtures::enter(&mut state, 7, (3200, 3200), 820),
            fixtures::enter(&mut state, 8, (3300, 3200), 820),
            fixtures::enter(&mut state, 9, (60_000, 60_000), 820),
        ];
        let _vnum = hold(&mut state, 7, 3);
        for inbox in &mut inboxes {
            let _ = drained(inbox);
        }
        (state, inboxes)
    }

    fn drop_seven(state: &mut GameState) -> MovedItems {
        state
            .drop_item(Vid::new(7), inventory(3), 0, a_place(41, 1, 1), a_mover())
            .expect("the drop happened")
    }

    /// A drop lays the item at the dropper's point (`AddToGround`, `G/item.cpp:549-584`): each
    /// player that sees the point is sent its add at once, the dropper's own add takes the
    /// ground record's place after the cell's record, and a player out of view is sent nothing.
    #[test]
    fn a_drop_shows_the_item_to_its_view_and_the_dropper_in_its_records() {
        let (mut state, [mut seven, mut eight, mut far]) = three_on_the_ground();
        // 7 stands at a height, so the item takes the body's z as well as its x and y.
        let mut effects = Vec::new();
        let place = state
            .maps
            .get_mut(&fixtures::PLACE)
            .expect("the map is hosted");
        assert!(place.show(
            view::EntityKey::Character(7),
            (3200, 3200, 55),
            10_500,
            &mut effects
        ));
        let dropped = drop_seven(&mut state);
        let spot = state.maps[&fixtures::PLACE]
            .spot(view::EntityKey::Ground(1))
            .expect("the item is in the view");
        assert_eq!(
            (spot.x, spot.y, spot.z),
            (3200, 3200, 55),
            "the body's point"
        );
        let add = view_encode::ground_add(1, a_plain_vnum(), spot);
        assert_eq!(drained(&mut eight), vec![add.clone()]);
        assert!(drained(&mut far).is_empty());
        assert!(
            drained(&mut seven).is_empty(),
            "the dropper's add rides with its records"
        );
        assert_eq!(dropped.ground_at, Some(1), "after the cell's record");
        assert_eq!(dropped.records.len(), 3, "the cell, the add, the notice");
        assert_eq!(dropped.records[1], add);
        assert_eq!(state.ground[&1].ground.x, 3200, "picked up where it lies");
    }

    /// An item picked up whole leaves the view (`RemoveFromGround`, `G/item.cpp:533-547`): the
    /// players that see it are sent its del at once, and the picker's own del comes first in
    /// its records.
    #[test]
    fn a_whole_pick_up_takes_the_item_out_of_its_view() {
        let (mut state, [mut seven, mut eight, mut far]) = three_on_the_ground();
        let _dropped = drop_seven(&mut state);
        let _ = drained(&mut eight);
        let picked = state
            .pickup_item(Vid::new(7), 1, a_place(41, 3200, 3200), a_mover())
            .expect("the pick-up happened");
        let del = view_encode::ground_del(1);
        assert_eq!(drained(&mut eight), vec![del.clone()]);
        assert!(drained(&mut far).is_empty());
        assert!(drained(&mut seven).is_empty());
        assert_eq!(picked.ground_at, Some(0));
        assert_eq!(picked.records[0], del);
        assert_eq!(
            picked
                .records
                .iter()
                .filter(|record| **record == del)
                .count(),
            1
        );
        assert!(state.maps[&fixtures::PLACE]
            .spot(view::EntityKey::Ground(1))
            .is_none());
    }

    /// A player shown where an item lies is sent its add with the rest of its view, after the
    /// players (V1); one shown out of its view is not.
    #[test]
    fn an_entrant_is_shown_the_items_lying_in_its_view() {
        let (mut state, _inboxes) = three_on_the_ground();
        let _dropped = drop_seven(&mut state);
        let add = view_encode::ground_add(
            1,
            a_plain_vnum(),
            state.maps[&fixtures::PLACE]
                .spot(view::EntityKey::Ground(1))
                .expect("the item is in the view"),
        );
        let (_near, shown) =
            fixtures::enter_seeing(&mut state, fixtures::PLACE, 10, (3400, 3300), 820);
        assert_eq!(shown.last(), Some(&add));
        assert_eq!(shown.iter().filter(|record| **record == add).count(), 1);
        let (_far, shown) =
            fixtures::enter_seeing(&mut state, fixtures::PLACE, 11, (60_100, 60_000), 820);
        assert!(!shown.contains(&add));
    }

    /// A drop where no sectree is keeps the item, takes no ground VID and sends nothing; legacy
    /// takes the item from the cell and loses it (a Defect not reproduced).
    #[test]
    fn a_drop_where_no_sectree_is_keeps_the_item() {
        let clock = fixtures::TestClock::default();
        let mut state = fixtures::a_world(&clock);
        let mut lost = fixtures::enter(&mut state, 7, (70_000, 3200), 820);
        let _vnum = hold(&mut state, 7, 3);
        let _ = drained(&mut lost);
        let refused = state.drop_item(Vid::new(7), inventory(3), 0, a_place(41, 1, 1), a_mover());
        assert_eq!(
            refused,
            Err(MoveItemRefused::Refused(MoveRefused::NoSectree))
        );
        assert_eq!(held_at(&state, inventory(3)), Lookup::Occupied(11));
        assert!(state.ground.is_empty());
        assert_eq!(state.next_ground_vid, 1);
        assert!(drained(&mut lost).is_empty());
        let mut holder = a_holder(&[(inventory(3), a_plain_item(11, a_plain_vnum()))], true);
        let off_the_grid = holder.drop_item(
            Vid::new(7),
            inventory(3),
            0,
            a_place(41, 64_000, 700),
            a_mover(),
        );
        assert_eq!(
            off_the_grid,
            Err(MoveItemRefused::Refused(MoveRefused::NoSectree)),
            "with no body, the descriptor's point"
        );
        let unhosted = holder.drop_item(
            Vid::new(7),
            inventory(3),
            0,
            a_place(42, 500, 700),
            a_mover(),
        );
        assert_eq!(
            unhosted,
            Err(MoveItemRefused::Refused(MoveRefused::NoSectree))
        );
    }

    /// The destroy event fires on its pulse, and each player that sees the item is sent its del
    /// (`CItem::DestroyEvent`, then `RemoveFromGround`). Two items due on one Pulse go in
    /// ground-VID order (V1).
    #[test]
    fn the_destroy_event_fires_on_its_pulse_and_the_items_view_hears_it() {
        let (state, [mut seven, mut eight, mut far]) = three_on_the_ground();
        let mut state = state.with_drop_lifetime(2);
        state
            .characters_mut()
            .find_by_vid_mut(Vid::new(7))
            .expect("the player is in the world")
            .items_mut()
            .set(inventory(4), &a_plain_item(12, a_plain_vnum()))
            .expect("the cell is free");
        state.process_pulse(10);
        let _dropped = drop_seven(&mut state);
        let _second = state
            .drop_item(Vid::new(7), inventory(4), 0, a_place(41, 1, 1), a_mover())
            .expect("the second drop happened");
        let _ = drained(&mut eight);
        state.process_pulse(59);
        assert_eq!(
            lying_on(&state, fixtures::PLACE).len(),
            2,
            "one pulse early"
        );
        assert!(drained(&mut eight).is_empty());
        state.process_pulse(60);
        assert!(state.ground.is_empty());
        let dels = vec![view_encode::ground_del(1), view_encode::ground_del(2)];
        assert_eq!(drained(&mut seven), dels);
        assert_eq!(drained(&mut eight), dels);
        assert!(drained(&mut far).is_empty(), "out of view");
        assert!(state.maps[&fixtures::PLACE]
            .spot(view::EntityKey::Ground(1))
            .is_none());
    }

    /// A ground item's `PacketAround` (`G/entity.cpp:88-105`), which `SetOwnership` will send
    /// its `GC_ITEM_OWNERSHIP` through: one copy to each player that sees the item and none to
    /// a player out of view; the item has no descriptor, so it is sent nothing itself.
    #[test]
    fn a_ground_item_sends_around_to_the_players_that_see_it() {
        let (mut state, [mut seven, mut eight, mut far]) = three_on_the_ground();
        let _dropped = drop_seven(&mut state);
        let _ = (drained(&mut seven), drained(&mut eight));
        let record = vec![0xab, 0xcd];
        let item = view::EntityKey::Ground(1);

        state.packet_around(item, &record, None);
        assert_eq!(drained(&mut seven), vec![record.clone()]);
        assert_eq!(drained(&mut eight), vec![record.clone()]);
        state.packet_around(item, &record, Some(view::EntityKey::Character(8)));
        assert_eq!(drained(&mut seven), vec![record]);
        assert!(drained(&mut eight).is_empty(), "the except");
        assert!(drained(&mut far).is_empty(), "out of view");
    }

    /// A `CG_SYNC_POSITION` naming a ground item's VID finds nothing: `CHARACTER_MANAGER::Find`
    /// finds characters only (`G/input_main.cpp:2062`), and a ground VID is not one.
    #[test]
    fn a_sync_naming_a_ground_vid_finds_nothing() {
        use protocol::cg_variable::{SyncPositionElement, SyncPositionPacket};

        let (mut state, [mut seven, mut eight, _far]) = three_on_the_ground();
        let _dropped = drop_seven(&mut state);
        let _ = (drained(&mut seven), drained(&mut eight));
        let packet = SyncPositionPacket {
            declared_size: crate::sync_position::SYNC_POSITION_PREFIX_SIZE + 12,
            elements: vec![SyncPositionElement {
                vid: 1,
                x: 3250,
                y: 3200,
            }],
        };

        let result = state
            .sync_positions(Vid::new(8), &packet)
            .expect("8 has a body");
        assert_eq!(result.processed_elements, 1);
        assert!(result.accepted_elements.is_empty());
        assert_eq!(result.close_reason, None);
        assert!(drained(&mut seven).is_empty() && drained(&mut eight).is_empty());
        let spot = state.maps[&fixtures::PLACE]
            .spot(view::EntityKey::Ground(1))
            .expect("the item lies where it fell");
        assert_eq!((spot.x, spot.y), (3200, 3200));
    }

    #[test]
    fn the_drop_lifetime_is_counted_in_pulses_and_never_zero() {
        assert_eq!(drop_lifetime_pulses(300), 7_500);
        assert_eq!(drop_lifetime_pulses(1), 25);
        assert_eq!(drop_lifetime_pulses(0), 1);
        assert_eq!(drop_lifetime_pulses(-5), 1);
        assert_eq!(GameState::new(owners()).drop_lifetime, 7_500);
    }

    /// A level 10 warrior at VID 7, 500 short in both pools, holding two small red potions,
    /// with a receiver the test keeps.
    fn a_hurt_drinker() -> (GameState, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let mut state = a_state();
        let (outbox, inbox) = a_live_outbox();
        state
            .enter_world_with_items(
                Vid::new(7),
                7,
                "Shaman",
                &[(inventory(0), a_potion_stack(11, 2))],
                outbox,
            )
            .expect("the character is admitted with its potions");
        let mut points = Points::load(&world::character::PointsRow {
            race: 0,
            level: 10,
            conqueror_level: 0,
            st: 6,
            ht: 4,
            dx: 3,
            iq: 3,
            sungma: [0; 4],
            hp: 10_000,
            sp: 10_000,
            stamina: 820,
            inven_point: 0,
            map_index: 1,
            part_base: 0,
            hair_part: 0,
            sash_part: 0,
        });
        let _ = points.compute_points();
        for kind in [common::point_slot::POINT_HP, common::point_slot::POINT_SP] {
            let _ = points
                .point_change(kind, -500, false, false)
                .expect("a pool changes");
        }
        state
            .characters
            .find_by_vid_mut(Vid::new(7))
            .expect("the character is in the world")
            .set_points(Some(points));
        (state, inbox)
    }

    fn points_at_seven(state: &GameState) -> Points {
        state
            .characters()
            .find_by_vid(Vid::new(7))
            .expect("the character is in the world")
            .points()
            .cloned()
            .expect("the character has points")
    }

    #[test]
    fn a_slow_potion_starts_a_recovery_that_the_pulse_pays_out_a_second_later() {
        let (mut state, mut inbox) = a_hurt_drinker();
        let before = points_at_seven(&state);
        state.process_pulse(3);
        let moved = state
            .use_item(Vid::new(7), inventory(0), a_mover())
            .expect("the potion is drunk");
        assert_eq!(moved.kind, world::character::MoveKind::Used);
        assert_eq!(
            state.affect_events.get(&Vid::new(7)).map(|event| event.due),
            Some(28)
        );
        let owed = points_at_seven(&state).get_point(common::point_slot::POINT_HP_RECOVERY);
        assert_eq!(owed, 300, "the small red potion owes 300 HP");
        assert_eq!(
            points_at_seven(&state).hp(),
            before.hp(),
            "nothing paid yet"
        );

        while inbox.try_recv().is_ok() {}
        state.process_pulse(27);
        assert!(
            inbox.try_recv().is_err(),
            "the event is not due before pulse 28"
        );
        state.process_pulse(28);
        let paid = points_at_seven(&state);
        let step = (before.max_hp() * 7 / 100).min(300);
        assert_eq!(paid.hp(), before.hp() + step);
        assert_eq!(
            paid.get_point(common::point_slot::POINT_HP_RECOVERY),
            300 - step
        );
        let frames: Vec<Vec<u8>> = std::iter::from_fn(|| inbox.try_recv().ok()).collect();
        assert!(!frames.is_empty(), "the payment reaches the client");
        assert_eq!(
            state.affect_events.get(&Vid::new(7)).map(|event| event.due),
            Some(53)
        );

        for pulse in (53..).step_by(25).take(20) {
            state.process_pulse(pulse);
        }
        let done = points_at_seven(&state);
        assert_eq!(done.hp(), before.hp() + 300, "the whole recovery was paid");
        assert_eq!(done.get_point(common::point_slot::POINT_HP_RECOVERY), 0);
        assert!(
            state.affect_events.is_empty(),
            "the event ended with the recovery"
        );
    }

    #[test]
    fn a_second_potion_does_not_restart_a_running_recovery() {
        let (mut state, _inbox) = a_hurt_drinker();
        let _ = state
            .use_item(Vid::new(7), inventory(0), a_mover())
            .expect("the first potion is drunk");
        state.process_pulse(10);
        let _ = state
            .use_item(Vid::new(7), inventory(0), a_mover())
            .expect("the second potion is drunk");
        assert_eq!(
            state.affect_events.get(&Vid::new(7)).map(|event| event.due),
            Some(25)
        );
    }

    #[test]
    fn leaving_hands_back_the_points_and_ends_the_recovery() {
        let (mut state, _inbox) = a_hurt_drinker();
        let _ = state
            .use_item(Vid::new(7), inventory(0), a_mover())
            .expect("the potion is drunk");
        let expected = points_at_seven(&state);
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::KeptOf {
            vid: Vid::new(7),
            reply,
        });
        let kept = answer
            .blocking_recv()
            .unwrap()
            .expect("the character is online");
        assert_eq!(kept.points, Some(expected.clone()));

        let departed = state
            .leave_world(Vid::new(7))
            .expect("the character was online");
        assert_eq!(departed.points, Some(expected));
        assert_eq!(departed.quickslots, kept.quickslots);
        assert!(state.affect_events.is_empty());
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::KeptOf {
            vid: Vid::new(7),
            reply,
        });
        assert_eq!(answer.blocking_recv().unwrap(), None);
    }

    fn slot_answer(state: &mut GameState, step: QuickslotStep) -> QuickslotAnswer {
        state
            .quickslot(Vid::new(7), step)
            .expect("the character is in the world")
    }

    #[test]
    fn a_quickslot_request_answers_its_records_and_the_slots_after_it() {
        let mut state = a_holder(&[(inventory(0), a_potion_stack(11, 2))], true);
        let potion = world::character::Quickslot { kind: 1, pos: 0 };
        let added = slot_answer(
            &mut state,
            QuickslotStep::Add {
                slot: 3,
                quickslot: potion,
            },
        );
        assert_eq!(added.records, vec![vec![28, 3, 1, 0]]);
        assert_eq!(added.quickslots.get(3), Some(potion));
        // An empty cell is refused without a record, and the slots stay as they were.
        let refused = slot_answer(
            &mut state,
            QuickslotStep::Add {
                slot: 4,
                quickslot: world::character::Quickslot { kind: 1, pos: 5 },
            },
        );
        assert!(refused.records.is_empty());
        assert_eq!(refused.quickslots, added.quickslots);
        let swapped = slot_answer(&mut state, QuickslotStep::Swap { slot: 3, with: 9 });
        assert_eq!(swapped.records, vec![vec![30, 3, 9]]);
        assert_eq!(swapped.quickslots.get(9), Some(potion));
        let deleted = slot_answer(&mut state, QuickslotStep::Del { slot: 9 });
        assert_eq!(deleted.records, vec![vec![29, 9]]);
        assert_eq!(deleted.quickslots, world::character::Quickslots::default());
        assert_eq!(
            state.quickslot(Vid::new(8), QuickslotStep::Del { slot: 0 }),
            None,
            "nobody is online under VID 8"
        );
    }

    #[test]
    fn an_item_step_answers_its_slot_records_and_the_slots_after_it() {
        let placed = [
            (inventory(0), a_potion_stack(11, 2)),
            (inventory(5), a_potion_stack(12, 3)),
        ];
        let mut state = a_holder(&placed, true);
        let potion = |pos| world::character::Quickslot { kind: 1, pos };
        let step = QuickslotStep::Add {
            slot: 3,
            quickslot: potion(0),
        };
        let _added = slot_answer(&mut state, step);
        // A whole move: the slot follows, after the set.
        let moved = state
            .move_item(Vid::new(7), a_move(0, 1, 0), a_mover())
            .expect("the potion moves");
        assert_eq!(moved.records.len(), 3);
        assert_eq!(moved.records[2], vec![28, 3, 1, 1]);
        let slots = moved.quickslots.expect("the step answers the slots");
        assert_eq!(slots.get(3), Some(potion(1)));
        // A merge that uses up the source: the small red potion is `USE_POTION`, so the clear
        // goes first and the slot then follows the other stack.
        let merged = state
            .move_item(Vid::new(7), a_move(1, 5, 0), a_mover())
            .expect("the stacks merge");
        assert_eq!(merged.records.len(), 3);
        assert_eq!(merged.records[1], vec![28, 3, 1, 5]);
        let slots = merged.quickslots.expect("the step answers the slots");
        assert_eq!(slots.get(3), Some(potion(5)));
        // A drop of part of the stack deletes the slot first.
        let dropped = drop_at(&mut state, 5, 1);
        assert_eq!(dropped.records[0], vec![29, 3]);
        assert_eq!(
            dropped.quickslots,
            Some(world::character::Quickslots::default())
        );
    }

    #[test]
    fn the_world_starts_from_the_quickslots_and_the_gold_the_load_set() {
        let mut state = a_state();
        let (outbox, _inbox) = a_live_outbox();
        let mut quickslots = world::character::Quickslots::default();
        let skill = world::character::Quickslot { kind: 2, pos: 4 };
        let _set = quickslots.set(6, skill, &mut Vec::new());
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::EnterWorld {
            vid: Vid::new(7),
            player_id: 7,
            name: "Shaman".to_string(),
            items: Vec::new(),
            loaded: Box::new(crate::game_loop_messages::Loaded {
                points: None,
                quickslots,
                gold: 1_200_000_000_000_000_000 - 1,
                show: None,
            }),
            outbox,
            reply,
        });
        assert_eq!(
            answer.blocking_recv().unwrap(),
            Ok(crate::game_loop_messages::Shown::default()),
            "an entry with no body shows nothing"
        );
        let character = state.characters().find_by_vid(Vid::new(7)).unwrap();
        assert_eq!(character.gold(), 1_200_000_000_000_000_000 - 1);
        let swapped = slot_answer(&mut state, QuickslotStep::Swap { slot: 6, with: 0 });
        assert_eq!(swapped.quickslots.get(0), Some(skill));
    }

    fn an_npc_map(index: i32) -> MapRegion {
        MapRegion {
            index,
            name: b"test".to_vec(),
            sx: 10_000,
            sy: 20_000,
            ex: 12_000,
            ey: 22_000,
            spawn: (0, 0),
            empire_spawns: None,
        }
    }

    fn a_smith_at(x: i32, y: i32) -> RegenEntry {
        RegenEntry {
            kind: gamedata::regen::RegenKind::Mob,
            sx: x,
            sy: y,
            ex: x,
            ey: y,
            z_section: 0,
            direction: 1,
            time: 60,
            max_count: 1,
            vnum: 20_016,
        }
    }

    /// Boot stands the NPCs up per (Channel, map), and `NpcsOn` shares the same table.
    #[test]
    fn the_npcs_boot_stood_up_are_shared_per_channel_and_map() {
        use gamedata::mob_locale_names::MobLocaleNames;
        use gamedata::mob_proto::{MobProto, MobProtos, CHAR_TYPE_NPC};
        use gamedata::records::MobTableRecord;
        use world::npc::NpcVids;

        let protos = MobProtos::from_rows(vec![MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: 20_016,
                mob_type: CHAR_TYPE_NPC,
                ..MobTableRecord::default()
            },
        }]);
        let names = MobLocaleNames::parse(b"VNUM\tNAME\n20016\tSmith\n").unwrap();
        let mut spawner = NpcSpawner::new(&protos, &names, NpcVids::default());
        let mut state = a_state();
        let entries = [a_smith_at(10_100, 20_100), a_smith_at(10_200, 20_200)];
        state
            .spawn_npcs(&mut spawner, 2, &an_npc_map(1), &entries)
            .expect("VIDs to spare");
        state
            .spawn_npcs(&mut spawner, 2, &an_npc_map(41), &entries[..1])
            .expect("VIDs to spare");

        let first = state.npcs_on(2, 1);
        assert_eq!(first.npcs.len(), 2);
        assert_eq!(first.npcs[0].name, b"Smith");
        assert_eq!(first.npcs[0].empire, 1, "map 1 is Shinsoo's");
        assert_eq!(state.npcs_on(2, 41).npcs.len(), 1);
        assert_eq!(state.npcs_on(2, 41).npcs[0].empire, 3, "map 41 is Jinno's");
        assert!(state.npcs_on(1, 1).npcs.is_empty(), "another Channel");
        assert!(state.npcs_on(2, 2).npcs.is_empty(), "a map with no regen");
        assert_eq!(spawner.report().spawned, 3);

        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::NpcsOn {
            channel: 2,
            map: 1,
            reply,
        });
        let shared = answer.blocking_recv().expect("the game thread answered");
        assert!(
            Arc::ptr_eq(&shared, &first),
            "the table is shared, not copied"
        );
    }

    /// A player's body on Channel 1's map 41 at `(x, y)`.
    fn stand_seven(state: &mut GameState, (x, y): (i32, i32)) {
        let card = crate::loading_phase::PcCard {
            name: "Shaman".to_owned(),
            job: 3,
            empire: 1,
            level: 10,
            conqueror_level: 0,
            language: 1,
            pk_mode: crate::loading_phase::PK_MODE_PROTECT,
        };
        let place = crate::game_loop_messages::EnterPlace {
            channel: 1,
            map: 41,
            x,
            y,
            z: 0,
        };
        let _own = state.place_body(Vid::new(7), place, card);
    }

    /// A drop and a pick-up are judged where the body stands, not at the point the descriptor
    /// last stored; the store's place gives only the Channel and the map.
    #[test]
    fn a_ground_place_carries_the_bodys_live_point() {
        let vnum = a_plain_vnum();
        let mut state = a_holder(&[(inventory(3), a_plain_item(11, vnum))], true);
        stand_seven(&mut state, (500, 700));
        let stored = a_place(41, 90_000, 90_000);
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::DropItem {
            vid: Vid::new(7),
            at: inventory(3),
            count: 0,
            place: stored,
            mover: a_mover(),
            reply,
        });
        let _dropped = answer.blocking_recv().unwrap().expect("the drop happened");
        assert_eq!(lying_on(&state, (1, 41)), vec![(1, 500, 700)]);
        assert_eq!(state.ground[&1].ground.item.vnum, vnum);
        assert_eq!(
            state.pickup_item(Vid::new(7), 1, stored, a_mover()),
            Err(MoveItemRefused::Refused(MoveRefused::TooFar)),
            "the stored point is out of reach"
        );
        let (reply, answer) = tokio::sync::oneshot::channel();
        state.apply(GameCommand::PickupItem {
            vid: Vid::new(7),
            ground: 1,
            place: stored,
            mover: a_mover(),
            reply,
        });
        let picked = answer.blocking_recv().unwrap();
        assert!(picked.is_ok(), "{picked:?}");
        assert_eq!(held_at(&state, inventory(0)), Lookup::Occupied(11));
    }

    fn an_npc(vid: u32, x: i32, y: i32) -> world::npc::Npc {
        world::npc::Npc {
            vid,
            vnum: 20_016,
            race: 20_016,
            char_type: 1,
            on_click: 1,
            x,
            y,
            z: 0,
            rotation: 0,
            empire: 1,
            moving_speed: 0,
            attack_speed: 0,
            name: Vec::new(),
        }
    }

    fn standing(npcs: Vec<world::npc::Npc>) -> MapNpcs {
        MapNpcs {
            npcs,
            positions: Vec::new(),
        }
    }

    /// Boot shows every NPC a sectree holds in the map's view at its point, and the shows
    /// reach no client; an NPC outside every sectree still stands in the table.
    #[test]
    fn npc_spawn_indexes_every_npc_and_delivers_nothing() {
        let mut state = a_state();
        let (outbox, mut inbox) = a_live_outbox();
        state
            .enter_world_with_items(Vid::new(7), 7, "Shaman", &[], outbox)
            .expect("the character is admitted");
        let npcs = vec![
            an_npc(901, 10_100, 20_100),
            an_npc(902, 10_300, 20_100),
            an_npc(903, 13_000, 20_100),
        ];
        state.stand_npcs(1, &an_npc_map(41), standing(npcs));
        let index = &state.maps[&(1, 41)];
        for vid in [901, 902] {
            let spot = index.spot(view::EntityKey::Npc(vid)).expect("indexed");
            assert!(spot.tree.is_some(), "{vid}");
            assert_eq!(
                state.npc_of[&vid],
                (1, 41, usize::try_from(vid - 901).unwrap())
            );
        }
        assert_eq!((index.spot(view::EntityKey::Npc(902)).unwrap().x), 10_300);
        assert!(index.sees(view::EntityKey::Npc(901), view::EntityKey::Npc(902)));
        assert!(
            index.spot(view::EntityKey::Npc(903)).is_none(),
            "no sectree holds it"
        );
        assert_eq!(state.npc_of[&903], (1, 41, 2));
        assert_eq!(state.npcs_on(1, 41).npcs.len(), 3);
        assert!(inbox.try_recv().is_err(), "nothing was delivered");
    }

    /// Standing a map up again takes its old NPCs out of the view and the VID table, and the
    /// new ones in.
    #[test]
    fn a_replaced_maps_old_npcs_are_unindexed() {
        let mut state = a_state();
        let first = vec![an_npc(901, 10_100, 20_100), an_npc(902, 10_300, 20_100)];
        state.stand_npcs(1, &an_npc_map(41), standing(first));
        state.stand_npcs(
            1,
            &an_npc_map(41),
            standing(vec![an_npc(911, 10_200, 20_100)]),
        );
        let index = &state.maps[&(1, 41)];
        for old in [901, 902] {
            assert!(index.spot(view::EntityKey::Npc(old)).is_none(), "{old}");
            assert!(!state.npc_of.contains_key(&old), "{old}");
            assert!(!index.sees(view::EntityKey::Npc(911), view::EntityKey::Npc(old)));
        }
        assert!(index.spot(view::EntityKey::Npc(911)).is_some());
        assert_eq!(state.npc_of[&911], (1, 41, 0));
        assert_eq!(state.npcs_on(1, 41).npcs.len(), 1);
    }

    /// `VIEW_RANGE + VIEW_BONUS_RANGE` (`G/entity_view.cpp:94`) is taken in 64 bits, so the
    /// largest configured range does not wrap (D4).
    #[test]
    fn the_radius_is_view_range_plus_500_in_64_bits() {
        assert_eq!(view_radius_of(i32::MAX), 2_147_484_147);
        assert_eq!(view_radius_of(DEFAULT_VIEW_RANGE), 5_500);
        assert_eq!(view_radius_of(0), 500);
        assert_eq!(a_state().view_radius, 5_500, "legacy's default range");
        assert_eq!(a_state().with_view_range(10_000).view_radius, 10_500);
    }
}
