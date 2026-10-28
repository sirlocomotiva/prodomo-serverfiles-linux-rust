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
use common::enums::EParts;
use common::logging::{init_from_env, init_logging, LogConfig};
use common::point_slot;
use db::accounts::{find_auth_account, record_login, AccountError, AccountId};
use db::players::{
    change_name, create_player, delete_player, load_character, lobby, select_empire, Character,
    Created, PlayerDelete, PLAYER_SLOTS,
};
use db::store::{schema_version, Store, StoreConfig};
use gamedata::banword::banwords_from_dump;
use gamedata::item_proto::ItemProtos;
use gamedata::locale_string::{Ending, LocaleStrings, LOCALE_COUNT};
use gamedata::map_atlas::{MapAtlas, MapRegion};
use gamedata::mob_group::MobGroups;
use gamedata::mob_locale_names::{
    ItemNamesByLanguage, MobLocaleNames, MobNamesByLanguage, ITEM_NAMES_FILE,
};
use gamedata::mob_names::MobNames;
use gamedata::mob_proto::MobProtos;
use gamedata::npc_shop::{shops_from_dump, NpcShops};
use gamedata::pc_motion::PcMotions;
use gamedata::regen::{self, RegenEntry};
use gamedata::server_attr::{self, SectreeGrid};
use prodomo::auth_login::{
    judge_account, judge_credentials, login_from_field, password_candidate, AuthClaim, AuthRefusal,
    AuthRegistry, LoginGrant,
};
use prodomo::channel_login::{
    admit, empire_shown, judge_key, login_success, random_key, ChannelRefusal, LogonClaim,
    LogonRegistry, MapLocations,
};
use prodomo::channel_status::ChannelStatusBoard;
use prodomo::chat::{
    judge_chat, ChatContext, ChatEffect, ChatOutcome, ChatState, WhisperCounter, CHAT_GUILD,
    CHAT_PARTY,
};
use prodomo::chat_line::Recipient;
use prodomo::client_live::{
    handshake_token, BootLiveClock, LiveClientSession, LiveClock, LiveError, LiveOutcome, LiveStep,
};
use prodomo::client_registry::{ChannelClients, ClientEntry, ClientOrder, Lease};
use prodomo::client_session::ClientPhase;
use prodomo::command::{
    interpret, one_argument, Caller, Command as LineCommand, CommandFlood, Interpreted, GM_PLAYER,
    POS_DEAD, POS_SITTING, POS_STANDING, TYPE_IN_FULL,
};
use prodomo::game_loop::{spawn_game_loop, GameLoopConfig, GameLoopHandle};
use prodomo::game_loop_messages::{
    Departure, GameLoopController, GameLoopTerminal, GroundPlace, Kept, RelayScope, RestartKind,
    Settled, Shown, WhisperOutcome,
};
use prodomo::game_state::{
    world_item_id_range, GameState, ShopAnswer, ShopStep, TradeAnswer, TradeSettled, TradeStep,
};
use prodomo::game_state::{
    GuildCommentAnswer, GuildCommentCommit, GuildCommentLine, GuildGradeAnswer, GuildGradeCommit,
    GuildGradeSet, GuildJoinAnswer, GuildJoinCommit, GuildMakeAnswer, GuildMakeCommit,
    GuildQuestStore, GuildRemoveAnswer, GuildRemoveCommit, GuildSkillUseAnswer,
    GuildSkillUseCommit, GUILD_COMMENT_MAX_COUNT,
};
use prodomo::game_state::{QuestStep, Quests, SafeboxAnswer, SafeboxStep};
use prodomo::handshake::HandshakeServerKind;
use prodomo::item_load::plan_item_load;
use prodomo::item_move::{MoveItemRefused, Mover};
use prodomo::lifecycle::{LifecycleError, PostHandshakePhase};
use prodomo::listeners::{listener_plan, ListenerRole, Listeners};
use prodomo::loading_phase::{
    enter_game_burst, entering_position, item_load_points, judge_enter_game, judge_select,
    load_points, loading_burst, map_is_allowed, name_field, public_map_index, EnterGameBurst,
    EnterGameVerdict, EnteringPosition, LoadingBurst, Neighbourhood, PcCard, SelectVerdict,
    PHASE_DEAD, PHASE_GAME, PK_MODE_PEACE,
};
use prodomo::movement::{judge_pose, PoseOutcome};
use prodomo::operator::{prepare, read_new_password, AccountCommand, GmCommand, OperatorCommand};
use prodomo::quickslot::QuickslotStep;
use prodomo::ready_gate::ReadyGate;
use prodomo::select_phase::{
    create_failure, create_offset, created, deleted, empire_selected, judge_create, judge_delete,
    judge_empire, judge_rename, renamed, CreateCooldown, DeleteVerdict, EmpireVerdict, NameRules,
    RenameVerdict, SelectAccount, CREATE_REFUSED, CREATE_TAKEN,
};
use prodomo::ServerState;
use protocol::cg_account::{
    CgEnterGame, CgLoginByKey, CgPlayerCreate, CgPlayerDelete, CgPlayerSelect,
};
use protocol::cg_attack::CgAttack;
use protocol::cg_chat::CgChat;
use protocol::cg_exchange::CgExchange;
use protocol::cg_fly_target::CgFlyTarget;
use protocol::cg_guild_answer::CgAnswerMakeGuild;
use protocol::cg_inventory::{
    HEADER_CG_ADD_FLY_TARGETING, HEADER_CG_ANSWER_MAKE_GUILD, HEADER_CG_ATTACK,
    HEADER_CG_CHANGE_NAME, HEADER_CG_CHARACTER_CREATE, HEADER_CG_CHARACTER_DELETE,
    HEADER_CG_CHARACTER_POSITION, HEADER_CG_CHARACTER_SELECT, HEADER_CG_CHAT, HEADER_CG_EMPIRE,
    HEADER_CG_ENTERGAME, HEADER_CG_EXCHANGE, HEADER_CG_FLY_TARGETING, HEADER_CG_GUILD,
    HEADER_CG_GUILD_SYMBOL_UPLOAD, HEADER_CG_ITEM_DROP, HEADER_CG_ITEM_DROP2, HEADER_CG_ITEM_MOVE,
    HEADER_CG_ITEM_PICKUP, HEADER_CG_ITEM_USE, HEADER_CG_LOGIN2, HEADER_CG_LOGIN3,
    HEADER_CG_MARK_CRCLIST, HEADER_CG_MARK_IDXLIST, HEADER_CG_MARK_LOGIN, HEADER_CG_MARK_UPLOAD,
    HEADER_CG_MESSENGER, HEADER_CG_MOVE, HEADER_CG_ON_CLICK, HEADER_CG_PARTY_INVITE,
    HEADER_CG_PARTY_INVITE_ANSWER, HEADER_CG_PARTY_PARAMETER, HEADER_CG_PARTY_REMOVE,
    HEADER_CG_PARTY_SET_STATE, HEADER_CG_PARTY_USE_SKILL, HEADER_CG_QUEST_INPUT_STRING,
    HEADER_CG_QUICKSLOT_ADD, HEADER_CG_QUICKSLOT_DEL, HEADER_CG_QUICKSLOT_SWAP,
    HEADER_CG_SCRIPT_ANSWER, HEADER_CG_SCRIPT_BUTTON, HEADER_CG_SHOOT, HEADER_CG_SHOP,
    HEADER_CG_STATE_CHECKER, HEADER_CG_SYMBOL_CRC, HEADER_CG_SYNC_POSITION, HEADER_CG_TARGET,
    HEADER_CG_USE_SKILL, HEADER_CG_WARP, HEADER_CG_WHISPER,
};
use protocol::cg_item_drop::CgItemDrop;
use protocol::cg_item_drop2::CgItemDrop2;
use protocol::cg_item_move::CgItemMove;
use protocol::cg_item_pickup::CgItemPickup;
use protocol::cg_item_use::CgItemUse;
use protocol::cg_login::CgEmpire;
use protocol::cg_login3::CgLogin3;
use protocol::cg_mark::{CgMarkCRCList, CgMarkIdxList, CgMarkLogin, CgMarkUpload, CgSymbolCrc};
use protocol::cg_micro::{CgScriptAnswer, CgWarp};
use protocol::cg_move::CgMove;
use protocol::cg_name::CgChangeName;
use protocol::cg_party::{CgPartyInvite, CgPartyInviteAnswer, CgPartyRemove};
use protocol::cg_party::{CgPartyParameter, CgPartySetState};
use protocol::cg_party_skill::CgPartyUseSkill;
use protocol::cg_pick::{CgScriptButton, CgTarget};
use protocol::cg_position::CgCharacterPosition;
use protocol::cg_quest_text::CgQuestInputString;
use protocol::cg_quickslot_add::CgQuickslotAdd;
use protocol::cg_quickslot_del::CgQuickslotDel;
use protocol::cg_quickslot_swap::CgQuickslotSwap;
use protocol::cg_safebox::{CgSafeBoxItem, SafeBoxKind};
use protocol::cg_safebox_move::CgSafeboxItemMove;
use protocol::cg_shoot::CgShoot;
use protocol::cg_shop::CgShop;
use protocol::cg_use_skill::CgUseSkill;
use protocol::cg_variable::GuildSymbolUpload;
use protocol::cg_vid::CgOnClick;
use protocol::cg_whisper::{CgWhisper, CgWhisperError};
use protocol::cg_wire::ClientFrame;
use protocol::gc::{GcAuthSuccess, GcLoginFailure};
use protocol::gc_chat::{
    CHAT_TYPE_BIG_NOTICE, CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_NOTICE,
};
use protocol::gc_inventory::{HEADER_GC_EMPIRE, HEADER_GC_PHASE};
use protocol::gc_mark::{mark_block_record, mark_idxlist_record, symbol_data_record};
use protocol::gc_small::GcHeaderAndByte;
use protocol::gc_whisper::{GcWhisper, WHISPER_TYPE_GM, WHISPER_TYPE_NORMAL};
use protocol::item_pos::ItemPos;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use world::cells::{HostedCells, MapCells};
use world::character::MoveRequest;
use world::item::Item;
use world::npc::{NpcSpawner, NpcVids, Respawner};

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

/// `guild_mark_server` and `guild_mark_min_level`: the handshake gate and the upload floor
/// are one struct so [`ConnectionContext`] keeps three bools.
#[derive(Clone, Copy)]
struct MarkServer {
    /// Whether this process answers the mark downloader.
    serves: bool,
    /// `[game] guild_mark_min_level`, clamped to `GUILD_MAX_LEVEL` at parse.
    min_level: u8,
}

/// What the server refuses: the login and the character creation.
#[derive(Clone, Copy)]
struct Refusals {
    /// `[game] shutdowned`: new logins are refused.
    shutdowned: bool,
    /// `[game] block_char_creation`: no new character may be created.
    block_char_creation: bool,
}

/// How the server treats every character: how far a shout reaches and who is a game master.
#[derive(Clone, Copy)]
struct Modes {
    /// `[game] enable_global_shout`: every empire hears a shout, not only the shouter's.
    global_shout: bool,
    /// `[game] test_server`: the test-server mode, which makes every character a game
    /// master (`CHARACTER::IsGM`, `G/char.cpp:6373-6380`).
    test_server: bool,
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
    /// `[game] shutdowned`: new logins are refused, and `[game] block_char_creation`: no new
    /// character may be created.
    refusals: Refusals,
    /// `[game] block_login`.
    block_login: Arc<str>,
    /// `[game] user_limit`.
    user_limit: i32,
    /// `guild_mark_server` plus `[game] guild_mark_min_level` (`G/config.cpp:912`), one
    /// field so the flag and the level cannot drift apart: the flag gates the handshake
    /// (`G/input.cpp:213`) and the upload reads the level unconditionally
    /// (`G/input_login.cpp:1071-1073`).
    mark_server: MarkServer,
    /// The logins held by Channel descriptors.
    logons: Arc<LogonRegistry>,
    /// The map regions of the legacy Game data.
    atlas: Arc<MapAtlas>,
    /// The cell attributes of every hosted map, which `Entergame` places a character by.
    cells: Arc<HostedCells>,
    /// The item prototypes, shared with the game thread, which the item load at character
    /// select reads for each item's size and flags.
    protos: Arc<ItemProtos>,
    /// The locale strings every language's chat lines are looked up in, shared with the game
    /// thread.
    locale: Arc<LocaleStrings>,
    /// The address clients are told to reconnect to.
    public_ip: Ipv4Addr,
    /// The maps each Channel hosts, and the Shared Channel's.
    routes: Arc<MapRoutes>,
    /// The last descriptor handle given out (legacy `DESC_MANAGER` handle count).
    handles: Arc<AtomicU32>,
    /// The Names a character may not take.
    names: Arc<NameRules>,
    /// The per-Channel client set that shouts go through and Warp orders reach a descriptor
    /// by (legacy `DESC_MANAGER::GetClientSet`).
    clients: Arc<ChannelClients>,
    /// When each account last created a character.
    creates: Arc<CreateCooldown>,
    /// `[game] player_delete_level_limit` and `player_delete_level_limit_lower`.
    delete_levels: (i32, i32),
    /// `[game] shout_limit_level`: the level a character needs to shout.
    shout_limit_level: i32,
    /// `[game] enable_global_shout`: every empire hears a shout, not only the shouter's, and
    /// `[game] test_server`: the test-server mode, which makes every character a game master
    /// (`CHARACTER::IsGM`, `G/char.cpp:6373-6380`).
    modes: Modes,
    /// `[game] pk_protect_level`: below it a character is in the protect PK mode.
    pk_protect_level: u8,
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
    /// The points of that character (legacy `CHARACTER::m_points` and `m_pointsInstant`),
    /// computed when the load answered.
    points: Option<world::character::Points>,
    /// The quickslots of that character (legacy `m_quickslot`): the ones the load set, then
    /// the ones the world's last quickslot answer left, which the save writes.
    quickslots: world::character::Quickslots,
    /// The nonzero skill slots the load read (`player_skill`), which the world
    /// holds as the character's levels from the admission on.
    skills: Vec<db::player_skills::StoredSkill>,
    /// The quest flags the load read (`quest_flag`), which the world holds as the
    /// character's quest record from the admission on, and which the save writes back.
    quest_flags: Vec<(Vec<u8>, i32)>,
    /// The saved affects the load read (`player_affect`), which the world holds as the
    /// character's timed affects from the admission on, and which the save writes back.
    affects: Vec<db::player_affects::StoredAffect>,
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
    /// `m_pulseCommandFlood` and `m_iCommandFloodCount` of the interpreter.
    command_flood: CommandFlood,
    /// The character's GM level (`m_pointsInstant.gm_level`), read once at the
    /// character load (`SetPlayerProto`, `G/input_db.cpp:1931-1955`) and held for the
    /// descriptor's life. It gates the GM commands and lifts the flood limit
    /// (`interpret_command`, `G/cmd.cpp:611-616`).
    gm_level: u8,
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
    /// When the character was selected, which legacy's `m_dwLastSkillTime` starts at.
    selected_at: Option<tokio::time::Instant>,
    /// `m_dwLastItemDropTime`: when a drop last passed the drop limit
    /// (`G/char_item.cpp:7465`), whatever the rest of the drop then did.
    last_item_drop: Option<tokio::time::Instant>,
    /// `m_dwLastGoldDropTime`: when a gold drop last passed its check
    /// (`G/char_item.cpp:7570`). Legacy's limit `g_GoldDropTimeLimitValue` is compiled in
    /// as 0 (`G/questmanager.cpp:33`), so the check never fires.
    /// `m_lWarpMapIndex` and `m_posWarp`: the destination a `WarpSet` stored, which the save
    /// writes instead of the position (`G/char.cpp:1551-1565`).
    warp: Option<prodomo::warp::WarpTarget>,
    /// `m_posExit` and `m_lExitMapIndex`: the place the character returns to when it leaves
    /// a private map, which the save writes beside the position (`SaveExitLocation`,
    /// `G/char.cpp:6679-6683`). A character that never left a public map has none.
    exit: Option<prodomo::save::ExitLocation>,
    /// Whether this descriptor has sent `GC_WARP`. Its character is saved at the destination
    /// and released by then, so the descriptor only waits for the client to close, dropping
    /// whatever the client still sends.
    departed: bool,
    /// The last move or relay handed to the world, until the world has run it. The records it
    /// sent this client go out before the next frame is analyzed, as legacy writes them
    /// before it reads that frame.
    settling: Option<Settled>,
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

/// The slice of `CHARACTER` the descriptor keeps: the save's copy of the place, and the pose.
///
/// The world owns the body, which moves, syncs and is seen there. `map`, `x` and `y` start at
/// the entering place and take the body's place each time the world hands back what it holds
/// ([`hold_kept`]), so a save writes the point the world last had. `sitting` starts false
/// because a loaded character is always standing.
#[derive(Debug, Clone, PartialEq)]
struct Avatar {
    /// `GetMapIndex()`.
    map: i32,
    /// `GetX()`, as the world last reported it.
    x: i32,
    /// `GetY()`, as the world last reported it.
    y: i32,
    /// `IsState(POS_SITTING)`.
    sitting: bool,
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
    leave_the_world(&context, addr, &mut held, Departure::Disconnect).await;
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
    }
    // `held` drops here, which releases the lease and deregisters the character.
    drop(held);
}

/// Take this descriptor's character out of the game thread's world, if it is in it, and hold
/// the points and quickslots the world kept, which the save then writes.
///
/// A disconnect reaches here (`DESC::SetPlayer(NULL)`), and so does a `WarpSet`, whose
/// `RemoveEntity` takes the character out of its sectree before `GC_WARP` is sent
/// (`G/char.cpp:6749-6755`).
async fn leave_the_world(
    context: &ConnectionContext,
    addr: SocketAddr,
    held: &mut Held,
    departure: Departure,
) {
    let Some(world_vid) = held.world.take() else {
        return;
    };
    match context.game.leave_world(world_vid, departure).await {
        // The world's points and quickslots are the ones the save writes: the potion
        // recovery changes the points on the world's pulse, after the last step this
        // descriptor held, and a trade the partner closed the quickslots.
        Ok(Some(departed)) => {
            hold_kept(held, departed);
            info!(%addr, vid = world_vid.raw(), "Character left the world");
        }
        // A leave that found nobody is a descriptor that never entered, which the
        // `Option` already rules out, so this is a world that lost the character
        // some other way. The disconnect continues either way.
        Ok(None) => {
            warn!(%addr, vid = world_vid.raw(), "The world did not hold this character at disconnect");
        }
        // A leave with no answer means the game thread is gone. The process is
        // losing the world with it, so the descriptor still closes.
        Err(error) => {
            warn!(%addr, vid = world_vid.raw(), %error, "The world could not be told this character left");
        }
    }
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
    // The game thread's orders to this client are lent out the same way.
    let mut orders: Option<mpsc::UnboundedReceiver<ClientOrder>> = None;
    let mut pending: Vec<Vec<u8>> = Vec::new();

    loop {
        lend_queues(held, &mut outbox, &mut orders);
        // A move or relay this client sent is waited for, so the records the world sent this
        // client for it are queued before anything a later frame answers directly. Then the
        // queue is written, with whatever other characters caused meanwhile, before the next
        // frame is analyzed: a record never sits behind an answer to a frame sent after it.
        settle(held).await;
        if !flush_turn(session, addr, &mut outbox, &mut pending, held).await {
            break;
        }
        match session.next_buffered(clock.now()).await {
            // A descriptor that sent `GC_WARP` has released its character, so there is
            // nothing left for a record to act on. Legacy would still analyze it against the
            // character it keeps until the close; the Rewrite drops it, which is recorded as
            // a Divergence of the departure.
            Ok(Some(_)) if held.departed => continue,
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
            () = save_tick(held.save.as_mut()) => save_on_tick(context, addr, held).await,
            _ = shutdown_rx.recv() => {
                info!(%addr, "Client session stopping for shutdown");
                break;
            }
            () = logon_kicked(held.logon.as_ref()) => {
                // `DESC::DisconnectOfSameLogin` without a character: `SetPhase(PHASE_CLOSE)`.
                info!(%addr, "Another client logged in with this login; closing");
                break;
            }
            // An order from the game thread: a warp or goto NPC this character stands by.
            order = next_order(&mut orders) => {
                if let Some(order) = order {
                    let follow = async |session: &mut LiveClientSession<S>, held: &mut Held| {
                        follow_order(session, addr, context, locations, held, order).await
                    };
                    if !take_order(session, addr, &mut outbox, &mut pending, held, follow).await {
                        break;
                    }
                } else {
                    // As for the broadcast queue: the lease is held, so the queue closed.
                    info!(%addr, "Order queue closed; no more orders");
                    orders = None;
                }
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

/// `CHARACTER::save_event` (`G/char.cpp:5070-5082`): the character is queued, and the row is
/// written on the next drain. There is no game loop to drain here, so this writes the row the
/// drain would have written.
async fn save_on_tick(context: &ConnectionContext, addr: SocketAddr, held: &mut Held) {
    if let Some(event) = held.save.as_mut() {
        event.queued = true;
    }
    hold_world_points(context, addr, held).await;
    if save_held(context, &*held, std::time::Instant::now()).await {
        info!(%addr, "Save event wrote the character row");
    }
}

/// Take the lease's queues back from the previous turn and lend them out again for this turn's
/// select. A lease the analyzer created still has its queues in place, so the takes hand those
/// straight over; a descriptor with no lease lends nothing.
fn lend_queues(
    held: &mut Held,
    outbox: &mut Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    orders: &mut Option<mpsc::UnboundedReceiver<ClientOrder>>,
) {
    let Some(lease) = held.presence.as_mut() else {
        *outbox = None;
        *orders = None;
        return;
    };
    if let Some(inbox) = outbox.take() {
        lease.put_receiver(inbox);
    }
    *outbox = lease.take_receiver();
    if let Some(queue) = orders.take() {
        lease.put_orders(queue);
    }
    *orders = lease.take_orders();
}

/// Wait until the world has run the last move or relay this client sent, so every record it
/// sent this client is in the lease's queue.
async fn settle(held: &mut Held) {
    if let Some(settling) = held.settling.take() {
        // An error only says the world dropped the command unrun, so it sent nothing.
        let _ran = settling.await;
    }
}

/// Write the pending records and then every record already in the lease's queue, in order,
/// without waiting for more; `false` when the session stopped.
async fn flush_outbox<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    outbox: &mut Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    pending: &mut Vec<Vec<u8>>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(queue) = outbox.as_mut() {
        while let Ok(record) = queue.try_recv() {
            pending.push(record);
        }
    }
    write_pending(session, addr, pending).await
}

/// [`flush_outbox`], then stand the player up when a record it wrote changed the phase (a
/// death or a restart, see [`stand_on_phase_change`]); `false` when the session stopped.
async fn flush_turn<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    outbox: &mut Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    pending: &mut Vec<Vec<u8>>,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let before = session.phase();
    let written = flush_outbox(session, addr, outbox, pending).await;
    if written {
        stand_on_phase_change(held, before, session.phase());
    }
    written
}

/// Write what the world queued before an order, then follow it, and stand the player up when
/// either changed the phase; `false` when the session stopped or the connection must close.
///
/// The world queued those records before it ordered (a goto NPC's show earlier in the same
/// turn, say), so they go out before what following the order writes, as legacy writes them
/// before `WarpSet` runs.
async fn take_order<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    outbox: &mut Option<mpsc::UnboundedReceiver<Vec<u8>>>,
    pending: &mut Vec<Vec<u8>>,
    held: &mut Held,
    follow: impl AsyncFnOnce(&mut LiveClientSession<S>, &mut Held) -> bool,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let before = session.phase();
    if !flush_outbox(session, addr, outbox, pending).await || !follow(session, held).await {
        return false;
    }
    stand_on_phase_change(held, before, session.phase());
    true
}

/// Write the broadcast records queued during the last turn; `false` when the session stopped.
///
/// A `GC_PHASE` record the world wrote, at a player's death or restart, is `DESC::SetPhase`:
/// the session writes the same record through the boundary it had and installs the phase, so
/// the records after it are read under the new phase in the order the world wrote them.
async fn write_pending<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    pending: &mut Vec<Vec<u8>>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for record in pending.drain(..) {
        let written = match phase_of_record(&record) {
            Some(phase) => session.set_phase(phase).await.map(drop),
            None => session.send(&record).await.map(drop),
        };
        if let Err(error) = written {
            warn!(%addr, %error, "Client session stopped while writing a broadcast");
            return false;
        }
    }
    true
}

/// The phase a `GC_PHASE` record of the world switches the descriptor to: `PHASE_DEAD` at a
/// death and `PHASE_GAME` at a restart. Any other record is `None`.
fn phase_of_record(record: &[u8]) -> Option<PostHandshakePhase> {
    let &[header, phase] = record else {
        return None;
    };
    if header != HEADER_GC_PHASE.value() {
        return None;
    }
    match phase {
        PHASE_DEAD => Some(PostHandshakePhase::Dead),
        PHASE_GAME => Some(PostHandshakePhase::Game),
        _ => None,
    }
}

/// `SetPosition(POS_DEAD)` at a death and `SetPosition(POS_STANDING)` at a restart, which come
/// with the phase the world switched the descriptor to: either leaves the player not sitting.
fn stand_on_phase_change(held: &mut Held, before: ClientPhase, after: ClientPhase) {
    if before == after {
        return;
    }
    if let Some(avatar) = held.avatar.as_mut() {
        avatar.sitting = false;
    }
}

/// Hold a character's points, and the level, experience, counters, pools and parts the save
/// writes from the held row.
fn hold_points(held: &mut Held, points: world::character::Points) {
    if let Some(held_character) = held.character.as_mut() {
        held_character.level = points.level();
        held_character.exp = points.exp();
        // The counters live in `smallint` columns; a level-up adds at most tens per level,
        // so the narrowing only guards a row a hand wrote past the slot.
        held_character.random_hp = i16::try_from(points.random_hp()).unwrap_or(i16::MAX);
        held_character.random_sp = i16::try_from(points.random_sp()).unwrap_or(i16::MAX);
        held_character.stat_point =
            i16::try_from(points.get_point(point_slot::POINT_STAT)).unwrap_or(i16::MAX);
        held_character.skill_point =
            i16::try_from(points.get_point(point_slot::POINT_SKILL)).unwrap_or(i16::MAX);
        held_character.sub_skill_point =
            i16::try_from(points.get_point(point_slot::POINT_SUB_SKILL)).unwrap_or(i16::MAX);
        held_character.level_step =
            i16::try_from(points.get_point(point_slot::POINT_LEVEL_STEP)).unwrap_or(i16::MAX);
        held_character.hp = points.hp();
        held_character.sp = points.sp();
        held_character.stamina = points.stamina();
        let parts = points.parts();
        held_character.main_part = parts[EParts::Main as usize];
        held_character.hair_part = parts[EParts::Hair as usize];
        held_character.sash_part = parts[EParts::Sash as usize];
    }
    held.points = Some(points);
}

/// Hold the points, quickslots, quest flags and place the world holds for a character.
fn hold_kept(held: &mut Held, kept: Kept) {
    if let Some(points) = kept.points {
        hold_points(held, points);
    }
    if let Some(held_character) = held.character.as_mut() {
        held_character.alignment = kept.alignment;
    }
    held.quickslots = kept.quickslots;
    held.quest_flags = kept.quest_flags;
    held.affects = kept.affects;
    if let (Some((map, x, y)), Some(avatar)) = (kept.place, held.avatar.as_mut()) {
        avatar.map = map;
        avatar.x = x;
        avatar.y = y;
    }
}

/// Hold the points, quickslots and place the world holds for this descriptor's character, which
/// the save writes from. The world changes them on its own (the potion recovery on its pulse, a
/// trade the partner closes, every move), so the copy the last item step left can be stale. A world that
/// cannot answer leaves the held copy, which is still a state the character was in.
async fn hold_world_points(context: &ConnectionContext, addr: SocketAddr, held: &mut Held) {
    let Some(vid) = held.world else {
        return;
    };
    match context.game.kept_of(vid).await {
        Ok(Some(kept)) => hold_kept(held, kept),
        Ok(None) => {}
        Err(error) => warn!(%addr, %error, "The world could not be asked for the points to save"),
    }
}

/// How long after an attack, or after the select, an item may not be worn: 1.5 s
/// (`G/char_item.cpp:8418-8419`).
const EQUIP_AFTER_FIGHT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Whether the character was selected within the last 1.5 s: `m_dwLastSkillTime`
/// (`G/char_item.cpp:8418-8419`). The Rewrite has no skills yet, so the skill time is only
/// ever the select's. The world adds `GetLastAttackTime`, which its moves keep.
fn recently_fought(held: &Held) -> bool {
    held.selected_at
        .is_some_and(|at| at.elapsed() <= EQUIP_AFTER_FIGHT)
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
    held: &mut Held,
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
    let actor = item_actor(held);
    let answer = context.game.move_item(vid, request, actor).await;
    finish_item_step(session, addr, context, held, actor, answer).await
}

/// `CG_ITEM_USE` (11) in the game phase: `CInputMain::ItemUse` → `CHARACTER::UseItem`
/// (`G/char_item.cpp:7168`). Only an equippable item is ported: it goes on, or comes off.
///
/// What the world did reaches the store and the client as a move's does.
async fn use_an_item<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgItemUse::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ITEM_USE; closing");
            return false;
        }
    };
    let Some(vid) = held.world else {
        info!(%addr, "ITEM_USE without a character in the world; ignoring");
        return true;
    };
    let actor = item_actor(held);
    let answer = context.game.use_item(vid, record.cell, actor).await;
    // A book read climbed a slot: store it as `Save()` writes the skills with the row.
    // The store runs before the item rows, so a failure closes without a half-written step.
    if let Ok(Ok(moved)) = &answer {
        if let Some((vnum, level, next_read)) = moved.climbed {
            let Some(character) = held.character.as_ref() else {
                return true;
            };
            if let Err(error) =
                db::player_skills::store_skill(&context.store, character.id, vnum, level, next_read)
                    .await
            {
                warn!(%addr, %error, vnum, level, "The book read could not be stored; closing");
                return false;
            }
            info!(%addr, vnum, level, "Skill read from a book");
        }
    }
    finish_item_step(session, addr, context, held, actor, answer).await
}

/// Whether `header` is `CG_ITEM_USE`, a ground step or a quickslot request.
fn is_belongings_step(header: u8) -> bool {
    header == HEADER_CG_ITEM_USE.value() || is_ground_step(header) || is_quickslot_step(header)
}

/// Run a step [`is_belongings_step`] accepts:
///
/// - `CG_ITEM_USE` (11): `CInputMain::ItemUse` (`G/input_main.cpp:993`).
/// - `CG_ITEM_DROP` (12), `CG_ITEM_DROP2` (20) and `CG_ITEM_PICKUP` (15), reached when the
///   character is not an observer (`G/input_main.cpp:3693-3717`).
/// - `CG_QUICKSLOT_ADD` (16), `CG_QUICKSLOT_DEL` (17) and `CG_QUICKSLOT_SWAP` (18), which no
///   observer check guards (`G/input_main.cpp:3742-3755`).
async fn belongings_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if frame.header == HEADER_CG_ITEM_USE.value() {
        use_an_item(session, addr, context, held, frame).await
    } else if is_quickslot_step(frame.header) {
        change_a_quickslot(session, addr, context, held, frame).await
    } else if frame.header == HEADER_CG_ITEM_PICKUP.value() {
        pick_up_an_item(session, addr, context, held, frame).await
    } else {
        drop_an_item(session, addr, context, held, frame).await
    }
}

/// Whether `header` is one of the three quickslot requests.
fn is_quickslot_step(header: u8) -> bool {
    [
        HEADER_CG_QUICKSLOT_ADD,
        HEADER_CG_QUICKSLOT_DEL,
        HEADER_CG_QUICKSLOT_SWAP,
    ]
    .iter()
    .any(|known| known.value() == header)
}

/// `CG_QUICKSLOT_ADD`, `CG_QUICKSLOT_DEL` and `CG_QUICKSLOT_SWAP` in the game phase:
/// `CInputMain::QuickslotAdd`, `QuickslotDelete` and `QuickslotSwap`
/// (`G/input_main.cpp:1083-1126`). The world changes the slots and answers the records for
/// this client, and the descriptor holds the slots the save writes.
async fn change_a_quickslot<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let decoded = if frame.header == HEADER_CG_QUICKSLOT_ADD.value() {
        CgQuickslotAdd::decode_frame(frame)
            .map(|record| QuickslotStep::Add {
                slot: record.pos,
                quickslot: world::character::Quickslot {
                    kind: record.slot.b_type,
                    pos: record.slot.b_pos,
                },
            })
            .map_err(|error| error.to_string())
    } else if frame.header == HEADER_CG_QUICKSLOT_DEL.value() {
        CgQuickslotDel::decode_frame(frame)
            .map(|record| QuickslotStep::Del { slot: record.pos })
            .map_err(|error| error.to_string())
    } else {
        CgQuickslotSwap::decode_frame(frame)
            .map(|record| QuickslotStep::Swap {
                slot: record.pos,
                with: record.change_pos,
            })
            .map_err(|error| error.to_string())
    };
    let step = match decoded {
        Ok(step) => step,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed quickslot record; closing");
            return false;
        }
    };
    let Some(vid) = held.world else {
        info!(%addr, "Quickslot request without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.quickslot(vid, step).await {
        Ok(Some(answer)) => answer,
        Ok(None) => {
            warn!(%addr, "The world holds no character for this descriptor; closing");
            return false;
        }
        Err(error) => {
            error!(%addr, %error, "The world could not be reached; closing");
            return false;
        }
    };
    held.quickslots = answer.quickslots;
    if let Err(error) = send_all(session, &answer.records).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
}

/// `CG_ITEM_DROP` (12) and `CG_ITEM_DROP2` (20) in the game phase: `CInputMain::ItemDrop` and
/// `ItemDrop2` (`G/input_main.cpp:1024-1051`) → `CHARACTER::DropItem` (`G/char_item.cpp:7444`),
/// or `CHARACTER::DropGold` (`:7554`) when the gold field is set.
///
/// `CG_ITEM_DROP` carries no count, which `DropItem` reads as the whole stack. The drop limit
/// is checked here, because its clock is the descriptor's, and passing it restarts the clock
/// whatever the world then answers, as legacy's does. A gold drop skips it: the limit's clock
/// is legacy's item-drop clock only, and the gold check's own limit is 0.
async fn drop_an_item<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let decoded = if frame.header == HEADER_CG_ITEM_DROP2.value() {
        CgItemDrop2::decode_frame(frame)
            .map(|record| (record.cell, record.gold, record.count))
            .map_err(|error| error.to_string())
    } else {
        CgItemDrop::decode_frame(frame)
            .map(|record| (record.cell, record.gold, 0))
            .map_err(|error| error.to_string())
    };
    let (cell, gold, count) = match decoded {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ITEM_DROP; closing");
            return false;
        }
    };
    let (Some(vid), Some(place)) = (held.world, ground_place(held)) else {
        info!(%addr, "ITEM_DROP without a character in the world; ignoring");
        return true;
    };
    if gold > 0 {
        let actor = item_actor(held);
        held.last_item_drop = Some(tokio::time::Instant::now());
        let answer = context.game.drop_gold(vid, gold, place, actor).await;
        return finish_item_step(session, addr, context, held, actor, answer).await;
    }
    let actor = item_actor(held);
    let now = tokio::time::Instant::now();
    let elapsed = held.last_item_drop.map(|last| now.duration_since(last));
    if !prodomo::item_move::drop_allowed(elapsed) {
        info!(%addr, "Item drop inside the drop limit");
        let line = prodomo::item_move::drop_limit_notice(actor.recipient(&context.locale));
        if let Err(error) = send_all(session, &[line]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
        return true;
    }
    held.last_item_drop = Some(now);
    let answer = context.game.drop_item(vid, cell, count, place, actor).await;
    finish_item_step(session, addr, context, held, actor, answer).await
}

/// `CG_ITEM_PICKUP` (15) in the game phase: `CInputMain::ItemPickup`
/// (`G/input_main.cpp:1076`) → `CHARACTER::PickupItem` (`G/char_item.cpp:7972`).
async fn pick_up_an_item<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgItemPickup::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ITEM_PICKUP; closing");
            return false;
        }
    };
    let (Some(vid), Some(place)) = (held.world, ground_place(held)) else {
        info!(%addr, "ITEM_PICKUP without a character in the world; ignoring");
        return true;
    };
    let actor = item_actor(held);
    let answer = context
        .game
        .pickup_item(vid, record.vid, place, actor)
        .await;
    finish_item_step(session, addr, context, held, actor, answer).await
}

/// Whether a game-phase header is a drop or a pick-up.
fn is_ground_step(header: u8) -> bool {
    [
        HEADER_CG_ITEM_DROP,
        HEADER_CG_ITEM_DROP2,
        HEADER_CG_ITEM_PICKUP,
    ]
    .iter()
    .any(|ground| ground.value() == header)
}

/// Where the character stands, which is where a drop lands and what a pick-up is measured from.
fn ground_place(held: &Held) -> Option<GroundPlace> {
    held.avatar.as_ref().map(|avatar| GroundPlace {
        channel: held.channel,
        map: avatar.map,
        x: avatar.x,
        y: avatar.y,
    })
}

/// The enter-game burst for the character `held` has loaded: `shown`, the records the world's
/// `Show` of it wrote to it, then its map's NPC list for the mini-map, then its own update.
///
/// `None` when the world did not answer, and the connection closes.
async fn entering_burst(
    context: &ConnectionContext,
    addr: SocketAddr,
    channel: u8,
    held: &Held,
    shown: Shown,
) -> Option<EnterGameBurst> {
    let character = held.character.as_ref()?;
    let map = held.map.unwrap_or_default();
    let npcs = match context.game.npcs_on(channel, map).await {
        Ok(npcs) => npcs,
        Err(error) => {
            warn!(%addr, %error, "The world could not list the NPCs; closing");
            return None;
        }
    };
    Some(enter_game_burst(
        character,
        shown.records,
        &npcs,
        shown.update,
        shown.silence,
        channel,
        global_time(),
    ))
}

/// What the descriptor knows of its character that a move or a use reads.
fn item_actor(held: &Held) -> Mover {
    Mover {
        recently_fought: recently_fought(held),
        // `ChatPacket` takes the empire from the descriptor, which is the character's.
        empire: held
            .character
            .as_ref()
            .map_or(0, |character| character.empire),
        language: descriptor_language(held.account.as_ref()),
        // The world sets it from the body's card (`GameState::run_item_step`).
        pk_mode: PK_MODE_PEACE,
        // The world sets the affect words from the character's affects, and the dead bit from
        // its fight, before a step runs.
        affect_flags: [0; 2],
        dead: false,
        // The world sets it from the body's card, as the PK mode.
        alignment: 0,
        // The world sets it from the character's guild, as the PK mode.
        view_guild: (0, 0),
    }
}

/// `DESC::GetLanguage`: the language of the account table the Channel login filled
/// (`G/desc.h:181-182`), which `LOGIN_BY_KEY` copies from the auth login
/// (`D/ClientManagerLogin.cpp:136-138`). A descriptor with no account table carries the 0
/// `DESC::Setup` zeroes it to.
fn descriptor_language(account: Option<&SelectAccount>) -> u8 {
    account.map_or(0, |account| account.language)
}

/// Store, send and broadcast what one item step did, or send the notice for its refusal.
///
/// The rows are written in one transaction before any record is sent, with the change a buy or
/// a sale made to the gold, and a pick-up's only once the drop that laid the item has stored its
/// own. The result is whether the descriptor stays open.
async fn finish_item_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    actor: Mover,
    answer: Result<
        Result<prodomo::item_move::MovedItems, MoveItemRefused>,
        prodomo::game_loop_messages::MoveItemError,
    >,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut moved = match answer {
        Ok(Ok(moved)) => moved,
        Ok(Err(MoveItemRefused::Refused(reason))) => {
            info!(%addr, %reason, "Item step refused");
            let to = actor.recipient(&context.locale);
            if let Some(line) = prodomo::item_move::refusal_notice(&reason, to) {
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
            warn!(%addr, %error, "The world could not be asked for an item step; closing");
            return false;
        }
    };
    // A pick-up's rows wait for the drop's, which another descriptor stores.
    if !moved.store_order.turn().await {
        warn!(%addr, kind = ?moved.kind, "The drop this step follows was not stored; closing");
        return false;
    }
    let stored = match moved.gold {
        Some(gold) => {
            db::items::apply_transfer(&context.store, moved.owner_id, &moved.changes, gold).await
        }
        None => db::items::apply_row_changes(&context.store, moved.owner_id, &moved.changes).await,
    };
    if let Err(error) = stored {
        warn!(%addr, %error, kind = ?moved.kind, "The item step could not be stored; closing");
        return false;
    }
    moved.store_order.stored();
    if let Some(points) = moved.points {
        hold_points(held, points);
    }
    if let Some(quickslots) = moved.quickslots {
        held.quickslots = quickslots;
    }
    if let Err(error) = send_all(session, &moved.records).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    info!(%addr, kind = ?moved.kind, rows = moved.changes.len(), "Item moved");
    // `PacketAround`: the look, the effects and the broadcast points reach the characters that
    // see this one. The world sends them after the rows are written, so nobody hears of a move
    // the store did not take. A ground item's view has already sent the others theirs.
    if moved.around.is_empty() {
        return true;
    }
    let Some(vid) = held.world else {
        return true;
    };
    match context.game.relay(vid, moved.around).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not relay the item step; closing");
            return false;
        }
    }
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

/// Wait for the next order the game thread gives this descriptor's client. As with
/// [`next_broadcast`], a descriptor with no lease never completes.
async fn next_order(
    orders: &mut Option<mpsc::UnboundedReceiver<ClientOrder>>,
) -> Option<ClientOrder> {
    match orders.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

/// Carry out an order of the game thread; `false` when the connection must close.
///
/// The game thread owns no socket (ADR-0002), so the effect of `FuncCheckWarp` that ends the
/// descriptor's session runs here: `WarpSet` for a warp NPC (`G/char.cpp:7967-7968`). A goto
/// NPC's `Show` and `Stop` (`:7971-7972`) move the body, which the world runs itself.
async fn follow_order<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    locations: Option<&MapLocations>,
    held: &mut Held,
    order: ClientOrder,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match order {
        ClientOrder::Warp {
            x,
            y,
            private_map_index,
            channel,
            exit,
        } => {
            // `SaveExitLocation` (`G/char.cpp:6679-6683`): the place the character came
            // from, which the save writes beside the position and which the war's end
            // sends the character back to.
            held.exit = exit;
            warp_set(
                session,
                addr,
                context,
                locations,
                held,
                (x, y),
                WarpDestination {
                    private_map_index,
                    channel,
                },
            )
            .await
        }
        ClientOrder::ItemRows { changes, order } => {
            write_item_rows(addr, context, held, changes, order).await
        }
        ClientOrder::Close => {
            // `SECTREE_MANAGER::DestroyPrivateMap` (`G/sectree_manager.cpp:1035-1039`):
            // the descriptor of every player on the private map is destroyed with it.
            // The departure saves the character, so its row names the map it stood on,
            // and the next login brings it back to the place it came from
            // (`SetPlayerProto`, `G/char.cpp:2289-2294`).
            leave_the_world(context, addr, held, Departure::Disconnect).await;
            false
        }
    }
}

/// The rows the death penalty took from the victim (`CHARACTER::ItemDropPenalty`,
/// `G/char_battle.cpp:1016-1200`); `false` when the connection must close.
///
/// The game thread cannot write the store (ADR-0002), so the descriptor that holds the
/// victim writes the rows of the items that fell, and tells the pick-ups of those items that
/// the rows are gone. A pick-up writes the item's own row, and the two rows would collide
/// while the victim's row is still there.
///
/// The rows are written in one transaction, as every other row change is. A store that
/// refuses them closes the descriptor, as a refused item step does: the pick-ups of the
/// dropped items wait for the tell, and a tell that never comes would leave them waiting.
async fn write_item_rows(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    changes: Vec<db::items::RowChange>,
    order: prodomo::item_move::StoreOrder,
) -> bool {
    let Some(player) = held.character.as_ref().map(|character| character.id) else {
        warn!(%addr, "The changed item rows have no character to write; closing");
        return false;
    };
    match db::items::apply_row_changes(&context.store, player, &changes).await {
        Ok(()) => {
            order.stored();
            info!(%addr, player, rows = changes.len(), "The changed item rows are written");
            true
        }
        Err(error) => {
            warn!(%addr, player, %error, "The changed item rows could not be stored; closing");
            false
        }
    }
}

/// The destination a warp names: the private map's copy and the Channel that serves it.
///
/// A warp NPC names neither, and a guild war's member names both: the war's own copy of the
/// map, and the Channel of the master that accepted the war (`G/guild_war.cpp:507,677`).
#[derive(Clone, Copy)]
struct WarpDestination {
    /// `lPrivateMapIndex`, legacy's third argument (`G/char.cpp:6694`), 0 for a public map.
    private_map_index: i32,
    /// The Channel that serves the map, when it is not the client's own.
    channel: Option<u8>,
}

/// `CHARACTER::WarpSet(x, y)` for this descriptor's character (`G/char.cpp:6694-6792`);
/// `false` when the connection must close.
///
/// The refusals come first and send nothing: a position no map on any Channel serves is
/// `CMapLocation::Get` failing, which legacy logs as an error and the warp NPC then retries
/// every 12 Pulses. The Rewrite logs it at debug level for the same reason it is retried:
/// ten of the owner's warp NPCs name such a position, and a player standing by one would
/// otherwise fill the log. A private map that is not a child of the resolved map is refused
/// too (`G/char.cpp:6727-6736`); a guild war's member names the war's own copy of a map, and
/// the copy's index is a child of the map the position lies on. The sort-inventory refusal
/// cannot be reached: the Rewrite has no inventory sort.
///
/// # The departure
///
/// Legacy queues a save (`G/char.cpp:1472-1476`), takes the character out of its sectree,
/// which sends its own client the delete of itself, stores the destination, sends `GC_WARP`,
/// and keeps the descriptor until the client closes it. The queued save runs after the
/// destination is stored, at the next 29-Pulse drain or at the close
/// (`CHARACTER::Disconnect`, `:1786-1789`), and either writes the destination
/// (`:1551-1557`); `GC_WARP` does not wait for it. The
/// Rewrite does the same work in an order a login on the new address can rely on: it takes the
/// character out of the world, writes the row **with the destination** before `GC_WARP`
/// leaves, and releases the login and the client-set entry, so the client's login by key finds
/// the row saved and the login free. The descriptor then waits for the client to close,
/// dropping what it sends. A save that fails closes the descriptor without `GC_WARP`, which
/// legacy never withholds: the destination stays held, so the close tries the save again, and
/// the row it writes is the one legacy's writes.
async fn warp_set<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    locations: Option<&MapLocations>,
    held: &mut Held,
    (x, y): (i32, i32),
    destination: WarpDestination,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let WarpDestination {
        private_map_index,
        channel,
    } = destination;
    if held.departed || held.world.is_none() {
        return true;
    }
    let (Some(locations), Some(map)) = (locations, held.avatar.as_ref().map(|avatar| avatar.map))
    else {
        return true;
    };
    // `CMapLocation::Get`: the atlas says which map holds the position, and the map is a
    // destination only when a Channel serves it. A warp that names the Channel itself is
    // sent there, so the map needs no other Channel to serve it.
    let resolve = |x: i32, y: i32| {
        context
            .atlas
            .index_at(x, y)
            .filter(|&index| channel.is_some() || locations.get(index).is_some())
    };
    let request = prodomo::warp::WarpSetRequest {
        x,
        y,
        private_map_index,
        sort_inventory_pulse: 0,
        now_ms: 0,
    };
    let outcome = prodomo::warp::judge_warp_set(&request, map, true, &resolve);
    let prodomo::warp::WarpSetOutcome::Accepted {
        resolved_map_index,
        stored_map_index,
        ..
    } = outcome
    else {
        debug!(%addr, x, y, map, ?outcome, "WarpSet refused");
        return true;
    };
    // The destination: the Channel the map is served by. A warp that names a Channel goes
    // to that one — a guild war's private map stands on the Channel of the master that
    // accepted the war, and a member of another Channel is sent there.
    let destination = match channel {
        Some(number) => context
            .routes
            .channel_ports
            .get(&number)
            .map(|&port| (i32::from_le_bytes(context.public_ip.octets()), port)),
        None => locations.get(resolved_map_index),
    };
    let Some((address, port)) = destination else {
        return true;
    };
    leave_the_world(context, addr, held, Departure::Warp).await;
    let target = prodomo::warp::WarpTarget {
        map_index: stored_map_index,
        x,
        y,
    };
    held.warp = Some(target);
    if !save_held(context, held, std::time::Instant::now()).await {
        warn!(%addr, x, y, "The warp could not save the character; closing without GC_WARP");
        return false;
    }
    // The row is the new descriptor's to write from here on.
    held.save = None;
    held.presence = None;
    held.logon = None;
    held.departed = true;
    let vid = held.character.as_ref().map_or(0, character_vid);
    let records = prodomo::warp::departure_records(vid, &target, address, port);
    if let Err(error) = send_all(session, &records).await {
        warn!(%addr, %error, "Client session stopped while writing GC_WARP");
        return false;
    }
    // `LogManager::CharLog(this, 0, "WARP", ...)`, whose `DestMapIdx` is the private map
    // argument, which a warp NPC leaves at 0 (`G/char.cpp:6787-6789`).
    info!(
        %addr,
        name = held.character.as_ref().map_or("", |character| character.name.as_str()),
        map,
        dest_map = request.private_map_index,
        x,
        y,
        empire = held.character.as_ref().map_or(0, |character| character.empire),
        "WARP"
    );
    true
}

/// `CG_WARP` (65) in the game phase: `CInputMain::Warp`, whose whole body is `ch->WarpEnd()`
/// (`G/input_main.cpp:2271-2274`).
///
/// `WarpEnd` returns at its first line unless `m_posWarp` is set (`G/char.cpp:6800-6801`). A
/// character a login loaded never holds one, and the one descriptor that stores one, the
/// `WarpSet` departure, drops every record after `GC_WARP`. So on every descriptor that
/// reaches this arm `WarpEnd` returns at once: nothing is sent and the connection stays open.
/// Before ledger 228 the header had no arm and closed the connection.
fn warp_end(addr: SocketAddr, frame: &ClientFrame) -> bool {
    match CgWarp::decode_frame(frame) {
        Ok(CgWarp) => {
            debug!(%addr, "CG_WARP with no pending Warp");
            true
        }
        Err(error) => {
            warn!(%addr, %error, "CG_WARP did not decode; closing");
            false
        }
    }
}
/// `ch->GetParty() != NULL` (`G/input_main.cpp:959`), asked lazily: only a party line
/// pays the world round-trip. A character outside the world holds no party —
/// `link_party` only runs on world entry. `None` closes the connection: the world
/// could not answer, as with the talking relay.
async fn party_answer(
    game: &GameLoopController,
    record: &CgChat,
    addr: SocketAddr,
    world: Option<common::vid::Vid>,
) -> Option<bool> {
    if record.chat_type != CHAT_PARTY {
        return Some(false);
    }
    match world {
        Some(world_vid) => match game.party_of(world_vid).await {
            Ok(holds) => Some(holds),
            Err(error) => {
                warn!(%addr, %error, "The world could not answer the party question; closing");
                None
            }
        },
        None => Some(false),
    }
}
/// `ch->GetGuild() != NULL` (`G/input_main.cpp:976`), asked lazily: only a guild line
/// pays the world round-trip. A character outside the world holds no guild —
/// `link_guild` only runs on world entry. `None` closes the connection: the world
/// could not answer, as with the talking relay.
async fn guild_answer(
    game: &GameLoopController,
    record: &CgChat,
    addr: SocketAddr,
    world: Option<common::vid::Vid>,
) -> Option<bool> {
    if record.chat_type != CHAT_GUILD {
        return Some(false);
    }
    match world {
        Some(world_vid) => match game.guild_of(world_vid).await {
            Ok(holds) => Some(holds),
            Err(error) => {
                warn!(%addr, %error, "The world could not answer the guild question; closing");
                None
            }
        },
        None => Some(false),
    }
}

/// `FindAffect(AFFECT_BLOCK_CHAT)` (`G/input_main.cpp:828-834`), asked lazily:
/// only a plain line pays the world round-trip — a slash line goes to the
/// interpreter before the counter and never reaches the silence check
/// (`G/input_main.cpp:802`). A character outside the world holds no silence.
/// `None` closes the connection: the world could not answer, as with the
/// talking relay.
async fn silence_answer(
    game: &GameLoopController,
    record: &CgChat,
    addr: SocketAddr,
    world: Option<common::vid::Vid>,
) -> Option<Option<i32>> {
    if record.text.len() > 1 && record.text[0] == b'/' {
        return Some(None);
    }
    let Some(world_vid) = world else {
        return Some(None);
    };
    match game.block_chat_of(world_vid).await {
        Ok(seconds) => Some(seconds),
        Err(error) => {
            warn!(%addr, %error, "The world could not answer the silence question; closing");
            None
        }
    }
}

/// An info line to the sender: the one `send_all` the judge's sender-only effects share.
/// `false` closes the connection.
async fn send_info_line<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    effect: &ChatEffect,
    to: Recipient<'_>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(bytes) = prodomo::chat::info_line(effect, to) else {
        warn!(%addr, "Info effect built no line; closing");
        return false;
    };
    if let Err(error) = send_all(session, &[bytes]).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
}
/// What a relayed chat line did: sent, dropped outside the world, or closing.
enum Relayed {
    Sent(Settled),
    Dropped,
    Closed,
}
/// A talking or party line through the world, in order with the moves it relays.
/// `FEmpireChatPacket` walks the process client set and filters by map index;
/// `ForEachOnlineMember` walks the party's member map. Neither self-excludes, so
/// both reach the speaker too.
async fn relay_chat_line(
    context: &ConnectionContext,
    addr: SocketAddr,
    world: Option<common::vid::Vid>,
    effect: &ChatEffect,
) -> Relayed {
    let (scope, bytes, vid) = match effect {
        ChatEffect::TalkingToMap { vid, empire, text } => {
            let record = prodomo::chat::talking_record(effect)
                .unwrap_or_else(|| panic!("a talking effect always builds a record"));
            if record.id != *vid || record.empire != *empire || record.text != *text {
                warn!(%addr, "Relayed chat record does not match its effect");
            }
            (RelayScope::Map, record.encode(), *vid)
        }
        ChatEffect::PartyToMembers { vid, empire, text } => {
            let record = prodomo::chat::party_record(effect)
                .unwrap_or_else(|| panic!("a party effect always builds a record"));
            if record.id != *vid || record.empire != *empire || record.text != *text {
                warn!(%addr, "Relayed chat record does not match its effect");
            }
            (RelayScope::Party, record.encode(), *vid)
        }
        ChatEffect::GuildToMembers { text } => {
            // `ChatPacket(CHAT_TYPE_GUILD, ...)`: `id` 0, `bCanFormat` true, and the
            // world overwrites `bEmpire` per recipient, so the empire here is a placeholder.
            let record = prodomo::chat::guild_record(effect, 0)
                .unwrap_or_else(|| panic!("a guild effect always builds a record"));
            if record.id != 0 || !record.can_format || record.text != *text {
                warn!(%addr, "Relayed chat record does not match its effect");
            }
            (RelayScope::Guild, record.encode(), 0)
        }
        _ => panic!("a relayed effect is talking, party, or guild"),
    };
    let bytes = bytes.unwrap_or_else(|error| panic!("a relayed chat record always fits: {error}"));
    let Some(world_vid) = world else {
        warn!(%addr, vid, "Chat line from a character outside the world; dropped");
        return Relayed::Dropped;
    };
    match context.game.relay(world_vid, vec![(scope, bytes)]).await {
        Ok(settling) => {
            info!(%addr, vid, "Chat line relayed to its scope");
            Relayed::Sent(settling)
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not relay the chat line; closing");
            Relayed::Closed
        }
    }
}

/// `CG_CHAT` (3) in the game phase: `CInputMain::Chat` (`input_main.cpp:781-991`).
///
/// The judge owns the legacy order, so this function only applies the effects. The four
/// broadcasts are the ones the judge asks for: an info line to the sender, a talking line to
/// every client on the sender's map, a party line to every linked member of the sender's
/// party, and a shout to every client on every Channel. The last three include the sender,
/// because the registry lease makes the sender one of its own recipients.
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
    // `ChatPacket` takes the empire and the language from the descriptor; the empire is the
    // speaker's.
    let to = Recipient {
        strings: &context.locale,
        language: descriptor_language(held.account.as_ref()),
        empire: character.empire,
    };
    let Some(in_party) = party_answer(&context.game, &record, addr, held.world).await else {
        return false;
    };
    let Some(in_guild) = guild_answer(&context.game, &record, addr, held.world).await else {
        return false;
    };
    let Some(block_chat_seconds) = silence_answer(&context.game, &record, addr, held.world).await
    else {
        return false;
    };
    let chat_context = ChatContext {
        name: character.name.clone(),
        vid: character.id,
        empire: character.empire,
        level: character.level,
        block_chat_seconds,
        in_party,
        in_guild,
        language: to.language,
        pulse: context.game.pulse(),
        shout_limit_level: context.shout_limit_level,
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
                // `interpret_command(ch, buf + 1, buflen - 1)`. It costs no chat counter, so a
                // client may send it between two normal lines.
                if !run_command(session, addr, context, held, argument).await {
                    return false;
                }
            }
            ChatEffect::DelayedDisconnect => {
                // `ch->GetDesc()->DelayedDisconnect(0)`: no record, the frame is consumed,
                // and the descriptor closes on its own schedule.
                info!(%addr, "Chat counter reached the disconnect line; closing");
                return false;
            }
            ChatEffect::BlockChatInfo { .. }
            | ChatEffect::InfoToSender { .. }
            | ChatEffect::ShoutBelowLevel { .. } => {
                if !send_info_line(session, addr, effect, to).await {
                    return false;
                }
            }
            ChatEffect::TalkingToMap { .. }
            | ChatEffect::PartyToMembers { .. }
            | ChatEffect::GuildToMembers { .. } => {
                match relay_chat_line(context, addr, held.world, effect).await {
                    Relayed::Sent(settling) => held.settling = Some(settling),
                    Relayed::Dropped => continue,
                    Relayed::Closed => return false,
                }
            }
            ChatEffect::UnknownType { chat_type } => {
                // The legacy default arm only logs. Nothing reaches the client.
                info!(%addr, chat_type, "Unknown chat type; nothing sent");
            }
            ChatEffect::ShoutOnCooldown => {
                // `return (iExtraLen)` with no line (`input_main.cpp:893-894`).
                info!(%addr, "Shout refused; the last shout's cooldown has not run out");
            }
            ChatEffect::Shout { empire, text } => {
                let sent = shout(context, *empire, text);
                info!(%addr, empire, recipients = sent, "Shout sent to every Channel");
            }
        }
    }
    true
}

/// Sends a shout line to every client on every Channel that hears it, and returns how many
/// were sent.
///
/// Legacy sends the line to its own Core's clients and relays it as `GG_SHOUT` to every other
/// Core, whose `FuncShout` builds the same line for each of its clients
/// (`input_main.cpp:903-911`, `input_p2p.cpp:215-240`). With one process there is no relay:
/// the registry holds every Channel's clients, and each gets the line in its own language and
/// with its own empire, as `ChatPacket` builds it from the recipient's descriptor.
fn shout(context: &ConnectionContext, empire: u8, text: &[u8]) -> usize {
    context.clients.deliver_everywhere(|entry| {
        prodomo::chat::shout_for(
            entry,
            &context.locale,
            context.modes.global_shout,
            empire,
            text,
        )
    })
}

/// `CG_WHISPER` (19) in the game and dead phases: `CInputMain::Whisper`
/// (`G/input_main.cpp:340-649`).
///
/// The line goes to the character the name finds, on this Channel or on another one, and the
/// sender hears nothing when it lands. The record is the sender's name and the text, typed
/// `WHISPER_TYPE_GM` when the sender is a game master.
async fn whisper_line(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgWhisper::decode(frame) {
        Ok(record) => record,
        Err(CgWhisperError::DeclaredSizeTooSmall { declared_size }) => {
            // `iExtraLen < 0`: the legacy logs the length and closes the descriptor
            // (`G/input_main.cpp:349-353`).
            info!(%addr, declared_size, "Whisper refused by size; closing");
            return false;
        }
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed whisper; closing");
            return false;
        }
    };
    // `#ifdef ENABLE_WHISPER_CHAT_SPAMLIMIT` (`G/input_main.cpp:356-362`): the
    // counter increments on the reset of a new window first, ahead of the
    // block-chat affect and the rest of the handler. At 10 and above the line
    // is consumed with no record and the descriptor disconnects
    // (`DelayedDisconnect(0)`); below 10 the line proceeds.
    if held.chat.judge_whisper_counter(context.game.pulse()) == WhisperCounter::Refused {
        info!(%addr, "Whisper refused by the chat counter; closing");
        return false;
    }
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "Whisper without a character; ignoring");
        return true;
    };
    // `if (ch->IsGM())` (`G/input_main.cpp:619-620`): the type is the game master's when the
    // sender is one, and the test-server mode makes every character one
    // (`G/char.cpp:6373-6380`).
    let whisper_type = if held.gm_level > GM_PLAYER || context.modes.test_server {
        WHISPER_TYPE_GM
    } else {
        WHISPER_TYPE_NORMAL
    };
    let line = GcWhisper {
        whisper_type,
        name_from: name_field(&character.name),
        can_format: true,
        text: record.text.clone(),
    };
    let Ok(bytes) = line.encode() else {
        warn!(%addr, "Whisper did not fit its record; ignoring");
        return true;
    };
    // The target's name, as the client sent it: a C string, so it ends at the first NUL.
    let name = record.target_name();
    match context
        .game
        .whisper(
            common::vid::Vid::new(character.id),
            name.to_vec(),
            record.text.clone(),
            bytes,
        )
        .await
    {
        Ok(WhisperOutcome::Delivered) => {
            info!(%addr, "Whisper delivered to its target");
        }
        Ok(WhisperOutcome::Dropped) => {
            info!(%addr, "Whisper dropped: the sender named itself, or the line was empty");
        }
        Ok(WhisperOutcome::Answered) => {
            info!(%addr, "Whisper left for a target that is not there; the sender was told");
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not answer a whisper; closing");
            return false;
        }
    }
    true
}

/// `CG_MOVE` (7) in the game phase: `CInputMain::Move` (`input_main.cpp:1757-1915`).
///
/// The world owns the body, so it judges the move, walks the body and relays the record to
/// the characters that see it; a refusal it logs and, for a move too far or a dead mover,
/// shows the body where it stands. Only a record that does not decode closes here.
async fn move_character(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgMove::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed MOVE; closing");
            return false;
        }
    };
    let Some(vid) = held.world else {
        warn!(%addr, "MOVE from a character outside the world; ignoring");
        return true;
    };
    match context.game.move_character(vid, record).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the move; closing");
            return false;
        }
    }
    true
}

/// `CInputMain::Target` (`G/input_main.cpp:2249-2269`): the world sets the character's target to
/// the one the VID names and answers with `GC_TARGET` when it changed.
async fn target_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgTarget::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed TARGET; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "TARGET from a character outside the world; ignoring");
        return true;
    };
    match context.game.target(world_vid, record.vid).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the target; closing");
            return false;
        }
    }
    debug!(%addr, target = record.vid, "Target handed to the world");
    true
}

/// `CInputMain::PartyInvite` (`G/input_main.cpp:2476-2499`): the world invites the VID
/// the record names.
async fn party_invite_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartyInvite::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_INVITE; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_INVITE from a character outside the world; ignoring");
        return true;
    };
    match context.game.party_invite(world_vid, record.vid).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the invite; closing");
            return false;
        }
    }
    debug!(%addr, guest = record.vid, "Party invite handed to the world");
    true
}

/// `CInputMain::PartyInviteAnswer` (`G/input_main.cpp:2501-2519`): the world answers the
/// invite from the VID the record names.
async fn party_invite_answer_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartyInviteAnswer::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_INVITE_ANSWER; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_INVITE_ANSWER from a character outside the world; ignoring");
        return true;
    };
    match context
        .game
        .party_invite_answer(world_vid, record.leader_vid, record.accept)
        .await
    {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the answer; closing");
            return false;
        }
    }
    debug!(%addr, inviter = record.leader_vid, "Party answer handed to the world");
    true
}

/// `CInputMain::PartyRemove` (`G/input_main.cpp:2578-2675`): the world removes the PID
/// the record names.
async fn party_remove_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartyRemove::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_REMOVE; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_REMOVE from a character outside the world; ignoring");
        return true;
    };
    match context.game.party_remove(world_vid, record.pid).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the removal; closing");
            return false;
        }
    }
    debug!(%addr, pid = record.pid, "Party removal handed to the world");
    true
}

/// `CInputMain::AnswerMakeGuild` (`G/input_main.cpp:2672-2730`): the world judges the
/// answer, the descriptor writes the guild rows, and the world commits.
///
/// A refusal sends the line, if any, and stays open. A judged creation is stored before
/// it is committed: the rows land first, then the commit takes the fee and the item and
/// the master's own records go out, and only then does the `GUILD_NAME` broadcast reach
/// every other PC — in that order, so nobody hears of a guild the store did not take. A
/// unique violation answers `[LS;473]`, as the `SELECT COUNT(*)` arm does
/// (`G/guild_manager.cpp:96-109`); any other store error closes, as an unstored move does.
async fn answer_make_guild_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgAnswerMakeGuild::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ANSWER_MAKE_GUILD; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "ANSWER_MAKE_GUILD from a character outside the world; ignoring");
        return true;
    };
    let judged = match context
        .game
        .answer_make_guild(world_vid, record.guild_name, global_time())
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the guild answer; closing");
            return false;
        }
    };
    let name = match judged {
        GuildMakeAnswer::Store { name } => name,
        GuildMakeAnswer::Refused { line } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildMakeAnswer::Committed { .. } => {
            warn!(%addr, "The world answered a judgement with a commit; closing");
            return false;
        }
    };
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "ANSWER_MAKE_GUILD without a character; closing");
        return false;
    };
    let pid = character.id;
    let id = match store_make_guild(context, &name, pid).await {
        Ok(id) => id,
        Err(Some(taken)) => {
            return answer_guild_taken(session, addr, context, held, character, taken).await;
        }
        Err(None) => {
            warn!(%addr, "The guild rows could not be stored; closing");
            return false;
        }
    };
    let commit = GuildMakeCommit { id, name };
    let committed = match context
        .game
        .commit_make_guild(world_vid, commit, item_actor(held))
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the guild; closing");
            return false;
        }
    };
    let GuildMakeAnswer::Committed {
        id,
        own,
        others,
        gold,
        changes,
    } = committed
    else {
        warn!(%addr, "The world answered a commit without one; closing");
        return false;
    };
    if let Err(error) = db::items::apply_transfer(&context.store, pid, &changes, gold).await {
        warn!(%addr, %error, id, "The guild fee could not be stored; closing");
        return false;
    }
    if let Err(error) = send_all(session, &own).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    // The `GUILD_NAME` broadcast to every PC (`CGuild::CGuild`, `G/guild.cpp:117-126`):
    // the master already holds its copy in `own`, so every other client hears it here.
    let own_vid = world_vid.raw();
    let heard = match others.first() {
        Some(record) => context
            .clients
            .deliver_everywhere(|entry| (entry.vid != own_vid).then(|| record.clone())),
        None => 0,
    };
    info!(%addr, id, others = heard, "Guild made");
    true
}
/// Write one judged creation's rows (`CGuild::CGuild`'s saves, `G/guild.cpp:59-127`).
/// Answers the new id, or the taken name for the `[LS;473]` arm, or nothing for a
/// store failure that closes.
async fn store_make_guild(
    context: &ConnectionContext,
    name: &str,
    pid: u32,
) -> Result<u32, Option<String>> {
    let grades: [(&str, u8); 15] = std::array::from_fn(|index| {
        if index == 0 {
            ("Leader", 15)
        } else {
            ("...", 0)
        }
    });
    match db::guilds::create_guild(&context.store, name, pid, &grades).await {
        Ok(id) => Ok(id),
        Err(db::guilds::CreateGuildError::GuildNameTaken { name }) => Err(Some(name)),
        Err(_) => Err(None),
    }
}

/// Answer the `[LS;473]` arm: a concurrent creation took the judged name
/// (`G/guild_manager.cpp:96-109`). Stays open; `false` only when the send died.
async fn answer_guild_taken<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    character: &db::players::Character,
    taken: String,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    info!(%addr, name = taken, "A concurrent creation took the guild name; answering 473");
    let to = Recipient {
        strings: &context.locale,
        language: descriptor_language(held.account.as_ref()),
        empire: character.empire,
    };
    let line =
        prodomo::chat_line::chat_packet(to, protocol::gc_chat::CHAT_TYPE_INFO, b"[LS;473]", &[]);
    if let Err(error) = send_all(session, &[line]).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
}

/// `CInputMain::Guild` (`G/input_main.cpp:2797-3162`): the 0x50 record's subheader picks the
/// door — invite, answer, removal, or one of the four grade arms — the closed money arms
/// stay quiet, and the later slices' sub-packets stay quiet.
///
/// Stays open on a refusal or a quiet arm; `false` only when the send died, the world
/// went silent, or the row could not be stored.
async fn cg_guild_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use protocol::cg_guild::{
        sub_packet_size, GUILD_SUBHEADER_CG_ADD_MEMBER, GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY,
        GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME, GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL,
        GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE, GUILD_SUBHEADER_CG_CHARGE_GSP,
        GUILD_SUBHEADER_CG_DELETE_COMMENT, GUILD_SUBHEADER_CG_DEPOSIT_MONEY,
        GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER, GUILD_SUBHEADER_CG_OFFER,
        GUILD_SUBHEADER_CG_POST_COMMENT, GUILD_SUBHEADER_CG_REFRESH_COMMENT,
        GUILD_SUBHEADER_CG_REMOVE_MEMBER, GUILD_SUBHEADER_CG_USE_SKILL,
        GUILD_SUBHEADER_CG_WITHDRAW_MONEY,
    };
    let Some(subheader) = frame.payload.first().copied() else {
        warn!(%addr, "Client sent a GUILD record without a subheader; closing");
        return false;
    };
    let body = &frame.payload[1..];
    if body.len() < sub_packet_size(subheader) {
        warn!(%addr, subheader, "Client sent a short GUILD sub-packet; closing");
        return false;
    }
    let Some(world_vid) = held.world else {
        warn!(%addr, "GUILD from a character outside the world; ignoring");
        return true;
    };
    match subheader {
        GUILD_SUBHEADER_CG_ADD_MEMBER => {
            let guest = u32::from_le_bytes(body[..4].try_into().unwrap());
            if let Err(error) = context
                .game
                .guild_add_member(world_vid, guest, global_time())
                .await
            {
                warn!(%addr, %error, "The world could not take the invite; closing");
                return false;
            }
            debug!(%addr, guest, "Guild invite handed to the world");
            true
        }
        GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER => {
            let guild_id = u32::from_le_bytes(body[..4].try_into().unwrap());
            let accept = body[4];
            guild_invite_answer_step(session, addr, context, held, world_vid, guild_id, accept)
                .await
        }
        GUILD_SUBHEADER_CG_REMOVE_MEMBER => {
            let pid = u32::from_le_bytes(body[..4].try_into().unwrap());
            guild_remove_step(session, addr, context, world_vid, pid).await
        }
        GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME
        | GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY
        | GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE
        | GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL
        | GUILD_SUBHEADER_CG_OFFER
        | GUILD_SUBHEADER_CG_CHARGE_GSP => {
            guild_grade_step(session, addr, context, world_vid, subheader, body).await
        }
        GUILD_SUBHEADER_CG_USE_SKILL => {
            let vnum = u32::from_le_bytes(body[..4].try_into().unwrap());
            let pid = u32::from_le_bytes(body[4..8].try_into().unwrap());
            guild_use_skill_step(session, addr, context, world_vid, vnum, pid).await
        }
        GUILD_SUBHEADER_CG_POST_COMMENT
        | GUILD_SUBHEADER_CG_DELETE_COMMENT
        | GUILD_SUBHEADER_CG_REFRESH_COMMENT => {
            guild_comment_step(session, addr, context, held, world_vid, subheader, body).await
        }
        // `CInputMain::Guild`'s money arms (`G/input_main.cpp:2826-2878`): both return
        // before their bodies — the funds are closed for now — so the Rewrite stays
        // quiet past the frame check.
        GUILD_SUBHEADER_CG_DEPOSIT_MONEY | GUILD_SUBHEADER_CG_WITHDRAW_MONEY => {
            debug!(%addr, subheader, "a closed GUILD money arm stays quiet");
            true
        }
        _ => {
            debug!(%addr, subheader, "an unported GUILD sub-packet stays quiet");
            true
        }
    }
}

/// Map stored comment rows to the lines the `COMMENTS` packet packs.
fn comment_lines(board: &[db::guilds::StoredComment]) -> Vec<GuildCommentLine> {
    board
        .iter()
        .map(|comment| GuildCommentLine {
            id: comment.id,
            name: comment.name.as_bytes().to_vec(),
            content: comment.content.as_bytes().to_vec(),
        })
        .collect()
}

/// Pack a read board into the COMMENTS self packet (`RefreshCommentForce`,
/// `G/guild.cpp:1097-1141`): one row per comment, the count byte first.
fn guild_comments_packet(board: &[GuildCommentLine]) -> Vec<u8> {
    use protocol::gc_guild::{
        guild_comments_record, pad_guild_comment, pad_member_name, GcGuildComment,
    };
    let rows: Vec<GcGuildComment> = board
        .iter()
        .map(|line| GcGuildComment {
            id: line.id,
            name: pad_member_name(&line.name),
            content: pad_guild_comment(&line.content),
        })
        .collect();
    guild_comments_record(&rows)
}

/// The comment half of [`cg_guild_step`]: the world judges, the descriptor stores the
/// rows, and the world reports the board. The COMMENTS record and the lines ride the
/// speaker's own session; nothing fans through the outboxes.
///
/// A refusal sends the line, if any, and stays open. A store error closes, as an
/// unstored move does.
async fn guild_comment_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    world_vid: common::vid::Vid,
    subheader: u8,
    body: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use protocol::cg_guild::{
        GUILD_SUBHEADER_CG_DELETE_COMMENT, GUILD_SUBHEADER_CG_POST_COMMENT,
        GUILD_SUBHEADER_CG_REFRESH_COMMENT,
    };
    // The post's length byte (`:3053-3060`): over 50 closes, and a body short of the
    // length closes — the length's own byte is the frame's first, so the check reads
    // `1 + length` against the body.
    let text: Vec<u8> = match subheader {
        GUILD_SUBHEADER_CG_POST_COMMENT => {
            let length = usize::from(body[0]);
            if length > protocol::gc_guild::GUILD_COMMENT_MAX_LEN {
                warn!(%addr, "Client sent a comment past the length; closing");
                return false;
            }
            if body.len() < 1 + length {
                warn!(%addr, "Client sent a short comment body; closing");
                return false;
            }
            body[1..=length].to_vec()
        }
        GUILD_SUBHEADER_CG_REFRESH_COMMENT | GUILD_SUBHEADER_CG_DELETE_COMMENT => Vec::new(),
        _ => {
            warn!(%addr, subheader, "an unported GUILD sub-packet reached the comment step");
            return false;
        }
    };
    if subheader == GUILD_SUBHEADER_CG_REFRESH_COMMENT {
        return guild_comment_refresh_step(session, addr, context, held, world_vid).await;
    }
    if subheader == GUILD_SUBHEADER_CG_DELETE_COMMENT {
        return guild_comment_delete_step(session, addr, context, held, world_vid, body).await;
    }
    guild_comment_post_step(session, addr, context, held, world_vid, subheader, text).await
}

/// The refresh half of [`guild_comment_step`]: the world judges the gates, the
/// descriptor reads the board, and the `COMMENTS` record rides the speaker's own
/// session.
async fn guild_comment_refresh_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    world_vid: common::vid::Vid,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = match context.game.answer_guild_refresh_comment(world_vid).await {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the refresh; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Done { .. } = judged else {
        if let GuildCommentAnswer::Refused { line } = judged {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        warn!(%addr, "The world answered a refresh with a store; closing");
        return false;
    };
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "REFRESH_COMMENT without a character; closing");
        return false;
    };
    let judged = match context.game.answer_guild_delete_comment(world_vid, 0).await {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not read the refresh gates; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Store { guild_id, .. } = judged else {
        warn!(%addr, "The world answered a refresh without its gates; closing");
        return false;
    };
    let _ = character;
    let board = match db::guilds::board(&context.store, guild_id, GUILD_COMMENT_MAX_COUNT).await {
        Ok(board) => board,
        Err(error) => {
            warn!(%addr, %error, "The board could not be read; closing");
            return false;
        }
    };
    let lines: Vec<GuildCommentLine> = board
        .iter()
        .map(|comment| GuildCommentLine {
            id: comment.id,
            name: comment.name.as_bytes().to_vec(),
            content: comment.content.as_bytes().to_vec(),
        })
        .collect();
    let packet = guild_comments_packet(&lines);
    if let Err(error) = send_all(session, &[packet]).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    info!(%addr, "Guild comment refresh stored");
    true
}

/// The delete half of [`guild_comment_step`]: the world judges, the descriptor deletes
/// the row and reads the board, and the world reports it.
async fn guild_comment_delete_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    world_vid: common::vid::Vid,
    body: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let comment_id = u32::from_le_bytes(body[..4].try_into().unwrap());
    let judged = match context
        .game
        .answer_guild_delete_comment(world_vid, comment_id)
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the delete; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Store {
        guild_id,
        pid,
        content: flag,
    } = judged
    else {
        if let GuildCommentAnswer::Refused { line } = judged {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        warn!(%addr, "The world answered a delete without its gates; closing");
        return false;
    };
    let leader = flag.first() == Some(&1);
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "DELETE_COMMENT without a character; closing");
        return false;
    };
    if character.id != pid {
        warn!(%addr, "A delete judged another speaker; closing");
        return false;
    }
    let name = character.name.clone();
    let deleted =
        match db::guilds::delete_comment(&context.store, guild_id, comment_id, leader, &name).await
        {
            Ok(deleted) => deleted,
            Err(error) => {
                warn!(%addr, %error, "The comment could not be deleted; closing");
                return false;
            }
        };
    let board = if deleted {
        match db::guilds::board(&context.store, guild_id, GUILD_COMMENT_MAX_COUNT).await {
            Ok(board) => Some(comment_lines(&board)),
            Err(error) => {
                warn!(%addr, %error, "The board could not be read; closing");
                return false;
            }
        }
    } else {
        None
    };
    let reported = match context
        .game
        .commit_guild_delete_comment(GuildCommentCommit::Read {
            speaker: world_vid,
            guild_id,
            board,
            deleted,
        })
        .await
    {
        Ok(reported) => reported,
        Err(error) => {
            warn!(%addr, %error, "The world could not report the delete; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Done { board, line } = reported else {
        warn!(%addr, "The world answered a delete without a report; closing");
        return false;
    };
    if let Some(board) = board {
        if let Err(error) = send_all(session, &[guild_comments_packet(&board)]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    if let Some(line) = line {
        if let Err(error) = send_all(session, &[line]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    info!(%addr, "Guild comment delete stored");
    true
}

/// The post half of [`guild_comment_step`]: the world judges, the descriptor stores the
/// row and reads the board, and the world commits it.
async fn guild_comment_post_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    world_vid: common::vid::Vid,
    subheader: u8,
    text: Vec<u8>,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = match context
        .game
        .answer_guild_post_comment(world_vid, text)
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the post; closing");
            return false;
        }
    };
    let (guild_id, pid, text) = match judged {
        GuildCommentAnswer::Store {
            guild_id,
            pid,
            content: text,
        } => (guild_id, pid, text),
        GuildCommentAnswer::Refused { line } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildCommentAnswer::Done { .. } => {
            warn!(%addr, "The world answered a judgement with a report; closing");
            return false;
        }
    };
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "POST_COMMENT without a character; closing");
        return false;
    };
    if character.id != pid {
        warn!(%addr, "A post judged another speaker; closing");
        return false;
    }
    let name = character.name.clone();
    let text = String::from_utf8_lossy(&text).into_owned();
    if let Err(error) = db::guilds::post_comment(&context.store, guild_id, &name, &text).await {
        warn!(%addr, %error, "The comment could not be posted; closing");
        return false;
    }
    let board = match db::guilds::board(&context.store, guild_id, GUILD_COMMENT_MAX_COUNT).await {
        Ok(board) => board,
        Err(error) => {
            warn!(%addr, %error, "The board could not be read; closing");
            return false;
        }
    };
    let lines: Vec<GuildCommentLine> = board
        .iter()
        .map(|comment| GuildCommentLine {
            id: comment.id,
            name: comment.name.as_bytes().to_vec(),
            content: comment.content.as_bytes().to_vec(),
        })
        .collect();
    let committed = match context
        .game
        .commit_guild_post_comment(GuildCommentCommit::Post {
            speaker: world_vid,
            guild_id,
            board: lines,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the post; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Done { board, line: _ } = committed else {
        warn!(%addr, "The world answered a commit without a board; closing");
        return false;
    };
    if let Some(board) = board {
        if let Err(error) = send_all(session, &[guild_comments_packet(&board)]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    info!(%addr, subheader, "Guild comment step stored");
    true
}

/// The invite-answer half of [`cg_guild_step`]: the world judges, the descriptor inserts
/// the member row, and the world commits.
///
/// A refusal sends the line, if any, to `to`'s client — which is this session when the
/// invitee answers on its own descriptor, as it always does — and stays open. A judged
/// join is stored before it is committed: the row lands first, then the commit fans and
/// the joiner's walk goes out. A store error closes, as an unstored move does.
async fn guild_invite_answer_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    world_vid: common::vid::Vid,
    guild_id: u32,
    accept: u8,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = match context
        .game
        .answer_guild_invite(world_vid, guild_id, accept, global_time())
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the invite answer; closing");
            return false;
        }
    };
    let judged_guild = match judged {
        GuildJoinAnswer::Store { guild_id } => guild_id,
        GuildJoinAnswer::Refused { line, .. } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildJoinAnswer::Committed { .. } => {
            warn!(%addr, "The world answered a judgement with a commit; closing");
            return false;
        }
    };
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "GUILD_INVITE_ANSWER without a character; closing");
        return false;
    };
    let pid = character.id;
    if let Err(error) = db::guilds::add_member(&context.store, judged_guild, pid, 15).await {
        warn!(%addr, %error, "The member row could not be stored; closing");
        return false;
    }
    let committed = match context
        .game
        .commit_guild_join(GuildJoinCommit {
            guild_id: judged_guild,
            pid,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the join; closing");
            return false;
        }
    };
    let GuildJoinAnswer::Committed { own } = committed else {
        warn!(%addr, "The world answered a commit without one; closing");
        return false;
    };
    if let Err(error) = send_all(session, &own).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    info!(%addr, guild_id = judged_guild, pid, "Guild joined");
    true
}

/// The removal half of [`cg_guild_step`]: the world judges, the descriptor deletes the
/// member row, and the world commits.
///
/// The pre line, if any, goes out before the delete; the post line, if any, after the
/// commit. A refusal sends the line, if any, and stays open. A store error closes, as an
/// unstored move does.
async fn guild_remove_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    world_vid: common::vid::Vid,
    pid: u32,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = match context
        .game
        .answer_guild_remove(world_vid, pid, global_time())
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the removal; closing");
            return false;
        }
    };
    let (victim, online, pre) = match judged {
        GuildRemoveAnswer::Store { pid, online, pre } => (pid, online, pre),
        GuildRemoveAnswer::Refused { line } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildRemoveAnswer::Committed { .. } => {
            warn!(%addr, "The world answered a judgement with a commit; closing");
            return false;
        }
    };
    if let Some(pre) = pre {
        if let Err(error) = send_all(session, &[pre]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    // The commit carries the judged online-ness, so the offline arm chats after the
    // store even if the victim entered between.
    if let Err(error) = db::guilds::remove_member(&context.store, victim).await {
        warn!(%addr, %error, "The member row could not be deleted; closing");
        return false;
    }
    let committed = match context
        .game
        .commit_guild_remove(GuildRemoveCommit {
            speaker: world_vid,
            pid: victim,
            online,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the removal; closing");
            return false;
        }
    };
    let GuildRemoveAnswer::Committed { post } = committed else {
        warn!(%addr, "The world answered a commit without one; closing");
        return false;
    };
    if let Some(post) = post {
        if let Err(error) = send_all(session, &[post]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    info!(%addr, pid = victim, "Guild member removed");
    true
}

/// The grade half of [`cg_guild_step`]: the world judges, the descriptor stores the
/// row, and the world commits.
///
/// A refusal sends the line, if any, and stays open. A store error closes, as an
/// unstored move does.
async fn guild_grade_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    world_vid: common::vid::Vid,
    subheader: u8,
    body: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = judge_guild_grade_step(context, world_vid, subheader, body).await;
    let Some(judged) = judged else {
        warn!(%addr, subheader, "an unported GUILD sub-packet reached the grade step");
        return false;
    };
    let judged = match judged {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the grade step; closing");
            return false;
        }
    };
    let (guild_id, set) = match judged {
        GuildGradeAnswer::Store { guild_id, set } => (guild_id, set),
        GuildGradeAnswer::Refused { line } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildGradeAnswer::Committed { .. } => {
            warn!(%addr, "The world answered a judgement with a commit; closing");
            return false;
        }
    };
    if let Err(error) = store_guild_grade_set(context, guild_id, &set).await {
        warn!(%addr, %error, "The grade row could not be stored; closing");
        return false;
    }
    commit_guild_grade_step(session, addr, context, world_vid, subheader, guild_id, set).await
}

/// Ask the world to judge one grade sub-packet, or `None` when no arm owns it.
async fn judge_guild_grade_step(
    context: &ConnectionContext,
    world_vid: common::vid::Vid,
    subheader: u8,
    body: &[u8],
) -> Option<Result<GuildGradeAnswer, prodomo::game_loop_messages::MoveItemError>> {
    use protocol::cg_guild::{
        GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY, GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME,
        GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL, GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE,
        GUILD_SUBHEADER_CG_CHARGE_GSP, GUILD_SUBHEADER_CG_OFFER,
    };
    let judged = match subheader {
        GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME => {
            context
                .game
                .answer_guild_grade_name(world_vid, body[0], body[1..10].to_vec())
                .await
        }
        GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY => {
            context
                .game
                .answer_guild_grade_auth(world_vid, body[0], body[1])
                .await
        }
        GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE => {
            let pid = u32::from_le_bytes(body[..4].try_into().unwrap());
            context
                .game
                .answer_guild_member_grade(world_vid, pid, body[4])
                .await
        }
        GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL => {
            let pid = u32::from_le_bytes(body[..4].try_into().unwrap());
            context
                .game
                .answer_guild_member_general(world_vid, pid, body[4])
                .await
        }
        GUILD_SUBHEADER_CG_OFFER => {
            let offer = u32::from_le_bytes(body[..4].try_into().unwrap());
            context.game.answer_guild_offer(world_vid, offer).await
        }
        GUILD_SUBHEADER_CG_CHARGE_GSP => {
            let offer = i32::from_le_bytes(body[..4].try_into().unwrap());
            context.game.answer_guild_charge(world_vid, offer).await
        }
        _ => return None,
    };
    Some(judged)
}

/// Store one judged grade set, answering the store's error.
async fn store_guild_grade_set(
    context: &ConnectionContext,
    guild_id: u32,
    set: &GuildGradeSet,
) -> Result<bool, db::accounts::AccountError> {
    match set {
        GuildGradeSet::GradeName { grade, name } => {
            db::guilds::set_grade_name(&context.store, guild_id, *grade, name).await
        }
        GuildGradeSet::GradeAuth { grade, auth } => {
            db::guilds::set_grade_auth(&context.store, guild_id, *grade, *auth).await
        }
        GuildGradeSet::MemberGrade { pid, grade } => {
            db::guilds::set_member_grade(&context.store, *pid, *grade).await
        }
        GuildGradeSet::MemberGeneral { pid, is_general } => {
            db::guilds::set_member_general(&context.store, *pid, *is_general != 0).await
        }
        GuildGradeSet::Offer {
            pid,
            floored,
            offer,
            exp,
            level,
            ladder,
        } => {
            db::guilds::add_offer(
                &context.store,
                db::guilds::GuildOffer {
                    guild_id,
                    pid: *pid,
                    floored: *floored,
                    offer: *offer,
                    level: *level,
                    exp: *exp,
                    ladder: *ladder,
                },
            )
            .await
        }
        GuildGradeSet::Charge {
            pid,
            debit,
            power,
            gained: _,
        } => {
            db::guilds::add_power(
                &context.store,
                guild_id,
                *pid,
                *debit,
                i32::try_from(*power).unwrap_or(i32::MAX),
            )
            .await
        }
    }
}

/// Commit one stored grade set and send the commit's post, if any.
async fn commit_guild_grade_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    world_vid: common::vid::Vid,
    subheader: u8,
    guild_id: u32,
    set: GuildGradeSet,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let committed = match context
        .game
        .commit_guild_grade(GuildGradeCommit {
            speaker: world_vid,
            guild_id,
            set,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the grade step; closing");
            return false;
        }
    };
    let GuildGradeAnswer::Committed { post } = committed else {
        warn!(%addr, "The world answered a commit without one; closing");
        return false;
    };
    if let Some(post) = post {
        if let Err(error) = send_all(session, &[post]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    info!(%addr, subheader, "Guild grade step stored");
    true
}

/// `CInputMain::Guild`'s `GUILD_SUBHEADER_CG_USE_SKILL` arm (`G/input_main.cpp:3113-3119`,
/// plus `CGuild::UseSkill`, `G/guild.cpp:1267-1418`): the world judges, the descriptor
/// writes the judged power, the world commits. Every line rides the outbox already, so
/// the descriptor sends only a refused line itself.
async fn guild_use_skill_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    world_vid: common::vid::Vid,
    vnum: u32,
    pid: u32,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let judged = match context
        .game
        .answer_guild_use_skill(world_vid, vnum, pid)
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the guild cast; closing");
            return false;
        }
    };
    let (guild_id, judged_vnum, judged_pid, power) = match judged {
        GuildSkillUseAnswer::Store {
            guild_id,
            vnum,
            pid,
            power,
        } => (guild_id, vnum, pid, power),
        GuildSkillUseAnswer::Refused { line } => {
            if let Some(line) = line {
                if let Err(error) = send_all(session, &[line]).await {
                    warn!(%addr, %error, "Client session stopped");
                    return false;
                }
            }
            return true;
        }
        GuildSkillUseAnswer::Committed => {
            warn!(%addr, "The world answered a judgement with a commit; closing");
            return false;
        }
    };
    if let Err(error) = db::guilds::spend_power(
        &context.store,
        guild_id,
        i32::try_from(power).unwrap_or(i32::MAX),
    )
    .await
    {
        warn!(%addr, %error, "The guild power could not be stored; closing");
        return false;
    }
    let committed = match context
        .game
        .commit_guild_use_skill(GuildSkillUseCommit {
            speaker: world_vid,
            guild_id,
            vnum: judged_vnum,
            pid: judged_pid,
            power,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the guild cast; closing");
            return false;
        }
    };
    if committed != GuildSkillUseAnswer::Committed {
        warn!(%addr, "The world answered a commit without one; closing");
        return false;
    }
    info!(%addr, vnum, "Guild cast stored");
    true
}

/// `CInputMain::PartySetState` (`G/input_main.cpp:2522-2575`): the world sets the role
/// the record names.
async fn party_set_state_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartySetState::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_SET_STATE; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_SET_STATE from a character outside the world; ignoring");
        return true;
    };
    match context
        .game
        .party_set_state(world_vid, record.pid, record.by_role, record.flag)
        .await
    {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the set; closing");
            return false;
        }
    }
    debug!(%addr, pid = record.pid, "Party set handed to the world");
    true
}

/// `CInputMain::PartyUseSkill` (`G/input_main.cpp:2734-2761`): the world uses the party
/// skill the record names.
async fn party_use_skill_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartyUseSkill::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_USE_SKILL; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_USE_SKILL from a character outside the world; ignoring");
        return true;
    };
    match context
        .game
        .party_use_skill(world_vid, record.skill_index, record.target_vid)
        .await
    {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the use; closing");
            return false;
        }
    }
    debug!(%addr, skill = record.skill_index, "Party skill handed to the world");
    true
}

/// `CInputMain::PartyParameter` (`G/input_main.cpp:2763-2772`): the world stores the
/// distribution mode the record names.
async fn party_parameter_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgPartyParameter::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed PARTY_PARAMETER; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "PARTY_PARAMETER from a character outside the world; ignoring");
        return true;
    };
    match context
        .game
        .party_parameter(world_vid, record.distribute_mode)
        .await
    {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the mode; closing");
            return false;
        }
    }
    debug!(%addr, mode = record.distribute_mode, "Party mode handed to the world");
    true
}
/// `CInputMain::Messenger` (`G/input_main.cpp:1128-1233`): the world runs the named arm,
/// and the caller writes the pair's rows when an answer or a remove says so.
///
/// A malformed frame or a world that cannot be asked closes, as every malformed frame
/// does; a record from a character outside the world is ignored. The `ADD_BY_VID` and
/// `ADD_BY_NAME` gates that name the target run here, where the descriptor's GM level
/// and the store's grant read live; the world runs `RequestToAdd` past them. The GM
/// gate's own read is `gm_get_level` (`G/gm.cpp:12-14`), matched on the name regardless
/// of case, which the test-server mode silences to `GM_IMPLEMENTOR`.
async fn messenger_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    use protocol::cg_messenger::CgMessenger;
    let record = match CgMessenger::decode(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed MESSENGER; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "MESSENGER from a character outside the world; ignoring");
        return true;
    };
    match record {
        CgMessenger::AddByVid { vid } => {
            match context.game.messenger_add_by_vid(world_vid, vid).await {
                Ok(settling) => held.settling = Some(settling),
                Err(error) => {
                    warn!(%addr, %error, "The world could not take the add; closing");
                    return false;
                }
            }
            debug!(%addr, target = vid, "Messenger add-by-vid handed to the world");
            true
        }
        CgMessenger::AddByName { name } => {
            // The name is the C string the 24-byte field carries: legacy copies it
            // with `strlcpy` at the arm (`G/input_main.cpp:1184`), so the NUL padding
            // never reaches the lookup.
            let record = CgMessenger::AddByName { name };
            let companion = record.companion_name().to_vec();
            // `if (ch->GetGMLevel() == GM_PLAYER && gm_get_level(name) != GM_PLAYER)`
            // (`G/input_main.cpp:1186-1191`): the caller's own level is the held one,
            // and the name's is the grant the store holds, matched on the account and
            // the name. The test-server mode silences the gate.
            let companion_level = if context.modes.test_server {
                None
            } else {
                match messenger_companion_level(&context.store, addr, &companion).await {
                    Ok(level) => level,
                    Err(()) => return false,
                }
            };
            match context
                .game
                .messenger_add_by_name(world_vid, companion, companion_level)
                .await
            {
                Ok(settling) => held.settling = Some(settling),
                Err(error) => {
                    warn!(%addr, %error, "The world could not take the add; closing");
                    return false;
                }
            }
            debug!(%addr, "Messenger add-by-name handed to the world");
            true
        }
        CgMessenger::Remove { name } => {
            // The C string, as above (`G/input_main.cpp:1241`): the padding reaches
            // neither the world nor the store.
            let record = CgMessenger::Remove { name };
            let companion = record.companion_name().to_vec();
            match context
                .game
                .messenger_remove(world_vid, companion.clone())
                .await
            {
                Ok(settling) => held.settling = Some(settling),
                Err(error) => {
                    warn!(%addr, %error, "The world could not take the remove; closing");
                    return false;
                }
            }
            let Some(character) = held.character.as_ref() else {
                return true;
            };
            let Ok(companion) = std::str::from_utf8(&companion) else {
                return true;
            };
            if let Err(error) =
                db::messenger::remove_pair(&context.store, character.name.as_str(), companion).await
            {
                warn!(%addr, %error, "The friend remove could not be stored; closing");
                return false;
            }
            debug!(%addr, companion, "Messenger remove handed to the world and stored");
            true
        }
        // The default arm: `sys_err(...); break` — the base alone is consumed
        // (`G/input_main.cpp:1225-1228`).
        CgMessenger::Consumed { subheader } => {
            debug!(%addr, subheader, "Messenger subheader is not reached; consuming");
            true
        }
    }
}

/// The `gm_get_level` read for a name the `ADD_BY_NAME` arm names: the grant's authority
/// matched on the account and the name (`G/input_db.cpp:1806-1830`), or `None` for a
/// player. Answers `None` (closing) when the store refuses.
async fn messenger_companion_level(
    store: &db::store::Store,
    addr: SocketAddr,
    name: &[u8],
) -> Result<Option<u8>, ()> {
    use std::str::from_utf8;
    let Ok(text) = from_utf8(name.split(|&byte| byte == 0).next().unwrap_or(name)) else {
        return Ok(None);
    };
    let grants = match db::accounts::list_gm_grants(store).await {
        Ok(grants) => grants,
        Err(error) => {
            warn!(%addr, %error, "GM grant load failed; closing");
            return Err(());
        }
    };
    Ok(grants
        .iter()
        .find(|grant| grant.name.as_bytes().eq_ignore_ascii_case(text.as_bytes()))
        .map(|grant| u8::try_from(grant.authority.as_i32()).unwrap_or(u8::MAX)))
}

/// `CInputMain::UseSkill` (`G/input_main.cpp:2173-2177`): the world runs the character's
/// cast and sends what it shows (see `game_state::pc_skill`). The cast answers nothing
/// itself, as `CInputMain::UseSkill` ignores `UseSkill`'s answer.
async fn use_skill_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgUseSkill::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed USE_SKILL; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "USE_SKILL from a character outside the world; ignoring");
        return true;
    };
    match context.game.use_skill(world_vid, record).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the skill; closing");
            return false;
        }
    }
    debug!(%addr, skill = record.skill_vnum, "Skill handed to the world");
    true
}

/// `CInputMain::Attack` (`G/input_main.cpp:1918-2008`): the world runs the character's hit on
/// the VID the record names and sends what it shows (see `game_state::fight`). A hit that
/// stole gold answers the signed delta, which the store applies with no row changes.
async fn attack_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgAttack::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed ATTACK; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "ATTACK from a character outside the world; ignoring");
        return true;
    };
    let answer = match context.game.attack(world_vid, record).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not take the attack; closing");
            return false;
        }
    };
    // Answered, so the world has run the hit and its records are queued: nothing to settle.
    debug!(%addr, target = record.target_vid, "Attack handed to the world");
    let Some(gold) = answer.gold else {
        return true;
    };
    if let Err(error) = db::items::apply_transfer(&context.store, world_vid.raw(), &[], gold).await
    {
        warn!(%addr, %error, gold, "The hit's stolen gold could not be stored; closing");
        return false;
    }
    debug!(%addr, gold, "The hit's stolen gold was stored");
    true
}

/// `CG_FLY_TARGETING` (51) and `CG_ADD_FLY_TARGETING` (53) in the game phase:
/// `CInputMain::Attack`'s fly arms (`G/input_main.cpp:2180-2190`, `G/char_battle.cpp:3342-3371`).
/// The world stores the VID and relays what it stored; see `game_state::pc_skill`.
async fn fly_target_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgFlyTarget::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed FLY_TARGETING; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "FLY_TARGETING from a character outside the world; ignoring");
        return true;
    };
    match context.game.fly_target(world_vid, record).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not take the fly target; closing");
            return false;
        }
    }
    debug!(%addr, target = record.dw_target_vid, "Fly target handed to the world");
    true
}

/// `CG_SHOOT` (54) in the game phase: `CInputMain::Attack`'s shoot arm
/// (`G/input_main.cpp:2000-2007`, `G/char_battle.cpp:3319-3340`). The world shoots the
/// stored VIDs and sends what the shot shows; see `game_state::pc_skill`. A shot that
/// stole gold answers the signed delta, which the store applies with no row changes.
async fn shoot_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    let record = match CgShoot::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed SHOOT; closing");
            return false;
        }
    };
    let Some(world_vid) = held.world else {
        warn!(%addr, "SHOOT from a character outside the world; ignoring");
        return true;
    };
    let answer = match context.game.shoot(world_vid, record).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not take the shoot; closing");
            return false;
        }
    };
    // Answered, so the world has run the shot and its records are queued: nothing to settle.
    debug!(%addr, btype = record.shoot_type, "Shoot handed to the world");
    let Some(gold) = answer.gold else {
        return true;
    };
    if let Err(error) = db::items::apply_transfer(&context.store, world_vid.raw(), &[], gold).await
    {
        warn!(%addr, %error, gold, "The shot's stolen gold could not be stored; closing");
        return false;
    }
    debug!(%addr, gold, "The shot's stolen gold was stored");
    true
}

/// `CG_CHARACTER_POSITION` (28) in the game phase: `CInputMain::Position`
/// (`input_main.cpp:1530-1548`).
///
/// The pose record goes to the characters that see this one and then to this character,
/// because `CHARACTER::Standup` and `CHARACTER::Sitdown` call `PacketAround` with no `except`
/// (`G/char.cpp:3331`, `:3348`), and `PacketAround` finishes with the self-send.
async fn character_pose(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
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
    let Some(world_vid) = held.world else {
        warn!(%addr, "CHARACTER_POSITION from a character outside the world; ignoring");
        return true;
    };
    let record = prodomo::movement::pose_record(world_vid.raw(), position).encode();
    match context
        .game
        .relay(world_vid, vec![(RelayScope::ViewAndSelf, record)])
        .await
    {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not relay the pose; closing");
            return false;
        }
    }
    info!(%addr, position, "Pose sent to the view and the sender");
    true
}

/// `CG_SYNC_POSITION` (8) in the game phase: `CInputMain::SyncPosition`
/// (`input_main.cpp:2010-2165`).
///
/// The world judges every claim against the bodies it holds, moves the victims, and sends the
/// ownership and position records to the characters that see them. A claim the policy closes
/// the client for closes the connection here.
async fn sync_positions(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    frame: &ClientFrame,
) -> bool {
    let packet = match protocol::cg_variable::decode_sync_position(frame) {
        Ok(packet) => packet,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed SYNC_POSITION; closing");
            return false;
        }
    };
    let Some(vid) = held.world else {
        warn!(%addr, "SYNC_POSITION from a character outside the world; ignoring");
        return true;
    };
    match context.game.sync_positions(vid, packet).await {
        Ok(Some(result)) => {
            if let Some(reason) = result.close_reason {
                warn!(%addr, ?reason, "SYNC_POSITION refused the client; closing");
                return false;
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
        // A character with no body has nothing to sync.
        Ok(None) => true,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the sync; closing");
            false
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
        // `CInputHandshake::Analyze`'s mark arm (`G/input.cpp:211-226`): on a mark
        // server the downloader's login moves to the login phase; elsewhere the
        // connection closes. The handle and key ride unread, as legacy never reads
        // them; the frame must still be whole, so a short login closes as a
        // malformed record does.
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Handshake && frame.header == HEADER_CG_MARK_LOGIN.value() =>
        {
            mark_login_handshake(session, addr, context, &frame).await
        }
        // `CInputAuth::Analyze` (`G/input_auth.cpp:209-217`).
        LiveStep::Record { phase, frame }
            if phase == ClientPhase::Auth && frame.header == HEADER_CG_LOGIN3.value() =>
        {
            auth_login(session, addr, context, &frame, &mut held.claim).await
        }
        // The mark arms of `CInputLogin::Analyze` (`G/input_login.cpp:1207-1236`,
        // `ENABLE_HEADER_100_FIX` defined): the mark login is consumed, the CRC list,
        // the index list, the mark upload, the symbol upload, and the symbol CRC.
        LiveStep::Record { phase, frame }
            if matches!(
                phase,
                ClientPhase::Login | ClientPhase::Select | ClientPhase::Loading
            ) && is_mark_record(frame.header) =>
        {
            match seat {
                Some(seat) => mark_step(session, addr, context, &seat, &frame).await,
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
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
            if phase == ClientPhase::Loading && frame.header == HEADER_CG_ENTERGAME.value() =>
        {
            match seat {
                Some(seat) => enter_game(session, addr, context, &seat, &frame, held).await,
                None => report_step(addr, LiveStep::Record { phase, frame }),
            }
        }
        // The records a phase's analyzer consumes with nothing done, see [`is_consumed`].
        LiveStep::Record { phase, frame } if is_consumed(phase, frame.header) => {
            debug!(%addr, ?phase, header = frame.header, "consumed with nothing done");
            true
        }
        // The main input and the dead phase's refusal, see [`main_record`].
        LiveStep::Record { phase, frame } => {
            main_step(session, addr, context, held, phase, frame).await
        }
        step => report_step(addr, step),
    }
}

/// Runs the main-phase record at `frame`, after [`main_record`] has sorted it.
async fn main_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    phase: ClientPhase,
    frame: ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match main_record(phase, frame.header) {
        Some(MainRecord::Refused) => {
            debug!(%addr, header = frame.header, "a dead player's pose is refused");
            true
        }
        Some(MainRecord::Chat) => chat_line(session, addr, context, held, &frame).await,
        Some(MainRecord::Whisper) => whisper_line(addr, context, held, &frame).await,
        Some(MainRecord::ItemMove) => move_an_item(session, addr, context, held, &frame).await,
        Some(MainRecord::Belongings) => belongings_step(session, addr, context, held, &frame).await,
        Some(MainRecord::World) => world_step(addr, context, held, &frame).await,
        Some(MainRecord::Npc) => npc_step(session, addr, context, held, &frame).await,
        Some(MainRecord::Messenger) => messenger_step(addr, context, held, &frame).await,
        Some(MainRecord::Guild) => {
            if frame.header == HEADER_CG_GUILD.value() {
                cg_guild_step(session, addr, context, held, &frame).await
            } else {
                answer_make_guild_step(session, addr, context, held, &frame).await
            }
        }
        Some(MainRecord::Transaction) => {
            transaction_record(session, addr, context, held, &frame).await
        }
        Some(MainRecord::Warp) => warp_end(addr, &frame),
        None => report_step(addr, LiveStep::Record { phase, frame }),
    }
}

/// The analyzer of the main input a record goes to, see [`main_record`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainRecord {
    /// B25: a dead player that sits leaves `POS_DEAD`, and its stand revives it with no penalty
    /// and no wait (`G/input_main.cpp:1530-1548`, `G/char.cpp:3335-3349`). Its
    /// `CG_CHARACTER_POSITION` is refused while dead.
    Refused,
    /// `CG_CHAT` (3) in the game and dead phases: `CInputMain::Chat` (`G/input_main.cpp:781`).
    /// `PHASE_DEAD` installs the main input as `PHASE_GAME` does (`G/desc.cpp:526-531`);
    /// `CInputDead` is never installed (B14).
    Chat,
    /// `CG_WHISPER` (19) in the game and dead phases: `CInputMain::Whisper`
    /// (`G/input_main.cpp:340-649`). The main input reads it in the dead phase as it reads
    /// the chat, because `PHASE_DEAD` installs the main input (B14).
    Whisper,
    /// `CG_ITEM_MOVE` (13): `CInputMain::ItemMove` (`G/input_main.cpp:1060-1066`), reached when
    /// the character is not an observer (`:3707-3710`). The Rewrite has no observer mode, so
    /// every character is reached.
    ItemMove,
    /// `CG_ITEM_USE` (11), the ground steps and the quickslot requests.
    Belongings,
    /// `CG_ATTACK` (2), `CG_MOVE` (7), `CG_SYNC_POSITION` (8), `CG_CHARACTER_POSITION` (28) and
    /// `CG_TARGET` (61).
    World,
    /// `CG_ON_CLICK` (26), `CG_SHOP` (50), `CG_SCRIPT_ANSWER` (29), `CG_QUEST_INPUT_STRING`
    /// (30) and `CG_SCRIPT_BUTTON` (66).
    Npc,
    /// `CG_MESSENGER` (67): `CInputMain::Messenger` (`G/input_main.cpp:1128-1233`).
    Messenger,
    /// `CG_ANSWER_MAKE_GUILD` (49): `CInputMain::AnswerMakeGuild`
    /// (`G/input_main.cpp:2672-2730`), read through the same main input
    /// (`G/input_main.cpp:3834-3836`).
    Guild,
    /// `CG_EXCHANGE` (27).
    Transaction,
    /// `CG_WARP` (65): `CInputMain::Warp` (`G/input_main.cpp:2271-2274`), which
    /// `CInputMain::Analyze` calls (`:3801-3802`).
    Warp,
}

/// Which analyzer of the main input takes a record of `header` in `phase`: the dead phase's
/// refusal first, then the main input's records, which the game and the dead phases read alike.
/// `None` for a record neither handles, and for every record of another phase.
fn main_record(phase: ClientPhase, header: u8) -> Option<MainRecord> {
    if is_dead_refused(phase, header) {
        return Some(MainRecord::Refused);
    }
    if !in_main_input(phase) {
        return None;
    }
    let record = if header == HEADER_CG_CHAT.value() {
        MainRecord::Chat
    } else if header == HEADER_CG_WHISPER.value() {
        MainRecord::Whisper
    } else if header == HEADER_CG_ITEM_MOVE.value() {
        MainRecord::ItemMove
    } else if is_belongings_step(header) {
        MainRecord::Belongings
    } else if is_world_step(header) {
        MainRecord::World
    } else if is_npc_step(header) {
        MainRecord::Npc
    } else if header == HEADER_CG_MESSENGER.value() {
        MainRecord::Messenger
    } else if header == HEADER_CG_ANSWER_MAKE_GUILD.value() || header == HEADER_CG_GUILD.value() {
        MainRecord::Guild
    } else if is_transaction_record(header) {
        MainRecord::Transaction
    } else if header == HEADER_CG_WARP.value() {
        MainRecord::Warp
    } else {
        return None;
    };
    Some(record)
}

/// Whether `header` is one of the six mark records `CInputLogin::Analyze` handles
/// (`G/input_login.cpp:1207-1236`): the mark login, the CRC list, the index list, the
/// mark upload, the symbol upload, and the symbol CRC.
fn is_mark_record(header: u8) -> bool {
    header == HEADER_CG_MARK_LOGIN.value()
        || header == HEADER_CG_MARK_CRCLIST.value()
        || header == HEADER_CG_MARK_IDXLIST.value()
        || header == HEADER_CG_MARK_UPLOAD.value()
        || header == HEADER_CG_GUILD_SYMBOL_UPLOAD.value()
        || header == HEADER_CG_SYMBOL_CRC.value()
}

/// Whether `phase`'s analyzer consumes a record of `header` and does nothing with it:
///
/// - `CG_ITEM_USE` (11) and `CG_TARGET` (61) in the login, select and loading phases, which a
///   client still sends for the character it left (`@fixme120`, `G/input_login.cpp:1254-1257`);
/// - `CG_ENTER_GAME` (10) in the game and dead phases: the header is in the main table
///   (`G/packet_info.cpp:116`), and `CInputMain::Analyze` has no case for it, so its `default`
///   consumes the record (`G/input_main.cpp:4126-4127`);
/// - The fly and shoot records (51/53/54) are not consumed: they run the world steps (230e).
fn is_consumed(phase: ClientPhase, header: u8) -> bool {
    match phase {
        ClientPhase::Login | ClientPhase::Select | ClientPhase::Loading => {
            header == HEADER_CG_ITEM_USE.value() || header == HEADER_CG_TARGET.value()
        }
        ClientPhase::Game | ClientPhase::Dead => header == HEADER_CG_ENTERGAME.value(),
        _ => false,
    }
}

/// Whether `phase` reads the main input: the game phase, and the dead phase, which legacy gives
/// the same input processor (`G/desc.cpp:526-531`).
const fn in_main_input(phase: ClientPhase) -> bool {
    matches!(phase, ClientPhase::Game | ClientPhase::Dead)
}

/// Whether the dead phase refuses a record of `header`: `CG_CHARACTER_POSITION` (B25).
fn is_dead_refused(phase: ClientPhase, header: u8) -> bool {
    phase == ClientPhase::Dead && header == HEADER_CG_CHARACTER_POSITION.value()
}

/// Whether `header` is a record the world answers alone: one that moves a character,
/// `CG_TARGET`, `CG_USE_SKILL`, `CG_ATTACK`, `CG_FLY_TARGETING`,
/// `CG_ADD_FLY_TARGETING` or `CG_SHOOT`.
fn is_world_step(header: u8) -> bool {
    is_motion_step(header)
        || header == HEADER_CG_TARGET.value()
        || header == HEADER_CG_PARTY_INVITE.value()
        || header == HEADER_CG_PARTY_INVITE_ANSWER.value()
        || header == HEADER_CG_PARTY_REMOVE.value()
        || header == HEADER_CG_PARTY_SET_STATE.value()
        || header == HEADER_CG_PARTY_USE_SKILL.value()
        || header == HEADER_CG_PARTY_PARAMETER.value()
        || header == HEADER_CG_USE_SKILL.value()
        || header == HEADER_CG_ATTACK.value()
        || header == HEADER_CG_FLY_TARGETING.value()
        || header == HEADER_CG_ADD_FLY_TARGETING.value()
        || header == HEADER_CG_SHOOT.value()
}

/// Run a step [`is_world_step`] accepts.
async fn world_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    if frame.header == HEADER_CG_TARGET.value() {
        target_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_INVITE.value() {
        party_invite_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_INVITE_ANSWER.value() {
        party_invite_answer_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_REMOVE.value() {
        party_remove_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_SET_STATE.value() {
        party_set_state_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_USE_SKILL.value() {
        party_use_skill_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_PARTY_PARAMETER.value() {
        party_parameter_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_USE_SKILL.value() {
        use_skill_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_ATTACK.value() {
        attack_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_FLY_TARGETING.value()
        || frame.header == HEADER_CG_ADD_FLY_TARGETING.value()
    {
        fly_target_step(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_SHOOT.value() {
        shoot_step(addr, context, held, frame).await
    } else {
        motion_step(addr, context, held, frame).await
    }
}

/// Whether `header` is one of the records that move a character.
fn is_motion_step(header: u8) -> bool {
    header == HEADER_CG_MOVE.value()
        || header == HEADER_CG_SYNC_POSITION.value()
        || header == HEADER_CG_CHARACTER_POSITION.value()
}

/// Run a step [`is_motion_step`] accepts:
///
/// - `CG_MOVE` (7): `CInputMain::Move` (`G/input_main.cpp:1757`).
/// - `CG_SYNC_POSITION` (8): `CInputMain::SyncPosition` (`G/input_main.cpp:2010`).
/// - `CG_CHARACTER_POSITION` (28): `CInputMain::Position` (`G/input_main.cpp:1530`).
async fn motion_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool {
    if frame.header == HEADER_CG_MOVE.value() {
        move_character(addr, context, held, frame).await
    } else if frame.header == HEADER_CG_SYNC_POSITION.value() {
        sync_positions(addr, context, held, frame).await
    } else {
        character_pose(addr, context, held, frame).await
    }
}

/// Whether `header` is one of the records an NPC is used through: a shop's or a quest's.
fn is_npc_step(header: u8) -> bool {
    is_shop_step(header) || is_quest_step(header)
}

/// Whether `header` is `CG_ON_CLICK` or `CG_SHOP`, the two records a shop is used through.
fn is_shop_step(header: u8) -> bool {
    header == HEADER_CG_ON_CLICK.value() || header == HEADER_CG_SHOP.value()
}

/// Run a shop step or a quest step.
async fn npc_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if is_quest_step(frame.header) {
        quest_step(session, addr, context, held, frame).await
    } else {
        shop_step(session, addr, context, held, frame).await
    }
}

/// `CG_ON_CLICK` (26) and `CG_SHOP` (50) in the game phase: `CInputMain::OnClick` and
/// `CInputMain::Shop` (`G/input_main.cpp:1241-1316`), which no observer check guards
/// (`:3756-3771`).
///
/// `CHARACTER::OnClick` (`G/char.cpp:6181-6352`) runs the quest click and then the NPC's click
/// trigger. A quest that takes the click answers with its dialog. Only the trigger that opens a
/// shop is ported, so any other click no quest takes is logged and ignored, as a click on nothing
/// is in legacy. A buy or a sale is stored and sent as a move is, with the gold it left; a
/// malformed record closes the connection.
async fn shop_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let decoded = if frame.header == HEADER_CG_ON_CLICK.value() {
        CgOnClick::decode_frame(frame)
            .map(|record| Some(ShopStep::Click { target: record.vid }))
            .map_err(|error| format!("{error:?}"))
    } else {
        CgShop::decode_frame(frame)
            .map(ShopStep::requested)
            .map_err(|error| error.to_string())
    };
    let step = match decoded {
        Ok(Some(step)) => step,
        Ok(None) => {
            let subheader = frame.payload.first();
            info!(%addr, ?subheader, "Client sent an unknown SHOP subheader; ignoring");
            return true;
        }
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed shop record; closing");
            return false;
        }
    };
    let (Some(vid), Some(place)) = (held.world, ground_place(held)) else {
        info!(%addr, ?step, "Shop step without a character in the world; ignoring");
        return true;
    };
    let actor = item_actor(held);
    let answer = match context.game.shop(vid, step, place, actor).await {
        Ok(Ok(ShopAnswer::Moved(moved))) => Ok(Ok(moved)),
        Ok(Ok(ShopAnswer::Sent(records))) => {
            info!(%addr, ?step, "Shop step answered");
            return send_shop_records(session, addr, &records).await;
        }
        Ok(Ok(ShopAnswer::Declined { reason, records })) => {
            info!(%addr, ?step, ?reason, "Shop step declined");
            return send_shop_records(session, addr, &records).await;
        }
        Ok(Ok(ShopAnswer::Quest {
            records,
            climbs,
            guild_ops,
        })) => {
            info!(%addr, ?step, "A quest took the click");
            if !store_quest_climbs(addr, context, held, &climbs).await {
                return false;
            }
            if !store_quest_guild_stores(session, addr, context, held, &guild_ops).await {
                return false;
            }
            return send_shop_records(session, addr, &records).await;
        }
        Ok(Err(refused)) => Ok(Err(refused)),
        Err(error) => Err(error),
    };
    finish_item_step(session, addr, context, held, actor, answer).await
}

/// Whether `header` is `CG_SCRIPT_ANSWER`, `CG_QUEST_INPUT_STRING` or `CG_SCRIPT_BUTTON`,
/// the three records a quest's dialog is answered with.
fn is_quest_step(header: u8) -> bool {
    header == HEADER_CG_SCRIPT_ANSWER.value()
        || header == HEADER_CG_QUEST_INPUT_STRING.value()
        || header == HEADER_CG_SCRIPT_BUTTON.value()
}

/// `CG_SCRIPT_ANSWER` (29) and `CG_QUEST_INPUT_STRING` (30) in the game phase:
/// `CInputMain::ScriptAnswer` and `QuestInputString` (`G/input_main.cpp:2200-2234`), which no
/// observer check guards (`:3786-3791`).
///
/// The world runs the character's quest step and answers the dialog and the chat lines its
/// script sent; a malformed record closes the connection. A GRAND read that climbed is
/// stored before the dialog is sent, as a book read's is.
/// Store each GRAND climb a quest step landed, in order: a failure closes, as a
/// book read's does. Answers whether the step may be sent.
async fn store_quest_climbs(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    climbs: &[(u8, u8, i32)],
) -> bool {
    if climbs.is_empty() {
        return true;
    }
    let Some(character) = held.character.as_ref() else {
        return true;
    };
    for (vnum, level, next_read) in climbs {
        if let Err(error) =
            db::player_skills::store_skill(&context.store, character.id, *vnum, *level, *next_read)
                .await
        {
            warn!(%addr, %error, vnum, level, "A quest climb could not be stored; closing");
            return false;
        }
    }
    true
}

/// Drain one quest step's `guild.*` stores, in order: the same writes the wire arms'
/// descriptors drain for the same judges — the master's three rows, the comment's
/// insert plus the board read and its commit (which also lands the post's records),
/// the ladder's row, the skills' whole-row write, the offer's row (the world already
/// fanned the commit), the grant's row. Answers whether the step may be sent.
async fn store_quest_guild_stores<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    stores: &[GuildQuestStore],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for store in stores {
        match store {
            GuildQuestStore::Master { guild_id, from, to } => {
                if let Err(error) =
                    db::guilds::set_master(&context.store, *guild_id, *from, *to).await
                {
                    warn!(%addr, %error, "A quest master change could not be stored; closing");
                    return false;
                }
            }
            GuildQuestStore::Comment {
                guild_id, content, ..
            } => {
                if !store_quest_guild_comment(session, addr, context, held, *guild_id, content)
                    .await
                {
                    return false;
                }
            }
            GuildQuestStore::Ladder { guild_id, ladder } => {
                if let Err(error) = db::guilds::set_ladder(&context.store, *guild_id, *ladder).await
                {
                    warn!(%addr, %error, "A quest ladder could not be stored; closing");
                    return false;
                }
            }
            GuildQuestStore::Skills {
                guild_id,
                skills,
                skill_point,
            } => {
                if let Err(error) =
                    db::guilds::set_skills(&context.store, *guild_id, skills, *skill_point).await
                {
                    warn!(%addr, %error, "Quest guild skills could not be stored; closing");
                    return false;
                }
            }
            GuildQuestStore::Offer {
                pid,
                floored,
                offer,
                guild_id,
                exp,
                level,
                ladder,
            } => {
                if let Err(error) = db::guilds::add_offer(
                    &context.store,
                    db::guilds::GuildOffer {
                        guild_id: *guild_id,
                        pid: *pid,
                        floored: *floored,
                        offer: *offer,
                        level: *level,
                        exp: *exp,
                        ladder: *ladder,
                    },
                )
                .await
                {
                    warn!(%addr, %error, "A quest offer could not be stored; closing");
                    return false;
                }
            }
            GuildQuestStore::Grant {
                guild_id,
                exp,
                level,
                ladder,
            } => {
                if let Err(error) =
                    db::guilds::grant_exp(&context.store, *guild_id, *exp, *level, *ladder).await
                {
                    warn!(%addr, %error, "A quest grant could not be stored; closing");
                    return false;
                }
            }
        }
    }
    true
}

/// Store one quest comment's post: the row, the board read, and the commit that lands
/// the post's records. Answers whether the step may be sent.
async fn store_quest_guild_comment<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    guild_id: u32,
    posted: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(character) = held.character.as_ref() else {
        warn!(%addr, "A quest comment without a character; closing");
        return false;
    };
    let name = character.name.clone();
    let text = String::from_utf8_lossy(posted).into_owned();
    if let Err(error) = db::guilds::post_comment(&context.store, guild_id, &name, &text).await {
        warn!(%addr, %error, "A quest comment could not be posted; closing");
        return false;
    }
    let board = match db::guilds::board(&context.store, guild_id, GUILD_COMMENT_MAX_COUNT).await {
        Ok(board) => board,
        Err(error) => {
            warn!(%addr, %error, "A quest board could not be read; closing");
            return false;
        }
    };
    let Some(world) = held.world else {
        warn!(%addr, "A quest comment without a character in the world; closing");
        return false;
    };
    let lines: Vec<GuildCommentLine> = board
        .iter()
        .map(|comment| GuildCommentLine {
            id: comment.id,
            name: comment.name.as_bytes().to_vec(),
            content: comment.content.as_bytes().to_vec(),
        })
        .collect();
    let committed = match context
        .game
        .commit_guild_post_comment(GuildCommentCommit::Post {
            speaker: world,
            guild_id,
            board: lines,
        })
        .await
    {
        Ok(committed) => committed,
        Err(error) => {
            warn!(%addr, %error, "The world could not commit the quest post; closing");
            return false;
        }
    };
    let GuildCommentAnswer::Done { board, line: _ } = committed else {
        warn!(%addr, "The world answered a quest commit without a board; closing");
        return false;
    };
    if let Some(board) = board {
        if let Err(error) = send_all(session, &[guild_comments_packet(&board)]).await {
            warn!(%addr, %error, "Client session stopped");
            return false;
        }
    }
    true
}

/// `CG_SCRIPT_ANSWER` (29), `CG_QUEST_INPUT_STRING` (30) and `CG_SCRIPT_BUTTON` (66) in the
/// game phase: `CInputMain::ScriptAnswer`, `QuestInputString` and `ScriptButton`
/// (`G/input_main.cpp:2179-2234`), which no observer check guards (`:3780-3791`).
///
/// The world runs the character's quest step and answers the dialog and the chat lines its
/// script sent; a malformed record closes the connection. A GRAND read that climbed is
/// stored before the dialog is sent, as a book read's is.
async fn quest_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let decoded = if frame.header == HEADER_CG_SCRIPT_ANSWER.value() {
        CgScriptAnswer::decode_frame(frame)
            .map(|record| QuestStep::Answer(record.answer))
            .map_err(|error| error.to_string())
    } else if frame.header == HEADER_CG_SCRIPT_BUTTON.value() {
        CgScriptButton::decode_frame(frame)
            .map(|record| QuestStep::Button(record.idx))
            .map_err(|error| error.to_string())
    } else {
        CgQuestInputString::decode_frame(frame)
            .map(|record| QuestStep::input(&record))
            .map_err(|error| format!("{error:?}"))
    };
    let step = match decoded {
        Ok(step) => step,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed quest record; closing");
            return false;
        }
    };
    let (Some(vid), Some(place)) = (held.world, ground_place(held)) else {
        info!(%addr, ?step, "Quest step without a character in the world; ignoring");
        return true;
    };

    match context.game.quest(vid, step, place, item_actor(held)).await {
        Ok(Ok((records, climbs, stores))) => {
            if !store_quest_climbs(addr, context, held, &climbs).await {
                return false;
            }
            if !store_quest_guild_stores(session, addr, context, held, &stores).await {
                return false;
            }
            send_shop_records(session, addr, &records).await
        }
        Ok(Err(refused)) => {
            warn!(%addr, %refused, "The world does not hold this descriptor's character; closing");
            false
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a quest step; closing");
            false
        }
    }
}

/// `CG_EXCHANGE` (27) in the game phase: `CInputMain::Exchange` (`G/input_main.cpp:1367-1527`).
///
/// The world runs the step and writes the other side's records to its client. A trade both
/// sides accepted is stored for both in one transaction before either is sent its moves; a
/// malformed record closes the connection.
async fn trade_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let step = match CgExchange::decode_frame(frame) {
        Ok(record) => TradeStep::requested(record),
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed exchange record; closing");
            return false;
        }
    };
    let Some(step) = step else {
        let subheader = frame.payload.first();
        info!(%addr, ?subheader, "Client sent an unknown EXCHANGE subheader; ignoring");
        return true;
    };
    let (Some(vid), Some(place)) = (held.world, ground_place(held)) else {
        info!(%addr, ?step, "Trade step without a character in the world; ignoring");
        return true;
    };
    match context.game.trade(vid, step, place, item_actor(held)).await {
        Ok(Ok(TradeAnswer::Sent(records))) => {
            info!(%addr, ?step, "Trade step answered");
            send_shop_records(session, addr, &records).await
        }
        Ok(Ok(TradeAnswer::Declined { reason, records })) => {
            info!(%addr, ?step, ?reason, "Trade step declined");
            send_shop_records(session, addr, &records).await
        }
        Ok(Ok(TradeAnswer::Settled(settled))) => {
            finish_trade(session, addr, context, held, *settled).await
        }
        Ok(Err(refused)) => {
            warn!(%addr, %refused, "The world does not hold this descriptor's character; closing");
            false
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a trade step; closing");
            false
        }
    }
}

/// Store both sides of a settled trade in one transaction (ADR-0003), then send the other
/// side's records to its client and this side's to this one.
///
/// The world has settled the trade already, so a failed write closes this descriptor and the
/// other side keeps what the world gave it until it relogs.
async fn finish_trade<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    settled: TradeSettled,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let TradeSettled {
        own,
        partner,
        partner_outbox,
    } = settled;
    let sides = [&own, &partner].map(|moved| db::items::TransferSide {
        owner_id: moved.owner_id,
        changes: &moved.changes,
        gold: moved.gold.unwrap_or(0),
    });
    if let Err(error) = db::items::apply_exchange(&context.store, &sides).await {
        warn!(%addr, %error, "The trade could not be stored; closing");
        return false;
    }
    let rows = own.changes.len() + partner.changes.len();
    if let Some(quickslots) = own.quickslots {
        held.quickslots = quickslots;
    }
    if let Some(outbox) = partner_outbox {
        for record in partner.records {
            let _sent = outbox.send(record);
        }
    }
    if let Err(error) = send_all(session, &own.records).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    info!(%addr, rows, "Trade settled");
    true
}

/// Send the records a shop or trade step that stored nothing answers; `false` when the session
/// stopped.
async fn send_shop_records<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    records: &[Vec<u8>],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Err(error) = send_all(session, records).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
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
    let no_more_clients = context.refusals.shutdowned || context.state.is_shutting_down();
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
    let linked = match select_guilds(&context.store, &account_lobby).await {
        Ok(linked) => linked,
        Err(error) => {
            error!(%addr, %error, "Channel login failed; closing");
            return false;
        }
    };
    let guilds = select_guild_views(&linked);
    let success = login_success(
        &account_lobby,
        &context.atlas,
        seat.locations,
        &guilds,
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
        language: grant.language,
        lobby: account_lobby,
    });
    send_select_list(session, addr, &empire, &success).await
}

/// Send the select screen's two records — `GC_EMPIRE` and the character list — and move the
/// session to the select phase. `false` closes the connection.
async fn send_select_list<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    empire: &[u8],
    success: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let delivery = async {
        session.send(empire).await?;
        session.send(success).await?;
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

/// The linked guild of each lobby slot's character, as `login_success` takes it: the id and
/// the name bytes (`db::guilds::lobby_guilds`). `None` answers the store error as a string,
/// for the select-phase callers that return [`SelectReplies`].
async fn select_guilds(
    store: &db::store::Store,
    lobby: &db::players::Lobby,
) -> Result<[Option<(u32, String)>; PLAYER_SLOTS], String> {
    db::guilds::lobby_guilds(store, lobby)
        .await
        .map_err(|error| error.to_string())
}

/// The linked guilds as `login_success` takes them: each id with its name bytes.
fn select_guild_views(
    linked: &[Option<(u32, String)>; PLAYER_SLOTS],
) -> [Option<(u32, &[u8])>; PLAYER_SLOTS] {
    std::array::from_fn(|slot| {
        linked[slot]
            .as_ref()
            .map(|(id, name)| (*id, name.as_bytes()))
    })
}

/// The select-screen headers.
const SELECT_HEADERS: [u8; 4] = [
    HEADER_CG_EMPIRE.value(),
    HEADER_CG_CHARACTER_CREATE.value(),
    HEADER_CG_CHARACTER_DELETE.value(),
    HEADER_CG_CHANGE_NAME.value(),
];

/// The characters the loading burst's `GC_ENTITY` lists: none (V7).
///
/// The characters and NPCs around are shown when the character enters the game
/// ([`entering_burst`]), as legacy's `Show` inserts them then. `Neighbourhood::default` is that
/// empty list, and the burst takes it the same way it would take a populated one.
fn empty_view() -> Neighbourhood {
    Neighbourhood::default()
}

/// The time `get_global_time()` reports: `time(0)` plus the game server's gap.
///
/// The Rewrite has no accumulated game time, so this is the wall clock. Legacy's
/// `get_global_time` adds a gap that starts at zero and grows with uptime (`G/game.cpp:106`),
/// which a client that shows a countdown would read differently; that is a Divergence until
/// the world system owns the clock.
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
    let Ok(selected) = the_selected_character(context, addr, held, frame).await else {
        return false;
    };
    let Some((character, player)) = selected else {
        // The selection was ignored, and the connection stays open.
        return true;
    };
    let Some(account) = held.account.as_ref() else {
        // The account table was there when the character was selected.
        return false;
    };
    let account_id = account.id;
    let vid = character_vid(&character);
    let view = empty_view();
    // `PlayerLoad` leaves the character on the map its saved position belongs to, and
    // `SetPlayerProto` computes the points there, because the map's conqueror will is part of
    // them.
    //
    // The row's own map index comes first, as legacy's `PlayerLoad` reads it
    // (`G/input_db.cpp:335-354`): a private map's index names the copy of a base map the
    // character stands on, and the position lies in the base map's coordinates, so the atlas
    // can only answer with the public map.
    let named = if character.map_index != 0 {
        Some(character.map_index)
    } else {
        context.atlas.index_at(character.x, character.y)
    };
    // `GetValidLocation` (`G/sectree_manager.cpp:816-842`): a private map the world does
    // not host is the base map it was copied from, because the copy a guild war is fought
    // on died with the process that held it. The world is asked, because only the world
    // knows which private maps it holds.
    let Ok(map_index) = resolved_map_index(context, seat, addr, named).await else {
        return false;
    };
    // `SetPlayerProto` (`G/char.cpp:2289-2294`): a character whose row names a private map
    // comes back to the place it came from, because the copy it stood on is gone with the
    // process that held it. Legacy holds that place as the pending Warp, so the next save
    // writes the row there and the war's end sends the character back.
    if map_index.is_some_and(|index| index >= prodomo::loading_phase::INSTANCE_MAP_BASE)
        && (character.exit_x != 0 || character.exit_y != 0)
    {
        held.warp = Some(prodomo::warp::WarpTarget {
            map_index: character.exit_map_index,
            x: character.exit_x,
            y: character.exit_y,
        });
    }
    let mut points = load_points(&character, map_index.unwrap_or(0));
    // `d->BindCharacter(ch)` is the step that makes the character the descriptor's own, and it
    // is what `CInputLogin::Entergame` later reads. Without it `CG_ENTER_GAME` has nothing to
    // answer, so the loaded row is held here and kept for the descriptor's life.
    let mut burst = loading_burst(&character, &points, vid, &view);
    // `PlayerLoad` sets each stored quickslot right after the map test and before the points
    // packet (`G/input_db.cpp:439-440`), and `TPlayerTable::skills` rides the player row.
    let Some((quickslots, skills, quest_flags, affects)) =
        load_select_rows(&context.store, addr, player, &mut burst).await
    else {
        return false;
    };
    held.quickslots = quickslots;
    held.skills = skills;
    held.quest_flags = quest_flags;
    held.affects = affects;
    held.character = Some(character.clone());
    held.channel = seat.number;
    // `SetPlayerProto`'s GM level (`G/input_db.cpp:1931-1955`): the character's own
    // grant, matched on the account and the name (`gm_get_level`,
    // `G/input_db.cpp:1806-1830`). The Rewrite keeps the grant in the store's
    // `gm_grant` table (ledger 179) and asks for it with the account and the name, so
    // a grant never reaches another account's character.
    held.gm_level = match gm_level_at_select(
        &context.store,
        addr,
        account_id,
        player,
        character.name.as_str(),
    )
    .await
    {
        Ok(level) => level,
        Err(()) => return false,
    };
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
    let Some(items) = load_items(session, addr, context, &character, &mut points, vid).await else {
        return false;
    };
    held.items = items;
    // The item load's `CheckMaximumPoints` may have brought the pools down, and the worn items
    // set the parts; the save writes both from the held row (`G/char.cpp:1606`).
    hold_points(held, points);
    // `CHARACTER::Initialize` sets `m_dwLastSkillTime` when the character is created
    // (`G/char.cpp:380`), which is the select.
    held.selected_at = Some(tokio::time::Instant::now());
    // The index the map test resolved is the character's map. It is held here because the
    // first thing the game phase does with it is join the client set.
    held.map = map_index;
    true
}

/// The character the client selected, with the player's id.
///
/// `Ok(None)` when the selection was ignored and the connection stays open, `Err` when the
/// descriptor must close.
async fn the_selected_character(
    context: &ConnectionContext,
    addr: SocketAddr,
    held: &Held,
    frame: &ClientFrame,
) -> Result<Option<(Character, u32)>, ()> {
    let record = match CgPlayerSelect::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed CHARACTER_SELECT; closing");
            return Err(());
        }
    };
    let Some(account) = held.account.as_ref() else {
        // No account table means the descriptor never completed a Channel login. Legacy's
        // `CharacterSelect` reads `c_r.players[pinfo->index]` straight away, which a client
        // that skipped the login can reach; there is nothing to select from here.
        warn!(%addr, "CHARACTER_SELECT without an account table; ignoring");
        return Ok(None);
    };
    let slots = account.lobby.slot_ids();
    let account_id = account.id;
    let player = match judge_select(record.index, Some(&slots)) {
        SelectVerdict::Ignore => return Ok(None),
        SelectVerdict::Close => {
            // `SetPhase(PHASE_CLOSE)` writes no record, so the descriptor is simply
            // dropped: `analyze` returns false and the read loop breaks.
            info!(%addr, slot = record.index, "Empty character slot; closing");
            return Err(());
        }
        SelectVerdict::Load { player } => player,
    };
    let character = match load_character(&context.store, account_id, player).await {
        Ok(character) => character,
        Err(error) => {
            warn!(%addr, account = account_id.get(), player, %error, "Character load failed; closing");
            return Err(());
        }
    };
    if held.character.is_some() {
        // `PlayerLoad` refuses a second character on one descriptor and logs it. The client is
        // not asked to leave, so the descriptor stays in the loading phase with the character
        // it already has.
        error!(%addr, player, "Login state already has a character; ignoring the selection");
        return Ok(None);
    }
    Ok(Some((character, player)))
}

/// The map the character comes back to, with a private map folded onto its base map.
///
/// `GetValidLocation` (`G/sectree_manager.cpp:816-842`): a private map the world does not
/// host is the base map it was copied from, because the copy a guild war is fought on died
/// with the process that held it. The world is asked, because only the world knows which
/// private maps it holds.
///
/// `Err` when the world could not be asked, which closes the descriptor.
async fn resolved_map_index(
    context: &ConnectionContext,
    seat: &ChannelSeat<'_>,
    addr: SocketAddr,
    named: Option<i32>,
) -> Result<Option<i32>, ()> {
    let Some(index) = named else {
        return Ok(None);
    };
    if index < prodomo::loading_phase::INSTANCE_MAP_BASE {
        return Ok(Some(index));
    }
    match context.game.map_of(seat.number, index).await {
        Ok(resolved) => Ok(Some(resolved)),
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for the map; closing");
            Err(())
        }
    }
}

/// The character's GM level at the select, or `Err` when the grant could not be read, which
/// closes the descriptor.
async fn gm_level_at_select(
    store: &Store,
    addr: SocketAddr,
    account: AccountId,
    player: u32,
    name: &str,
) -> Result<u8, ()> {
    match read_gm_level(store, account, name).await {
        Ok(level) => Ok(level),
        Err(error) => {
            warn!(%addr, player, %error, "GM grant load failed; closing");
            Err(())
        }
    }
}

/// The character's GM level at select: `SetPlayerProto`'s read (`G/input_db.cpp:1931-1955`),
/// the character's own grant matched on the account and the name (`gm_get_level`,
/// `G/input_db.cpp:1806-1830`). The Rewrite keeps the grant in the store's `gm_grant` table
/// (ledger 179), so a grant never reaches another account's character.
///
/// # Errors
///
/// Returns [`AccountError`] when the grant read fails.
async fn read_gm_level(store: &Store, account: AccountId, name: &str) -> Result<u8, AccountError> {
    match db::accounts::gm_authority(store, account, name).await? {
        Some(authority) => Ok(u8::try_from(authority.as_i32()).unwrap_or(GM_PLAYER)),
        None => Ok(GM_PLAYER),
    }
}

/// The rows `PlayerLoad` reads after its map test and before its points packet: the stored
/// quickslots (`G/input_db.cpp:439-440`, spliced to the front of `after_map_test`) and the
/// skill levels, which ride the player row here. Answers the loaded rows, or `None` when a
/// load failed — which closes the descriptor.
async fn load_select_rows(
    store: &Store,
    addr: SocketAddr,
    player: u32,
    burst: &mut LoadingBurst,
) -> Option<(
    world::character::Quickslots,
    Vec<db::player_skills::StoredSkill>,
    Vec<(Vec<u8>, i32)>,
    Vec<db::player_affects::StoredAffect>,
)> {
    let stored = match db::quickslots::load_quickslots(store, player).await {
        Ok(stored) => stored,
        Err(error) => {
            warn!(%addr, player, %error, "Quickslot load failed; closing");
            return None;
        }
    };
    let (quickslots, quickslot_records) = prodomo::quickslot::load(&stored);
    burst.after_map_test.splice(0..0, quickslot_records);
    let skills = match db::player_skills::load_skills(store, player).await {
        Ok(skills) => skills,
        Err(error) => {
            warn!(%addr, player, %error, "Skill load failed; closing");
            return None;
        }
    };
    prodomo::loading_phase::fill_skill_levels(burst, &skills);
    // `CInputDB::QuestLoad` (`G/input_db.cpp:1079-1101`): the character's quest flags,
    // which the world holds as its quest record from the admission on.
    let flags = match db::quest_flags::load_flags(store, player).await {
        Ok(flags) => flags,
        Err(error) => {
            warn!(%addr, player, %error, "Quest flag load failed; closing");
            return None;
        }
    };
    let quest_flags: Vec<(Vec<u8>, i32)> = flags
        .into_iter()
        .map(|flag| (flag.name, flag.value))
        .collect();
    if !quest_flags.is_empty() {
        info!(%addr, player, flags = quest_flags.len(), "quest flags loaded");
    }
    // `LoadAffect` (`G/char_affect.cpp:573-666`): the saved affects the character
    // carries again from the admission on.
    let affects = match db::player_affects::load_affects(store, player).await {
        Ok(affects) => affects,
        Err(error) => {
            warn!(%addr, player, %error, "Affect load failed; closing");
            return None;
        }
    };
    Some((quickslots, skills, quest_flags, affects))
}

/// The item load at character select, the way `CInputDB::ItemLoad` places a character's
/// items (`G/input_db.cpp:1451-1567`).
///
/// One `GC_ITEM_SET` per placed item, then `CheckMaximumPoints`, which clamps the stored pools
/// to the maxima the load computed with one `GC_CHARACTER_POINT_CHANGE` each, then
/// `PointsPacket`'s gold and points records, which legacy sends even for a character with no
/// item. A row the load refuses is logged and
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
    points: &mut world::character::Points,
    vid: u32,
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
    let load = plan_item_load(&rows, &context.protos, 0, character.level);
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
    // `EquipTo` computes the points with each worn item as the load places it.
    let storage = load.storage();
    points.compute_loaded(&world::character::Equipment::of(&storage, &context.protos));
    records.extend(item_load_points(character, points, vid));
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
            if let Err(error) = db::players::save_position(
                &context.store,
                account.id,
                player,
                target.x,
                target.y,
                target.map_index,
            )
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
    if held.presence.is_some() || held.world.is_some() {
        // The loading phase is entered once per descriptor and left by this record, so a
        // character already in the world cannot send it here.
        warn!(%addr, "ENTER_GAME for a character already in the world; closing");
        return false;
    }
    place_entering_character(context, addr, held);
    // The reducer proved the character is there; the field is the only place it lives.
    let Some(character) = held.character.as_ref() else {
        return false;
    };
    let vid = character_vid(character);
    let map = held.map.unwrap_or_default();
    let language = descriptor_language(held.account.as_ref());
    let (x, y) = (character.x, character.y);
    // `DESC::SetPlayer` makes the descriptor one the others can reach, and `PlayerLoad` then
    // shows the character on its map (`G/input_login.cpp:590`), so the view's records are
    // ready before the burst that carries them is written.
    let lease = context.clients.join(ClientEntry {
        channel: seat.number,
        map,
        name: character.name.clone(),
        vid,
        empire: character.empire,
        language,
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
    let outbox = lease.outbox();
    held.presence = Some(lease);
    let show = prodomo::game_loop_messages::Showing {
        place: prodomo::game_loop_messages::EnterPlace {
            channel: seat.number,
            map,
            x,
            y,
            z: 0,
        },
        card: PcCard::of(character, language, context.pk_protect_level),
    };
    let items = std::mem::take(&mut held.items);
    // `MessengerManager::Login`'s store reads (`G/messenger_manager.cpp:58-65`): the
    // account's rows and the GM team. A refused read closes, as a refused load does.
    let Some(messenger) = messenger_load(&context.store, addr, character.name.as_str()).await
    else {
        return false;
    };
    let loaded = loaded_state(held, Some(show), messenger);
    let Some((world_vid, shown)) =
        join_the_world(context, addr, character, vid, (items, loaded), outbox).await
    else {
        return false;
    };
    held.world = Some(world_vid);
    // From here a failed write closes the connection, whose close path takes the character
    // out of the world, so the characters that see it are sent its removal.
    let Some(burst) = entering_burst(context, addr, seat.number, held, shown).await else {
        return false;
    };
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
    held.avatar = Some(Avatar {
        map,
        x,
        y,
        sitting: false,
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
    true
}

/// Move the held character to where `CInputLogin::Entergame` shows it: the first movable point
/// around its saved position, else its empire's recall position when the map has a sectree
/// there, else the saved position (`G/input_login.cpp:572-590`).
///
/// Everything after this reads the new position: the enter-game burst, the world, and the save.
fn place_entering_character(context: &ConnectionContext, addr: SocketAddr, held: &mut Held) {
    let map = held.map.unwrap_or_default();
    let Some(character) = held.character.as_mut() else {
        return;
    };
    let saved = (character.x, character.y);
    let placed = entering_position(
        context.cells.get(map),
        &context.atlas,
        map,
        character.empire,
        saved,
    );
    let (x, y) = match placed {
        EnteringPosition::Movable { x, y } => (x, y),
        EnteringPosition::Recalled { x, y } => {
            warn!(
                %addr,
                name = %character.name.as_str(),
                x = saved.0,
                y = saved.1,
                map,
                to_x = x,
                to_y = y,
                "No movable position around the saved one; showing the character at its \
                 empire's recall position",
            );
            (x, y)
        }
        EnteringPosition::Kept => {
            warn!(
                %addr,
                name = %character.name.as_str(),
                x = saved.0,
                y = saved.1,
                map,
                "No movable position around the saved one and no recall position on the map; \
                 keeping it",
            );
            saved
        }
    };
    character.x = x;
    character.y = y;
}

/// What the load left for the world besides the items: the points, the quickslots and the
/// gold, and the body it `show`s.
///
/// The store's `CHECK` keeps `player.gold` from being negative, so the fall-back to 0 is not
/// a path a row can reach.
fn loaded_state(
    held: &Held,
    show: Option<prodomo::game_loop_messages::Showing>,
    messenger: MessengerLoad,
) -> prodomo::game_loop_messages::Loaded {
    let mut points = held.points.clone();
    // `m_pointsInstant.gm_level` (`char.h:468`) rides the admission, so the world reads it
    // where legacy reads it: the death penalty's `GetGMLevel()` (`G/char_battle.cpp:1341`).
    if let Some(points) = points.as_mut() {
        points.set_gm_level(held.gm_level);
    }
    prodomo::game_loop_messages::Loaded {
        points,
        quickslots: held.quickslots.clone(),
        gold: held
            .character
            .as_ref()
            .map_or(0, |character| u64::try_from(character.gold).unwrap_or(0)),
        show,
        skills: held.skills.clone(),
        quest_flags: held.quest_flags.clone(),
        messenger_companions: messenger.companions,
        messenger_team: messenger.team,
        messenger_levels: messenger.levels,
        affects: held.affects.clone(),
    }
}

/// The store's messenger rows for one entering character: its friend names, the GM team
/// names other than its own, and every grant's name-to-level map.
struct MessengerLoad {
    companions: Vec<Vec<u8>>,
    team: Vec<Vec<u8>>,
    levels: std::collections::BTreeMap<Vec<u8>, u8>,
}

/// Read the store's messenger rows for the character `name`: its friend names in the pair
/// key's order, the GM names other than its own, and every grant's name-to-level map
/// (`MessengerManager::Login`, `G/messenger_manager.cpp:46-65`, and its team half,
/// `LoadTeamList`, `:24-52`). Answers `None` when the store refuses, which closes the
/// descriptor as a refused load does.
async fn messenger_load(
    store: &db::store::Store,
    addr: SocketAddr,
    name: &str,
) -> Option<MessengerLoad> {
    let companions = match db::messenger::load_list(store, name).await {
        Ok(companions) => companions,
        Err(error) => {
            warn!(%addr, %error, "Messenger list load failed; closing");
            return None;
        }
    };
    let grants = match db::accounts::list_gm_grants(store).await {
        Ok(grants) => grants,
        Err(error) => {
            warn!(%addr, %error, "GM grant load failed; closing");
            return None;
        }
    };
    let mut team = Vec::new();
    let mut levels = std::collections::BTreeMap::new();
    for grant in &grants {
        let level = u8::try_from(grant.authority.as_i32()).unwrap_or(u8::MAX);
        levels.insert(grant.name.as_bytes().to_vec(), level);
        if !grant.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()) {
            team.push(grant.name.as_bytes().to_vec());
        }
    }
    Some(MessengerLoad {
        companions,
        team,
        levels,
    })
}

/// Puts a live client's character into the game thread's world, and records the entry.
///
/// `DESC::SetPlayer` is the step that does this, and the world is where items live.
/// The client set answers shouts and orders; it holds no inventory, so a grant would find
/// nobody and a client that had been given an item would find it gone at relog (ADR-0002,
/// which puts every world on one game thread).
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
/// Returns the entry's VID and the records the world's `Show` of the character wrote to it on
/// success, and `None` when the world refuses or cannot be reached, because in both cases this
/// client must not go on playing in a world it is not part of.
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
    (items, loaded): (Vec<(ItemPos, Item)>, prodomo::game_loop_messages::Loaded),
    outbox: prodomo::client_registry::ClientOutbox,
) -> Option<(common::vid::Vid, prodomo::game_loop_messages::Shown)> {
    let world_vid = common::vid::Vid::new(vid);
    let entered = context
        .game
        .enter_world_with_items(
            world_vid,
            character.id,
            character.name.clone(),
            items,
            loaded,
            outbox,
        )
        .await;
    match entered {
        Ok(Ok(shown)) => {
            info!(
                %addr,
                vid,
                name = %character.name.as_str(),
                shown = shown.records.len(),
                "Character entered the world on the game thread"
            );
            Some((world_vid, shown))
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
    // `CHARACTER::Save` writes the pending Warp destination, when there is one, instead of
    // the position (`G/char.cpp:1551-1565`). The map rides with it, as legacy's
    // `tab.lMapIndex` does (`G/char.cpp:1552-1564`): a private map's index names the copy
    // of a base map the character is on, which its position cannot name.
    let live = prodomo::warp::LivePoint {
        x: avatar.x,
        y: avatar.y,
        z: 0,
        map_index: avatar.map,
    };
    let (x, y, map_index) = match held
        .warp
        .map_or(prodomo::warp::SavePosition::Live(live), |warp| {
            prodomo::warp::save_position(&warp, &live)
        }) {
        prodomo::warp::SavePosition::Warp(target) => (target.x, target.y, target.map_index),
        prodomo::warp::SavePosition::Live(point) => (point.x, point.y, point.map_index),
    };
    let save = prodomo::save::player_save(
        character,
        prodomo::save::SavePosition { x, y, map_index },
        held.exit.unwrap_or_default(),
        prodomo::save::playtime(character.playtime_minutes, elapsed),
    );
    match db::players::save_character(&context.store, account.id, character.id, &save).await {
        // Legacy writes the quickslots in the player row (`G/char.cpp:1603-1604`); here they
        // are their own table, written after the row.
        Ok(true) => {
            let slots = prodomo::quickslot::stored(&held.quickslots);
            if let Err(error) =
                db::quickslots::save_quickslots(&context.store, character.id, &slots).await
            {
                warn!(player = character.id, %error, "Could not save the quickslots");
                return false;
            }
            // `CHARACTER::Save` writes the quest flags after the player row
            // (`G/char.cpp:1673-1679`, `quest::PC::Save`, `G/questpc.cpp:520-588`).
            let flags: Vec<db::quest_flags::StoredQuestFlag> = held
                .quest_flags
                .iter()
                .map(|(name, value)| db::quest_flags::StoredQuestFlag {
                    name: name.clone(),
                    value: *value,
                })
                .collect();
            if let Err(error) =
                db::quest_flags::save_flags(&context.store, character.id, &flags).await
            {
                warn!(player = character.id, %error, "Could not save the quest flags");
                return false;
            }
            // `QUERY_ADD_AFFECT` (`db/ClientManagerPlayer.cpp:1563-1585`): the saved
            // affects the character still carries, written after the player row.
            if let Err(error) =
                db::player_affects::save_affects(&context.store, character.id, &held.affects).await
            {
                warn!(player = character.id, %error, "Could not save the affects");
                return false;
            }
            true
        }
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
    let linked = select_guilds(&context.store, &account.lobby).await?;
    let guilds = select_guild_views(&linked);
    let success = login_success(
        &account.lobby,
        &context.atlas,
        seat.locations,
        &guilds,
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
        context.refusals.block_char_creation,
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

/// Answer the downloader's handshake-phase `MARK_LOGIN`; `false` when the connection
/// must close. On a mark server the phase moves to login; elsewhere, and on a short
/// frame, the connection closes, as `CInputHandshake::Analyze`'s mark arm closes it
/// (`G/input.cpp:211-226`).
async fn mark_login_handshake<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if CgMarkLogin::decode_frame(frame).is_err() {
        warn!(%addr, "The downloader sent a short MARK_LOGIN; closing");
        return false;
    }
    if !context.mark_server.serves {
        warn!(%addr, "A mark login reached a process that serves no marks; closing");
        return false;
    }
    match session.mark_login().await {
        Ok(_) => true,
        Err(error) => {
            warn!(%addr, %error, "Client session stopped");
            false
        }
    }
}

/// Run one of the six login-phase mark records; `false` when the connection must
/// close. `MARK_LOGIN` is consumed, the CRC list and the index list are answered from
/// the world's memory, and the uploads judge, store, then commit, as the guild arms
/// do. A malformed record closes; a quiet judgement sends nothing; a store error on a
/// judged upload closes, as an unstored move does.
async fn mark_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    _seat: &ChannelSeat<'_>,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if frame.header == HEADER_CG_MARK_LOGIN.value() {
        debug!(%addr, "consumed with nothing done");
        return true;
    }
    if frame.header == HEADER_CG_MARK_CRCLIST.value() {
        return mark_crc_list(session, addr, context, frame).await;
    }
    if frame.header == HEADER_CG_MARK_IDXLIST.value() {
        return mark_idx_list(session, addr, context, frame).await;
    }
    if frame.header == HEADER_CG_MARK_UPLOAD.value() {
        return mark_upload(session, addr, context, frame).await;
    }
    if frame.header == HEADER_CG_GUILD_SYMBOL_UPLOAD.value() {
        return symbol_upload(session, addr, context, frame).await;
    }
    debug_assert_eq!(frame.header, HEADER_CG_SYMBOL_CRC.value());
    symbol_crc(session, addr, context, frame).await
}

/// Answer one `MARK_CRCLIST` with the blocks the client's list lacks. A malformed
/// record or a dead world closes; an unknown image slot answers the empty block
/// record, as legacy's empty diff does.
async fn mark_crc_list<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgMarkCRCList::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed MARK_CRCLIST; closing");
            return false;
        }
    };
    let diff = match context
        .game
        .mark_crc_list(record.img_idx, record.crc_list)
        .await
    {
        Ok(diff) => diff,
        Err(error) => {
            warn!(%addr, %error, "The world could not diff the mark blocks; closing");
            return false;
        }
    };
    let blocks: Vec<(u8, &[u8])> = diff
        .iter()
        .map(|(pos, bytes)| (*pos, bytes.as_slice()))
        .collect();
    let record = mark_block_record(record.img_idx, &blocks);
    if let Err(error) = session.send(&record).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
}

/// Answer one `MARK_IDXLIST` with every (guild, mark) pair. A malformed record or a
/// dead world closes; an empty manager answers the empty list, as legacy's zero
/// `bufSize` does.
async fn mark_idx_list<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if CgMarkIdxList::decode_frame(frame).is_err() {
        warn!(%addr, "Client sent a malformed MARK_IDXLIST; closing");
        return false;
    }
    let pairs = match context.game.mark_idx_list().await {
        Ok(pairs) => pairs,
        Err(error) => {
            warn!(%addr, %error, "The world could not list the mark ids; closing");
            return false;
        }
    };
    let record = mark_idxlist_record(&pairs);
    if let Err(error) = session.send(&record).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
}

/// Run one `MARK_UPLOAD`: the world judges, the descriptor stores the row, and the
/// judgement stands. A malformed record closes; a quiet judgement sends nothing; a
/// store error closes, as an unstored move does. A save stores the id and the 768
/// image bytes; a delete removes the row.
async fn mark_upload<S>(
    _session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgMarkUpload::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed MARK_UPLOAD; closing");
            return false;
        }
    };
    let mut image = [0_u32; prodomo::game_loop_messages::MARK_PIXELS];
    for (i, word) in image.iter_mut().enumerate() {
        let at = i * 4;
        *word = u32::from_le_bytes([
            record.image[at],
            record.image[at + 1],
            record.image[at + 2],
            record.image[at + 3],
        ]);
    }
    let judged = match context
        .game
        .mark_upload(record.gid, image, context.mark_server.min_level)
        .await
    {
        Ok(judged) => judged,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the mark upload; closing");
            return false;
        }
    };
    match judged {
        prodomo::game_state::MarkUploadAnswer::Saved { mark_id } => {
            if let Err(error) =
                db::guilds::save_mark(&context.store, record.gid, mark_id, &record.image).await
            {
                warn!(%addr, %error, "The mark row could not be stored; closing");
                return false;
            }
        }
        prodomo::game_state::MarkUploadAnswer::Deleted => {
            if let Err(error) = db::guilds::delete_mark(&context.store, record.gid).await {
                warn!(%addr, %error, "The mark row could not be removed; closing");
                return false;
            }
        }
        prodomo::game_state::MarkUploadAnswer::Quiet => {}
    }
    true
}

/// Run one `GUILD_SYMBOL_UPLOAD`: the size gates close, as legacy's `SetPhase` does;
/// the world commits the bytes, and the descriptor stores the row. An unknown guild
/// is quiet, as the land gate's close is: the port refuses instead of closing (230v).
async fn symbol_upload<S>(
    _session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match GuildSymbolUpload::decode(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed GUILD_SYMBOL_UPLOAD; closing");
            return false;
        }
    };
    let held = match context
        .game
        .symbol_upload(record.guild_id, record.symbol.clone())
        .await
    {
        Ok(held) => held,
        Err(error) => {
            warn!(%addr, %error, "The world could not judge the symbol upload; closing");
            return false;
        }
    };
    if !held {
        debug!(%addr, "a symbol upload for an unknown guild is quiet");
        return true;
    }
    if let Err(error) =
        db::guilds::save_symbol(&context.store, record.guild_id, &record.symbol).await
    {
        warn!(%addr, %error, "The symbol row could not be stored; closing");
        return false;
    }
    true
}

/// Answer one `SYMBOL_CRC` with the stored bytes on a size or CRC mismatch. A
/// malformed record or a dead world closes; a match or an unknown guild is quiet.
async fn symbol_crc<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = match CgSymbolCrc::decode_frame(frame) {
        Ok(record) => record,
        Err(error) => {
            warn!(%addr, %error, "Client sent a malformed SYMBOL_CRC; closing");
            return false;
        }
    };
    let mismatch = match context
        .game
        .symbol_crc(record.guild_id, record.crc, record.size)
        .await
    {
        Ok(mismatch) => mismatch,
        Err(error) => {
            warn!(%addr, %error, "The world could not answer the symbol CRC; closing");
            return false;
        }
    };
    let Some(bytes) = mismatch else {
        debug!(%addr, "a symbol CRC hit is quiet");
        return true;
    };
    let record = symbol_data_record(record.guild_id, &bytes);
    if let Err(error) = session.send(&record).await {
        warn!(%addr, %error, "Client session stopped");
        return false;
    }
    true
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
    if context.refusals.shutdowned || context.state.is_shutting_down() {
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
    // The war-end poll: the guild tick queues its outcomes in the world (it has no
    // descriptor), and this timer drains them once a second for the store. Pinned once
    // like the others: recreating it each pass would restart the second.
    let mut war_ends = Box::pin(tokio::time::interval(std::time::Duration::from_secs(1)));
    // The first tick fires at once; the queue is empty until a war ends, so skip it.
    war_ends.tick().await;

    let exit = 'run: loop {
        tokio::select! {
                   signal_result = &mut shutdown => {
                       signal_result?;
                       break 'run AcceptLoopExit::Signal;
                   }
                   terminal = game_loop.wait_for_terminal() => {
                       break 'run AcceptLoopExit::GameLoop(terminal?);
                   }
        _ = war_ends.tick(), if ready => {
        drain_war_ends(game_loop, store).await;
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
                        // The stored guilds ride the same ordering: they are only readable
                        // once the schema is up, and a world without them judges a name a
                        // concurrent boot would see as taken free.
                        install_guilds(game_loop, store).await?;
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
                                    `echo 'item give <name> <vnum> [count]' > <path>` and read \
                                    its answer in this log"
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
                           // A connection's state is larger than a future is kept on the stack for,
                           // so it lives on the heap from the start.
                           tokio::spawn(async move {
                               Box::pin(handle_connection(
                                   stream,
                                   addr,
                                   role,
                                   connection,
                                   &connection_shutdown,
                               ))
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
/// Drain the war ends the guild tick queued: each outcome's two rows land in the
/// store. A failed poll or write is logged, not fatal — the memory already ended
/// the war, and the next second's poll retries whatever the queue still holds
/// (a failed write's rows stay queued only when the poll itself failed; a write
/// that failed after the drain is lost, as a crashed descriptor's commit is).
async fn drain_war_ends(game_loop: &mut GameLoopHandle, store: &Store) {
    let outcomes = match game_loop.controller().take_war_ends().await {
        Ok(outcomes) => outcomes,
        Err(error) => {
            warn!(%error, "War ends could not be drained; retrying next second");
            return;
        }
    };
    for outcome in outcomes {
        for row in [outcome.lo, outcome.hi] {
            if let Err(error) = db::guilds::set_war_tallies(
                store,
                row.guild_id,
                row.ladder,
                row.wins,
                row.draws,
                row.losses,
            )
            .await
            {
                warn!(guild_id = row.guild_id, %error, "A war-end row could not be stored");
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

/// Each Channel's maps, the port each Channel is reached on, and the Shared Channel's port
/// and maps.
struct MapRoutes {
    channel_maps: HashMap<u8, Vec<u32>>,
    channel_ports: HashMap<u8, u16>,
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

/// Load the player motion clips under `<game_data>/data` (`CMotionManager::Build`), which give
/// each race and weapon its walk and run speed.
///
/// A clip legacy would not load (absent, or with no `MotionDuration`) is left out and warned
/// about, and its race and mode fall back to 300 as legacy's do (`G/char.cpp:3576`). A clip that
/// exists but cannot be read, or is not a legacy text file, stops the server before any port
/// opens, as the other Game data files do (a Divergence: legacy goes on without it).
fn load_pc_motions(config: &ServerConfig) -> Result<Arc<PcMotions>, String> {
    let data_dir = config.data_dir();
    let motions = PcMotions::load(&data_dir).map_err(|error| {
        format!(
            "Motion clips are unusable in {}: {error}",
            data_dir.display()
        )
    })?;
    for path in &motions.skipped {
        warn!(
            clip = %path.display(),
            "A motion clip is absent or has no MotionDuration; its speed falls back to 300"
        );
    }
    info!(
        clips = motions.loaded(),
        skipped = motions.skipped.len(),
        "Motion clips loaded"
    );
    Ok(Arc::new(motions))
}

/// Load the mob skill tables: the hosted `skill_proto` rows from the Game data
/// `player.sql` dump and the mob power row from `common.sql`'s `locale` table
/// (`CClientManager::InitializeSkillTable`, `DB/ClientManagerBoot.cpp:654-740`).
///
/// A dump that does not parse, or a hosted row outside the hosted subset, stops the
/// server before any port opens, as the other Game data files do.
fn load_mob_skills(
    config: &ServerConfig,
) -> Result<Arc<gamedata::mob_skill_data::MobSkills>, String> {
    let player_path = config.game_tables.join("player.sql");
    let player = std::fs::read(&player_path)
        .map_err(|error| format!("Failed to read {}: {error}", player_path.display()))?;
    let common_path = config.game_tables.join("common.sql");
    let common = std::fs::read(&common_path)
        .map_err(|error| format!("Failed to read {}: {error}", common_path.display()))?;
    let skills = gamedata::mob_skill_data::MobSkills::from_dumps(&player, &common)
        .map_err(|error| format!("Mob skills are unusable: {error}"))?;
    info!("Mob skill tables loaded");
    Ok(Arc::new(skills))
}

/// Load the PC skill rows: the `skill_proto` PC rows and the power rows
/// (`CSkillManager::Initialize` lays them out, `G/skill.cpp`).
///
/// A row whose polys refuse (the `number(`/`floor(` rows) stays out of the map, as
/// legacy's `sys_err` + skip leaves it uncastable; a dump that does not parse stops
/// the server before any port opens.
fn load_pc_skills(config: &ServerConfig) -> Result<Arc<gamedata::pc_skill_data::PcSkills>, String> {
    let player_path = config.game_tables.join("player.sql");
    let player = std::fs::read(&player_path)
        .map_err(|error| format!("Failed to read {}: {error}", player_path.display()))?;
    let common_path = config.game_tables.join("common.sql");
    let common = std::fs::read(&common_path)
        .map_err(|error| format!("Failed to read {}: {error}", common_path.display()))?;
    let skills = gamedata::pc_skill_data::PcSkills::from_dumps(&player, &common)
        .map_err(|error| format!("PC skills are unusable: {error}"))?;
    info!("PC skill tables loaded");
    Ok(Arc::new(skills))
}

/// Load the guild skill rows: the 151-162 `skill_proto` rows (`CSkillManager::
/// Initialize` lays out the same table, `G/skill.cpp`).
///
/// A row whose poly refuses stays out of the map, as legacy's `sys_err` + skip leaves
/// it uncastable; a dump that does not parse stops the server before any port opens.
fn load_guild_skills(
    config: &ServerConfig,
) -> Result<Arc<gamedata::guild_skill_data::GuildSkills>, String> {
    let player_path = config.game_tables.join("player.sql");
    let player = std::fs::read(&player_path)
        .map_err(|error| format!("Failed to read {}: {error}", player_path.display()))?;
    let skills = gamedata::guild_skill_data::GuildSkills::from_dump(&player)
        .map_err(|error| format!("Guild skills are unusable: {error}"))?;
    info!("Guild skill tables loaded");
    Ok(Arc::new(skills))
}

/// Load the mob skill clips: each skilling folder's `SPECIALn` durations and splash
/// entries (`CMotionManager::Build`, `G/motion.cpp:327-347`).
///
/// A malformed clip is skipped and warned about (legacy's `sys_err` arm); a missing
/// line or file leaves the slot empty.
fn load_mob_clips(
    config: &ServerConfig,
    protos: &Arc<gamedata::mob_proto::MobProtos>,
) -> Arc<gamedata::mob_skill_data::MobSkillClips> {
    let clips =
        gamedata::mob_skill_data::MobSkillClips::load(&config.data_dir().join("monster"), protos);
    for path in &clips.skipped {
        warn!(
            clip = path,
            "A skill clip failed to parse; its slot stays empty"
        );
    }
    info!("Mob skill clips loaded");
    Arc::new(clips)
}

/// Load the locale strings of every language: `<country_dir>/<code>/locale_string.txt`.
///
/// Legacy reads them at boot (`G/locale_service.cpp:418-455`) and goes on without a file it
/// cannot open, which leaves that language untranslated. The Rewrite refuses to start instead,
/// as for every other Game data file (a Divergence). A file that stops early is read as far as
/// legacy reads it and is warned about.
fn load_locale_strings(config: &ServerConfig) -> Result<Arc<LocaleStrings>, String> {
    let strings = LocaleStrings::load(&config.country_dir())
        .map_err(|error| format!("Locale strings are unusable: {error}"))?;
    for language in 1..LOCALE_COUNT {
        let Some(table) = strings.table(language) else {
            continue;
        };
        let code = gamedata::locale_string::country_code(language);
        if let Ending::Stopped { at, cause } = table.ending() {
            warn!(
                code,
                at,
                ?cause,
                "Locale strings stop before the end of the file"
            );
        }
        info!(
            code,
            pairs = table.pairs(),
            strings = table.len(),
            "Locale strings loaded"
        );
    }
    Ok(Arc::new(strings))
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
    // The port each Channel is reached on. A legacy Channel had one port per Core, and the
    // client list named one of them; the first bound port is that one.
    let channel_ports: HashMap<u8, u16> = config
        .channels
        .iter()
        .filter_map(|channel| {
            let port = listeners
                .iter()
                .find(|listener| listener.role() == ListenerRole::Channel(channel.number))?
                .local_addr()
                .port();
            Some((channel.number, port))
        })
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
        channel_ports,
        shared,
    }
}

/// The Game data a connection reads, loaded before any port opens.
struct GameData {
    /// The map regions, which the descriptors and the world's town recall share.
    atlas: Arc<MapAtlas>,
    /// The cell attributes of every hosted map.
    cells: Arc<HostedCells>,
    /// The Names a character may not take.
    names: NameRules,
    /// The item prototypes, the same table the game thread holds.
    protos: Arc<ItemProtos>,
    /// The locale strings, the same ones the game thread holds.
    locale: Arc<LocaleStrings>,
    /// What the game thread stands the NPCs up from before it starts.
    npcs: NpcData,
    /// The shops a keeper opens, which the game thread holds.
    shops: Arc<NpcShops>,
    /// The drop tables, which the game thread holds.
    drops: Arc<gamedata::drop::DropTables>,
    /// The `item_attr` rows, which a dropped item's magic rolls against.
    attrs: Arc<Vec<gamedata::records::ItemAttrRecord>>,
    /// The vnum mask table, which a made item's shown vnum answers through.
    masks: Arc<gamedata::item_mask::ItemMasks>,
    /// The player motion clips, the same table the game thread holds.
    /// The player motion clips, the same table the game thread holds.
    motions: Arc<PcMotions>,
    /// The hosted mob skill rows, the same table the game thread holds.
    mob_skills: Arc<gamedata::mob_skill_data::MobSkills>,
    /// The mob skill clips, the same table the game thread holds.
    mob_clips: Arc<gamedata::mob_skill_data::MobSkillClips>,
    /// The PC skill rows, the same table the game thread holds.
    pc_skills: Arc<gamedata::pc_skill_data::PcSkills>,
    /// The guild skill rows, the same table the game thread holds.
    guild_skills: Arc<gamedata::guild_skill_data::GuildSkills>,
}

/// What boot stands the NPCs and monsters up from: the mob prototypes, the mob groups, the names
/// a client is sent, and the regen entries of every map each Channel hosts.
struct NpcData {
    protos: Arc<MobProtos>,
    groups: Arc<MobGroups>,
    names: Arc<MobLocaleNames>,
    maps: Vec<(u8, MapRegion, Vec<RegenEntry>)>,
}

/// Load the Game data and the quests, which the game thread takes, before any port opens.
///
/// The map regions are Game data the Channel login needs; a client is not accepted before they
/// are loaded, so a missing or malformed file stops the server before any port opens.
fn load_game_data(config: &ServerConfig) -> Result<(GameData, Quests), String> {
    let atlas = load_atlas(config)?;
    let names = load_name_rules(config)?;
    let protos = load_item_protos(config)?;
    let locale = load_locale_strings(config)?;
    let npcs = load_npc_data(config, &atlas)?;
    let cells = load_map_cells(config, &atlas)?;
    let shops = load_npc_shops(config, &protos)?;
    let drops = load_drop_tables(config, &protos)?;
    let attrs = load_item_attrs(config)?;
    let masks = load_item_masks(config)?;
    let motions = load_pc_motions(config)?;
    let mob_skills = load_mob_skills(config)?;
    let mob_clips = load_mob_clips(config, &npcs.protos);
    let pc_skills = load_pc_skills(config)?;
    let guild_skills = load_guild_skills(config)?;
    let quests = load_quests(config, &npcs)?;
    let data = GameData {
        atlas: Arc::new(atlas),
        cells,
        names,
        protos,
        locale,
        npcs,
        shops,
        drops,
        attrs,
        masks,
        motions,
        mob_skills,
        mob_clips,
        pc_skills,
        guild_skills,
    };
    Ok((data, quests))
}

/// Load the NPC shops: the `shop` and `shop_item` rows of the Game data tables, laid out as
/// `CShopManager::Initialize` lays them out (`G/shop_manager.cpp:39-69`).
///
/// An item that cannot be laid out is left out of its shop and warned about; legacy loses it and
/// every item after it (a Divergence).
fn load_npc_shops(config: &ServerConfig, protos: &ItemProtos) -> Result<Arc<NpcShops>, String> {
    let dump_path = config.game_tables.join("player.sql");
    let dump = std::fs::read(&dump_path)
        .map_err(|error| format!("Failed to read {}: {error}", dump_path.display()))?;
    let table = shops_from_dump(&dump).map_err(|error| {
        format!(
            "Failed to read the shops in {}: {error}",
            dump_path.display()
        )
    })?;
    let shops = NpcShops::lay_out(&table, protos);
    for skipped in shops.skipped() {
        warn!(
            shop = skipped.shop_vnum,
            vnum = skipped.vnum,
            skip = ?skipped.skip,
            "A shop item cannot be laid out; it is left out of its shop"
        );
    }
    info!(
        shops = shops.shops().len(),
        skipped = shops.skipped().len(),
        "NPC shops loaded"
    );
    Ok(Arc::new(shops))
}

/// Load the drop tables from the Locale folder (`BSelectCalendar`, `G/input_db.cpp:899-942`).
///
/// A missing or refused file stops the server, as legacy's boot shuts down; an empty one loads
/// fine. `protos` is the already-read `item_proto.txt`, which an item is resolved against.
fn load_drop_tables(
    config: &ServerConfig,
    protos: &ItemProtos,
) -> Result<Arc<gamedata::drop::DropTables>, String> {
    let locale_dir = config.locale_dir();
    let tables = gamedata::drop::DropTables::load(&locale_dir, protos).map_err(|error| {
        format!(
            "Drop tables are unusable in {}: {error}",
            locale_dir.display()
        )
    })?;
    info!(
        limit = tables.mob.limit.len(),
        kill = tables.mob.kill.len(),
        gloves = tables.mob.gloves.len(),
        groups = tables.groups.groups.len(),
        "Drop tables loaded"
    );
    Ok(Arc::new(tables))
}

/// Load the `item_attr` rows of the Game data tables, which a dropped item's magic rolls
/// against (`CClientManager::InitializeItemAttrTable`, `DB/ClientManagerBoot.cpp:774-842`).
///
/// An empty table refuses boot, as legacy's does (`no result from item_attr`); a malformed
/// row is the same class.
fn load_item_attrs(
    config: &ServerConfig,
) -> Result<Arc<Vec<gamedata::records::ItemAttrRecord>>, String> {
    let dump_path = config.game_tables.join("player.sql");
    let dump = std::fs::read(&dump_path)
        .map_err(|error| format!("Failed to read {}: {error}", dump_path.display()))?;
    let rows = gamedata::item_attr::attrs_from_dump(&dump).map_err(|error| {
        format!(
            "Failed to read the item attributes in {}: {error}",
            dump_path.display()
        )
    })?;
    info!(attrs = rows.len(), "Item attributes loaded");
    Ok(Arc::new(rows))
}

/// Load the vnum mask table from the Locale folder
/// (`ITEM_MANAGER::ReadItemVnumMaskTable`, `G/item_manager_read_tables.cpp:938-953`).
///
/// A missing file loads as empty, as legacy's boot does (it only logs); a malformed one
/// refuses.
fn load_item_masks(config: &ServerConfig) -> Result<Arc<gamedata::item_mask::ItemMasks>, String> {
    let locale_dir = config.locale_dir();
    let masks = gamedata::item_mask::ItemMasks::load(&locale_dir).map_err(|error| {
        format!(
            "Mask table is unusable in {}: {error}",
            locale_dir.display()
        )
    })?;
    info!(masks = masks.pairs().len(), "Item masks loaded");
    Ok(Arc::new(masks))
}

/// Load what the NPCs are stood up from: `mob_proto.txt` with its names, the `LOCALE_YMIR` mob
/// names, and the regen files of every map each Channel hosts.
///
/// A missing or malformed file stops the server. Legacy's DB server stops on a mob proto it
/// cannot read; it skips a regen file it cannot open and answers `NoName` for every mob when
/// the names file is missing, where the Rewrite refuses both (Divergences). A hosted map the
/// map index does not list has no NPCs, as legacy never builds it.
fn load_npc_data(config: &ServerConfig, atlas: &MapAtlas) -> Result<NpcData, String> {
    let proto_dir = config.proto_dir();
    let protos = MobProtos::load(&proto_dir).map_err(|error| {
        format!(
            "Mob prototypes are unusable in {}: {error}",
            proto_dir.display()
        )
    })?;
    for (vnum, line) in protos.duplicates() {
        warn!(
            vnum,
            line, "A mob vnum is listed more than once; the first row is used"
        );
    }
    let names = MobLocaleNames::load(&config.country_dir())
        .map_err(|error| format!("Mob names are unusable: {error}"))?;
    let groups = load_mob_groups(config)?;
    let map_dir = config.map_dir();
    let mut maps = Vec::new();
    for channel in &config.channels {
        for &map in &channel.maps {
            let Some(region) = i32::try_from(map).ok().and_then(|map| atlas.region(map)) else {
                warn!(
                    channel = channel.number,
                    map, "A hosted map is not in the map index"
                );
                continue;
            };
            let entries = regen::load_map(&map_dir, region)
                .map_err(|error| format!("Regen files are unusable: {error}"))?;
            maps.push((channel.number, region.clone(), entries));
        }
    }
    info!(
        prototypes = protos.rows().len(),
        names = names.len(),
        maps = maps.len(),
        "Mob prototypes and regen files loaded"
    );
    Ok(NpcData {
        protos: Arc::new(protos),
        groups: Arc::new(groups),
        names: Arc::new(names),
        maps,
    })
}

/// Load the Locale's mob groups and group-groups (`CMobManager::LoadGroup` and
/// `LoadGroupGroup`, `G/mob_manager.cpp:255-409`).
///
/// A missing or malformed file stops the server, as legacy's boot exits on either. A group
/// legacy skips is warned about with legacy's reason, and a vnum listed twice keeps its first
/// group (legacy's `insert` keeps the first too).
fn load_mob_groups(config: &ServerConfig) -> Result<MobGroups, String> {
    let locale_dir = config.locale_dir();
    let groups = MobGroups::load(&locale_dir).map_err(|error| {
        format!(
            "Mob groups are unusable in {}: {error}",
            locale_dir.display()
        )
    })?;
    for skipped in groups.skipped() {
        warn!(
            file = skipped.file,
            name = %String::from_utf8_lossy(&skipped.name),
            reason = skipped.reason,
            "A mob group is skipped"
        );
    }
    for &(file, vnum) in groups.duplicates() {
        warn!(
            file,
            vnum, "A mob group vnum is listed more than once; the first group is used"
        );
    }
    let (count, group_groups) = groups.len();
    info!(groups = count, group_groups, "Mob groups loaded");
    Ok(groups)
}

/// Load the `server_attr` of every map a Channel hosts, once however many Channels host it
/// (`SECTREE_MANAGER::LoadAttribute`, `G/sectree_manager.cpp:392-490`).
///
/// A missing or refused file stops the server (a Divergence): legacy's `Build` ignores
/// `LoadAttribute`'s answer (`:752-753`), so a map whose file it cannot read runs with sectrees
/// that hold no attributes, and the first read of one dereferences NULL. A hosted map the map
/// index does not list has no cells, as legacy never builds it; [`load_npc_data`] warns about
/// it.
fn load_map_cells(config: &ServerConfig, atlas: &MapAtlas) -> Result<Arc<HostedCells>, String> {
    let map_dir = config.map_dir();
    let mut hosted = HostedCells::default();
    let indexes: std::collections::BTreeSet<i32> = config
        .channels
        .iter()
        .flat_map(|channel| channel.maps.iter())
        .filter_map(|&map| i32::try_from(map).ok())
        .collect();
    for index in indexes {
        let Some(region) = atlas.region(index) else {
            continue;
        };
        let attr = server_attr::load_map(&map_dir, region)
            .map_err(|error| format!("Map {index} is unusable: {error}"))?;
        // `load_map` has placed the region's sectrees, so the grid is there.
        let grid = SectreeGrid::of(region)
            .ok_or_else(|| format!("Map {index} is unusable: its sectrees cannot be placed"))?;
        hosted.insert(index, MapCells::new(grid, Arc::new(attr)));
    }
    info!(maps = hosted.len(), "Map attributes loaded");
    Ok(Arc::new(hosted))
}

/// Load the quests: every script of the Locale's `quest` folder compiled by the port of `qc` and
/// loaded into one Lua 5.1 state with the quest libraries (ADR-0004, ADR-0006), and the mob and
/// item names of every language, which `mob_name` and `item_name` answer in the character's.
///
/// A library, a state table or a file that fails stops the server, and so does a missing
/// `questnpc.txt`. A script `qc` refuses is warned about and not loaded, as legacy's build left
/// out a script that did not compile.
fn load_quests(config: &ServerConfig, npcs: &NpcData) -> Result<Quests, String> {
    let locale_dir = config.locale_dir();
    let manager = quest::manager::Manager::load(&locale_dir)
        .map_err(|error| format!("Quests are unusable in {}: {error}", locale_dir.display()))?;
    let names = MobNamesByLanguage::load(&config.country_dir())
        .map_err(|error| format!("Mob names are unusable: {error}"))?;
    let item_names = ItemNamesByLanguage::load_file(&config.country_dir(), ITEM_NAMES_FILE)
        .map_err(|error| format!("Item names are unusable: {error}"))?;
    let host = manager.host();
    info!(
        quests = host.quests().len(),
        files = host.objects().len(),
        refused = host.refused().len(),
        "Quests loaded"
    );
    Ok(Quests::new(manager, Arc::clone(&npcs.protos), names).with_item_names(item_names))
}

/// Stand the NPCs, monsters and stones up in the world before the game thread starts, on each
/// map's cells, drawing from its dice, and hand the world what its regens spawn from after. A
/// hosted map with no cells is one the map index does not list, which has no regen entries
/// either.
fn stand_up_npcs(
    state: &mut GameState,
    data: &NpcData,
    cells: &Arc<HostedCells>,
    clips: &Arc<gamedata::mob_skill_data::MobSkillClips>,
) -> Result<(), String> {
    let mut spawner = NpcSpawner::new(&data.protos, &data.groups, &data.names, NpcVids::default())
        .with_clips(clips);
    for (channel, region, entries) in &data.maps {
        let Some(map_cells) = cells.get(region.index) else {
            continue;
        };
        state
            .spawn_npcs(&mut spawner, *channel, (region, map_cells), entries)
            .map_err(|error| format!("NPCs could not be stood up: {error}"))?;
    }
    let report = spawner.report();
    info!(
        spawned = report.spawned,
        stones = report.stones,
        gated = report.gated,
        no_proto = report.no_proto,
        no_group = report.no_group,
        unplaced = report.unplaced,
        idle = report.idle,
        "NPCs and monsters stood up"
    );
    let respawner = Respawner::new(
        Arc::clone(&data.protos),
        Arc::clone(&data.groups),
        Arc::clone(&data.names),
        spawner.into_vids(),
    )
    .with_clips(Arc::clone(clips));
    state.install_respawner(respawner, Arc::clone(cells));
    Ok(())
}

/// What every connection shares, built once the listeners are bound and the Game data loaded.
///
/// `clock` is the process's `get_dword_time()`, which the world reads too, so a handshake time
/// and a move's start are on one clock.
fn connection_context(
    config: &ServerConfig,
    state: &Arc<ServerState>,
    store: &Store,
    listeners: &Listeners,
    data: GameData,
    (clients, clock): (Arc<ChannelClients>, BootLiveClock),
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
        clock,
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
        refusals: Refusals {
            shutdowned: config.game.shutdowned,
            block_char_creation: config.game.block_char_creation,
        },
        block_login: Arc::from(config.game.block_login.as_str()),
        user_limit: config.game.user_limit,
        mark_server: MarkServer {
            serves: config.game.guild_mark_server,
            min_level: config
                .game
                .guild_mark_min_level
                .min(u8::try_from(common::constants::GUILD_MAX_LEVEL).unwrap_or(u8::MAX)),
        },
        logons: LogonRegistry::new(),
        atlas: data.atlas,
        cells: data.cells,
        protos: data.protos,
        locale: data.locale,
        public_ip: config.public_ip,
        routes: Arc::new(map_routes(config, listeners)),
        handles: Arc::new(AtomicU32::new(0)),
        names: Arc::new(data.names),
        clients,
        creates: Arc::new(CreateCooldown::default()),
        delete_levels: (
            config.game.player_delete_level_limit,
            config.game.player_delete_level_limit_lower,
        ),
        shout_limit_level: config.game.shout_limit_level,
        modes: Modes {
            global_shout: config.game.enable_global_shout,
            test_server: config.game.test_server,
        },
        pk_protect_level: config.game.pk_protect_level,
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

/// The stored guilds `serve` moves into the game thread once the store is up.
///
/// Every failure here stops the server instead of opening the gate, as a missing
/// allocator does: a world without its guilds judges a concurrent name free.
async fn install_guilds(
    game_loop: &mut GameLoopHandle,
    store: &Store,
) -> Result<(), Box<dyn Error>> {
    let stored = db::guilds::load_all(store)
        .await
        .map_err(|error| format!("Stored guilds are unusable: {error}"))?;
    let marks = db::guilds::load_marks(store)
        .await
        .map_err(|error| format!("Stored guild marks are unusable: {error}"))?;
    let symbols = db::guilds::load_symbols(store)
        .await
        .map_err(|error| format!("Stored guild symbols are unusable: {error}"))?;
    info!(
        guilds = stored.guilds.len(),
        marks = marks.len(),
        symbols = symbols.len(),
        "Stored guilds loaded"
    );
    match game_loop
        .controller()
        .install_guilds(stored, marks, symbols)
        .await?
    {
        Ok(()) => Ok(()),
        Err(error) => Err(format!("The world refused the stored guilds: {error}").into()),
    }
}

/// The world `serve` moves into the game thread, with its NPCs stood up.
///
/// It is built without an item id allocator on purpose: the start id is `MAX(id)` over the
/// item table, and that table is only readable once the store has migrated, which happens
/// inside the accept loop. The allocator arrives as a command from there.
fn build_world(
    config: &ServerConfig,
    data: &GameData,
    quests: Quests,
    clients: &Arc<ChannelClients>,
    clock: BootLiveClock,
) -> Result<GameState, String> {
    // The maps each Channel serves, and the Shared Channel's, which a town restart's recall
    // point must be on (`CMapLocation::Get`), as `map_routes` reads them.
    let channel_maps: HashMap<u8, Vec<u32>> = config
        .channels
        .iter()
        .map(|channel| (channel.number, channel.maps.clone()))
        .collect();
    let shared: Vec<u32> = config
        .channels
        .iter()
        .find(|channel| channel.is_shared())
        .map(|channel| channel.maps.clone())
        .unwrap_or_default();
    let mut game_state = GameState::new(Arc::clone(&data.protos))
        .with_item_count_limit(config.game.item_count_limit)
        .with_npc_shops(Arc::clone(&data.shops))
        .with_drop_tables(Arc::clone(&data.drops))
        .with_item_attrs(Arc::clone(&data.attrs))
        .with_item_masks(Arc::clone(&data.masks))
        .with_motions(Arc::clone(&data.motions))
        .with_mob_skills(Arc::clone(&data.mob_skills))
        .with_mob_clips(Arc::clone(&data.mob_clips))
        .with_pc_skills(Arc::clone(&data.pc_skills))
        .with_guild_skills(Arc::clone(&data.guild_skills))
        .with_shop_price_3x_disabled(config.game.disable_shop_price_3x)
        .with_test_server(config.game.test_server)
        .with_drop_lifetime(config.game.item_destroy_time_dropitem)
        .with_gold_lifetime(config.game.item_destroy_time_dropgold)
        .with_clients(Arc::clone(clients))
        .with_clock(Box::new(clock))
        .with_view_range(config.game.view_range)
        .with_no_wander(config.game.no_wander)
        .with_locale_strings(Arc::clone(&data.locale))
        .with_quests(quests)
        .with_max_level(config.game.max_level)
        .with_status_point_get_level_limit(config.game.status_point_get_level_limit)
        .with_protect_level(config.game.pk_protect_level)
        .with_recall(Arc::clone(&data.atlas), &channel_maps, &shared);
    stand_up_npcs(&mut game_state, &data.npcs, &data.cells, &data.mob_clips)?;
    Ok(game_state)
}

/// Stop and join the game thread and close the store after the accept loop failed, so the
/// error `serve` returns is not raced by a world still running.
async fn clean_up_after_error(
    controller: &GameLoopController,
    game_loop: GameLoopHandle,
    store: &Store,
) {
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

    let (data, quests) = load_game_data(&config)?;

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

    // One client registry serves the descriptors and the world, which tells a map when an item
    // on its ground is destroyed.
    let clients = Arc::new(ChannelClients::new());
    // One `get_dword_time()` for the process: the descriptors' handshakes and the world's
    // moves read the same clock.
    let clock = BootLiveClock::new();
    let game_state = build_world(&config, &data, quests, &clients, clock)?;
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
            data,
            (clients, clock),
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
            clean_up_after_error(&controller, game_loop, &store).await;
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

/// `interpret_command` for a slash line, with the commands this build ports.
async fn run_command<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    line: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // The character's GM level, read at the character load and held for the
    // descriptor's life: it gates the GM commands and lifts the flood limit
    // (`interpret_command`, `G/cmd.cpp:611-616`).
    let caller = Caller {
        pulse: context.game.pulse(),
        gm_level: held.gm_level,
        position: caller_position(session.phase(), held),
    };
    let (entry, rest) = match interpret(line, &mut held.command_flood, caller) {
        Interpreted::Nothing => return true,
        Interpreted::Disconnect => {
            info!(%addr, "Command lines flooded; closing");
            return false;
        }
        Interpreted::Notice(text) => {
            return send_line(session, addr, context, held, CHAT_TYPE_INFO, text).await;
        }
        Interpreted::Run { entry, rest } => (entry, rest),
    };
    match entry.command() {
        LineCommand::TypeInFull => {
            send_line(session, addr, context, held, CHAT_TYPE_INFO, TYPE_IN_FULL).await
        }
        LineCommand::ClickSafebox => {
            let text = "ShowMeSafeboxPassword";
            send_line(session, addr, context, held, CHAT_TYPE_COMMAND, text).await
        }
        LineCommand::ClickMall => {
            let text = "ShowMeMallPassword";
            send_line(session, addr, context, held, CHAT_TYPE_COMMAND, text).await
        }
        LineCommand::SafeboxClose => {
            let step = SafeboxStep::Close;
            safebox_step(session, addr, context, held, step)
                .await
                .unwrap_or(true)
        }
        LineCommand::MallClose => {
            let step = SafeboxStep::CloseMall;
            safebox_step(session, addr, context, held, step)
                .await
                .unwrap_or(true)
        }
        LineCommand::SafeboxPassword => {
            open_store(session, addr, context, held, false, &rest).await
        }
        LineCommand::MallPassword => open_store(session, addr, context, held, true, &rest).await,
        LineCommand::SafeboxChangePassword => {
            change_store_password(session, addr, context, held, &rest).await
        }
        LineCommand::Restart(kind) => restart(addr, context, held, kind).await,
        LineCommand::SkillUp => skill_up(addr, context, held, rest).await,
        LineCommand::GuildSkillUp => guild_skill_up_step(addr, context, held, rest).await,
        LineCommand::GuildWar => guild_war_step(addr, context, held, rest).await,
        LineCommand::GuildNoWar => guild_nowar_step(addr, context, held, rest).await,
        LineCommand::GuildWarStop => guild_war_stop_step(addr, context, held, rest).await,
        LineCommand::GuildWarCancel => guild_war_cancel_step(addr, context, held, rest).await,
        LineCommand::GuildWarList => guild_war_list_step(session, addr, context, held).await,
        LineCommand::PartyRequest => party_request_step(addr, context, held, rest).await,
        LineCommand::PartyRequestAccept => {
            party_request_accept_step(addr, context, held, rest).await
        }
        LineCommand::PartyRequestDeny => party_request_deny_step(addr, context, held, rest).await,
        LineCommand::MessengerAuth => messenger_auth_step(addr, context, held, rest).await,
        LineCommand::SetBlockMode => set_block_mode_step(addr, context, held, rest).await,
        LineCommand::BlockChat => block_chat_step(addr, context, held, rest).await,
        LineCommand::GlobalNotice => global_notice_step(addr, context, held, &rest),
        LineCommand::MapNotice => map_notice_step(addr, context, held, &rest),
        LineCommand::BigNotice => big_notice_step(session, addr, context, held, &rest).await,
        LineCommand::NotPorted(handler) => {
            info!(%addr, handler, "Command not ported; ignoring");
            true
        }
    }
}

/// The position a command line's caller runs it from. A dead player's session is in the dead
/// phase, which `POS_DEAD` goes with; sitting is the one other position besides standing this
/// build tracks.
fn caller_position(phase: ClientPhase, held: &Held) -> u8 {
    let sitting = held.avatar.as_ref().is_some_and(|avatar| avatar.sitting);
    if phase == ClientPhase::Dead {
        POS_DEAD
    } else if sitting {
        POS_SITTING
    } else {
        POS_STANDING
    }
}

/// `do_restart` and `do_restart_now` (`G/cmd_general.cpp:471-706`): the world runs the restart
/// and writes what it shows to this client's queue, and the gold `/restart_now` took is stored
/// as a transfer, the one way gold changes in the store (ledger 225). A world that cannot be
/// asked, or a store that refuses the gold, closes the connection as an item step does. Unlike
/// an item step, the gold record has already left, behind the revive's records in the world's
/// queue; a refused store closes the session it went to, and the store keeps the gold.
async fn restart(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    kind: RestartKind,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, ?kind, "Restart without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.restart(vid, kind, item_actor(held)).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a restart; closing");
            return false;
        }
    };
    let Some(gold) = answer.gold else {
        return true;
    };
    if let Err(error) = db::items::apply_transfer(&context.store, vid.raw(), &[], gold).await {
        warn!(%addr, %error, gold, "The restart's gold could not be stored; closing");
        return false;
    }
    info!(%addr, gold, "Restarted for gold");
    true
}
/// `do_skillup`: the world runs the `/skillup` line, and a landed up's slot is
/// stored (`Save()` writes the skills with the row).
async fn skill_up(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Skill-up without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.skill_up(vid, rest).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a skill-up; closing");
            return false;
        }
    };
    let Some((vnum, level)) = answer.climbed else {
        return true;
    };
    let Some(character) = held.character.as_ref() else {
        return true;
    };
    if let Err(error) =
        db::player_skills::store_skill(&context.store, character.id, vnum, level, 0).await
    {
        warn!(%addr, %error, vnum, level, "The skill-up could not be stored; closing");
        return false;
    }
    info!(%addr, vnum, level, "Skill up");
    true
}

/// `do_setblockmode` (`G/cmd_general.cpp:2300-2309`): the world runs the line,
/// the `setblockmode %d` command line rode the outbox already, and the seven
/// `game_option.block_*` flag writes are stored (`SetQuestFlag` writes the
/// rows). An empty line is quiet (`if (*arg1)`): the world answers no line
/// and no flags, and there is nothing to store.
async fn set_block_mode_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Block mode without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.set_block_mode(vid, rest).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a block mode; closing");
            return false;
        }
    };
    let Some(character) = held.character.as_ref() else {
        return true;
    };
    // `SetQuestFlag` writes the world's quest record now (`G/char.cpp:7649-7655`),
    // and the save writes the descriptor's record later (`G/char.cpp:1673-1679`):
    // the flag moves into the held record first, and the whole record is stored.
    for (name, value) in &answer.flags {
        if let Some(slot) = held.quest_flags.iter_mut().find(|(held, _)| held == name) {
            slot.1 = *value;
        } else {
            held.quest_flags.push((name.clone(), *value));
        }
    }
    held.quest_flags.retain(|(_, value)| *value != 0);
    let flags: Vec<db::quest_flags::StoredQuestFlag> = held
        .quest_flags
        .iter()
        .map(|(name, value)| db::quest_flags::StoredQuestFlag {
            name: name.clone(),
            value: *value,
        })
        .collect();
    if let Err(error) = db::quest_flags::save_flags(&context.store, character.id, &flags).await {
        warn!(%addr, %error, "The block mode could not be stored; closing");
        return false;
    }
    info!(%addr, "Block mode");
    true
}

/// `do_block_chat` (`G/cmd_gm.cpp:3364-3433`): the world lays the silence the line
/// names and answers what moved. A moved live character's lay is stored: the
/// `QUERY_ADD_AFFECT` write `AddAffect` makes for a PC
/// (`G/char_affect.cpp:756-765`), which a 0-spelling reaches too (its lay stands
/// on the next tick, as the loaded row would). An offline name's row is stored
/// by the name (`CClientManager::BlockChat`, `db/ClientManager.cpp:2159-2186`),
/// quiet when no row carries it. Every line rode the outboxes already: the
/// usage and format lines and the privilege refusal to the caller, the
/// `[LS;852]` to the target, and the requested line when the name named nobody
/// online. A refused store closes the session, as a refused block mode does.
async fn block_chat_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    use prodomo::game_loop_messages::BlockChatAnswer;
    let Some(vid) = held.world else {
        info!(%addr, "Block chat without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.block_chat(vid, rest).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a chat block; closing");
            return false;
        }
    };
    match answer {
        BlockChatAnswer::Shown => true,
        BlockChatAnswer::Moved {
            name,
            player,
            seconds,
        } => {
            let rows = [db::player_affects::StoredAffect {
                affect_type: prodomo::loading_phase::AFFECT_BLOCK_CHAT,
                apply_on: 0,
                apply_value: 0,
                flag: 0,
                duration: seconds,
                sp_cost: 0,
            }];
            if let Err(error) =
                db::player_affects::save_affects(&context.store, player, &rows).await
            {
                warn!(%addr, %error, "The chat block could not be stored; closing");
                return false;
            }
            info!(
                %addr,
                name = String::from_utf8_lossy(&name).into_owned(),
                seconds,
                "Chat block stored"
            );
            true
        }
        BlockChatAnswer::Offline { name, seconds } => {
            match db::player_affects::lay_affect(
                &context.store,
                &String::from_utf8_lossy(&name),
                seconds,
            )
            .await
            {
                Ok(laid) => {
                    info!(
                        %addr,
                        name = String::from_utf8_lossy(&name).into_owned(),
                        seconds,
                        laid,
                        "Offline chat block stored"
                    );
                    true
                }
                Err(error) => {
                    warn!(%addr, %error, "The offline chat block could not be stored; closing");
                    false
                }
            }
        }
    }
}

/// `do_guildskillup`: the world runs the `/gskillup` line, and a landed up's row is
/// stored (`SaveSkill` writes the power, the point and the blob). A refused up is
/// quiet on the descriptor: every line rode the outbox already.
async fn guild_skill_up_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Guild skill-up without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.guild_skill_up(vid, rest).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a guild skill-up; closing");
            return false;
        }
    };
    let Some(climbed) = answer.climbed else {
        return true;
    };
    if let Err(error) = db::guilds::set_skills(
        &context.store,
        climbed.guild_id,
        &climbed.skills,
        climbed.point,
    )
    .await
    {
        warn!(%addr, %error, vnum = climbed.vnum, level = climbed.level, "The guild skill-up could not be stored; closing");
        return false;
    }
    info!(%addr, vnum = climbed.vnum, level = climbed.level, "Guild skill up");
    true
}

/// `do_war`: the world runs the `/war` line and may answer a declare to announce — the
/// two guild names for the `[LS;475;%s;%s]` notice (`SendNotice`, `G/cmd_gm.cpp:1389`,
/// via `BroadcastNotice`'s relay, `G/cmd_general.cpp:2828`). The notice leaves after
/// the world's fans reached the outboxes, so every client hears the WAR records
/// before what they mean; there is no row to store, declares living only in memory.
/// A refused line is quiet on the descriptor: every line rode the outbox already.
async fn guild_war_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Guild war without a character in the world; ignoring");
        return true;
    };
    let answer = match context.game.guild_war(vid, rest, global_time()).await {
        Ok(answer) => answer,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a guild war; closing");
            return false;
        }
    };
    let Some(notice) = answer.notice else {
        return true;
    };
    let (key, first, second) = match &notice {
        prodomo::game_state::GuildWarNotice::Declare { from_name, to_name } => {
            (&b"[LS;475;%s;%s]"[..], from_name, to_name)
        }
        prodomo::game_state::GuildWarNotice::Accept { from_name, to_name } => {
            (&b"[LS;477;%s;%s]"[..], from_name, to_name)
        }
    };
    let sent = context.clients.deliver_everywhere(|entry| {
        let to = prodomo::chat_line::Recipient {
            strings: &context.locale,
            language: entry.language,
            empire: entry.empire,
        };
        Some(prodomo::chat_line::chat_packet(
            to,
            protocol::gc_chat::CHAT_TYPE_NOTICE,
            key,
            &[
                prodomo::chat_line::Arg::Text(first),
                prodomo::chat_line::Arg::Text(second),
            ],
        ))
    });
    info!(%addr, recipients = sent, "Guild war declared");
    true
}

/// `do_nowar`: the world runs the `/nowar` line; the refuse's lines all rode the
/// outbox already, so the descriptor only logs.
async fn guild_nowar_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Guild nowar without a character in the world; ignoring");
        return true;
    };
    if let Err(error) = context.game.guild_nowar(vid, rest).await {
        warn!(%addr, %error, "The world could not be asked for a guild nowar; closing");
        return false;
    }
    info!(%addr, "Guild war refused");
    true
}
/// `gwstop`: the world ends the pair in a forced draw; the descriptor writes both rows.
/// Every fan rode the outbox already, so a live war only stores, and a non-war is
/// quiet past the echo. Answers whether the session stays open.
async fn guild_war_stop_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Guild war-stop without a character in the world; ignoring");
        return true;
    };
    let outcome = match context.game.guild_war_stop(vid, rest, global_time()).await {
        Ok(outcome) => outcome,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a guild war-stop; closing");
            return false;
        }
    };
    let Some(outcome) = outcome else {
        return true;
    };
    for row in [outcome.lo, outcome.hi] {
        if let Err(error) = db::guilds::set_war_tallies(
            &context.store,
            row.guild_id,
            row.ladder,
            row.wins,
            row.draws,
            row.losses,
        )
        .await
        {
            warn!(%addr, %error, "A war-end row could not be stored; closing");
            return false;
        }
    }
    info!(%addr, "Guild war stopped");
    true
}

/// `gwcancel`: the world cancels the pair with no ladder math; nothing is stored.
/// Every fan rode the outbox already, so the descriptor only logs.
async fn guild_war_cancel_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Guild war-cancel without a character in the world; ignoring");
        return true;
    };
    if let Err(error) = context
        .game
        .guild_war_cancel(vid, rest, global_time())
        .await
    {
        warn!(%addr, %error, "The world could not be asked for a guild war-cancel; closing");
        return false;
    }
    info!(%addr, "Guild war cancelled");
    true
}

/// `do_party_request` (`G/cmd_general.cpp:2428-2455`): the world runs the line; the
/// gates' info lines and the leader's `PartyRequest <VID>` command ride the outbox, so
/// the descriptor only logs.
async fn party_request_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Party request without a character in the world; ignoring");
        return true;
    };
    match context.game.party_request(vid, rest).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a party request; closing");
            return false;
        }
    }
    info!(%addr, "Party request");
    true
}

/// `do_party_request_accept` (`G/cmd_general.cpp:2457-2472`): the world runs the line;
/// the guest's join walk and the denial lines ride the outbox.
async fn party_request_accept_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Party accept without a character in the world; ignoring");
        return true;
    };
    match context.game.party_request_accept(vid, rest).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a party accept; closing");
            return false;
        }
    }
    info!(%addr, "Party request accepted");
    true
}

/// `do_party_request_deny` (`G/cmd_general.cpp:2475-2487`): the world runs the line; the
/// guest's `PartyRequestDenied` command rides the outbox.
async fn party_request_deny_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Party deny without a character in the world; ignoring");
        return true;
    };
    match context.game.party_request_deny(vid, rest).await {
        Ok(settling) => held.settling = Some(settling),
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a party deny; closing");
            return false;
        }
    }
    info!(%addr, "Party request denied");
    true
}
/// `do_messenger_auth` (`G/cmd_general.cpp:2272-2296`): the world consumes the request the
/// second word names and answers whether it became a friendship, and the caller writes
/// the pair's rows when it did. An empty line is quiet (`if (!*arg1 || !*arg2) return`),
/// as is an answer to a request nobody made. The deny's own `[LS;548]` line rode the
/// outbox already.
async fn messenger_auth_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: Vec<u8>,
) -> bool {
    let Some(vid) = held.world else {
        info!(%addr, "Messenger auth without a character in the world; ignoring");
        return true;
    };
    // `two_arguments(argument, arg1, arg2, ...)`: the answer and the requester's name.
    let mut words = rest
        .split(|&byte| byte == b' ' || byte == 0x09)
        .filter(|word| !word.is_empty());
    let (Some(answer), Some(requester)) = (words.next(), words.next()) else {
        return true;
    };
    let deny = !matches!(answer.first(), Some(b'y' | b'Y'));
    let added = match context
        .game
        .messenger_auth(vid, requester.to_vec(), deny)
        .await
    {
        Ok(added) => added,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a friend answer; closing");
            return false;
        }
    };
    // A deny consumes the request and answers the requester, but writes no rows:
    // `AuthToAdd` only inserts on a yes (`G/messenger_manager.cpp:330-332`).
    if deny || !added {
        return true;
    }
    let Some(character) = held.character.as_ref() else {
        return true;
    };
    if let Err(error) = db::messenger::add_pair(
        &context.store,
        character.name.as_str(),
        &String::from_utf8_lossy(requester),
    )
    .await
    {
        warn!(%addr, %error, "The friendship could not be stored; closing");
        return false;
    }
    info!(%addr, "Friend answer stored");
    true
}

/// `gwlist`: the GM hears the fixed line (`do_gwlist`, `G/cmd_gm.cpp:2760-2764`) and one
/// line per live war (`ShowGuildWarList`, `G/guild_manager.cpp:785-800`).
async fn guild_war_list_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if held.world.is_none() {
        info!(%addr, "Guild war-list without a character in the world; ignoring");
        return true;
    }
    let lines = match context.game.guild_war_list(global_time()).await {
        Ok(lines) => lines,
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for the guild war list; closing");
            return false;
        }
    };
    if !send_line(
        session,
        addr,
        context,
        held,
        protocol::gc_chat::CHAT_TYPE_NOTICE,
        "This is a guild that is currently at war.",
    )
    .await
    {
        return false;
    }
    for line in lines {
        let args = [
            prodomo::chat_line::Arg::Text(&line.lo_name),
            prodomo::chat_line::Arg::Int(i64::from(line.lo_id)),
            prodomo::chat_line::Arg::Text(&line.hi_name),
            prodomo::chat_line::Arg::Int(i64::from(line.hi_id)),
            prodomo::chat_line::Arg::Int(i64::from(line.seconds)),
        ];
        if !send_line_args(
            session,
            addr,
            context,
            held,
            protocol::gc_chat::CHAT_TYPE_NOTICE,
            b"%s[%d] vs %s[%d] time %u sec.",
            &args,
        )
        .await
        {
            return false;
        }
    }
    info!(%addr, "Guild war list sent");
    true
}

/// `do_notice` (`G/cmd_gm.cpp:1628-1637`): `__GM_PE_N__` is defined
/// (`common/prodomodefines.h:152`), so the line is `"<GM Name> :<argument>"`, the
/// argument keeping the spaces after the command word. `SendNotice` (`:1389-1425`)
/// looks the composed line up in every recipient's language. Every Channel is this
/// one process, so the registry walks them all; legacy relayed the line to another
/// Core as `HEADER_GG_NOTICE` (`:1513-1536`), and that protocol is retired (ADR-0001).
fn global_notice_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    rest: &[u8],
) -> bool {
    let Some(character) = held.character.as_ref() else {
        info!(%addr, "Notice without a loaded character; ignoring");
        return true;
    };
    let mut text = Vec::with_capacity(character.name.len() + 3 + rest.len());
    text.extend_from_slice(character.name.as_bytes());
    text.extend_from_slice(b" :");
    text.extend_from_slice(rest);
    let sent = context.clients.deliver_everywhere(|entry| {
        Some(prodomo::chat_line::chat_packet(
            Recipient {
                strings: &context.locale,
                language: entry.language,
                empire: entry.empire,
            },
            CHAT_TYPE_NOTICE,
            &text,
            &[],
        ))
    });
    info!(%addr, recipients = sent, "Notice sent");
    true
}

/// `do_map_notice` (`G/cmd_gm.cpp:1639-1642`): `SendNoticeMap(argument, GetMapIndex(),
/// false)` (`:1453-1489`) says `CHAT_TYPE_NOTICE` to every character on the map.
///
/// Legacy walks one process's descriptors, and a process is one Channel, so the
/// registry's `channel` and `map` must both equal the caller's. Hosting every Channel
/// in one process makes that filter explicit; without it a Channel-99 client standing
/// on a map index another Channel also hosts would hear a line legacy could not send.
fn map_notice_step(
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    rest: &[u8],
) -> bool {
    let Some(map) = held.map else {
        info!(%addr, "Map notice without a loaded map; ignoring");
        return true;
    };
    let channel = held.channel;
    let sent = context.clients.deliver_everywhere(|entry| {
        (entry.channel == channel && entry.map == map).then(|| {
            prodomo::chat_line::chat_packet(
                Recipient {
                    strings: &context.locale,
                    language: entry.language,
                    empire: entry.empire,
                },
                CHAT_TYPE_NOTICE,
                rest,
                &[],
            )
        })
    });
    info!(%addr, channel, map, recipients = sent, "Map notice sent");
    true
}

/// `do_big_notice` (`G/cmd_gm.cpp:1644-1651`): `ENABLE_FULL_NOTICE` is not defined in
/// this tree, so only the caller hears its argument, in the big font. Legacy's
/// `ChatPacket(..., "%s", argument)` does not translate the argument, so it is the
/// `%s` argument here too; a `%` in it stays literal.
async fn big_notice_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    rest: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let args = [prodomo::chat_line::Arg::Text(rest)];
    let sent = send_line_args(
        session,
        addr,
        context,
        held,
        CHAT_TYPE_BIG_NOTICE,
        b"%s",
        &args,
    )
    .await;
    if sent {
        info!(%addr, "Big notice sent");
    }
    sent
}

/// One server line to this descriptor, in its language.
async fn send_line<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    chat_type: u8,
    text: &str,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_line_args(
        session,
        addr,
        context,
        held,
        chat_type,
        text.as_bytes(),
        &[],
    )
    .await
}

/// One server line to this descriptor, in its language, with its format arguments.
async fn send_line_args<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    chat_type: u8,
    text: &[u8],
    args: &[prodomo::chat_line::Arg<'_>],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let to = item_actor(held).recipient(&context.locale);
    let line = prodomo::chat_line::chat_packet(to, chat_type, text, args);
    send_shop_records(session, addr, &[line]).await
}

/// `do_safebox_password` or `do_mall_password`: check the password, then load the rows.
async fn open_store<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    mall: bool,
    rest: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (word, _) = one_argument(rest);
    let Ok(password) = db::safebox::SafeboxPassword::new(&word) else {
        return send_line(session, addr, context, held, CHAT_TYPE_INFO, "[LS;526]").await;
    };
    let Some(account) = held.account.as_ref().map(|account| account.id) else {
        return true;
    };
    let begin = if mall {
        SafeboxStep::BeginMall
    } else {
        SafeboxStep::BeginOpen
    };
    if let Some(alive) = safebox_step(session, addr, context, held, begin).await {
        return alive;
    }
    let step = match db::safebox::verify_password(&context.store, account, &password).await {
        Ok(false) => SafeboxStep::WrongPassword { mall },
        Ok(true) => {
            let window = if mall {
                db::items::MALL
            } else {
                db::items::SAFEBOX
            };
            let rows =
                match db::items::load_account_items(&context.store, account.get(), window).await {
                    Ok(rows) => rows,
                    Err(error) => {
                        warn!(%addr, %error, "The stored items could not be loaded; closing");
                        return false;
                    }
                };
            let (items, refused) = prodomo::item_load::stored_items(&rows, &context.protos);
            for refusal in &refused {
                warn!(%addr, ?refusal, "A stored item was not loaded");
            }
            let account = account.get();
            if mall {
                SafeboxStep::OpenMall { account, items }
            } else {
                SafeboxStep::Open { account, items }
            }
        }
        Err(error) => {
            warn!(%addr, %error, "The safebox password could not be checked; closing");
            return false;
        }
    };
    safebox_step(session, addr, context, held, step)
        .await
        .unwrap_or(true)
}

/// `do_safebox_change_password`.
async fn change_store_password<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &Held,
    rest: &[u8],
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (old, rest) = one_argument(rest);
    let (new, _) = one_argument(rest);
    let checked = (
        db::safebox::SafeboxPassword::new(&old),
        db::safebox::SafeboxPassword::new(&new),
    );
    let (Ok(old), Ok(new)) = checked else {
        return send_line(session, addr, context, held, CHAT_TYPE_INFO, "[LS;526]").await;
    };
    let Some(account) = held.account.as_ref().map(|account| account.id) else {
        return true;
    };
    let text = match db::safebox::change_password(&context.store, account, &old, &new).await {
        Ok(true) => "[LS;774]",
        Ok(false) => "[LS;775]",
        Err(error) => {
            warn!(%addr, %error, "The safebox password could not be changed; closing");
            return false;
        }
    };
    send_line(session, addr, context, held, CHAT_TYPE_INFO, text).await
}

/// Whether the header is a trade record or a safebox record.
fn is_transaction_record(header: u8) -> bool {
    header == HEADER_CG_EXCHANGE.value()
        || SafeBoxKind::from_header(header).is_some()
        || header == CgSafeboxItemMove::header().value()
}

/// A trade record or a safebox record.
async fn transaction_record<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if frame.header == HEADER_CG_EXCHANGE.value() {
        trade_step(session, addr, context, held, frame).await
    } else {
        store_record(session, addr, context, held, frame).await
    }
}

/// A checkin, a checkout or a move inside the safebox, as its client record asks.
async fn store_record<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    frame: &ClientFrame,
) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let step = if frame.header == CgSafeboxItemMove::header().value() {
        match CgSafeboxItemMove::decode_frame(frame) {
            Ok(record) => SafeboxStep::Move {
                from: u32::from(record.from.cell),
                to: u32::from(record.to.cell),
                count: record.count,
            },
            Err(error) => {
                warn!(%addr, %error, "Client sent a malformed safebox move; closing");
                return false;
            }
        }
    } else {
        match CgSafeBoxItem::decode_frame(frame) {
            Ok(record) if record.kind.is_deposit() => SafeboxStep::Checkin {
                from: record.item_pos,
                safe_pos: record.container_pos,
            },
            Ok(record) => SafeboxStep::Checkout {
                safe_pos: record.container_pos,
                to: record.item_pos,
                mall: record.kind == SafeBoxKind::MallCheckout,
            },
            Err(error) => {
                warn!(%addr, %error, "Client sent a malformed safebox record; closing");
                return false;
            }
        }
    };
    safebox_step(session, addr, context, held, step)
        .await
        .unwrap_or(true)
}

/// Run one safebox step: `None` when the connection is to check the password and load the
/// rows, else whether the connection lives on.
async fn safebox_step<S>(
    session: &mut LiveClientSession<S>,
    addr: SocketAddr,
    context: &ConnectionContext,
    held: &mut Held,
    step: SafeboxStep,
) -> Option<bool>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(vid) = held.world else {
        info!(%addr, "Safebox step without a character in the world; ignoring");
        return Some(true);
    };
    let actor = item_actor(held);
    let answer = match context.game.safebox(vid, step, actor).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(refused)) => {
            warn!(%addr, %refused, "The world does not hold this descriptor's character; closing");
            return Some(false);
        }
        Err(error) => {
            warn!(%addr, %error, "The world could not be asked for a safebox step; closing");
            return Some(false);
        }
    };
    Some(match answer {
        SafeboxAnswer::Proceed => return None,
        SafeboxAnswer::Sent(records) => send_shop_records(session, addr, &records).await,
        SafeboxAnswer::Moved(moved) => {
            finish_item_step(session, addr, context, held, actor, Ok(Ok(moved))).await
        }
        SafeboxAnswer::Rearranged {
            account,
            changes,
            records,
        } => {
            let stored = db::items::apply_account_changes(&context.store, account, &changes);
            if let Err(error) = stored.await {
                warn!(%addr, %error, "The safebox move could not be stored; closing");
                return Some(false);
            }
            send_shop_records(session, addr, &records).await
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prodomo::descriptor_crypto::DescriptorCrypto;
    use prodomo::lifecycle::ClientLifecycle;
    use protocol::cg_handshake::{CgHandshakeHeader, CgInboundHandshake};
    use tokio::io::{AsyncReadExt, DuplexStream};

    /// A session in the game phase, behind the TEA boundary a login installs, and the client's
    /// end of its socket.
    fn game_session() -> (LiveClientSession<DuplexStream>, DuplexStream) {
        let (ours, theirs) = tokio::io::duplex(4096);
        let token = 0x0102_0304;
        let handshake = CgInboundHandshake::new(CgHandshakeHeader::Handshake, token, 0, 0);
        let lifecycle = ClientLifecycle::start(token, HandshakeServerKind::Game, 0)
            .state
            .on_handshake(handshake, 0)
            .expect("the handshake logs in")
            .state
            .transition_to(PostHandshakePhase::Game)
            .expect("a login enters the game")
            .state;
        let mut crypto = DescriptorCrypto::with_default_limit().expect("a cipher");
        crypto
            .enable_default_legacy_tea()
            .expect("the default pair");
        let session =
            LiveClientSession::with_cipher(ours, lifecycle, crypto).expect("a game session");
        (session, theirs)
    }

    /// A player sitting on map 41.
    fn sitting() -> Avatar {
        Avatar {
            map: 41,
            x: 1,
            y: 2,
            sitting: true,
        }
    }

    /// The peer address the descriptor's log lines carry.
    fn addr() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 13_000))
    }

    /// A record the world queued before an order reaches the client before anything following
    /// the order writes, so a warp's departure never overtakes the records that led to it.
    #[tokio::test]
    async fn an_order_follows_the_records_queued_before_it() {
        let (ours, mut theirs) = tokio::io::duplex(4096);
        let lifecycle = ClientLifecycle::start(1, HandshakeServerKind::Game, 0).state;
        let mut session = LiveClientSession::new(ours, lifecycle).expect("a plaintext session");
        let (queue, receiver) = mpsc::unbounded_channel();
        queue.send(vec![0xA1, 0xA2]).expect("the queue is open");
        let mut outbox = Some(receiver);
        let mut pending = vec![vec![0xA0]];
        let mut held = Held::default();

        let follow = async |session: &mut LiveClientSession<DuplexStream>, _: &mut Held| {
            session.send(&[0xB1]).await.is_ok()
        };
        let taken = take_order(
            &mut session,
            addr(),
            &mut outbox,
            &mut pending,
            &mut held,
            follow,
        );
        assert!(taken.await);
        assert!(pending.is_empty());
        let refused = async |_: &mut LiveClientSession<DuplexStream>, _: &mut Held| false;
        let taken = take_order(
            &mut session,
            addr(),
            &mut outbox,
            &mut pending,
            &mut held,
            refused,
        );
        assert!(!taken.await);
        drop(session);

        let mut written = Vec::new();
        theirs
            .read_to_end(&mut written)
            .await
            .expect("the session wrote and closed");
        assert_eq!(
            written,
            [0xA0, 0xA1, 0xA2, 0xB1],
            "pending, then queued, then the order"
        );
    }

    /// A world's `GC_PHASE` record of the dead or the game phase switches the descriptor's
    /// phase; a record of another length, header or phase is written as it is.
    #[test]
    fn a_phase_record_switches_the_phase_and_nothing_else_does() {
        let header = HEADER_GC_PHASE.value();
        assert_eq!(
            phase_of_record(&[header, PHASE_DEAD]),
            Some(PostHandshakePhase::Dead)
        );
        assert_eq!(
            phase_of_record(&[header, PHASE_GAME]),
            Some(PostHandshakePhase::Game)
        );
        for record in [
            vec![],
            vec![header],
            vec![header, PHASE_DEAD, 0],
            vec![header, PHASE_DEAD - 2],
            vec![header, PHASE_DEAD + 1],
            vec![HEADER_GC_EMPIRE.value(), PHASE_DEAD],
        ] {
            assert_eq!(phase_of_record(&record), None, "{record:?}");
        }
    }

    /// The dead phase reads the main input as the game phase does (`G/desc.cpp:526-531`), and
    /// consumes the same records; it alone refuses `CG_CHARACTER_POSITION` (B25).
    #[test]
    fn a_dead_session_reads_the_main_input() {
        assert!(in_main_input(ClientPhase::Game));
        assert!(in_main_input(ClientPhase::Dead));
        for phase in [
            ClientPhase::Close,
            ClientPhase::Handshake,
            ClientPhase::Login,
            ClientPhase::Select,
            ClientPhase::Loading,
        ] {
            assert!(!in_main_input(phase), "{phase:?}");
        }
        assert!(is_consumed(ClientPhase::Dead, HEADER_CG_ENTERGAME.value()));
        // `CG_USE_SKILL` runs the melee casts (230d); the fly and shoot records run
        // the world steps (230e).
        for header in [
            HEADER_CG_FLY_TARGETING,
            HEADER_CG_ADD_FLY_TARGETING,
            HEADER_CG_SHOOT,
        ] {
            assert!(!is_consumed(ClientPhase::Dead, header.value()));
        }
        assert!(!is_consumed(ClientPhase::Dead, HEADER_CG_USE_SKILL.value()));
        assert!(!is_consumed(ClientPhase::Dead, HEADER_CG_ITEM_USE.value()));

        let position = HEADER_CG_CHARACTER_POSITION.value();
        assert!(is_dead_refused(ClientPhase::Dead, position));
        assert!(!is_dead_refused(ClientPhase::Game, position));
        for header in [HEADER_CG_MOVE, HEADER_CG_CHAT, HEADER_CG_ITEM_USE] {
            assert!(!is_dead_refused(ClientPhase::Dead, header.value()));
        }
    }

    /// A death or a restart stands a sitting player up; a turn that leaves the phase as it was
    /// leaves the pose.
    #[test]
    fn a_phase_change_stands_the_player_up() {
        let mut held = Held {
            avatar: Some(sitting()),
            ..Held::default()
        };
        stand_on_phase_change(&mut held, ClientPhase::Game, ClientPhase::Game);
        assert_eq!(held.avatar, Some(sitting()));
        stand_on_phase_change(&mut held, ClientPhase::Game, ClientPhase::Dead);
        assert_eq!(
            held.avatar.as_ref().map(|avatar| avatar.sitting),
            Some(false)
        );

        held.avatar = Some(sitting());
        stand_on_phase_change(&mut held, ClientPhase::Dead, ClientPhase::Game);
        assert_eq!(
            held.avatar.as_ref().map(|avatar| avatar.sitting),
            Some(false)
        );
        let mut away = Held::default();
        stand_on_phase_change(&mut away, ClientPhase::Game, ClientPhase::Dead);
        assert!(away.avatar.is_none());
    }

    /// The main input reads a dead player's chat, which carries its restart lines, as it reads a
    /// living one's, and routes every other record alike in both phases, except the pose: a dead
    /// player's is refused (B25), a living one's goes to the world.
    #[test]
    fn a_dead_player_chats_as_a_living_one_but_its_pose_is_refused() {
        let routes = [
            (HEADER_CG_CHAT, MainRecord::Chat),
            (HEADER_CG_ITEM_MOVE, MainRecord::ItemMove),
            (HEADER_CG_ITEM_USE, MainRecord::Belongings),
            (HEADER_CG_MOVE, MainRecord::World),
            (HEADER_CG_ON_CLICK, MainRecord::Npc),
            (HEADER_CG_EXCHANGE, MainRecord::Transaction),
            (HEADER_CG_WARP, MainRecord::Warp),
        ];
        let position = HEADER_CG_CHARACTER_POSITION.value();
        let unrouted = HEADER_CG_LOGIN2.value();
        for phase in [ClientPhase::Game, ClientPhase::Dead] {
            for (header, route) in routes {
                assert_eq!(main_record(phase, header.value()), Some(route), "{phase:?}");
            }
            assert_eq!(main_record(phase, unrouted), None, "{phase:?}");
        }
        let (dead, living) = (ClientPhase::Dead, ClientPhase::Game);
        assert_eq!(main_record(dead, position), Some(MainRecord::Refused));
        assert_eq!(main_record(living, position), Some(MainRecord::World));
        for phase in [ClientPhase::Loading, ClientPhase::Close] {
            for header in [HEADER_CG_CHAT.value(), position] {
                assert_eq!(main_record(phase, header), None, "{phase:?}");
            }
        }
    }

    /// A world's `GC_PHASE` record is `DESC::SetPhase`: writing it installs the phase it names,
    /// the dead phase at a death and the game phase at a restart. Another record keeps the phase.
    #[tokio::test]
    async fn a_phase_record_the_world_wrote_installs_its_phase() {
        let (mut session, _client) = game_session();
        let header = HEADER_GC_PHASE.value();
        let mut pending = vec![vec![HEADER_GC_EMPIRE.value(), 1]];
        assert!(write_pending(&mut session, addr(), &mut pending).await);
        assert_eq!(session.phase(), ClientPhase::Game);

        let mut pending = vec![vec![header, PHASE_DEAD]];
        assert!(write_pending(&mut session, addr(), &mut pending).await);
        assert_eq!(session.phase(), ClientPhase::Dead);
        let mut pending = vec![vec![header, PHASE_GAME], vec![HEADER_GC_EMPIRE.value(), 1]];
        assert!(write_pending(&mut session, addr(), &mut pending).await);
        assert_eq!(session.phase(), ClientPhase::Game);
        assert!(pending.is_empty());
    }

    /// The read loop's flush of the world's queue stands a sitting player up when it writes a
    /// death (critic 8); a flush that keeps the phase keeps the pose.
    #[tokio::test]
    async fn a_flushed_death_stands_the_player_up() {
        let (mut session, _client) = game_session();
        let (queue, receiver) = mpsc::unbounded_channel();
        let mut outbox = Some(receiver);
        let mut pending = Vec::new();
        let mut held = Held {
            avatar: Some(sitting()),
            ..Held::default()
        };
        queue
            .send(vec![HEADER_GC_EMPIRE.value(), 1])
            .expect("the queue is open");
        assert!(flush_turn(&mut session, addr(), &mut outbox, &mut pending, &mut held).await);
        assert_eq!(held.avatar, Some(sitting()));

        queue
            .send(vec![HEADER_GC_PHASE.value(), PHASE_DEAD])
            .expect("the queue is open");
        assert!(flush_turn(&mut session, addr(), &mut outbox, &mut pending, &mut held).await);
        assert_eq!(session.phase(), ClientPhase::Dead);
        assert_eq!(
            held.avatar.as_ref().map(|avatar| avatar.sitting),
            Some(false)
        );
    }

    /// An order's turn stands a sitting player up when the records it writes change the phase,
    /// as a death the world queued before the order does (critic 8).
    #[tokio::test]
    async fn an_order_turn_that_changes_the_phase_stands_the_player_up() {
        let (mut session, _client) = game_session();
        let mut outbox = None;
        let mut held = Held {
            avatar: Some(sitting()),
            ..Held::default()
        };
        let follow = async |_: &mut LiveClientSession<DuplexStream>, _: &mut Held| true;
        let mut pending = vec![vec![HEADER_GC_PHASE.value(), PHASE_DEAD]];
        let taken = take_order(
            &mut session,
            addr(),
            &mut outbox,
            &mut pending,
            &mut held,
            follow,
        );
        assert!(taken.await);
        assert_eq!(session.phase(), ClientPhase::Dead);
        assert_eq!(
            held.avatar.as_ref().map(|avatar| avatar.sitting),
            Some(false)
        );
    }

    /// A dead caller runs its lines from `POS_DEAD`, which the restart lines need, sitting or
    /// not; a living one runs them from its pose.
    #[test]
    fn a_dead_caller_runs_its_lines_from_the_dead_position() {
        let mut held = Held {
            avatar: Some(sitting()),
            ..Held::default()
        };
        assert_eq!(caller_position(ClientPhase::Dead, &held), POS_DEAD);
        assert_eq!(caller_position(ClientPhase::Game, &held), POS_SITTING);
        held.avatar = None;
        assert_eq!(caller_position(ClientPhase::Dead, &held), POS_DEAD);
        assert_eq!(caller_position(ClientPhase::Game, &held), POS_STANDING);
    }

    /// A character row with a value in every column, as `load_character` returns one.
    fn alice() -> db::players::Character {
        db::players::Character {
            id: 1,
            slot: 0,
            name: "Alice".to_owned(),
            empire: 1,
            job: 0,
            level: 10,
            exp: 1_000,
            conqueror_level: 0,
            random_hp: 0,
            random_sp: 0,
            stat_point: 0,
            skill_point: 0,
            sub_skill_point: 0,
            level_step: 0,
            st: 6,
            ht: 4,
            dx: 3,
            iq: 3,
            hp: 500,
            sp: 100,
            stamina: 800,
            gold: 0,
            conqueror_exp: 0,
            sungma_str: 0,
            sungma_hp: 0,
            sungma_move: 0,
            sungma_immune: 0,
            voice: 0,
            part_base: 0,
            main_part: 0,
            hair_part: 0,
            sash_part: 0,
            x: 1,
            y: 2,
            map_index: 21,
            skill_group: 0,
            playtime_minutes: 0,
            alignment: 0,
            change_name: false,
            exit_map_index: 0,
            exit_x: 0,
            exit_y: 0,
        }
    }

    /// The save writes the level, experience and counters the world holds, so a level-up is
    /// kept rather than the values the character was loaded with.
    #[test]
    fn the_held_character_takes_the_experience_the_world_left() {
        let character = alice();
        let mut points = load_points(&character, 41);
        assert!(points.lose_exp(100, 120).is_some(), "the death penalty");
        let mut held = Held {
            character: Some(character),
            ..Held::default()
        };
        hold_points(&mut held, points);
        assert_eq!(held.character.as_ref().map(|held| held.exp), Some(900));
        // A level-up's level, random pools and counters move the same way: level 1 holds
        // 290 of 300, so 20 crosses the cost and banks 10.
        let mut low = load_points(
            &db::players::Character {
                level: 1,
                exp: 290,
                ..held.character.clone().expect("held")
            },
            41,
        );
        let limits = world::character::ExpLimits {
            max_level: 120,
            status_point_get_level_limit: 120,
        };
        let outcome = low.gain_exp(20, 0, limits, &mut world::character::Pcg32::new(0, 0));
        assert_eq!(outcome.level_after, 2);
        hold_points(&mut held, low);
        let kept = held.character.as_ref().expect("held");
        assert_eq!(kept.level, 2);
        assert_eq!(kept.exp, 10);
        assert!(kept.random_hp >= 36 && kept.random_hp <= 44);
        assert!(kept.random_sp >= 18 && kept.random_sp <= 22);
        assert_eq!(kept.stat_point, 3);
        assert_eq!(kept.level_step, 0);
    }

    /// The alignment the world holds is what the save writes: the row the descriptor holds
    /// takes it from the world, not from the value it was loaded with. The kills raise the
    /// world's alignment (`G/char_battle.cpp:794-807`) and the store keeps it
    /// (`G/char.cpp:1546`), so a save that wrote the loaded value would lose the kills.
    #[test]
    fn the_held_character_takes_the_alignment_the_world_left() {
        let character = db::players::Character {
            alignment: 4321,
            ..alice()
        };
        let mut held = Held {
            character: Some(character),
            ..Held::default()
        };
        hold_kept(
            &mut held,
            Kept {
                points: None,
                quickslots: world::character::Quickslots::default(),
                place: None,
                alignment: -250,
                quest_flags: Vec::new(),
                affects: Vec::new(),
            },
        );
        assert_eq!(
            held.character.as_ref().map(|held| held.alignment),
            Some(-250),
            "the row takes the alignment the world holds"
        );
    }

    /// Boot hands the world what its regens spawn from: an ore vein boot stood ends within 15
    /// minutes, and its regen stands another under the first VID boot did not use.
    #[test]
    fn boot_hands_the_regens_their_spawner_and_the_vids_it_left() {
        use gamedata::mob_proto::{MobProto, CHAR_TYPE_NPC};
        use gamedata::records::MobTableRecord;
        use gamedata::regen::RegenKind;
        use gamedata::server_attr::{CellBlock, SectreeGrid, ServerAttr, SECTREE_CELLS};
        use prodomo::game_loop::PulseProcessor;
        use world::cells::MapCells;
        use world::npc::FIRST_NPC_VID;

        let region = MapRegion {
            index: 41,
            name: b"vein".to_vec(),
            sx: 0,
            sy: 0,
            ex: 6_400,
            ey: 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        };
        let block = CellBlock::from_cells(&vec![0; SECTREE_CELLS * SECTREE_CELLS]);
        let attr = ServerAttr::from_blocks(1, 1, vec![block]).expect("one open block");
        let grid = SectreeGrid::of(&region).expect("one sectree");
        let mut hosted = HostedCells::default();
        hosted.insert(region.index, MapCells::new(grid, Arc::new(attr)));
        let vein = RegenEntry {
            kind: RegenKind::Mob,
            sx: 500,
            sy: 500,
            ex: 500,
            ey: 500,
            z_section: 0,
            direction: 1,
            time: 1,
            max_count: 1,
            vnum: 20_047,
        };
        let proto = MobProto {
            line: 2,
            table: MobTableRecord {
                vnum: 20_047,
                mob_type: CHAR_TYPE_NPC,
                max_hp: 100,
                ..MobTableRecord::default()
            },
        };
        let data = NpcData {
            protos: Arc::new(MobProtos::from_rows(vec![proto])),
            groups: Arc::new(MobGroups::default()),
            names: Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").expect("a header")),
            maps: vec![(1, region, vec![vein])],
        };
        let mut state = GameState::new(ItemProtos::default());
        stand_up_npcs(
            &mut state,
            &data,
            &Arc::new(hosted),
            &Arc::new(gamedata::mob_skill_data::MobSkillClips::default()),
        )
        .expect("boot stands the vein");
        let vids = |state: &GameState| -> Vec<u32> {
            state
                .npcs_on(1, 41)
                .npcs
                .iter()
                .map(|npc| npc.vid)
                .collect()
        };
        assert_eq!(vids(&state), [FIRST_NPC_VID]);

        // A vein lives at most 15 minutes, and its regen runs every second after that.
        for pulse in 1..=(15 * 60 + 1) * 25 {
            state.process_pulse(pulse);
        }
        assert_eq!(vids(&state), [FIRST_NPC_VID + 1]);
    }

    /// Boot hands the quests every language's `item_names.txt` (`LocaleService_LoadItemNameFile`,
    /// `G/locale_service.cpp:480-500`): 60003 names an item there and a monster in
    /// `mob_names.txt`, and Spanish names the swords its own way.
    #[test]
    fn boot_hands_the_quests_the_owners_item_names() {
        use common::enums::{ELocale, LOCALE_DEFAULT};

        const ONE_CHANNEL: &str = r#"
[store]
url = "postgres://prodomo@127.0.0.1/prodomo"
[auth]
port = 30001
[[channel]]
number = 1
ports = [30003, 30005]
maps = [1]
"#;
        let mut config = common::config::parse_server_config(ONE_CHANNEL, "test")
            .unwrap_or_else(|error| panic!("{error}"));
        config.game_data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata");
        let data = NpcData {
            protos: Arc::new(MobProtos::from_rows(Vec::new())),
            groups: Arc::new(MobGroups::default()),
            names: Arc::new(MobLocaleNames::parse(b"VNUM\tNAME\n").expect("a header")),
            maps: Vec::new(),
        };
        let quests = load_quests(&config, &data).unwrap_or_else(|error| panic!("{error}"));
        let names = quests.item_names();
        let english = LOCALE_DEFAULT as u8;
        assert_eq!(names.find(60_003, english), b"Simbol Eroic");
        assert_eq!(names.find(10, english), b"Sabie+0");
        assert_eq!(names.find(10, ELocale::Es as u8), b"Espada+0");
    }
}
