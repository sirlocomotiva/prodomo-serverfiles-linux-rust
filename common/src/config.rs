//! The `prodomo.toml` configuration.
//!
//! One document configures the whole process (ADR-0002): the addresses it binds and announces, the
//! PostgreSQL store, the auth listener, one `[[channel]]` table per Channel, and the `[game]`
//! settings every Channel shares. The legacy `CONFIG` and `conf.txt` files are not read; ledger
//! sections 169 and 178 map their keys to this document.
//!
//! Unknown keys are rejected at every level, so a misspelled setting fails at startup instead of
//! silently keeping its default. A document that parses but describes an impossible topology,
//! such as two listeners on one port, is rejected by [`ServerConfig::validate`].

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

/// The Shared Channel's number: legacy `GUILD_WARP_WAR_CHANNEL` (`db/GuildManager.h:13`).
pub const SHARED_CHANNEL: u8 = 99;

/// Default path of the configuration document.
pub const DEFAULT_CONFIG_PATH: &str = "prodomo.toml";

/// A string that is never printed by `Debug`.
///
/// In TOML it is a plain string. Only [`Secret::expose`] returns the value.
#[derive(Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Wrap a value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("\"\"")
        } else {
            f.write_str("\"***\"")
        }
    }
}

/// Replace the password and query of a connection URL with `***`.
///
/// Only the scheme, the user name, the host, and the path survive, so the result can be logged.
/// A value without `://` is not a URL this project reads and is replaced entirely.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "***".to_owned();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => match userinfo.split_once(':') {
            Some((user, _password)) => format!("{user}:***@{host}"),
            None => format!("{userinfo}@{host}"),
        },
        None => authority.to_owned(),
    };
    // A libpq-style URL may carry `?password=...`, so the whole query goes.
    let tail = match tail.find(['?', '#']) {
        Some(index) => format!("{}?***", &tail[..index]),
        None => tail.to_owned(),
    };
    format!("{scheme}://{authority}{tail}")
}

/// The whole process configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Local address every listener binds. Legacy `bind_ip`, default `0.0.0.0`.
    #[serde(default = "default_bind_ip")]
    pub bind_ip: IpAddr,
    /// IPv4 address the Reference client is told to reconnect to on a Warp and in the
    /// character list. Legacy sent `proxy_ip` when set and `bind_ip` otherwise; this one key
    /// replaces both. The client field is four bytes, so IPv6 is not accepted.
    #[serde(default = "default_public_ip")]
    pub public_ip: Ipv4Addr,
    /// The legacy Game data folder (the `share` tree: `locale/europe/map`, the protos, the
    /// quests), read in place and never written. A relative path is taken from the working
    /// directory. Default `legacy/gamedata`.
    #[serde(default = "default_game_data")]
    pub game_data: PathBuf,
    /// The owner's `mysqldump` files of the Game data tables (`player.sql` with `banword`,
    /// `common.sql`), read in place and never written. A relative path is taken from the working
    /// directory. Default `legacy/sql/gamedata`.
    #[serde(default = "default_game_tables")]
    pub game_tables: PathBuf,
    /// The PostgreSQL store.
    pub store: StoreSettings,
    /// The auth listener.
    pub auth: AuthSettings,
    /// One entry per Channel, written as `[[channel]]` tables. An empty list parses and then
    /// fails [`ServerConfig::validate`].
    #[serde(rename = "channel", default)]
    pub channels: Vec<ChannelSettings>,
    /// Gameplay settings shared by every Channel.
    #[serde(default)]
    pub game: GameSettings,
}

/// The owner's `ITEM_ID_RANGE` (`legacy/config/db/conf.txt:8`).
///
/// Not a value legacy compiled in -- see [`ItemIdSpan`]. A deployment that needs a
/// different id space sets the key; one that does not gets the owner's span, which
/// is the span the live deployment used.
const fn default_item_id_span() -> ItemIdSpan {
    ItemIdSpan {
        first: 100_000_000,
        last: 200_000_000,
    }
}

/// Reads `item_id_range = [first, last]`.
///
/// A two-element array rather than a sub-table because legacy's `GetTwoValue` read
/// two whitespace-separated numbers off one line, and the array is the same shape.
fn deserialize_item_id_span<'de, D>(deserializer: D) -> Result<ItemIdSpan, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let pair: [u32; 2] = serde::Deserialize::deserialize(deserializer)?;
    Ok(ItemIdSpan {
        first: pair[0],
        last: pair[1],
    })
}

const fn default_bind_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

const fn default_public_ip() -> Ipv4Addr {
    Ipv4Addr::LOCALHOST
}

fn default_game_data() -> PathBuf {
    PathBuf::from(DEFAULT_GAME_DATA)
}

/// The default [`ServerConfig::game_data`].
pub const DEFAULT_GAME_DATA: &str = "legacy/gamedata";

fn default_game_tables() -> PathBuf {
    PathBuf::from(DEFAULT_GAME_TABLES)
}

/// The default [`ServerConfig::game_tables`].
pub const DEFAULT_GAME_TABLES: &str = "legacy/sql/gamedata";

/// The Locale folder under [`ServerConfig::game_data`]; only `europe` is ported.
pub const LOCALE_DIR: &str = "locale/europe";

impl ServerConfig {
    /// The map folder: `<game_data>/locale/europe/map`, holding the `index` file and one folder
    /// per map.
    #[must_use]
    pub fn map_dir(&self) -> PathBuf {
        self.game_data.join(LOCALE_DIR).join("map")
    }

    /// The language folder: `<game_data>/locale/europe/country`, holding one folder per
    /// language, each with its `locale_string.txt`.
    #[must_use]
    pub fn country_dir(&self) -> PathBuf {
        self.game_data.join(LOCALE_DIR).join("country")
    }

    /// The text proto folder: `<game_data>/proto` (`PROTO_FROM_DB = 0`).
    #[must_use]
    pub fn proto_dir(&self) -> PathBuf {
        self.game_data.join("proto")
    }
}

/// The `[store]` table.
#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreSettings {
    /// A `postgres://` connection URL. Its password is never logged.
    pub url: String,
    /// Upper bound on open connections.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

const fn default_max_connections() -> u32 {
    8
}

impl fmt::Debug for StoreSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreSettings")
            .field("url", &redact_url(&self.url))
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// The `[auth]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSettings {
    /// Port of the auth listener. `0` lets the operating system choose one, which is logged.
    pub port: u16,
}

/// One `[[channel]]` table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSettings {
    /// The Channel number, `1..=99`. [`SHARED_CHANNEL`] is the Shared Channel.
    pub number: u8,
    /// Ports that all lead into this Channel. A legacy Channel had one port per Core, and every
    /// one of them is kept so that a client list naming any of them still works. `0` lets the
    /// operating system choose, which is logged.
    pub ports: Vec<u16>,
    /// Map indices this Channel hosts: the union of its legacy Cores' `MAP_ALLOW` lists.
    pub maps: Vec<u32>,
}

impl ChannelSettings {
    /// Whether this is the Shared Channel.
    #[must_use]
    pub const fn is_shared(&self) -> bool {
        self.number == SHARED_CHANNEL
    }
}

/// Gameplay settings shared by every Channel: the `[game]` table.
///
/// Each key keeps the TOML name ledger section 169.5 gave it, and its default is the legacy
/// compiled-in value from `game/config.cpp`, except where a field says otherwise. The keys that
/// described the legacy process layout, SQL connections, and logging are gone; ledger section
/// 178 lists them. No field is read by gameplay code yet.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GameSettings {
    /// Admin page source addresses.
    pub adminpage_ips: Vec<String>,
    /// Admin page password. Legacy defaults to `SHOWMETHEMONEY`; here the default is empty,
    /// which disables the admin page.
    pub adminpage_password: Secret,
    /// The item-id span the world's allocator hands ids out of, as `first` and `last`.
    ///
    /// Legacy read this as `ITEM_ID_RANGE` in the DB process. It is a `[game]` key
    /// here because the Rewrite has one process (ADR-0002), so the id space belongs
    /// to the world and not to a database peer.
    #[serde(
        default = "default_item_id_span",
        deserialize_with = "deserialize_item_id_span"
    )]
    pub item_id_range: ItemIdSpan,

    /// The path of the named pipe the Operator console reads, or `None` to leave the
    /// console off.
    ///
    /// The console is how an Operator reaches a live world, and it has to run inside
    /// `serve`: the world exists only as that process's memory (ADR-0002), so a
    /// separate `prodomo` subcommand would need a channel between two processes, which
    /// is the class of protocol ADR-0001 retired.
    ///
    /// It is **off unless this is set**. A console that is on by default is a console
    /// that is listening before anyone has decided an Operator should be able to, and
    /// its access control is the pipe's own mode, which is only meaningful once someone
    /// chose the path.
    #[serde(default)]
    pub operator_console: Option<PathBuf>,

    /// Seconds between character saves. Legacy multiplied the configured value by
    /// `passes_per_sec` when parsing; here the value stays in seconds.
    pub save_event_second_cycle: u32,
    /// Seconds between pings. Stored in seconds, like `save_event_second_cycle`.
    pub ping_event_second_cycle: u32,
    /// Test-server mode. Legacy defaults to on (`config.cpp:72`).
    pub test_server: bool,
    /// Whether guild marks are served.
    pub guild_mark_server: bool,
    /// Minimum guild level for a guild mark.
    pub guild_mark_min_level: u8,
    /// Monsters do not wander.
    pub no_wander: bool,
    /// Player limit; `0` or less is unlimited.
    pub user_limit: i32,
    /// Above this many characters in game, the Channel status list shows every Channel as busy
    /// (status 2). Legacy `g_iBusyUserCount` (`config.cpp:82`), read from the `state_user_count`
    /// file, which was empty in the owner's deployment.
    pub busy_user_count: u32,
    /// Above this many characters in game, the Channel status list shows every Channel as full
    /// (status 3). Legacy `g_iFullUserCount` (`config.cpp:81`).
    pub full_user_count: u32,
    /// Whispers between empires are allowed.
    pub empire_whisper: bool,
    /// Accounts created on or after this `YYYYMMDD` date are refused. Legacy `BLOCK_LOGIN`,
    /// compared with `strncmp` over eight bytes, so an empty value refuses every account.
    pub block_login: String,
    /// New logins are refused.
    pub shutdowned: bool,
    /// Monster regeneration is off.
    pub no_regen: bool,
    /// Shutdown age.
    pub shutdown_age: i32,
    /// Shutdown enable.
    pub shutdown_enable: i32,
    /// Empire shop prices are not tripled.
    pub disable_shop_price_3x: bool,
    /// Shout addon enable.
    pub enable_shout_addon: bool,
    /// Shouts reach every empire.
    pub enable_global_shout: bool,
    /// Prisms are not required.
    pub disable_prism_need: bool,
    /// The emotion mask is not required.
    pub disable_emotion_mask: bool,
    /// Largest stack count.
    pub item_count_limit: u16,
    /// Seconds between item bonus changes.
    pub item_bonus_change_time: u32,
    /// Attacks are allowed from every mount.
    pub enable_all_mount_attack: bool,
    /// Enable bootary check.
    pub enable_bootary_check: bool,
    /// GM accounts only work from their listed hosts.
    pub gm_host_check: bool,
    /// Guild invite limit.
    pub guild_invite_limit: bool,
    /// Guilds have no member limit.
    pub guild_infinite_members: bool,
    /// China intoxication check.
    pub china_intoxication_check: bool,
    /// Speed hackers are disconnected.
    pub enable_speedhack_crash: bool,
    /// Level up to which status points are granted.
    pub status_point_get_level_limit: i32,
    /// Largest value of one status point.
    pub status_point_set_max_value: i32,
    /// Minimum level to shout.
    pub shout_limit_level: i32,
    /// Item log level.
    pub db_log_level: i32,
    /// Seconds before an automatically given item is destroyed.
    pub item_destroy_time_autogive: i32,
    /// Seconds before dropped gold is destroyed.
    pub item_destroy_time_dropgold: i32,
    /// Seconds before a dropped item is destroyed.
    pub item_destroy_time_dropitem: i32,
    /// Chat between empires is not scrambled.
    pub disable_empire_language_check: bool,
    /// Minimum seconds between skill book reads.
    pub skillbook_nextread_min: u32,
    /// Maximum seconds between skill book reads.
    pub skillbook_nextread_max: u32,
    /// Check the client version.
    pub check_version_server: bool,
    /// Expected client version.
    pub check_version_value: String,
    /// Enable hack check.
    pub enable_hack_check: bool,
    /// Check multi hack.
    pub check_multihack: bool,
    /// Speed hack limit count.
    pub speedhack_limit_count: i32,
    /// Speed hack limit bonus.
    pub speedhack_limit_bonus: i32,
    /// Sync hack limit count.
    pub synchack_limit_count: i32,
    /// Server ID sent to the item mall.
    pub server_id: i32,
    /// Item mall URL.
    pub mall_url: String,
    /// View range.
    pub view_range: i32,
    /// Spam block duration in seconds.
    pub spam_block_duration: u32,
    /// Spam block score.
    pub spam_block_score: u32,
    /// Spam block reload cycle in seconds.
    pub spam_block_reload_cycle: u32,
    /// Highest level spam blocking applies to.
    pub spam_block_max_level: i32,
    /// `VRAJESTE_PERMANENT`.
    pub vrajeste_permanent: bool,
    /// `INTARIRE_PERMANENT`.
    pub intarire_permanent: bool,
    /// `MANTIE_PERMANENT`.
    pub mantie_permanent: bool,
    /// Protect normal player.
    pub protect_normal_player: bool,
    /// Notice battle zone.
    pub notice_battle_zone: bool,
    /// PK protect level.
    pub pk_protect_level: i32,
    /// Player max level.
    pub max_level: i32,
    /// Player max conqueror level.
    pub max_conqueror_level: i32,
    /// Character creation is blocked.
    pub block_char_creation: bool,
    /// A character at this level or above cannot be deleted (legacy DB `conf.txt`
    /// `PLAYER_DELETE_LEVEL_LIMIT`).
    pub player_delete_level_limit: i32,
    /// A character below this level cannot be deleted (`PLAYER_DELETE_LEVEL_LIMIT_LOWER`).
    pub player_delete_level_limit_lower: i32,
    /// Skills are disabled.
    pub skill_disable: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            adminpage_ips: Vec::new(),
            adminpage_password: Secret::default(),
            operator_console: None,
            save_event_second_cycle: 120,
            ping_event_second_cycle: 60,
            test_server: true,
            guild_mark_server: true,
            guild_mark_min_level: 3,
            no_wander: false,
            user_limit: 32768,
            busy_user_count: 650,
            full_user_count: 1200,
            empire_whisper: true,
            block_login: "30000705".to_owned(),
            shutdowned: false,
            no_regen: false,
            shutdown_age: 0,
            shutdown_enable: 0,
            disable_shop_price_3x: false,
            enable_shout_addon: false,
            enable_global_shout: false,
            disable_prism_need: false,
            disable_emotion_mask: false,
            item_count_limit: 5000,
            item_bonus_change_time: 60,
            enable_all_mount_attack: false,
            enable_bootary_check: false,
            gm_host_check: false,
            guild_invite_limit: false,
            guild_infinite_members: false,
            china_intoxication_check: false,
            enable_speedhack_crash: false,
            status_point_get_level_limit: 90,
            status_point_set_max_value: 90,
            shout_limit_level: 15,
            db_log_level: 5, // LOG_LEVEL_MAX
            item_destroy_time_autogive: 300,
            item_destroy_time_dropgold: 150,
            item_destroy_time_dropitem: 300,
            disable_empire_language_check: false,
            skillbook_nextread_min: 28800,
            skillbook_nextread_max: 43200,
            check_version_server: true,
            check_version_value: "1215955205".to_owned(),
            enable_hack_check: false,
            check_multihack: true,
            speedhack_limit_count: 50,
            speedhack_limit_bonus: 80,
            synchack_limit_count: 10,
            server_id: 0,
            mall_url: "www.metin2galaxy.ro".to_owned(),
            view_range: 5000,
            spam_block_duration: 60 * 15,
            spam_block_score: 100,
            spam_block_reload_cycle: 60 * 10,
            spam_block_max_level: 10,
            vrajeste_permanent: false,
            intarire_permanent: false,
            mantie_permanent: false,
            protect_normal_player: false,
            notice_battle_zone: false,
            pk_protect_level: 0,
            max_level: 99,
            max_conqueror_level: 30,
            block_char_creation: false,
            // `PLAYER_MAX_LEVEL_CONST + 1` (`D/ClientManager.cpp:297`).
            player_delete_level_limit: 251,
            player_delete_level_limit_lower: 0,
            skill_disable: false,
            item_id_range: default_item_id_span(),
        }
    }
}

/// The item-id span the allocator draws from, as legacy's `ITEM_ID_RANGE` pair.
///
/// Legacy read this with `GetTwoValue("ITEM_ID_RANGE", &dwMin, &dwMax)` in the **DB**
/// process (`ClientManager.cpp:3394`) and fed it straight to
/// `CItemIDRangeManager::BuildRange`. The owner's snapshot has exactly one value,
/// `100000000 200000000` (`legacy/config/db/conf.txt:8`), and it is the only
/// `conf.txt` of the five that carries the key. There is no compiled-in default:
/// `InitializeNowItemID` returns false when the key is missing, which aborts the
/// boot, so the default here is the owner's value and not a value legacy had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemIdSpan {
    /// `dwMin`. The first id in the span.
    pub first: u32,
    /// `dwMax`. The end of the span.
    ///
    /// Legacy never hands this one out: the range is switched "once the next ID
    /// reaches this value", and `BuildRange` refuses a span whose remaining count is
    /// below `MINIMUM_REMAIN_COUNT`.
    pub last: u32,
}

impl ItemIdSpan {
    /// Whether `first` is below `last`, which is the only shape a span can take.
    #[must_use]
    pub const fn is_ordered(self) -> bool {
        self.first < self.last
    }

    /// How many ids are left above `next`.
    ///
    /// Zero when `next` is at or past `last`. That is the whole point of writing it
    /// this way: a plain subtraction would wrap and report billions of ids left in a
    /// span that has none, and the store would go on handing out ids tens of millions
    /// past its configured end.
    #[must_use]
    pub const fn remaining_from(self, next: u32) -> u32 {
        self.last.saturating_sub(next)
    }
}

impl fmt::Display for ItemIdSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..={}", self.first, self.last)
    }
}

/// A topology the process cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    /// A Channel number is outside `1..=99`.
    ChannelNumber(u8),
    /// Two `[[channel]]` tables share a number.
    DuplicateChannel(u8),
    /// No Channel other than the Shared Channel is configured, so none can be picked at login.
    NoLoginChannel,
    /// A Channel has no port.
    NoPorts(u8),
    /// A Channel hosts no map.
    NoMaps(u8),
    /// A Channel lists map index 0.
    ZeroMap(u8),
    /// A Channel lists the same map twice.
    DuplicateMap {
        /// The Channel.
        channel: u8,
        /// The repeated map index.
        map: u32,
    },
    /// Two listeners share a nonzero port.
    DuplicatePort(u16),
    /// A `[game]` event cycle is zero seconds, so its event would never wait.
    ZeroCycle(&'static str),
    /// `game.item_id_range` is not `first..last`.
    ///
    /// Checked at startup rather than at first use because a range that only fails
    /// when the first item is granted is a range that has already admitted a client.
    UnorderedItemIdRange {
        /// The configured first id.
        first: u32,
        /// The configured last id.
        last: u32,
    },
    /// `game.item_count_limit` is 0, or above the largest stack the store and the client's
    /// stack count hold ([`crate::item_slots::ITEM_COUNT_LIMIT`]).
    ///
    /// Legacy reads any `WORD` (`config.cpp:976`). A limit of 0 makes every stack merge
    /// move nothing, and one above 5000 builds a stack the store refuses to write, so a
    /// merge would succeed on the client and fail at the save.
    ItemCountLimit(u16),
    /// A Shared Channel map is also hosted by another Channel, so a Warp to it is ambiguous.
    SharedMapElsewhere {
        /// The map index.
        map: u32,
        /// The other Channel hosting it.
        channel: u8,
    },
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChannelNumber(number) => {
                write!(f, "channel number {number} is outside 1..={SHARED_CHANNEL}")
            }
            Self::DuplicateChannel(number) => write!(f, "channel {number} is configured twice"),
            Self::NoLoginChannel => write!(
                f,
                "no channel other than the shared channel {SHARED_CHANNEL}, so none can be \
                 picked at login"
            ),
            Self::NoPorts(number) => write!(f, "channel {number} has no port"),
            Self::NoMaps(number) => write!(f, "channel {number} hosts no map"),
            Self::ZeroMap(number) => write!(f, "channel {number} lists map index 0"),
            Self::DuplicateMap { channel, map } => {
                write!(f, "channel {channel} lists map {map} twice")
            }
            Self::DuplicatePort(port) => write!(f, "port {port} is used by two listeners"),
            Self::ZeroCycle(key) => write!(f, "game.{key} must be at least 1 second"),
            Self::UnorderedItemIdRange { first, last } => write!(
                f,
                "game.item_id_range is [{first}, {last}]; the first id must be below the last"
            ),
            Self::ItemCountLimit(limit) => write!(
                f,
                "game.item_count_limit is {limit}; it must be 1 to {}",
                crate::item_slots::ITEM_COUNT_LIMIT
            ),
            Self::SharedMapElsewhere { map, channel } => write!(
                f,
                "map {map} is on the shared channel and also on channel {channel}"
            ),
        }
    }
}

impl std::error::Error for TopologyError {}

impl ServerConfig {
    /// Check the topology.
    ///
    /// # Errors
    ///
    /// Returns the first [`TopologyError`] found.
    pub fn validate(&self) -> Result<(), TopologyError> {
        if self.game.save_event_second_cycle == 0 {
            return Err(TopologyError::ZeroCycle("save_event_second_cycle"));
        }
        if self.game.ping_event_second_cycle == 0 {
            return Err(TopologyError::ZeroCycle("ping_event_second_cycle"));
        }
        let span = self.game.item_id_range;
        if !span.is_ordered() {
            return Err(TopologyError::UnorderedItemIdRange {
                first: span.first,
                last: span.last,
            });
        }
        let limit = self.game.item_count_limit;
        if limit == 0 || limit > crate::item_slots::ITEM_COUNT_LIMIT {
            return Err(TopologyError::ItemCountLimit(limit));
        }
        let mut numbers = BTreeSet::new();
        let mut ports = BTreeSet::new();
        claim_port(&mut ports, self.auth.port)?;
        for channel in &self.channels {
            if !(1..=SHARED_CHANNEL).contains(&channel.number) {
                return Err(TopologyError::ChannelNumber(channel.number));
            }
            if !numbers.insert(channel.number) {
                return Err(TopologyError::DuplicateChannel(channel.number));
            }
            if channel.ports.is_empty() {
                return Err(TopologyError::NoPorts(channel.number));
            }
            for &port in &channel.ports {
                claim_port(&mut ports, port)?;
            }
            if channel.maps.is_empty() {
                return Err(TopologyError::NoMaps(channel.number));
            }
            let mut maps = BTreeSet::new();
            for &map in &channel.maps {
                if map == 0 {
                    return Err(TopologyError::ZeroMap(channel.number));
                }
                if !maps.insert(map) {
                    return Err(TopologyError::DuplicateMap {
                        channel: channel.number,
                        map,
                    });
                }
            }
        }
        if !self.channels.iter().any(|channel| !channel.is_shared()) {
            return Err(TopologyError::NoLoginChannel);
        }
        self.check_shared_maps()
    }

    fn check_shared_maps(&self) -> Result<(), TopologyError> {
        let shared: BTreeSet<u32> = self
            .channels
            .iter()
            .filter(|channel| channel.is_shared())
            .flat_map(|channel| channel.maps.iter().copied())
            .collect();
        for channel in self.channels.iter().filter(|channel| !channel.is_shared()) {
            if let Some(&map) = channel.maps.iter().find(|map| shared.contains(map)) {
                return Err(TopologyError::SharedMapElsewhere {
                    map,
                    channel: channel.number,
                });
            }
        }
        Ok(())
    }
}

fn claim_port(ports: &mut BTreeSet<u16>, port: u16) -> Result<(), TopologyError> {
    // Port 0 asks the operating system for a free port, so several zeros never collide.
    if port != 0 && !ports.insert(port) {
        return Err(TopologyError::DuplicatePort(port));
    }
    Ok(())
}

/// Errors produced while loading a configuration document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The configuration file could not be read.
    Read {
        /// Path that was attempted.
        path: String,
        /// Underlying I/O error text.
        source: String,
    },
    /// The file was not valid UTF-8 or not a well-formed TOML document.
    Syntax {
        /// Path that was attempted.
        path: String,
        /// The parser's message, including line and column when available.
        message: String,
    },
    /// A key had the wrong type, a required key was missing, or an unknown key was present.
    Value {
        /// Path that was attempted.
        path: String,
        /// The deserializer's message.
        message: String,
    },
    /// The document parsed but describes a topology the process cannot run.
    Invalid {
        /// Path that was attempted.
        path: String,
        /// What is wrong.
        error: TopologyError,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "failed to read config file {path}: {source}"),
            Self::Syntax { path, message } => {
                write!(f, "invalid TOML in config file {path}: {message}")
            }
            Self::Value { path, message } => {
                write!(f, "invalid value in config file {path}: {message}")
            }
            Self::Invalid { path, error } => write!(f, "invalid topology in {path}: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Load and validate the configuration from a TOML file.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when the file cannot be read, [`ConfigError::Syntax`] when it is
/// not a well-formed TOML document, [`ConfigError::Value`] when a key has the wrong type, a
/// required key is missing, or an unknown key is present, and [`ConfigError::Invalid`] when the
/// topology fails [`ServerConfig::validate`].
pub fn load_server_config<P: AsRef<Path>>(path: P) -> Result<ServerConfig, ConfigError> {
    let path = path.as_ref();
    let shown = path.display().to_string();
    let content = fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: shown.clone(),
        source: e.to_string(),
    })?;
    parse_server_config(&content, &shown)
}

/// Parse and validate a configuration document already in memory.
///
/// `path` is only used in error messages.
///
/// # Errors
///
/// Returns the [`ConfigError`] variants of [`load_server_config`] other than `Read`.
pub fn parse_server_config(content: &str, path: &str) -> Result<ServerConfig, ConfigError> {
    // Parse into a table first so that malformed TOML is reported separately from a bad value.
    let table: toml::Table = toml::from_str(content).map_err(|e| ConfigError::Syntax {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let config: ServerConfig =
        table
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Value {
                path: path.to_owned(),
                message: e.to_string(),
            })?;
    config.validate().map_err(|error| ConfigError::Invalid {
        path: path.to_owned(),
        error,
    })?;
    Ok(config)
}
