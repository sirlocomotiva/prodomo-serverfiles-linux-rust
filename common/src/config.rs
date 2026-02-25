//! TOML configuration for the Metin2 game and database servers.
//!
//! Both binaries are configured by a TOML document. The legacy C++ `CONFIG` and `conf.txt`
//! `key=value` files are **not** read; see `docs/REWRITE_LEDGER.md` for the legacy-key to
//! TOML-key mapping an operator can follow to convert an old file by hand.
//!
//! Scalar settings are top-level keys that keep their Rust field names. The three SQL
//! connections of each binary are sub-tables so that a credential is never a space-separated
//! string. Unknown keys are rejected so that a typo fails at startup instead of silently
//! selecting a default.

use std::fmt;
use std::fs;
use std::path::Path;

/// SQL connection configuration.
///
/// In TOML this is a sub-table such as `[player_sql]`, not a space-separated value.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SqlConfig {
    /// Database host address
    pub host: String,
    /// Database user
    pub user: String,
    /// Database password
    pub password: String,
    /// Database name
    pub database: String,
    /// Database port (0 = default)
    pub port: u16,
}

/// Game server configuration, deserialized from a TOML document.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GameConfig {
    // Connection settings
    /// Server hostname
    pub hostname: String,
    /// Channel number
    pub channel: u8,
    /// Mother port (client listen port)
    pub mother_port: u16,
    /// P2P port
    pub p2p_port: u16,
    /// DB server port
    pub db_port: u16,
    /// DB server address
    pub db_addr: String,
    /// Bind IP address
    pub bind_ip: String,

    // SQL connections
    /// Player SQL connection
    pub player_sql: SqlConfig,
    /// Common SQL connection
    pub common_sql: SqlConfig,
    /// Log SQL connection
    pub log_sql: SqlConfig,

    // Map settings
    /// Allowed map indices
    pub map_allow: Vec<i32>,

    // Auth server settings
    /// Whether this is an auth server
    pub auth_server: bool,
    /// Auth master IP (if slave)
    pub auth_master_ip: String,
    /// Auth master port
    pub auth_master_port: u16,

    // Admin page settings
    /// Admin page IP addresses
    pub adminpage_ips: Vec<String>,
    /// Admin page password
    pub adminpage_password: String,

    // Game settings
    /// Passes per second
    pub passes_per_sec: i32,
    /// Save event cycle (in seconds, multiplied by `passes_per_sec`)
    pub save_event_second_cycle: i32,
    /// Ping event cycle (in seconds, multiplied by `passes_per_sec`)
    pub ping_event_second_cycle: i32,
    /// Test server flag
    pub test_server: bool,
    /// Guild mark server
    pub guild_mark_server: bool,
    /// Guild mark minimum level
    pub guild_mark_min_level: u8,
    /// No wander flag
    pub no_wander: bool,
    /// User limit
    pub user_limit: i32,
    /// Empire whisper enabled
    pub empire_whisper: bool,
    /// Table postfix
    pub table_postfix: String,
    /// Block login date
    pub block_login: String,
    /// Log keep days
    pub log_keep_days: i32,

    // Shutdown settings
    /// No more clients (shutdowned)
    pub shutdowned: bool,
    /// No regen flag
    pub no_regen: bool,
    /// Shutdown age
    pub shutdown_age: i32,
    /// Shutdown enable
    pub shutdown_enable: i32,

    // Feature flags (ENABLE_NEWSTUFF)
    /// Empire shop price triple disable
    pub disable_shop_price_3x: bool,
    /// Shout addon enable
    pub enable_shout_addon: bool,
    /// Global shout enable
    pub enable_global_shout: bool,
    /// Disable prism need
    pub disable_prism_need: bool,
    /// Disable emotion mask
    pub disable_emotion_mask: bool,
    /// Item count limit
    pub item_count_limit: u16,
    /// Item bonus change time
    pub item_bonus_change_time: u32,
    /// All mount attack
    pub enable_all_mount_attack: bool,
    /// Enable bootary check
    pub enable_bootary_check: bool,
    /// GM host check
    pub gm_host_check: bool,
    /// Guild invite limit
    pub guild_invite_limit: bool,
    /// Guild infinite members
    pub guild_infinite_members: bool,
    /// China intoxication check
    pub china_intoxication_check: bool,
    /// Enable speed hack crash
    pub enable_speedhack_crash: bool,
    /// Status point get level limit
    pub status_point_get_level_limit: i32,
    /// Status point set max value
    pub status_point_set_max_value: i32,
    /// Shout limit level
    pub shout_limit_level: i32,
    /// DB log level
    pub db_log_level: i32,
    /// System log level
    pub sys_log_level: i32,
    /// Item destroy time - autogive
    pub item_destroy_time_autogive: i32,
    /// Item destroy time - drop gold
    pub item_destroy_time_dropgold: i32,
    /// Item destroy time - drop item
    pub item_destroy_time_dropitem: i32,
    /// Disable empire language check
    pub disable_empire_language_check: bool,
    /// Skill book next read min (seconds)
    pub skillbook_nextread_min: u32,
    /// Skill book next read max (seconds)
    pub skillbook_nextread_max: u32,
    /// Proxy IP
    pub proxy_ip: String,

    // Traffic profiler
    /// Traffic profile enabled
    pub traffic_profile: bool,

    // Version check
    /// Check client version
    pub check_version_server: bool,
    /// Client version value
    pub check_version_value: String,

    // Hack check
    /// Enable hack check
    pub enable_hack_check: bool,
    /// Check multi hack
    pub check_multihack: bool,

    // Quest settings
    /// Quest directory
    pub quest_dir: String,
    /// Quest object directories
    pub quest_object_dirs: Vec<String>,

    // Speed hack settings
    /// Speedhack limit count
    pub speedhack_limit_count: i32,
    /// Speedhack limit bonus
    pub speedhack_limit_bonus: i32,
    /// Sync hack limit count
    pub synchack_limit_count: i32,

    // Server settings
    /// Server ID
    pub server_id: i32,
    /// Web mall URL
    pub mall_url: String,
    /// View range
    pub view_range: i32,

    // Spam block settings
    /// Spam block duration (seconds)
    pub spam_block_duration: u32,
    /// Spam block score
    pub spam_block_score: u32,
    /// Spam block reload cycle (seconds)
    pub spam_block_reload_cycle: u32,
    /// Spam block max level
    pub spam_block_max_level: i32,

    // Permanent flags (custom)
    /// VRAJESTE permanent
    pub vrajeste_permanent: bool,
    /// INTARIRE permanent
    pub intarire_permanent: bool,
    /// MANTIE permanent
    pub mantie_permanent: bool,

    // Player protection
    /// Protect normal player
    pub protect_normal_player: bool,
    /// Notice battle zone
    pub notice_battle_zone: bool,

    // PK settings
    /// PK protect level
    pub pk_protect_level: i32,

    // Level settings
    /// Player max level
    pub max_level: i32,
    /// Player max conqueror level
    pub max_conqueror_level: i32,

    // Character creation
    /// Block character creation
    pub block_char_creation: bool,

    /// Skill disable
    pub skill_disable: bool,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            channel: 0,
            mother_port: 50080,
            p2p_port: 50900,
            db_port: 0,
            db_addr: String::new(),
            bind_ip: String::new(),
            player_sql: SqlConfig::default(),
            common_sql: SqlConfig::default(),
            log_sql: SqlConfig::default(),
            map_allow: Vec::new(),
            auth_server: false,
            auth_master_ip: String::new(),
            auth_master_port: 0,
            adminpage_ips: Vec::new(),
            adminpage_password: "SHOWMETHEMONEY".to_string(),
            passes_per_sec: 25,
            save_event_second_cycle: 25 * 120,
            ping_event_second_cycle: 25 * 60,
            test_server: true,
            guild_mark_server: true,
            guild_mark_min_level: 3,
            no_wander: false,
            user_limit: 32768,
            empire_whisper: true,
            table_postfix: String::new(),
            block_login: String::new(),
            log_keep_days: 0,
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
            db_log_level: 5,  // LOG_LEVEL_MAX
            sys_log_level: 5, // LOG_LEVEL_MAX
            item_destroy_time_autogive: 300,
            item_destroy_time_dropgold: 150,
            item_destroy_time_dropitem: 300,
            disable_empire_language_check: false,
            skillbook_nextread_min: 28800,
            skillbook_nextread_max: 43200,
            proxy_ip: String::new(),
            traffic_profile: false,
            check_version_server: true,
            check_version_value: "1215955205".to_string(),
            enable_hack_check: false,
            check_multihack: true,
            quest_dir: "./quest".to_string(),
            quest_object_dirs: vec!["./quest/object".to_string()],
            speedhack_limit_count: 50,
            speedhack_limit_bonus: 80,
            synchack_limit_count: 10,
            server_id: 0,
            mall_url: "www.metin2galaxy.ro".to_string(),
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
            skill_disable: false,
        }
    }
}

/// Database server configuration, deserialized from a TOML document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DbConfig {
    /// Test server flag
    pub test_server: bool,
    /// Log enabled
    pub log: bool,
    /// Client heart beat FPS
    pub client_heart_fps: i32,
    /// Log keep days
    pub log_keep_days: i32,
    /// Locale
    pub locale: String,
    /// Table postfix
    pub table_postfix: String,
    /// Player cache flush seconds
    pub player_cache_flush_seconds: i32,
    /// Item cache flush seconds
    pub item_cache_flush_seconds: i32,
    /// Item price list cache flush seconds
    pub item_pricelist_cache_flush_seconds: i32,
    /// Cache flush limit per second
    pub cache_flush_limit_per_second: u32,
    /// Player ID start
    pub player_id_start: i32,
    /// Name column
    pub name_column: String,
    /// TCP bind port for game server connections
    pub bind_port: u16,
    /// TCP bind IP address (0.0.0.0 for all interfaces)
    pub bind_ip: String,
    /// Player SQL connection
    pub sql_player: SqlConfig,
    /// Account SQL connection
    pub sql_account: SqlConfig,
    /// Common SQL connection
    pub sql_common: SqlConfig,
    /// Source addresses allowed to connect as an ordinary game peer.
    ///
    /// This is transport identity, not authentication. See
    /// `db-server::peer_auth`. Empty, the default, denies every peer.
    pub trusted_game_peers: Vec<String>,
    /// Source addresses allowed to connect as the auth server peer.
    ///
    /// This is transport identity, not authentication. Empty, the default,
    /// means no peer can ever claim the auth role.
    pub trusted_auth_peers: Vec<String>,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            test_server: false,
            log: true,
            client_heart_fps: 50,
            log_keep_days: 3,
            locale: "latin1".to_string(),
            table_postfix: String::new(),
            player_cache_flush_seconds: 60 * 7,
            item_cache_flush_seconds: 60 * 5,
            item_pricelist_cache_flush_seconds: 540,
            cache_flush_limit_per_second: 0,
            player_id_start: 0,
            name_column: "name".to_string(),
            bind_port: 5300,
            bind_ip: "0.0.0.0".to_string(),
            sql_player: SqlConfig::default(),
            sql_account: SqlConfig::default(),
            sql_common: SqlConfig::default(),
            // Empty is the fail-closed default: no configured address means
            // every peer stays unauthenticated and is denied.
            trusted_game_peers: Vec::new(),
            trusted_auth_peers: Vec::new(),
        }
    }
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
    /// The document parsed but a key had the wrong type or an unknown key was present.
    Value {
        /// Path that was attempted.
        path: String,
        /// The deserializer's message.
        message: String,
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
        }
    }
}

impl std::error::Error for ConfigError {}

/// Percent-encode one URL component, as required by RFC 3986 for userinfo.
fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        let keep = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if keep {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

impl SqlConfig {
    /// Build a `MySQL` connection URL from these settings.
    ///
    /// The result is `mysql://user:password@host:port/database`. A zero `port` omits the
    /// authority's port, which lets the client library use its default. User and password are
    /// percent-encoded, so a password containing `@` or `/` cannot corrupt the URL.
    #[must_use]
    pub fn connection_url(&self) -> String {
        let mut url = format!(
            "mysql://{}:{}@{}",
            encode_component(&self.user),
            encode_component(&self.password),
            self.host
        );
        if self.port != 0 {
            url.push(':');
            url.push_str(&self.port.to_string());
        }
        url.push('/');
        url.push_str(&self.database);
        url
    }

    /// Whether a host and database have been supplied.
    ///
    /// An unset connection is a configuration error for any binary that must reach SQL.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.host.is_empty() && !self.database.is_empty()
    }
}

/// Deserialize a TOML configuration file into `T`.
///
/// A syntax error and a value error are reported separately: malformed TOML fails before any
/// value is read, while a wrong type or an unknown key is a value error.
fn load_toml<T, P>(path: P) -> Result<T, ConfigError>
where
    T: serde::de::DeserializeOwned,
    P: AsRef<Path>,
{
    let path = path.as_ref();
    let shown = path.display().to_string();
    let content = fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: shown.clone(),
        source: e.to_string(),
    })?;
    // Parse into a table first so that malformed TOML is reported separately from a bad value.
    let table: toml::Table = toml::from_str(&content).map_err(|e| ConfigError::Syntax {
        path: shown.clone(),
        message: e.to_string(),
    })?;
    table
        .try_into()
        .map_err(|e: toml::de::Error| ConfigError::Value {
            path: shown,
            message: e.to_string(),
        })
}

/// Load the game server configuration from a TOML file.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when the file cannot be read, [`ConfigError::Syntax`] when it
/// is not a well-formed TOML document, and [`ConfigError::Value`] when a key has the wrong type
/// or an unknown key is present. An unknown key is an error so that a misspelled setting does
/// not silently keep its default.
pub fn parse_game_config<P: AsRef<Path>>(path: P) -> Result<GameConfig, ConfigError> {
    load_toml(path)
}

/// Load the database server configuration from a TOML file.
///
/// # Errors
///
/// Returns the same [`ConfigError`] variants as [`parse_game_config`].
pub fn parse_db_config<P: AsRef<Path>>(path: P) -> Result<DbConfig, ConfigError> {
    load_toml(path)
}
