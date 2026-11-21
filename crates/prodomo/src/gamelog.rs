//! Kept game-log events on the `gamelog` tracing target (`docs/STATUS.md`, owner question 73).
//!
//! Legacy's `LogManager` (`G/log.cpp`) writes rows to a MySQL log database. No log table comes
//! back (ADR-0001). Each kept kind is one `tracing` event on the target `gamelog`, named as
//! legacy names it and carrying legacy's columns as fields, so an operator can route `gamelog`
//! to its own log file.
//!
//! Kinds not kept (`FishLog`; per-drop/pickup/use `ItemLog`; `MoneyLog`'s `MONSTER`,
//! `MONSTER_KILL`, `DROP`) and dead legacy methods (`HackCRCLog`, `PCBangLoginLog`, `CubeLog`,
//! `ChatLog`, `InvalidServerLog`) are omitted by design.

/// The five kept `EMoneyLogType` kinds (`docs/STATUS.md`, `G/log.cpp:124-137`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MoneyLogKind {
    /// `MONEY_LOG_QUEST` (`3`).
    Quest = 3,
    /// `MONEY_LOG_REFINE` (`4`).
    Refine = 4,
    /// `MONEY_LOG_SHOP` (`5`).
    Shop = 5,
    /// `MONEY_LOG_GUILD` (`6`).
    Guild = 6,
    /// `MONEY_LOG_MISC` (`9`).
    Misc = 9,
}

/// `enum GOLDBAR_HOW` (`G/log.h:28-37`, `G/log.cpp:176-214`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoldBarHow {
    /// `PERSONAL_SHOP_BUY` (`'BUY'`).
    PersonalShopBuy,
    /// `PERSONAL_SHOP_SELL` (`'SELL'`).
    PersonalShopSell,
    /// `SHOP_BUY` (`'SHOP_BUY'`).
    ShopBuy,
    /// `SHOP_SELL` (`'SHOP_SELL'`).
    ShopSell,
    /// `EXCHANGE_TAKE` (`'EXCHANGE_TAKE'`).
    ExchangeTake,
    /// `EXCHANGE_GIVE` (`'EXCHANGE_GIVE'`).
    ExchangeGive,
    /// `QUEST` (`'QUEST'`).
    Quest,
}

impl GoldBarHow {
    /// The string legacy writes to `goldlog.how` (`G/log.cpp:179-214`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PersonalShopBuy => "BUY",
            Self::PersonalShopSell => "SELL",
            Self::ShopBuy => "SHOP_BUY",
            Self::ShopSell => "SHOP_SELL",
            Self::ExchangeTake => "EXCHANGE_TAKE",
            Self::ExchangeGive => "EXCHANGE_GIVE",
            Self::Quest => "QUEST",
        }
    }
}

/// Whether `vnum` is one of the five gold-bar items (`80003..=80007`, `G/shop.cpp`, `G/exchange.cpp`).
#[must_use]
pub const fn is_gold_bar(vnum: u32) -> bool {
    vnum >= 80_003 && vnum <= 80_007
}

/// `LogManager::HackLog` (`G/log.cpp:139-145`).
pub fn hack_log(login: &str, name: &str, ip: &str, server: &str, why: &str) {
    tracing::info!(
        target: "gamelog",
        login,
        name,
        ip,
        server,
        why,
        "HackLog"
    );
}

/// `LogManager::SpeedHackLog` (`G/log.cpp:229-235`).
pub fn speed_hack_log(pid: u32, x: i32, y: i32, hack_count: u32) {
    tracing::info!(
        target: "gamelog",
        pid,
        x,
        y,
        hack_count,
        "SpeedHackLog"
    );
}

/// `LogManager::GMCommandLog` (`G/log.cpp:245-252`).
pub fn gm_command_log(userid: u32, ip: &str, port: u8, username: &str, command: &str) {
    tracing::info!(
        target: "gamelog",
        userid,
        server = 999_u16,
        ip,
        port,
        username,
        command,
        "GMCommandLog"
    );
}

/// `LogManager::LoginLog` (`G/log.cpp:117-122`).
pub fn login_log(
    is_login: bool,
    channel: u8,
    account_id: u32,
    pid: u32,
    level: u8,
    job: u8,
    playtime: u32,
) {
    let kind = if is_login { "LOGIN" } else { "LOGOUT" };
    tracing::info!(
        target: "gamelog",
        r#type = kind,
        channel,
        account_id,
        pid,
        level,
        job,
        playtime,
        "LoginLog"
    );
}

/// `LogManager::DetailLoginLog` (`G/log.cpp:316-341`).
pub fn detail_login_log(
    is_login: bool,
    is_gm: bool,
    channel: u8,
    account_id: u32,
    pid: u32,
    ip: &str,
    client_version: &str,
) {
    let kind = if is_login { "LOGIN" } else { "LOGOUT" };
    let gm = if is_gm { "Y" } else { "N" };
    tracing::info!(
        target: "gamelog",
        r#type = kind,
        is_gm = gm,
        channel,
        account_id,
        pid,
        ip,
        client_version,
        "DetailLoginLog"
    );
}

/// `LogManager::BootLog` (`G/log.cpp:286-291`).
pub fn boot_log(hostname: &str, channel: u8) {
    tracing::info!(
        target: "gamelog",
        hostname,
        channel,
        "BootLog"
    );
}

/// `LogManager::ChangeNameLog` (`G/log.cpp:237-243`).
pub fn change_name_log(pid: u32, old_name: &str, new_name: &str, ip: &str) {
    tracing::info!(
        target: "gamelog",
        pid,
        old_name,
        new_name,
        ip,
        "ChangeNameLog"
    );
}

/// `LogManager::LevelLog` (`G/log.cpp:271-284`).
pub fn level_log(name: &str, level: u8, account_id: u32, pid: u32, playtime: u32) {
    tracing::info!(
        target: "gamelog",
        name,
        level,
        account_id,
        pid,
        playtime,
        "LevelLog"
    );
}

/// `LogManager::CharLog` (`G/log.cpp:89-115`).
pub fn char_log(who: u32, pos: (i32, i32), what: u64, how: &str, hint: &str, ip: &str) {
    let (x, y) = pos;
    tracing::info!(
        target: "gamelog",
        r#type = "CHARACTER",
        who,
        x,
        y,
        what,
        how,
        hint,
        ip,
        "CharLog"
    );
}

/// `LogManager::QuestRewardLog` (`G/log.cpp:305-314`).
pub fn quest_reward_log(quest_name: &str, pid: u32, level: u8, value1: i64, value2: i64) {
    tracing::info!(
        target: "gamelog",
        quest_name,
        pid,
        level,
        value1,
        value2,
        "QuestRewardLog"
    );
}

/// `LogManager::TopPlayerAffLog` (`G/log.cpp:375-379`).
pub fn top_player_aff_log(pid: u32, level: u8, affect: u32) {
    tracing::info!(
        target: "gamelog",
        pid,
        level,
        affect,
        "TopPlayerAffLog"
    );
}

/// `LogManager::ItemLog` (`G/log.cpp:52-86`).
pub fn item_log(who: u32, pos: (i32, i32), item: (u32, u32), how: &str, hint: &str, ip: &str) {
    let (x, y) = pos;
    let (what, vnum) = item;
    tracing::info!(
        target: "gamelog",
        r#type = "ITEM",
        who,
        x,
        y,
        what,
        how,
        hint,
        ip,
        vnum,
        "ItemLog"
    );
}

/// `LogManager::MoneyLog` (`G/log.cpp:124-137`) for the five kept types.
pub fn money_log(kind: MoneyLogKind, vnum: u32, gold: i64) {
    tracing::info!(
        target: "gamelog",
        r#type = kind as u8,
        vnum,
        gold,
        "MoneyLog"
    );
}

/// `LogManager::GoldBarLog` (`G/log.cpp:176-218`).
pub fn gold_bar_log(pid: u32, what: u32, how: GoldBarHow, hint: &str) {
    tracing::info!(
        target: "gamelog",
        pid,
        what,
        how = how.as_str(),
        hint,
        "GoldBarLog"
    );
}

/// `LogManager::RefineLog` (`G/log.cpp:254-261`).
pub fn refine_log(
    pid: u32,
    item_name: &str,
    item_id: u32,
    step: i32,
    is_success: bool,
    set_type: &str,
) {
    tracing::info!(
        target: "gamelog",
        pid,
        item_name,
        item_id,
        step,
        is_success = u8::from(is_success),
        set_type,
        "RefineLog"
    );
}

/// `LogManager::SashLog` (`G/log.cpp:369-373`).
pub fn sash_log(
    pid: u32,
    pos: (i32, i32),
    item: (u32, u32, u16),
    item_abs_chance: u8,
    success: bool,
) {
    let (x, y) = pos;
    let (item_vnum, item_uid, item_count) = item;
    tracing::info!(
        target: "gamelog",
        pid,
        x,
        y,
        item_vnum,
        item_uid,
        item_count,
        item_abs_chance,
        success = u8::from(success),
        "SashLog"
    );
}

/// `LogManager::DragonSlayLog` (`G/log.cpp:343-350`).
pub fn dragon_slay_log(guild_id: u32, dragon_vnum: u32, start_time: u32, end_time: u32) {
    tracing::info!(
        target: "gamelog",
        guild_id,
        dragon_vnum,
        start_time,
        end_time,
        "DragonSlayLog"
    );
}

/// `LogManager::MeleyLog` (`G/log.cpp:384-409`).
pub fn meley_log(guild_id: u32, participants: u32, time: u32) {
    tracing::info!(
        target: "gamelog",
        guild_id,
        participants,
        time,
        "MeleyLog"
    );
}

/// `LogManager::OkayEventLog` (`G/log.cpp:61-66`).
pub fn okay_event_log(pid: u32, name: &str, points: i32) {
    tracing::info!(
        target: "gamelog",
        pid,
        name,
        points,
        "OkayEventLog"
    );
}

/// `LogManager::ShoutLog` (`G/log.cpp:264-269`).
pub fn shout_log(channel: u8, empire: u8, text: &str) {
    tracing::info!(
        target: "gamelog",
        channel,
        empire,
        text,
        "ShoutLog"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::*;

    type CapturedEvents = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    struct Capture {
        events: CapturedEvents,
    }

    struct FieldMap(Vec<(String, String)>);

    impl Visit for FieldMap {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .push((field.name().to_string(), format!("{value:?}")));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().to_string(), value.to_string()));
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "gamelog"
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = FieldMap(Vec::new());
            event.record(&mut fields);
            let msg = fields
                .0
                .iter()
                .find(|(k, _)| k == "message")
                .map_or_default(|(_, v)| v.clone());
            self.events
                .lock()
                .expect("capture lock")
                .push((msg, fields.0));
        }

        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    #[test]
    fn all_twenty_kept_log_kinds_emit_on_the_gamelog_target() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Capture {
            events: Arc::clone(&events),
        };
        tracing::subscriber::with_default(subscriber, || {
            hack_log("alice", "Alpha", "127.0.0.1", "ch1", "TEST");
            speed_hack_log(1, 100, 200, 3);
            gm_command_log(1, "127.0.0.1", 1, "Alpha", "level 50");
            login_log(true, 1, 10, 1, 50, 0, 12);
            detail_login_log(true, true, 1, 10, 1, "127.0.0.1", "5.4");
            boot_log("127.0.0.1", 1);
            change_name_log(1, "Old", "New", "127.0.0.1");
            level_log("Alpha", 50, 10, 1, 12);
            char_log(1, (100, 200), 0, "WARP", "map=1", "127.0.0.1");
            quest_reward_log("q", 1, 50, 100, 1);
            top_player_aff_log(1, 120, 1);
            item_log(
                1,
                (100, 200),
                (42, 10),
                "REFINE SUCCESS",
                "Sword",
                "127.0.0.1",
            );
            money_log(MoneyLogKind::Refine, 10, -2000);
            gold_bar_log(1, 42, GoldBarHow::ShopBuy, "80003");
            refine_log(1, "Sword+0", 42, 0, true, "POWER");
            sash_log(1, (100, 200), (85001, 42, 1), 15, true);
            dragon_slay_log(7, 2493, 100, 200);
            meley_log(7, 4, 300);
            okay_event_log(1, "Alpha", 400);
            shout_log(1, 1, "hello");
        });
        let captured = events.lock().expect("capture lock");
        let names: Vec<&str> = captured.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "HackLog",
                "SpeedHackLog",
                "GMCommandLog",
                "LoginLog",
                "DetailLoginLog",
                "BootLog",
                "ChangeNameLog",
                "LevelLog",
                "CharLog",
                "QuestRewardLog",
                "TopPlayerAffLog",
                "ItemLog",
                "MoneyLog",
                "GoldBarLog",
                "RefineLog",
                "SashLog",
                "DragonSlayLog",
                "MeleyLog",
                "OkayEventLog",
                "ShoutLog",
            ]
        );
        assert!(is_gold_bar(80_003) && is_gold_bar(80_007) && !is_gold_bar(80_002));
    }
}
