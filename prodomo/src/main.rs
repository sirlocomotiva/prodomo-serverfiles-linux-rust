//! The Prodomo server binary.
//!
//! `prodomo serve` runs auth and every Channel in one process (ADR-0002):
//! - loads and validates `prodomo.toml`
//! - arms SIGTERM and SIGINT before any port opens
//! - prepares the PostgreSQL store without connecting
//! - binds the auth listener and every Channel port
//! - runs the game loop, migrates the store, and only then admits clients
//! - shuts everything down in order on a signal
//!
//! `prodomo account ...` and `prodomo gm ...` are the Operator commands
//! ([`prodomo::operator`]).

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use common::config::{load_server_config, ItemIdSpan, ServerConfig, DEFAULT_CONFIG_PATH};
use common::enums::ELocale;
use common::logging::{init_from_env, init_logging, LogConfig};
use common::vid::Vid;
use db::accounts::{find_auth_account, record_login, AccountError};
use db::players::{
    change_name, create_player, delete_player, load_character, lobby, select_empire, Created,
    PlayerDelete,
};
use db::store::{schema_version, Store, StoreConfig};
use gamedata::banword::banwords_from_dump;
use gamedata::item_proto::ItemProtos;
use gamedata::map_atlas::MapAtlas;
use gamedata::mob_names::MobNames;
use prodomo::auth_login::{
    judge_account, judge_credentials, login_from_field, password_candidate, AuthClaim, AuthRefusal,
    AuthRegistry, LoginGrant,
};
use prodomo::channel_login::{
    admit, empire_shown, judge_key, login_success, random_key, ChannelRefusal, LogonClaim,
    LogonRegistry, MapLocations,
};
use prodomo::channel_status::ChannelStatusBoard;
use prodomo::chat::{judge_chat, ChatContext, ChatEffect, ChatOutcome, ChatState};
use prodomo::client_live::{
    handshake_token, BootLiveClock, LiveClientSession, LiveClock, LiveError, LiveOutcome, LiveStep,
};
use prodomo::client_registry::{ChannelClients, ClientEntry, Lease, PositionTable};
use prodomo::client_session::ClientPhase;
use prodomo::game_loop::{spawn_game_loop, GameLoopConfig, GameLoopHandle};
use prodomo::game_loop_messages::{GameLoopController, GameLoopTerminal};
use prodomo::game_state::{world_item_id_range, GameState};
use prodomo::handshake::HandshakeServerKind;
use prodomo::item_load::plan_item_load;
use prodomo::item_move::MoveItemRefused;
use prodomo::lifecycle::{LifecycleError, PostHandshakePhase};
use prodomo::listeners::{listener_plan, ListenerRole, Listeners};
use prodomo::loading_phase::{
    enter_game_burst, judge_enter_game, judge_select, loading_burst, map_is_allowed, points,
    points_packet, public_map_index, EnterGameVerdict, Neighbourhood, SelectVerdict,
};
use prodomo::movement::{
    judge_move, judge_pose, MoveContext, MoveDisposition, MoveOutcome, MoveRefusal, PoseOutcome,
};
use prodomo::operator::{prepare, read_new_password, AccountCommand, GmCommand, OperatorCommand};
use prodomo::ready_gate::ReadyGate;
use prodomo::select_phase::{
    create_failure, create_offset, created, deleted, empire_selected, judge_create, judge_delete,
    judge_empire, judge_rename, renamed, CreateCooldown, DeleteVerdict, EmpireVerdict, NameRules,
    RenameVerdict, SelectAccount, CREATE_REFUSED, CREATE_TAKEN,
};
use prodomo::sync_position::{
    judge_sync_ownership, SyncOwnershipOutcome, SyncOwnershipState, SyncPositionActor,
    SyncPositionCloseReason, SyncPositionPorts, SyncPositionVictim,
};
use prodomo::ServerState;
use protocol::cg_account::{
    CgEnterGame, CgLoginByKey, CgPlayerCreate, CgPlayerDelete, CgPlayerSelect,
};
use protocol::cg_chat::CgChat;
use protocol::cg_inventory::{
    HEADER_CG_CHANGE_NAME, HEADER_CG_CHARACTER_CREATE, HEADER_CG_CHARACTER_DELETE,
    HEADER_CG_CHARACTER_POSITION, HEADER_CG_CHARACTER_SELECT, HEADER_CG_CHAT, HEADER_CG_EMPIRE,
    HEADER_CG_ENTERGAME, HEADER_CG_ITEM_MOVE, HEADER_CG_LOGIN2, HEADER_CG_LOGIN3, HEADER_CG_MOVE,
    HEADER_CG_STATE_CHECKER, HEADER_CG_SYNC_POSITION,
};
use protocol::cg_item_move::CgItemMove;
use protocol::cg_login::CgEmpire;
use protocol::cg_login3::CgLogin3;
use protocol::cg_move::CgMove;
use protocol::cg_name::CgChangeName;
use protocol::cg_position::CgCharacterPosition;
use protocol::cg_wire::ClientFrame;
use protocol::gc::{GcAuthSuccess, GcLoginFailure};
use protocol::gc_inventory::HEADER_GC_EMPIRE;
use protocol::gc_small::GcHeaderAndByte;
use protocol::item_pos::ItemPos;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, warn};
use world::character::MoveRequest;
use world::item::Item;

/// The command line.
#[derive(Debug, Parser)]
#[command(
    name = "prodomo",
    version,
    about = "Prodomo server: auth, every Channel, and the Operator commands"
)]
struct Cli {
    /// Path to the TOML configuration.
    #[arg(short, long, global = true, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,
    /// What to do.
    #[command(subcommand)]
    command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server until SIGTERM or SIGINT.
    Serve {
        /// Log to stdout at the level in `RUST_LOG`, into the directory in `LOG_DIR`, with colour
        /// unless `LOG_ANSI` is `false`.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Create accounts, change passwords, and add Coins or Cash.
    Account {
        /// What to do.
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Grant, revoke, and list GM authority.
    Gm {
        /// What to do.
        #[command(subcommand)]
        command: GmCommand,
    },
}

/// The first pause after the store could not be reached. Each later pause doubles, up to
/// [`STORE_RETRY_MAX`].
const STORE_RETRY_FIRST: Duration = Duration::from_secs(1);

/// The longest pause between two attempts to reach the store.
const STORE_RETRY_MAX: Duration = Duration::from_secs(30);

/// Load the configuration and start logging.
fn initialize_server(config_path: &Path, verbose: bool) -> Result<ServerConfig, Box<dyn Error>> {
    let config = load_server_config(config_path)
        .map_err(|error| format!("Failed to load config: {error}"))?;

    if verbose {
        init_from_env();
    } else {
        init_logging(&LogConfig::default());
    }

    info!("Configuration loaded from {}", config_path.display());
    info!(bind_ip = %config.bind_ip, public_ip = %config.public_ip, "Addresses");
    for channel in &config.channels {
        info!(
            channel = channel.number,
            shared = channel.is_shared(),
            ports = ?channel.ports,
            maps = channel.maps.len(),
            "Channel configured"
        );
    }
    Ok(config)
}

/// What a connection needs from the server: the descriptor clock, the ping cycle, the server
/// state, the Channel status list, the store, the auth and logon registries, and where each map
/// is served.
#[derive(Clone)]
struct ConnectionContext {
    clock: BootLiveClock,
    ping_cycle: Duration,
    /// `[game] save_event_second_cycle` as an interval, from `prodomo::save::event_period`.
    save_cycle: Duration,
    state: Arc<ServerState>,
    channels: Arc<ChannelStatusBoard>,
    store: Store,
    auth: Arc<AuthRegistry>,
    /// `[game] shutdowned`: new logins are refused.
    shutdowned: bool,
    /// `[game] block_login`.
    block_login: Arc<str>,
    /// `[game] user_limit`.
    user_limit: i32,
    /// The logins held by Channel descriptors.
    logons: Arc<LogonRegistry>,
    /// The map regions of the legacy Game data.
    atlas: Arc<MapAtlas>,
    /// The item prototypes, shared with the game thread, which the item load at character
    /// select reads for each item's size and flags.
    protos: Arc<ItemProtos>,
    /// The address clients are told to reconnect to.
    public_ip: Ipv4Addr,
    /// The maps each Channel hosts, and the Shared Channel's.
    routes: Arc<MapRoutes>,
    /// The last descriptor handle given out (legacy `DESC_MANAGER` handle count).
    handles: Arc<AtomicU32>,
    /// The Names a character may not take.
    names: Arc<NameRules>,
    /// The per-Channel client set that talking chat and movement broadcast through
    /// (legacy `DESC_MANAGER::GetClientSet`).
    clients: Arc<ChannelClients>,
    /// Every character position in the process, which stands in for the sectree the legacy
    /// `FindCharacter` and `Sync` walk.
    positions: Arc<PositionTable>,
    /// When each account last created a character.
    creates: Arc<CreateCooldown>,
    /// `[game] block_char_creation`.
    block_char_creation: bool,
    /// `[game] player_delete_level_limit` and `player_delete_level_limit_lower`.
    delete_levels: (i32, i32),
    /// The game thread that owns the world every live client is put into.
    ///
    /// Held by every descriptor because entering and leaving the world are both
    /// crossings (ADR-0002), and a descriptor that could not reach the game thread
    /// would be a descriptor whose character the world cannot see.
    game: GameLoopController,
}

impl ConnectionContext {
    /// Where a client connected to `port` of Channel `number` is told each map is served.
    fn locations(&self, number: u8, port: u16) -> MapLocations {
        let maps = self
            .routes
            .channel_maps
            .get(&number)
            .cloned()
            .unwrap_or_default();
        MapLocations::new(self.public_ip, port, maps, self.routes.shared.clone())
    }
}

/// What a descriptor holds between frames.
#[derive(Default)]
struct Held {
    /// The login held after a successful auth (legacy `ConnectAccount`).
    claim: Option<AuthClaim>,
    /// The login held after a successful Channel login (legacy `InsertLogonAccount`).
    logon: Option<LogonClaim>,
    /// The account table a Channel login filled (legacy `DESC::m_accountTable`).
    account: Option<SelectAccount>,
    /// The character the client selected, once the load answered
    /// (legacy `DESC::m_pCharacter`).
    character: Option<db::players::Character>,
    /// The channel the client logged in through, which is `g_bChannel` on the descriptor.
    ///
    /// It is held from the Channel login rather than read from the listener, because a
    /// descriptor reaches the game phase only through a Channel listener and legacy keeps the
    /// one value in the process.
    channel: u8,
    /// Where the character is and what it can do, once the load has answered.
    avatar: Option<Avatar>,
    /// The atlas index of the loaded position, which is `GetMapIndex()` for a loaded
    /// character. It is set at character select, because the map test in the loading burst
    /// is the first place the index exists.
    map: Option<i32>,
    /// `m_bChatCounter` (`server/server/game/char.cpp:8759`), which is per character and
    /// therefore per descriptor.
    chat: ChatState,
    /// The registry entry this descriptor occupies on its map, taken when the game phase is
    /// entered and released when the descriptor ends.
    presence: Option<Lease>,
    /// The world entry this character holds, taken when the game phase is entered and
    /// released before the final save.
    ///
    /// Separate from `presence` because the two are not the same claim and do not
    /// fail together. `presence` is this descriptor's slot in the broadcast set;
    /// `world` says the game thread has a character here. A descriptor that is in
    /// the first and not the second is a client the world cannot be given items for,
    /// and the close path has to leave the world only when it actually joined.
    world: Option<common::vid::Vid>,
    /// The save event, started when the character enters the game and stopped when the
    /// descriptor ends (legacy `StartSaveEvent` at `G/input_login.cpp:656`).
    save: Option<SaveEvent>,
    /// The items the load placed at character select, each at the cell the client was
    /// told, held until the world admits the character and taken with it.
    items: Vec<(ItemPos, Item)>,
}

/// The per-descriptor half of legacy's save cycle.
///
/// Legacy keeps a shared queue (`CHARACTER_MANAGER::m_set_pkChrForDelayedSave`) that the game
/// loop drains every 29 Pulses, and a per-character event that queues. There is no game loop yet,
/// so the Rewrite runs the event on the descriptor that owns the character and writes the row
/// itself. The queue is therefore not reproduced: [`SaveEvent::queued`] records only whether the
/// event has fired at least once, which is when legacy's set would first have held this
/// character, and both arms of the disconnect save write the row, so nothing observable rides on
/// it. What is reproduced is the two things a client can see: the row is written within one
/// cycle, and it is written when the character disconnects.
struct SaveEvent {
    /// `event_create(save_event, info, save_event_second_cycle)`. The first tick is one full
    /// cycle after `enter_game`, because `event_create` arms the first fire a period out.
    interval: tokio::time::Interval,
    /// `m_dwPlayStartTime`, which `ResetPlayTime()` sets to the current `get_dword_time()` in
    /// `CInputLogin::Entergame` (`G/input_login.cpp:653`).
    play_start: std::time::Instant,
    /// Whether the save event has fired, which is when legacy's queue first holds the character.
    queued: bool,
}

/// The slice of `CHARACTER` that movement and chat need, kept on the descriptor rather than
/// in a world, because one world per Channel has not been built yet.
///
/// # Where each field comes from
///
/// `map` is the atlas lookup of the loaded x and y, `x` and `y` are the saved columns,
/// `sitting` starts false because a loaded character is always standing, and
/// `move_speed` is `POINT_MOV_SPEED` from the same [`points`] array the loading burst
/// sends, so the movement gate and the record the client shows cannot disagree.
#[derive(Debug, Clone, PartialEq)]
struct Avatar {
    /// `GetMapIndex()`.
    map: i32,
    /// `GetX()`, the authoritative x.
    x: i32,
    /// `GetY()`, the authoritative y.
    y: i32,
    /// `m_fRot`, from `SetRotation(pinfo->bRot * 5)`.
    rotation: f32,
    /// `m_posDest`, which `Goto` sets and `Stop` clears.
    destination: Option<(i32, i32)>,
    /// `IsState(POS_SITTING)`.
    sitting: bool,
    /// `GetLimitPoint(POINT_MOV_SPEED)`, which gates the `FUNC_MOVE` branch.
    move_speed: i32,
    /// `GetCurrentMoveDuration()`, the `dwDuration` a `FUNC_MOVE` record carries.
    move_duration: u32,
    /// `GetSyncHackCount()`, the per-character sync-position refusal counter.
    sync_hack_count: u32,
}

/// Resolves when another descriptor has logged in with the login this one holds; never without
/// a held login.
async fn logon_kicked(logon: Option<&LogonClaim>) {
    match logon {
        Some(claim) => claim.kicked().await,
        None => std::future::pending().await,
    }
}

/// Serve one client connection as the legacy descriptor does.
///
/// On accept the descriptor sends `GC_PHASE(PHASE_HANDSHAKE)` and `GC_HANDSHAKE`
/// (`DESC::Setup`), then reads frames and applies the phase analyzers the Rewrite has, and every
/// `ping_event_second_cycle` runs the ping event. A header no analyzer handles closes the
/// connection.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    role: ListenerRole,
    context: ConnectionContext,
    shutdown_tx: &broadcast::Sender<()>,
) {
    info!(%addr, %role, "New client connection");
    let mut shutdown_rx = shutdown_tx.subscribe();
    let kind = match role {
        ListenerRole::Auth => HandshakeServerKind::Auth,
        ListenerRole::Channel(_) => HandshakeServerKind::Game,
    };
    let channel_number = match role {
        ListenerRole::Channel(number) => number,
        ListenerRole::Auth => 0,
    };
    let locations = match (role, stream.local_addr()) {
        (ListenerRole::Channel(number), Ok(local)) => Some(context.locations(number, local.port())),
        (ListenerRole::Channel(_), Err(error)) => {
            warn!(%addr, %error, "Client socket has no local address");
            return;
        }
        (ListenerRole::Auth, _) => None,
    };
    let handle = context
        .handles
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let clock = context.clock;
    let mut session =
        match LiveClientSession::start(stream, handshake_token(), kind, clock.now()).await {
            Ok((session, _)) => session,
            Err(error) => {
                warn!(%addr, %error, "Client setup failed");
                return;
            }
        };
    // `event_create(ping_event, ..., ping_event_second_cycle)`: the first ping is one full cycle
    // after the accept.
    let ping = tokio::time::interval_at(
        tokio::time::Instant::now() + context.ping_cycle,
        context.ping_cycle,
    );
    let mut held = Held::default();
    let mut descriptor = Descriptor {
        addr,
        clock,
        ping,
        locations,
        handle,
        channel_number,
    };
    pump_descriptor(
        &mut session,
        &context,
        &mut descriptor,
        &mut shutdown_rx,
        &mut held,
    )
    .await;
    // `DESC::SetPlayer(NULL)` on a disconnect (`G/input_db.cpp`, `PlayerDestroy`) removes
    // the character from the world, and it runs **before** the save below. That order
    // is the one that matters: leaving the world frees the cells this character's
    // items hold, so a save that ran first would name a cell another character can
    // already have been given, and a grant that arrived between the two would be
    // placed into an inventory the world is about to release.
    if let Some(world_vid) = held.world.take() {
        match context.game.leave_world(world_vid).await {
            Ok(true) => info!(%addr, vid = world_vid.raw(), "Character left the world"),
            // A leave that found nobody is a descriptor that never entered, which the
            // `Option` already rules out, so this is a world that lost the character
            // some other way. The disconnect continues either way.
            Ok(false) => {
                warn!(%addr, vid = world_vid.raw(), "The world did not hold this character at disconnect");
            }
            // A leave with no answer means the game thread is gone. The process is
            // losing the world with it, so the descriptor still closes.
            Err(error) => {
                warn!(%addr, vid = world_vid.raw(), %error, "The world could not be told this character left");
            }
        }
    }
    // `CHARACTER::Disconnect` flushes the queued save and, if the character was not queued,
    // calls `SaveReal` itself (`G/char.cpp:1786-1789`), then sets `m_bSkipSave` so nothing
    // writes after (`G/char.cpp:1800`). Both arms write the row, so the branch is the log line
    // only; the write is here because the descriptor still exists, which is the other of
    // `SaveReal`'s two guards (`G/char.cpp:1660-1664`).
    if held.save.is_some() {
        let queued = held.save.as_ref().is_some_and(|event| event.queued);
        let written = save_held(&context, &held, std::time::Instant::now()).await;
        info!(
            %addr,
            ?queued,
            route = ?prodomo::save::judge_disconnect_save(queued),
            written,
            "Character disconnected; wrote the row"
        );
    }
    if let Some(lease) = held.presence.as_ref() {
        info!(%addr, presence = lease.id(), "Leaving the client set");
        // `DESC::SetPlayer(NULL)` on a disconnect removes the character from the world, so a
        // later `FindCharacter` in `CInputMain::SyncPosition` finds nobody. Releasing the
        // lease takes the client out of the Channel set, but the position table stands in
        // for the world, so it has to be told as well or a departed character would keep
        // answering claims.
        context.positions.forget(lease.id());
    }
    // `held` drops here, which releases the lease and deregisters the character.
    drop(held);
}

/// The per-connection values the read loop needs, none of which change while it runs.
///
/// They are grouped so the loop takes five arguments instead of ten, and so the values a
/// handler might need are named in one place.
struct Descriptor {
    /// The peer's address, which every log line carries.
    addr: SocketAddr,
    /// The monotonic clock the ping and the session timeouts read.
    clock: BootLiveClock,
    /// `event_create(ping_event, ..., ping_event_second_cycle)`.
    ping: tokio::time::Interval,
    /// Where each map is served from, for a Channel descriptor.
    locations: Option<MapLocations>,
    /// The connection's ordinal, which is `m_dwHandle`.
    handle: u32,
    /// The Channel number, which is `g_bChannel`.
    channel_number: u8,
}

/// The descriptor read loop: consume frames, write broadcasts, and stop at a close.
///
/// The loop owns the lease's queue between turns, so it is a function of its own rather than
/// a block inside `handle_connection`. It returns when the descriptor must end; the caller
/// then drops the lease, which deregisters the character.
async fn pump_descriptor<S>(
    session: &mut LiveClientSession<S>,
    context: &ConnectionContext,
    descriptor: &mut Descriptor,
    shutdown_rx: &mut broadcast::Receiver<()>,
    held: &mut Held,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let addr = descriptor.addr;
    let clock = descriptor.clock;
    let locations = descriptor.locations.as_ref();
    let handle = descriptor.handle;
    let channel_number = descriptor.channel_number;
    // Broadcast records land in the lease's queue and are written here, so no world work
    // ever awaits a socket. The queue is lent out of the lease for the duration of one
    // `select!`, because the lease itself belongs to `held` and the analyzer may replace it.
    let mut outbox: Option<mpsc::UnboundedReceiver<Vec<u8>>> = None;
    let mut pending: Vec<Vec<u8>> = Vec::new();

    loop {
        // Take the queue back from the previous turn and lend it out again for this turn's
        // select. A lease the analyzer created still has its queue in place, so
        // `take_receiver` hands that one straight over.
        match held.presence.as_mut() {
            Some(lease) => {
                if let Some(inbox) = outbox.take() {
                    lease.put_receiver(inbox);
                }
                outbox = lease.take_receiver();
            }
            None => outbox = None,
        }
        // A burst another client caused while this one was thinking is written before the
        // next read, so a record can never sit behind a frame the client already sent.
        let mut wrote = true;
        for record in pending.drain(..) {
            if let Err(error) = session.send(&record).await {
                warn!(%addr, %error, "Client session stopped while writing a broadcast");
                wrote = false;
                break;
            }
        }
        if !wrote {
            break;
        }
        match session.next_buffered(clock.now()).await {
            Ok(Some(step)) => {
                let seat = locations.map(|locations| ChannelSeat {
                    locations,
                    handle,
                    number: channel_number,
                });
                let open = analyze(session, addr, context, seat, step, held).await;
                if !open || session.phase() == ClientPhase::Close {
                    break;
                }
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(%addr, %error, "Client session stopped");
                break;
            }
        }
        tokio::select! {
            read = session.read_input() => match read {
                Ok(0) => {
                    info!(%addr, leftover = session.buffered(), "Client connection closed");
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    warn!(%addr, %error, "Client session stopped");
                    break;
                }
            },
            _ = descriptor.ping.tick() => {
                if let Err(error) = session.tick(clock.now()).await {
                    warn!(%addr, %error, "Client session stopped");
                    break;
                }
                if session.phase() == ClientPhase::Close {
                    info!(%addr, "Client did not answer the ping; closing");
                    break;
                }
            }
            // `CHARACTER::save_event` (`G/char.cpp:5070-5082`): the character is queued, and
            // the row is written on the next drain. There is no game loop to drain here, so
            // this arm writes the row the drain would have written.
            () = save_tick(held.save.as_mut()) => {
                if let Some(event) = held.save.as_mut() {
                    event.queued = true;
                }
                if save_held(context, &*held, std::time::Instant::now()).await {
                    info!(%addr, "Save event wrote the character row");
                }
            }
            _ = shutdown_rx.recv() => {
                info!(%addr, "Client session stopping for shutdown");
                break;
            }
            () = logon_kicked(held.logon.as_ref()) => {
                // `DESC::DisconnectOfSameLogin` without a character: `SetPhase(PHASE_CLOSE)`.
                info!(%addr, "Another client logged in with this login; closing");
                break;
            }
            // A record another character caused: chat on this map, or a move around it. The
            // arm only receives; the write happens after the select, because `session` is
            // borrowed mutably by the other arms.
            broadcast = next_broadcast(&mut outbox) => {
                if let Some(record) = broadcast {
                    pending.push(record);
                } else {
                    // The member's sender is gone, which deregistration would also do. The
                    // lease is still held, so the only way here is a closed queue: keep the
                    // descriptor and stop expecting records.
                    info!(%addr, "Broadcast queue closed; no more records");
                    outbox = None;
                }
            }
        }
    }
}

/// `CG_ITEM_MOVE` (13) in the game phase: `CHARACTER::MoveItem` (`G/char_item.cpp:7602`).
///
/// The world moves the item, the rows are written in one transaction, and only then are the
/// records sent, so the client is never told of a move the store did not take. A refusal
/// sends the notice legacy sends for it, if any, and keeps the connection. A frame that does
/// not decode, a world that does not answer, and a write that fails each close the
/// descriptor; the close takes the character out of the world, and the next login loads what
/// the store holds.
async fn move_an_item<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgItemMove::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ITEM_MOVE; closing");
            return false;
        }
    };
    let Some(vid) = held.world else {
        // `if (ch)`: legacy does nothing for a descriptor with no character, and so does this.
        info!(%addr, "ITEM_MOVE without a character in the world; ignoring");
        return true;
    };
    let request = MoveRequest {
        from: record.from,
        to: record.to,
        count: record.count,
    };
    let moved = match context.game.move_item(vid, request).await {
        Ok(Ok(moved)) => moved,
        Ok(Err(MoveItemRefused::Refused(reason))) => {
            info!(%addr, %reason, "Item move refused");
            // `ChatPacket` takes the empire from the descriptor, which is the character's.
            let empire = held
                .character
                .as_ref()
                .map_or(0, |character| character.empire);
            if let Some(line) = prodomo::item_move::refusal_notice(&reason, empire) {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        Ok(Err(refused)) => {
            warn!(%addr, %refused, "The world does not hold this descriptor's character; closing");
            return false;
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked to move an item; closing");
            return false;
        }
    };
    if let Err(error) =
        db::items::apply_row_changes(&context.store, moved.owner_id, &moved.changes).await
    {
        warn!(%addr, %error, kind = ?moved.kind, "The item move could not be stored; closing");
        return false;
    }
    if let Err(error) = send_all(session, &moved.records).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    info!(%addr, kind = ?moved.kind, rows = moved.changes.len(), "Item moved");
    true
}

/// Wait for the next record another character caused this descriptor's map to receive.
///
/// A descriptor with no lease has no queue to wait on, so its future never completes and
/// `select!` waits on the other arms alone. That is why this is one function rather than a
/// `select!` arm over an `Option`: an arm over a `None` receiver returns at once and would
/// spin the loop.
async fn next_broadcast(outbox: &mut Option<mpsc::UnboundedReceiver<Vec<u8>>>) -> Option<Vec<u8>> {
    match outbox.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

/// `CG_CHAT` (3) in the game phase: `CInputMain::Chat` (`input_main.cpp:781-991`).
///
/// The judge owns the legacy order, so this function only applies the effects. The two
/// broadcasts are the ones the judge asks for: an info line to the sender, and a talking
/// line to every client on the sender's map including the sender. The registry lease is
/// what makes the sender one of its own recipients.
async fn chat_line<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgChat::decode(frame) {
        Ok(record) => record,
        Err(error) => {
            // A `size` under the fixed part is the legacy close case and is already
            // expressed as `ChatOutcome::Close`; anything else here is a frame this
            // process could not read, which is the recorded divergence.
            warn!(%addr, %error, "Client sent a malformed CHAT; closing");
            return false;
        }
    };
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "CHAT without a character; ignoring");
        return true;
    };
    let map = held.avatar.as_ref().map_or(0, |avatar| avatar.map);
    let chat_context = ChatContext {
        name: character.name.clone(),
        vid: character.id,
        empire: character.empire,
        level: character.level,
        // The Rewrite has no way to set `AFFECT_BLOCK_CHAT` yet, so it is always absent,
        // which is the legacy state for a character that was never silenced.
        block_chat_seconds: None,
    };
    let outcome = judge_chat(&record, &mut held.chat, &chat_context);
    let ChatOutcome::Judged { effects } = outcome else {
        info!(%addr, "CHAT declared a size under the fixed part; closing");
        return false;
    };
    for effect in &effects {
        match effect {
            ChatEffect::Close => {
                info!(%addr, "CHAT refused by size; closing");
                return false;
            }
            ChatEffect::Command { argument } => {
                // `interpret_command(ch, buf + 1, buflen - 1)`. The Rewrite has no command
                // interpreter, so a slash line is consumed and logged. It is still a
                // distinct effect from the rest, because it costs no chat counter and so a
                // client may send it between two normal lines.
                info!(%addr, bytes = argument.len(), "Command line received; no interpreter yet");
            }
            ChatEffect::DelayedDisconnect => {
                // `ch->GetDesc()->DelayedDisconnect(0)`: no record, the frame is consumed,
                // and the descriptor closes on its own schedule.
                info!(%addr, "Chat counter reached the disconnect line; closing");
                return false;
            }
            ChatEffect::InfoToSender { .. } | ChatEffect::ShoutBelowLevel { .. } => {
                // `ChatPacket` takes the empire from the descriptor, which is the speaker's.
                let Some(bytes) = prodomo::chat::info_line(effect, character.empire) else {
                    warn!(%addr, "Info effect built no line; closing");
                    return false;
                };
                if let Err(error) = send_all(session, &[bytes]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            ChatEffect::TalkingToMap { vid, empire, text } => {
                let record = prodomo::chat::talking_record(effect)
                    .unwrap_or_else(|| panic!("a talking effect always builds a record"));
                if record.id != *vid || record.empire != *empire || record.text != *text {
                    warn!(%addr, "Talking record does not match its effect");
                }
                // `FEmpireChatPacket` walks the process client set and filters by map
                // index, with no self-exclusion, so this reaches the speaker too.
                let bytes = record
                    .encode()
                    .unwrap_or_else(|error| panic!("a talking record always fits: {error}"));
                let sent = context.clients.broadcast_on_map(held.channel, map, &bytes);
                info!(%addr, vid, map, recipients = sent, "Talking line sent to the map");
            }
            ChatEffect::UnknownType { chat_type } => {
                // The legacy default arm only logs. Nothing reaches the client.
                info!(%addr, chat_type, "Unknown chat type; nothing sent");
            }
            ChatEffect::ShoutOnCooldown => {
                // `return (iExtraLen)` with no line (`input_main.cpp:893-894`). The shout's
                // own broadcast is not ported, so every shout at or above the level lands
                // here.
                info!(%addr, "Shout dropped; the shout broadcast is not ported");
            }
        }
    }
    true
}

/// `CG_MOVE` (7) in the game phase: `CInputMain::Move` (`input_main.cpp:1757-1915`).
fn move_character<S>(
    _session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgMove::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed MOVE; closing");
            return false;
        }
    };
    let Some(avatar) = held.avatar.as_mut() else {
        warn!(%addr, "MOVE without a character; ignoring");
        return true;
    };
    let move_context = MoveContext {
        vid: held.character.as_ref().map_or(0, |character| character.id),
        map: avatar.map,
        x: avatar.x,
        y: avatar.y,
        // The Rewrite has no mount yet, so nothing is riding.
        riding: false,
        // The Rewrite has no death state yet.
        dead: false,
        // The Rewrite has no stun affect and no private shop, so `CanMove` always holds.
        can_move: true,
        move_speed: avatar.move_speed,
        move_duration: avatar.move_duration,
    };
    let outcome = judge_move(&record, &move_context);
    let Some(lease) = held.presence.as_ref() else {
        // A move with no registry entry is a character that never entered the game phase,
        // which the phase gate already prevents. Log it rather than broadcasting blind.
        warn!(%addr, "MOVE without a registry lease; ignoring");
        return true;
    };
    match outcome {
        MoveOutcome::Ignore(reason) => {
            info!(%addr, reason, "MOVE consumed without a record");
        }
        MoveOutcome::Refuse(refusal) => {
            // `ch->Show(current)` and `ch->Stop()`: the client is pulled back. The
            // Rewrite has no view record for a character to show itself, so the
            // rejection is a log line; the position is already unchanged.
            match refusal {
                MoveRefusal::CannotMove => info!(%addr, "MOVE refused: cannot move"),
                MoveRefusal::InvalidFunction { function } => {
                    warn!(%addr, function, "MOVE refused: invalid function byte");
                }
                MoveRefusal::TooFar { distance } => {
                    warn!(%addr, distance, "MOVE refused: too far");
                }
                MoveRefusal::Dead => info!(%addr, "MOVE refused: dead character"),
            }
        }
        MoveOutcome::Accept(accepted) => {
            avatar.rotation = accepted.rotation;
            match accepted.disposition {
                MoveDisposition::Goto { x, y } => {
                    // `Goto` only stores the destination. The authoritative position is not
                    // updated, so the next move is measured from where the character still
                    // is, which is what the legacy distance test reads.
                    avatar.destination = Some((x, y));
                }
                MoveDisposition::Step { x, y } => {
                    avatar.x = x;
                    avatar.y = y;
                    avatar.destination = Some((x, y));
                }
            }
            // Only a step moves the character, so only a step updates the table a
            // sync-position claim is measured against. A `Goto` leaves the position where it
            // was, which is the legacy behaviour and the reason the next move is still
            // measured from the old spot.
            if let MoveDisposition::Step { x, y } = accepted.disposition {
                context.positions.sync(lease.id(), x, y, sync_clock());
            }
            // `ch->PacketAround(&pack, sizeof(TPacketGCMove), ch)`: the mover is the
            // `except` argument, so the record never echoes to its own sender.
            let recipients = context.clients.broadcast_excluding(
                held.channel,
                avatar.map,
                lease.id(),
                &accepted.broadcast.encode(),
            );
            info!(
                %addr,
                function = record.function,
                x = record.x,
                y = record.y,
                recipients,
                "Move broadcast"
            );
        }
    }
    true
}

/// `CG_CHARACTER_POSITION` (28) in the game phase: `CInputMain::Position`
/// (`input_main.cpp:1530-1548`).
///
/// The pose record goes to everyone on the map **including** this character, because
/// `CHARACTER::Standup` and `CHARACTER::Sitdown` call `PacketAround` with no `except`
/// and `CEntity::PacketView` always finishes with a self-send.
fn character_pose<S>(
    _session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgCharacterPosition::decode(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed CHARACTER_POSITION; closing");
            return false;
        }
    };
    let Some(avatar) = held.avatar.as_mut() else {
        warn!(%addr, "CHARACTER_POSITION without a character; ignoring");
        return true;
    };
    let position = match judge_pose(record.position) {
        PoseOutcome::Ignore => return true,
        PoseOutcome::Stand => {
            if !avatar.sitting {
                // `if (!IsPosition(POS_SITTING)) return;`
                return true;
            }
            avatar.sitting = false;
            protocol::cg_position::POSITION_GENERAL
        }
        PoseOutcome::Sit { ground } => {
            if avatar.sitting {
                // `if (IsPosition(POS_SITTING)) return;`
                return true;
            }
            avatar.sitting = true;
            // Legacy always writes the ground value. The Rewrite honours the request,
            // which is the one deliberate difference in this function.
            if ground {
                protocol::cg_position::POSITION_SITTING_GROUND
            } else {
                protocol::cg_position::POSITION_SITTING_CHAIR
            }
        }
    };
    let Some(lease) = held.presence.as_ref() else {
        warn!(%addr, "CHARACTER_POSITION without a registry lease; ignoring");
        return true;
    };
    let vid = held.character.as_ref().map_or(0, |character| character.id);
    let recipients = context.clients.broadcast_on_map(
        held.channel,
        avatar.map,
        &prodomo::movement::pose_record(vid, position).encode(),
    );
    info!(%addr, position, recipients, "Pose broadcast, including the sender");
    // `PacketAround` with a `NULL` except still runs the trailing self-send, so the
    // sender's own descriptor needs the record too. It is already in the map broadcast
    // above, so the lease is only read for the map scope.
    let _ = lease;
    true
}

/// The live ports for `CG_SYNC_POSITION` (8): `CInputMain::SyncPosition`
/// (`input_main.cpp:2088-2171`).
///
/// The policy in [`prodomo::sync_position`] is already complete, so this adapter supplies
/// the six things it deliberately does not own: the victim lookup, the sync-ownership
/// bookkeeping, the movement itself, the close, and the view broadcast.
struct LiveSync<'a> {
    /// The position table, which stands in for the process character map.
    table: &'a PositionTable,
    /// The client set, for the victim search and the broadcast.
    clients: &'a ChannelClients,
    /// The claimer's Channel.
    channel: u8,
    /// The claimer's map. A claim never leaves the claimer's map, which is how the
    /// Rewrite keeps one process from answering for another Channel.
    map: i32,
    /// The claimer's own lease, which is the `except` of the broadcast.
    actor: u64,
    /// Every close the policy asked for, in order. The handler acts on them after the
    /// policy returns, because the policy must not be able to end the descriptor
    /// half-way through its own broadcast.
    closes: Vec<SyncPositionCloseReason>,
    /// The lease the last successful `resolve` found, so the ownership and stamp calls
    /// that follow it do not each have to search again.
    remembered: Option<u64>,
    /// The clock reading the stamps are taken from, so every element in one frame carries
    /// the same instant, as legacy's single `get_dword_time()` does.
    now: Duration,
    /// The claimer's own x, which `SetSyncOwner`'s `DISTANCE_APPROX` compares.
    actor_x: i32,
    /// The claimer's own y.
    actor_y: i32,
    /// One `TPacketGCOwnership` per accepted claim. `SetSyncOwner` writes each one with a
    /// plain `PacketAround` as the claim is judged, which is before `CInputMain::SyncPosition`
    /// sends its own position batch at the end of the handler.
    ownership: Vec<Vec<u8>>,
    /// The position batch, which is the handler's own `PacketAround(ch, ...)` and the only
    /// record that excepts the claimer.
    batch: Option<Vec<u8>>,
}

impl SyncPositionPorts for LiveSync<'_> {
    fn resolve(&mut self, vid: Vid) -> Option<SyncPositionVictim> {
        let (id, tracked) =
            self.table
                .find_on_map(self.clients, self.channel, self.map, vid.raw())?;
        self.remembered = Some(id);
        Some(SyncPositionVictim {
            vid,
            kind: tracked.kind,
            x: tracked.x,
            y: tracked.y,
        })
    }

    fn set_sync_owner(&mut self, actor: Vid, victim: &SyncPositionVictim) -> bool {
        // `CHARACTER::SetSyncOwner` (`server/server/game/char.cpp:5469-5551`) is judged by
        // the pure policy; this applies the two state effects its acceptance asks for.
        let Some(id) = self.remembered else {
            return false;
        };
        let state = match self.table.sync_owner(id) {
            Some((owner, claimed_at)) => SyncOwnershipState {
                owner: Some(Vid::new(owner)),
                claimed_at,
            },
            None => SyncOwnershipState {
                owner: None,
                claimed_at: Duration::ZERO,
            },
        };
        let outcome =
            judge_sync_ownership(actor, victim, state, self.actor_x, self.actor_y, self.now);
        match outcome {
            SyncOwnershipOutcome::Refused => false,
            SyncOwnershipOutcome::Accepted {
                owner_changed,
                record,
            } => {
                // A new owner resets the victim's last-sync stamp, which is what lets its
                // first accepted sync pass the 100 ms interval check. The claim stamp is
                // refreshed on every accepted claim, which is what keeps the claim
                // exclusive for as long as the other owner keeps claiming.
                if owner_changed {
                    self.table.forget_sync(id);
                }
                self.table.set_sync_owner(id, actor.raw(), self.now);
                self.ownership.push(record);
                true
            }
        }
    }

    fn broadcast_around_victim(&mut self, _victim: Vid, packet: &[u8]) {
        self.ownership.push(packet.to_vec());
    }

    fn last_sync_time(&mut self, victim: Vid) -> Option<Duration> {
        let (id, _) = self
            .table
            .find_on_map(self.clients, self.channel, self.map, victim.raw())?;
        self.remembered = Some(id);
        self.table.last_sync(id)
    }

    fn set_last_sync_time(&mut self, victim: Vid, now: Duration) {
        if let Some((id, _)) =
            self.table
                .find_on_map(self.clients, self.channel, self.map, victim.raw())
        {
            self.table.sync(
                id,
                self.table.get(id).map_or(0, |t| t.x),
                self.table.get(id).map_or(0, |t| t.y),
                now,
            );
        }
    }

    fn sync(&mut self, victim: Vid, x: i32, y: i32) {
        if let Some((id, _)) =
            self.table
                .find_on_map(self.clients, self.channel, self.map, victim.raw())
        {
            self.table.sync(id, x, y, self.now);
        }
    }

    fn close(&mut self, _actor: Vid, reason: SyncPositionCloseReason) {
        self.closes.push(reason);
    }

    fn broadcast(&mut self, _actor: Vid, packet: &[u8]) {
        // `CInputMain::SyncPosition` passes `ch` as the `except`, and
        // `FuncPacketAround::operator()` returns early for the excepted entity, so the
        // claimer never receives its own accepted positions. The record is queued so the
        // ownership records `SetSyncOwner` already wrote are delivered first.
        self.batch = Some(packet.to_vec());
    }
}

/// `CG_SYNC_POSITION` (8) in the game phase.
fn sync_positions<S>(
    _session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let packet = match protocol::cg_variable::decode_sync_position(frame) {
        Ok(packet) => packet,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed SYNC_POSITION; closing");
            return false;
        }
    };
    let Some(avatar) = held.avatar.as_mut() else {
        warn!(%addr, "SYNC_POSITION without a character; ignoring");
        return true;
    };
    let Some(lease) = held.presence.as_ref() else {
        warn!(%addr, "SYNC_POSITION without a registry lease; ignoring");
        return true;
    };
    let mut actor = SyncPositionActor {
        vid: held
            .character
            .as_ref()
            .map_or_else(|| Vid::new(0), |character| Vid::new(character.id)),
        x: avatar.x,
        y: avatar.y,
        sync_hack_count: avatar.sync_hack_count,
    };
    let now = sync_clock();
    let mut port = LiveSync {
        table: &context.positions,
        clients: &context.clients,
        channel: held.channel,
        map: avatar.map,
        actor: lease.id(),
        closes: Vec::new(),
        now,
        remembered: None,
        actor_x: avatar.x,
        actor_y: avatar.y,
        ownership: Vec::new(),
        batch: None,
    };
    let result = prodomo::sync_position::process(&mut port, &packet, &mut actor, now);
    // The policy counts a refusal on the actor, and that count survives the frame even
    // though the descriptor stays open.
    avatar.sync_hack_count = actor.sync_hack_count;
    if !port.closes.is_empty() {
        warn!(
            %addr,
            reason = ?port.closes,
            "SYNC_POSITION refused the client; closing"
        );
        return false;
    }
    // `SetSyncOwner` writes `TPacketGCOwnership` with a plain `PacketAround`, so the
    // victim and every other client on the map sees it, the claimer included. The
    // position batch is written after every claim is judged and excepts the claimer.
    for record in &port.ownership {
        context
            .clients
            .broadcast_on_map(held.channel, port.map, record);
    }
    if let Some(batch) = &port.batch {
        context
            .clients
            .broadcast_excluding(held.channel, port.map, port.actor, batch);
    }
    info!(
        %addr,
        processed = result.processed_elements,
        accepted = result.accepted_elements.len(),
        broadcast = result.gc_packet.is_some(),
        "Sync positions judged"
    );
    true
}

/// Apply the analyzer of the descriptor's phase to one step; `false` when the connection must
/// close. `seat` is `None` on the auth port.
async fn analyze<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: Option<ChannelSeat<'_>>,
    step: LiveStep,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match step {
        // `CInputHandshake::Analyze` (`G/input.cpp:227-234`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Handshake
                && frame.header == HEADER_CG_STATE_CHECKER.value() =>
        {
            send_channel_status(session, addr, context).await
        }
        // `CInputAuth::Analyze` (`G/input_auth.cpp:209-217`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Auth && frame.header == HEADER_CG_LOGIN3.value() =>
        {
            auth_login(session, addr, context, &frame, &mut held.claim).await
        }
        // `CInputLogin::Analyze` (`G/input_login.cpp:1181-1182`), which serves the
        // login, select, and loading phases (`G/desc.cpp:515-519`).
        LiveStep::Record { phase, frame }
            if matches!(
                phase,
                ClientPhase::Login | ClientPhase::Select | ClientPhase::Loading
            ) && frame.header == HEADER_CG_LOGIN2.value() =>
        {
            match seat {
                Some(seat) => channel_login(session, addr, context, &seat, &frame, held).await,
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
        }
        // The select-screen records of `CInputLogin::Analyze` (`G/input_login.cpp:1185-1244`).
        LiveStep::Record { phase, frame }
            if matches!(
                phase,
                ClientPhase::Login | ClientPhase::Select | ClientPhase::Loading
            ) && SELECT_HEADERS.contains(&frame.header) =>
        {
            match seat {
                Some(seat) => {
                    select_screen(session, addr, context, &seat, &frame, &mut held.account).await
                }
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
        }
        // `CG_CHARACTER_SELECT` (6) in the select phase: `CInputLogin::CharacterSelect`
        // (`G/input_login.cpp:265-306`). Legacy sends nothing and waits for the load answer,
        // which the store gives in this process.
        LiveStep::Record { phase, frame }
            if matches!(phase, ClientPhase::Select | ClientPhase::Loading)
                && frame.header == HEADER_CG_CHARACTER_SELECT.value() =>
        {
            match seat {
                Some(seat) => select_character(session, addr, context, &seat, &frame, held).await,
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
        }
        // `CG_ENTER_GAME` (10) in the loading phase: `CInputLogin::Entergame`
        // (`G/input_login.cpp:562-606`).
        LiveStep::Record { phase, frame }
            if matches!(phase, ClientPhase::Loading | ClientPhase::Game)
                && frame.header == HEADER_CG_ENTERGAME.value() =>
        {
            match seat {
                Some(seat) => enter_game(session, addr, context, &seat, &frame, held).await,
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
        }
        // `CG_CHAT` (3) in the game phase: `CInputMain::Chat` (`G/input_main.cpp:781`).
        // `CInputDead::Analyze` maps the same header, but a dead character cannot be
        // reached yet because the Rewrite has no death state.
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Game && frame.header == HEADER_CG_CHAT.value() =>
        {
            chat_line(session, addr, context, held, &frame).await
        }
        // `CG_ITEM_MOVE` (13) in the game phase: `CInputMain::ItemMove`
        // (`G/input_main.cpp:1060-1066`), reached when the character is not an observer
        // (`:3707-3710`). The Rewrite has no observer mode, so every character is reached.
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Game && frame.header == HEADER_CG_ITEM_MOVE.value() =>
        {
            move_an_item(session, addr, context, held, &frame).await
        }
        // `CG_MOVE` (7) in the game phase: `CInputMain::Move` (`G/input_main.cpp:1757`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Game && frame.header == HEADER_CG_MOVE.value() =>
        {
            move_character(session, addr, context, held, &frame)
        }
        // `CG_CHARACTER_POSITION` (28) in the game phase: `CInputMain::Position`
        // (`G/input_main.cpp:1530`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Game
                && frame.header == HEADER_CG_CHARACTER_POSITION.value() =>
        {
            character_pose(session, addr, context, held, &frame)
        }
        // `CG_SYNC_POSITION` (8) in the game phase: `CInputMain::SyncPosition`
        // (`G/input_main.cpp:2088`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Game && frame.header == HEADER_CG_SYNC_POSITION.value() =>
        {
            sync_positions(session, addr, context, held, &frame)
        }
        step => report_step(addr, step),
    }
}

/// What a Channel descriptor knows about its seat: where each map is served from its port, and
/// its handle.
struct ChannelSeat<'a> {
    locations: &'a MapLocations,
    handle: u32,
    /// The Channel number, which `g_bChannel` is on a legacy game process.
    number: u8,
}

/// Answer `LOGIN2` with the character list or `LOGIN_FAILURE`; `false` when the connection must
/// close. A refusal keeps the connection open, as in legacy.
///
/// The order is legacy's: the shutdown and user-limit checks answer under the handshake key; then
/// the client key is installed (`SetSecurityKey`) and the login key is judged
/// (`QUERY_LOGIN_BY_KEY`); on success `GC_EMPIRE`, `GC_LOGIN_SUCCESS`, and the select phase follow
/// (`CInputDB::LoginSuccess`).
async fn channel_login<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgLoginByKey::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed LOGIN2; closing");
            return false;
        }
    };
    let no_more_clients = context.shutdowned || context.state.is_shutting_down();
    if let Err(refusal) = admit(
        no_more_clients,
        context.user_limit,
        context.channels.online(),
    ) {
        return refuse_channel_login(session, addr, refusal).await;
    }
    let mut client_key = [0; 16];
    for (bytes, word) in client_key.chunks_exact_mut(4).zip(record.client_key) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    if let Err(error) = session.install_client_keys(client_key) {
        warn!(%addr, %error, "Client key could not be installed; closing");
        return false;
    }
    let typed_login = login_from_field(&record.login);
    let grant = match judge_key(
        context.auth.grant_for(record.login_key),
        typed_login.as_ref(),
        record.client_key,
    ) {
        Ok(grant) => grant,
        Err(refusal) => return refuse_channel_login(session, addr, refusal).await,
    };
    let claim = match context.logons.claim(&grant.login) {
        Ok(claim) => claim,
        // `CInputDB::LoginAlready` kicks the holder before it answers. When the holder is this
        // descriptor it is already in `PHASE_CLOSE`, where legacy drops every output.
        Err(_)
            if held
                .logon
                .as_ref()
                .is_some_and(|held| held.login() == &grant.login) =>
        {
            info!(%addr, "Client logged in again with the login it holds; closing");
            return false;
        }
        Err(refusal) => return refuse_channel_login(session, addr, refusal).await,
    };
    let account_lobby = match lobby(&context.store, grant.account).await {
        Ok(account_lobby) => account_lobby,
        // `RESULT_LOGIN_BY_KEY` finds no account row: `LOGIN_NOT_EXIST`.
        Err(AccountError::NoSuchAccountId(_)) => {
            return refuse_channel_login(session, addr, ChannelRefusal::NoId).await;
        }
        Err(error) => {
            error!(%addr, %error, "Channel login failed; closing");
            return false;
        }
    };
    let mut empire = Vec::with_capacity(GcHeaderAndByte::WIRE_SIZE);
    GcHeaderAndByte::new(HEADER_GC_EMPIRE.value(), empire_shown(&account_lobby))
        .encode_into(&mut empire);
    let success = login_success(
        &account_lobby,
        &context.atlas,
        seat.locations,
        seat.handle,
        random_key(),
    )
    .encode();
    info!(
        %addr,
        login = %claim.login().as_str(),
        characters = account_lobby.players.len(),
        "Channel login accepted"
    );
    held.logon = Some(claim);
    held.account = Some(SelectAccount {
        id: grant.account,
        login: grant.login,
        lobby: account_lobby,
    });
    let delivery = async {
        session.send(&empire).await?;
        session.send(&success).await?;
        session.set_phase(PostHandshakePhase::Select).await
    };
    match delivery.await {
        Ok(_) => true,
        Err(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

/// The select-screen headers.
const SELECT_HEADERS: [u8; 4] = [
    HEADER_CG_EMPIRE.value(),
    HEADER_CG_CHARACTER_CREATE.value(),
    HEADER_CG_CHARACTER_DELETE.value(),
    HEADER_CG_CHANGE_NAME.value(),
];

/// The world view the bursts describe.
///
/// The Rewrite has no world yet, so a Channel always reports an empty view: a second character
/// in the same map and a map's NPCs are both contributed by the world system, and neither has
/// a store to read them from. `Neighbourhood::default` is that empty view, and the bursts take
/// it the same way they will take a populated one.
fn empty_view() -> Neighbourhood {
    Neighbourhood::default()
}

/// The time `get_global_time()` reports: `time(0)` plus the game server's gap.
///
/// The Rewrite has no accumulated game time, so this is the wall clock. Legacy's
/// `get_global_time` adds a gap that starts at zero and grows with uptime (`G/game.cpp:106`),
/// which a client that shows a countdown would read differently; that is a Divergence until
/// the world system owns the clock.
/// The clock the sync-position policy reads.
///
/// The legacy stamps come from `get_dword_time()`, which is a 32-bit second count, but the
/// policy's shortest interval is 100 ms, so the Rewrite passes a `Duration` read from the
/// system clock at nanosecond resolution. A backwards clock step can only make a stamp look
/// older, which lets a client through the interval check sooner, exactly as a second counter
/// that rolls back would.
fn sync_clock() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

fn global_time() -> u32 {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    u32::try_from(seconds).unwrap_or(u32::MAX)
}

/// The VID the bursts use for the entering character.
///
/// Legacy's VID comes from `CHARACTER_MANAGER::CreateCharacter`, which hands out a world-wide
/// handle distinct from the player's ID. The Rewrite has no world, so the character's own
/// store ID stands in. A second character in the world will need a real allocator; the wire
/// layout is unchanged either way, so this is a Divergence in the value only.
fn character_vid(character: &db::players::Character) -> u32 {
    character.id
}

/// Serve one select-screen record; `false` when the connection must close. A malformed record
/// closes the connection.
async fn select_screen<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    account: &mut Option<SelectAccount>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let header = frame.header;
    let replies = if header == HEADER_CG_EMPIRE.value() {
        choose_empire(context, seat, frame, account).await
    } else if header == HEADER_CG_CHARACTER_CREATE.value() {
        create_character(context, seat, frame, account).await
    } else if header == HEADER_CG_CHARACTER_DELETE.value() {
        delete_character(context, frame, account).await
    } else {
        rename_character(context, frame, account).await
    };
    let replies = match replies {
        Ok(replies) => replies,
        Err(reason) => {
            warn!(%addr, header, %reason, "Select-screen record closes the connection");
            return false;
        }
    };
    for reply in &replies {
        if let Err(error) = session.send(reply).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    true
}

/// `CG_CHARACTER_SELECT`: load the selected character and send the loading burst; `false` when
/// the connection must close.
///
/// The order is legacy's `CInputDB::PlayerLoad` (`G/input_db.cpp:414-455`): the descriptor moves
/// to the loading phase first, then the entity and own-character records, then the `map_allow_find`
/// test, and only then the gold, points, and skill-level records. A character on a map this
/// Channel does not host is refused at that test, which is the branch the two halves of
/// [`LoadingBurst`] exist for.
///
/// The load itself is a store read here, where legacy asked its DB process. The two are the
/// same step in the same order; the Rewrite has no server-to-server protocol (ADR-0001).
async fn select_character<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgPlayerSelect::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed CHARACTER_SELECT; closing");
            return false;
        }
    };
    let Some(account) = held.account.as_ref() else {
        // No account table means the descriptor never completed a Channel login. Legacy's
        // `CharacterSelect` reads `c_r.players[pinfo->index]` straight away, which a client
        // that skipped the login can reach; there is nothing to select from here.
        warn!(%addr, "CHARACTER_SELECT without an account table; ignoring");
        return true;
    };
    let slots = account.lobby.slot_ids();
    let player = match judge_select(record.index, Some(&slots)) {
        SelectVerdict::Ignore => return true,
        SelectVerdict::Close => {
            // `SetPhase(PHASE_CLOSE)` writes no record, so the descriptor is simply
            // dropped: `analyze` returns false and the read loop breaks.
            info!(%addr, slot = record.index, "Empty character slot; closing");
            return false;
        }
        SelectVerdict::Load { player } => player,
    };
    let character = match load_character(&context.store, account.id, player).await {
        Ok(character) => character,
        Err(error) => {
            warn!(%addr, account = account.id.get(), player, %error, "Character load failed; closing");
            return false;
        }
    };
    if held.character.is_some() {
        // `PlayerLoad` refuses a second character on one descriptor and logs it. The client is
        // not asked to leave, so the descriptor stays in the loading phase with the character
        // it already has.
        error!(%addr, player, "Login state already has a character; ignoring the selection");
        return true;
    }
    let vid = character_vid(&character);
    let view = empty_view();
    // `d->BindCharacter(ch)` is the step that makes the character the descriptor's own, and it
    // is what `CInputLogin::Entergame` later reads. Without it `CG_ENTER_GAME` has nothing to
    // answer, so the loaded row is held here and kept for the descriptor's life.
    let burst = loading_burst(&character, vid, &view);
    held.character = Some(character.clone());
    held.channel = seat.number;
    info!(
        %addr,
        player,
        vid,
        channel = seat.number,
        name = %character.name.as_str(),
        "Character selected; sending the loading burst",
    );
    let delivery = async {
        // `SetPhase(PHASE_LOADING)` writes `GC_PHASE` and then installs the loading phase's
        // input boundary, before any other loading record.
        session.set_phase(PostHandshakePhase::Loading).await?;
        send_all(session, &burst.before_map_test).await?;
        // The map test. `PlayerLoad` folds an instance map onto its base map first.
        let map_index = context.atlas.index_at(character.x, character.y);
        let allowed = map_index.is_some_and(|index| {
            map_is_allowed(
                public_map_index(index),
                context
                    .routes
                    .channel_maps
                    .get(&seat.number)
                    .map_or(&[][..], Vec::as_slice),
            )
        });
        if !allowed {
            return refuse_map(session, addr, context, seat, &character, account, map_index).await;
        }
        send_all(session, &burst.after_map_test).await
    };
    if let Err(error) = delivery.await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    // Legacy's DB process answers the player row and then the item rows, so `ItemLoad` runs
    // after `PlayerLoad` has sent the loading burst, while the descriptor is still loading.
    let Some(items) = load_items(session, addr, context, &character).await else {
        return false;
    };
    held.items = items;
    // `PlayerLoad` leaves the character on the map its saved position belongs to, so the index
    // the map test just resolved is the character's map. It is held here because the first
    // thing the game phase does with it is join the client set.
    held.map = context.atlas.index_at(character.x, character.y);
    true
}

/// The item load at character select, the way `CInputDB::ItemLoad` places a character's
/// items (`G/input_db.cpp:1451-1567`).
///
/// One `GC_ITEM_SET` per placed item, then `PointsPacket`'s gold and points records, which
/// legacy sends even for a character with no item. A row the load refuses is logged and
/// left in the store; `prodomo::item_load` lists why a row is refused and where that
/// differs from legacy.
///
/// Returns the placed items for the world, or `None` when the descriptor must close: the
/// store could not be read, or the client stopped.
async fn load_items<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    character: &db::players::Character,
) -> Option<Vec<(ItemPos, Item)>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let player = character.id;
    let rows = match db::items::load_owner_items(&context.store, player).await {
        Ok(rows) => rows,
        Err(error) => {
            warn!(%addr, player, %error, "Item load failed; closing");
            return None;
        }
    };
    // `Inven_Point` is 0: the store keeps no value for it yet, and the world's character
    // starts at 0, so a set-aside item and a later grant search the same cells.
    let load = plan_item_load(&rows, &context.protos, 0);
    for refusal in &load.refused {
        warn!(
            %addr,
            player,
            item = refusal.id,
            window = refusal.window,
            pos = refusal.pos,
            why = %refusal.why,
            "An item was not loaded; its row is kept",
        );
    }
    for row in load.moved_rows(&rows) {
        // The client is told the new cell either way, and the world holds the item there.
        // A row that keeps its old cell is set aside again at the next load, so a failed
        // write is reported rather than fatal.
        if let Err(error) = db::items::save_item(&context.store, &row).await {
            warn!(
                %addr,
                player,
                item = row.id,
                pos = row.pos,
                %error,
                "A moved item's new cell was not written; its row keeps the old one",
            );
        }
    }
    let mut records: Vec<Vec<u8>> = load
        .placed
        .iter()
        .map(|placed| placed.record().encode())
        .collect();
    records.extend(points_packet(character));
    if let Err(error) = send_all(session, &records).await {
        warn!(%addr, %error, "Client session stopped");
        return None;
    }
    info!(
        %addr,
        player,
        placed = load.placed.len(),
        refused = load.refused.len(),
        "Items loaded",
    );
    Some(
        load.placed
            .into_iter()
            .map(|placed| (placed.pos, placed.item))
            .collect(),
    )
}

/// The loading map test failed: move the character home in the store and close, the way
/// `CInputDB::PlayerLoad` does (`G/input_db.cpp:426-436`).
///
/// Two separate things happen, and only one of them is observable on the wire:
///
/// 1. `SetPhase(PHASE_CLOSE)` is **silent**. It assigns the phase before it calls `Packet`, and
///    `Packet` returns at once for `PHASE_CLOSE` (`G/desc.cpp:495-500` against `:397-401`), so
///    the record it built is dropped. The client sees the close and nothing else.
/// 2. The pending warp location is what `CHARACTER::Save` writes instead of the live position
///    (`G/char.cpp:1551-1565`), and the save on close is what persists it
///    (`G/char.cpp:1784-1788`). No `GC_WARP` is sent on this path; the move happens entirely in
///    the store, and the next login loads the home map.
///
/// ADR-0003 makes a Warp one transaction, so the position write is the same event as the
/// refusal and happens here rather than in a later background save.
///
/// # Errors
///
/// Returns [`LiveError`] when the close record cannot be written. A position that cannot be
/// stored is logged and the descriptor still closes, because the refusal has to close either
/// way; leaving the character where it is makes the next login refuse the same map again, which
/// is the legacy Defect recorded below.
async fn refuse_map<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    character: &db::players::Character,
    account: &SelectAccount,
    map_index: Option<i32>,
) -> Result<(), LiveError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let player = character.id;
    match prodomo::warp::home_warp_location(account.lobby.empire) {
        Some(target) => {
            if let Err(error) =
                db::players::save_position(&context.store, account.id, player, target.x, target.y)
                    .await
            {
                warn!(%addr, player, %error, "Could not store the Warp position");
            } else {
                info!(
                    %addr,
                    player,
                    channel = seat.number,
                    map = ?map_index,
                    to_map = target.map_index,
                    x = target.x,
                    y = target.y,
                    "Entering map is not allowed on this Channel; warping home and closing \
                     silently",
                );
            }
        }
        None => {
            // Empire 0 or above 3 has no start, so there is nowhere to move the character to and
            // the row is left as it is. This is a legacy Defect: `EMPIRE_START_X` and
            // `EMPIRE_START_Y` are `((DWORD []) {0, ...})[ch->GetEmpire()]`, so an out-of-range
            // empire reads past the array; and legacy stores `(0, 0)`, which `Save` treats as
            // no pending warp, so the next login refuses the same map again.
            warn!(
                %addr,
                player,
                channel = seat.number,
                map = ?map_index,
                "Entering map is not allowed on this Channel and the account has no empire \
                 start; closing without moving the character",
            );
        }
    }
    session.close_phase().await?;
    Err(LiveError::Phase(LifecycleError::Closed))
}

/// `CG_ENTER_GAME`: send the enter-game burst; `false` when the connection must close.
///
/// The order is legacy's `CInputLogin::Entergame` (`G/input_login.cpp:562-606`): the own and
/// visible characters, the NPCs, and the revive-invisible affect, then `SetPhase(PHASE_GAME)`,
/// then the time, Channel, and event records. The phase change is the boundary between the two
/// halves of [`EnterGameBurst`], because in legacy it is a `DESC::SetPhase` call.
async fn enter_game<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Err(error) = CgEnterGame::decode_frame(frame) {
        warn!(%addr, %error, "Client sent a malformed ENTER_GAME; closing");
        return false;
    }
    if let EnterGameVerdict::Close = judge_enter_game(held.character.is_some()) {
        info!(%addr, "ENTER_GAME without a character; closing");
        return false;
    }
    // The reducer proved the character is there; the field is the only place it lives.
    let Some(character) = held.character.as_ref() else {
        return false;
    };
    let vid = character_vid(character);
    let burst = enter_game_burst(
        character,
        vid,
        &empty_view(),
        seat.number,
        global_time(),
        LANGUAGE_EUROPE,
    );
    info!(
        %addr,
        vid,
        channel = seat.number,
        "Entering the game; sending the enter-game burst",
    );
    let delivery = async {
        send_all(session, &burst.before_phase).await?;
        // `SetPhase(PHASE_GAME)` writes `GC_PHASE` through the boundary the loading phase left
        // installed, which is the encrypted one.
        session.set_phase(PostHandshakePhase::Game).await?;
        send_all(session, &burst.after_phase).await
    };
    if let Err(error) = delivery.await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    // The world state is created only once the game phase is on the wire, because a
    // descriptor that never received `GC_PHASE(5)` never gets a lease and so never appears
    // in another client's broadcast. `DESC::SetPlayer` is the legacy step that makes a
    // descriptor visible to the others, and it happens at this same moment.
    let slots = points(character);
    let speed = i32::try_from(slots[common::point_slot::POINT_MOV_SPEED]).unwrap_or(0);
    let map = held.map.unwrap_or_default();
    held.avatar = Some(Avatar {
        map,
        x: character.x,
        y: character.y,
        rotation: 0.0,
        destination: None,
        sitting: false,
        move_speed: speed,
        move_duration: 0,
        sync_hack_count: 0,
    });
    // `CInputLogin::Entergame` calls `ResetPlayTime()` and then `StartSaveEvent()`
    // (`G/input_login.cpp:653-656`), so the playtime clock and the save event both start when
    // the character enters the game, not when the descriptor is accepted.
    if held.save.is_none() {
        let now = tokio::time::Instant::now();
        held.save = Some(SaveEvent {
            interval: tokio::time::interval_at(now + context.save_cycle, context.save_cycle),
            play_start: std::time::Instant::now(),
            queued: false,
        });
    }
    if held.presence.is_none() {
        let lease = context.clients.join(ClientEntry {
            channel: seat.number,
            map,
            name: character.name.clone(),
            vid,
        });
        info!(
            %addr,
            vid,
            channel = seat.number,
            map,
            name = %character.name.as_str(),
            presence = lease.id(),
            "Character entered the game; registered on its map"
        );
        // `DESC::SetPlayer` also puts the character in the world, which is what a
        // sync-position claim later looks it up in. The table stands in for the world.
        context.positions.track(
            lease.id(),
            prodomo::sync_position::SyncPositionVictimKind::Player,
            vid,
            character.x,
            character.y,
        );
        let outbox = lease.outbox();
        held.presence = Some(lease);
        let items = std::mem::take(&mut held.items);
        let Some(world_vid) = join_the_world(context, addr, character, vid, items, outbox).await
        else {
            return false;
        };
        held.world = Some(world_vid);
    }
    true
}

/// Puts a live client's character into the game thread's world, and records the entry.
///
/// `DESC::SetPlayer` is the step that does this, and the world is where items live.
/// The client set and the position table answer broadcasts; neither holds an
/// inventory, so a grant would find nobody and a client that had been given an item
/// would find it gone at relog (ADR-0002, which puts every world on one game thread).
///
/// The outbox is the same queue the broadcast path already uses, so the world writes
/// into a queue this descriptor is already draining and there is no second delivery
/// path to keep in step with the first. The world never touches the socket.
///
/// The VID is the store's `player.id`, which is the number `character_vid` already
/// published in the enter-game burst. Handing the world the same number is what stops
/// one character from having two identities; legacy's own counter is a divergence
/// recorded in ledger 205.
///
/// Returns the entry's VID on success and `None` when the world refuses or cannot be
/// reached, because in both cases this client must not go on playing in a world it is
/// not part of.
///
/// The VID is returned rather than stored, so the caller records it after the
/// immutable borrow of `held.character` this call needs has ended.
///
/// `items` are the ones the load placed at character select. The world places them as it
/// admits the character, so it never holds this character with an inventory the client
/// has not been shown.
async fn join_the_world(
    context: &ConnectionContext,
    addr: SocketAddr,
    character: &db::players::Character,
    vid: u32,
    items: Vec<(ItemPos, Item)>,
    outbox: prodomo::client_registry::ClientOutbox,
) -> Option<common::vid::Vid> {
    let world_vid = common::vid::Vid::new(vid);
    let entered = context
        .game
        .enter_world_with_items(
            world_vid,
            character.id,
            character.name.clone(),
            items,
            outbox,
        )
        .await;
    match entered {
        Ok(Ok(())) => {
            info!(
                %addr,
                vid,
                name = %character.name.as_str(),
                "Character entered the world on the game thread"
            );
            Some(world_vid)
        }
        // A refusal means the world already holds this VID, player id, or Name.
        // Continuing would put two clients on one identity, so the descriptor closes:
        // legacy refuses the same way when `PlayerLoad` finds a character already in
        // game (`G/input_db.cpp`, "already exist in game").
        Ok(Err(error)) => {
            error!(
                %addr,
                vid,
                name = %character.name.as_str(),
                %error,
                "The world refused this character; closing"
            );
            None
        }
        // No answer at all means the game thread is gone. The world cannot be
        // reached, so this client must not play in it.
        Err(error) => {
            error!(
                %addr,
                vid,
                name = %character.name.as_str(),
                %error,
                "The world could not be reached; closing"
            );
            None
        }
    }
}

/// Write the character's row, the way `CHARACTER::SaveReal` does.
///
/// Returns `false` when the character could not be written, which is either legacy's own
/// refusal (`m_bSkipSave`, or no descriptor) or a store error. The caller logs; nothing here
/// reports success to a client, because legacy's write goes to the DB process and the game loop
/// never learns whether it landed.
///
/// The playtime is accumulated from [`SaveEvent::play_start`], not from the session length, so a
/// save at ninety seconds banks the minute a save at thirty seconds did not. Legacy's remainder
/// carry is what makes that work, and [`prodomo::save::playtime`] is the rule.
async fn save_held(context: &ConnectionContext, held: &Held, at: std::time::Instant) -> bool {
    let (Some(character), Some(account), Some(avatar)) = (
        held.character.as_ref(),
        held.account.as_ref(),
        held.avatar.as_ref(),
    ) else {
        return false;
    };
    let Some(event) = held.save.as_ref() else {
        return false;
    };
    if prodomo::save::judge_save_real(false, true) != prodomo::save::SaveRealOutcome::Write {
        return false;
    }
    // `get_dword_time() - m_dwPlayStartTime` is an unsigned 32-bit millisecond count
    // (`server/server/libthecore/utils.cpp:467-472`), so an elapsed time that overflowed a
    // 32-bit `DWORD` wraps in legacy and wraps here. `Instant` cannot go backwards, so this is
    // the one value that is clamped rather than wrapped: a negative elapsed time is not a time
    // travel, it is a clock this Rewrite does not have.
    let elapsed = u32::try_from(at.saturating_duration_since(event.play_start).as_millis())
        .unwrap_or(u32::MAX);
    let save = prodomo::save::player_save(
        character,
        prodomo::save::SavePosition {
            x: avatar.x,
            y: avatar.y,
        },
        prodomo::save::playtime(character.playtime_minutes, elapsed),
    );
    match db::players::save_character(&context.store, account.id, character.id, &save).await {
        Ok(true) => true,
        Ok(false) => {
            warn!(
                player = character.id,
                account = account.id.get(),
                "Save reached no character row"
            );
            false
        }
        Err(error) => {
            warn!(player = character.id, %error, "Could not save the character");
            false
        }
    }
}

/// The next fire of the save event, or nothing at all when the character is not in the game.
///
/// The arm in the descriptor loop borrows `held.save`, so it cannot be a `tokio::time::Interval`
/// held in the select itself. Before `enter_game` the event does not exist in legacy either, and
/// a descriptor that never reaches the game has no row to write.
async fn save_tick(save: Option<&mut SaveEvent>) {
    match save {
        Some(event) => {
            event.interval.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// The `bLanguage` byte a Channel descriptor carries.
///
/// `DESC::GetLanguage` returns `m_accountTable.bLanguage` (`G/desc.h:181`), and the account table
/// is zeroed by `DESC::Setup`, so a descriptor that has not been through character creation
/// carries 0. `SetLanguage` has exactly one caller, the character-creation path
/// (`G/char.cpp:11641`). The Rewrite ports only the `europe` Locale (`CONTEXT.md`), whose Game
/// data is the one the legacy deployment loaded for that byte, so 0 is the only value this port
/// can produce and the only one a first-login descriptor would carry in legacy either.
const LANGUAGE_EUROPE: u8 = ELocale::Ymir as u8;

/// Send each record in order as its own frame; `false` when the connection must close.
async fn send_all<S>(
    session: &mut LiveClientSession<S>,
    records: &[Vec<u8>],
) -> Result<(), LiveError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for record in records {
        session.send(record).await?;
    }
    Ok(())
}

/// The records a select-screen handler sends, or why the connection closes.
type SelectReplies = Result<Vec<Vec<u8>>, String>;

/// `EMPIRE`: choose the account's empire and send `GC_EMPIRE` and the character list again
/// (`CInputLogin::Empire` and `CInputDB::EmpireSelect`).
async fn choose_empire(
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    account: &mut Option<SelectAccount>,
) -> SelectReplies {
    let record = CgEmpire::decode_frame(frame).map_err(|error| error.to_string())?;
    let (empire, start) = match judge_empire(account.as_ref(), record.empire) {
        EmpireVerdict::Close => return Err(format!("empire {} does not exist", record.empire)),
        EmpireVerdict::Ignore => return Ok(Vec::new()),
        EmpireVerdict::Select { empire, start } => (empire, start),
    };
    let Some(account) = account.as_mut() else {
        return Ok(Vec::new());
    };
    // The store refuses when another descriptor already settled the account.
    if !select_empire(&context.store, account.id, empire, start)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(Vec::new());
    }
    empire_selected(&mut account.lobby, empire, start);
    let mut shown = Vec::with_capacity(GcHeaderAndByte::WIRE_SIZE);
    GcHeaderAndByte::new(HEADER_GC_EMPIRE.value(), empire).encode_into(&mut shown);
    let success = login_success(
        &account.lobby,
        &context.atlas,
        seat.locations,
        seat.handle,
        random_key(),
    )
    .encode();
    info!(account = account.id.get(), empire, "Empire chosen");
    Ok(vec![shown, success])
}

/// `CHARACTER_CREATE`: create a character in the store and list it
/// (`CInputLogin::CharacterCreate`, `CClientManager::__QUERY_PLAYER_CREATE`, and
/// `CInputDB::PlayerCreateSuccess`).
async fn create_character(
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
    account: &mut Option<SelectAccount>,
) -> SelectReplies {
    let record = CgPlayerCreate::decode_frame(frame).map_err(|error| error.to_string())?;
    let Some(account) = account.as_mut() else {
        return Ok(vec![create_failure(CREATE_REFUSED)]);
    };
    let offset = (create_offset(random_key()), create_offset(random_key()));
    let player = match judge_create(
        &context.names,
        context.block_char_creation,
        account,
        &record,
        offset,
    ) {
        Ok(player) => player,
        Err(failure) => return Ok(vec![create_failure(failure)]),
    };
    // The DB server checks the per-account cooldown before anything else.
    let now = Instant::now();
    if context.creates.cooling(account.id, now) {
        return Ok(vec![create_failure(CREATE_REFUSED)]);
    }
    let id = match create_player(&context.store, account.id, &player).await {
        Ok(Created::Player(id)) => id,
        Ok(Created::Taken) => return Ok(vec![create_failure(CREATE_TAKEN)]),
        Err(AccountError::NoSuchAccountId(_)) => return Ok(vec![create_failure(CREATE_REFUSED)]),
        Err(error) => return Err(error.to_string()),
    };
    context.creates.created(account.id, now);
    info!(
        account = account.id.get(),
        player = id,
        slot = player.slot,
        "Character created"
    );
    Ok(vec![created(
        &mut account.lobby,
        &player,
        id,
        &context.atlas,
        seat.locations,
    )])
}

/// `CHARACTER_DELETE`: delete a character from the store (`CInputLogin::CharacterDelete`,
/// `CClientManager::__QUERY_PLAYER_DELETE`, and `CInputDB::PlayerDeleteSuccess`).
async fn delete_character(
    context: &ConnectionContext,
    frame: &ClientFrame,
    account: &mut Option<SelectAccount>,
) -> SelectReplies {
    let record = CgPlayerDelete::decode_frame(frame).map_err(|error| error.to_string())?;
    let verdict = judge_delete(account.as_ref(), record.index);
    let Some(account) = account.as_mut() else {
        return Ok(Vec::new());
    };
    let player = match verdict {
        DeleteVerdict::Ignore => return Ok(Vec::new()),
        DeleteVerdict::Refuse => return Ok(vec![deleted(&mut account.lobby, record.index, false)]),
        DeleteVerdict::Ask { player } => player,
    };
    let (level_limit, level_limit_lower) = context.delete_levels;
    let delete = PlayerDelete {
        slot: record.index,
        player,
        code: &record.private_code,
        level_limit,
        level_limit_lower,
    };
    let done = delete_player(&context.store, account.id, &delete)
        .await
        .map_err(|error| error.to_string())?;
    info!(
        account = account.id.get(),
        player, done, "Character delete answered"
    );
    Ok(vec![deleted(&mut account.lobby, record.index, done)])
}

/// `CHANGE_NAME`: give a character the new Name it was asked to choose
/// (`CInputLogin::ChangeName` and `CInputDB::ChangeName`).
async fn rename_character(
    context: &ConnectionContext,
    frame: &ClientFrame,
    account: &mut Option<SelectAccount>,
) -> SelectReplies {
    let record = CgChangeName::decode_frame(frame).map_err(|error| error.to_string())?;
    let (player, name) = match judge_rename(&context.names, account.as_ref(), &record) {
        RenameVerdict::Ignore => return Ok(Vec::new()),
        RenameVerdict::Close => return Err(format!("slot {} holds no character", record.index)),
        RenameVerdict::Refuse => return Ok(vec![create_failure(CREATE_REFUSED)]),
        RenameVerdict::Ask { player, name } => (player, name),
    };
    let Some(account) = account.as_mut() else {
        return Ok(Vec::new());
    };
    if !change_name(&context.store, account.id, player, &name)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(vec![create_failure(CREATE_TAKEN)]);
    }
    info!(account = account.id.get(), player, "Character renamed");
    Ok(renamed(&mut account.lobby, player, &name)
        .into_iter()
        .collect())
}

/// Send a Channel login refusal; `false` when the connection must close.
async fn refuse_channel_login<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    refusal: ChannelRefusal,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    info!(%addr, ?refusal, "Channel login refused");
    match session
        .send(&GcLoginFailure::new(refusal.status()).encode())
        .await
    {
        Ok(_) => true,
        Err(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

/// Answer `STATE_CHECKER` with the Channel status list; `false` when the connection must close.
///
/// Legacy asks the DB server and ignores a second request while one is pending; the answer here
/// is immediate, so none is ever pending.
async fn send_channel_status<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = context.channels.respond(context.state.is_shutting_down());
    match session.send(&record).await {
        Ok(_) => {
            info!(%addr, "Sent the Channel status list");
            true
        }
        Err(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

/// Answer `LOGIN3` with `AUTH_SUCCESS` or `LOGIN_FAILURE`; `false` when the connection must
/// close. A refusal keeps the connection open, as in legacy.
async fn auth_login<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
    claim: &mut Option<AuthClaim>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgLogin3::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed LOGIN3; closing");
            return false;
        }
    };
    let reply = match authenticate(context, &record).await {
        Ok(Ok((key, new_claim))) => {
            info!(%addr, login = %new_claim.login().as_str(), "Auth login accepted");
            *claim = Some(new_claim);
            GcAuthSuccess::new(key, 1).encode()
        }
        Ok(Err(refusal)) => {
            info!(%addr, ?refusal, "Auth login refused");
            GcLoginFailure::new(refusal.status()).encode()
        }
        Err(error) => {
            error!(%addr, %error, "Auth login failed; closing");
            return false;
        }
    };
    match session.send(&reply).await {
        Ok(_) => true,
        Err(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

/// The legacy auth checks in their legacy order: `CInputAuth::Login`, then the
/// `QID_AUTH_LOGIN` result (`G/db.cpp:246-460`). The outer error is a store or hashing failure.
async fn authenticate(
    context: &ConnectionContext,
    record: &CgLogin3,
) -> Result<Result<(u32, AuthClaim), AuthRefusal>, Box<dyn Error + Send + Sync>> {
    let Some(login) = login_from_field(&record.login) else {
        return Ok(Err(AuthRefusal::NoId));
    };
    if context.shutdowned || context.state.is_shutting_down() {
        return Ok(Err(AuthRefusal::Shutdown));
    }
    if context.auth.is_held(&login) {
        return Ok(Err(AuthRefusal::Already));
    }
    let Some(account) = find_auth_account(&context.store, &login).await? else {
        return Ok(Err(AuthRefusal::NoId));
    };
    let digest = account.password.clone();
    let candidate = password_candidate(&record.passwd).to_vec();
    let matches = tokio::task::spawn_blocking(move || digest.verify(&candidate)).await??;
    if let Err(refusal) = judge_credentials(&account, matches) {
        return Ok(Err(refusal));
    }
    let new_claim = match context.auth.claim(&login) {
        Ok(new_claim) => new_claim,
        Err(refusal) => return Ok(Err(refusal)),
    };
    if let Err(refusal) = judge_account(&account, record.b_language, &context.block_login) {
        return Ok(Err(refusal));
    }
    record_login(&context.store, account.id, record.b_language).await?;
    let key = context.auth.grant(LoginGrant {
        account: account.id,
        login,
        client_key: record.adw_client_key,
        language: record.b_language,
    });
    Ok(Ok((key, new_claim)))
}

/// Log one handled step; `false` when the connection must close.
fn report_step(addr: SocketAddr, step: LiveStep) -> bool {
    match step {
        LiveStep::Handled(LiveOutcome::KeepAlive { .. }) => {
            info!(%addr, "Received client keepalive");
            true
        }
        LiveStep::Handled(LiveOutcome::Pong { .. }) => {
            info!(%addr, "Received client pong");
            true
        }
        LiveStep::Handled(LiveOutcome::Handshake { phase, .. }) => {
            info!(%addr, ?phase, "Received client handshake");
            true
        }
        LiveStep::Pending { .. } => true,
        LiveStep::Record { phase, frame } => {
            warn!(
                %addr,
                ?phase,
                header = frame.header,
                "Client sent a header no analyzer handles; closing"
            );
            false
        }
        LiveStep::PeerClosed { .. } => false,
        LiveStep::Unsupported(unsupported) => {
            warn!(%addr, %unsupported, "Client sent a header no analyzer handles; closing");
            false
        }
        LiveStep::Failed(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

fn begin_shutdown(state: &ServerState, shutdown_tx: &broadcast::Sender<()>) {
    if state.is_shutting_down() {
        return;
    }
    info!("Initiating graceful shutdown sequence");
    state.initiate_shutdown();
    drop(shutdown_tx.send(()));
}

/// The OS shutdown-signal handlers, already installed.
///
/// Installing a handler is the point of this type. `tokio::signal::unix::signal`
/// registers the handler when the stream is *created*, so [`arm_shutdown_signal`]
/// must run before anything observable, such as a bound listener, can let a
/// caller believe the process is ready.
struct ShutdownSignal {
    /// SIGTERM. Only the unix build has a real one.
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    /// SIGINT, registered the same way rather than through `ctrl_c`.
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
}

/// Install the shutdown-signal handlers without waiting for a signal.
///
/// # Errors
///
/// Returns an error if the runtime refuses to register a handler. This is fatal
/// on purpose: a process that cannot handle SIGTERM cannot shut down cleanly.
fn arm_shutdown_signal() -> Result<ShutdownSignal, Box<dyn Error>> {
    #[cfg(unix)]
    {
        let terminate = signal::unix::signal(signal::unix::SignalKind::terminate())
            .map_err(|error| format!("Failed to install SIGTERM handler: {error}"))?;
        let interrupt = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .map_err(|error| format!("Failed to install SIGINT handler: {error}"))?;
        Ok(ShutdownSignal {
            terminate,
            interrupt,
        })
    }

    #[cfg(not(unix))]
    {
        Ok(ShutdownSignal {})
    }
}

impl ShutdownSignal {
    /// Wait for SIGTERM or SIGINT.
    ///
    /// # Errors
    ///
    /// Returns an error if the signal stream fails.
    async fn wait(self) -> Result<(), Box<dyn Error>> {
        #[cfg(unix)]
        {
            let mut terminate = self.terminate;
            let mut interrupt = self.interrupt;
            tokio::select! {
                _ = terminate.recv() => info!("Received SIGTERM"),
                _ = interrupt.recv() => info!("Received SIGINT (Ctrl+C)"),
            }
            Ok(())
        }

        #[cfg(not(unix))]
        {
            signal::ctrl_c()
                .await
                .map_err(|error| format!("Failed to install Ctrl+C handler: {error}"))?;
            info!("Received SIGINT (Ctrl+C)");
            Ok(())
        }
    }
}

struct ServerContext {
    state: Arc<ServerState>,
    shutdown_tx: broadcast::Sender<()>,
    connection: ConnectionContext,
}

enum AcceptLoopExit {
    Signal,
    GameLoop(GameLoopTerminal),
}
/// What the world is given once the store is ready.
///
/// These two travel together because they are the same moment: the item id allocator is
/// what makes a grant possible, and the Operator console is how a grant is asked for, so
/// one installed without the other gives an Operator a console that refuses everything.
struct WorldStartup {
    /// The span the world's allocator hands ids out of.
    item_id_span: ItemIdSpan,
    /// The named pipe the Operator console reads, or `None` to leave the console off.
    console_path: Option<PathBuf>,
}

/// Read the world's readiness inputs out of the configuration.
fn world_startup(config: &ServerConfig) -> WorldStartup {
    WorldStartup {
        item_id_span: config.game.item_id_range,
        console_path: config.game.operator_console.clone(),
    }
}

/// Create the Operator console's pipe and start its reader, or report why not.
///
/// The pipe is created here, in the accept loop, rather than inside the reader task, so a
/// path this server cannot create is one line at startup instead of a reader that retries
/// forever without saying why. The `None` return is the real off switch: the first draft
/// logged the failure and started the reader anyway, so the message said "the console will
/// stay off" while the console was very much on, pointed at a regular file whose contents
/// the tail loop then replayed for as long as the server ran.
async fn start_operator_console(
    path: &Path,
    store: &Store,
    game_loop: &mut GameLoopHandle,
    context: &ServerContext,
) -> Option<tokio::task::JoinHandle<()>> {
    if let Err(error) = prodomo::operator_console::prepare(path).await {
        // Not fatal, and the console really does stay off. Clients matter more than a
        // console, and a server that refuses to start over an Operator's pipe trades a
        // convenience for an outage.
        error!(
            %error,
            path = %path.display(),
            "Operator console is configured but its pipe is unusable; the console will \
             stay off"
        );
        return None;
    }
    info!(path = %path.display(), "Operator console pipe ready");

    // The path, the store, the controller, and the shutdown receiver are all taken by
    // value, so the task owns everything it reads and borrows nothing from this frame.
    let path = path.to_path_buf();
    let store = store.clone();
    let controller = game_loop.controller();
    let shutdown = context.shutdown_tx.subscribe();
    Some(tokio::spawn(async move {
        prodomo::operator_console::run(
            &path,
            prodomo::operator_console::ConsoleContext { store, controller },
            shutdown,
        )
        .await;
    }))
}

/// Bring the store's schema up to date, retrying while the server is unreachable.
///
/// The ports stay open and the ready gate closed meanwhile, so a database that
/// comes up after the server is picked up without a restart. Any other error,
/// such as a refused password or a migration edited after it was applied, is
/// fatal: retrying would only hide it.
async fn prepare_store(store: &Store) -> Result<(), Box<dyn Error>> {
    let mut pause = STORE_RETRY_FIRST;
    loop {
        match store.migrate().await {
            Ok(()) => {
                info!("Store ready at schema version {}", schema_version());
                return Ok(());
            }
            Err(error) if error.is_transient() => {
                warn!(%error, retry_in = ?pause, "Store unreachable; retrying");
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(STORE_RETRY_MAX);
            }
            Err(error) => return Err(format!("Store unusable: {error}").into()),
        }
    }
}

async fn run_accept_loop(
    listeners: &mut Listeners,
    context: &ServerContext,
    game_loop: &mut GameLoopHandle,
    shutdown_signal: ShutdownSignal,
    store: &Store,
    ready_gate: &ReadyGate,
    startup: &WorldStartup,
) -> Result<AcceptLoopExit, Box<dyn Error>> {
    // Boxed and pinned once, not per `select!` iteration: the wait future owns
    // the signal streams, so recreating it each pass would drop them and lose a
    // signal that arrived between accepts.
    let mut shutdown = Box::pin(shutdown_signal.wait());
    // Pinned once for the same reason: a pass that recreated it would restart
    // the retry pause from the beginning after every refused client.
    let mut store_ready = Box::pin(prepare_store(store));
    let mut ready = false;
    // Background tasks started by this loop, so the console is shut down with the
    // server rather than left reading a pipe nobody owns.
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    let exit = 'run: loop {
        tokio::select! {
            signal_result = &mut shutdown => {
                signal_result?;
                break 'run AcceptLoopExit::Signal;
            }
            terminal = game_loop.wait_for_terminal() => {
                break 'run AcceptLoopExit::GameLoop(terminal?);
            }
            prepared = &mut store_ready, if !ready => {
                prepared?;
                // The world's item id allocator is installed before the gate opens,
                // and not before: its start id is a fact about the stored items, so
                // it cannot be known until the schema is up and the table is
                // readable. Opening the gate without it would admit clients onto a
                // world that refuses every grant, which is the failure this ordering
                // exists to prevent.
                install_item_ids(game_loop, store, startup.item_id_span).await?;
                ready = true;
                ready_gate.open();
                info!("Accepting clients");

                // The Operator console starts HERE, and not earlier, for the same
                // reason the gate does not open earlier: the world's item id allocator
                // is installed one line above, and a console that accepted a grant
                // before it would refuse every one of them with `IdsExhausted` or a
                // missing allocator. A console that is up but refuses everything is
                // worse than one that is not up, because it looks like the grant failed.
                if let Some(console_path) = &startup.console_path {
                    if let Some(task) =
                        start_operator_console(console_path, store, game_loop, context).await
                    {
                        info!(
                            path = %console_path.display(),
                            "Operator console task started; write a command with \
                             `echo 'item give <name> <vnum> [count]' > <path>`"
                        );
                        tasks.push(task);
                    }
                }
            }
            (role, result) = listeners.accept() => match result {
                Ok((stream, addr)) if !context.state.should_accept_connections() => {
                    warn!(%addr, %role, "Rejecting new connection while shutting down");
                    drop(stream);
                }
                // The ready gate is checked before the handler is spawned, not
                // inside it, so an unready server does not allocate a
                // per-connection task for every client that arrives.
                Ok((stream, addr)) if !ready_gate.admit() => {
                    warn!(
                        %addr,
                        %role,
                        refused = ready_gate.refused(),
                        "Refusing client connection before startup has finished"
                    );
                    drop(stream);
                }
                Ok((stream, addr)) => {
                    let connection_shutdown = context.shutdown_tx.clone();
                    let connection = context.connection.clone();
                    tokio::spawn(async move {
                        handle_connection(stream, addr, role, connection, &connection_shutdown)
                            .await;
                    });
                }
                Err(error) => error!(%error, %role, "Failed to accept connection"),
            }
        }
    };

    // The console is a background task, and dropping its `JoinHandle` would leave it
    // running: the pipe stays open and the process would not exit. It is aborted
    // explicitly, and the abort is acknowledged, because an Operator looking at a
    // console that went quiet needs to know the server is the reason.
    for task in tasks {
        task.abort();
        match task.await {
            Err(error) if error.is_cancelled() => {
                info!("Operator console task stopped with the server");
            }
            Err(error) => warn!(%error, "Operator console task join failed"),
            Ok(()) => info!("Operator console task finished before the server stopped"),
        }
    }
    Ok(exit)
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve { verbose } => serve(&cli.config, verbose).await,
        Command::Account { command } => {
            operate(&cli.config, &OperatorCommand::Account(command)).await
        }
        Command::Gm { command } => operate(&cli.config, &OperatorCommand::Gm(command)).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `prodomo account ...` and `prodomo gm ...`.
///
/// Everything that can be checked without the store is checked, and the password read, before
/// the store is opened. The schema is migrated first, once and without retrying: an Operator at
/// the terminal is better served by the error.
async fn operate(config_path: &Path, command: &OperatorCommand) -> Result<(), Box<dyn Error>> {
    let config = load_server_config(config_path)
        .map_err(|error| format!("Failed to load config: {error}"))?;
    let prepared = prepare(command, read_new_password)?;
    let store = Store::connect(&StoreConfig::from(&config.store))
        .await
        .map_err(|error| format!("Store unavailable: {error}"))?;
    let outcome: Result<(), Box<dyn Error>> = match store.migrate().await {
        Ok(()) => prepared
            .execute(&store, &mut io::stdout().lock())
            .await
            .map_err(Into::into),
        Err(error) => Err(format!("Store unusable: {error}").into()),
    };
    store.close().await;
    outcome
}

/// Each Channel's maps, and the Shared Channel's port and maps.
struct MapRoutes {
    channel_maps: HashMap<u8, Vec<u32>>,
    shared: Option<(u16, Vec<u32>)>,
}

/// Load the map regions the Channel login places characters with.
fn load_atlas(config: &ServerConfig) -> Result<MapAtlas, String> {
    let map_dir = config.map_dir();
    let atlas = MapAtlas::load(&map_dir)
        .map_err(|error| format!("Failed to load the maps in {}: {error}", map_dir.display()))?;
    info!(regions = atlas.regions().len(), dir = %map_dir.display(), "Map regions loaded");
    Ok(atlas)
}

/// Load the item prototypes, which the grant path and the item load need.
///
/// They are read before any port opens, for the same reason as the atlas: a missing or
/// malformed file is a startup failure rather than a grant that fails a minute later. The
/// one table is shared by the game thread, for a grant, and by a descriptor, for the item
/// load at character select.
fn load_item_protos(config: &ServerConfig) -> Result<Arc<ItemProtos>, String> {
    let proto_dir = config.proto_dir();
    let protos = ItemProtos::load(&proto_dir).map_err(|error| {
        format!(
            "Item prototypes are unusable in {}: {error}",
            proto_dir.display()
        )
    })?;
    info!(prototypes = protos.rows().len(), "Item prototypes loaded");
    Ok(Arc::new(protos))
}

/// Load the Name rules: the banned words of the Game data tables and the mob names of the protos.
fn load_name_rules(config: &ServerConfig) -> Result<NameRules, String> {
    let dump_path = config.game_tables.join("player.sql");
    let dump = std::fs::read(&dump_path)
        .map_err(|error| format!("Failed to read {}: {error}", dump_path.display()))?;
    let banwords = banwords_from_dump(&dump).map_err(|error| {
        format!(
            "Failed to read the banwords in {}: {error}",
            dump_path.display()
        )
    })?;
    let proto_dir = config.proto_dir();
    let mobs = MobNames::load(&proto_dir)
        .map_err(|error| format!("Failed to load the mob names: {error}"))?;
    info!(banwords = banwords.len(), "Name rules loaded");
    Ok(NameRules::new(banwords, mobs))
}

/// Read the map routes from the configuration and the bound listeners.
fn map_routes(config: &ServerConfig, listeners: &Listeners) -> MapRoutes {
    let channel_maps = config
        .channels
        .iter()
        .map(|channel| (channel.number, channel.maps.clone()))
        .collect();
    // The Shared Channel's maps are reached through its first port.
    let shared = config
        .channels
        .iter()
        .find(|channel| channel.is_shared())
        .and_then(|channel| {
            let port = listeners
                .iter()
                .find(|listener| listener.role() == ListenerRole::Channel(channel.number))?
                .local_addr()
                .port();
            Some((port, channel.maps.clone()))
        });
    MapRoutes {
        channel_maps,
        shared,
    }
}

/// The Game data a connection reads, loaded before any port opens.
struct GameData {
    /// The map regions.
    atlas: MapAtlas,
    /// The Names a character may not take.
    names: NameRules,
    /// The item prototypes, the same table the game thread holds.
    protos: Arc<ItemProtos>,
}

/// What every connection shares, built once the listeners are bound and the Game data loaded.
fn connection_context(
    config: &ServerConfig,
    state: &Arc<ServerState>,
    store: &Store,
    listeners: &Listeners,
    data: GameData,
    game: GameLoopController,
) -> ConnectionContext {
    // Every legacy game Core reported its own client port (`mother_port`) to the status list; the
    // auth server reported nothing (`G/desc_client.cpp:286-290`).
    let channel_ports: Vec<u16> = listeners
        .iter()
        .filter(|listener| matches!(listener.role(), ListenerRole::Channel(_)))
        .map(|listener| listener.local_addr().port())
        .collect();
    ConnectionContext {
        clock: BootLiveClock::new(),
        ping_cycle: Duration::from_secs(u64::from(config.game.ping_event_second_cycle)),
        // `ServerConfig::validate` has already refused a zero cycle, so this cannot be `None`
        // for a process that started; it is `expect`ed rather than defaulted so a caller that
        // builds a context by hand cannot get a busy loop.
        save_cycle: prodomo::save::event_period(
            config.game.save_event_second_cycle,
            prodomo::save::PASSES_PER_SEC,
        )
        .expect("[game] save_event_second_cycle was refused by validate"),
        state: Arc::clone(state),
        channels: Arc::new(ChannelStatusBoard::new(channel_ports, &config.game)),
        store: store.clone(),
        auth: AuthRegistry::new(),
        shutdowned: config.game.shutdowned,
        block_login: Arc::from(config.game.block_login.as_str()),
        user_limit: config.game.user_limit,
        logons: LogonRegistry::new(),
        atlas: Arc::new(data.atlas),
        protos: data.protos,
        public_ip: config.public_ip,
        routes: Arc::new(map_routes(config, listeners)),
        handles: Arc::new(AtomicU32::new(0)),
        names: Arc::new(data.names),
        clients: Arc::new(ChannelClients::new()),
        positions: Arc::new(PositionTable::new()),
        creates: Arc::new(CreateCooldown::default()),
        block_char_creation: config.game.block_char_creation,
        delete_levels: (
            config.game.player_delete_level_limit,
            config.game.player_delete_level_limit_lower,
        ),
        game,
    }
}

/// Resolves the world's item id range against the store and hands it to the game
/// thread.
///
/// Every failure here stops the server instead of opening the gate. A range the
/// store cannot resolve, a range the world refuses, and a lost command all leave a
/// world that answers no grant, and a server that admits clients and cannot give
/// anything out is worse than one that does not start.
async fn install_item_ids(
    game_loop: &mut GameLoopHandle,
    store: &Store,
    span: ItemIdSpan,
) -> Result<(), Box<dyn Error>> {
    let resolved = db::items::resolve_item_id_range(store, span)
        .await
        .map_err(|error| format!("Item id range {span} is unusable: {error}"))?;
    let range = world_item_id_range(resolved)
        .map_err(|error| format!("Item id range {span} is unusable: {error}"))?;
    info!(
        first = range.first,
        last = range.last,
        first_usable = range.first_usable,
        "Item id range resolved"
    );
    match game_loop.controller().install_item_ids(range).await? {
        Ok(()) => Ok(()),
        Err(error) => Err(format!("The world refused the item id range: {error}").into()),
    }
}

/// `prodomo serve`.
async fn serve(config_path: &Path, verbose: bool) -> Result<(), Box<dyn Error>> {
    let config = initialize_server(config_path, verbose)?;
    let state = Arc::new(ServerState::new());
    let (shutdown_tx, _) = broadcast::channel::<()>(16);

    // Arm the shutdown signal BEFORE any listener exists.
    //
    // `tokio::signal::unix::signal` installs its handler when the stream is
    // created, and the kernel keeps the default disposition until then. If a
    // listener were bound first, a process could accept a connection (and a
    // test could observe readiness) in the window before the handler exists, and
    // a SIGTERM in that window would kill the process outright instead of
    // shutting it down cleanly. Arming first makes "the port is open" imply
    // "SIGTERM is handled".
    let shutdown_signal = arm_shutdown_signal()?;

    // The pool is built without connecting, so a malformed URL fails here,
    // before any port opens, and a database that is down does not stop the
    // listeners from coming up.
    let store = Store::lazy(&StoreConfig::from(&config.store))
        .map_err(|error| format!("Invalid store configuration: {error}"))?;
    info!(store = ?config.store, "Store configured; no connection opened yet");

    // The map regions are Game data the Channel login needs; a client is not accepted before
    // they are loaded, so a missing or malformed file stops the server before any port opens.
    let atlas = load_atlas(&config)?;
    let names = load_name_rules(&config)?;

    let protos = load_item_protos(&config)?;

    let mut listeners = Listeners::bind(&listener_plan(&config)).await?;
    for listener in listeners.iter() {
        info!(
            "Listening for {} clients on {}",
            listener.role(),
            listener.local_addr()
        );
    }
    // The client ports open before the world is ready, as legacy's does
    // (`main.cpp:671`). The ready gate keeps that ordering from admitting
    // clients onto a world whose store is not ready, which legacy does not do.
    // The gate opens in the accept loop once the schema is up to date.
    let ready_gate = ReadyGate::closed();

    // The world is built here and moved into the game thread. It is built without an
    // item id allocator on purpose: the start id is `MAX(id)` over the item table, and
    // that table is only readable once the store has migrated, which happens inside
    // the accept loop below. The allocator arrives as a command from there.
    let game_state =
        GameState::new(Arc::clone(&protos)).with_item_count_limit(config.game.item_count_limit);
    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), game_state)?;
    let controller = game_loop.controller();
    info!(thread_id = ?game_loop.thread_id(), "Dedicated game loop started");
    info!("Waiting for the store before accepting clients");

    let context = ServerContext {
        state: Arc::clone(&state),
        shutdown_tx,
        connection: connection_context(
            &config,
            &state,
            &store,
            &listeners,
            GameData {
                atlas,
                names,
                protos,
            },
            controller.clone(),
        ),
    };
    let exit = run_accept_loop(
        &mut listeners,
        &context,
        &mut game_loop,
        shutdown_signal,
        &store,
        &ready_gate,
        &world_startup(&config),
    )
    .await;
    ready_gate.close();
    drop(listeners);
    begin_shutdown(&context.state, &context.shutdown_tx);
    let exit = match exit {
        Ok(exit) => exit,
        Err(run_error) => {
            if let Err(error) = controller.request_stop().await {
                warn!(%error, "Game loop stop command was not accepted during error cleanup");
            }
            match game_loop.join().await {
                Ok(terminal) => {
                    info!(?terminal, "Game loop error cleanup acknowledged");
                    info!("Game loop thread joined during error cleanup");
                }
                Err(error) => error!(%error, "Game loop error cleanup failed"),
            }
            store.close().await;
            info!("Store closed");
            return Err(run_error);
        }
    };
    let terminal = match exit {
        AcceptLoopExit::Signal => {
            if let Err(error) = controller.request_stop().await {
                warn!(%error, "Game loop stop command was not accepted");
            }
            game_loop.wait_for_terminal().await?
        }
        AcceptLoopExit::GameLoop(terminal) => terminal,
    };
    info!(?terminal, "Game loop stop acknowledged");
    let terminal = game_loop.join().await?;
    info!("Game loop thread joined");
    store.close().await;
    info!("Store closed");
    info!("Server shutdown complete");

    match terminal {
        GameLoopTerminal::Stopped(_) => Ok(()),
        GameLoopTerminal::Failed { reason, .. } => {
            Err(format!("Game loop terminated with failure: {reason:?}").into())
        }
    }
}
