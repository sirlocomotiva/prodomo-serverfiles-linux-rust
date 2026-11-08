//! The command interpreter: `interpret_command` (`G/cmd.cpp:610-717`), which runs a chat line
//! that starts with `/`.
//!
//! [`interpret`](crate::command::interpret) is legacy's walk. The anti-flood check
//! (`ENABLE_ANTI_CMD_FLOOD`) counts every line of a character that is not a GM, and the tenth
//! within a second disconnects it. Then the line is copied with each `$` doubled (`double_dollar`),
//! its first word is lowered (`first_cmd`), and the walk takes the first entry of the table
//! (`table::COMMANDS`) the word names: a `do_cmd` entry by its whole name, every other entry by a
//! prefix, so an empty word names `who`, the first. The character's position is checked against the
//! entry's before the walk's end is, so a position refusal answers a word that names nothing too;
//! then a word that names nothing, and an entry above the character's GM level, answer the
//! does-not-exist line.
//!
//! The entries this build ports are [`Command`](crate::command::Command)'s; every other entry
//! is [`Command::NotPorted`](crate::command::Command::NotPorted), which the caller logs and
//! answers with nothing.
//!
//! # Divergences
//!
//! - **A command this build does not port answers nothing.** Legacy runs its handler.

use crate::game_loop_messages::{EventLine, ExitKind, GmLine, HorseLine, RankLine, RestartKind};

mod table;

/// `POS_DEAD` (`G/char.h:360`).
pub const POS_DEAD: u8 = 0;
/// `POS_SLEEPING`.
pub const POS_SLEEPING: u8 = 1;
/// `POS_RESTING`.
pub const POS_RESTING: u8 = 2;
/// `POS_SITTING`.
pub const POS_SITTING: u8 = 3;
/// `POS_FISHING`.
pub const POS_FISHING: u8 = 4;
/// `POS_FIGHTING`.
pub const POS_FIGHTING: u8 = 5;
/// `POS_MOUNTING`.
pub const POS_MOUNTING: u8 = 6;
/// `POS_STANDING`: the position of a character of this build that is not sitting; it tracks
/// no other yet.
pub const POS_STANDING: u8 = 7;

/// `GM_PLAYER` (`common/length.h:447`).
pub const GM_PLAYER: u8 = 0;
/// `GM_LOW_WIZARD`.
pub const GM_LOW_WIZARD: u8 = 1;
/// `GM_WIZARD`.
pub const GM_WIZARD: u8 = 2;
/// `GM_HIGH_WIZARD`.
pub const GM_HIGH_WIZARD: u8 = 3;
/// `GM_GOD`.
pub const GM_GOD: u8 = 4;
/// `GM_IMPLEMENTOR`.
pub const GM_IMPLEMENTOR: u8 = 5;
/// `GM_DISABLE`: no level runs the entry.
pub const GM_DISABLE: u8 = 6;

/// The line for a word that names no command, or a command above the character's level.
pub const NO_SUCH_COMMAND: &str = "@@(cmd.cpp)tradus:This command does not exist.";

/// `do_inputall`'s line: the command has to be typed in full.
pub const TYPE_IN_FULL: &str = "[LS;916]";

/// The Pulses a flood count lasts: `PASSES_PER_SEC(1)`.
pub const FLOOD_PULSES: u64 = 25;

/// The count of lines within [`FLOOD_PULSES`] that disconnects.
pub const FLOOD_LIMIT: u32 = 10;

/// `first_cmd`'s buffer, less its terminator.
const WORD_MAX: usize = 128;

/// `double_dollar`'s buffer, less its terminator.
const LINE_MAX: usize = 256;

/// `one_argument`'s `char arg[256]`, less its terminator.
const ARGUMENT_MAX: usize = 255;

/// One entry of `cmd_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandEntry {
    /// The name.
    pub name: &'static str,
    /// The handler legacy calls, by its function name.
    pub handler: &'static str,
    /// The lowest position that runs it.
    pub position: u8,
    /// The lowest GM level that runs it; 0 for every character.
    pub gm_level: u8,
}

/// A table row.
const fn entry(
    name: &'static str,
    handler: &'static str,
    position: u8,
    gm_level: u8,
) -> CommandEntry {
    CommandEntry {
        name,
        handler,
        position,
        gm_level,
    }
}

impl CommandEntry {
    /// Whether the walk takes only the whole name: `do_cmd`'s entries, and `restart_now`,
    /// which legacy's prefix walk gives every word from `rest` to `restart_n`, as it stands
    /// before `restart_here` (B8). A typo would otherwise pay 300 000 gold.
    fn exact(&self) -> bool {
        matches!(self.handler, "do_cmd" | "do_restart_now")
    }

    /// What this build runs for the entry.
    #[must_use]
    pub fn command(&self) -> Command {
        match self.handler {
            "do_inputall" => Command::TypeInFull,
            "do_shutdown" => Command::Shutdown,
            "do_click_safebox"
            | "do_click_mall"
            | "do_safebox_close"
            | "do_safebox_size"
            | "do_safebox_password"
            | "do_safebox_change_password"
            | "do_mall_password"
            | "do_mall_close" => Self::store_name(self.handler),
            "do_restart" | "do_restart_now" | "do_purge" | "do_weaken" => self.restart_name(),
            "do_cmd" => self.exit_name(),
            "do_skillup" => Command::SkillUp,
            "do_pvp" => Command::Pvp,
            "do_guildskillup"
            | "do_war"
            | "do_nowar"
            | "do_stop_guild_war"
            | "do_cancel_guild_war"
            | "do_gwlist"
            | "do_guild_state"
            | "do_makeguild"
            | "do_deleteguild" => Self::guild_name(self.handler),
            "do_party_request" => Command::PartyRequest,
            "do_party_request_accept" => Command::PartyRequestAccept,
            "do_party_request_deny" => Command::PartyRequestDeny,
            "do_ungroup" => Command::Ungroup,
            "do_messenger_auth" => Command::MessengerAuth,
            "do_notice" => Command::GlobalNotice,
            "do_map_notice" => Command::MapNotice,
            "do_big_notice" => Command::BigNotice,
            "do_setblockmode" => Command::SetBlockMode,
            "do_block_chat" => Command::BlockChat,
            "do_block_chat_list" => Command::BlockChatList,
            "do_vote_block_chat" => Command::VoteBlockChat,
            "do_disconnect" | "do_transfer" | "do_kill" | "do_poison" | "do_stun" => {
                Self::strike_name(self.handler)
            }
            "do_getqf" | "do_setqf" | "do_delqf" | "do_set_state" | "do_qf" | "do_clear_quest" => {
                Self::quest_name(self.handler)
            }
            "do_horse_state" => Command::Horse(HorseLine::State),
            "do_horse_level" => Command::Horse(HorseLine::Level),
            "do_horse_set_stat" => Command::Horse(HorseLine::SetStat),
            "do_horse_summon" => Command::Horse(HorseLine::Summon),
            "do_horse_unsummon" => Command::Horse(HorseLine::Unsummon),
            "do_user_horse_back" => Command::Horse(HorseLine::Back),
            "do_horse_ride" => Command::Horse(HorseLine::Ride),
            "do_user_horse_ride" => Command::Horse(HorseLine::UserRide),
            "do_unmount" => Command::Horse(HorseLine::Unmount),
            "do_mount_test" => Command::Horse(HorseLine::MountTest),
            "do_ride" => Command::Ride,
            "do_cards" => Command::Cards,
            "do_open_global_rank" => Command::RankLine(RankLine::Open),
            "do_global_rank_end_season" => Command::RankLine(RankLine::EndSeason),
            _ => self.mid_name(),
        }
    }

    /// The mid-table lines from the view to the end: split out so
    /// [`CommandEntry::command`] does not trip the line lint.
    fn mid_name(&self) -> Command {
        if Self::is_tail_line(self.handler) {
            return Self::tail_name(self.handler);
        }
        match self.handler {
            "do_view_equip" => Command::ViewEquip,
            "do_warp" => Command::Warp,
            "do_observer_exit" => Command::ObserverExit,
            "do_private" => Command::Private,
            "do_reload" => Command::Reload,
            "do_rewarp" => Command::Rewarp,
            "do_goto" => Command::Goto,
            "do_level" => Command::Level,
            "do_advance" => Command::Advance,
            "do_reset" => Command::Reset,
            "do_greset" => Command::GuildReset,
            "do_user" => Command::OnlineList,
            "do_who" => Command::Who,
            "do_respawn" => Command::Respawn,
            "do_mob" => Command::Mob,
            "do_mob_ld" => Command::MobLd,
            "do_mob_aggresive" => Command::MobAggressive,
            "do_mob_coward" => Command::MobCoward,
            "do_mob_map" => Command::MobMap,
            "do_detaillog" => Command::DetailLog,
            "do_monsterlog" => Command::MonsterLog,
            "do_polymorph" => Command::Polymorph,
            "do_polymorph_item" => Command::Polyitem,
            "do_socket_item" => Command::SocketItem,
            "do_save_attribute_to_image" => Command::SaveAti,
            "do_get_item_id_list"
            | "do_get_mob_count"
            | "do_effect"
            | "do_add_socket"
            | "do_set_socket"
            | "do_change_attr"
            | "do_add_attr"
            | "do_change_rare_attr"
            | "do_add_rare_attr"
            | "do_force_logout"
            | "do_reset_subskill"
            | "do_conqueror_level"
            | "do_conqueror_point"
            | "do_sort_inventory"
            | "do_pickup_affect"
            | "do_move_channel"
            | "do_flush"
            | "do_observer"
            | "do_user_horse_feed"
            | "do_oxevent_show_quiz"
            | "do_gaya_system"
            | "do_cube"
            | "do_fetch_casket_items"
            | "do_break_marriage"
            | "do_close_shop"
            | "do_check_monarch_money" => self.gm_tail_name(),
            "do_forgetme" => Command::ForgetMe,
            "do_aggregate" => Command::Aggregate,
            "do_attract_ranger" => Command::AttractRanger,
            "do_pull_monster" => Command::PullMonster,
            "do_group" => Command::Group,
            "do_group_random" => Command::GroupRandom,
            "do_item" => Command::SelfGrant,
            "do_setskill" => Command::SetSkill,
            "do_setskillother" => Command::SetSkillOther,
            "do_set_skill_point" => Command::SetSkillPoint,
            "do_set_skill_group" => Command::SetJob,
            "do_invisibility" => Command::Invisible,
            "do_cooltime" => Command::Cooltime,
            "do_affect_remove" => Command::AffectRemove,
            "do_hide_costume" => Command::HideCostume,
            "do_emotion_allow" | "do_emotion" => self.emotion_name(),
            "do_priv_empire" => Command::PrivEmpire,
            "do_event_flag" | "do_get_event_flag" | "do_daily_gift_event" => self.flag_name(),
            "do_xmas" | "do_eclipse" | "do_event_helper" => self.event_name(),
            "give_reward" | "letters_open" => self.reward_name(),
            "do_stat" => Command::Stat,
            "do_stat_minus" => Command::StatMinus,
            "do_stat_val" => Command::StatVal,
            "do_stat_plus_amount" | "do_set_stat" => self.stat_line(),
            "do_stat_reset" => Command::StatReset,
            "do_can_dead" | "do_pkmode" => self.pk_line(),
            "do_set_walk_mode" | "do_set_run_mode" => Self::mode_name(self.handler),
            _ => Command::NotPorted(self.name),
        }
    }
    /// The reward and letter openers: split out so `mid_name` stays under the line lint.
    fn reward_name(&self) -> Command {
        match self.handler {
            "give_reward" => Command::GiveReward,
            _ => Command::LettersOpen,
        }
    }

    /// The event-flag lines: split out so `mid_name` stays under the line lint.
    fn flag_name(&self) -> Command {
        match self.handler {
            "do_event_flag" => Command::EventFlag,
            "do_get_event_flag" => Command::GetEventFlag,
            _ => Command::DailyGiftEvent,
        }
    }

    /// The tail GM lines: split out so `mid_name` stays under the line lint.
    fn gm_tail_name(&self) -> Command {
        match self.handler {
            "do_get_item_id_list" => Command::GmLine(GmLine::ItemIdList),
            "do_get_mob_count" => Command::GmLine(GmLine::MobCount),
            "do_effect" => Command::GmLine(GmLine::Effect),
            "do_add_socket" => Command::GmLine(GmLine::AddSocket),
            "do_set_socket" => Command::GmLine(GmLine::SetSocket),
            "do_change_attr" => Command::GmLine(GmLine::ChangeAttr),
            "do_add_attr" => Command::GmLine(GmLine::AddAttr),
            "do_change_rare_attr" => Command::GmLine(GmLine::ChangeRareAttr),
            "do_add_rare_attr" => Command::GmLine(GmLine::AddRareAttr),
            "do_force_logout" => Command::GmLine(GmLine::ForceLogout),
            "do_reset_subskill" => Command::GmLine(GmLine::ResetSubSkill),
            "do_conqueror_level" => Command::GmLine(GmLine::ConquerorLevel),
            "do_conqueror_point" => Command::GmLine(GmLine::ConquerorPoint),
            "do_sort_inventory" => Command::GmLine(GmLine::SortInventory),
            "do_pickup_affect" => Command::GmLine(GmLine::PickupAffect),
            "do_move_channel" => Command::GmLine(GmLine::MoveChannel),
            "do_flush" => Command::GmLine(GmLine::Flush),
            "do_observer" => Command::GmLine(GmLine::Observer),
            "do_user_horse_feed" => Command::GmLine(GmLine::UserHorseFeed),
            "do_oxevent_show_quiz" => Command::GmLine(GmLine::ShowQuiz),
            "do_gaya_system" => Command::GmLine(GmLine::GayaSystem),
            "do_cube" => Command::GmLine(GmLine::Cube),
            "do_fetch_casket_items" => Command::GmLine(GmLine::FetchCasket),
            "do_break_marriage" => Command::GmLine(GmLine::BreakMarriage),
            "do_close_shop" => Command::GmLine(GmLine::CloseShop),
            "do_check_monarch_money" => Command::GmLine(GmLine::CheckMmoney),
            _ => Command::NotPorted(self.handler),
        }
    }

    /// Whether `handler` is one of the tail lines after the walk-mode ones.
    fn is_tail_line(handler: &str) -> bool {
        matches!(
            handler,
            "do_all_skill_master"
                | "do_full_set"
                | "do_item_full_set"
                | "do_attr_full_set"
                | "do_refine_rod"
                | "do_refine_pick"
                | "do_max_pick"
                | "do_use_item"
                | "do_clear_affect"
                | "do_dragon_soul"
                | "do_ds_list"
                | "do_ds_change_attr"
                | "do_dice"
                | "do_multi_farm"
                | "do_gift"
                | "do_hair"
                | "do_fishing_simul"
                | "do_costume"
                | "do_state"
                | "do_inventory"
                | "do_slow"
                | "do_item_purge"
                | "do_book"
                | "do_console"
                | "do_set"
                | "do_advance_skill_select"
                | "do_click_special_inv"
                | "do_temp"
        )
    }

    /// The tail lines after the walk-mode ones: split out so [`CommandEntry::command`]
    /// does not trip the line lint.
    fn tail_name(handler: &'static str) -> Command {
        match handler {
            "do_all_skill_master"
            | "do_full_set"
            | "do_item_full_set"
            | "do_attr_full_set"
            | "do_refine_rod"
            | "do_refine_pick"
            | "do_max_pick"
            | "do_use_item"
            | "do_clear_affect"
            | "do_dragon_soul"
            | "do_ds_list"
            | "do_ds_change_attr"
            | "do_dice"
            | "do_multi_farm"
            | "do_gift"
            | "do_hair"
            | "do_fishing_simul"
            | "do_costume"
            | "do_state"
            | "do_inventory" => Self::kit_name(handler),
            "do_slow" | "do_item_purge" | "do_book" | "do_console" => Self::purge_name(handler),
            "do_set" => Command::SetAttr,
            "do_advance_skill_select" => Command::AdvanceSkill,
            "do_click_special_inv" => Command::ClickSpecialInv,
            "do_temp" => Command::Temp,
            _ => Command::NotPorted(handler),
        }
    }

    /// The safebox and mall lines: split out so the match does not trip the line lint.
    fn store_name(handler: &'static str) -> Command {
        match handler {
            "do_click_safebox" => Command::ClickSafebox,
            "do_click_mall" => Command::ClickMall,
            "do_safebox_close" => Command::SafeboxClose,
            "do_safebox_size" => Command::SafeboxSize,
            "do_safebox_password" => Command::SafeboxPassword,
            "do_safebox_change_password" => Command::SafeboxChangePassword,
            "do_mall_password" => Command::MallPassword,
            "do_mall_close" => Command::MallClose,
            _ => Command::NotPorted(handler),
        }
    }

    /// The guild management lines: split out so the match does not trip the line lint.
    fn guild_name(handler: &'static str) -> Command {
        match handler {
            "do_guildskillup" => Command::GuildSkillUp,
            "do_war" => Command::GuildWar,
            "do_nowar" => Command::GuildNoWar,
            "do_stop_guild_war" => Command::GuildWarStop,
            "do_cancel_guild_war" => Command::GuildWarCancel,
            "do_gwlist" => Command::GuildWarList,
            "do_guild_state" => Command::GuildState,
            "do_makeguild" => Command::Makeguild,
            "do_deleteguild" => Command::Deleteguild,
            _ => Command::NotPorted(handler),
        }
    }

    /// The disconnect, transfer, kill, poison and stun lines: split out so the match does
    /// not trip the line lint.
    fn strike_name(handler: &'static str) -> Command {
        match handler {
            "do_disconnect" => Command::Dc,
            "do_transfer" => Command::Transfer,
            "do_kill" => Command::Kill,
            "do_poison" => Command::Poison,
            "do_stun" => Command::Stun,
            _ => Command::NotPorted(handler),
        }
    }

    /// The restart, purge and weaken lines: split out so the match does not trip the
    /// line lint.
    fn restart_name(&self) -> Command {
        match self.handler {
            "do_restart" if self.name == "restart_town" => Command::Restart(RestartKind::Town),
            "do_restart" => Command::Restart(RestartKind::Here),
            "do_restart_now" => Command::Restart(RestartKind::Now),
            "do_purge" => Command::Purge,
            "do_weaken" => Command::Weaken,
            _ => Command::NotPorted(self.name),
        }
    }

    /// The event lines: `do_xmas`'s three sub-commands by name, `do_eclipse` and
    /// `do_event_helper`.
    fn event_name(&self) -> Command {
        match (self.handler, self.name) {
            ("do_xmas", "xmas_boom") => Command::EventLine(EventLine::XmasBoom),
            ("do_xmas", "xmas_snow") => Command::EventLine(EventLine::XmasSnow),
            ("do_xmas", "xmas_santa") => Command::EventLine(EventLine::XmasSanta),
            ("do_eclipse", _) => Command::EventLine(EventLine::Eclipse),
            ("do_event_helper", _) => Command::EventLine(EventLine::EventHelper),
            _ => Command::NotPorted(self.name),
        }
    }

    /// The exit lines: split out so the match does not trip the line lint.
    fn exit_name(&self) -> Command {
        match self.name {
            "logout" => Command::Exit(ExitKind::Logout),
            "quit" => Command::Exit(ExitKind::Quit),
            "phase_select" => Command::Exit(ExitKind::PhaseSelect),
            _ => Command::NotPorted(self.name),
        }
    }

    /// The quest-flag lines: split out so the match does not trip the line lint.
    fn quest_name(handler: &'static str) -> Command {
        match handler {
            "do_getqf" => Command::GetQf,
            "do_setqf" => Command::SetQf,
            "do_delqf" => Command::DelQf,
            "do_set_state" => Command::SetState,
            "do_qf" => Command::Qf,
            "do_clear_quest" => Command::ClearQuest,
            _ => Command::NotPorted(handler),
        }
    }

    /// The master, kit, use, clear, soul, dice, gift, hair, costume, state and
    /// inventory lines: split out so the match does not trip the line lint.
    fn kit_name(handler: &'static str) -> Command {
        match handler {
            "do_all_skill_master" => Command::AllSkillMaster,
            "do_full_set" => Command::FullSet,
            "do_item_full_set" => Command::ItemFullSet,
            "do_refine_rod" => Command::RefineRod,
            "do_refine_pick" => Command::RefinePick,
            "do_max_pick" => Command::MaxPick,
            "do_attr_full_set" => Command::AttrFullSet,
            "do_use_item" => Command::UseItem,
            "do_clear_affect" => Command::ClearAffect,
            "do_dragon_soul" => Command::DragonSoul,
            "do_ds_list" => Command::DsList,
            "do_ds_change_attr" => Command::DsChangeAttr,
            "do_dice" => Command::Dice,
            "do_multi_farm" => Command::MultiFarm,
            "do_gift" => Command::Gift,
            "do_hair" => Command::Hair,
            "do_fishing_simul" => Command::FishSimul,
            "do_costume" => Command::Costume,
            "do_state" => Command::State,
            "do_inventory" => Command::Inventory,
            _ => Command::NotPorted(handler),
        }
    }

    /// The `*+`/`t*` and `set_stat` lines: split out so the match does not trip the
    /// line lint.
    fn stat_line(&self) -> Command {
        match self.handler {
            "do_stat_plus_amount" => Self::stat_name(self.name, Command::StatPlus),
            "do_set_stat" => Self::stat_name(self.name, Command::SetStat),
            _ => Command::NotPorted(self.handler),
        }
    }

    /// The can-dead and PK-mode lines: split out so the match does not trip the line
    /// lint.
    fn pk_line(&self) -> Command {
        match self.handler {
            "do_can_dead" => Command::CanDead(self.name == "can_dead"),
            "do_pkmode" => Command::PkMode,
            _ => Command::NotPorted(self.handler),
        }
    }

    /// The emotion lines: split out so the match does not trip the line lint.
    fn emotion_name(&self) -> Command {
        match self.handler {
            "do_emotion_allow" => Command::EmotionAllow,
            "do_emotion" => Command::Emotion,
            _ => Command::NotPorted(self.handler),
        }
    }

    /// The slow, purge, book and console lines: split out so the match does not trip
    /// the line lint.
    fn purge_name(handler: &'static str) -> Command {
        match handler {
            "do_slow" => Command::Slow,
            "do_item_purge" => Command::ItemPurge,
            "do_book" => Command::Book,
            "do_console" => Command::Console,
            _ => Command::NotPorted(handler),
        }
    }

    /// The walk and run lines: split out so the match does not trip the line lint.
    fn mode_name(handler: &'static str) -> Command {
        match handler {
            "do_set_walk_mode" => Command::WalkMode(true),
            "do_set_run_mode" => Command::WalkMode(false),
            _ => Command::NotPorted(handler),
        }
    }

    /// The `*+` and `t*` attribute the entry names: the `con`/`tcon` pair
    /// names `Ht`, and so on. Split out so the match does not trip the line
    /// lint.
    fn stat_name(name: &'static str, wrap: fn(StatPlusKind) -> Command) -> Command {
        let kind = match name {
            "con+" | "tcon" => StatPlusKind::Ht,
            "int+" | "tint" => StatPlusKind::Iq,
            "str+" | "tstr" => StatPlusKind::St,
            "dex+" | "tdex" => StatPlusKind::Dx,
            _ => return Command::NotPorted(name),
        };
        wrap(kind)
    }
}

/// A command this build runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `do_inputall`: [`TYPE_IN_FULL`].
    TypeInFull,
    /// `do_shutdown`: the 10-second shutdown countdown starts
    /// (`G/cmd_general.cpp:265-275`).
    Shutdown,
    /// `do_click_safebox`: the client is told to ask for the safebox password.
    ClickSafebox,
    /// `do_click_mall`: the client is told to ask for the mall password.
    ClickMall,
    /// `do_safebox_close`.
    SafeboxClose,
    /// `do_safebox_size`: the GM's size word.
    SafeboxSize,
    /// `do_safebox_password`: open the safebox.
    SafeboxPassword,
    /// `do_safebox_change_password`.
    SafeboxChangePassword,
    /// `do_mall_password`: open the mall.
    MallPassword,
    /// `do_mall_close`.
    MallClose,
    /// `do_restart` (`restart_here` and `restart_town`) and `do_restart_now`: the restart of the
    /// entry's kind.
    Restart(RestartKind),
    /// `do_cmd` (`logout`, `quit` and `phase_select`): the exit of the entry's kind.
    Exit(ExitKind),
    /// `do_skillup`: the `/skillup` line's vnum climbs a level when every gate passes.
    SkillUp,
    /// `do_pvp`: the `/pvp` line's VID is challenged or agreed with.
    Pvp,
    /// `do_guildskillup`: the `/gskillup` line's vnum climbs a level when every gate
    /// passes.
    GuildSkillUp,
    /// `do_war`: the `/war` line's guild is declared on when every gate passes.
    GuildWar,
    /// `do_nowar`: the `/nowar` line's guild is refused when a received declare holds.
    GuildNoWar,
    /// `do_stop_guild_war`: the `gwstop` line's pair ends in a forced draw.
    GuildWarStop,
    /// `do_cancel_guild_war`: the `gwcancel` line's pair ends with no ladder math.
    GuildWarCancel,
    /// `do_gwlist`: the GM hears each live guild war's pair and age.
    GuildWarList,
    /// `do_guild_state`: the GM hears the named guild's id, master and war.
    GuildState,
    /// `do_makeguild`: the GM founds a guild under the line's word.
    Makeguild,
    /// `do_deleteguild`: the caller's guild disbands when it leads and no war holds.
    Deleteguild,
    /// `do_party_request`: the requester asks the VID's party leader to take it in.
    PartyRequest,
    /// `do_party_request_accept`: the leader accepts the guest's request.
    PartyRequestAccept,
    /// `do_party_request_deny`: the leader denies the guest's request.
    PartyRequestDeny,
    /// `do_ungroup`: the caller leaves its party (`G/cmd_general.cpp:1060-1088`).
    Ungroup,
    /// `do_messenger_auth`: the target answers a friend request named by the line's
    /// second word (`G/cmd_general.cpp:2272-2296`).
    MessengerAuth,
    /// `do_notice`: every client on every Channel hears the GM's line.
    GlobalNotice,
    /// `do_setblockmode`: the `/setblockmode` line's byte becomes the character's own
    /// block mode, and the client hears `setblockmode %d` beside seven quest flags.
    SetBlockMode,
    /// `do_map_notice`: every client on the GM's Channel and map hears the line.
    MapNotice,
    /// `do_big_notice`: the GM hears its own line in the big font.
    BigNotice,
    /// `do_block_chat`: the named character is silenced for the parsed time, or
    /// unsilenced when the time is `0` (`G/cmd_gm.cpp:3364-3433`).
    BlockChat,
    /// `do_block_chat_list`: the caller hears one line per silenced character,
    /// or `No one currently blocked` (`G/cmd_gm.cpp:3298-3310`).
    BlockChatList,
    /// `do_vote_block_chat`: nothing; the body returns before its first word
    /// (`G/cmd_gm.cpp:3312-3314`).
    VoteBlockChat,
    /// `do_disconnect`: the named character's descriptor closes
    /// (`G/cmd_gm.cpp:1755-1782`).
    Dc,
    /// `do_transfer`: the named character warps to the caller's side
    /// (`G/cmd_gm.cpp:102-151`).
    Transfer,
    /// `do_kill`: the named character dies (`G/cmd_gm.cpp:1784-1805`).
    Kill,
    /// `do_poison`: the named character is poisoned (`G/cmd_gm.cpp:1808-1829`).
    Poison,
    /// `do_stun`: the named character is stunned for 30 ticks (`G/cmd_gm.cpp:87-90`,
    /// through `Command_ApplyAffect`, `:50-83`).
    Stun,
    /// `do_getqf`: the target's quest flags, one line each
    /// (`G/cmd_gm.cpp:2863-2888`).
    GetQf,
    /// `do_setqf`: sets the target's quest flag (`G/cmd_gm.cpp:2958-2992`).
    SetQf,
    /// `do_delqf`: deletes the target's quest flag (`G/cmd_gm.cpp:2994-3027`).
    DelQf,
    /// `do_set_state`: the target's quest moves to the named state
    /// (`G/cmd_gm.cpp:2891-2956`).
    SetState,
    /// `do_qf`: the caller's current quest moves to the named state
    /// (`G/cmd_gm.cpp:2326-2364`).
    Qf,
    /// `do_clear_quest`: the caller's quest forgets every flag and rests at 0
    /// (`G/cmd_gm.cpp:3732-3743`).
    ClearQuest,
    /// `do_view_equip`: the caller views the VID's worn equipment
    /// (`G/cmd_general.cpp:2395-2426`).
    ViewEquip,
    /// `do_warp`: the caller warps to the named character or the metres
    /// (`G/cmd_gm.cpp:399-457`).
    Warp,
    /// `do_private`: the caller warps to a private copy of the named map
    /// (`G/cmd_gm.cpp:2301-2325`).
    Private,
    /// `do_reload`: the named tables reload (`G/cmd_gm.cpp:2639-2733`).
    Reload,
    /// `do_rewarp`: the caller rewarps in place (`G/cmd_gm.cpp:459-465`).
    Rewarp,
    /// `do_observer_exit`: an observer leaves the mode, warping to the saved place
    /// (`G/cmd_general.cpp:2370-2393`).
    ObserverExit,
    /// `do_goto`: the caller shows at the metres or warps to the named map
    /// (`G/cmd_gm.cpp:313-397`).
    Goto,
    /// `do_level`: the caller's level becomes the line's, clamped to the max
    /// (`G/cmd_gm.cpp:2740-2758`).
    Level,
    /// `do_advance`: the named character's level becomes the line's, clamped
    /// to `0..=max` (`G/cmd_gm.cpp:2083-2106`).
    Advance,
    /// `do_reset`: the caller's hit and spell pools refill, and the filled
    /// pools bank (`G/cmd_gm.cpp:2076-2081`).
    Reset,
    /// `do_greset`: the caller's guild's faith refills to its maximum
    /// (`G/cmd_gm.cpp:2175-2179`).
    GuildReset,
    /// `do_user`: the online Names roll past the caller, four a line, then
    /// the total (`G/cmd_gm.cpp:1694-1753`).
    OnlineList,
    /// `do_who`: the empires' totals and the caller's Channel's own count
    /// (`G/cmd_gm.cpp:1682-1692`).
    Who,
    /// `do_respawn`: the regens around fire one Pulse on, everywhere with
    /// `all` (`G/cmd_gm.cpp:2108-2123`).
    Respawn,
    /// `do_mob`: the named mob stands around the caller (`G/cmd_gm.cpp:699-758`).
    Mob,
    /// `do_mob_ld`: the named mob at the named metres (`G/cmd_gm.cpp:760-815`).
    MobLd,
    /// `do_mob_aggresive`: the named mob stands around the caller with the
    /// aggressive bit (`G/cmd_gm.cpp:639-698`).
    MobAggressive,
    /// `do_mob_coward`: the named mob stands around the caller with the
    /// coward bit (`G/cmd_gm.cpp:557-617`).
    MobCoward,
    /// `do_mob_map`: the numbered mob at a random point of the caller's map
    /// (`G/cmd_gm.cpp:618-637`).
    MobMap,
    /// `do_detaillog`: the caller's detail flag flips
    /// (`G/cmd_general.cpp:2242-2245`).
    DetailLog,
    /// `do_monsterlog`: the caller's monster subscription flips
    /// (`G/cmd_general.cpp:2247-2250`).
    MonsterLog,
    /// `do_polymorph`: the caller shows as the named mob vnum, or unmorphs on an
    /// empty or job vnum (`G/cmd_gm.cpp:3049-3068`).
    Polymorph,
    /// `do_polymorph_item`: the caller is granted a 70104 ball holding the named mob
    /// vnum in its first socket (`G/cmd_gm.cpp:3070-3104`).
    Polyitem,
    /// `do_socket_item`: the caller is given one item of the named vnum with its first
    /// sockets set to 1, three by default (`G/cmd_gm.cpp:3231-3268`).
    SocketItem,
    /// `do_save_attribute_to_image`: the named map's attributes are written as a TGA to
    /// the named file (`G/cmd_gm.cpp:3843-3863`).
    SaveAti,
    /// `do_get_item_id_list`, `do_get_mob_count`, `do_effect`, `do_add_socket`,
    /// `do_set_socket`, `do_change_attr`, `do_add_attr`, `do_change_rare_attr`,
    /// `do_add_rare_attr`, `do_force_logout` and `do_reset_subskill`: the small GM lines
    /// the world runs whole.
    GmLine(GmLine),
    /// `do_forgetme`: every monster around whose victim is the caller drops
    /// it, then the caller hides for 5 seconds (`G/cmd_gm.cpp:3029-3032`).
    ForgetMe,
    /// `do_aggregate`: each victimless monster around within 5000, with a
    /// one-in-two draw, begins to fight the caller (`G/cmd_gm.cpp:3034-3037`).
    Aggregate,
    /// `do_attract_ranger`: each monster around whose victim is none or the
    /// caller and whose attack range is past 150 is cut to 150 for 3 minutes
    /// (`G/cmd_gm.cpp:3039-3042`).
    AttractRanger,
    /// `do_pull_monster`: each monster around 100 to 3000 out is pulled 300
    /// toward the caller (`G/cmd_gm.cpp:3044-3047`).
    PullMonster,
    /// `do_group`: the group in the 500-box around the caller
    /// (`G/cmd_gm.cpp:537-555`).
    Group,
    /// `do_group_random`: one group drawn uniformly from the group-group,
    /// spawned as `/group` spawns it (`G/cmd_gm.cpp:521-536`).
    GroupRandom,
    /// `do_item`: the named item lands in the caller's own inventory
    /// (`G/cmd_gm.cpp:467-519`).
    SelfGrant,
    /// `do_setskill`: the caller's named skill becomes the line's level
    /// (`G/cmd_gm.cpp:2574-2605`).
    SetSkill,
    /// `do_setskillother`: the named character's skill becomes the line's
    /// level (`G/cmd_gm.cpp:2528-2572`).
    SetSkillOther,
    /// `do_purge`: every NPC around leaves, `all` or near
    /// (`G/cmd_gm.cpp:816-868`).
    Purge,
    /// `do_weaken`: every NPC around falls to 10 hit points, `all` or near
    /// (`G/cmd_gm.cpp:2824-2861`).
    Weaken,
    /// `do_set_skill_point`: the caller's skill counter becomes the line's
    /// number (`G/cmd_gm.cpp:2607-2618`).
    SetSkillPoint,
    /// `do_set_skill_group`: the caller's skill group becomes the line's,
    /// then the skills clear (`G/cmd_gm.cpp:2620-2631`).
    SetJob,
    /// `do_invisibility`: the caller's invisibility toggles
    /// (`G/cmd_gm.cpp:2253-2264`).
    Invisible,
    /// `do_cooltime`: the caller's skill cooldowns stop gating
    /// (`G/cmd_gm.cpp:2735-2738`).
    Cooltime,
    /// `do_affect_remove`: the caller's affect list, or the named affect's
    /// removal (`G/cmd_gm.cpp:3865-3918`).
    AffectRemove,
    /// `do_hide_costume`: the line's part hides or shows its costume, the client hears
    /// `SetXCostumeHidden`, and the `costume_option.hide_*` quest flag follows
    HideCostume,
    /// `do_emotion_allow`: the line's VID joins the caller's consent pairs.
    EmotionAllow,
    /// `do_emotion`: the entry's emotion runs against the line's named victim.
    Emotion,
    /// `do_priv_empire`: the line's empire takes the type's rate bonus for the hours.
    PrivEmpire,
    /// `do_event_flag`: the line's name takes the line's value.
    EventFlag,
    /// `do_get_event_flag`: every flag is listed on the caller.
    GetEventFlag,
    /// `do_daily_gift_event`: the usage, or the event starts or stops.
    DailyGiftEvent,
    /// `do_xmas`, `do_eclipse` and `do_event_helper`: one named flag is set, or the
    /// firework sellers stand or go.
    EventLine(EventLine),
    /// `do_horse_state`, `do_horse_level` and `do_horse_set_stat`: the GM horse lines.
    Horse(HorseLine),
    /// `give_reward`: the six letters are traded for the box's draw.
    GiveReward,
    /// `/open_global_rank` or `/rank_end_season`: the global rank's lines.
    RankLine(RankLine),
    /// `letters_open`: the caller hears the box's table and opens the window.
    LettersOpen,
    /// `do_stat`: the line's attribute rises by 1 for a stat point.
    Stat,
    /// `do_stat_minus`: the line's attribute falls by 1 and the point returns.
    StatMinus,
    /// `do_stat_val`: the line's attribute rises by the line's amount for stat
    /// points (`G/cmd_general.cpp:3699-3750`).
    StatVal,
    /// `do_stat_plus_amount`: the entry's attribute rises by the line's
    /// amount for stat points (`G/cmd_gm.cpp:4068-4162`).
    StatPlus(StatPlusKind),
    /// `do_set_stat`: the named character's entry attribute moves by the
    /// line's delta for the named character's stat points
    /// (`G/cmd_gm.cpp:4575-4685`).
    SetStat(StatPlusKind),
    /// `do_stat_reset`: the reset count rises to 12.
    StatReset,
    /// `do_can_dead`: the caller's `cannot_dead` clears (`can_dead`) or
    /// sets (`cannot_dead`); `true` is `can_dead` (`G/cmd_gm.cpp:4715-4721`).
    CanDead(bool),
    /// `do_pkmode`: the caller's PK mode becomes the line's number.
    PkMode,
    /// `do_set_walk_mode` and `do_set_run_mode`: the caller walks or runs.
    WalkMode(bool),
    /// `do_all_skill_master`: every usable skill becomes 40 (`G/cmd_gm.cpp:4734-4758`).
    AllSkillMaster,
    /// `do_full_set`: the master line, then the kit and its attributes
    /// (`G/cmd_gm.cpp:4723-4732`).
    FullSet,
    /// `do_item_full_set`: the job's eight fresh pieces go on
    /// (`G/cmd_gm.cpp:4760-4902`).
    ItemFullSet,
    /// `do_attr_full_set`: five attributes are forced on each worn piece
    /// (`G/cmd_gm.cpp:4904-5019`).
    AttrFullSet,
    /// `do_refine_rod`: the named inventory cell's rod refines
    /// (`G/cmd_gm.cpp:2181-2192`, `fishing::RealRefineRod`, `G/fishing.cpp:861-928`).
    RefineRod,
    /// `do_refine_pick`: the named cell's pick maxes its training and refines
    /// (`G/cmd_gm.cpp:2195-2208`, `mining::CHEAT_MAX_PICK` and
    /// `mining::RealRefinePick`, `G/mining.cpp:234-315`).
    RefinePick,
    /// `do_max_pick`: the named cell's pick maxes its training
    /// (`G/cmd_gm.cpp:2210-2222`, `mining::CHEAT_MAX_PICK`, `G/mining.cpp:304-315`).
    MaxPick,
    /// `do_use_item`: the cell the line names runs through `UseItem`
    /// (`G/cmd_gm.cpp:4999-5016`).
    UseItem,
    /// `do_ride`: the ride toggle, which may use a ride item
    /// (`G/cmd_general.cpp:3516-3642`).
    Ride,
    /// `do_cards`: the Okey card game, whose arms open, pull, end, destroy, accept
    /// and restore (`G/cmd_general.cpp:3812-3864`).
    Cards,
    /// `do_clear_affect`: `ClearAffect(true)` on the caller
    /// (`G/cmd_gm.cpp:5019-5022`).
    ClearAffect,
    /// `do_dragon_soul`: the named deck activates, or every deck deactivates
    /// (`G/cmd_gm.cpp:5024-5047`).
    DragonSoul,
    /// `do_ds_list`: one line per held dragon soul stone
    /// (`G/cmd_gm.cpp:5049-5059`).
    DsList,
    /// `do_ds_change_attr`: the named soul-inventory stone's attributes redrawn
    /// (`G/cmd_general.cpp:3864-3931`).
    DsChangeAttr,
    /// `do_dice`: a rolled number to the caller, or the party
    /// (`G/cmd_general.cpp:3455-3489`).
    Dice,
    /// `do_multi_farm`: the caller's farm status toggles past its 10 s protect
    /// (`G/cmd_general.cpp:304-317`).
    MultiFarm,
    /// `do_gift`: the client opens its gift window
    /// (`G/cmd_general.cpp:3251-3254`).
    Gift,
    /// `do_hair`: the caller's hair affect as one line
    /// (`G/cmd_general.cpp:3200-3208`).
    Hair,
    /// `do_fishing_simul`: the usage, then the simulation's lines
    /// (`G/cmd_gm.cpp:2227-2251`).
    FishSimul,
    /// `do_costume`: the costume status lines, and the named piece comes off
    /// (`G/cmd_general.cpp:3089-3198`).
    Costume,
    /// `do_state`: the target's dump, told to the caller
    /// (`G/cmd_gm.cpp:1017-1202`).
    State,
    /// `do_inventory`: the caller's own cells, told to the caller
    /// (`G/cmd_general.cpp:3210-3250`).
    Inventory,
    /// `do_slow`: ten fresh 12019s, numbered in socket 4 (`G/cmd_gm.cpp:92-100`).
    Slow,
    /// `do_item_purge`: every flat and dragon soul cell cleared
    /// (`G/cmd_gm.cpp:868-1016`).
    ItemPurge,
    /// `do_book`: one fresh 50300 with the named skill in socket 0
    /// (`G/cmd_gm.cpp:2501-2526`).
    Book,
    /// `do_console`: the `ConsoleEnable` command line (`G/cmd_general.cpp:466-469`).
    Console,
    /// `do_set`: the named PC's field set (`G/cmd_gm.cpp:1862-2074`).
    SetAttr,
    /// `do_temp`: the empire's treasury moves (`G/cmd_gm.cpp:4368-4388`).
    Temp,
    /// `do_advance_skill_select`: the caller's skill group becomes the line's,
    /// then the skills clear (`G/cmd_general.cpp:3982-4002`).
    AdvanceSkill,
    /// `do_click_special_inv`: the `ShowSpecialInventory` command line
    /// (`G/cmd_general.cpp:4006-4009`).
    ClickSpecialInv,
    /// An entry this build does not port, by name.
    NotPorted(&'static str),
}

/// One of the four attributes the `*+` entries name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatPlusKind {
    /// `con+`: `POINT_HT`.
    Ht,
    /// `int+`: `POINT_IQ`.
    Iq,
    /// `str+`: `POINT_ST`.
    St,
    /// `dex+`: `POINT_DX`.
    Dx,
}

/// The flood count of one character (`m_dwCmdAntiFloodPulse`, `m_dwCmdAntiFloodCount`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandFlood {
    pulse: u64,
    count: u32,
}

/// Who runs a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    /// The Pulse now.
    pub pulse: u64,
    /// The character's GM level.
    pub gm_level: u8,
    /// Its position.
    pub position: u8,
}

/// What a line does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpreted {
    /// Nothing: an empty line, or a position legacy has no line for.
    Nothing,
    /// The flood limit was reached: `DelayedDisconnect(0)`.
    Disconnect,
    /// A `CHAT_TYPE_INFO` line to the character.
    Notice(&'static str),
    /// Run the entry with the rest of the line.
    Run {
        /// The entry.
        entry: CommandEntry,
        /// The line after the command word, spaces and all.
        rest: Vec<u8>,
    },
}

/// `interpret_command(ch, line, len)` for the text after the `/`.
#[must_use]
pub fn interpret(line: &[u8], flood: &mut CommandFlood, caller: Caller) -> Interpreted {
    if caller.gm_level == GM_PLAYER {
        if caller.pulse > flood.pulse.saturating_add(FLOOD_PULSES) {
            flood.count = 0;
            flood.pulse = caller.pulse;
        }
        flood.count = flood.count.saturating_add(1);
        if flood.count >= FLOOD_LIMIT {
            return Interpreted::Disconnect;
        }
    }
    let line = double_dollar(line);
    if line.is_empty() {
        return Interpreted::Nothing;
    }
    let (word, rest) = first_cmd(&line);
    let found = table::COMMANDS.iter().find(|entry| {
        let name = entry.name.as_bytes();
        if entry.exact() {
            name == word.as_slice()
        } else {
            name.get(..word.len()) == Some(word.as_slice())
        }
    });
    let position = found.map_or(POS_DEAD, |entry| entry.position);
    if caller.position < position {
        return match caller.position {
            POS_MOUNTING => Interpreted::Notice("[LS;917]"),
            POS_DEAD => Interpreted::Notice("[LS;918]"),
            POS_SLEEPING => Interpreted::Notice("[LS;919]"),
            POS_RESTING | POS_SITTING => Interpreted::Notice("[LS;920]"),
            _ => Interpreted::Nothing,
        };
    }
    let Some(entry) = found else {
        return Interpreted::Notice(NO_SUCH_COMMAND);
    };
    if entry.gm_level != GM_PLAYER
        && (entry.gm_level > caller.gm_level || entry.gm_level == GM_DISABLE)
    {
        return Interpreted::Notice(NO_SUCH_COMMAND);
    }
    Interpreted::Run {
        entry: *entry,
        rest: rest.to_vec(),
    }
}

/// `isnhspace`: the C locale's white space.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | 0x09..=0x0d)
}

/// `double_dollar`: the line up to its first NUL, each `$` doubled, cut at [`LINE_MAX`] bytes
/// without splitting a pair.
fn double_dollar(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len().min(LINE_MAX));
    for &byte in line.iter().take_while(|&&byte| byte != 0) {
        let width = if byte == b'$' { 2 } else { 1 };
        if out.len() + width > LINE_MAX {
            break;
        }
        out.resize(out.len() + width, byte);
    }
    out
}

/// `first_cmd`: the first word lowered, at most [`WORD_MAX`] bytes, and the line after it.
fn first_cmd(line: &[u8]) -> (Vec<u8>, &[u8]) {
    let start = line
        .iter()
        .position(|&byte| !is_space(byte))
        .unwrap_or(line.len());
    let line = &line[start..];
    let length = line
        .iter()
        .take(WORD_MAX)
        .take_while(|&&byte| !is_space(byte))
        .count();
    let word = line[..length].to_ascii_lowercase();
    (word, &line[length..])
}

/// `one_argument`: the first argument, where a `"` toggles quoting and is dropped, at most
/// `ARGUMENT_MAX` (255) bytes, and the rest after the spaces that follow it.
#[must_use]
pub fn one_argument(line: &[u8]) -> (Vec<u8>, &[u8]) {
    let skip = |line: &[u8]| {
        line.iter()
            .position(|&byte| !is_space(byte))
            .unwrap_or(line.len())
    };
    let mut at = skip(line);
    let mut quoted = false;
    let mut argument = Vec::new();
    while let Some(&byte) = line.get(at) {
        if byte == 0 || argument.len() >= ARGUMENT_MAX {
            break;
        }
        if byte == b'"' {
            quoted = !quoted;
            at += 1;
            continue;
        }
        if !quoted && is_space(byte) {
            break;
        }
        argument.push(byte);
        at += 1;
    }
    let rest = &line[at..];
    (argument, &rest[skip(rest)..])
}

/// `two_arguments` (`G/utils.cpp:143-146`): the first two arguments, each as
/// [`one_argument`] reads them.
#[must_use]
pub fn two_arguments(line: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (first, rest) = one_argument(line);
    let (second, _) = one_argument(rest);
    (first, second)
}
#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: Caller = Caller {
        pulse: 1_000,
        gm_level: GM_PLAYER,
        position: POS_STANDING,
    };

    fn run(line: &[u8], caller: Caller) -> Interpreted {
        interpret(line, &mut CommandFlood::default(), caller)
    }

    fn ran(line: &[u8]) -> (&'static str, Vec<u8>) {
        match run(line, PLAYER) {
            Interpreted::Run { entry, rest } => (entry.name, rest),
            other => panic!("{line:?} ran nothing: {other:?}"),
        }
    }

    #[test]
    fn the_table_is_legacys_under_the_owners_defines() {
        assert_eq!(table::COMMANDS.len(), 267);
        assert_eq!(table::COMMANDS[0].name, "who");
        let named = |name: &str| table::COMMANDS.iter().find(|entry| entry.name == name);
        // `ENABLE_NEWSTUFF` is on and `OFFLINE_SHOP` off.
        assert!(named("click_safebox").is_some());
        assert!(named("click_mall").is_some());
        assert!(table::COMMANDS
            .iter()
            .all(|entry| !entry.name.contains("offline")));
        let password = named("safebox_password").expect("an entry");
        assert_eq!(
            (password.position, password.gm_level),
            (POS_DEAD, GM_PLAYER)
        );
        assert_eq!(
            named("safebox").map(|entry| entry.gm_level),
            Some(GM_HIGH_WIZARD)
        );
    }

    #[test]
    fn a_word_names_the_first_entry_it_is_a_prefix_of() {
        assert_eq!(ran(b"safebox_password 123456").0, "safebox_password");
        assert_eq!(ran(b"safebox_password 123456").1, b" 123456");
        assert_eq!(ran(b"click_saf").0, "click_safebox");
        assert_eq!(ran(b"SAFEBOX_CLOSE").0, "safebox_close");
        assert_eq!(ran(b"  \tmall_close").0, "mall_close");
        // An empty word names `who`, which only an implementor runs.
        assert_eq!(run(b" ", PLAYER), Interpreted::Notice(NO_SUCH_COMMAND));
        let implementor = Caller {
            gm_level: GM_IMPLEMENTOR,
            ..PLAYER
        };
        assert!(matches!(
            run(b" zz", implementor),
            Interpreted::Notice(NO_SUCH_COMMAND)
        ));
        assert!(matches!(
            run(b" ", implementor),
            Interpreted::Run { entry, .. } if entry.name == "who"
        ));
    }

    #[test]
    fn a_prefix_stops_at_the_type_it_in_full_entries() {
        for line in [
            &b"safebox_passwor 1"[..],
            b"safebox_p",
            b"mall_pass",
            b"safebox_change",
        ] {
            let Interpreted::Run { entry, .. } = run(line, PLAYER) else {
                panic!("{line:?}");
            };
            assert_eq!(entry.command(), Command::TypeInFull, "{line:?}");
        }
        assert_eq!(
            table::COMMANDS
                .iter()
                .find(|entry| entry.name == "safebox_password")
                .map(CommandEntry::command),
            Some(Command::SafeboxPassword)
        );
    }

    #[test]
    fn a_do_cmd_entry_takes_only_its_whole_name() {
        let quit = ran(b"quit").0;
        assert_eq!(quit, "quit");
        // `qui` is the type-it-in-full entry before `quit`; `quitt` names nothing.
        assert_eq!(ran(b"qui").0, "qui");
        assert_eq!(run(b"quitt", PLAYER), Interpreted::Notice(NO_SUCH_COMMAND));
    }

    #[test]
    fn only_restart_now_needs_its_whole_name() {
        let index = |name: &str| {
            table::COMMANDS
                .iter()
                .position(|entry| entry.name == name)
                .expect("an entry")
        };
        // Legacy's walk meets `restart_now` first, so each of these paid for a revive (B8).
        assert!(index("restart_now") < index("restart_here"));
        assert!(index("restart_here") < index("restart_town"));
        let dead = Caller {
            position: POS_DEAD,
            ..PLAYER
        };
        let ran_dead = |line: &[u8]| match run(line, dead) {
            Interpreted::Run { entry, .. } => entry.name,
            other => panic!("{line:?} ran nothing: {other:?}"),
        };
        for line in [&b"rest"[..], b"restart", b"restart_"] {
            assert_eq!(ran_dead(line), "restart_here", "{line:?}");
        }
        assert_eq!(
            run(b"restart_n", dead),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
        assert_eq!(ran_dead(b"restart_now"), "restart_now");
        assert_eq!(ran_dead(b"restart_t"), "restart_town");
        assert_eq!(ran_dead(b"RESTART_NOW"), "restart_now");
    }

    #[test]
    fn restart_here_and_town_share_a_handler_but_not_a_command() {
        let command = |name: &str| {
            table::COMMANDS
                .iter()
                .find(|entry| entry.name == name)
                .map(CommandEntry::command)
        };
        assert_eq!(
            command("restart_here"),
            Some(Command::Restart(RestartKind::Here))
        );
        assert_eq!(
            command("restart_town"),
            Some(Command::Restart(RestartKind::Town))
        );
        assert_eq!(
            command("restart_now"),
            Some(Command::Restart(RestartKind::Now))
        );
    }

    #[test]
    fn a_word_that_names_nothing_answers_the_does_not_exist_line() {
        assert_eq!(
            run(b"no_such", PLAYER),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
        assert_eq!(run(b"", PLAYER), Interpreted::Nothing);
        assert_eq!(run(b"\0who", PLAYER), Interpreted::Nothing);
    }

    #[test]
    fn a_command_above_the_level_does_not_exist() {
        assert_eq!(
            run(b"safebox 3", PLAYER),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
        let high = Caller {
            gm_level: GM_HIGH_WIZARD,
            ..PLAYER
        };
        assert!(matches!(run(b"safebox 3", high), Interpreted::Run { .. }));
        let wizard = Caller {
            gm_level: GM_WIZARD,
            ..PLAYER
        };
        assert_eq!(
            run(b"safebox 3", wizard),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
    }

    #[test]
    fn the_position_is_checked_before_the_word_is() {
        let fighting = table::COMMANDS
            .iter()
            .find(|entry| entry.position == POS_FIGHTING)
            .expect("an entry");
        let at = |position| Caller { position, ..PLAYER };
        let line = fighting.name.as_bytes();
        for (position, notice) in [
            (POS_DEAD, Interpreted::Notice("[LS;918]")),
            (POS_SLEEPING, Interpreted::Notice("[LS;919]")),
            (POS_RESTING, Interpreted::Notice("[LS;920]")),
            (POS_SITTING, Interpreted::Notice("[LS;920]")),
            (POS_FISHING, Interpreted::Nothing),
        ] {
            assert_eq!(run(line, at(position)), notice, "{position}");
        }
        assert!(matches!(
            run(line, at(POS_FIGHTING)),
            Interpreted::Run { .. }
        ));
        // A word that names nothing meets the terminator's `POS_DEAD`, which every position
        // passes.
        assert_eq!(
            run(b"no_such", at(POS_DEAD)),
            Interpreted::Notice(NO_SUCH_COMMAND)
        );
    }

    #[test]
    fn the_tenth_line_within_a_second_disconnects_a_player() {
        let mut flood = CommandFlood::default();
        for pulse in 1..10 {
            let caller = Caller { pulse, ..PLAYER };
            assert_ne!(
                interpret(b"mall_close", &mut flood, caller),
                Interpreted::Disconnect
            );
        }
        let caller = Caller {
            pulse: 10,
            ..PLAYER
        };
        assert_eq!(interpret(b"", &mut flood, caller), Interpreted::Disconnect);
        // 26 Pulses on, the count starts over.
        let mut flood = CommandFlood::default();
        for pulse in (0..40).map(|step| step * 3) {
            let caller = Caller { pulse, ..PLAYER };
            assert_ne!(
                interpret(b"", &mut flood, caller),
                Interpreted::Disconnect,
                "{pulse}"
            );
        }
        let gm = Caller {
            gm_level: GM_LOW_WIZARD,
            ..PLAYER
        };
        let mut flood = CommandFlood::default();
        for _ in 0..20 {
            assert_ne!(interpret(b"", &mut flood, gm), Interpreted::Disconnect);
        }
    }

    #[test]
    fn the_count_lasts_through_its_twenty_fifth_pulse() {
        // `thecore_pulse() > pulse + PASSES_PER_SEC(1)` (`G/cmd.cpp:615`): a line 25 Pulses
        // after the count started still counts, and one 26 Pulses after starts a new count.
        let (start, mut flood) = (
            Caller {
                pulse: 100,
                ..PLAYER
            },
            CommandFlood::default(),
        );
        assert_ne!(interpret(b"", &mut flood, start), Interpreted::Disconnect);
        let edge = Caller {
            pulse: 100 + FLOOD_PULSES,
            ..PLAYER
        };
        for _ in 0..8 {
            assert_ne!(interpret(b"", &mut flood, edge), Interpreted::Disconnect);
        }
        assert_eq!(interpret(b"", &mut flood, edge), Interpreted::Disconnect);
        let mut flood = CommandFlood::default();
        assert_ne!(interpret(b"", &mut flood, start), Interpreted::Disconnect);
        let past = Caller {
            pulse: 101 + FLOOD_PULSES,
            ..PLAYER
        };
        for _ in 0..9 {
            assert_ne!(interpret(b"", &mut flood, past), Interpreted::Disconnect);
        }
    }

    #[test]
    fn a_dollar_is_doubled_and_the_line_cut_at_256_bytes() {
        assert_eq!(double_dollar(b"a$b"), b"a$$b");
        assert_eq!(double_dollar(&[b'x'; 300]).len(), LINE_MAX);
        let mut line = vec![b'x'; 255];
        line.push(b'$');
        assert_eq!(double_dollar(&line).len(), 255);
        assert_eq!(double_dollar(b"ab\0cd"), b"ab");
    }

    #[test]
    fn the_word_is_at_most_128_bytes() {
        let long = [b'a'; 200];
        let (word, rest) = first_cmd(&long);
        assert_eq!((word.len(), rest.len()), (WORD_MAX, 72));
    }

    #[test]
    fn every_ported_kit_line_routes_past_not_ported() {
        // `command()` routes by handler and `kit_name` maps the same handlers:
        // a line dropped from the route answers nothing (ledger 355 caught
        // `do_attr_full_set` missing from it).
        for (handler, name) in [
            ("do_all_skill_master", "all_skill_master"),
            ("do_full_set", "full_set"),
            ("do_item_full_set", "item_full_set"),
            ("do_attr_full_set", "attr_full_set"),
            ("do_refine_rod", "refine_rod"),
            ("do_refine_pick", "refine_pick"),
            ("do_max_pick", "max_pick"),
            ("do_use_item", "use_item"),
            ("do_clear_affect", "do_clear_affect"),
            ("do_dragon_soul", "dragon_soul"),
            ("do_ds_list", "ds_list"),
            ("do_dice", "dice"),
            ("do_multi_farm", "multi_farm"),
            ("do_gift", "gift"),
            ("do_hair", "hair"),
            ("do_fishing_simul", "fish_simul"),
            ("do_costume", "costume"),
            ("do_state", "state"),
            ("do_inventory", "inventory"),
        ] {
            let entry = CommandEntry {
                name,
                handler,
                position: POS_DEAD,
                gm_level: GM_IMPLEMENTOR,
            };
            assert!(
                !matches!(entry.command(), Command::NotPorted(_)),
                "{handler} routes to a ported command"
            );
        }
    }

    #[test]
    fn an_argument_reads_quotes_and_skips_spaces() {
        assert_eq!(one_argument(b"  abc def"), (b"abc".to_vec(), &b"def"[..]));
        assert_eq!(one_argument(b" \"a b\"c d"), (b"a bc".to_vec(), &b"d"[..]));
        assert_eq!(one_argument(b""), (Vec::new(), &b""[..]));
        let (argument, rest) = one_argument(&[b'z'; 300]);
        assert_eq!((argument.len(), rest.len()), (ARGUMENT_MAX, 45));
    }
}
