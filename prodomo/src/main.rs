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
use common::config::{load_server_config, ServerConfig, DEFAULT_CONFIG_PATH};
use common::logging::{init_from_env, init_logging, LogConfig};
use db::accounts::{find_auth_account, record_login, AccountError};
use db::players::{
    change_name, create_player, delete_player, lobby, select_empire, Created, PlayerDelete,
};
use db::store::{schema_version, Store, StoreConfig};
use gamedata::banword::banwords_from_dump;
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
use prodomo::client_live::{
    handshake_token, BootLiveClock, LiveClientSession, LiveClock, LiveOutcome, LiveStep,
};
use prodomo::client_session::ClientPhase;
use prodomo::game_loop::{spawn_game_loop, GameLoopConfig, GameLoopHandle};
use prodomo::game_loop_messages::GameLoopTerminal;
use prodomo::handshake::HandshakeServerKind;
use prodomo::lifecycle::PostHandshakePhase;
use prodomo::listeners::{listener_plan, ListenerRole, Listeners};
use prodomo::operator::{prepare, read_new_password, AccountCommand, GmCommand, OperatorCommand};
use prodomo::ready_gate::ReadyGate;
use prodomo::select_phase::{
    create_failure, create_offset, created, deleted, empire_selected, judge_create, judge_delete,
    judge_empire, judge_rename, renamed, CreateCooldown, DeleteVerdict, EmpireVerdict, NameRules,
    RenameVerdict, SelectAccount, CREATE_REFUSED, CREATE_TAKEN,
};
use prodomo::ServerState;
use protocol::cg_account::{CgLoginByKey, CgPlayerCreate, CgPlayerDelete};
use protocol::cg_inventory::{
    HEADER_CG_CHANGE_NAME, HEADER_CG_CHARACTER_CREATE, HEADER_CG_CHARACTER_DELETE,
    HEADER_CG_EMPIRE, HEADER_CG_LOGIN2, HEADER_CG_LOGIN3, HEADER_CG_STATE_CHECKER,
};
use protocol::cg_login::CgEmpire;
use protocol::cg_login3::CgLogin3;
use protocol::cg_name::CgChangeName;
use protocol::cg_wire::ClientFrame;
use protocol::gc::{GcAuthSuccess, GcLoginFailure};
use protocol::gc_inventory::HEADER_GC_EMPIRE;
use protocol::gc_small::GcHeaderAndByte;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

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
    /// The address clients are told to reconnect to.
    public_ip: Ipv4Addr,
    /// The maps each Channel hosts, and the Shared Channel's.
    routes: Arc<MapRoutes>,
    /// The last descriptor handle given out (legacy `DESC_MANAGER` handle count).
    handles: Arc<AtomicU32>,
    /// The Names a character may not take.
    names: Arc<NameRules>,
    /// When each account last created a character.
    creates: Arc<CreateCooldown>,
    /// `[game] block_char_creation`.
    block_char_creation: bool,
    /// `[game] player_delete_level_limit` and `player_delete_level_limit_lower`.
    delete_levels: (i32, i32),
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
    let mut ping = tokio::time::interval_at(
        tokio::time::Instant::now() + context.ping_cycle,
        context.ping_cycle,
    );
    let mut held = Held::default();

    loop {
        match session.next_buffered(clock.now()).await {
            Ok(Some(step)) => {
                let open = analyze(
                    &mut session,
                    addr,
                    &context,
                    locations
                        .as_ref()
                        .map(|locations| ChannelSeat { locations, handle }),
                    step,
                    &mut held,
                )
                .await;
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
            _ = ping.tick() => {
                if let Err(error) = session.tick(clock.now()).await {
                    warn!(%addr, %error, "Client session stopped");
                    break;
                }
                if session.phase() == ClientPhase::Close {
                    info!(%addr, "Client did not answer the ping; closing");
                    break;
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
        }
    }
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
        step => report_step(addr, step),
    }
}

/// What a Channel descriptor knows about its seat: where each map is served from its port, and
/// its handle.
struct ChannelSeat<'a> {
    locations: &'a MapLocations,
    handle: u32,
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
) -> Result<AcceptLoopExit, Box<dyn Error>> {
    // Boxed and pinned once, not per `select!` iteration: the wait future owns
    // the signal streams, so recreating it each pass would drop them and lose a
    // signal that arrived between accepts.
    let mut shutdown = Box::pin(shutdown_signal.wait());
    // Pinned once for the same reason: a pass that recreated it would restart
    // the retry pause from the beginning after every refused client.
    let mut store_ready = Box::pin(prepare_store(store));
    let mut ready = false;

    loop {
        tokio::select! {
            signal_result = &mut shutdown => {
                signal_result?;
                return Ok(AcceptLoopExit::Signal);
            }
            terminal = game_loop.wait_for_terminal() => {
                return Ok(AcceptLoopExit::GameLoop(terminal?));
            }
            prepared = &mut store_ready, if !ready => {
                prepared?;
                ready = true;
                ready_gate.open();
                info!("Accepting clients");
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
    }
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

/// What every connection shares, built once the listeners are bound and the Game data loaded.
fn connection_context(
    config: &ServerConfig,
    state: &Arc<ServerState>,
    store: &Store,
    listeners: &Listeners,
    atlas: MapAtlas,
    names: NameRules,
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
        state: Arc::clone(state),
        channels: Arc::new(ChannelStatusBoard::new(channel_ports, &config.game)),
        store: store.clone(),
        auth: AuthRegistry::new(),
        shutdowned: config.game.shutdowned,
        block_login: Arc::from(config.game.block_login.as_str()),
        user_limit: config.game.user_limit,
        logons: LogonRegistry::new(),
        atlas: Arc::new(atlas),
        public_ip: config.public_ip,
        routes: Arc::new(map_routes(config, listeners)),
        handles: Arc::new(AtomicU32::new(0)),
        names: Arc::new(names),
        creates: Arc::new(CreateCooldown::default()),
        block_char_creation: config.game.block_char_creation,
        delete_levels: (
            config.game.player_delete_level_limit,
            config.game.player_delete_level_limit_lower,
        ),
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

    let mut game_loop = spawn_game_loop(GameLoopConfig::default(), |_| {})?;
    let controller = game_loop.controller();
    info!(thread_id = ?game_loop.thread_id(), "Dedicated game loop started");
    info!("Waiting for the store before accepting clients");

    let context = ServerContext {
        state: Arc::clone(&state),
        shutdown_tx,
        connection: connection_context(&config, &state, &store, &listeners, atlas, names),
    };
    let exit = run_accept_loop(
        &mut listeners,
        &context,
        &mut game_loop,
        shutdown_signal,
        &store,
        &ready_gate,
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
