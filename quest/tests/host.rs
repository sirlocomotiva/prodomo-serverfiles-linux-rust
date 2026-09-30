//! The quest host against the Game data: the library chain, every compiled script, the API
//! names and the sandbox.

use std::fs;
use std::path::{Path, PathBuf};

use mlua::{FromLuaMulti, Function, Table, Value};
use quest::api;
use quest::host::{self, Bgm, GotoInfo, Host};
use quest::qc::crc32;

fn locale_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe")
}

fn load() -> Host {
    Host::load(&locale_dir()).unwrap_or_else(|error| panic!("{error}"))
}

fn eval<T: FromLuaMulti>(host: &Host, code: &str) -> mlua::Result<T> {
    host::load_chunk(host.lua(), code.as_bytes(), "=test")?.call(())
}

fn lookup(host: &Host, name: &str) -> Value {
    let globals = host.lua().globals();
    match name.split_once('.') {
        None => globals.get(name).unwrap(),
        Some((namespace, member)) => globals
            .get::<Table>(namespace)
            .unwrap()
            .get(member)
            .unwrap(),
    }
}

#[test]
fn every_script_but_change_empire_loads() {
    let host = load();
    let refused: Vec<String> = host
        .refused()
        .iter()
        .map(|refused| format!("{}: {}", refused.path.display(), refused.error))
        .collect();
    assert_eq!(
        refused,
        ["_basic/change_empire.lua: line 139: expecting 'when' or 'function'"]
    );
    // 50 scripts, two of which declare `guild_building`.
    assert_eq!(host.quests().len(), 49);
    let guild_building = host
        .quests()
        .iter()
        .filter(|quest| quest.name == b"guild_building")
        .count();
    assert_eq!(guild_building, 1);
    // `quest_list` comes first.
    assert_eq!(host.quest_index(b"dungeoninfo"), Some(1));
    assert_eq!(host.quest_index(b"change_empire"), None);
    for (index, quest) in host.quests().iter().enumerate() {
        assert_eq!(host.quest_index(&quest.name), u32::try_from(index + 1).ok());
    }
    let mut chunks = 0;
    let mut empty_conditions = 0;
    let mut texts = 0;
    for (file, bytes) in host.objects() {
        let path = String::from_utf8_lossy(file);
        let empty_condition = path.ends_with(".when") && bytes.is_empty();
        let text = path.ends_with(".arg") && !path.contains("/chat/");
        let runs = !path.starts_with("state/") && !empty_condition && !text;
        assert_eq!(host.chunk(file).is_some(), runs, "{path}");
        chunks += usize::from(runs);
        empty_conditions += usize::from(empty_condition);
        texts += usize::from(text);
    }
    // 49 state tables, 38 `when` blocks without a condition, and the arguments of 5 `target`
    // and 1 `click` blocks.
    assert_eq!(host.objects().len(), 455);
    assert_eq!((empty_conditions, texts), (38, 6));
    assert_eq!(chunks, 455 - 49 - 38 - 6);
}

#[test]
fn a_chat_argument_runs_as_script_to_string_did() {
    let host = load();
    let arg = host
        .chunk(b"20011/chat/oxevent_manager.start.0.arg")
        .unwrap();
    assert_eq!(arg.call::<String>(()).unwrap(), "OX Contest ");
    // A target's argument names the target and is compared as text.
    let target = b"notarget/target/sash_mission.information.0.arg";
    assert_eq!(host.objects()[&target[..]], b"theowahdan.click");
    assert!(host.chunk(target).is_none());
}

#[test]
fn every_state_is_indexed_both_ways() {
    let host = load();
    let oxevent =
        &host.quests()[usize::try_from(host.quest_index(b"oxevent_manager").unwrap()).unwrap() - 1];
    assert_eq!(oxevent.states.len(), 1);
    assert_eq!(oxevent.states[&b"start"[..]], 0);
    let sash =
        &host.quests()[usize::try_from(host.quest_index(b"sash_mission").unwrap()).unwrap() - 1];
    let information = i32::from_le_bytes(crc32(b"information").to_le_bytes());
    assert_eq!(sash.states[&b"information"[..]], information);
    let mut states = 0;
    for quest in host.quests() {
        let table: Table = host
            .lua()
            .globals()
            .get(String::from_utf8_lossy(&quest.name).as_ref())
            .unwrap();
        for (state, number) in &quest.states {
            let name: mlua::String = table.raw_get(*number).unwrap();
            assert_eq!(name.as_bytes().as_ref(), &state[..]);
            states += 1;
        }
    }
    assert_eq!(
        eval::<String>(&host, "return sash_mission[sash_mission.information]").unwrap(),
        "information"
    );
    assert!(states > host.quests().len());
}

#[test]
fn settings_lua_records_the_music_the_goto_list_and_the_arenas() {
    let host = load();
    let settings = host.settings();
    let source = fs::read(locale_dir().join("settings.lua")).unwrap();
    let source = String::from_utf8_lossy(&source);
    let calls = |prefix: &str| {
        source
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    };
    // The positive controls: the calls the file makes.
    assert_eq!(calls("add_bgm_info("), 51);
    assert_eq!(calls("add_goto_info("), 88);
    assert_eq!(calls("arena.add_map("), 4);
    assert_eq!(settings.bgm.len(), 51);
    assert_eq!(
        settings.bgm[&304],
        Bgm {
            name: b"mt.mp3".to_vec(),
            volume: 0.5
        }
    );
    assert!(settings.bgm_volume_enable);
    assert_eq!(settings.goto_info.len(), 88);
    assert_eq!(
        settings.goto_info[0],
        GotoInfo {
            name: b"a1".to_vec(),
            empire: 0,
            map_index: 1,
            x: 4693,
            y: 9642
        }
    );
    assert_eq!(settings.arenas.len(), 4);
    assert_eq!(settings.arenas[3].map_index, 112);
    assert_eq!(settings.arenas[3].start_a, (8584, 155));
    assert_eq!(settings.arenas[3].start_b, (8614, 155));
}

#[test]
fn every_api_name_is_registered_and_the_unported_ones_refuse() {
    let inventory = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.scratch/parity/quest-api.md"),
    )
    .unwrap();
    let ids: Vec<&str> = inventory
        .lines()
        .filter_map(|line| line.strip_prefix("| `lua."))
        .map(|line| &line[..line.find('`').unwrap()])
        .collect();
    assert_eq!(ids, api::NAMES);
    let host = load();
    let mut refusing = 0;
    let mut waiting = Vec::new();
    for name in api::NAMES {
        let Value::Function(function) = lookup(&host, name) else {
            panic!("{name} is not a function");
        };
        // No library replaces an API function with a Lua one.
        assert_eq!(function.info().what, "C", "{name}");
        if let Err(error) = function.call::<()>(()) {
            let error = error.to_string();
            if error.contains(&format!("not ported: {name}")) {
                refusing += 1;
            } else if error.contains(&format!("{name}: no script is running")) {
                waiting.push(name);
            }
        }
    }
    // The bridged names need a running script; the four recorders, `get_locale_base_path` and
    // `q.yield` answer.
    assert_eq!(waiting, host::BRIDGED);
    assert_eq!(refusing, api::NAMES.len() - 6 - host::BRIDGED.len());
    // A Lua function reports `Lua`: the negative control of the `C` check.
    let say_title: Function = host.lua().globals().get("say_title").unwrap();
    assert_eq!(say_title.info().what, "Lua");
    assert!(eval::<bool>(&host, "return rawequal(q.yield, coroutine.yield)").unwrap());
    let error = eval::<()>(&host, "pc.warp(896500, 24600)").unwrap_err();
    assert!(error.to_string().contains("not ported: pc.warp"), "{error}");
    let base: mlua::String = eval(&host, "return get_locale_base_path()").unwrap();
    assert_eq!(
        base.as_bytes().as_ref(),
        locale_dir().as_os_str().as_encoded_bytes()
    );
}

#[test]
fn the_libraries_legacy_never_loads_load_in_a_separate_state() {
    let host = load();
    let kind = |function: &str| eval::<String>(&host, &format!("return type({function})")).unwrap();
    let dofile = |library: &str| {
        eval::<()>(
            &host,
            &format!("dofile(get_locale_base_path() .. '/quest/{library}')"),
        )
    };
    assert_eq!(kind("mysql_query_old"), "nil");
    dofile("questing.lua").unwrap();
    assert_eq!(kind("mysql_query_old"), "function");
    // `questlib_extra.lua` ends by reading event flags, which only a running script can: its
    // functions are defined when that call fails.
    assert_eq!(kind("check_event_flags"), "nil");
    let error = dofile("questlib_extra.lua").unwrap_err().to_string();
    assert!(
        error.contains("game.get_event_flag: no script is running"),
        "{error}"
    );
    assert!(
        error.contains("questlib_extra.lua:152: in main chunk"),
        "{error}"
    );
    assert_eq!(kind("check_event_flags"), "function");
}

#[test]
fn a_quest_gets_no_files_no_processes_and_no_debug() {
    let host = load();
    for name in ["io", "debug", "load", "loadfile", "require", "package"] {
        assert_eq!(
            eval::<String>(&host, &format!("return type({name})")).unwrap(),
            "nil",
            "{name}"
        );
    }
    let os: Vec<String> = eval(
        &host,
        "local names = {} for name in pairs(os) do names[table.getn(names) + 1] = name end \
         table.sort(names) return names",
    )
    .unwrap();
    assert_eq!(os, ["clock", "date", "difftime", "time"]);
    // `dofile` reads below the locale directory only.
    eval::<()>(
        &host,
        "dofile(get_locale_base_path() .. '/quest/locale.lua')",
    )
    .unwrap();
    for outside in [
        "'/etc/hostname'",
        "get_locale_base_path() .. '/../europe/quest/locale.lua'",
        "get_locale_base_path()",
        "get_locale_base_path() .. '_x/settings.lua'",
    ] {
        let error = eval::<()>(&host, &format!("dofile({outside})")).unwrap_err();
        assert!(
            error.to_string().contains("dofile reads only below"),
            "{outside}: {error}"
        );
    }
}

#[test]
fn loadstring_reads_the_dialect() {
    let host = load();
    assert!(eval::<bool>(&host, "return loadstring('return 1 != 2 and !false')()").unwrap());
    let (function, message): (Value, String) =
        eval(&host, "return loadstring('return #x', 'chunk')").unwrap();
    assert_eq!(function, Value::Nil);
    assert!(message.contains("chunk"), "{message}");
    let (function, message): (Value, String) =
        eval(&host, "return loadstring('return (', 'chunk')").unwrap();
    assert_eq!(function, Value::Nil);
    assert!(message.contains("chunk"), "{message}");
}
