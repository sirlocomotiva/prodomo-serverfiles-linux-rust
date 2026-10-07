//! The in-process client registry: the process-wide descriptor set, the outbox the world
//! writes a client's records into, and the orders it gives a descriptor.
//!
//! Legacy has no such type because it does not need one: `DESC_MANAGER::GetClientSet()`
//! (`server/server/game/desc_manager.cpp:310-313`) is the process-wide set of
//! descriptors. The Rewrite runs every Channel in one process (ADR-0002), so that set has to
//! be built explicitly.
//!
//! # Why one registry keyed by Channel
//!
//! A map index names a map on one Channel only. Keying this registry by Channel number and
//! then by map keeps a client on Channel 1 from receiving a record about Channel 2, even when
//! both host the same map index.
//!
//! # What reaches a client through here
//!
//! The world sends the records about bodies (moves, poses, talking chat, the view) itself,
//! through each client's [`ClientOutbox`]. What still walks this registry is the ground-item
//! record a map's clients are sent ([`ChannelClients::broadcast_on_map`], V8), the client a
//! warp NPC orders ([`ChannelClients::members_on_map`]), and the shout.
//!
//! # The shout crosses every Channel
//!
//! A shout is the one line that leaves its process in legacy: `GG_SHOUT` carries it to every
//! other core over P2P, and each core's `FuncShout` sends it to its own clients
//! (`input_main.cpp:903-911`, `input_p2p.cpp:215-240`). Every Channel is in this registry, so
//! [`ChannelClients::deliver_everywhere`] is that whole walk, and the in-process registry is
//! the bus ADR-0002 puts in P2P's place. `FuncShout` sends each client a line of its own,
//! because `ChatPacket` looks the text up in the client's language and writes the client's
//! empire, which is why a [`ClientEntry`] carries both.

#![warn(missing_docs)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    /// The character's empire: the `bEmpire` of every line `ChatPacket` sends the client,
    /// and what `FuncShout` compares with the shouter's.
    pub empire: u8,
    /// The descriptor's language, which `ChatPacket` looks every line up in.
    pub language: u8,
}

/// What the game thread asks a descriptor to do to its own character.
///
/// The world owns no descriptor, so a step that ends in the descriptor's own state is sent as
/// an order, and the descriptor runs it on its next turn. A warp NPC's `WarpSet` saves the
/// character and sends `GC_WARP`. A goto NPC's `Show` moves the body, which the world owns, so
/// it is no order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientOrder {
    /// `WarpSet(x, y)` from a warp NPC (`FuncCheckWarp`, `G/char.cpp:7967-7968`).
    Warp {
        /// The target x.
        x: i32,
        /// The target y.
        y: i32,
    },
}

#[derive(Debug)]
struct Member {
    id: u64,
    entry: ClientEntry,
    outbox: ClientOutbox,
    orders: mpsc::UnboundedSender<ClientOrder>,
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
        let (orders, ordered) = mpsc::unbounded_channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let channel = entry.channel;
        if let Ok(mut channels) = self.inner.lock() {
            channels.entry(channel).or_default().push(Member {
                id,
                entry,
                outbox: ClientOutbox::new(tx.clone()),
                orders,
            });
        }
        Lease {
            id,
            channel,
            outbox: ClientOutbox::new(tx),
            registry: Arc::clone(self),
            inbox: Some(inbox),
            orders: Some(ordered),
        }
    }

    /// The clients on one map of one Channel with their lease identifiers, in join order.
    ///
    /// A warp NPC needs both: the entry for the empire and the language of the line it sends,
    /// and the lease for the order.
    #[must_use]
    pub fn members_on_map(&self, channel: u8, map: i32) -> Vec<(u64, ClientEntry)> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&channel)
            .map(|members| {
                members
                    .iter()
                    .filter(|member| member.entry.map == map)
                    .map(|member| (member.id, member.entry.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Hand one order to the descriptor that holds lease `id` on `channel`.
    ///
    /// Returns `false` when no such member is left or its descriptor has stopped taking orders.
    pub fn order(&self, channel: u8, id: u64, order: ClientOrder) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&channel)
            .and_then(|members| members.iter().find(|member| member.id == id))
            .is_some_and(|member| member.orders.send(order).is_ok())
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

    /// How many clients are on one map of one Channel.
    #[must_use]
    pub fn count_on_map(&self, channel: u8, map: i32) -> usize {
        self.on_map(channel, map).len()
    }

    /// Deliver one record to every client on the map, **including** the sender.
    ///
    /// A send to a member whose receiver is gone is skipped rather than failing the
    /// broadcast. The public API cannot produce that state, because dropping a lease is
    /// the only way to close its receiver and dropping also deregisters, so the branch
    /// is defensive rather than covered by a test.
    pub fn broadcast_on_map(&self, channel: u8, map: i32, record: &[u8]) -> usize {
        let Ok(channels) = self.inner.lock() else {
            return 0;
        };
        let Some(members) = channels.get(&channel) else {
            return 0;
        };
        let mut sent = 0;
        for member in members.iter().filter(|member| member.entry.map == map) {
            if member.outbox.send(record.to_vec()) {
                sent += 1;
            }
        }
        sent
    }

    /// Deliver a line built for each client to every client on every Channel, and answer how
    /// many were sent one.
    ///
    /// This is the shout scope: `FuncShout` on every core, over every descriptor with a
    /// character (`input_p2p.cpp:215-240`). `line` builds the client's own line, or answers
    /// `None` for a client that must not hear it.
    pub fn deliver_everywhere(&self, line: impl Fn(&ClientEntry) -> Option<Vec<u8>>) -> usize {
        let Ok(channels) = self.inner.lock() else {
            return 0;
        };
        let mut sent = 0;
        for member in channels.values().flatten() {
            if let Some(record) = line(&member.entry) {
                if member.outbox.send(record) {
                    sent += 1;
                }
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
    orders: Option<mpsc::UnboundedReceiver<ClientOrder>>,
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

    /// Lend the order queue to a caller's `select!` arm, as [`Lease::take_receiver`] lends the
    /// record queue. Returns `None` when it is already lent out.
    pub fn take_orders(&mut self) -> Option<mpsc::UnboundedReceiver<ClientOrder>> {
        self.orders.take()
    }

    /// Take the order queue back after a [`Lease::take_orders`] arm. A lease that already holds
    /// one drops the other, as [`Lease::put_receiver`] does.
    pub fn put_orders(&mut self, orders: mpsc::UnboundedReceiver<ClientOrder>) {
        if self.orders.is_none() {
            self.orders = Some(orders);
        }
    }

    /// The lease's own identifier, which [`ChannelClients::order`] takes to name its client.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(channel: u8, map: i32, name: &str) -> ClientEntry {
        ClientEntry {
            channel,
            map,
            name: name.to_string(),
            vid: u32::try_from(name.len()).expect("a short Name fits a VID"),
            empire: 1,
            language: 1,
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
    fn a_shout_reaches_every_channel_and_map_with_a_line_built_for_each_client() {
        let registry = Arc::new(ChannelClients::new());
        let mut here = registry.join(entry(1, 100, "Here"));
        let mut elsewhere = registry.join(ClientEntry {
            empire: 2,
            language: 5,
            ..entry(1, 200, "Elsewhere")
        });
        let mut other = registry.join(ClientEntry {
            empire: 3,
            ..entry(99, 72, "Other")
        });
        let mut skipped = registry.join(entry(2, 100, "Skipped"));

        let sent = registry.deliver_everywhere(|client| {
            (client.name != "Skipped").then(|| {
                format!("{} {} {}", client.name, client.empire, client.language).into_bytes()
            })
        });

        assert_eq!(sent, 3, "a client the line skips is not counted");
        assert_eq!(drained(&mut here), vec!["Here 1 1".to_string()]);
        assert_eq!(drained(&mut elsewhere), vec!["Elsewhere 2 5".to_string()]);
        assert_eq!(drained(&mut other), vec!["Other 3 1".to_string()]);
        assert!(drained(&mut skipped).is_empty());
    }

    #[test]
    fn a_shout_does_not_count_a_client_whose_queue_is_gone() {
        let registry = Arc::new(ChannelClients::new());
        let mut here = registry.join(entry(1, 100, "Here"));
        let mut gone = registry.join(entry(2, 100, "Gone"));
        drop(gone.take_receiver());

        let sent = registry.deliver_everywhere(|_| Some(b"line".to_vec()));

        assert_eq!(sent, 1, "a line nobody can read was not sent");
        assert_eq!(drained(&mut here), vec!["line".to_string()]);
    }

    #[test]
    fn the_members_on_a_map_come_in_join_order_with_their_leases() {
        let registry = Arc::new(ChannelClients::new());
        let first = registry.join(entry(1, 100, "First"));
        let _elsewhere = registry.join(entry(1, 200, "Elsewhere"));
        let _other = registry.join(entry(2, 100, "Other"));
        let second = registry.join(ClientEntry {
            empire: 3,
            ..entry(1, 100, "Second")
        });

        let members = registry.members_on_map(1, 100);

        let seen: Vec<(u64, &str, u8)> = members
            .iter()
            .map(|(id, client)| (*id, client.name.as_str(), client.empire))
            .collect();
        assert_eq!(
            seen,
            vec![(first.id(), "First", 1), (second.id(), "Second", 3)]
        );
        assert!(registry.members_on_map(3, 100).is_empty());
    }

    #[test]
    fn an_order_reaches_only_the_lease_it_names_on_its_channel() {
        let registry = Arc::new(ChannelClients::new());
        let mut named = registry.join(entry(1, 100, "Named"));
        let mut beside = registry.join(entry(1, 100, "Beside"));
        let warp = ClientOrder::Warp {
            x: 400_200,
            y: 899_500,
        };

        assert!(registry.order(1, named.id(), warp));
        assert!(!registry.order(2, named.id(), warp), "another Channel");

        let mut orders = named.take_orders().expect("the queue is home");
        assert_eq!(orders.try_recv().ok(), Some(warp));
        assert!(orders.try_recv().is_err(), "exactly one order");
        assert!(named.take_orders().is_none(), "lent out");
        named.put_orders(orders);
        assert!(named.take_orders().is_some(), "taken back");
        let mut beside_orders = beside.take_orders().expect("the queue is home");
        assert!(beside_orders.try_recv().is_err());
    }

    #[test]
    fn an_order_to_a_departed_or_deaf_client_reports_failure() {
        let registry = Arc::new(ChannelClients::new());
        let mut deaf = registry.join(entry(1, 100, "Deaf"));
        let departed = registry.join(entry(1, 100, "Departed"));
        let departed_id = departed.id();
        drop(departed);
        drop(deaf.take_orders());
        let warp = ClientOrder::Warp {
            x: 162_500,
            y: 676_100,
        };

        assert!(!registry.order(1, departed_id, warp));
        assert!(!registry.order(1, deaf.id(), warp));
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
        assert_eq!(registry.deliver_everywhere(|_| Some(b"line".to_vec())), 0);
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
}
