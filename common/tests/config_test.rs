//! Integration coverage for the TOML configuration loaders.
//!
//! These tests pin the new file format. The legacy `CONFIG` and `conf.txt` `key=value` files are
//! not read any more, and the legacy-key to TOML-key mapping is recorded in the ledger.

use common::config::{
    parse_db_config, parse_game_config, ConfigError, DbConfig, GameConfig, SqlConfig,
};
use std::io::Write;
use tempfile::NamedTempFile;

fn temp_toml(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

const FULL_GAME: &str = r#"
hostname = "game1"
channel = 1
mother_port = 50080
p2p_port = 50900
db_port = 3306
db_addr = "192.168.1.100"
bind_ip = "0.0.0.0"

map_allow = [1, 21, 41, 43]

auth_server = true
auth_master_ip = "10.0.0.1"
auth_master_port = 51000

adminpage_ips = ["127.0.0.1", "192.168.1.1"]
adminpage_password = "SECRET123"

passes_per_sec = 25
test_server = true
guild_mark_server = true
guild_mark_min_level = 5
user_limit = 10000
empire_whisper = true
table_postfix = "_test"

item_count_limit = 8000
enable_global_shout = true
disable_prism_need = true
gm_host_check = true
guild_invite_limit = true
status_point_get_level_limit = 99
shout_limit_level = 20
db_log_level = 3
sys_log_level = 4

check_version_server = true
check_version_value = "20240101"

quest_dir = "./custom_quest"
quest_object_dirs = ["./quest/obj1", "./quest/obj"]

[player_sql]
host = "192.168.1.100"
port = 3306
user = "root"
password = "password"
database = "player_db"

[common_sql]
host = "192.168.1.100"
port = 3306
user = "root"
password = "password"
database = "common_db"

[log_sql]
host = "192.168.1.100"
port = 3306
user = "root"
password = "password"
database = "log_db"
"#;

const FULL_DB: &str = r#"
test_server = true
log = true
client_heart_fps = 30
log_keep_days = 7
locale = "utf8"
table_postfix = "_test"
player_cache_flush_seconds = 420
item_cache_flush_seconds = 300
item_pricelist_cache_flush_seconds = 540
cache_flush_limit_per_second = 12
player_id_start = 100
name_column = "name"
bind_port = 5300
bind_ip = "127.0.0.1"

trusted_game_peers = ["127.0.0.1", "10.0.0.5"]
trusted_auth_peers = ["10.0.0.9"]

[sql_player]
host = "db.internal"
port = 3306
user = "metin2"
password = "pw"
database = "player_db"

[sql_account]
host = "db.internal"
port = 3306
user = "metin2"
password = "pw"
database = "account_db"

[sql_common]
host = "db.internal"
port = 3306
user = "metin2"
password = "pw"
database = "common_db"
"#;

#[test]
fn game_config_reads_every_group() {
    let config = parse_game_config(temp_toml(FULL_GAME).path()).unwrap();
    assert_eq!(config.hostname, "game1");
    assert_eq!(config.channel, 1);
    assert_eq!(config.mother_port, 50080);
    assert_eq!(config.p2p_port, 50900);
    assert_eq!(config.db_port, 3306);
    assert_eq!(config.db_addr, "192.168.1.100");
    assert_eq!(config.bind_ip, "0.0.0.0");
    assert_eq!(config.map_allow, vec![1, 21, 41, 43]);
    assert!(config.auth_server);
    assert_eq!(config.auth_master_ip, "10.0.0.1");
    assert_eq!(config.auth_master_port, 51000);
    assert_eq!(config.adminpage_ips, vec!["127.0.0.1", "192.168.1.1"]);
    assert_eq!(config.adminpage_password, "SECRET123");
    assert_eq!(config.passes_per_sec, 25);
    assert!(config.test_server);
    assert!(config.guild_mark_server);
    assert_eq!(config.guild_mark_min_level, 5);
    assert_eq!(config.user_limit, 10000);
    assert!(config.empire_whisper);
    assert_eq!(config.table_postfix, "_test");
    assert_eq!(config.item_count_limit, 8000);
    assert!(config.enable_global_shout);
    assert!(config.disable_prism_need);
    assert!(config.gm_host_check);
    assert!(config.guild_invite_limit);
    assert_eq!(config.status_point_get_level_limit, 99);
    assert_eq!(config.shout_limit_level, 20);
    assert_eq!(config.db_log_level, 3);
    assert_eq!(config.sys_log_level, 4);
    assert!(config.check_version_server);
    assert_eq!(config.check_version_value, "20240101");
    assert_eq!(config.quest_dir, "./custom_quest");
    assert_eq!(
        config.quest_object_dirs,
        vec!["./quest/obj1", "./quest/obj"]
    );
}

#[test]
fn game_config_reads_the_three_sql_sub_tables() {
    let config = parse_game_config(temp_toml(FULL_GAME).path()).unwrap();
    assert_eq!(
        config.player_sql,
        SqlConfig {
            host: "192.168.1.100".into(),
            user: "root".into(),
            password: "password".into(),
            database: "player_db".into(),
            port: 3306,
        }
    );
    assert_eq!(config.common_sql.database, "common_db");
    assert_eq!(config.log_sql.database, "log_db");
}

#[test]
fn db_config_reads_every_group() {
    let config = parse_db_config(temp_toml(FULL_DB).path()).unwrap();
    assert!(config.test_server);
    assert!(config.log);
    assert_eq!(config.client_heart_fps, 30);
    assert_eq!(config.log_keep_days, 7);
    assert_eq!(config.locale, "utf8");
    assert_eq!(config.table_postfix, "_test");
    assert_eq!(config.player_cache_flush_seconds, 420);
    assert_eq!(config.item_cache_flush_seconds, 300);
    assert_eq!(config.item_pricelist_cache_flush_seconds, 540);
    assert_eq!(config.cache_flush_limit_per_second, 12);
    assert_eq!(config.player_id_start, 100);
    assert_eq!(config.name_column, "name");
    assert_eq!(config.bind_port, 5300);
    assert_eq!(config.bind_ip, "127.0.0.1");
    assert_eq!(config.trusted_game_peers, vec!["127.0.0.1", "10.0.0.5"]);
    assert_eq!(config.trusted_auth_peers, vec!["10.0.0.9"]);
    assert_eq!(config.sql_player.database, "player_db");
    assert_eq!(config.sql_account.database, "account_db");
    assert_eq!(config.sql_common.database, "common_db");
}

#[test]
fn omitted_keys_keep_their_documented_defaults() {
    let config = parse_game_config(temp_toml("mother_port = 51000\n").path()).unwrap();
    let d = GameConfig::default();
    assert_eq!(config.mother_port, 51000);
    assert_eq!(config.p2p_port, d.p2p_port);
    assert_eq!(config.passes_per_sec, d.passes_per_sec);
    assert_eq!(config.user_limit, d.user_limit);
    assert_eq!(config.adminpage_password, d.adminpage_password);
    assert_eq!(config.max_level, d.max_level);
    assert_eq!(config.view_range, d.view_range);
    assert_eq!(config.quest_dir, d.quest_dir);
}

#[test]
fn an_empty_document_is_all_defaults() {
    assert_eq!(
        parse_game_config(temp_toml("").path()).unwrap(),
        GameConfig::default()
    );
    assert_eq!(
        parse_db_config(temp_toml("").path()).unwrap(),
        common::config::DbConfig::default()
    );
}

#[test]
fn malformed_toml_is_a_syntax_error() {
    let err = parse_game_config(temp_toml("mother_port = \n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { .. }), "got {err:?}");

    let err = parse_game_config(temp_toml("[unclosed\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { .. }), "got {err:?}");
}

#[test]
fn a_wrong_type_is_a_value_error() {
    let err = parse_game_config(temp_toml("mother_port = \"not a port\"\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");

    let err = parse_db_config(temp_toml("client_heart_fps = 1.5\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
}

#[test]
fn an_unknown_key_is_rejected_rather_than_silently_defaulted() {
    // A misspelled key must fail at startup, not keep its default.
    let err = parse_game_config(temp_toml("mother_prot = 1\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
    assert!(err.to_string().contains("mother_prot"), "got {err}");

    let err = parse_db_config(temp_toml("bind_prot = 1\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
}

#[test]
fn an_unknown_sql_sub_table_key_is_rejected() {
    let err = parse_game_config(temp_toml("[player_sql]\nhosts = \"x\"\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
}

#[test]
fn a_missing_file_is_a_read_error() {
    let err = parse_game_config("/nonexistent/path/game.toml").unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }), "got {err:?}");

    let err = parse_db_config("/nonexistent/path/db.toml").unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }), "got {err:?}");
}

#[test]
fn a_realistic_legacy_config_file_is_rejected() {
    // Note that a single legacy scalar line such as `mother_port=50080` also happens to be
    // valid TOML, and is accepted. A whole legacy `CONFIG` is not accepted, because the
    // space-separated list and SQL lines are not TOML values. Operators must convert.
    let legacy = "\
mother_port = 50080
map_allow=1 21 41
player_sql=192.168.1.100 root password player_db 3306
";
    let err = parse_game_config(temp_toml(legacy).path()).unwrap_err();
    assert!(matches!(err, ConfigError::Syntax { .. }), "got {err:?}");
}

#[test]
fn a_legacy_sql_line_cannot_silently_become_a_toml_string() {
    // The legacy SQL value was one space-separated string. It must not be accepted as the
    // `[player_sql]` table, or a credential would be silently mis-parsed.
    let err =
        parse_game_config(temp_toml("player_sql = \"h user pw db 3306\"\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let config = parse_game_config(
        temp_toml(
            "# a leading comment\n\
             mother_port = 52000\n\
             \n\
             # another comment\n\
             channel = 3\n",
        )
        .path(),
    )
    .unwrap();
    assert_eq!(config.mother_port, 52000);
    assert_eq!(config.channel, 3);
}

#[test]
fn booleans_must_be_real_toml_booleans() {
    assert!(
        parse_game_config(temp_toml("test_server = true\n").path())
            .unwrap()
            .test_server
    );
    assert!(
        !parse_game_config(temp_toml("test_server = false\n").path())
            .unwrap()
            .test_server
    );
    // Legacy "1" was a boolean; in TOML it is an integer and must be rejected.
    let err = parse_game_config(temp_toml("test_server = 1\n").path()).unwrap_err();
    assert!(matches!(err, ConfigError::Value { .. }), "got {err:?}");
}

#[test]
fn a_round_trip_through_toml_is_stable() {
    // Serializing the default config and reading it back must reproduce the defaults, so an
    // operator can start from a generated file.
    let rendered = toml::to_string(&GameConfig::default()).unwrap();
    let config = parse_game_config(temp_toml(&rendered).path()).unwrap();
    assert_eq!(config, GameConfig::default());
}

#[test]
fn connection_url_uses_mysql_scheme_and_includes_the_port() {
    let sql = SqlConfig {
        host: "db.internal".into(),
        user: "metin2".into(),
        password: "pw".into(),
        database: "player".into(),
        port: 3306,
    };
    assert_eq!(
        sql.connection_url(),
        "mysql://metin2:pw@db.internal:3306/player"
    );
}

#[test]
fn connection_url_omits_a_zero_port() {
    let sql = SqlConfig {
        host: "localhost".into(),
        user: "u".into(),
        password: "p".into(),
        database: "d".into(),
        port: 0,
    };
    assert_eq!(sql.connection_url(), "mysql://u:p@localhost/d");
}

#[test]
fn connection_url_percent_encodes_credentials() {
    // A password containing URL syntax must not be able to redirect the connection.
    let sql = SqlConfig {
        host: "h".into(),
        user: "us@er".into(),
        password: "p@ss:w/d?#".into(),
        database: "d".into(),
        port: 0,
    };
    assert_eq!(
        sql.connection_url(),
        "mysql://us%40er:p%40ss%3Aw%2Fd%3F%23@h/d"
    );
}

#[test]
fn is_configured_requires_a_host_and_a_database() {
    assert!(!SqlConfig::default().is_configured());
    let with_host = SqlConfig {
        host: "h".into(),
        ..SqlConfig::default()
    };
    assert!(!with_host.is_configured());
    let with_db = SqlConfig {
        host: "h".into(),
        database: "d".into(),
        ..SqlConfig::default()
    };
    assert!(with_db.is_configured());
}

/// The trusted-peer lists are part of the DB configuration surface.
mod trusted_peers {
    use super::*;

    /// Parse a DB document that must be valid.
    fn db_config(text: &str) -> DbConfig {
        parse_db_config(temp_toml(text).path()).expect("the document should parse")
    }

    #[test]
    fn both_lists_default_to_empty_which_denies_every_peer() {
        let config = db_config("");
        assert!(config.trusted_game_peers.is_empty());
        assert!(config.trusted_auth_peers.is_empty());
    }

    #[test]
    fn a_complete_document_can_list_trusted_peers() {
        let config = db_config(
            "trusted_game_peers = [\"127.0.0.1\", \"10.0.0.5\"]\ntrusted_auth_peers = [\"10.0.0.9\"]\n",
        );
        assert_eq!(config.trusted_game_peers, vec!["127.0.0.1", "10.0.0.5"]);
        assert_eq!(config.trusted_auth_peers, vec!["10.0.0.9"]);
    }

    #[test]
    fn a_single_empty_list_still_parses() {
        let config = db_config("trusted_game_peers = []\n");
        assert!(config.trusted_game_peers.is_empty());
        assert!(config.trusted_auth_peers.is_empty());
    }

    #[test]
    fn a_bare_string_is_not_a_peer_list() {
        let error = parse_db_config(temp_toml("trusted_game_peers = \"127.0.0.1\"\n").path());
        assert!(matches!(error, Err(ConfigError::Value { .. })));
    }

    #[test]
    fn a_non_string_entry_is_rejected() {
        let error = parse_db_config(temp_toml("trusted_auth_peers = [7]\n").path());
        assert!(matches!(error, Err(ConfigError::Value { .. })));
    }

    #[test]
    fn the_lists_are_game_only_keys() {
        // A game config has no trusted-peer keys, and a typo is still caught.
        let error = parse_game_config(temp_toml("trusted_game_peers = [\"127.0.0.1\"]\n").path());
        assert!(matches!(error, Err(ConfigError::Value { .. })));
    }
}
