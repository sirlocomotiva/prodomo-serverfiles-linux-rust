//! Integration coverage for the `prodomo.toml` loader.
//!
//! These tests pin the document format and the topology rules. The legacy `CONFIG` and
//! `conf.txt` files are not read; ledger sections 169 and 178 map their keys to this document.

use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use common::config::{
    load_server_config, parse_server_config, redact_url, ConfigError, GameSettings, ItemIdSpan,
    Secret, TopologyError, SHARED_CHANNEL,
};
use tempfile::NamedTempFile;

fn temp_toml(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

/// The smallest valid document: a store, auth, and one Channel.
/// One valid non-shared Channel, the smallest text that reaches `validate`.
const CHANNEL: &str = "[[channel]]\nnumber = 1\nports = [1]\nmaps = [1]\n";

const MINIMAL: &str = r#"
[store]
url = "postgres://prodomo@127.0.0.1/prodomo"

[auth]
port = 30001

[[channel]]
number = 1
ports = [30003]
maps = [1]
"#;

const FULL: &str = r#"
bind_ip = "10.0.0.2"
public_ip = "203.0.113.7"
game_data = "/srv/share"
game_tables = "/srv/tables"

[store]
url = "postgres://prodomo:pw@db.local:5432/prodomo"
max_connections = 16

[auth]
port = 30001

[[channel]]
number = 1
ports = [30003, 30005]
maps = [1, 21, 41, 56]

[[channel]]
number = 2
ports = [30007]
maps = [1, 21, 41]

[[channel]]
number = 99
ports = [30019]
maps = [72, 73]

[game]
adminpage_ips = ["127.0.0.1"]
adminpage_password = "SECRET123"
save_event_second_cycle = 180
ping_event_second_cycle = 180
max_level = 120
item_count_limit = 2000
enable_global_shout = true
disable_emotion_mask = true
mantie_permanent = true
"#;

fn parse(content: &str) -> Result<common::config::ServerConfig, ConfigError> {
    parse_server_config(content, "test.toml")
}

fn invalid(content: &str) -> TopologyError {
    match parse(content) {
        Err(ConfigError::Invalid { error, .. }) => error,
        other => panic!("expected a topology error, got {other:?}"),
    }
}

#[test]
fn a_full_document_reads_every_table() {
    let file = temp_toml(FULL);
    let config = load_server_config(file.path()).unwrap();

    assert_eq!(config.bind_ip, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
    assert_eq!(config.public_ip, Ipv4Addr::new(203, 0, 113, 7));
    assert_eq!(config.game_data, Path::new("/srv/share"));
    assert_eq!(config.map_dir(), Path::new("/srv/share/locale/europe/map"));
    assert_eq!(config.proto_dir(), Path::new("/srv/share/proto"));
    assert_eq!(config.game_tables, Path::new("/srv/tables"));
    assert_eq!(
        config.store.url,
        "postgres://prodomo:pw@db.local:5432/prodomo"
    );
    assert_eq!(config.store.max_connections, 16);
    assert_eq!(config.auth.port, 30001);
    let summary: Vec<(u8, Vec<u16>, usize, bool)> = config
        .channels
        .iter()
        .map(|c| (c.number, c.ports.clone(), c.maps.len(), c.is_shared()))
        .collect();
    assert_eq!(
        summary,
        vec![
            (1, vec![30003, 30005], 4, false),
            (2, vec![30007], 3, false),
            (99, vec![30019], 2, true),
        ]
    );
    assert_eq!(config.game.adminpage_ips, vec!["127.0.0.1"]);
    assert_eq!(config.game.adminpage_password.expose(), "SECRET123");
    assert_eq!(config.game.save_event_second_cycle, 180);
    assert_eq!(config.game.ping_event_second_cycle, 180);
    assert_eq!(config.game.max_level, 120);
    assert_eq!(config.game.item_count_limit, 2000);
    assert!(config.game.enable_global_shout);
    assert!(config.game.disable_emotion_mask);
    assert!(config.game.mantie_permanent);
}

#[test]
fn omitted_keys_keep_their_documented_defaults() {
    let config = parse(MINIMAL).unwrap();
    assert_eq!(config.bind_ip, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    assert_eq!(config.public_ip, Ipv4Addr::LOCALHOST);
    assert_eq!(config.game_data, Path::new("legacy/gamedata"));
    assert_eq!(config.game_tables, Path::new("legacy/sql/gamedata"));
    assert_eq!(
        config.map_dir(),
        Path::new("legacy/gamedata/locale/europe/map")
    );
    assert_eq!(config.store.max_connections, 8);
    assert_eq!(config.game, GameSettings::default());
}

#[test]
fn game_defaults_are_the_legacy_compiled_in_values() {
    // `game/config.cpp:20-135`, except `adminpage_password` and the two cycles (see the ledger).
    let game = GameSettings::default();
    assert!(game.test_server, "config.cpp:72 defaults test_server to 1");
    assert_eq!(
        game.block_login, "30000705",
        "an empty value would refuse every account"
    );
    assert_eq!(game.save_event_second_cycle, 120);
    assert_eq!(game.ping_event_second_cycle, 60);
    assert_eq!(game.item_count_limit, 5000);
    assert_eq!(game.item_bonus_change_time, 60);
    assert_eq!(game.status_point_get_level_limit, 90);
    assert_eq!(game.view_range, 5000);
    assert_eq!(game.max_level, 99);
    assert_eq!(
        game.player_delete_level_limit, 251,
        "D/ClientManager.cpp:297"
    );
    assert_eq!(game.player_delete_level_limit_lower, 0);
    assert_eq!(game.user_limit, 32768);
    assert_eq!(game.check_version_value, "1215955205");
    assert!(
        game.adminpage_password.is_empty(),
        "the admin page has no default password"
    );
}

#[test]
fn the_shipped_example_is_a_valid_document() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../config/prodomo.toml.example"
    );
    let config = load_server_config(path).unwrap();
    let numbers: Vec<u8> = config.channels.iter().map(|c| c.number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, SHARED_CHANNEL]);
    assert_eq!(config.auth.port, 30001);
    assert_eq!(config.channels[0].maps.len(), 57);
    assert!(config.channels[1..4].iter().all(|c| c.maps.len() == 49));
    assert_eq!(config.channels[4].maps.len(), 29);
}

#[test]
fn secrets_never_reach_debug_output() {
    let config = parse(FULL).unwrap();
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("SECRET123"),
        "admin page password leaked: {shown}"
    );
    assert!(!shown.contains(":pw@"), "store password leaked: {shown}");
    assert!(
        shown.contains("prodomo:***@db.local:5432/prodomo"),
        "got {shown}"
    );
    assert_eq!(format!("{:?}", Secret::default()), "\"\"");
}

#[test]
fn redaction_keeps_the_host_and_drops_the_password_and_query() {
    assert_eq!(
        redact_url("postgres://u:p@h:5432/d?password=q&sslmode=require"),
        "postgres://u:***@h:5432/d?***"
    );
    assert_eq!(redact_url("postgres://u@h/d"), "postgres://u@h/d");
    assert_eq!(redact_url("postgres://h/d"), "postgres://h/d");
    // `@` may appear in an unencoded password; the last one ends the userinfo.
    assert_eq!(redact_url("postgres://u:a@b@h/d"), "postgres://u:***@h/d");
    assert_eq!(
        redact_url("u:p@h/d"),
        "***",
        "not a URL, so nothing survives"
    );
}

#[test]
fn malformed_toml_is_a_syntax_error() {
    let error = parse("[store\nurl = 1").unwrap_err();
    assert!(matches!(error, ConfigError::Syntax { .. }), "got {error:?}");
}

#[test]
fn a_missing_required_table_is_a_value_error() {
    for table in ["[store]", "[auth]"] {
        let without: String = MINIMAL
            .split("\n\n")
            .filter(|block| !block.trim_start().starts_with(table))
            .collect::<Vec<_>>()
            .join("\n\n");
        let error = parse(&without).unwrap_err();
        assert!(
            matches!(error, ConfigError::Value { .. }),
            "{table}: got {error:?}"
        );
    }
}

#[test]
fn a_wrong_type_is_a_value_error() {
    let error = parse(&MINIMAL.replace("port = 30001", "port = \"30001\"")).unwrap_err();
    assert!(matches!(error, ConfigError::Value { .. }), "got {error:?}");
    let error = parse(&format!("{MINIMAL}\n[game]\ntest_server = 1\n")).unwrap_err();
    assert!(
        matches!(error, ConfigError::Value { .. }),
        "booleans are not 1/0: {error:?}"
    );
}

#[test]
fn an_ipv6_public_ip_is_refused_because_the_client_field_is_four_bytes() {
    let error = parse(&format!("public_ip = \"::1\"\n{MINIMAL}")).unwrap_err();
    assert!(matches!(error, ConfigError::Value { .. }), "got {error:?}");
}

#[test]
fn unknown_keys_are_rejected_at_every_level() {
    let cases = [
        format!("mother_port = 1\n{MINIMAL}"),
        MINIMAL.replace("[store]", "[store]\npool = 1"),
        MINIMAL.replace("port = 30001", "port = 30001\nmaster = true"),
        MINIMAL.replace("maps = [1]", "maps = [1]\nmap_allow = [1]"),
        format!("{MINIMAL}\n[game]\npasses_per_sec = 25\n"),
        format!("{MINIMAL}\n[game]\nplayer_sql = 1\n"),
    ];
    for case in cases {
        let error = parse(&case).unwrap_err();
        assert!(
            matches!(error, ConfigError::Value { .. }),
            "{case}\ngot {error:?}"
        );
    }
}

fn with_channels(channels: &str) -> String {
    format!("[store]\nurl = \"postgres://h/d\"\n[auth]\nport = 30001\n{channels}")
}

#[test]
fn event_cycles_are_at_least_one_second() {
    let channel = "[[channel]]\nnumber = 1\nports = [1]\nmaps = [1]\n";
    for key in ["save_event_second_cycle", "ping_event_second_cycle"] {
        let zero = with_channels(&format!("[game]\n{key} = 0\n{channel}"));
        assert_eq!(invalid(&zero), TopologyError::ZeroCycle(key));
        // Control: one second is accepted.
        let one = with_channels(&format!("[game]\n{key} = 1\n{channel}"));
        assert!(parse(&one).is_ok(), "{key} = 1 was refused");
    }
}

#[test]
fn channel_numbers_must_be_1_to_99_and_unique() {
    let zero = with_channels("[[channel]]\nnumber = 0\nports = [1]\nmaps = [1]\n");
    assert_eq!(invalid(&zero), TopologyError::ChannelNumber(0));
    let hundred = with_channels("[[channel]]\nnumber = 100\nports = [1]\nmaps = [1]\n");
    assert_eq!(invalid(&hundred), TopologyError::ChannelNumber(100));
    let twice = with_channels(
        "[[channel]]\nnumber = 2\nports = [1]\nmaps = [1]\n\
         [[channel]]\nnumber = 2\nports = [2]\nmaps = [1]\n",
    );
    assert_eq!(invalid(&twice), TopologyError::DuplicateChannel(2));
}

#[test]
fn a_shared_channel_alone_cannot_be_picked_at_login() {
    assert_eq!(invalid(&with_channels("")), TopologyError::NoLoginChannel);
    let shared_only = with_channels("[[channel]]\nnumber = 99\nports = [1]\nmaps = [72]\n");
    assert_eq!(invalid(&shared_only), TopologyError::NoLoginChannel);
}

#[test]
fn every_channel_needs_a_port_and_a_map() {
    let no_ports = with_channels("[[channel]]\nnumber = 1\nports = []\nmaps = [1]\n");
    assert_eq!(invalid(&no_ports), TopologyError::NoPorts(1));
    let no_maps = with_channels("[[channel]]\nnumber = 1\nports = [1]\nmaps = []\n");
    assert_eq!(invalid(&no_maps), TopologyError::NoMaps(1));
}

#[test]
fn maps_are_nonzero_and_listed_once_per_channel() {
    let zero = with_channels("[[channel]]\nnumber = 3\nports = [1]\nmaps = [1, 0]\n");
    assert_eq!(invalid(&zero), TopologyError::ZeroMap(3));
    let twice = with_channels("[[channel]]\nnumber = 3\nports = [1]\nmaps = [5, 7, 5]\n");
    assert_eq!(
        invalid(&twice),
        TopologyError::DuplicateMap { channel: 3, map: 5 }
    );
    let negative = with_channels("[[channel]]\nnumber = 3\nports = [1]\nmaps = [-1]\n");
    assert!(matches!(parse(&negative), Err(ConfigError::Value { .. })));
}

#[test]
fn nonzero_ports_are_unique_across_every_listener_and_zero_may_repeat() {
    let with_auth = with_channels("[[channel]]\nnumber = 1\nports = [30001]\nmaps = [1]\n");
    assert_eq!(invalid(&with_auth), TopologyError::DuplicatePort(30001));
    let across = with_channels(
        "[[channel]]\nnumber = 1\nports = [30003]\nmaps = [1]\n\
         [[channel]]\nnumber = 2\nports = [30003]\nmaps = [1]\n",
    );
    assert_eq!(invalid(&across), TopologyError::DuplicatePort(30003));
    let within = with_channels("[[channel]]\nnumber = 1\nports = [7, 7]\nmaps = [1]\n");
    assert_eq!(invalid(&within), TopologyError::DuplicatePort(7));
    let zeros = "[store]\nurl = \"postgres://h/d\"\n[auth]\nport = 0\n\
                 [[channel]]\nnumber = 1\nports = [0, 0]\nmaps = [1]\n";
    assert!(
        parse(zeros).is_ok(),
        "port 0 is chosen by the operating system"
    );
}

#[test]
fn a_shared_channel_map_is_hosted_nowhere_else() {
    let clash = with_channels(
        "[[channel]]\nnumber = 1\nports = [1]\nmaps = [1, 72]\n\
         [[channel]]\nnumber = 99\nports = [2]\nmaps = [72]\n",
    );
    assert_eq!(
        invalid(&clash),
        TopologyError::SharedMapElsewhere {
            map: 72,
            channel: 1
        }
    );
    let shared_by_login_channels = with_channels(
        "[[channel]]\nnumber = 1\nports = [1]\nmaps = [1]\n\
         [[channel]]\nnumber = 2\nports = [2]\nmaps = [1]\n",
    );
    assert!(
        parse(&shared_by_login_channels).is_ok(),
        "Channels repeat maps by design"
    );
}

#[test]
fn errors_name_the_file() {
    let error = load_server_config("/nonexistent/prodomo.toml").unwrap_err();
    assert!(matches!(error, ConfigError::Read { .. }), "got {error:?}");
    assert!(
        error.to_string().contains("/nonexistent/prodomo.toml"),
        "got {error}"
    );
    let error = parse(&with_channels("")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid topology in test.toml: no channel other than the shared channel 99, so none \
         can be picked at login"
    );
}

/// Legacy `ITEM_ID_RANGE` in the owner's `conf.txt`
/// (`legacy/config/db/conf.txt:8`), which is the only one of the five that carries
/// the key. Positive control: the value is the one the live deployment used, so a
/// config that does not set the key must produce it.
const OWNER_SPAN: ItemIdSpan = ItemIdSpan {
    first: 100_000_000,
    last: 200_000_000,
};

#[test]
fn the_item_id_span_defaults_to_the_owners_configured_range() {
    let config = parse(&with_channels(CHANNEL)).unwrap();
    assert_eq!(config.game.item_id_range, OWNER_SPAN);
}

#[test]
fn the_item_id_span_is_read_as_a_pair() {
    let config = parse(&with_channels(&format!(
        "[game]\nitem_id_range = [7, 9]\n{CHANNEL}"
    )))
    .unwrap();
    assert_eq!(config.game.item_id_range, ItemIdSpan { first: 7, last: 9 });
}

#[test]
fn an_item_id_span_that_is_not_ascending_is_refused_before_anything_binds() {
    // Legacy `BuildRange` refuses this span at boot, but only after the listeners
    // were up. Here `validate` runs first, so a bad span never opens a port.
    for (first, last) in [(9u32, 9u32), (9, 7), (0, 0)] {
        let text = with_channels(&format!(
            "[game]\nitem_id_range = [{first}, {last}]\n{CHANNEL}"
        ));
        assert_eq!(
            invalid(&text),
            TopologyError::UnorderedItemIdRange { first, last }
        );
    }
}

#[test]
fn the_item_id_span_pair_must_have_exactly_two_numbers() {
    for value in ["[7]", "[7, 9, 11]", "7", "\"7 9\""] {
        let text = with_channels(&format!("[game]\nitem_id_range = {value}\n{CHANNEL}"));
        let error = parse(&text).unwrap_err();
        assert!(
            matches!(error, ConfigError::Value { .. }),
            "item_id_range = {value}\ngot {error:?}"
        );
    }
}

#[test]
fn a_span_above_its_last_id_reports_no_room_rather_than_wrapping() {
    // The failure this guards is silent: a wrapped subtraction turns an exhausted
    // id space into one that looks like it has four billion ids left.
    let span = ItemIdSpan {
        first: 100,
        last: 200,
    };
    assert_eq!(span.remaining_from(100), 100);
    assert_eq!(span.remaining_from(199), 1);
    assert_eq!(span.remaining_from(200), 0);
    assert_eq!(span.remaining_from(201), 0);
    assert_eq!(span.remaining_from(u32::MAX), 0);
    // A plain subtraction here would give 4_294_967_295, which is what the store
    // would read as a span with billions of ids left. 10_000 is
    // `db::item_id_range::MINIMUM_REMAIN_COUNT`, spelled out because `common` is
    // below `db` and cannot import it.
    assert!(span.remaining_from(u32::MAX) < 10_000);
}

#[test]
fn a_span_reports_room_below_its_own_last_id_only() {
    let span = ItemIdSpan {
        first: 100,
        last: 200,
    };
    assert!(span.is_ordered());
    assert!(!ItemIdSpan {
        first: 200,
        last: 100
    }
    .is_ordered());
    assert!(!ItemIdSpan {
        first: 200,
        last: 200
    }
    .is_ordered());
}
