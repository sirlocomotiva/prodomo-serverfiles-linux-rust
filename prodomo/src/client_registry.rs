//! The in-process client registry that talking chat and movement broadcasts need.
//!
//! Legacy has no such type because it does not need one: `DESC_MANAGER::GetClientSet()`
//! (`server/server/game/desc_manager.cpp:310-313`) is the process-wide set of
//! descriptors, and `FEmpireChatPacket` walks it directly. The Rewrite runs every
//! Channel in one process (ADR-0002), so that set has to be built explicitly.
//!
//! # Why one registry keyed by Channel
//!
//! Legacy's talking-chat scope is *the whole map*, and the filter is
//! `d->GetCharacter()->GetMapIndex() == iMapIndex` (`server/server/game/input_main.cpp:691-692`)
//! applied to every descriptor in the process. Legacy only gets away with walking the
//! process set because one process hosted one Channel. Keying this registry by Channel
//! number and then by map reproduces that scope exactly: a client on Channel 1 never
//! receives a line from Channel 2, even when both host the same map index.
//!
//! # The sender is a recipient
//!
//! `FEmpireChatPacket` has no self-exclusion, so the speaker receives their own line and
//! [`ChannelClients::broadcast_on_map`] delivers to every member of the map. Movement is
//! the opposite: `PacketAround` passes the moving character as `except`
//! (`input_main.cpp:1891`), so [`ChannelClients::broadcast_excluding`] is what a move needs.

#![warn(missing_docs)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::sync_position::SyncPositionVictimKind;
use tokio::sync::mpsc;

/// One client as the registry knows it. The socket lives in the descriptor task; this is
/// only the addressing a broadcast needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEntry {
    /// The Channel the client logged in through (`g_bChannel`).
    pub channel: u8,
    /// The character's map index (`GetMapIndex()`), which is the talking-chat filter.
    /// The atlas lookup that produced it is signed, because legacy's `GetMapIndex()`
    /// returns `int` and a position off every map is a negative index.
    pub map: i32,
    /// The character Name, for the log line a refusal produces.
    pub name: String,
    /// The character's VID, for log lines and for the world work that comes next.
    pub vid: u32,
}

#[derive(Debug)]
struct Member {
    id: u64,
    entry: ClientEntry,
    outbox: ClientOutbox,
}

/// A sender the game thread writes records to for one client.
///
/// The world has no socket, and giving it one would put a file descriptor on a
/// thread that must not block (ADR-0002). So the world holds this and the descriptor
/// task drains it, which is the same arrangement the broadcast path already uses
/// and needs no second delivery mechanism.
///
/// It is a separate type from a raw [`mpsc::UnboundedSender`] so that the world
/// cannot be handed a **receiver**, and so a record addressed to a client that has
/// gone is a reported `false` rather than a silent drop. `send` is unbounded for
/// the same reason the broadcast outbox is: the game thread must never park on a
/// slow client, and a dropped client is detected by the closed channel.
#[derive(Clone, Debug)]
pub struct ClientOutbox {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl ClientOutbox {
    /// Wraps a sender half.
    #[must_use]
    pub const fn new(tx: mpsc::UnboundedSender<Vec<u8>>) -> Self {
        Self { tx }
    }

    /// Hands one record to the client.
    ///
    /// Returns `false` when the descriptor is gone, which is the honest answer for a
    /// world write to a departed client: the record was not delivered and cannot be.
    /// A caller that needs to know the difference between "sent" and "sent and
    /// buffered" cannot have it, and does not need to: the socket write is the
    /// descriptor's, and it is the descriptor that will fail.
    pub fn send(&self, record: Vec<u8>) -> bool {
        self.tx.send(record).is_ok()
    }

    /// Whether the descriptor is still draining this queue.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// The per-Channel, per-map client set.
///
/// One registry exists per process, held in an `Arc` inside `ConnectionContext`, and a
/// lease hands a task its own `Arc` clone. That is what makes deregistration on drop work
/// without a self-referential borrow: the registry outlives every descriptor task.
#[derive(Debug, Default)]
pub struct ChannelClients {
    /// Keyed by Channel number, because legacy's scope is one process per Channel.
    inner: Mutex<HashMap<u8, Vec<Member>>>,
    next_id: AtomicU64,
}

impl ChannelClients {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Add a client to its Channel and map, and take the matching outbox.
    ///
    /// The returned [`Lease`] removes the client when it drops, which is what
    /// `DESC_MANAGER` does when a descriptor is destroyed.
    pub fn join(self: &Arc<Self>, entry: ClientEntry) -> Lease {
        let (tx, inbox) = mpsc::unbounded_channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let channel = entry.channel;
        if let Ok(mut channels) = self.inner.lock() {
            channels.entry(channel).or_default().push(Member {
                id,
                entry,
                outbox: ClientOutbox::new(tx.clone()),
            });
        }
        Lease {
            id,
            channel,
            outbox: ClientOutbox::new(tx),
            registry: Arc::clone(self),
            inbox: Some(inbox),
        }
    }

    /// The clients on one map of one Channel, in join order.
    ///
    /// The lock is never held across an `await`; the caller copies what it needs.
    #[must_use]
    pub fn on_map(&self, channel: u8, map: i32) -> Vec<ClientEntry> {
        let Ok(channels) = self.inner.lock() else {
            return Vec::new();
        };
        channels
            .get(&channel)
            .map(|members| {
                members
                    .iter()
                    .filter(|member| member.entry.map == map)
                    .map(|member| member.entry.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The lease identifiers on one map of one Channel.
    ///
    /// The position table needs the lease, not the client entry, because that is the key it
    /// tracks a position under.
    #[must_use]
    pub fn ids_on_map(&self, channel: u8, map: i32) -> Vec<u64> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&channel)
            .map(|members| {
                members
                    .iter()
                    .filter(|member| member.entry.map == map)
                    .map(|member| member.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many clients are on one map of one Channel.
    #[must_use]
    pub fn count_on_map(&self, channel: u8, map: i32) -> usize {
        self.on_map(channel, map).len()
    }

    /// Deliver one record to every client on the map, **including** the sender.
    ///
    /// This is the talking-chat scope.
    ///
    /// A send to a member whose receiver is gone is skipped rather than failing the
    /// broadcast. The public API cannot produce that state, because dropping a lease is
    /// the only way to close its receiver and dropping also deregisters, so the branch
    /// is defensive rather than covered by a test.
    pub fn broadcast_on_map(&self, channel: u8, map: i32, record: &[u8]) -> usize {
        self.deliver(channel, Some(map), None, record)
    }

    /// Deliver one record to every client on the map except one lease.
    ///
    /// This is the `PacketAround` scope: `except` is the moving character
    /// (`input_main.cpp:1891`), and the moving client must not receive an echo.
    pub fn broadcast_excluding(&self, channel: u8, map: i32, except: u64, record: &[u8]) -> usize {
        self.deliver(channel, Some(map), Some(except), record)
    }

    /// Deliver one record to every client on the Channel, whatever its map.
    ///
    /// This is the shout scope: `SendShout` walks the whole process client set
    /// (`server/server/game/input_p2p.cpp:237-241`).
    pub fn broadcast_on_channel(&self, channel: u8, record: &[u8]) -> usize {
        self.deliver(channel, None, None, record)
    }

    fn deliver(&self, channel: u8, map: Option<i32>, except: Option<u64>, record: &[u8]) -> usize {
        let Ok(channels) = self.inner.lock() else {
            return 0;
        };
        let Some(members) = channels.get(&channel) else {
            return 0;
        };
        let mut sent = 0;
        for member in members {
            if let Some(map) = map {
                if member.entry.map != map {
                    continue;
                }
            }
            if except == Some(member.id) {
                continue;
            }
            if member.outbox.send(record.to_vec()) {
                sent += 1;
            }
        }
        sent
    }

    /// Remove one member. A missing entry is not an error: a double drop must not panic.
    fn leave(&self, channel: u8, id: u64) {
        if let Ok(mut channels) = self.inner.lock() {
            if let Some(members) = channels.get_mut(&channel) {
                members.retain(|member| member.id != id);
                if members.is_empty() {
                    channels.remove(&channel);
                }
            }
        }
    }
}

/// The registry's claim on one client. The outbox half is what the descriptor task
/// drains; dropping the lease is the deregistration.
#[derive(Debug)]
pub struct Lease {
    id: u64,
    channel: u8,
    outbox: ClientOutbox,
    registry: Arc<ChannelClients>,
    inbox: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl Lease {
    /// Take the next record addressed to this client, if one is waiting.
    pub fn try_next(&mut self) -> Option<Vec<u8>> {
        self.inbox.as_mut().and_then(|inbox| inbox.try_recv().ok())
    }

    /// Wait for the next record addressed to this client.
    ///
    /// This resolves to `None` once every sender is gone, which is also how a descriptor
    /// task learns to stop listening for broadcasts.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        match self.inbox.as_mut() {
            Some(inbox) => inbox.recv().await,
            None => std::future::pending().await,
        }
    }

    /// Lend the queue to a caller's `select!` arm and take it back on the next call.
    ///
    /// A descriptor task holds the lease inside its own state, and the analyzer may replace
    /// that state, so the queue cannot be borrowed across an await that might run the
    /// analyzer. Moving it out for the duration of one select and back on the next turn is
    /// the same discipline as the two-phase borrow of a mutex guard around a `select!`.
    ///
    /// Returns `None` when the queue is already lent out, so a second call before the next
    /// take-back is a no-op rather than a silent loss.
    pub fn take_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<Vec<u8>>> {
        self.inbox.take()
    }

    /// Take the queue back after a [`Lease::take_receiver`] arm.
    pub fn put_receiver(&mut self, inbox: mpsc::UnboundedReceiver<Vec<u8>>) {
        if self.inbox.is_none() {
            self.inbox = Some(inbox);
        } else {
            // The lease already holds a queue, so the new one is dropped. That ends the
            // borrow rather than leaving a live queue nobody reads, and a member whose
            // receiver is gone stops being a recipient.
            drop(inbox);
        }
    }

    /// The lease's own identifier, which `broadcast_excluding` takes as `except`.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// A sender for this client that the game thread can write through.
    ///
    /// This is what lets the world deliver a record to a real descriptor. The
    /// descriptor keeps draining the same queue it already drains for broadcasts, so
    /// a world write and a broadcast write are indistinguishable downstream and
    /// cannot arrive out of order relative to each other.
    #[must_use]
    pub fn outbox(&self) -> ClientOutbox {
        self.outbox.clone()
    }

    /// The Channel this lease joined.
    #[must_use]
    pub const fn channel(&self) -> u8 {
        self.channel
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.registry.leave(self.channel, self.id);
    }
}

/// One tracked actor's position, which is what a sync-position claim is checked against.
///
/// The Rewrite has no world, so the client set is the world stand-in: a character's
/// position is held here rather than in a sector, and `input_main.cpp`'s victim lookup
/// (`FindCharacter`) becomes a lookup in this table. The table is keyed by lease, because
/// the lease is what a descriptor owns for as long as it is in the game phase, and it is
/// released on drop like everything else on a descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracked {
    /// The character's VID, which is what a claim names.
    pub vid: u32,
    /// `CHARTYPE_*`, which decides whether the legacy switch skips the claim.
    pub kind: SyncPositionVictimKind,
    /// The authoritative x.
    pub x: i32,
    /// The authoritative y.
    pub y: i32,
    /// `m_dwLastSync`, which the 100 ms interval check reads.
    pub last_sync: Option<std::time::Duration>,
    /// `m_pkChrSyncOwner`, the character allowed to move this one, and `m_fSyncTime`, the
    /// stamp that lets a different character take the ownership over once it is old
    /// enough. `ENABLE_FLY_FIX` selects the 100-unit form of `IsSyncOwner`.
    pub sync_owner: Option<(u32, std::time::Duration)>,
}

/// Every position in the process, grouped by Channel and map so a claim can only name a
/// character on the claimer's own map.
///
/// `LEGACY` finds a victim through the process character map, which holds every Channel's
/// characters, so the Rewrite keeps one table for the whole process and filters by Channel
/// and map when it answers, exactly as the legacy lookup does.
#[derive(Debug, Default)]
pub struct PositionTable {
    entries: std::sync::Mutex<std::collections::HashMap<u64, Tracked>>,
}

impl PositionTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start tracking a character, replacing any earlier entry for the same lease.
    pub fn track(&self, id: u64, kind: SyncPositionVictimKind, vid: u32, x: i32, y: i32) {
        self.lock().insert(
            id,
            Tracked {
                vid,
                kind,
                x,
                y,
                last_sync: None,
                // `CHARACTER::CHARACTER` seeds `m_fSyncTime` with `get_float_time() - 3`,
                // so a character that has never been claimed is already claimable.
                sync_owner: None,
            },
        );
    }

    /// One tracked character.
    #[must_use]
    pub fn get(&self, id: u64) -> Option<Tracked> {
        self.lock().get(&id).copied()
    }

    /// The tracked character on one map of one Channel with a given VID, if any.
    ///
    /// The legacy lookup is by VID alone and then checks the map, so a VID is unique in the
    /// process and this returns the first match. A duplicate is a Defect of the Rewrite's
    /// own making, so the first match is the deterministic answer.
    #[must_use]
    pub fn find_on_map(
        &self,
        registry: &ChannelClients,
        channel: u8,
        map: i32,
        vid: u32,
    ) -> Option<(u64, Tracked)> {
        let entries = self.lock();
        for member in registry.ids_on_map(channel, map) {
            if let Some(tracked) = entries.get(&member) {
                if tracked.vid == vid {
                    return Some((member, *tracked));
                }
            }
        }
        None
    }

    /// Move a character and stamp its last-sync time.
    pub fn sync(&self, id: u64, x: i32, y: i32, now: std::time::Duration) {
        if let Some(entry) = self.lock().get_mut(&id) {
            entry.x = x;
            entry.y = y;
            entry.last_sync = Some(now);
        }
    }

    /// The last-sync stamp alone, which `SetSyncOwner` resets.
    #[must_use]
    pub fn last_sync(&self, id: u64) -> Option<std::time::Duration> {
        self.lock().get(&id).and_then(|entry| entry.last_sync)
    }

    /// Clear a character's last-sync stamp, which `SetSyncOwner` does.
    pub fn forget_sync(&self, id: u64) {
        if let Some(entry) = self.lock().get_mut(&id) {
            entry.last_sync = None;
        }
    }

    /// The `(owner, claim stamp)` pair, which `IsSyncOwner` reads.
    #[must_use]
    pub fn sync_owner(&self, id: u64) -> Option<(u32, std::time::Duration)> {
        self.lock().get(&id).and_then(|entry| entry.sync_owner)
    }

    /// Set the sync owner and the claim stamp, which a successful `SetSyncOwner` does.
    pub fn set_sync_owner(&self, id: u64, owner: u32, now: std::time::Duration) {
        if let Some(entry) = self.lock().get_mut(&id) {
            entry.sync_owner = Some((owner, now));
        }
    }

    /// Drop a character's sync owner, which `SetSyncOwner(NULL)` does.
    pub fn release_sync_owner(&self, id: u64) {
        if let Some(entry) = self.lock().get_mut(&id) {
            entry.sync_owner = None;
        }
    }

    /// Forget a character. The lease drop calls this so a disconnected client cannot be
    /// claimed as a sync victim.
    pub fn forget(&self, id: u64) {
        self.lock().remove(&id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, Tracked>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(channel: u8, map: i32, name: &str) -> ClientEntry {
        ClientEntry {
            channel,
            map,
            name: name.to_string(),
            vid: u32::try_from(name.len()).expect("a short Name fits a VID"),
        }
    }

    fn drained(lease: &mut Lease) -> Vec<String> {
        std::iter::from_fn(|| lease.try_next())
            .map(|bytes| String::from_utf8(bytes).expect("text records"))
            .collect()
    }

    #[test]
    fn a_map_broadcast_reaches_the_sender_too() {
        let registry = Arc::new(ChannelClients::new());
        let mut ayla = registry.join(entry(1, 100, "Ayla"));
        let mut brann = registry.join(entry(1, 100, "Brann"));

        assert_eq!(registry.broadcast_on_map(1, 100, b"line"), 2);
        assert_eq!(drained(&mut ayla), vec!["line".to_string()]);
        assert_eq!(drained(&mut brann), vec!["line".to_string()]);
    }

    #[test]
    fn a_map_broadcast_skips_every_other_map() {
        let registry = Arc::new(ChannelClients::new());
        let mut here = registry.join(entry(1, 100, "Here"));
        let mut elsewhere = registry.join(entry(1, 101, "Elsewhere"));

        assert_eq!(registry.broadcast_on_map(1, 100, b"line"), 1);
        assert_eq!(drained(&mut here), vec!["line".to_string()]);
        assert!(drained(&mut elsewhere).is_empty());
    }

    #[test]
    fn a_map_broadcast_skips_every_other_channel() {
        let registry = Arc::new(ChannelClients::new());
        let mut one = registry.join(entry(1, 100, "One"));
        let mut two = registry.join(entry(2, 100, "Two"));

        assert_eq!(registry.broadcast_on_map(1, 100, b"line"), 1);
        assert_eq!(drained(&mut one), vec!["line".to_string()]);
        assert!(drained(&mut two).is_empty());
    }

    #[test]
    fn excluding_a_lease_leaves_the_mover_without_an_echo() {
        let registry = Arc::new(ChannelClients::new());
        let mut mover = registry.join(entry(1, 100, "Mover"));
        let mut watcher = registry.join(entry(1, 100, "Watcher"));

        assert_eq!(registry.broadcast_excluding(1, 100, mover.id(), b"move"), 1,);
        assert!(drained(&mut mover).is_empty(), "a move never echoes");
        assert_eq!(drained(&mut watcher), vec!["move".to_string()]);
    }

    #[test]
    fn a_channel_broadcast_reaches_every_map() {
        let registry = Arc::new(ChannelClients::new());
        let mut here = registry.join(entry(1, 100, "Here"));
        let mut elsewhere = registry.join(entry(1, 200, "Elsewhere"));
        let mut other = registry.join(entry(2, 100, "Other"));

        assert_eq!(registry.broadcast_on_channel(1, b"shout"), 2);
        assert_eq!(drained(&mut here), vec!["shout".to_string()]);
        assert_eq!(drained(&mut elsewhere), vec!["shout".to_string()]);
        assert!(drained(&mut other).is_empty());
    }

    #[test]
    fn dropping_a_lease_deregisters_the_client() {
        let registry = Arc::new(ChannelClients::new());
        let watcher = registry.join(entry(1, 100, "Watcher"));
        assert_eq!(registry.count_on_map(1, 100), 1);
        {
            let _mover = registry.join(entry(1, 100, "Mover"));
            assert_eq!(registry.count_on_map(1, 100), 2);
        }
        assert_eq!(registry.count_on_map(1, 100), 1, "the lease deregistered");
        assert_eq!(registry.broadcast_on_map(1, 100, b"line"), 1);
        drop(watcher);
        assert_eq!(registry.count_on_map(1, 100), 0);
    }

    #[test]
    fn the_registry_reports_an_unknown_channel_as_empty() {
        let registry = ChannelClients::new();
        assert_eq!(registry.count_on_map(9, 100), 0);
        assert_eq!(registry.broadcast_on_map(9, 100, b"line"), 0);
        assert_eq!(registry.broadcast_on_channel(9, b"line"), 0);
        assert!(registry.on_map(9, 100).is_empty());
    }

    #[test]
    fn every_lease_has_a_distinct_identifier_and_knows_its_channel() {
        let registry = Arc::new(ChannelClients::new());
        let first = registry.join(entry(1, 100, "A"));
        let second = registry.join(entry(3, 100, "B"));
        assert_ne!(first.id(), second.id());
        assert_eq!(first.channel(), 1);
        assert_eq!(second.channel(), 3);
    }

    #[test]
    fn a_fresh_lease_has_nothing_queued() {
        let registry = Arc::new(ChannelClients::new());
        let mut lease = registry.join(entry(1, 100, "A"));
        assert!(lease.try_next().is_none());
    }

    #[test]
    fn forgetting_a_character_removes_its_position_and_its_ownership() {
        let registry = Arc::new(ChannelClients::new());
        let lease = registry.join(entry(1, 0, "Victim"));
        let table = PositionTable::new();
        table.track(
            lease.id(),
            SyncPositionVictimKind::Player,
            3_000_007,
            10,
            20,
        );
        table.set_sync_owner(lease.id(), 3_000_009, std::time::Duration::from_millis(40));
        table.sync(lease.id(), 11, 21, std::time::Duration::from_millis(500));
        let found = table
            .find_on_map(&registry, 1, 0, 3_000_007)
            .expect("tracked");
        assert_eq!(found.0, lease.id());
        assert_eq!(found.1.vid, 3_000_007);
        assert_eq!(
            table.sync_owner(lease.id()),
            Some((3_000_009, std::time::Duration::from_millis(40)))
        );
        assert_eq!(
            table.last_sync(lease.id()),
            Some(std::time::Duration::from_millis(500))
        );

        // What `handle_connection` does as the descriptor ends: a disconnect takes the
        // character out of the world, so `FindCharacter` must find nobody afterwards.
        table.forget(lease.id());

        assert_eq!(table.find_on_map(&registry, 1, 0, 3_000_007), None);
        assert_eq!(table.sync_owner(lease.id()), None);
        assert_eq!(table.last_sync(lease.id()), None);
    }

    #[test]
    fn a_forgotten_character_is_not_reclaimable() {
        let registry = Arc::new(ChannelClients::new());
        let lease = registry.join(entry(1, 0, "Victim"));
        let table = PositionTable::new();
        table.track(
            lease.id(),
            SyncPositionVictimKind::Player,
            3_000_007,
            10,
            20,
        );
        table.forget(lease.id());
        assert_eq!(table.find_on_map(&registry, 1, 0, 3_000_007), None);

        // A late answer for a closed descriptor must not put a phantom character back in
        // the world: `set_sync_owner` and `sync` write through `get_mut`, so a row that is
        // gone stays gone.
        table.set_sync_owner(lease.id(), 3_000_009, std::time::Duration::from_millis(40));
        table.sync(lease.id(), 11, 21, std::time::Duration::from_millis(500));
        assert_eq!(table.find_on_map(&registry, 1, 0, 3_000_007), None);
        assert_eq!(table.sync_owner(lease.id()), None);
    }
}
