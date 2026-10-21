//! The guilds: `CGuildManager` + `CGuild` (`G/guild_manager.{h,cpp}`, `G/guild.{h,cpp}`):
//! creation, the in-memory registry, the creation walk's records, the guild line's fan,
//! and the doors: invite, answer, removal.
//!
//! - **The world holds every guild.** Legacy's manager loads all guilds at boot
//!   (`CGuildManager::Initialize`, `G/guild_manager.cpp:212-243`) and links every member
//!   PID (`CGuild::LoadGuildMemberData`, `G/guild.cpp:543-574`), so the duplicate-name
//!   judgement, the joinable ladder and the guild line's defence read memory, not the
//!   store. The store owns the rows (`db::guilds`); the world owns the judgements.
//! - **Creation, join and removal are judged, stored, then committed.** `AnswerMakeGuild`
//!   judges with the descriptor in one world step, which answers
//!   [`GuildMakeAnswer::Store`] when the rows must be written; the descriptor writes
//!   them and sends a second step, which commits. The join (`InviteAccept` →
//!   `RequestAddMember` → `AddMember`) and the removal (`RequestRemoveMember` →
//!   `RemoveMember`) follow the same shape (`GuildJoinAnswer`, `GuildRemoveAnswer`):
//!   the world judges, the descriptor writes the member row, the world fans.
//! - **No P2P, no wars, no land, no bonuses.** The DB-peer and game-to-game packets are
//!   retired (ADR-0001/0002): `RequestAddMember` commits locally, the `GUILD_NAME`
//!   broadcast is one fan, and the guild line's GG relay is gone. `SendEnemyGuild` sends
//!   nothing with no wars (`G/guild_war.cpp:60-109`), `HasLand` answers 0 with no building
//!   manager (`G/guild.cpp:1990-1993`), and the `__GUILD_BONUSES__` arms wait for
//!   `sys.guild.bonuses`.
//! - **Deposit, withdraw, skills, offer, comments, the money change and the war list
//!   are later slices.** Their `CG_GUILD` sub-packets frame (`protocol::cg_guild`) but
//!   the world answers nothing yet; the money pair stays quiet because legacy's arms
//!   return before their bodies (`G/input_main.cpp:2826-2878`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tracing::warn;

use common::cfloat::f64_to_i32;
use common::item_slots::{
    usable_inventory_cells, EWindows, CUSTOM_INVENTORY_SLOT_END, CUSTOM_INVENTORY_SLOT_START,
};
use common::vid::Vid;
use gamedata::server_attr::SectreeGrid;
use protocol::cg_guild_answer::GUILD_NAME_FIELD_SIZE;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};
use protocol::gc_guild::{
    guild_change_exp_record, guild_change_member_general_record, guild_change_member_grade_record,
    guild_grade_auth_record, guild_grade_name_record, guild_grade_record, guild_info_record,
    guild_invite_record, guild_list_one_record, guild_list_record, guild_login_record,
    guild_logout_record, guild_name_record, guild_remove_record, guild_skill_info_record,
    guild_war_end_list_record, guild_war_list_record, guild_war_record, guild_war_score_record,
    pad_grade_name, pad_guild_name, pad_member_name, GcGuildGrade, GcGuildInfo, GcGuildMember,
    GcGuildSkillInfo, GUILD_COMMENT_MAX_LEN, GUILD_GRADE_COUNT, GUILD_GRADE_NAME_MAX_LEN,
    GUILD_NAME_MAX_LEN, GUILD_SKILL_COUNT,
};
use protocol::item_pos::ItemPos;
use world::character::{
    Character, ItemChange, ItemRecord, MoveDone, MoveKind, MoveRecord, PointRecord,
};
use world::item::gc_item_clear;

use super::fight::AFF_STUN_WORD;
use super::guild_mark::{self, INVALID_MARK_ID, MARK_PIXELS};
use super::GameState;
use crate::chat_line::{chat_packet, Arg};
use crate::item_move::{gold_delta, row_changes, Mover};

/// `GUILD_CREATE_FEE`: the 200000 gold `CInputMain::AnswerMakeGuild` gates on and takes
/// (`G/input_main.cpp:2677,2719-2720`). The quest arm gates on the same 200000
/// (`guild_manage.lua:126`).
pub(super) const GUILD_CREATE_FEE: u64 = 200_000;
/// `GUILD_CREATE_ITEM_VNUM` (`G/guild.h:21`): 70101, the item `AnswerMakeGuild` removes one
/// of (`G/input_main.cpp:2728`).
pub(super) const GUILD_CREATE_ITEM_VNUM: u32 = 70_101;
/// `GUILD_BASE_POWER` (`G/guild.h:14`): 400. A new guild's power and max power are both
/// this (`ComputeGuildPoints`, `G/guild.cpp:1439-1444`, at level 1).
const GUILD_BASE_POWER: u32 = 400;
/// `GUILD_POWER_PER_LEVEL` (`G/guild.h:16`): 100. A loaded guild's max power is
/// `400 + (level - 1) * 100` (`ComputeGuildPoints`, `G/guild.cpp:1439-1444`).
const GUILD_POWER_PER_LEVEL: u32 = 100;
/// `GUILD_AUTH_ADD_MEMBER | GUILD_AUTH_REMOVE_MEMBER | GUILD_AUTH_NOTICE |
/// GUILD_AUTH_USE_SKILL` (`G/guild.h:95-100`): the leader grade's auth (`G/guild.cpp:74`).
const LEADER_AUTH: u8 = 15;
/// The leader grade's name (`G/guild.cpp:70-74`): `Leader` under
/// `__MULTI_LANGUAGE_SYSTEM__`, which this build defines.
const LEADER_GRADE_NAME: &[u8] = b"Leader";
/// Every other grade's name at creation (`G/guild.cpp:76-83`): `...`.
const FOLLOWER_GRADE_NAME: &[u8] = b"...";
/// `GUILD_LEADER_GRADE` (`G/guild.h:13`): 1. The master's member row is this grade
/// (`RequestAddMember(cp.master, GUILD_LEADER_GRADE)`, `G/guild.cpp:126`).
const GUILD_LEADER_GRADE: u8 = 1;
/// `GUILD_GRADE_COUNT` (`G/guild.h:11`): 15, as the grades' top.
const GUILD_GRADE_COUNT_U8: u8 = 15;
/// `GetMaxGeneralCount` (`G/guild.h:233`): 1. The skill arm is commented out, so the
/// max is the literal.
const MAX_GENERAL_COUNT: usize = 1;
/// The grade an accepted invitee joins at (`RequestAddMember(pchInvitee, 15)`,
/// `G/guild.cpp:2147`).
const JOIN_GRADE: u8 = 15;
/// `GUILD_AUTH_ADD_MEMBER` (`G/guild.h:94`): bit 0.
const GUILD_AUTH_ADD_MEMBER: u8 = 1;
/// `GUILD_AUTH_REMOVE_MEMBER` (`G/guild.h:95`): bit 1.
const GUILD_AUTH_REMOVE_MEMBER: u8 = 2;
/// `GUILD_AUTH_NOTICE` (`G/guild.h:96`): bit 2, the comment-`!` gate.
const GUILD_AUTH_NOTICE: u8 = 4;
/// `GUILD_AUTH_USE_SKILL` (`G/guild.h:97`): bit 3, the skill-use gate.
const GUILD_AUTH_USE_SKILL: u8 = 8;
/// `GUILD_SKILL_START` (`G/skill.h:162`): 151.
const GUILD_SKILL_START: u32 = 151;
/// `GUILD_SKILL_TELEPORT` (`G/skill.h:170`): 158.
const GUILD_SKILL_TELEPORT: u32 = 158;
/// `SKILL_EUNHYUNG` (`G/skill.h:74`): 34, the hide the guild cast strips.
const SKILL_EUNHYUNG: u32 = 34;
/// `GUILD_MAX_LEVEL` (`common/length.h:67`): 20.
const GUILD_MAX_LEVEL: u8 = 20;
/// `guild_exp_table2` (`G/constants.cpp:617-642`): the exp each level costs, indexed by
/// level, with the 0 at level 0 legacy's table carries.
const GUILD_LEVELUP_EXP: [u32; 21] = [
    0, 6_000, 18_000, 36_000, 64_000, 94_000, 130_000, 172_000, 220_000, 274_000, 334_000, 400_000,
    600_000, 840_000, 1_120_000, 1_440_000, 1_800_000, 2_600_000, 3_200_000, 4_000_000, 16_800_000,
];
/// `GUILD_LADDER_POINT_PER_LEVEL` (`G/guild.h:20`): 1000, the ladder step each guild
/// level adds (`GuildPointChange`, `G/guild.cpp:1616-1617`).
const GUILD_LADDER_POINT_PER_LEVEL: i32 = 1000;
/// `GUILD_WAR_TYPE_FIELD` (`common/length.h:769`): 0, the open-field war — fought where
/// the players already stand, so it needs no map.
pub(super) const GUILD_WAR_TYPE_FIELD: u8 = 0;
/// `GUILD_WAR_TYPE_BATTLE` (`common/length.h:770`): 1, the war fought on the battle map.
pub(super) const GUILD_WAR_TYPE_BATTLE: u8 = 1;
/// `GUILD_WAR_TYPE_FLAG` (`common/length.h:771`): 2, the war fought on the flag map.
pub(super) const GUILD_WAR_TYPE_FLAG: u8 = 2;
/// `GUILD_WAR_TYPE_MAX_NUM` (`common/length.h:772`): 3. A `/war` type at or past it
/// folds to field (`G/cmd_general.cpp:2055-2056`).
pub(super) const GUILD_WAR_TYPE_MAX_NUM: u8 = 3;
/// `EGuildWarState` (`common/length.h:776-785`): none, sent-declare, refused,
/// received-declare, wait-start, cancelled, on-war, ended, over, reserved.
const GUILD_WAR_NONE: u8 = 0;
const GUILD_WAR_SEND_DECLARE: u8 = 1;
const GUILD_WAR_RECV_DECLARE: u8 = 3;
/// `GUILD_WAR_ON_WAR` (`common/length.h:782`): 6. The accept lands it (230x); the fan's
/// `[LS;558]` arm and `UnderAnyWar` read it.
const GUILD_WAR_ON_WAR: u8 = 6;
const GUILD_WAR_END: u8 = 7;
const GUILD_WAR_RESERVE: u8 = 9;
/// `GUILD_WAR_MIN_MEMBER_COUNT` (`G/guild.h:19`): 8. Both guilds need this many member
/// rows to start a war (`CGuild::CanStartWar`, `G/guild_war.cpp:143-152`).
const GUILD_WAR_MIN_MEMBER_COUNT: usize = 8;
/// `KOR_aGuildWarInfo[type].iWarPrice` (`G/constants.cpp:1142-1146`): 0 for every type,
/// so `/war`'s money arms never fire — ported as the table read they are.
fn war_price(war_type: u8) -> i32 {
    super::war_map::WAR_TYPES
        .get(usize::from(war_type))
        .map_or(0, |info| info.price)
}

/// `GUILD_WAR_DURATION` (`common/length.h:787`): 1800, the field war's seconds.
const GUILD_WAR_DURATION: u32 = 1800;
/// `GUILD_WAR_LADDER_HALF_PENALTY_TIME` (`common/length.h:789`): 43200, the seconds an
/// ended pair halves the next win's gain.
const GUILD_WAR_LADDER_HALF_PENALTY: u32 = 43_200;
/// `GUILD_COMMENT_MAX_COUNT` (`G/guild.h:12`): 12, the board's read limit.
pub const GUILD_COMMENT_MAX_COUNT: i64 = 12;
/// The comment gate's minute (`PASSES_PER_SEC(60)` at the post, `G/guild.cpp:1071` — the
/// "10 minutes" comment is stale), in Pulses.
const GUILD_COMMENT_WAIT_PULSES: u64 = 1500;
/// The invite event's ten seconds (`PASSES_PER_SEC(10)`, `G/guild.cpp:2094`), in Pulses.
const GUILD_INVITE_WAIT_PULSES: u64 = 250;
/// The day in seconds. The disband/withdraw delays multiply the event flag by
/// `test_server ? 60 : 86400` (`G/guild_manager.cpp:66-75`); the Rewrite has no test
/// server, so the multiplier is always 86400.
const DAY_SECONDS: i64 = 86_400;

/// `TGuildMember` (`G/guild.h:30-43`) with its link: the row plus the VID the member is
/// online under, when it is.
#[derive(Debug, Clone)]
struct GuildMember {
    /// `grade`: 1 to 15.
    grade: u8,
    /// `is_general`.
    is_general: u8,
    /// `job`, from the player row at load, from the body at creation.
    job: u8,
    /// `level`, from the player row at load, from the body at creation.
    level: u8,
    /// `offer_exp`.
    offer: u32,
    /// The member's name. Legacy holds it in the row (`LoadGuildMemberData` joins the
    /// player table, `G/guild.cpp:673-675`); the port holds it the same way.
    name: Vec<u8>,
    /// The VID the member is online under, when it is (`m_memberOnline`,
    /// `G/guild.h:386`).
    vid: Option<Vid>,
}

/// `TGuildWar` (`G/guild.h:137-158`): one enemy entry — the start time, the score
/// against it, its state, its type, and its war-map index.
#[derive(Debug, Clone, Copy)]
// The start time, the score and the map land with the entry; the on-war slice reads
// them (`sys.guild.war`).
#[allow(dead_code)]
struct GuildWar {
    /// `war_start_time`.
    war_start_time: u32,
    /// `score`: this guild's score against the enemy.
    score: u32,
    /// `state`: one of the `GUILD_WAR_*` states.
    state: u8,
    /// `type`: one of the `GUILD_WAR_TYPE_*` types.
    war_type: u8,
    /// `map_index`: the war map, 0 for a field war. It is the private copy's index, which
    /// names the map it was copied from (`SECTREE_MANAGER::CreatePrivateMap`).
    map_index: i32,
}

impl GuildWar {
    /// A fresh entry of `war_type` (`SGuildWar(type)`, `G/guild.h:143-152`): the score
    /// and the map at 0, the state received-declare — the manager's declare overwrites
    /// it with the sent or received arm.
    fn new(war_type: u8) -> Self {
        Self {
            war_start_time: 0,
            score: 0,
            state: GUILD_WAR_RECV_DECLARE,
            war_type,
            map_index: 0,
        }
    }
}

/// `TGuildData` (`G/guild.h:102-130`) with its members: the row plus the member map.
/// The dungeon triple is not held (`sys.guild.dungeon` is not ported).
#[derive(Debug, Clone)]
struct Guild {
    /// `guild_id`.
    id: u32,
    /// `master_pid`.
    master_pid: u32,
    /// `exp`.
    exp: u32,
    /// `level`.
    level: u8,
    /// `name`: the 12 legacy bytes.
    name: Vec<u8>,
    /// `grade_array`: the 15 `(name, auth)` pairs.
    grades: [(Vec<u8>, u8); GUILD_GRADE_COUNT],
    /// `skill_point`.
    skill_point: u8,
    /// `abySkill`.
    skills: [u8; GUILD_SKILL_COUNT],
    /// `power`.
    power: u32,
    /// `max_power`.
    max_power: u32,
    /// `gold`.
    gold: u32,
    /// `m_iMemberCountBonus`, clamped to 0-18 on read (`GetMaxMemberCount`,
    /// `G/guild.cpp:1870-1878`). No quest or GM command on the owner's data sets it, so it
    /// is always 0.
    member_count_bonus: i32,
    /// `m_member`: the members by PID (`G/guild.h:383`).
    members: BTreeMap<u32, GuildMember>,
    /// `TGuildData::ladder_point`.
    ladder_point: i32,
    /// `TGuildData::win`, `draw`, `loss` (`G/guild.h:102-130`): the war tallies, which the
    /// ranking strings print and the `*count0` readers answer. `SetWarData` is commented
    /// out of the quest table, so only the war system writes them (`sys.guild.war`).
    war_wins: i32,
    war_draws: i32,
    war_losses: i32,
    /// `m_guildPostCommentPulse` (`G/guild.h:436`): the Pulse a guild may post again on.
    /// Zero posts at once, as a fresh guild's pulse does.
    post_comment_due: u64,
    /// `abSkillUsable` (`G/guild.h:389`): the cooldown flags, memory-only (never
    /// stored; a reboot re-arms every skill). The DB server's `m_pqSkill` pump
    /// (`db/GuildManager.cpp:280-286`) re-fans them on expiry; the port re-arms
    /// them in [`GameState::run_guild_ticks`] when `usable_due` passes.
    usable: [bool; GUILD_SKILL_COUNT],
    /// The Pulse each unusable skill re-arms on (`now + cooltime * 25`: the DB
    /// queue compares `time_t` seconds, `db/GuildManager.cpp:764`). `None` while
    /// usable.
    usable_due: [Option<u64>; GUILD_SKILL_COUNT],
    /// `m_EnemyGuild` (`G/guild.h:384-385`): the enemies by guild id, in id order
    /// (`std::map`). Memory-only: a reboot clears every declare, as the DB process's
    /// `m_DeclareMap` dying does.
    enemies: BTreeMap<u32, GuildWar>,
}

/// The `guild.*` quest calls' shadow (230s): the guilds as the world holds them at a quest
/// step's start, cloned into the [`Session`](super::quests::Session) so a call answers the
/// same rows the world would judge. Writers judged after the step re-read the live guilds.
pub(super) type GuildShadow = Guilds;

/// The `change_master_with_limit` arguments past the name (230s): the min level, resign,
/// other-leader, other-member and ignore flag, each present when the script passed it.
type WithLimitArgs<'a> = (
    Option<&'a quest::manager::GuildArg>,
    Option<&'a quest::manager::GuildArg>,
    Option<&'a quest::manager::GuildArg>,
    Option<&'a quest::manager::GuildArg>,
    Option<&'a quest::manager::GuildArg>,
);
/// `CGuildManager` (`G/guild_manager.h`): the guilds by id and the PID→guild index
/// (`m_map_pkGuildByPID`).
#[derive(Debug, Default, Clone)]
pub(super) struct Guilds {
    /// The guilds, by id (`m_mapGuild`).
    by_id: BTreeMap<u32, Guild>,
    /// Each member PID's guild id (`m_map_pkGuildByPID`).
    of: HashMap<u32, u32>,
    /// The pending invites (`m_GuildInviteEventMap`, `G/guild.h:412`): `(guild_id,
    /// invitee_pid)` to the Pulse the invite lapses on. Legacy holds one map per guild;
    /// the port holds one map keyed by both, which answers the same questions.
    invites: BTreeMap<(u32, u32), u64>,
    /// Whether the boot install ran. A second install is refused, as a second
    /// allocator install is.
    pub(super) installed: bool,
    /// `m_GuildWar` (`G/guild_manager.h`): the live wars as sorted `(lo, hi)` pairs, in
    /// pair order (`std::set`). Memory-only, as the declares are.
    pub(super) wars: BTreeSet<(u32, u32)>,
    /// The DB-side end-time map (`m_mapGuildWarEndTime`, `db/GuildManager.h`): the sorted
    /// pair each ended war stamped, in epoch seconds. `IsHalfWinLadderPoint` reads it
    /// (`db/GuildManager.cpp:389-401`); the game side's same-named map is write-only and
    /// stays unported. Memory-only, as the wars are.
    war_end_times: BTreeMap<(u32, u32), u32>,
    /// The war ends the tick judged but no descriptor stored yet: the tick has no
    /// session, so it queues the outcomes here and a poll command drains them.
    pending_war_ends: Vec<WarEndOutcome>,
}

/// What one `CG_ANSWER_MAKE_GUILD` step judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildMakeAnswer {
    /// The descriptor writes the rows, then sends [`GuildMakeCommit`]: the world judged
    /// every gate and the store has not been asked yet. Carries the name legacy persists
    /// (`m_data.name`, `G/guild.cpp:67`), as a `String`: the alphabet check held it to
    /// ASCII letters and digits, so the conversion cannot fail.
    Store {
        /// The name to store, without the client's NUL.
        name: String,
    },
    /// The world committed the creation: the registry and the master's link are in, the
    /// fee and the item left the master's pockets, and the answer carries every record
    /// for the descriptor to send — to the master and to every PC — plus the fee and the
    /// item half for the descriptor to store with the guild rows.
    Committed {
        /// The new guild's id.
        id: u32,
        /// What the master is sent, in order: the `GUILD_NAME` record, the `[LS;682]`
        /// line, the fee record, the item records, and the login walk.
        own: Vec<Vec<u8>>,
        /// What every other PC is sent: the `GUILD_NAME` broadcast.
        others: Vec<Vec<u8>>,
        /// The 200000 fee, as a negative gold change on the stored gold.
        gold: i64,
        /// The row changes for the removed 70101 item.
        changes: Vec<db::items::RowChange>,
    },
    /// A gate refused: the line legacy sends (`[LS;678]`, `[LS;679]`, `[LS;680]`,
    /// `[LS;473]`, `[LS;683]`), or nothing for the quiet arms (the gold, the has-guild
    /// arm). The descriptor sends the line, if any, and stores nothing.
    Refused {
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// What one `MARK_UPLOAD` step judged: saved under an id, deleted, or quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkUploadAnswer {
    /// The upload committed under this id; the descriptor stores the row.
    Saved {
        /// The id the manager kept or allocated.
        mark_id: u32,
    },
    /// The image was all zero: the mark left the manager and the map. The descriptor
    /// deletes the stored row.
    Deleted,
    /// No guild, under level, or no free id: legacy logs and sends nothing.
    Quiet,
}

/// The commit half of a creation: the id the store answered plus the judged name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildMakeCommit {
    /// The new guild's id.
    pub id: u32,
    /// The judged name, without the client's NUL, as ASCII.
    pub name: String,
}

/// What one `GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER` step judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildJoinAnswer {
    /// The descriptor inserts the member row, then sends [`GuildJoinCommit`]: the world
    /// judged the ladder and the row is not stored yet. Carries the guild, so the
    /// descriptor's insert names it.
    Store {
        /// The guild the invitee joins.
        guild_id: u32,
    },
    /// The world committed the join: the row is in, the LIST-one and the LOGIN already
    /// fanned through the outboxes, and the answer carries the joiner's login walk for
    /// the descriptor to send on its own session. An offline joiner hears no walk
    /// (`FindByPID` NULL takes the P2P arm, `G/guild.cpp:188-193`, which is retired).
    Committed {
        /// The joiner's login walk (grades, INFO, LIST, the LOGINs, skills), empty when
        /// the joiner is offline.
        own: Vec<Vec<u8>>,
    },
    /// A gate refused: the line legacy sends to the inviter (`[LS;940]` aside — block
    /// modes are unported) or to the invitee (the accept ladder), or nothing for the
    /// quiet arms (no guild, no invite, a deny, the guilded/duplicate row). The
    /// descriptor sends the line, if any, to `to`, and stores nothing.
    Refused {
        /// Whose client hears the line.
        to: Vid,
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// The commit half of a join: the guild plus the joiner's PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildJoinCommit {
    /// The guild the invitee joins.
    pub guild_id: u32,
    /// The joiner's PID.
    pub pid: u32,
}

/// What one `GUILD_SUBHEADER_CG_REMOVE_MEMBER` step judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildRemoveAnswer {
    /// The descriptor deletes the member row, then sends [`GuildRemoveCommit`]: the
    /// world judged the arm and the row is still stored. Carries the victim's judged
    /// online-ness — the commit's post arm reads it, not a re-read — and the line legacy
    /// chats *before* the round trip (the online arm's no-auth scold), if any, for the
    /// descriptor to send first.
    Store {
        /// The victim's PID.
        pid: u32,
        /// Whether the victim was online when judged.
        online: bool,
        /// The line to the speaker, if the arm chats before the store.
        pre: Option<Vec<u8>>,
    },
    /// The world committed the removal: the row is out, the victim unlinked, the REMOVE
    /// already fanned through the outboxes, and the answer carries the line legacy chats
    /// *after* the round trip (the offline arm's forced-leave line), if any, for the
    /// descriptor to send on its own session.
    Committed {
        /// The line to the speaker, if the arm chats after the store.
        post: Option<Vec<u8>>,
    },
    /// A gate refused: the line legacy sends (`[LS;523]`, the foreign-guild line, the
    /// offline no-auth line, `"Couldn't find such a person."`), or nothing for the
    /// quiet arms (the leader's quiet refusal). The descriptor sends the line, if any,
    /// and stores nothing.
    Refused {
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// The commit half of a removal: the speaker's VID plus the victim's PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildRemoveCommit {
    /// The VID of the removing character.
    pub speaker: Vid,
    /// The victim's PID.
    pub pid: u32,
    /// Whether the victim was online when judged: the online arm fans only, the offline
    /// arm chats `"Guild members were forced to leave."` after the store.
    pub online: bool,
}

/// What one grade-arm step judged (`GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME`,
/// `GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY`,
/// `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE`,
/// `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildGradeAnswer {
    /// The descriptor stores the judged row, then sends [`GuildGradeCommit`]: the world
    /// judged the arm and the row is not stored yet. Carries the id and the judged
    /// value, so the descriptor's update names them.
    Store {
        /// The guild id.
        guild_id: u32,
        /// What the descriptor stores.
        set: GuildGradeSet,
    },
    /// The world committed the change: the row is in, the fan already went through the
    /// outboxes, and the answer carries the line legacy chats *after* the round trip,
    /// if any, for the descriptor to send on its own session.
    Committed {
        /// The line to the speaker, if the arm chats after the store.
        post: Option<Vec<u8>>,
    },
    /// A gate refused: the line legacy sends, or nothing for the quiet arms (no
    /// guild has its `[LS;523]`, but a non-row, a grade-1 write and a missing member
    /// stay silent). The descriptor sends the line, if any, and stores nothing.
    Refused {
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// One comment of a `GuildCommentAnswer::Board`: the id, the poster and the content,
/// as `board` reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildCommentLine {
    /// The row id.
    pub id: u32,
    /// The poster's name.
    pub name: Vec<u8>,
    /// The content.
    pub content: Vec<u8>,
}

/// What one comment-arm step judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildCommentAnswer {
    /// The descriptor inserts the post, then sends `GuildCommentCommit::Post`: the world
    /// judged the gates and the row is not stored yet. Carries the judged content, as
    /// bytes, and the poster.
    Store {
        /// The guild id.
        guild_id: u32,
        /// The poster's PID.
        pid: u32,
        /// The judged content, as bytes.
        content: Vec<u8>,
    },
    /// The world ran the step: a post or a refresh carries the board for the
    /// descriptor to send on its own session; a delete carries whether a row was
    /// there, which the descriptor's answer already knows — the board or the line
    /// rides this answer instead. The outboxes stay empty: every comment record is
    /// self.
    Done {
        /// The board, newest first, or the delete's line when the row was missing.
        board: Option<Vec<GuildCommentLine>>,
        /// The delete's missing-row line, when the row was missing.
        line: Option<Vec<u8>>,
    },
    /// A gate refused: the line legacy sends, or nothing for the quiet arms (no
    /// guild has its `[LS;523]`, but a non-row stays silent). The descriptor sends
    /// the line, if any, and stores nothing.
    Refused {
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// The commit half of a comment step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildCommentCommit {
    /// The descriptor inserted the post; the world sets the minute and answers the
    /// board for the speaker's own session. Carries the speaker, the guild and the
    /// board — the descriptor read it after the insert, as `FuncAfterQuery` reads it
    /// after the query (`G/guild.cpp:1066`).
    Post {
        /// The VID of the character whose client posted.
        speaker: Vid,
        /// The guild id.
        guild_id: u32,
        /// The board after the post.
        board: Vec<GuildCommentLine>,
    },
    /// The descriptor read the board (a refresh) or deleted a row (a delete); the
    /// world answers it for the speaker's own session. A delete carries whether a row
    /// was there: the board when one was, the missing-row line when none was.
    Read {
        /// The VID of the character whose client sent the arm.
        speaker: Vid,
        /// The guild id.
        guild_id: u32,
        /// The board after the read, or the delete's board when a row was there.
        board: Option<Vec<GuildCommentLine>>,
        /// Whether the delete removed a row.
        deleted: bool,
    },
}

/// What one grade-arm step stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildGradeSet {
    /// The grade's new name, without the client's NUL, as ASCII (`ChangeGradeName`'s
    /// update, `G/guild.cpp:851-855`).
    GradeName {
        /// The 1-based grade.
        grade: u8,
        /// The judged name.
        name: String,
    },
    /// The grade's new auth (`ChangeGradeAuth`'s update, `G/guild.cpp:881-885`).
    GradeAuth {
        /// The 1-based grade.
        grade: u8,
        /// The judged auth byte.
        auth: u8,
    },
    /// The member's new grade (the grade half of `SaveMember`,
    /// `G/guild.cpp:1660-1670`).
    MemberGrade {
        /// The victim's PID.
        pid: u32,
        /// The judged grade.
        grade: u8,
    },
    /// The member's new general flag (the `is_general` half of `SaveMember`,
    /// `G/guild.cpp:1660-1670`).
    MemberGeneral {
        /// The victim's PID.
        pid: u32,
        /// The judged 0/1 flag.
        is_general: u8,
    },
    /// An experience offer: the levelled delta, `OfferExp`'s `amount / 100`
    /// (`G/guild.cpp:974`), for the guild row and the member's `offer_exp`
    /// (`:978-979`). Carries the loop's whole answer — the new exp, level and
    /// ladder — so the descriptor stores it in one update.
    Offer {
        /// The speaker's PID, whose `offer_exp` grows and whose `POINT_EXP` fell.
        pid: u32,
        /// The floored debit the speaker's `POINT_EXP` fell by.
        floored: u32,
        /// The levelled delta.
        offer: u32,
        /// The guild's judged exp.
        exp: u32,
        /// The guild's judged level.
        level: u8,
        /// The guild's judged ladder.
        ladder: i32,
    },
    /// A faith charge: `ChargeSP`'s capped power (`G/guild.cpp:1813-1819`), the SP
    /// the guild holds after the charge.
    Charge {
        /// The speaker's PID, whose `POINT_GOLD` fell.
        pid: u32,
        /// The gold the speaker's `POINT_GOLD` fell by.
        debit: u64,
        /// The guild's judged power.
        power: u32,
        /// The capped SP the `[LS;938]` count chats (`G/guild.cpp:1830`), which is
        /// the SP the guild gained.
        gained: u32,
    },
}

/// One `guild.*` quest call a script made (230s): the call's member name and its arguments,
/// in order. The Session records each call while the script waits; the world judges each
/// after the step, when the guilds are reachable again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildQuestOp {
    /// The call's member name (`level`, `get_sp0`).
    pub name: Vec<u8>,
    /// Its arguments, dworded numbers or raw text.
    pub args: Vec<quest::manager::GuildArg>,
}

/// What one `guild.*` quest call judged (230s): the answer legacy pushed, the records the
/// caller hears now (a refusal line, a countdown), and the writer's store halves for the
/// descriptor to drain after the step, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct GuildQuestJudged {
    /// What the script is pushed.
    pub answer: quest::manager::GuildAnswer,
    /// The records the caller's own client is sent now, in order.
    pub own: Vec<Vec<u8>>,
    /// The writer's store halves, in order.
    pub stores: Vec<GuildQuestStore>,
}

impl GuildQuestJudged {
    /// A reader's answer: the pushed value, no records, no stores.
    fn silent(answer: quest::manager::GuildAnswer) -> Self {
        Self {
            answer,
            own: Vec::new(),
            stores: Vec::new(),
        }
    }
}

/// One store half a `guild.*` quest call queued (230s): what the descriptor writes after
/// the step, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildQuestStore {
    /// A master change: the guild row's new master, the new master's grade 1, the old
    /// master's grade 15 (`db/GuildManager.cpp:1175-1192`).
    Master {
        /// The guild id.
        guild_id: u32,
        /// The old master's PID.
        from: u32,
        /// The new master's PID.
        to: u32,
    },
    /// A comment post: `AddComment`'s insert (`G/guild.cpp:1066-1068`).
    Comment {
        /// The guild id.
        guild_id: u32,
        /// The poster's PID.
        pid: u32,
        /// The judged content, as bytes.
        content: Vec<u8>,
    },
    /// A ladder change: `ChangeLadderPoint`'s delta (`G/guild_war.cpp:753-758`).
    Ladder {
        /// The guild id.
        guild_id: u32,
        /// The judged ladder.
        ladder: i32,
    },
    /// A skill set: `SetSkillLevel`'s and `SetSkillPoint`'s `SendDBSkillUpdate` whole-row
    /// write (`G/guild.cpp:1262,2304`).
    Skills {
        /// The guild id.
        guild_id: u32,
        /// The 12 skill bytes after the set.
        skills: [u8; GUILD_SKILL_COUNT],
        /// The skill point after the set.
        skill_point: u8,
    },
    /// An experience offer: `OfferExp`'s `amount / 100` (`G/guild.cpp:974`), as
    /// [`GuildGradeSet::Offer`] carries it.
    Offer {
        /// The speaker's PID, whose `offer_exp` grows and whose `POINT_EXP` fell.
        pid: u32,
        /// The floored debit the speaker's `POINT_EXP` fell by.
        floored: u32,
        /// The levelled delta.
        offer: u32,
        /// The guild id.
        guild_id: u32,
        /// The guild's judged exp.
        exp: u32,
        /// The guild's judged level.
        level: u8,
        /// The guild's judged ladder.
        ladder: i32,
    },
    /// A free grant: `give_exp0`'s loop answer (no debit, no member row).
    Grant {
        /// The guild id.
        guild_id: u32,
        /// The guild's judged exp.
        exp: u32,
        /// The guild's judged level.
        level: u8,
        /// The guild's judged ladder.
        ladder: i32,
    },
}

/// The commit half of a grade-arm step: the speaker's VID plus the judged value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildGradeCommit {
    /// The VID of the character whose client sent the arm.
    pub speaker: Vid,
    /// The guild id.
    pub guild_id: u32,
    /// What the descriptor stored.
    pub set: GuildGradeSet,
}

/// What one `GUILD_SUBHEADER_CG_USE_SKILL` step judged (`CGuild::UseSkill`,
/// `G/guild.cpp:1267-1418`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildSkillUseAnswer {
    /// The descriptor writes the power, then sends [`GuildSkillUseCommit`]: the world
    /// judged every gate and the store has not been asked yet. The strips already
    /// landed and the cooldown is stamped; legacy ran everything synchronously, but
    /// a failed store closes the session, so the early landing is unobservable (the
    /// charge precedent judges with world mutation too).
    Store {
        /// The guild id.
        guild_id: u32,
        /// The cast vnum.
        vnum: u32,
        /// The target PID, as the client sent it (the teleport reads it).
        pid: u32,
        /// The guild's judged power, after the cost.
        power: u32,
    },
    /// The world committed the cast: the power landed, the `SKILL_INFO` fanned, and
    /// the branch ran (the teleport order or the war-gate chat). Every line rode the
    /// outbox at judge/commit, so the descriptor sends nothing itself.
    Committed,
    /// A gate refused: the line legacy sends (`[LS;523]`, `[LS;931]`, `[LS;932]`),
    /// or nothing for the quiet arms (auth, stun, range, row, level, held). The
    /// descriptor sends the line, if any, and stores nothing.
    Refused {
        /// The line, if the arm sends one.
        line: Option<Vec<u8>>,
    },
}

/// The commit half of a use-skill step: the speaker's VID plus the judged cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildSkillUseCommit {
    /// The VID of the character whose client sent the cast.
    pub speaker: Vid,
    /// The guild id.
    pub guild_id: u32,
    /// The cast vnum.
    pub vnum: u32,
    /// The target PID, as the client sent it.
    pub pid: u32,
    /// The judged power, after the cost.
    pub power: u32,
}

/// What one `/gskillup` line climbed (`do_guildskillup`, `G/cmd_general.cpp:878-904`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuildSkillUpAnswer {
    /// The climbed skill, or `None` when a gate refused.
    pub climbed: Option<GuildSkillUp>,
}

/// One landed `/gskillup`: the vnum, the level it climbed to, and the whole row the
/// store writes (`SaveSkill`: power, point and blob).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuildSkillUp {
    /// The guild id.
    pub guild_id: u32,
    /// The climbed vnum.
    pub vnum: u32,
    /// The level it climbed to.
    pub level: u8,
    /// The skill point left.
    pub point: u8,
    /// The 12 skill bytes after the climb.
    pub skills: [u8; GUILD_SKILL_COUNT],
}

/// What one `/war` line did (`do_war`, `G/cmd_general.cpp:2028-2210`). The world fans
/// every record but the start notice itself; the descriptor fans that, per client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuildWarAnswer {
    /// The start notice's two guild names when a declare or an accept landed — declarer
    /// then declaree for `[LS;475]` (`CGuildManager::DeclareWar`,
    /// `G/guild_manager.cpp:552-561`), acceptor then declarer for `[LS;477]`
    /// (`CGuildManager::StartWar`, `:629-651`, which names `from, to` as the accept sent
    /// them).
    pub notice: Option<GuildWarNotice>,
}

/// Which notice a `/war` line's names ride: the declare's `[LS;475]` or the accept's
/// `[LS;477]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildWarNotice {
    /// A declare landed: declarer, declaree.
    Declare {
        /// The declaring guild's name.
        from_name: Vec<u8>,
        /// The declared-on guild's name.
        to_name: Vec<u8>,
    },
    /// An accept started the war: acceptor, declarer.
    Accept {
        /// The accepting guild's name.
        from_name: Vec<u8>,
        /// The declaring guild's name.
        to_name: Vec<u8>,
    },
}
/// What one war end judged: both rows for the descriptor's store write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarEndOutcome {
    /// The lower guild's row.
    pub lo: WarEndRow,
    /// The higher guild's row.
    pub hi: WarEndRow,
}

/// One guild's judged end row: the ladder and the tallies `set_war_tallies` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarEndRow {
    /// The guild id.
    pub guild_id: u32,
    /// The judged ladder.
    pub ladder: i32,
    /// The judged wins.
    pub wins: i32,
    /// The judged draws.
    pub draws: i32,
    /// The judged losses.
    pub losses: i32,
}

/// One `/gwlist` line (`CGuildManager::ShowGuildWarList`, `G/guild_manager.cpp:785-800`):
/// a live pair with both guilds' names and ids and the war's age in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildWarListLine {
    /// The lower id's guild name.
    pub lo_name: Vec<u8>,
    /// The lower guild id.
    pub lo_id: u32,
    /// The higher id's guild name.
    pub hi_name: Vec<u8>,
    /// The higher guild id.
    pub hi_id: u32,
    /// `get_global_time() - A->GetWarStartTime(B->GetID())`.
    pub seconds: u32,
}

/// Fill `outcome`'s row for `id` from the live `guild`.
fn outcome_row(outcome: &mut WarEndOutcome, id: u32, guild: &Guild) {
    let row = if id == outcome.lo.guild_id {
        &mut outcome.lo
    } else {
        &mut outcome.hi
    };
    row.ladder = guild.ladder_point;
    row.wins = guild.war_wins;
    row.draws = guild.war_draws;
    row.losses = guild.war_losses;
}

impl GameState {
    /// The guild id of `pid`'s guild, if any (`CGuildManager::GetLinkedGuild`,
    /// `G/guild_manager.cpp:126-134`).
    pub(super) fn guild_of(&self, pid: u32) -> Option<u32> {
        self.guilds.of.get(&pid).copied()
    }

    /// The guild fields `EncodeInsertPacket` and `UpdatePacket` write for the character of
    /// `pid` (`dwGuildID` and `dwNewIsGuildName`, `G/char.cpp:1176-1199` and
    /// `:1327-1352`): the guild id, and 3 for the master, 2 for a general, 1 for a plain
    /// member, 0 with no guild. The member map answers the general flag both times —
    /// the memory read `char.cpp:1186` takes, not the store re-read `:1336`.
    pub(super) fn view_guild(&self, pid: u32) -> (u32, u8) {
        let Some(id) = self.guild_of(pid) else {
            return (0, 0);
        };
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return (0, 0);
        };
        if guild.master_pid == pid {
            return (id, 3);
        }
        let general = guild
            .members
            .get(&pid)
            .is_some_and(|member| member.is_general != 0);
        (id, u8::from(general) + 1)
    }

    /// The guild fields of the character online under `vid`, `(0, 0)` when none is.
    pub(super) fn view_guild_of_vid(&self, vid: u32) -> (u32, u8) {
        self.characters
            .find_by_vid(Vid::new(vid))
            .map_or((0, 0), |character| self.view_guild(character.player_id()))
    }

    /// `ch->SendGuildName(guild)` for the viewer `to` (`G/char.cpp:8621-8642`): the NAME
    /// record, once per guild per viewer — a known id sends nothing. `None` when the viewer
    /// has no body, when no guild holds `id`, or when id 0 names the guildless.
    pub(super) fn send_guild_name(&mut self, to: u32, id: u32) -> Option<Vec<u8>> {
        let name = self.guild_named(id);
        let body = self.bodies.get_mut(&Vid::new(to))?;
        if !body.known_guild.insert(id) || name.is_empty() {
            return None;
        }
        Some(guild_name_record(id, &name))
    }

    /// `ch->GetGuild() != NULL` for the character online under `vid`
    /// (`G/input_main.cpp:976`): its PID holds a guild. The descriptor asks before judging
    /// a guild chat line.
    pub(super) fn holds_guild(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .is_ok_and(|character| self.guild_of(character.player_id()).is_some())
    }
    /// `m_memberOnline` for the character online under `vid` (`G/guild.h:386`): the linked
    /// members' VIDs in PID order, the speaker included. Empty when the character holds no
    /// guild. Legacy walks a pointer set; the Rewrite goes in PID order (V1).
    pub(super) fn guild_member_vids(&self, vid: Vid) -> Vec<Vid> {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| self.guild_of(character.player_id()))
            .and_then(|id| self.guilds.by_id.get(&id))
            .map(|guild| {
                guild
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Load every stored guild, as `CGuildManager::Initialize` loads them
    /// (`G/guild_manager.cpp:212-243`) through `CGuild::Load` (`G/guild.cpp:642-675`): the
    /// rows, the grades in grade order, and the members in pid order, each PID linked.
    /// Grades outside 1-15 are dropped (`LoadGuildGradeData`, `G/guild.cpp:590-591`).
    /// `power` is clamped into `0..=max_power` (`ComputeGuildPoints`, `:1439-1444`).
    pub fn install_guilds(
        &mut self,
        stored: &db::guilds::StoredGuilds,
        marks: &[db::guilds::StoredMark],
        symbols: &[db::guilds::StoredSymbol],
    ) {
        for guild in &stored.guilds {
            let mut grades: [(Vec<u8>, u8); GUILD_GRADE_COUNT] =
                std::array::from_fn(|_| (Vec::new(), 0));
            for grade in stored
                .grades
                .iter()
                .filter(|grade| grade.guild_id == guild.id)
            {
                if (1..=15).contains(&grade.grade) {
                    grades[usize::from(grade.grade - 1)] =
                        (grade.name.as_bytes().to_vec(), grade.auth);
                }
            }
            let mut members = BTreeMap::new();
            for member in stored
                .members
                .iter()
                .filter(|member| member.guild_id == guild.id)
            {
                members.insert(
                    member.pid,
                    GuildMember {
                        grade: member.grade,
                        is_general: u8::from(member.is_general),
                        job: member.job,
                        level: member.level,
                        offer: member.offer,
                        name: member.name.as_bytes().to_vec(),
                        vid: None,
                    },
                );
                self.guilds.of.insert(member.pid, guild.id);
            }
            let mut skills = [0; GUILD_SKILL_COUNT];
            let len = guild.skills.len().min(GUILD_SKILL_COUNT);
            skills[..len].copy_from_slice(&guild.skills[..len]);
            let max_power =
                GUILD_BASE_POWER + u32::from(guild.level.saturating_sub(1)) * GUILD_POWER_PER_LEVEL;
            self.guilds.by_id.insert(
                guild.id,
                Guild {
                    id: guild.id,
                    master_pid: guild.master_pid,
                    exp: guild.exp,
                    level: guild.level,
                    name: guild.name.as_bytes().to_vec(),
                    grades,
                    skill_point: guild.skill_point,
                    skills,
                    power: max_power.min(u32::try_from(guild.sp.max(0)).unwrap_or(0)),
                    max_power,
                    gold: u32::try_from(guild.gold.max(0)).unwrap_or(0),
                    member_count_bonus: 0,
                    members,
                    ladder_point: guild.ladder_point,
                    war_wins: guild.win,
                    war_draws: guild.draw,
                    war_losses: guild.loss,
                    post_comment_due: 0,
                    usable: [true; GUILD_SKILL_COUNT],
                    usable_due: [None; GUILD_SKILL_COUNT],
                    enemies: BTreeMap::new(),
                },
            );
        }
        self.install_marks(marks, symbols);
    }

    /// Seed the mark images and the symbols from the stored rows (`LoadMarkIndex` +
    /// `LoadMarkImages` + `LoadSymbol`, `G/guild_manager.cpp:240-242`): each mark id is
    /// recorded and its 768 image bytes drawn, each symbol stored and hashed. Rows for
    /// unknown guilds are dropped, as `LoadMarkImages` only touches images the index
    /// names and the symbol map is only read through a guild; short images are dropped,
    /// as the TGA read would not fill the tile.
    fn install_marks(
        &mut self,
        marks: &[db::guilds::StoredMark],
        symbols: &[db::guilds::StoredSymbol],
    ) {
        for mark in marks {
            if !self.guilds.by_id.contains_key(&mark.guild_id) {
                continue;
            }
            if mark.image.len() != MARK_PIXELS * 4 {
                continue;
            }
            let mut pixels = [0_u32; MARK_PIXELS];
            for (i, pixel) in pixels.iter_mut().enumerate() {
                let at = i * 4;
                *pixel = u32::from_le_bytes([
                    mark.image[at],
                    mark.image[at + 1],
                    mark.image[at + 2],
                    mark.image[at + 3],
                ]);
            }
            if self.marks.add_mark_id(mark.guild_id, mark.mark_id) {
                self.marks.draw_mark(mark.mark_id, &pixels);
            }
        }
        for symbol in symbols {
            if !self.guilds.by_id.contains_key(&symbol.guild_id) {
                continue;
            }
            guild_mark::upload_symbol(&mut self.symbols, symbol.guild_id, &symbol.data);
        }
    }

    /// Judge one `MARK_UPLOAD` (`GuildMarkUpload`, `G/input_login.cpp:1059-1090`): the
    /// guild must exist and its level must reach `min_level`. An all-zero image deletes;
    /// anything else commits into the manager, keeping the guild's id or allocating the
    /// lowest free one. Answers the id for a save, `None` for a delete-or-refusal — the
    /// descriptor cannot tell those two apart, and must not try: both write nothing but
    /// the delete's row removal, which the answer flag carries.
    pub(super) fn judge_mark_upload(
        &mut self,
        guild_id: u32,
        image: &[u32; MARK_PIXELS],
        min_level: u8,
    ) -> MarkUploadAnswer {
        let Some(guild) = self.guilds.by_id.get(&guild_id) else {
            return MarkUploadAnswer::Quiet;
        };
        if guild.level < min_level {
            return MarkUploadAnswer::Quiet;
        }
        if image.iter().all(|&pixel| pixel == 0) {
            self.marks.delete_mark(guild_id);
            self.symbols.remove(&guild_id);
            return MarkUploadAnswer::Deleted;
        }
        let mark_id = self.marks.save_mark(guild_id, image);
        if mark_id == INVALID_MARK_ID {
            return MarkUploadAnswer::Quiet;
        }
        MarkUploadAnswer::Saved { mark_id }
    }

    /// Commit one judged symbol upload (`GuildSymbolUpload`'s store half,
    /// `G/input_login.cpp:1025-1026`): the bytes go into the map, replacing any earlier
    /// ones, and the descriptor persists them. Unknown guilds are refused: legacy's land
    /// gate closes their uploaders on a production server, and the port refuses them
    /// instead of closing.
    pub(super) fn commit_symbol_upload(&mut self, guild_id: u32, data: &[u8]) -> bool {
        if !self.guilds.by_id.contains_key(&guild_id) {
            return false;
        }
        guild_mark::upload_symbol(&mut self.symbols, guild_id, data);
        true
    }

    /// Answer one `SYMBOL_CRC` (`GuildSymbolCRC`, `G/input_login.cpp:1030-1057`): the
    /// stored bytes when the size or the CRC mismatches, `None` on a match or an
    /// unknown guild.
    pub(super) fn symbol_mismatch(&self, guild_id: u32, crc: u32, size: u32) -> Option<Vec<u8>> {
        let symbol = guild_mark::guild_symbol(&self.symbols, guild_id)?;
        if u64::from(size) == symbol.raw.len() as u64 && crc == symbol.crc {
            return None;
        }
        Some(symbol.raw.clone())
    }

    /// Link the character online under `vid` into its guild's online set, if it holds one
    /// (`CGuild::LoginMember`, `G/guild.cpp:270-296`): the LOGIN packet to every already
    /// online member, then the four self packets — grades, INFO, LIST, skills — plus the
    /// war send, which sends nothing with no wars (`SendEnemyGuild`,
    /// `G/guild_war.cpp:60-109`).
    pub(super) fn link_guild(&mut self, vid: Vid) {
        let linked_pid = self.characters.find_by_vid(vid).ok().and_then(|character| {
            let pid = character.player_id();
            self.guild_of(pid).map(|id| (pid, id))
        });
        let Some((pid, id)) = linked_pid else {
            return;
        };
        let linked = self
            .guilds
            .by_id
            .get_mut(&id)
            .and_then(|guild| guild.members.get_mut(&pid))
            .is_some_and(|member| {
                member.vid = Some(vid);
                true
            });
        if linked {
            self.guild_login_packets(vid, pid, id);
        }
    }

    /// Unlink the character online under `vid` from its guild's online set, if it holds one
    /// (`CGuild::LogoutMember`, `G/guild.cpp:316-334`): the LOGOUT packet to every member
    /// still online.
    pub(super) fn unlink_guild(&mut self, vid: Vid) {
        let linked_pid = self.characters.find_by_vid(vid).ok().and_then(|character| {
            let pid = character.player_id();
            self.guild_of(pid).map(|id| (pid, id))
        });
        let Some((pid, id)) = linked_pid else {
            return;
        };
        let linked = self
            .guilds
            .by_id
            .get_mut(&id)
            .and_then(|guild| guild.members.get_mut(&pid))
            .is_some_and(|member| {
                member.vid = None;
                true
            });
        if linked {
            self.guild_logout_packets(pid, id);
        }
    }

    /// Run one `GUILD_SUBHEADER_CG_ADD_MEMBER` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:2880-2899`, plus `CGuild::Invite`, `G/guild.cpp:2034-2113`): the
    /// guildless gate, the guest lookup by VID, the quest-running pair, the auth and
    /// empire gates, the joinable ladder, the dup-invite quiet return, and the invite:
    /// the pending row plus the `GUILD_INVITE` record to the guest alone.
    ///
    /// No store half: the invite lives in memory and lapses after 250 Pulses
    /// (`GuildInviteEvent`, `G/guild.cpp:2013-2031`).
    pub(super) fn guild_add_member(&mut self, vid: Vid, guest: u32, now: u32) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let Some(id) = self.guild_of(pid) else {
            self.tell_guild(vid, b"[LS;523]", &[]);
            return;
        };
        let invitee_vid = Vid::new(guest);
        let Ok(invitee_character) = self.characters.find_by_vid(invitee_vid) else {
            self.tell_guild(vid, b"<Guild> Couldn't find such a person.", &[]);
            return;
        };
        let guest_pid = invitee_character.player_id();
        // `@fixme145`: `characters` holds PCs only — NPCs live in `npcs`/`npc_of` — so a
        // VID found here is always a PC and the pair gate is vacuous.
        if self.quest_running(vid) {
            self.tell_guild(vid, b"[LS;1058]", &[]);
            return;
        }
        if self.quest_running(invitee_vid) {
            return;
        }
        // `BLOCK_GUILD_INVITE`: block modes are unported, so nobody blocks.
        // Both empire reads use `card.empire`: `characters` holds PCs only, so a VID
        // found here always has a card.
        let empires_differ = self.bodies.get(&vid).map(|body| body.card.empire)
            != self.bodies.get(&invitee_vid).map(|body| body.card.empire);
        let auth = self
            .guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.grade_auth_of(pid));
        let no_auth = auth.is_none_or(|auth| (auth & GUILD_AUTH_ADD_MEMBER) == 0);
        // `Invite`'s order (`G/guild.cpp:2046-2059`): block, auth, empire. `HasGradeAuth`
        // indexes `grade_array[grade-1]` on the member row (`G/guild.h:242`); a speaker
        // with no row cannot happen — `of` links only rows — but reads as no-auth.
        if no_auth {
            self.tell_guild(vid, b"[LS;941]", &[]);
            return;
        }
        if empires_differ {
            self.tell_guild(vid, b"[LS;942]", &[]);
            return;
        }
        if let Some(line) = self.joinable_line(id, guest_pid, vid, false, now) {
            let _sent = self.write_to_client(vid, line);
            return;
        }
        if self.guilds.invites.contains_key(&(id, guest_pid)) {
            return;
        }
        let due = self.last_pulse.saturating_add(GUILD_INVITE_WAIT_PULSES);
        self.guilds.invites.insert((id, guest_pid), due);
        if let Some(guild) = self.guilds.by_id.get(&id) {
            let name = guild.name.clone();
            let _sent = self.write_to_client(invitee_vid, guild_invite_record(id, &name));
        }
    }

    /// `ChatPacket(CHAT_TYPE_INFO, format, ...)` to the player under `vid`.
    fn tell_guild(&self, vid: Vid, format: &[u8], args: &[Arg<'_>]) {
        if let Some(mover) = self.own_mover(vid) {
            self.tell(vid, mover, CHAT_TYPE_INFO, format, args);
        }
    }

    /// `VerifyGuildJoinableCondition`'s ladder (`G/guild.cpp:2165-2208`) as the line `to`
    /// hears, or `None` when the invitee may join: the withdraw penalty, the disband
    /// penalty, the already-guilded arm, and the full arm (`:2167-2181`). The war arm is
    /// vacuous (no wars) and the invite-limit arm is vacuous (`g_bGuildInviteLimit` is
    /// false, `G/config.cpp:46`). The line goes to `to` — the inviter on invite, the
    /// invitee on accept — and the accept texts differ by one word each (`InviteAccept`,
    /// `:2127-2144`), which `accept` selects. `now` is `get_global_time()` in seconds,
    /// which the descriptor reads: the world's own clock counts boot milliseconds and
    /// cannot be compared with the epoch-second quest flags.
    fn joinable_line(
        &self,
        id: u32,
        guest_pid: u32,
        to: Vid,
        accept: bool,
        now: u32,
    ) -> Option<Vec<u8>> {
        let now = i64::from(now);
        let quest_flag = |name: &[u8]| {
            self.quests
                .as_ref()
                .and_then(|quests| quests.manager().quest_flag(guest_pid, name))
                .unwrap_or(0)
        };
        let event_flag = |name: &[u8]| {
            self.quests
                .as_ref()
                .map_or(0, |quests| quests.manager().event_flag(name))
        };
        let mover = self.own_mover(to)?;
        let line = |format: &[u8], args: &[Arg<'_>]| {
            chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, format, args)
        };
        // `GetWithdrawDelay` / `GetDisbandDelay` (`G/guild_manager.cpp:67-75`): the event
        // flag times 86400 — the Rewrite has no test server, so the 60 branch is dead.
        if now - i64::from(quest_flag(b"guild_manage.new_withdraw_time"))
            < i64::from(event_flag(b"guild_withdraw_delay")) * DAY_SECONDS
        {
            let flag = event_flag(b"guild_withdraw_delay");
            let text: &[u8] = if accept {
                b"<Guild> You cannot invite people within %d days after leaving the guild."
            } else {
                b"<Guild> You cannot invite people within %d days of leaving the guild."
            };
            return Some(line(text, &[Arg::Int(i64::from(flag))]));
        }
        if now - i64::from(quest_flag(b"guild_manage.new_disband_time"))
            < i64::from(event_flag(b"guild_disband_delay")) * DAY_SECONDS
        {
            let flag = event_flag(b"guild_disband_delay");
            return Some(line(
                b"<Guild> You cannot invite anyone within %d days of disbanding the guild.",
                &[Arg::Int(i64::from(flag))],
            ));
        }
        if self.guild_of(guest_pid).is_some() {
            return Some(line(
                b"<Guild> The opponent already belongs to another guild.",
                &[],
            ));
        }
        if self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| guild.members.len() >= usize::from(guild.max_member_count()))
        {
            return Some(line(
                b"<Guild> You have exceeded the maximum number of guild members.",
                &[],
            ));
        }
        None
    }

    /// Judge one `GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3144-3160`, plus `InviteAccept`/`InviteDeny`,
    /// `G/guild.cpp:2115-2162`): a missing guild is quiet; a deny removes the invite and
    /// is quiet; an accept without an invite is quiet; an accept re-runs the ladder to
    /// the invitee, then answers [`GuildJoinAnswer::Store`] for the descriptor to insert
    /// the member row.
    pub(super) fn answer_guild_invite(
        &mut self,
        vid: Vid,
        guild_id: u32,
        accept: u8,
        now: u32,
    ) -> GuildJoinAnswer {
        let quiet = |vid: Vid| GuildJoinAnswer::Refused {
            to: vid,
            line: None,
        };
        if !self.guilds.by_id.contains_key(&guild_id) {
            return quiet(vid);
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return quiet(vid);
        };
        let pid = character.player_id();
        // The deny removes the invite too (`InviteDeny`, `:2152-2162`): it answers nothing
        // either way, so the removal is folded into the one check. An accept without a
        // row is quiet (`InviteAccept`, `:2116-2123`); a ladder-refused accept keeps the
        // removal, as legacy's erased-before-judging event does (`:2125-2144`).
        if self.guilds.invites.remove(&(guild_id, pid)).is_none() {
            return quiet(vid);
        }
        if accept == 0 {
            return quiet(vid);
        }
        if let Some(line) = self.joinable_line(guild_id, pid, vid, true, now) {
            return GuildJoinAnswer::Refused {
                to: vid,
                line: Some(line),
            };
        }
        // `RequestAddMember`'s quiet arms (`G/guild.cpp:144-154`): a guilded invitee and a
        // duplicate row join nothing. The ladder already refused the guilded, so only the
        // row is re-checked — a row without an `of` entry cannot happen, because the two
        // move together, but the check is the shape of legacy's.
        if self
            .guilds
            .by_id
            .get(&guild_id)
            .is_some_and(|guild| guild.members.contains_key(&pid))
        {
            return quiet(vid);
        }
        GuildJoinAnswer::Store { guild_id }
    }

    /// Commit one judged join under the stored member row (`AddMember`, `G/guild.cpp:168-193`
    /// plus the online invitee's `LoginMember`, `:270-290`): the row lands at grade 15 —
    /// overwriting the five synced fields when it exists (`:176-184`) — the PID links, the
    /// LIST-one and the LOGIN fan to the already-online members, and the answer carries the
    /// joiner's walk for the descriptor to send on its own session. The joiner hears no
    /// LIST-one of itself (`SendListOneToAll` at `:182`, before `m_memberOnline.insert`
    /// at `:286`). An offline joiner keeps the row with no fan and no walk (`FindByPID`
    /// NULL takes the retired P2P arm, `:186-193`).
    ///
    /// The joiner's name, job and level come from its body, as legacy's
    /// `TPacketDGGuildMember` reads them from the player row.
    pub(super) fn commit_guild_join(&mut self, commit: &GuildJoinCommit) -> GuildJoinAnswer {
        let GuildJoinCommit { guild_id, pid } = *commit;
        let offline = GuildJoinAnswer::Committed { own: Vec::new() };
        let join_vid = self.characters.find_by_pid(pid).ok().map(Character::vid);
        let (job, level, name) = join_vid
            .and_then(|vid| self.bodies.get(&vid))
            .map(|body| {
                (
                    body.card.job,
                    body.card.level,
                    body.card.name.as_bytes().to_vec(),
                )
            })
            .unwrap_or_default();
        // `AddMember`'s upsert (`:172-184`): a fresh row holds the store's grade,
        // is-general and offer — the judge stored grade 15, a joiner is no general and
        // offers nothing — while a row already there keeps those three and the five
        // synced fields land on it. A named PID keeps its name too: the DB packet's
        // `szName` (`:168`) names the player row, which has not changed.
        let row = self.guilds.by_id.get_mut(&guild_id).and_then(|guild| {
            guild
                .members
                .entry(pid)
                .and_modify(|row| {
                    row.job = job;
                    row.level = level;
                    row.offer = 0;
                })
                .or_insert(GuildMember {
                    grade: JOIN_GRADE,
                    is_general: 0,
                    job,
                    level,
                    offer: 0,
                    name,
                    vid: join_vid,
                });
            guild.members.get(&pid).map(|row| GcGuildMember {
                pid,
                grade: row.grade,
                is_general: row.is_general,
                job: row.job,
                level: row.level,
                offer: row.offer,
                name_flag: 1,
                name: pad_member_name(&row.name),
            })
        });
        let Some(one) = row else {
            return offline;
        };
        self.guilds.of.insert(pid, guild_id);
        // The already-online members, snapshotted before the joiner's link lands: the
        // LIST-one fans to them alone (`SendListOneToAll` at `:182`, before
        // `m_memberOnline.insert` at `:286`).
        let members: Vec<Vid> = self
            .guilds
            .by_id
            .get(&guild_id)
            .map(|guild| {
                guild
                    .members
                    .iter()
                    .filter(|(member_pid, _)| **member_pid != pid)
                    .filter_map(|(_, member)| member.vid)
                    .collect()
            })
            .unwrap_or_default();
        if join_vid.is_none() {
            return GuildJoinAnswer::Committed { own: Vec::new() };
        }
        let one_record = guild_list_record(&[one]);
        let join_login = guild_login_record(pid);
        for member in &members {
            let _sent = self.write_to_client(*member, one_record.clone());
        }
        for member in &members {
            let _sent = self.write_to_client(*member, join_login.clone());
        }
        let Some(guild) = self.guilds.by_id.get(&guild_id) else {
            return offline;
        };
        let grades: [GcGuildGrade; GUILD_GRADE_COUNT] = std::array::from_fn(|index| {
            let (name, auth) = &guild.grades[index];
            GcGuildGrade {
                name: pad_grade_name(name),
                auth: *auth,
            }
        });
        let mut own = vec![
            guild_grade_record(&grades),
            guild_info_record(&guild.info()),
            guild_list_record(&guild.list()),
        ];
        for other_pid in guild
            .members
            .iter()
            .filter_map(|(pid, member)| member.vid.map(|_| pid))
        {
            own.push(guild_login_record(*other_pid));
        }
        own.push(guild_skill_info_record(&guild.skill_info()));
        GuildJoinAnswer::Committed { own }
    }

    /// `LoginMember`'s sends for the member under `vid` (`G/guild.cpp:270-296`): the LOGIN
    /// packet to every member already online, then the five self sends — grades, INFO,
    /// LIST, skills — plus the LOGIN packets for every online member, which `SendListPacket`
    /// sends after the rows (`G/guild.cpp:474-484`): the entrant hears itself too, because it
    /// joined the online set first. Then the war send (`SendEnemyGuild`,
    /// `G/guild_war.cpp:60-109`): each enemy's NAME plus its WAR record, and each on-war
    /// enemy's score pair — own score, then the opp's score against self.
    fn guild_login_packets(&mut self, vid: Vid, pid: u32, id: u32) {
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return;
        };
        for other in guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .filter(|other| *other != vid)
        {
            let _sent = self.write_to_client(other, guild_login_record(pid));
        }
        let grades: [GcGuildGrade; GUILD_GRADE_COUNT] = std::array::from_fn(|index| {
            let (name, auth) = &guild.grades[index];
            GcGuildGrade {
                name: pad_grade_name(name),
                auth: *auth,
            }
        });
        let _sent = self.write_to_client(vid, guild_grade_record(&grades));
        let _sent = self.write_to_client(vid, guild_info_record(&guild.info()));
        let _sent = self.write_to_client(vid, guild_list_record(&guild.list()));
        for other_pid in guild
            .members
            .iter()
            .filter_map(|(pid, member)| member.vid.map(|_| pid))
        {
            let _sent = self.write_to_client(vid, guild_login_record(*other_pid));
        }
        let _sent = self.write_to_client(vid, guild_skill_info_record(&guild.skill_info()));
        let enemies: Vec<(u32, u8, u8, u32)> = guild
            .enemies
            .iter()
            .map(|(opp, enemy)| (*opp, enemy.war_type, enemy.state, enemy.score))
            .collect();
        for (opp, war_type, state, score) in enemies {
            if let Some(name) = self.send_guild_name(vid.raw(), opp) {
                let _sent = self.write_to_client(vid, name);
            }
            let _sent = self.write_to_client(vid, guild_war_record(id, opp, war_type, state));
            if state == GUILD_WAR_ON_WAR {
                let _sent = self.write_to_client(vid, guild_war_score_record(id, opp, score));
                // The opp's score against self; a guild the world lost reads 0 (legacy's
                // `TouchGuild` always answers, so this arm is unreachable there).
                let back = self
                    .guilds
                    .by_id
                    .get(&opp)
                    .and_then(|other| other.enemies.get(&id))
                    .map_or(0, |enemy| enemy.score);
                let _sent = self.write_to_client(vid, guild_war_score_record(opp, id, back));
            }
        }
    }

    /// `LogoutMember`'s sends for `pid` (`G/guild.cpp:316-334`).
    fn guild_logout_packets(&self, pid: u32, id: u32) {
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return;
        };
        for other in guild.members.values().filter_map(|member| member.vid) {
            let _sent = self.write_to_client(other, guild_logout_record(pid));
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_REMOVE_MEMBER` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:2901-2954`, plus `RequestRemoveMember`,
    /// `G/guild.cpp:195-211`): the guildless gate, the row's absence, the leader's
    /// quiet refusal, and the two arms — online (the foreign-guild line, the no-auth
    /// line, the withdraw stamp, then the store) and offline (no-auth at the judge,
    /// the forced-leave line after the store, `"Couldn't find such a person."` when
    /// the row is gone).
    ///
    /// This judge sends nothing itself: the pre lines go out before the descriptor
    /// deletes the row, the post line after. The withdrawal stamp lands in the judge
    /// because `Manager::set_quest_flag` creates the PC record, as legacy's
    /// `SetQuestFlag` does (`G/input_main.cpp:2940`).
    pub(super) fn answer_guild_remove(
        &mut self,
        vid: Vid,
        pid: u32,
        now: u32,
    ) -> GuildRemoveAnswer {
        let no_guild = GuildRemoveAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return no_guild;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildRemoveAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return no_guild;
        };
        // The war arm (`G/input_main.cpp:2901-2905`): a guild under any war removes
        // nobody — before the row lookup, so a bogus PID chats the line too.
        if self.guild_under_any_war(id) {
            return GuildRemoveAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You cannot leave a guild member during a Guild War.",
                    &[],
                ),
            };
        }
        let Some(row) = guild.members.get(&pid) else {
            return GuildRemoveAnswer::Refused {
                line: self.guild_line(vid, b"<Guild> Couldn't find such a person.", &[]),
            };
        };
        if row.grade == GUILD_LEADER_GRADE {
            // `RequestRemoveMember`'s leader arm (`G/guild.cpp:202-203`): quiet — the
            // arm's `else` branch never runs, so no forced-leave line and no
            // couldn't-find line.
            return GuildRemoveAnswer::Refused { line: None };
        }
        // `HasGradeAuth` (`G/guild.h:242`) on the speaker's row: a speaker with no row
        // cannot happen — `of` links only rows — but reads as no-auth.
        let has_remove = self
            .guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.grade_auth_of(speaker))
            .is_some_and(|auth| (auth & GUILD_AUTH_REMOVE_MEMBER) != 0);
        let online = row.vid.is_some();
        if online {
            // `FindByPID`'s foreign-guild arm (`G/input_main.cpp:2926-2932`): the victim
            // is online but holds another guild. A victim online in this guild holds
            // this guild — the link and the row move together.
            if self.guild_of(pid) != Some(id) {
                return GuildRemoveAnswer::Refused {
                    line: self.guild_line(
                        vid,
                        b"<Guild> The opponent is not from the same guild.",
                        &[],
                    ),
                };
            }
            // The online no-auth line chats *and* the removal runs
            // (`G/input_main.cpp:2934-2945`): the line is a scold, not a refusal.
            let pre = if has_remove {
                None
            } else {
                self.guild_line(
                    vid,
                    b"<Guild> You do not have the right to forcibly leave the guild.",
                    &[],
                )
            };
            if let Some(quests) = self.quests.as_mut() {
                quests.manager_mut().set_quest_flag(
                    pid,
                    b"guild_manage.new_withdraw_time",
                    i32::try_from(now).unwrap_or(i32::MAX),
                );
            }
            GuildRemoveAnswer::Store {
                pid,
                online: true,
                pre,
            }
        } else if !has_remove {
            GuildRemoveAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have the right to forcibly leave the guild.",
                    &[],
                ),
            }
        } else {
            GuildRemoveAnswer::Store {
                pid,
                online: false,
                pre: None,
            }
        }
    }

    /// `ChatPacket(CHAT_TYPE_INFO, format, ...)` as bytes for the player under `vid`, or
    /// `None` when the speaker has no mover — a speaker found in `characters` always
    /// has one, so the `None` arm never runs, but its type is the shape of the line.
    fn guild_line(&self, vid: Vid, format: &[u8], args: &[Arg<'_>]) -> Option<Vec<u8>> {
        self.own_mover(vid)
            .map(|mover| chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, format, args))
    }

    /// Commit one judged removal under the deleted member row (`RemoveMember`,
    /// `G/guild.cpp:213-249`): the victim unlinks, the REMOVE fans through the outboxes
    /// to the still-online members, and the answer carries the line legacy chats *after*
    /// the store (the offline arm's forced-leave line), if any, for the descriptor to
    /// send on its own session. The fan excludes the victim: the row's link leaves first
    /// (`:230,memberOnline.erase`), then the fan reaches the rest (`:336-354`).
    pub(super) fn commit_guild_remove(&mut self, commit: &GuildRemoveCommit) -> GuildRemoveAnswer {
        let GuildRemoveCommit {
            speaker,
            pid,
            online,
        } = *commit;
        let empty = GuildRemoveAnswer::Committed { post: None };
        let Some(id) = self.guild_of(pid) else {
            return empty;
        };
        self.guilds.of.remove(&pid);
        let fan = guild_remove_record(pid);
        let members: Vec<Vid> = self
            .guilds
            .by_id
            .get_mut(&id)
            .and_then(|guild| {
                guild.members.remove(&pid)?;
                Some(
                    guild
                        .members
                        .values()
                        .filter_map(|member| member.vid)
                        .collect(),
                )
            })
            .unwrap_or_default();
        for member in members {
            let _sent = self.write_to_client(member, fan.clone());
        }
        let post = if online {
            None
        } else {
            self.guild_line(speaker, b"<Guild> Guild members were forced to leave.", &[])
        };
        GuildRemoveAnswer::Committed { post }
    }

    /// The `[LS;523]` line to the player under `vid`, for the arms the dispatcher
    /// answers: the guildless gate chats, while a non-row or a missing guild closes.
    pub(super) fn guild_line_of(&self, vid: Vid) -> Option<Vec<u8>> {
        self.guild_line(vid, b"[LS;523]", &[])
    }

    /// The speaker's grade, or `None` when the speaker holds no row in `id`
    /// (`GetMember(ch->GetPlayerID())` NULL, which every arm's `m == NULL` answers with
    /// `-1` — a close).
    fn guild_speaker_grade(&self, id: u32, speaker: u32) -> Option<u8> {
        self.guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.members.get(&speaker))
            .map(|row| row.grade)
    }

    /// Judge one `GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:2956-2979`, plus `ChangeGradeName`, `G/guild.cpp:822-857`): the
    /// guildless gate, the leader gate, the grade-1 gate, the name shape, and the
    /// judged name. The 10-byte body is the grade byte plus the 9-byte field, which
    /// `strlcpy(gradename, c_pData + 1, 9)` reads (`:2958`): the bytes before the
    /// first NUL, truncated to 8 — a NUL-less field names its first 8 bytes, with no
    /// over-read, because the frame check holds 9 field bytes.
    ///
    /// The name is judged as the guild name is: the bytes before the first NUL, as
    /// ASCII letters and digits of length 2-8 (`check_name_alphabet` on europe,
    /// `G/locale_service.cpp:317-337,1156`, plus the 9-byte field's length), refusing
    /// the banned words and the mob names (`check_name_independent`, `:86-99`).
    /// `ChangeGradeName`'s own `strlen > 8` and empty arms (`:843-848`) are vacuous
    /// past the judge.
    pub(super) fn answer_grade_name(&self, vid: Vid, grade: u8, field: &[u8]) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(speaker_grade) = self.guild_speaker_grade(id, speaker) else {
            return close;
        };
        if speaker_grade != GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have permission to change the position name.",
                    &[],
                ),
            };
        }
        if grade == GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> Guild leader position names cannot be changed.",
                    &[],
                ),
            };
        }
        if !(2..=GUILD_GRADE_COUNT_U8).contains(&grade) {
            // `ChangeGradeName`'s `grade > 15` arm (`G/guild.cpp:831-836`): quiet, as
            // `sys_err` is.
            return close;
        }
        let end = field
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(field.len())
            .min(GUILD_GRADE_NAME_MAX_LEN);
        let name = &field[..end];
        if name.len() < 2
            || !name.iter().all(u8::is_ascii_alphanumeric)
            || !self.name_rules.guild_name_ok(name)
        {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"<Guild> Invalid position name.", &[]),
            };
        }
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::GradeName {
                grade,
                name: String::from_utf8_lossy(name).into_owned(),
            },
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY` (`CInputMain::Guild`'s
    /// arm, `G/input_main.cpp:2982-3000`, plus `ChangeGradeAuth`,
    /// `G/guild.cpp:870-913`): the guildless gate, the leader gate, the grade-1
    /// gate, and the judged auth. The auth byte rides the body whole — no mask —
    /// as legacy stores `*(c_pData + 1)` (`:2998`).
    pub(super) fn answer_grade_auth(&self, vid: Vid, grade: u8, auth: u8) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(speaker_grade) = self.guild_speaker_grade(id, speaker) else {
            return close;
        };
        if speaker_grade != GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have the right to change the position authority.",
                    &[],
                ),
            };
        }
        if grade == GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> The guild leader's authority cannot be changed.",
                    &[],
                ),
            };
        }
        if !(2..=GUILD_GRADE_COUNT_U8).contains(&grade) {
            // `ChangeGradeAuth`'s `grade > 15` arm (`G/guild.cpp:876-880`): quiet, as
            // `sys_err` is.
            return close;
        }
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::GradeAuth { grade, auth },
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3062-3081`, plus `ChangeMemberGrade`,
    /// `G/guild.cpp:1190-1229`): the guildless gate, the leader gate, the own-row
    /// gate, the grade-1 gate, and the judged grade. A victim with no row stays
    /// quiet (`ChangeMemberGrade`'s missing-member arm, `:1195-1198`); a victim of
    /// another guild answers `"Couldn't find such a person."`, because the row it
    /// names is not this guild's.
    pub(super) fn answer_member_grade(&self, vid: Vid, pid: u32, grade: u8) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(speaker_grade) = self.guild_speaker_grade(id, speaker) else {
            return close;
        };
        if speaker_grade != GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have the right to change positions.",
                    &[],
                ),
            };
        }
        if speaker == pid {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> The position of guild leader cannot be changed.",
                    &[],
                ),
            };
        }
        if grade == GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You cannot change your position to guild leader.",
                    &[],
                ),
            };
        }
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return close;
        };
        if !guild.members.contains_key(&pid) {
            let line = if self.guild_of(pid).is_some_and(|other| other != id) {
                self.guild_line(vid, b"<Guild> Couldn't find such a person.", &[])
            } else {
                None
            };
            return GuildGradeAnswer::Refused { line };
        }
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::MemberGrade { pid, grade },
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_OFFER` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3005-3029`, plus `OfferExp`, `G/guild.cpp:942-982`): the
    /// guildless gate, the max-level gate, and the judged floor. The frame holds a
    /// DWORD, so the wraparound (`m_data.exp+amount < m_data.exp`, `:949-950`) and
    /// negative (`:952-953`) arms are unreachable; the refusal line answers the only
    /// reachable failure, the speaker's own exp falling short (`[LS;929]`, `:958`).
    /// The exp the speaker keeps and spends lives in memory, so the judge debits it
    /// here, through `PointChange(POINT_EXP, -offer)` (`:964`), whose records ride
    /// the session. The store half debits the player row and the commit fans the
    /// loop's whole answer.
    pub(super) fn answer_guild_offer(&mut self, vid: Vid, sent: u32) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(row) = self
            .guilds
            .by_id
            .get(&id)
            .map(|guild| (guild.level, guild.exp))
        else {
            return close;
        };
        if row.0 >= GUILD_MAX_LEVEL {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> Your guild is already at the highest level.",
                    &[],
                ),
            };
        }
        let floored = sent / 100 * 100;
        let held = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(Character::points)
            .map_or(0, world::character::Points::exp);
        if held < i64::from(floored) {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;929]", &[]),
            };
        }
        {
            let max_level = self.exp_limits.max_level;
            let record = self
                .pc_points_mut(vid.raw())
                .and_then(|points| points.lose_exp(i64::from(floored), max_level));
            if let Some(record) = record {
                self.send_point_records(vid, &[record]);
            }
        }
        let step = floored / 100;
        let (exp, level, ladder) = self.judge_offer_loop(id, step);
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::Offer {
                pid: speaker,
                floored,
                offer: step,
                exp,
                level,
                ladder,
            },
        }
    }

    /// Run `GuildPointChange`'s exp loop over the judged `step` (`G/guild.cpp:1601-1635`)
    /// and answer the guild's exp, level and ladder: each level step spends the level's
    /// table cost, raises the level and the skill point, and adds the ladder step. A
    /// guild already at the top keeps its exp and level — the dispatch's max-level gate
    /// holds that arm — and a guild the loop carries to the top ends at exp 0.
    fn judge_offer_loop(&mut self, id: u32, step: u32) -> (u32, u8, i32) {
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return (0, 1, 0);
        };
        let mut exp = guild.exp.saturating_add(step);
        let mut level = guild.level;
        let mut ladder = guild.ladder_point;
        let mut skill_point = guild.skill_point;
        while level < GUILD_MAX_LEVEL && exp >= GUILD_LEVELUP_EXP[usize::from(level)] {
            exp -= GUILD_LEVELUP_EXP[usize::from(level)];
            level += 1;
            skill_point = skill_point.saturating_add(1);
            ladder = ladder.saturating_add(GUILD_LADDER_POINT_PER_LEVEL);
        }
        if level == GUILD_MAX_LEVEL {
            exp = 0;
        }
        let max_power =
            GUILD_BASE_POWER + u32::from(level.saturating_sub(1)) * GUILD_POWER_PER_LEVEL;
        guild.max_power = max_power;
        guild.power = max_power;
        guild.skill_point = skill_point;
        (exp, level, ladder)
    }

    /// Judge one `GUILD_SUBHEADER_CG_CHARGE_GSP` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3030-3047`, plus `ChargeSP`, `G/guild.cpp:1804-1831`): the
    /// guildless gate, the arithmetic gates, and the judged cap. `ENABLE_REMOVE_LIMIT_GOLD`
    /// is defined (`common/prodomodefines.h:157`), so the gold reads `unsigned long long`
    /// and the only reachable refusal is the speaker's gold falling short — the negative
    /// and wraparound arms stay as the code's shape, as the frame holds the `int`. The
    /// charge over the headroom caps to it (`:1813-1819`), debits the gold through
    /// `PointChange(POINT_GOLD, -gold)` (`:1822`), and the `[LS;938]` count answers at
    /// the commit. The store half lands the power half; the player row keeps the gold.
    pub(super) fn answer_guild_charge(&mut self, vid: Vid, sent: i32) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let gold = u64::try_from(sent.max(0)).unwrap_or(0).saturating_mul(100);
        let held = self.characters.find_by_vid(vid).map(Character::gold);
        let Ok(held) = held else {
            return close;
        };
        if sent < 0 || gold < u64::try_from(sent.max(0)).unwrap_or(0) || held < gold {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"<Guild> Money is insufficient.", &[]),
            };
        }
        let headroom = self
            .guilds
            .by_id
            .get(&id)
            .map_or(0, |guild| guild.max_power.saturating_sub(guild.power));
        let capped = u32::try_from(sent).unwrap_or(0).min(headroom);
        let debit = u64::from(capped).saturating_mul(100);
        // `PointChange(POINT_GOLD, -gold)` (`:1822`) is `NOT_PORTED` (gold lives on the
        // character, not the points), so the debit lands as the creation fee's does:
        // `set_gold` plus the `POINT_GOLD` record, whose `amount` is the negative debit
        // and whose `value` is the gold left.
        let after = held.saturating_sub(debit);
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(after);
        }
        self.send_point_records(
            vid,
            &[PointRecord {
                kind: u8::try_from(common::point_slot::POINT_GOLD).unwrap(),
                amount: -i64::try_from(debit).unwrap_or(i64::MAX),
                value: i64::try_from(after).unwrap_or(i64::MAX),
                broadcast: false,
            }],
        );
        let power = self
            .guilds
            .by_id
            .get(&id)
            .map_or(0, |guild| guild.power.saturating_add(capped));
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::Charge {
                pid: speaker,
                debit,
                power,
                gained: capped,
            },
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_POST_COMMENT` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3048-3080`, plus `AddComment`, `G/guild.cpp:1049-1072`): the
    /// guildless gate, the notice gate, the minute gate, and the judged content. The
    /// dispatch holds the frame's length byte (`length > 50` closes, `:3053-3060`);
    /// the judge holds the rest. An empty body is quiet (`str.length() == 0`,
    /// `:1052`); a `!`-first body without the notice auth chats the permission line
    /// (`:3071-3074`); a post inside the minute chats the Romanian countdown
    /// (`:1056-1062`), whose minutes and seconds floor from the Pulses left. The
    /// minute sets at the commit, after the insert — as `AddComment` sets
    /// `m_guildPostCommentPulse` after the query (`:1071`) — so a failed insert
    /// leaves the board open.
    pub(super) fn answer_post_comment(&self, vid: Vid, body: &[u8]) -> GuildCommentAnswer {
        let close = GuildCommentAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildCommentAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return close;
        };
        if !guild.members.contains_key(&speaker) {
            return close;
        }
        if body.is_empty() || body.len() > GUILD_COMMENT_MAX_LEN {
            return close;
        }
        let auth = guild.grade_auth_of(speaker).unwrap_or(0);
        if body[0] == b'!' && (auth & GUILD_AUTH_NOTICE) == 0 {
            return GuildCommentAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have permission to write notices.",
                    &[],
                ),
            };
        }
        if guild.post_comment_due > self.last_pulse {
            // `((due / 25) - (now / 25))` (`:1058`): the Pulses floor to seconds on
            // each side before the subtract, not after.
            let delta = guild.post_comment_due / 25 - self.last_pulse / 25;
            let minutes = delta / 60;
            let seconds = delta - minutes * 60;
            return GuildCommentAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"Poti posta un nou comentariu in: %02d minute si %02d secunde!",
                    &[
                        Arg::Int(i64::try_from(minutes).unwrap_or(i64::MAX)),
                        Arg::Int(i64::try_from(seconds).unwrap_or(i64::MAX)),
                    ],
                ),
            };
        }
        GuildCommentAnswer::Store {
            guild_id: id,
            pid: speaker,
            content: body.to_vec(),
        }
    }

    /// Commit one judged post under the inserted row (`AddComment`'s tail,
    /// `G/guild.cpp:1066-1071`): the minute sets, and the board — which the descriptor
    /// read after the insert, as `FuncAfterQuery` does — answers for the speaker's own
    /// session. The outboxes stay empty: the COMMENTS record is self.
    pub(super) fn commit_post_comment(
        &mut self,
        commit: &GuildCommentCommit,
    ) -> GuildCommentAnswer {
        let empty = GuildCommentAnswer::Done {
            board: None,
            line: None,
        };
        let GuildCommentCommit::Post {
            speaker: _,
            guild_id,
            board,
        } = commit
        else {
            return empty;
        };
        let Some(guild) = self.guilds.by_id.get_mut(guild_id) else {
            return empty;
        };
        guild.post_comment_due = self.last_pulse.saturating_add(GUILD_COMMENT_WAIT_PULSES);
        GuildCommentAnswer::Done {
            board: Some(board.clone()),
            line: None,
        }
    }

    /// Run one `GUILD_SUBHEADER_CG_REFRESH_COMMENT` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3089-3091`, plus `RefreshComment`, `G/guild.cpp:1092-1095`):
    /// the board — which the descriptor read — answers for the speaker's own session.
    /// The guildless gate chats `[LS;523]`; a non-row closes, as the arm's `m == NULL`
    /// does.
    pub(super) fn answer_refresh_comment(&self, vid: Vid) -> GuildCommentAnswer {
        let close = GuildCommentAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildCommentAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return close;
        };
        if !guild.members.contains_key(&speaker) {
            return close;
        }
        GuildCommentAnswer::Done {
            board: None,
            line: None,
        }
    }

    /// Run one `GUILD_SUBHEADER_CG_DELETE_COMMENT` (`CInputMain::Guild`'s arm,
    /// `G/input_main.cpp:3081-3088`, plus `DeleteComment`, `G/guild.cpp:1075-1090`): a
    /// row found answers the board — which the descriptor read after the delete, as
    /// `RefreshCommentForce` reads it after the query (`:1090`) — and a row missing
    /// chats the missing line (`:1088`). The leader names the id; anyone else names
    /// the id and their own name, which the descriptor's delete holds.
    /// The delete's gates: the speaker's guild, row and leader flag, or `None` when
    /// the arm closes. The dispatch reads this before deleting, because the delete
    /// names the row's guild and the leader flag (`G/guild.cpp:1082-1086`).
    pub(super) fn delete_comment_gates(&self, vid: Vid) -> Option<(u32, u32, bool)> {
        let speaker = self.characters.find_by_vid(vid).ok()?.player_id();
        let id = self.guild_of(speaker)?;
        let guild = self.guilds.by_id.get(&id)?;
        let row = guild.members.get(&speaker)?;
        Some((id, speaker, row.grade == GUILD_LEADER_GRADE))
    }

    /// Answer one `GUILD_SUBHEADER_CG_DELETE_COMMENT`'s judge (`CInputMain::Guild`'s
    /// arm plus the delete's gates): the judged guild, poster and leader flag — the
    /// flag rides `content`'s first byte — for the dispatcher's delete, the
    /// guildless line, or a close. Split out so `apply_command` stays small.
    pub(super) fn answer_delete_comment(&self, vid: Vid) -> GuildCommentAnswer {
        if let Some((guild_id, pid, leader)) = self.delete_comment_gates(vid) {
            return GuildCommentAnswer::Store {
                guild_id,
                pid,
                content: if leader { vec![1] } else { vec![0] },
            };
        }
        let speaker = self
            .characters
            .find_by_vid(vid)
            .ok()
            .map(world::character::Character::player_id);
        // A guildless speaker hears `[LS;523]`; a non-row or a missing guild
        // closes, as the arm's `m == NULL` does.
        if speaker.is_some() && speaker.and_then(|speaker| self.guild_of(speaker)).is_none() {
            return GuildCommentAnswer::Refused {
                line: self.guild_line_of(vid),
            };
        }
        GuildCommentAnswer::Refused { line: None }
    }

    /// Report one judged delete's store half (`DeleteComment`'s tail,
    /// `G/guild.cpp:1088-1090`): a row found answers the board — which the descriptor
    /// read after the delete, as `RefreshCommentForce` reads it after the query
    /// (`:1090`) — and a row missing chats the missing line (`:1088`).
    pub(super) fn commit_delete_comment(&self, commit: &GuildCommentCommit) -> GuildCommentAnswer {
        let empty = GuildCommentAnswer::Done {
            board: None,
            line: None,
        };
        let GuildCommentCommit::Read {
            speaker,
            board,
            deleted,
            ..
        } = commit
        else {
            return empty;
        };
        if *deleted {
            GuildCommentAnswer::Done {
                board: board.clone(),
                line: None,
            }
        } else {
            // `DeleteComment`'s missing-row line (`:1088`) is EUC-KR in the source;
            // the port relays the exact bytes, as free text relays the exact bytes.
            GuildCommentAnswer::Done {
                board: None,
                line: self.guild_line(
                    *speaker,
                    b"<\xb1\xe6\xb5\xe5> \xbb\xe8\xc1\xa6\xc7\xd2 \xbc\xf6 \xbe\xf8\xb4\xc2 \xb1\xdb\xc0\xd4\xb4\xcf\xb4\xd9.",
                    &[],
                ),
            }
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL`
    /// (`CInputMain::Guild`'s arm, `G/input_main.cpp:3095-3116`, plus
    /// `ChangeMemberGeneral`, `G/guild.cpp:1144-1189`): the guildless gate, the
    /// leader gate, the general-count gate, and the judged flag. The count gate
    /// runs at the judge, because the count lives in memory; the `is_general`
    /// byte is judged to 0/1 (`is_general?1:0`, `:1153`). A victim with no row
    /// stays quiet (`ChangeMemberGeneral`'s missing-member arm, `:1148-1151`).
    pub(super) fn answer_member_general(
        &self,
        vid: Vid,
        pid: u32,
        is_general: u8,
    ) -> GuildGradeAnswer {
        let close = GuildGradeAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return close;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let Some(speaker_grade) = self.guild_speaker_grade(id, speaker) else {
            return close;
        };
        if speaker_grade != GUILD_LEADER_GRADE {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"<Guild> You do not have the right to appoint a general.",
                    &[],
                ),
            };
        }
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return close;
        };
        if !guild.members.contains_key(&pid) {
            return close;
        }
        let flag = u8::from(is_general != 0);
        if flag != 0 && self.guild_general_count(id) >= MAX_GENERAL_COUNT {
            return GuildGradeAnswer::Refused {
                line: self.guild_line(vid, b"<Guild> You can no longer specify a longevity.", &[]),
            };
        }
        GuildGradeAnswer::Store {
            guild_id: id,
            set: GuildGradeSet::MemberGeneral {
                pid,
                is_general: flag,
            },
        }
    }

    /// `GetGeneralCount` (`G/guild.h:234`): the members of `id` flagged general.
    fn guild_general_count(&self, id: u32) -> usize {
        self.guilds
            .by_id
            .get(&id)
            .map(|guild| {
                guild
                    .members
                    .values()
                    .filter(|member| member.is_general != 0)
                    .count()
            })
            .unwrap_or_default()
    }

    /// Commit one judged grade-arm step under the stored row: the value lands, the
    /// fan goes through the outboxes to the still-online members, and the answer
    /// carries the line legacy chats *after* the round trip, if any, for the
    /// descriptor to send on its own session. The general commit re-checks the
    /// count, because a rival general may have landed between the judge and the
    /// commit; a full house keeps the stored flag and chats the line, as legacy
    /// chats it when `ChangeMemberGeneral` answers false
    /// (`G/input_main.cpp:3110-3113`). The grade-1 write and the missing member
    /// stay quiet, as `ChangeMemberGrade`'s and `ChangeMemberGeneral`'s early
    /// returns do (`G/guild.cpp:1191-1198`, `:1144-1151`).
    pub(super) fn commit_guild_grade(&mut self, commit: &GuildGradeCommit) -> GuildGradeAnswer {
        let empty = GuildGradeAnswer::Committed { post: None };
        let GuildGradeCommit {
            speaker,
            guild_id,
            set,
        } = commit;
        let Some(guild) = self.guilds.by_id.get(guild_id) else {
            return empty;
        };
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        let fan = match set {
            GuildGradeSet::GradeName { grade, name } => {
                let index = usize::from(grade.saturating_sub(1));
                let Some(grades) = self
                    .guilds
                    .by_id
                    .get_mut(guild_id)
                    .map(|guild| &mut guild.grades)
                else {
                    return empty;
                };
                let Some(slot) = grades.get_mut(index) else {
                    return empty;
                };
                slot.0 = name.as_bytes().to_vec();
                guild_grade_name_record(*grade, name.as_bytes())
            }
            GuildGradeSet::GradeAuth { grade, auth } => {
                let index = usize::from(grade.saturating_sub(1));
                let Some(grades) = self
                    .guilds
                    .by_id
                    .get_mut(guild_id)
                    .map(|guild| &mut guild.grades)
                else {
                    return empty;
                };
                let Some(slot) = grades.get_mut(index) else {
                    return empty;
                };
                slot.1 = *auth;
                guild_grade_auth_record(*grade, *auth)
            }
            GuildGradeSet::MemberGrade { pid, grade } => {
                if *grade == GUILD_LEADER_GRADE {
                    return empty;
                }
                let Some(row) = self
                    .guilds
                    .by_id
                    .get_mut(guild_id)
                    .and_then(|guild| guild.members.get_mut(pid))
                else {
                    return empty;
                };
                row.grade = *grade;
                guild_change_member_grade_record(*pid, *grade)
            }
            GuildGradeSet::MemberGeneral { pid, is_general } => {
                let Some(fan) = self.commit_member_general(*guild_id, *pid, *is_general) else {
                    return empty;
                };
                // A full house answers the longevity line instead of a fan.
                if fan.is_empty() {
                    return GuildGradeAnswer::Committed {
                        post: self.guild_line(
                            *speaker,
                            b"<Guild> You can no longer specify a longevity.",
                            &[],
                        ),
                    };
                }
                fan
            }
            GuildGradeSet::Offer {
                pid,
                floored: _,
                offer,
                exp,
                level,
                ladder,
            } => return self.commit_guild_offer(*guild_id, *pid, *offer, *exp, *level, *ladder),
            GuildGradeSet::Charge {
                pid: _,
                debit: _,
                power,
                gained,
            } => {
                return self.commit_guild_charge(*guild_id, *speaker, *power, *gained);
            }
        };
        for member in members {
            let _sent = self.write_to_client(member, fan.clone());
        }
        empty
    }

    /// Land one judged general flag (`ChangeMemberGeneral`, `G/guild.cpp:1144-1151`):
    /// the stored flag or the full-house quiet — an empty fan, which the caller answers
    /// with the longevity line — or `None` for a missing row. Split out so the grade
    /// commit stays under the line limit.
    fn commit_member_general(
        &mut self,
        guild_id: u32,
        pid: u32,
        is_general: u8,
    ) -> Option<Vec<u8>> {
        if is_general != 0 && self.guild_general_count(guild_id) >= MAX_GENERAL_COUNT {
            return Some(Vec::new());
        }
        let row = self
            .guilds
            .by_id
            .get_mut(&guild_id)
            .and_then(|guild| guild.members.get_mut(&pid))?;
        row.is_general = is_general;
        Some(guild_change_member_general_record(pid, is_general))
    }

    /// Commit one judged offer under the stored rows (`OfferExp`, `G/guild.cpp:942-982`,
    /// plus `GuildPointChange`'s exp arm, `:1582-1667`): the member's `offer_exp` grows,
    /// the guild's exp, level and ladder land, and the LIST-one, the `CHANGE_EXP` and one
    /// INFO per level step fan through the outboxes. The LIST-one fans even when the
    /// speaker is offline (the fan does not read the speaker); the LIST-one carries the
    /// member's stored name, as the online container's `_dummy = 1` row does
    /// (`:985-995`). The blind row below a missing guild is the missing-row commit's
    /// quiet.
    fn commit_guild_offer(
        &mut self,
        id: u32,
        pid: u32,
        offer: u32,
        exp: u32,
        level: u8,
        ladder: i32,
    ) -> GuildGradeAnswer {
        let empty = GuildGradeAnswer::Committed { post: None };
        let old_level = self.guilds.by_id.get(&id).map_or(0, |guild| guild.level);
        let Some((one, info, members, levels)) = self.guilds.by_id.get_mut(&id).and_then(|guild| {
            let row = guild.members.get_mut(&pid)?;
            row.offer = row.offer.saturating_add(offer);
            let one = GcGuildMember {
                pid,
                grade: row.grade,
                is_general: row.is_general,
                job: row.job,
                level: row.level,
                offer: row.offer,
                name_flag: 1,
                name: pad_member_name(&row.name),
            };
            guild.exp = exp;
            guild.level = level;
            guild.ladder_point = ladder;
            let info = guild.info();
            let members: Vec<Vid> = guild
                .members
                .values()
                .filter_map(|member| member.vid)
                .collect();
            Some((
                one,
                info,
                members,
                usize::from(level.saturating_sub(old_level)),
            ))
        }) else {
            return empty;
        };
        for member in &members {
            let _sent = self.write_to_client(*member, guild_list_one_record(&one));
            let _sent = self.write_to_client(*member, guild_change_exp_record(level, exp));
            for _ in 0..levels {
                let _sent = self.write_to_client(*member, guild_info_record(&info));
            }
        }
        empty
    }

    /// Run one `/gskillup` line (`do_guildskillup`, `G/cmd_general.cpp:878-904`, plus
    /// `CGuild::SkillLevelUp`, `G/guild.cpp:1234-1264`): the empty-arg quiet, the
    /// guildless `[LS;523]`, the non-leader `[LS;524]`, then the up — out-of-range
    /// quiet, missing-row quiet (the `sys_err` has no sink), at-max quiet, no-point
    /// quiet, else the point is spent, the byte climbs, and the `SKILL_INFO` fans to
    /// the online members. Answers the climbed skill for the store half (`SaveSkill`:
    /// power, point and blob).
    ///
    /// The leader gate reads the row's grade, not auth bits. A member row that `of`
    /// names but `members` lacks is quiet: legacy dereferences the row unchecked (a
    /// Defect), and the two maps move together, so the port holds the invariant and
    /// documents it.
    pub(super) fn guild_skill_up(&mut self, vid: Vid, rest: &[u8]) -> GuildSkillUpAnswer {
        let quiet = GuildSkillUpAnswer { climbed: None };
        let Some(vnum) = super::pc_skill::skill_up_vnum(rest) else {
            return quiet;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return quiet;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            self.tell_guild(vid, b"[LS;523]", &[]);
            return quiet;
        };
        let leader = self
            .guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.members.get(&speaker))
            .map(|row| row.grade == GUILD_LEADER_GRADE);
        match leader {
            None => return quiet,
            Some(false) => {
                self.tell_guild(vid, b"[LS;524]", &[]);
                return quiet;
            }
            Some(true) => {}
        }
        let real = vnum.saturating_sub(GUILD_SKILL_START);
        let index = usize::try_from(real).unwrap_or(GUILD_SKILL_COUNT);
        if index >= GUILD_SKILL_COUNT {
            return quiet;
        }
        let Some(row) = self.guild_skill_tables.get(vnum) else {
            return quiet;
        };
        let max = row.record.max_level;
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return quiet;
        };
        if guild.skills[index] >= max || guild.skill_point == 0 {
            return quiet;
        }
        guild.skill_point -= 1;
        guild.skills[index] += 1;
        // `ComputeGuildPoints` (`G/guild.cpp:1439-1444`): the max recomputes from the
        // level and the power clamps into it.
        guild.max_power =
            GUILD_BASE_POWER + u32::from(guild.level.saturating_sub(1)) * GUILD_POWER_PER_LEVEL;
        guild.power = guild.power.min(guild.max_power);
        let level = guild.skills[index];
        let point = guild.skill_point;
        let skills = guild.skills;
        let info = guild.skill_info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_skill_info_record(&info));
        }
        GuildSkillUpAnswer {
            climbed: Some(GuildSkillUp {
                guild_id: id,
                vnum,
                level,
                point,
                skills,
            }),
        }
    }

    /// Run one `/war` line (`do_war`, `G/cmd_general.cpp:2028-2210`): the guildless
    /// quiet, the own-war `[LS;533]`, the two arguments (an empty name is quiet; the
    /// type defaults to field, folds past the max to field, and refuses a negative
    /// quiet under `__FIX_COMANDA_RAZBOI__`), the master-only ERR 1116, the unknown-name
    /// ERR 1117, the state switch (none: the opp-war ERR 1118 and the two dead money
    /// arms, ERR 1119/1120; sent: ERR 1121; received: the opp-war ERR 1122 plus a
    /// refuse; reserved: ERR 1123; ended: quiet; anything else: ERR 1124 plus a
    /// refuse), the own `CanStartWar` (`[LS;1308]` ladderless, `[LS;543]` short), the
    /// opp's (`[LS;544]` ladderless, `[LS;545]` short), the master-online arms
    /// (`[LS;1048]` plus a refuse — the P2P lookup is a row-`vid` check in one
    /// process), and the declare.
    ///
    /// The declare collapses the game→DB→game round trip (`AddDeclare`'s dedup plus the
    /// forward, `db/ClientManagerGuild.cpp:145-148`, `G/input_db.cpp:1336-1340`): the
    /// entry existing is the same guard, and the manager's declare runs inline — both
    /// `DeclareWar` fans, then the `[LS;475]` notice the descriptor fans. An accept on a
    /// received field declare starts the war inline the same way, stamping `now`. A type past
    /// field answers `[LS;669]` to the master (`GuildWar_IsWarMap` finds no war map on
    /// this build). Nothing is stored: declares are memory-only both sides.
    pub(super) fn guild_war(&mut self, vid: Vid, rest: &[u8], now: u32) -> GuildWarAnswer {
        let quiet = GuildWarAnswer { notice: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return quiet;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return quiet;
        };
        if self.guild_under_any_war(id) {
            self.tell_guild(vid, b"[LS;533]", &[]);
            return quiet;
        }
        let (arg1, after) = crate::command::one_argument(rest);
        if arg1.is_empty() {
            return quiet;
        }
        let (arg2, _) = crate::command::one_argument(after);
        let war_type = parse_war_type_or_field(&arg2);
        // `__FIX_COMANDA_RAZBOI__` (`G/cmd_general.cpp:2057-2062`): a leading `-` or
        // a negative `strtol` refuses quiet, after the fold.
        if !arg2.is_empty() && (arg2.first() == Some(&b'-') || parse_war_long(&arg2) < 0) {
            return quiet;
        }
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return quiet;
        };
        if guild.master_pid != speaker {
            self.tell_guild(vid, b"#ERR_CODE: 1116030623 Send this to a GM.", &[]);
            return quiet;
        }
        let Some(opp) = self.guild_named_id(&arg1) else {
            self.tell_guild(vid, b"#ERR_CODE: 1117030623 Send this to a GM.", &[]);
            return quiet;
        };
        if opp == id {
            // `RequestDeclareWar`'s self arm (`G/guild_war.cpp:281-286`): logged, quiet.
            // The state switch below would read the guild's entry against itself, which
            // no declare ever inserts, so `NONE` with `UnderAnyWar` on the other side —
            // legacy never names itself either way.
            return quiet;
        }
        let state = self.guild_war_state(id, opp);
        match state {
            GUILD_WAR_NONE => {
                if !self.guild_war_price_passes(vid, id, opp, war_type) {
                    return quiet;
                }
            }
            GUILD_WAR_SEND_DECLARE => {
                // The source's EUC-KR head did not survive (`cmd_general.cpp:2114`):
                // U+FFFD U+0329 U+FFFD, ported byte-exact.
                self.tell_guild(
                    vid,
                    b"\xEF\xBF\xBD\xCC\xA9\xEF\xBF\xBD#ERR_CODE: 1121030623 Send this to a GM.",
                    &[],
                );
                return quiet;
            }
            GUILD_WAR_RECV_DECLARE => {
                if self.guild_under_any_war(opp) {
                    self.tell_guild(vid, b"#ERR_CODE: 1122030623 Send this to a GM.", &[]);
                    self.request_refuse_war(id, opp);
                    return quiet;
                }
            }
            GUILD_WAR_RESERVE => {
                self.tell_guild(vid, b"#ERR_CODE: 1123030623 Send this to a GM.", &[]);
                return quiet;
            }
            GUILD_WAR_END => return quiet,
            _ => {
                self.tell_guild(vid, b"#ERR_CODE: 1124030623 Send this to a GM.", &[]);
                self.request_refuse_war(id, opp);
                return quiet;
            }
        }
        if !self.guild_war_starts_pass(vid, id, opp, war_type) {
            return quiet;
        }
        if !self.guild_master_online(id) {
            self.tell_guild(vid, b"[LS;1048]", &[]);
            self.request_refuse_war(id, opp);
            return quiet;
        }
        if !self.guild_master_online(opp) {
            self.tell_guild(vid, b"[LS;1048]", &[]);
            self.request_refuse_war(id, opp);
            return quiet;
        }
        self.request_declare_war(id, opp, war_type, now)
    }

    /// `/war`'s none-state gate (`G/cmd_general.cpp:2101-2112`): the opp must hold no
    /// begun war, and both sides must hold the type's price. True to continue.
    fn guild_war_price_passes(&mut self, vid: Vid, id: u32, opp: u32, war_type: u8) -> bool {
        if self.guild_under_any_war(opp) {
            self.tell_guild(vid, b"#ERR_CODE: 1118030623 Send this to a GM.", &[]);
            return false;
        }
        let price = war_price(war_type);
        let own_gold = self.guilds.by_id.get(&id).map_or(0, |guild| guild.gold);
        // `GetGuildMoney` is a signed `int` (`G/guild.h:283`); the table holds 0 for
        // every type, so neither arm fires — ported as the reads they are.
        if i64::from(own_gold) < i64::from(price) {
            self.tell_guild(vid, b"#ERR_CODE: 1119030623 Send this to a GM.", &[]);
            return false;
        }
        let opp_gold = self.guilds.by_id.get(&opp).map_or(0, |guild| guild.gold);
        if i64::from(opp_gold) < i64::from(price) {
            self.tell_guild(vid, b"#ERR_CODE: 1120030623 Send this to a GM.", &[]);
            return false;
        }
        true
    }

    /// `/war`'s start gates (`G/cmd_general.cpp:2126-2162`): both sides must pass
    /// `CanStartWar` — the own side at the asked type, the opp always at field
    /// (`:2156`) — each with its own chat. True to continue.
    fn guild_war_starts_pass(&mut self, vid: Vid, id: u32, opp: u32, war_type: u8) -> bool {
        if !self.guild_can_start_war(id, war_type) {
            let guild = self.guilds.by_id.get(&id);
            if guild.is_some_and(|guild| guild.ladder_point == 0) {
                self.tell_guild(vid, b"[LS;1308]", &[]);
            } else if guild.is_some_and(|guild| guild.members.len() < GUILD_WAR_MIN_MEMBER_COUNT) {
                self.tell_guild(
                    vid,
                    b"[LS;543;%d]",
                    &[Arg::Int(
                        i64::try_from(GUILD_WAR_MIN_MEMBER_COUNT).unwrap_or(i64::MAX),
                    )],
                );
            }
            return false;
        }
        if !self.guild_can_start_war(opp, GUILD_WAR_TYPE_FIELD) {
            let guild = self.guilds.by_id.get(&opp);
            if guild.is_some_and(|guild| guild.ladder_point == 0) {
                self.tell_guild(vid, b"[LS;544]", &[]);
            } else if guild.is_some_and(|guild| guild.members.len() < GUILD_WAR_MIN_MEMBER_COUNT) {
                self.tell_guild(vid, b"[LS;545]", &[]);
            }
            return false;
        }
        true
    }

    /// Run one `/nowar` line (`do_nowar`, `G/cmd_general.cpp:2211-2243`): the guildless
    /// quiet, the empty-name quiet, the non-master `[LS;534]`, the unknown-name `[LS;535]`,
    /// and the refuse — which only lands when this guild holds a received declare
    /// (`CGuild::RequestRefuseWar`, `G/guild_war.cpp:540-556`); a sent declare stands.
    pub(super) fn guild_nowar(&mut self, vid: Vid, rest: &[u8]) {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return;
        };
        let (arg1, _) = crate::command::one_argument(rest);
        if arg1.is_empty() {
            return;
        }
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return;
        };
        if guild.master_pid != speaker {
            self.tell_guild(vid, b"[LS;534]", &[]);
            return;
        }
        let Some(opp) = self.guild_named_id(&arg1) else {
            self.tell_guild(vid, b"[LS;535]", &[]);
            return;
        };
        self.request_refuse_war(id, opp);
    }
    /// One `gwstop` line (`do_stop_guild_war`, `G/cmd_gm.cpp:2766-2790`): two `int` words
    /// (`two_arguments` + `str_to_number`, where an empty word stays 0 — hence the two
    /// guards), either 0 → quiet, sorted, the speaker echoes `TALKING "%d %d"`, and the
    /// pair ends in a forced draw with the full ladder math (`RequestEndWar` → DB
    /// `WarEnd(force=true)`). Answers the ended rows for the descriptor's store write,
    /// or `None` when the pair was no live war (the echo already left: it goes before
    /// the end is attempted).
    pub(super) fn guild_war_stop(
        &mut self,
        vid: Vid,
        rest: &[u8],
        now: u32,
    ) -> Option<WarEndOutcome> {
        let (arg1, after) = crate::command::one_argument(rest);
        let (arg2, _) = crate::command::one_argument(after);
        if arg1.is_empty() || arg2.is_empty() {
            return None;
        }
        let (mut id1, mut id2) = (parse_gm_guild_id(&arg1), parse_gm_guild_id(&arg2));
        if id1 == 0 || id2 == 0 {
            return None;
        }
        if id1 > id2 {
            std::mem::swap(&mut id1, &mut id2);
        }
        if let Some(mover) = self.own_mover(vid) {
            self.tell(
                vid,
                mover,
                protocol::gc_chat::CHAT_TYPE_TALKING,
                b"%d %d",
                &[Arg::Int(i64::from(id1)), Arg::Int(i64::from(id2))],
            );
        }
        let (lo, hi) = (
            u32::try_from(id1).unwrap_or(u32::MAX),
            u32::try_from(id2).unwrap_or(u32::MAX),
        );
        self.manager_end_war(lo, hi, true, true, now)
    }

    /// One `gwcancel` line (`do_cancel_guild_war`, `G/cmd_gm.cpp:2791-2805`): the same
    /// parse with NO empty-word and NO zero guards (a 0 runs, and misses the war set),
    /// no echo, sorted, and the pair cancels: the over notice, the END fans, the
    /// `WAR_END_LIST`, both masters' `[LS;482]` and every PC's `[LS;483]` — but no
    /// ladder math and no tally writes (`CancelWar` → `RemoveWar` + game `EndWar`).
    /// Answers whether the pair was a live war.
    pub(super) fn guild_war_cancel(&mut self, rest: &[u8], now: u32) -> bool {
        let (arg1, after) = crate::command::one_argument(rest);
        let (arg2, _) = crate::command::one_argument(after);
        let (mut id1, mut id2) = (parse_gm_guild_id(&arg1), parse_gm_guild_id(&arg2));
        if id1 > id2 {
            std::mem::swap(&mut id1, &mut id2);
        }
        let (lo, hi) = (
            u32::try_from(id1).unwrap_or(u32::MAX),
            u32::try_from(id2).unwrap_or(u32::MAX),
        );
        let Some(_) = self.manager_end_war(lo, hi, false, false, now) else {
            return false;
        };
        self.notify_guild_master(lo, b"[LS;482]", &[]);
        self.notify_guild_master(hi, b"[LS;482]", &[]);
        // `CancelWar`'s last act (`G/guild_manager.cpp:763-768`): both guilds' names
        // to every PC, as a notice.
        let lo_name = self.guild_name_of(lo);
        let hi_name = self.guild_name_of(hi);
        self.notice_every_pc(
            b"[LS;483;%s;%s]",
            &[Arg::Text(&lo_name), Arg::Text(&hi_name)],
        );
        true
    }

    /// The id of the guild named `name` (`CGuildManager::FindGuildByName`,
    /// `G/guild_manager.cpp:198-207`): the first in id order whose name compares equal,
    /// byte for byte — `std::string ==` folds no case, so neither does this.
    fn guild_named_id(&self, name: &[u8]) -> Option<u32> {
        self.guilds
            .by_id
            .iter()
            .find(|(_, guild)| guild.name == name)
            .map(|(id, _)| *id)
    }

    /// `id`'s war state against `opp` (`CGuild::GetGuildWarState`,
    /// `G/guild_war.cpp:112-121`): none against itself or an unknown guild.
    fn guild_war_state(&self, id: u32, opp: u32) -> u8 {
        if opp == id {
            return GUILD_WAR_NONE;
        }
        self.guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.enemies.get(&opp))
            .map_or(GUILD_WAR_NONE, |enemy| enemy.state)
    }

    /// Whether `id` holds any begun war (`CGuild::UnderAnyWar` with no type,
    /// `G/guild_war.cpp:163-179`): an enemy in on-war. Declares do not count.
    pub(super) fn guild_under_any_war(&self, id: u32) -> bool {
        self.guilds.by_id.get(&id).is_some_and(|guild| {
            guild
                .enemies
                .values()
                .any(|enemy| enemy.state == GUILD_WAR_ON_WAR)
        })
    }

    /// Whether `id` may start a war of `war_type` (`CGuild::CanStartWar`,
    /// `G/guild_war.cpp:143-152`): the type below the max, a positive ladder, and 8
    /// member rows. The test-server/event-flag arm is gone: the Rewrite has no test
    /// server, and `guild_war_test` is a test flag.
    fn guild_can_start_war(&self, id: u32, war_type: u8) -> bool {
        if war_type >= GUILD_WAR_TYPE_MAX_NUM {
            return false;
        }
        self.guilds.by_id.get(&id).is_some_and(|guild| {
            guild.ladder_point > 0 && guild.members.len() >= GUILD_WAR_MIN_MEMBER_COUNT
        })
    }

    /// Whether `id`'s master is online: its member row holds a VID. Legacy asks the
    /// local character plus the P2P map (`G/cmd_general.cpp:2164-2207`); one process
    /// holds every client, so the row answers both.
    fn guild_master_online(&self, id: u32) -> bool {
        self.guilds.by_id.get(&id).is_some_and(|guild| {
            guild
                .members
                .get(&guild.master_pid)
                .is_some_and(|master| master.vid.is_some())
        })
    }

    /// `id`'s declare against `opp` (`CGuild::RequestDeclareWar`,
    /// `G/guild_war.cpp:279-370`, with the DB forward collapsed): the self and
    /// unknown-type quiets, then the entry's arm. No entry: a field declare lands
    /// through the manager; a type whose war map the map index does not hold answers
    /// `[LS;669]` to the master (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`). A
    /// received entry would accept: field accepts are the on-war slice
    /// (`sys.guild.war`), and a type whose war map the map index does not hold answers
    /// `[LS;669]` the same way. A sent entry chats `[LS;780]` to the master; anything
    /// else logs, quiet.
    fn request_declare_war(&mut self, id: u32, opp: u32, war_type: u8, now: u32) -> GuildWarAnswer {
        let quiet = GuildWarAnswer { notice: None };
        if opp == id || war_type >= GUILD_WAR_TYPE_MAX_NUM {
            return quiet;
        }
        let state = self.guild_war_state(id, opp);
        let fresh = self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| !guild.enemies.contains_key(&opp));
        if fresh {
            if war_type != GUILD_WAR_TYPE_FIELD && !self.guild_war_is_war_map(war_type) {
                self.notify_guild_master(id, b"[LS;669]", &[]);
                return quiet;
            }
            return self.manager_declare_war(id, opp, war_type);
        }
        match state {
            GUILD_WAR_RECV_DECLARE => {
                let saved = self
                    .guilds
                    .by_id
                    .get(&id)
                    .and_then(|guild| guild.enemies.get(&opp))
                    .map_or(GUILD_WAR_TYPE_FIELD, |enemy| enemy.war_type);
                if saved != GUILD_WAR_TYPE_FIELD && !self.guild_war_is_war_map(saved) {
                    self.notify_guild_master(id, b"[LS;669]", &[]);
                    return quiet;
                }
                // A field accept starts the war (`G/guild_war.cpp:332-343`, with the DB
                // round trip collapsed): the manager's start runs inline.
                self.manager_start_war(id, opp, now)
            }
            GUILD_WAR_SEND_DECLARE => {
                self.notify_guild_master(id, b"[LS;780]", &[]);
                quiet
            }
            _ => {
                warn!(id, opp, state, "a declare met an enemy in an unknown state");
                quiet
            }
        }
    }

    /// `GuildWar_IsWarMap` (`G/guild_war.cpp:260-271`): the field war needs no map, and
    /// every other type needs its map in the map index — the map's own files are there.
    fn guild_war_is_war_map(&self, war_type: u8) -> bool {
        match war_type {
            GUILD_WAR_TYPE_FIELD => true,
            GUILD_WAR_TYPE_BATTLE | GUILD_WAR_TYPE_FLAG => {
                let map = super::war_map::WAR_TYPES
                    .get(usize::from(war_type))
                    .map_or(0, |info| info.map_index);
                self.recall.in_atlas(map)
            }
            _ => false,
        }
    }

    /// The war's map (`CGuild::WaitStartWar`, `G/guild_war.cpp:440-530`): a war of a map
    /// type takes a private copy of the map it is fought on, hosted on the Channel that
    /// runs the war. The field war needs no map, and a map the Channel does not host
    /// (`map_allow_find`, `G/config.cpp:174-182`) leaves the war without one.
    ///
    /// The private map is a copy of the base map's own sectrees, so it stands on the base
    /// map's grid (`SECTREE_MANAGER::CreatePrivateMap`,
    /// `G/sectree_manager.cpp:943-1026`).
    fn guild_war_map(&mut self, id: u32, war_type: u8) -> i32 {
        if war_type == GUILD_WAR_TYPE_FIELD {
            return 0;
        }
        let Some(info) = super::war_map::WAR_TYPES.get(usize::from(war_type)) else {
            return 0;
        };
        let base = info.map_index;
        // `CreateWarMap` (`G/war_map.cpp:1042-1049`): the type's map must be one of the
        // war maps, or there is no map to copy.
        if super::war_map::war_map_of(base).is_none() {
            return 0;
        }
        // The Channel that runs the war is the one the accepting master stands on.
        let Some(channel) = self.guild_master_channel(id) else {
            return 0;
        };
        if !self.recall.hosts(channel, base) {
            return 0;
        }
        let taken: Vec<i32> = self.maps.keys().map(|&(_, map)| map).collect();
        let Some(private) = super::war_map::private_index(base, &taken) else {
            return 0;
        };
        let Some(grid) = self
            .recall
            .atlas()
            .and_then(|atlas| atlas.region(base))
            .and_then(SectreeGrid::of)
        else {
            return 0;
        };
        self.host_map(channel, private, grid);
        private
    }

    /// The Channel the master of the guild `id` stands on: the place of its member row's
    /// character. A guild whose master is not in the world has no Channel.
    fn guild_master_channel(&self, id: u32) -> Option<u8> {
        let guild = self.guilds.by_id.get(&id)?;
        let master = guild.members.get(&guild.master_pid)?;
        let vid = master.vid?;
        self.place_of(vid.into()).map(|(channel, _)| channel)
    }

    /// `id`'s refuse against `opp` (`CGuild::RequestRefuseWar`,
    /// `G/guild_war.cpp:540-556`, with the DB forward collapsed): only a received
    /// declare refuses; every other state — including a sent one — stands, quiet.
    fn request_refuse_war(&mut self, id: u32, opp: u32) {
        if opp == id {
            return;
        }
        if self.guild_war_state(id, opp) != GUILD_WAR_RECV_DECLARE {
            return;
        }
        self.manager_refuse_war(id, opp);
    }

    /// The forwarded accept (`CGuildManager::StartWar`, `G/guild_manager.cpp:629-651`, with
    /// the DB round trip collapsed): both enemies move to `ON_WAR` with `now` as their start
    /// time, both sides hear their on-war fan (each chats `[LS;558]` first), the sorted pair
    /// joins the war set with its every-PC `WAR_LIST` fan, and the `[LS;477]` notice names
    /// the acceptor first. `now` is `get_global_time()` in seconds, which the descriptor
    /// reads. Either check failing leaves the declares standing, quiet — legacy's
    /// `CheckStartWar` failures return the same way.
    /// `CWarMapManager::CreateWarMap` (`G/war_map.cpp:1042-1059`): the war's private
    /// map becomes a war map of the type the map was copied from, and the two guilds
    /// stand on it, the lower id first.
    fn create_war_map(&mut self, map_index: i32, from: u32, to: u32) {
        let Some(info) = super::war_map::war_map_of(map_index) else {
            return;
        };
        let guilds = if from < to { [from, to] } else { [to, from] };
        let map = super::war_map::WarMap::new(map_index, info.map_index, info.map_type, guilds, 0);
        self.war_maps.insert(map_index, map);
    }

    /// `CWarMap::IncMember` (`G/war_map.cpp:342-398`): a character that enters a war map
    /// joins the team of its guild when it carries the war member's flag, and watches
    /// the war otherwise.
    pub(super) fn join_war_map(&mut self, vid: Vid, map: i32) {
        if !self.war_maps.contains_key(&map) {
            return;
        }
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let guild = self.guild_of(pid).unwrap_or(0);
        // `ch->GetQuestFlag("war.is_war_member") > 0` (`G/war_map.cpp:353`): the flag the
        // war entry set (`G/guild_war.cpp:674`).
        let flag = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, b"war.is_war_member"))
            .unwrap_or(0);
        // `IncMember` clears the flag it read (`G/war_map.cpp:357`): the entry's flag
        // is consumed by the join.
        if flag > 0 {
            if let Some(quests) = self.quests.as_mut() {
                quests
                    .manager_mut()
                    .set_quest_flag(pid, b"war.is_war_member", 0);
            }
        }
        if let Some(war) = self.war_maps.get_mut(&map) {
            war.inc_member(vid, pid, guild, flag > 0);
        }
        self.war_user_count(map);
    }

    /// `CWarMap::DecMember` (`G/war_map.cpp:400-451`): a player that leaves the war's map
    /// falls out of the count of its team, or of the watchers'.
    pub(super) fn leave_war_map(&mut self, vid: Vid, map: i32) {
        let Some(war) = self.war_maps.get(&map) else {
            return;
        };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        let pid = character.player_id();
        let guild = self.guild_of(pid).unwrap_or(0);
        let observer = match war.team_of(guild) {
            Some(index) => !war.teams[index].joiners.contains(&pid),
            None => true,
        };
        if let Some(war) = self.war_maps.get_mut(&map) {
            war.dec_member(vid, guild, observer);
        }
        self.war_user_count(map);
    }

    /// `CWarMap::UpdateUserCount` (`G/war_map.cpp:330-340`): every player on the war's
    /// map hears the two command lines the legacy sends it — the watcher count, and the
    /// two guilds with the players that joined them.
    fn war_user_count(&mut self, map: i32) {
        let Some(war) = self.war_maps.get(&map) else {
            return;
        };
        let (guild1, count1, guild2, count2, observers) = war.user_count();
        let mut vids: Vec<Vid> = war.present.iter().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let Some(mover) = self.own_mover(vid) else {
                continue;
            };
            self.tell(
                vid,
                mover,
                CHAT_TYPE_COMMAND,
                b"ObserverCount %d",
                &[Arg::Int(i64::from(observers))],
            );
            self.tell(
                vid,
                mover,
                CHAT_TYPE_COMMAND,
                b"WarUC %d %d %d %d %d",
                &[
                    Arg::Int(i64::from(guild1)),
                    Arg::Int(i64::try_from(count1).unwrap_or(i64::MAX)),
                    Arg::Int(i64::from(guild2)),
                    Arg::Int(i64::try_from(count2).unwrap_or(i64::MAX)),
                    Arg::Int(i64::from(observers)),
                ],
            );
        }
    }

    fn manager_start_war(&mut self, from: u32, to: u32, now: u32) -> GuildWarAnswer {
        let quiet = GuildWarAnswer { notice: None };
        if from == to {
            return quiet;
        }
        if !self.guilds.by_id.contains_key(&from) || !self.guilds.by_id.contains_key(&to) {
            return quiet;
        }
        // `CheckStartWar` both sides (`G/guild_war.cpp:404-416`): the enemy exists and is
        // not already on-war.
        for (id, opp) in [(from, to), (to, from)] {
            let starts = self
                .guilds
                .by_id
                .get(&id)
                .and_then(|guild| guild.enemies.get(&opp))
                .is_some_and(|enemy| enemy.state != GUILD_WAR_ON_WAR);
            if !starts {
                return quiet;
            }
        }
        // The war's map (`CGuild::WaitStartWar`, `G/guild_war.cpp:440-530`): a war of a map
        // type takes a private copy of its map, and both sides learn its index. The field
        // war needs no map, and a map the Channel does not host leaves the war without
        // one.
        let war_type = self
            .guilds
            .by_id
            .get(&from)
            .and_then(|guild| guild.enemies.get(&to))
            .map_or(GUILD_WAR_TYPE_FIELD, |enemy| enemy.war_type);
        let map_index = self.guild_war_map(from, war_type);
        for (id, opp) in [(from, to), (to, from)] {
            if let Some(enemy) = self
                .guilds
                .by_id
                .get_mut(&id)
                .and_then(|guild| guild.enemies.get_mut(&opp))
            {
                enemy.map_index = map_index;
            }
        }
        // `CWarMapManager::CreateWarMap` (`G/war_map.cpp:1042-1059`): the private copy
        // the war's map type names becomes the war's map, and the two guilds stand on
        // it, the lower id first (`GetTeamIndex`, `G/war_map.cpp:199-215`).
        if map_index != 0 {
            self.create_war_map(map_index, from, to);
        }
        // `CGuild::StartWar` both sides (`G/guild_war.cpp:418-437`): the state, the stamp,
        // and the fan. Field wars ask no entry.
        for (id, opp) in [(from, to), (to, from)] {
            if let Some(enemy) = self
                .guilds
                .by_id
                .get_mut(&id)
                .and_then(|guild| guild.enemies.get_mut(&opp))
            {
                enemy.state = GUILD_WAR_ON_WAR;
                enemy.war_start_time = now;
            }
            // `GuildWarPacket(dwOppGID, gw.type, GUILD_WAR_ON_WAR)`
            // (`G/guild_war.cpp:428`): the fan carries the war's own type.
            self.guild_war_fan(id, opp, war_type, GUILD_WAR_ON_WAR);
        }
        let (lo, hi) = if from < to { (from, to) } else { (to, from) };
        self.guilds.wars.insert((lo, hi));
        // `FSendWarList` to every PC (`:649-650`): the world's outbox keys are its only
        // all-PC handle, sorted for determinism.
        let record = guild_war_list_record(&[(lo, hi)]);
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let _sent = self.write_to_client(vid, record.clone());
        }
        let from_name = self
            .guilds
            .by_id
            .get(&from)
            .map_or(Vec::new(), |guild| guild.name.clone());
        let to_name = self
            .guilds
            .by_id
            .get(&to)
            .map_or(Vec::new(), |guild| guild.name.clone());
        GuildWarAnswer {
            notice: Some(GuildWarNotice::Accept { from_name, to_name }),
        }
    }

    /// The forwarded declare (`CGuildManager::DeclareWar`, `G/guild_manager.cpp:541-561`):
    /// both guilds insert — sent here, received there — and the notice names them.
    /// A side that already holds the enemy refuses the pair: with the round trip
    /// collapsed, the caller already checked, so the `false` is unreachable — but the
    /// guard stays, as legacy's does.
    fn manager_declare_war(&mut self, from: u32, to: u32, war_type: u8) -> GuildWarAnswer {
        let quiet = GuildWarAnswer { notice: None };
        if from == to {
            return quiet;
        }
        if !self.guilds.by_id.contains_key(&from) || !self.guilds.by_id.contains_key(&to) {
            return quiet;
        }
        let sent = self.declare_war(from, to, war_type, GUILD_WAR_SEND_DECLARE);
        let received = self.declare_war(to, from, war_type, GUILD_WAR_RECV_DECLARE);
        if !(sent && received) {
            return quiet;
        }
        let from_name = self
            .guilds
            .by_id
            .get(&from)
            .map_or(Vec::new(), |guild| guild.name.clone());
        let to_name = self
            .guilds
            .by_id
            .get(&to)
            .map_or(Vec::new(), |guild| guild.name.clone());
        GuildWarAnswer {
            notice: Some(GuildWarNotice::Declare { from_name, to_name }),
        }
    }

    /// `CGuildManager::Kill`'s field arm (`G/guild_manager.cpp:838-860`, called from
    /// `char_battle.cpp:1308-1311`): the killer PC's guild scores the victim PC's level
    /// against the victim's guild when the killer's enemy vs the victim's guild is a field
    /// war begun (`UnderWar`), and both guilds' online members hear the new total. The
    /// DB server's accumulate-and-fan collapses inline: the stored score IS the running
    /// total (`DWORD iScore`, `db/GuildManager.h:75`). A war-map killer, a guildless side,
    /// or a past-field enemy scores nothing; no war maps exist, so `GetWarMap()` is
    /// always null.
    pub(super) fn guild_war_kill(&mut self, killer: Vid, victim: Vid) {
        let (Ok(killer_character), Ok(victim_character)) = (
            self.characters.find_by_vid(killer),
            self.characters.find_by_vid(victim),
        ) else {
            return;
        };
        let (killer_pid, victim_pid) = (killer_character.player_id(), victim_character.player_id());
        let (Some(attack), Some(defend)) = (self.guild_of(killer_pid), self.guild_of(victim_pid))
        else {
            return;
        };
        let field = self
            .guilds
            .by_id
            .get(&attack)
            .and_then(|guild| guild.enemies.get(&defend))
            .is_some_and(|enemy| enemy.war_type == GUILD_WAR_TYPE_FIELD);
        if !field || !self.guild_under_war(attack, defend) {
            return;
        }
        let level = self
            .pc_points_mut(victim.raw())
            .map_or(0, |points| points.level());
        // `u32`: the DB's `iScore` is a `DWORD`, and the fan writes its 4 bytes.
        let total = self
            .guilds
            .by_id
            .get(&attack)
            .and_then(|guild| guild.enemies.get(&defend))
            .map_or(0, |enemy| enemy.score)
            .wrapping_add(u32::from(level));
        if let Some(enemy) = self
            .guilds
            .by_id
            .get_mut(&attack)
            .and_then(|guild| guild.enemies.get_mut(&defend))
        {
            enemy.score = total;
        }
        // `SetWarScoreAgainstTo`'s field fan (`G/guild_war.cpp:198-217`): the same record
        // to the gainer's online members, then the opp's (`Packet` each, `:217-218`).
        let record = guild_war_score_record(attack, defend, total);
        for id in [attack, defend] {
            let members: Vec<Vid> = self
                .guilds
                .by_id
                .get(&id)
                .map(|guild| {
                    guild
                        .members
                        .values()
                        .filter_map(|member| member.vid)
                        .collect()
                })
                .unwrap_or_default();
            for member in members {
                let _sent = self.write_to_client(member, record.clone());
            }
        }
    }

    /// Whether a PC killer under `killer` loses its monster reward to a field war
    /// (`char_battle.cpp:1477`): it is a player, its guild holds a begun FIELD enemy, and
    /// the victim is not a player. The caller passes the victim kind it already knows.
    pub(super) fn guild_blocks_monster_reward(&self, killer: Vid) -> bool {
        let Ok(character) = self.characters.find_by_vid(killer) else {
            return false;
        };
        let Some(id) = self.guild_of(character.player_id()) else {
            return false;
        };
        self.guilds.by_id.get(&id).is_some_and(|guild| {
            guild.enemies.values().any(|enemy| {
                enemy.war_type == GUILD_WAR_TYPE_FIELD && enemy.state == GUILD_WAR_ON_WAR
            })
        })
    }

    /// The war-reward line (`char_battle.cpp:1501-1507`): `[LS;558]` to the PC killer
    /// whose guild's field war ate its monster reward, at most once a minute. The stamp
    /// is boot milliseconds (`get_dword_time()`), which the world's clock keeps.
    pub(super) fn tell_war_reward_line(&mut self, killer: Vid) {
        let now = self.clock.now();
        let muted = self
            .pc_combat
            .get(&killer.raw())
            .and_then(|combat| combat.war_reward_mute_until_ms)
            .is_some_and(|until| now < until);
        if muted {
            return;
        }
        self.pc_combat
            .entry(killer.raw())
            .or_default()
            .war_reward_mute_until_ms = Some(now.wrapping_add(60_000));
        self.tell_guild(killer, b"[LS;558]", &[]);
    }

    /// Whether `id`'s enemy `opp` is a war begun (`CGuild::UnderWar`,
    /// `G/guild_war.cpp:155-162`): the entry exists and its state is `ON_WAR`.
    pub(super) fn guild_under_war(&self, id: u32, opp: u32) -> bool {
        id != opp
            && self
                .guilds
                .by_id
                .get(&id)
                .and_then(|guild| guild.enemies.get(&opp))
                .is_some_and(|enemy| enemy.state == GUILD_WAR_ON_WAR)
    }

    /// The war arm of `battle_is_attackable` (`G/battle.cpp:125-138`): two players whose
    /// guilds are under a war may fight whatever the PK policy says. The legacy's arm
    /// reads the two guilds and `UnderWar` (`G/guild_war.cpp:155-162`), which answers for
    /// a war begun.
    pub(super) fn guild_war_allows(&self, attacker: Vid, victim: Vid) -> bool {
        let guild_of = |vid: Vid| {
            self.characters
                .find_by_vid(vid)
                .ok()
                .and_then(|character| self.guild_of(character.player_id()))
        };
        let (Some(attacker_guild), Some(victim_guild)) = (guild_of(attacker), guild_of(victim))
        else {
            return false;
        };
        self.guild_under_war(attacker_guild, victim_guild)
    }

    /// The forwarded refuse (`CGuildManager::RefuseWar`, `G/guild_manager.cpp:564-580`):
    /// the declarer's master hears `[LS;476]` naming the refuser, then both entries
    /// leave with their end fans.
    fn manager_refuse_war(&mut self, from: u32, to: u32) {
        let from_name = self
            .guilds
            .by_id
            .get(&from)
            .map_or(Vec::new(), |guild| guild.name.clone());
        // `g2` is the declaree the packet names `To`: the refuse packet's `From` is the
        // refuser, so the declarer hears who refused it.
        self.notify_guild_master(to, b"[LS;476;%s]", &[Arg::Text(&from_name)]);
        self.refuse_war(from, to);
        self.refuse_war(to, from);
    }
    /// One side's end (`CGuild::EndWar`, `G/guild_war.cpp:592-627`): a missing enemy is
    /// quiet, else the END fan, the erase, and — when the guild then holds no war — the
    /// strip: the six war-casts (BLOOD 152, BLESS 153, SEONGHWI 154, ACCEL 155, BUNNO 156,
    /// JUMUN 157, `G/skill.h:164-170`) plus `RemoveBadAffect` from every online member.
    /// (The war-map `SetEnded` is unported: no maps.)
    fn end_war_side(&mut self, id: u32, opp: u32, war_type: u8) {
        let present = self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| guild.enemies.contains_key(&opp));
        if !present {
            return;
        }
        self.guild_war_fan(id, opp, war_type, GUILD_WAR_END);
        if let Some(guild) = self.guilds.by_id.get_mut(&id) {
            guild.enemies.remove(&opp);
        }
        if self.guild_under_any_war(id) {
            return;
        }
        let members: Vec<Vid> = self
            .guilds
            .by_id
            .get(&id)
            .map(|guild| {
                guild
                    .members
                    .values()
                    .filter_map(|member| member.vid)
                    .collect()
            })
            .unwrap_or_default();
        for member in members {
            for kind in [152, 153, 154, 155, 156, 157] {
                let _stripped = self.remove_pc_affect(member, kind);
            }
            self.remove_bad_affect(member);
        }
    }

    /// The war's end judged (`CGuildManager::EndWar`, `G/guild_manager.cpp:676-717`,
    /// with the DB side's `WarEnd`/`ProcessWinLose`/`ProcessDraw`/`ChangeLadderPoint`
    /// collapsed in, `db/GuildManager.cpp:407-526`): quiet (`None`) when the pair is
    /// not a live war or a guild is gone. Else the ladder math lands in memory (a draw
    /// tallies both draws and rewrites both ladders unchanged; a win tallies the
    /// winner's wins and the loser's losses, pays `5%` of the loser's ladder halved
    /// under the half-win rule, and bills the loser `7%`, floored at 0), the over
    /// notice fans to every PC, both sides end, the pair stamps the end-time map and
    /// leaves the war set, and the `WAR_END_LIST` fans to every PC. `force_draw` is
    /// `gwstop`'s arm: it turns the math into a draw, as the DB's forced `WarEnd`
    /// does, but leaves the over notice to the scores, as the game side's `EndWar`
    /// does. `math` false is `gwcancel`'s (the math and tallies skip, the
    /// stamp stays). The outcome carries both rows for the descriptor's store write.
    fn manager_end_war(
        &mut self,
        lo: u32,
        hi: u32,
        force_draw: bool,
        math: bool,
        now: u32,
    ) -> Option<WarEndOutcome> {
        if !self.guilds.wars.contains(&(lo, hi)) {
            return None;
        }
        if !self.guilds.by_id.contains_key(&lo) || !self.guilds.by_id.contains_key(&hi) {
            return None;
        }
        let score = |guilds: &Guilds, id: u32, opp: u32| {
            guilds
                .by_id
                .get(&id)
                .and_then(|guild| guild.enemies.get(&opp))
                .map_or(0, |enemy| enemy.score)
        };
        let war_type = self
            .guilds
            .by_id
            .get(&lo)
            .and_then(|guild| guild.enemies.get(&hi))
            .map_or(GUILD_WAR_TYPE_FIELD, |enemy| enemy.war_type);
        // `CGuild::EndWar` (`G/guild_war.cpp:592-627`): the war's map ends with the
        // war, and the players on it leave it.
        let map_index = self
            .guilds
            .by_id
            .get(&lo)
            .and_then(|guild| guild.enemies.get(&hi))
            .map_or(0, |enemy| enemy.map_index);
        let _ = self.war_maps.remove(&map_index);
        let (score_lo, score_hi) = (score(&self.guilds, lo, hi), score(&self.guilds, hi, lo));
        // Two draws, and they are not the same one: the ladder math follows the DB's
        // `WarEnd`, which a forced draw turns into a draw whatever the scores are
        // (`db/GuildManager.cpp:485-500`), while the over notice follows the game
        // side's `EndWar`, which judges the scores alone
        // (`G/guild_manager.cpp:697-703`) — the force flag never reaches the game
        // side, so a `gwstop` on a war with unequal scores still names a winner.
        let notice_draw = score_lo == score_hi;
        let (winner, loser) = if score_lo >= score_hi {
            (lo, hi)
        } else {
            (hi, lo)
        };
        let mut outcome = WarEndOutcome {
            lo: WarEndRow {
                guild_id: lo,
                ladder: 0,
                wins: 0,
                draws: 0,
                losses: 0,
            },
            hi: WarEndRow {
                guild_id: hi,
                ladder: 0,
                wins: 0,
                draws: 0,
                losses: 0,
            },
        };
        if math {
            self.war_end_math(
                (lo, hi),
                winner,
                loser,
                force_draw || notice_draw,
                now,
                &mut outcome,
            );
        }
        if war_type == GUILD_WAR_TYPE_FIELD {
            let (key, first, second) = if notice_draw {
                let lo_name = self.guild_name_of(lo);
                let hi_name = self.guild_name_of(hi);
                (&b"[LS;479;%s;%s]"[..], lo_name, hi_name)
            } else {
                let winner_name = self.guild_name_of(winner);
                let loser_name = self.guild_name_of(loser);
                (&b"[LS;480;%s;%s]"[..], winner_name, loser_name)
            };
            self.notice_every_pc(key, &[Arg::Text(&first), Arg::Text(&second)]);
        }
        self.end_war_side(lo, hi, war_type);
        self.end_war_side(hi, lo, war_type);
        self.guilds.war_end_times.insert((lo, hi), now);
        self.guilds.wars.remove(&(lo, hi));
        let record = guild_war_end_list_record(lo, hi);
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let _sent = self.write_to_client(vid, record.clone());
        }
        Some(outcome)
    }

    /// The ladder math of one end (`WarEnd`/`ProcessWinLose`/`ProcessDraw`,
    /// `db/GuildManager.cpp:470-526`), collapsed into `outcome`'s rows for the sorted
    /// `pair`: `draw` tallies both sides and writes both ladders back unchanged; otherwise
    /// the winner gains 5% of the loser's ladder, halved under the half-win rule, and the
    /// loser loses 7% of its own, floored at 0. `winner`/`loser` come from the score
    /// order, as the game side judges it (`lo` wins ties). The over notice is not this
    /// function's.
    fn war_end_math(
        &mut self,
        pair: (u32, u32),
        winner: u32,
        loser: u32,
        draw: bool,
        now: u32,
        outcome: &mut WarEndOutcome,
    ) {
        let (lo, hi) = pair;
        if draw {
            for (id, row) in [(lo, &mut outcome.lo), (hi, &mut outcome.hi)] {
                if let Some(guild) = self.guilds.by_id.get_mut(&id) {
                    guild.war_draws = guild.war_draws.saturating_add(1);
                    row.ladder = guild.ladder_point;
                    row.wins = guild.war_wins;
                    row.draws = guild.war_draws;
                    row.losses = guild.war_losses;
                }
            }
            return;
        }
        let loser_ladder = self
            .guilds
            .by_id
            .get(&loser)
            .map_or(0, |guild| guild.ladder_point.max(0));
        // Legacy scales by 0.05 and 0.07 in `double` and truncates
        // (`db/GuildManager.cpp:486-499`); for a non-negative `i32` ladder the quotients
        // below are the same truncated ones.
        let mut gain = loser_ladder / 20;
        let loss = i32::try_from(i64::from(loser_ladder) * 7 / 100).unwrap_or(i32::MAX);
        let half = self
            .guilds
            .war_end_times
            .get(&(lo, hi))
            .is_some_and(|ended| ended.saturating_add(GUILD_WAR_LADDER_HALF_PENALTY) > now);
        if half {
            gain /= 2;
        }
        if let Some(guild) = self.guilds.by_id.get_mut(&winner) {
            guild.war_wins = guild.war_wins.saturating_add(1);
            guild.ladder_point = guild.ladder_point.saturating_add(gain).max(0);
            outcome_row(outcome, winner, guild);
        }
        if let Some(guild) = self.guilds.by_id.get_mut(&loser) {
            guild.war_losses = guild.war_losses.saturating_add(1);
            guild.ladder_point = guild.ladder_point.saturating_sub(loss).max(0);
            outcome_row(outcome, loser, guild);
        }
    }

    /// A `CHAT_TYPE_NOTICE` line to every PC (`SendNotice`,
    /// `G/cmd_gm.cpp:1389-1425`): the client set, each descriptor's character, in
    /// VID order.
    fn notice_every_pc(&self, key: &[u8], args: &[Arg<'_>]) {
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            if let Some(mover) = self.own_mover(vid) {
                self.tell(vid, mover, CHAT_TYPE_NOTICE, key, args);
            }
        }
    }

    /// One side's `id` name for the over notice, empty when the guild is gone (the
    /// caller already checked, but the notice reads after the math).
    fn guild_name_of(&self, id: u32) -> Vec<u8> {
        self.guilds
            .by_id
            .get(&id)
            .map_or(Vec::new(), |guild| guild.name.clone())
    }

    /// One side's insert (`CGuild::DeclareWar`, `G/guild_war.cpp:372-384`): false when
    /// the enemy is already there, else the entry plus its fan.
    fn declare_war(&mut self, id: u32, opp: u32, war_type: u8, state: u8) -> bool {
        let fresh = self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| !guild.enemies.contains_key(&opp));
        if !fresh {
            return false;
        }
        let mut enemy = GuildWar::new(war_type);
        enemy.state = state;
        if let Some(guild) = self.guilds.by_id.get_mut(&id) {
            guild.enemies.insert(opp, enemy);
        }
        self.guild_war_fan(id, opp, war_type, state);
        true
    }

    /// One side's refuse (`CGuild::RefuseWar`, `G/guild_war.cpp:558-571`): a sent or
    /// received entry leaves with its end fan.
    fn refuse_war(&mut self, id: u32, opp: u32) {
        let war_type = self
            .guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.enemies.get(&opp))
            .filter(|enemy| {
                enemy.state == GUILD_WAR_SEND_DECLARE || enemy.state == GUILD_WAR_RECV_DECLARE
            })
            .map(|enemy| enemy.war_type);
        let Some(war_type) = war_type else {
            return;
        };
        if let Some(guild) = self.guilds.by_id.get_mut(&id) {
            guild.enemies.remove(&opp);
        }
        self.guild_war_fan(id, opp, war_type, GUILD_WAR_END);
    }

    /// `id`'s war fan against `opp` (`CGuild::GuildWarPacket`, `G/guild_war.cpp:28-58`):
    /// every online member hears the opp's NAME — once per viewer, through the same
    /// `SendGuildName` dedup the view uses — then the WAR record. An on-war fan chats
    /// `[LS;558]` first.
    fn guild_war_fan(&mut self, id: u32, opp: u32, war_type: u8, state: u8) {
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return;
        };
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            if state == GUILD_WAR_ON_WAR {
                self.tell_guild(member, b"[LS;558]", &[]);
            }
            if let Some(name) = self.send_guild_name(member.raw(), opp) {
                let _sent = self.write_to_client(member, name);
            }
            let _sent = self.write_to_client(member, guild_war_record(id, opp, war_type, state));
        }
    }

    /// An info line to `id`'s master, when it is online (`CGuild::NotifyGuildMaster`,
    /// `G/guild_war.cpp:268-274`).
    fn notify_guild_master(&self, id: u32, format: &[u8], args: &[Arg<'_>]) {
        let master = self.guilds.by_id.get(&id).and_then(|guild| {
            guild
                .members
                .get(&guild.master_pid)
                .and_then(|master| master.vid)
        });
        if let Some(master) = master {
            self.tell_guild(master, format, args);
        }
    }

    /// Judge one `GUILD_SUBHEADER_CG_USE_SKILL` (`CGuild::UseSkill`,
    /// `G/guild.cpp:1267-1340`): the guildless `[LS;523]` (the shared gate,
    /// `G/input_main.cpp:2816-2824`), the member+auth quiet, the stun quiet (only
    /// the stun arm of `CanMove` is ported — no private shop), the range quiet,
    /// the missing-row quiet, the level-0 quiet, the SELFONLY-held quiet, then
    /// the strips run (revive-invisible, then Eunhyung 34 — just the remove, not
    /// the cloaked-attack level walk), the cost gates (`[LS;931]` underfunded,
    /// `[LS;932]` unusable), and the cooldown stamps. Answers what the descriptor
    /// must store: the judged power, after the cost.
    pub(super) fn answer_guild_use_skill(
        &mut self,
        vid: Vid,
        vnum: u32,
        pid: u32,
    ) -> GuildSkillUseAnswer {
        let quiet = GuildSkillUseAnswer::Refused { line: None };
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return quiet;
        };
        let speaker = character.player_id();
        let Some(id) = self.guild_of(speaker) else {
            return GuildSkillUseAnswer::Refused {
                line: self.guild_line(vid, b"[LS;523]", &[]),
            };
        };
        let authed = self
            .guilds
            .by_id
            .get(&id)
            .and_then(|guild| guild.grade_auth_of(speaker))
            .is_some_and(|auth| auth & GUILD_AUTH_USE_SKILL != 0);
        if !authed {
            return quiet;
        }
        if self.affect_flags_of(vid)[0] & AFF_STUN_WORD != 0 {
            return quiet;
        }
        let real = vnum.saturating_sub(GUILD_SKILL_START);
        let index = usize::try_from(real).unwrap_or(GUILD_SKILL_COUNT);
        if index >= GUILD_SKILL_COUNT {
            return quiet;
        }
        let Some(row) = self.guild_skill_tables.get(vnum) else {
            return quiet;
        };
        let (max, flag, cost_poly, cool_poly) = (
            row.record.max_level,
            row.record.flag,
            row.sp_cost.clone(),
            row.cooldown.clone(),
        );
        let Some(level) = self.guilds.by_id.get(&id).map(|guild| guild.skills[index]) else {
            return quiet;
        };
        if level == 0 {
            return quiet;
        }
        if flag & gamedata::mob_skill_data::SKILL_FLAG_SELFONLY != 0
            && self
                .affect_events
                .get(&vid)
                .is_some_and(|event| event.affects.iter().any(|affect| affect.kind == vnum))
        {
            return quiet;
        }
        self.remove_revive_invisible(vid);
        let _cloaked = self.remove_pc_affect(vid, SKILL_EUNHYUNG);
        // `k = 1.0 * level / max` (`G/guild.cpp:1301`): the floating division.
        let k = f64::from(level) / f64::from(max.max(1));
        let vars = std::collections::BTreeMap::from([("k", k)]);
        let needed = f64_to_i32(cost_poly.eval(&vars));
        let power = self.guilds.by_id.get(&id).map_or(0, |guild| guild.power);
        if i64::from(power) < i64::from(needed) {
            return GuildSkillUseAnswer::Refused {
                line: self.guild_line(
                    vid,
                    b"[LS;931;%d;%d]",
                    &[Arg::Int(i64::from(power)), Arg::Int(i64::from(needed))],
                ),
            };
        }
        let usable = self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| guild.usable[index]);
        if !usable {
            return GuildSkillUseAnswer::Refused {
                line: self.guild_line(vid, b"[LS;932]", &[]),
            };
        }
        let cool_secs = f64_to_i32(cool_poly.eval(&vars)).max(0);
        let due = self.last_pulse.saturating_add(
            u64::try_from(cool_secs)
                .unwrap_or(u64::MAX)
                .saturating_mul(25),
        );
        if let Some(guild) = self.guilds.by_id.get_mut(&id) {
            guild.usable[index] = false;
            guild.usable_due[index] = Some(due);
        }
        GuildSkillUseAnswer::Store {
            guild_id: id,
            vnum,
            pid,
            power: power.saturating_sub(u32::try_from(needed.max(0)).unwrap_or(0)),
        }
    }

    /// Commit one judged cast under the stored power (`UseSkill`'s branch,
    /// `G/guild.cpp:1340-1418`): the power lands with the `MINMAX(0, power,
    /// max_power)` clamp (`GuildPointChange`'s SP arm, `:1572-1581`), the
    /// `SKILL_INFO` fans, then the branch runs. 158 teleports to any online PID
    /// (`CHARACTER_MANAGER::FindByPID` is global, not guild-scoped): same-Channel
    /// orders the warp, cross-Channel chats `[LS;934]` with (victim, own), offline
    /// chats `[LS;935]`. Every other vnum chats `[LS;937]`: no wars run, so the
    /// `RemoveAffect` + `ComputeSkill` buff loop waits for `sys.guild.war` — and
    /// the SP stays consumed, as legacy consumes before branching.
    pub(super) fn commit_guild_use_skill(
        &mut self,
        commit: &GuildSkillUseCommit,
    ) -> GuildSkillUseAnswer {
        let GuildSkillUseCommit {
            speaker,
            guild_id,
            vnum,
            pid,
            power,
        } = *commit;
        let Some(guild) = self.guilds.by_id.get_mut(&guild_id) else {
            return GuildSkillUseAnswer::Committed;
        };
        guild.power = power.min(guild.max_power);
        let info = guild.skill_info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_skill_info_record(&info));
        }
        if vnum != GUILD_SKILL_TELEPORT {
            self.tell_guild(speaker, b"[LS;937]", &[]);
            return GuildSkillUseAnswer::Committed;
        }
        let victim = self
            .characters
            .find_by_pid(pid)
            .ok()
            .map(world::character::Character::vid);
        let Some(victim) = victim else {
            self.tell_guild(speaker, b"[LS;935]", &[]);
            return GuildSkillUseAnswer::Committed;
        };
        let (Some(own), Some(theirs)) = (self.place_of(speaker.raw()), self.place_of(victim.raw()))
        else {
            self.tell_guild(speaker, b"[LS;935]", &[]);
            return GuildSkillUseAnswer::Committed;
        };
        if own.0 != theirs.0 {
            self.tell_guild(
                speaker,
                b"[LS;934;%d;%d]",
                &[Arg::Int(i64::from(theirs.0)), Arg::Int(i64::from(own.0))],
            );
            return GuildSkillUseAnswer::Committed;
        }
        let Some(spot) = self.spot_of(victim.raw()) else {
            self.tell_guild(speaker, b"[LS;935]", &[]);
            return GuildSkillUseAnswer::Committed;
        };
        self.order_warp(speaker, (spot.x, spot.y));
        GuildSkillUseAnswer::Committed
    }

    /// Commit one judged charge under the stored row (`ChargeSP`, `G/guild.cpp:1804-1831`,
    /// plus `GuildPointChange`'s SP arm, `:1572-1581`): the power lands and the `SKILL_INFO`
    /// fans through the outboxes, and the `[LS;938]` count answers on the speaker's own
    /// session. `%u` prints as `%d` does here: the gained SP is never negative.
    fn commit_guild_charge(
        &mut self,
        id: u32,
        speaker: Vid,
        power: u32,
        gained: u32,
    ) -> GuildGradeAnswer {
        let empty = GuildGradeAnswer::Committed { post: None };
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return empty;
        };
        guild.power = power;
        let info = guild.skill_info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_skill_info_record(&info));
        }
        // `ChargeSP`'s count (`G/guild.cpp:1830`) chats to the speaker after the
        // store, on its own session: the descriptor sends `post`.
        GuildGradeAnswer::Committed {
            post: self.guild_line(speaker, b"[LS;938;%u]", &[Arg::Int(i64::from(gained))]),
        }
    }

    /// Lapse the invites due on `pulse` (`GuildInviteEvent`, `G/guild.cpp:2013-2031`):
    /// quiet, as legacy's handler is. Then end the field wars whose 30 minutes passed
    /// (the DB server's `Update` heap pop, `db/GuildManager.cpp:244-278`): score-checked,
    /// never forced. The outcomes queue in `pending_war_ends` for the poll command —
    /// the tick has no descriptor of its own. `process_pulse` calls this after the
    /// regens, with the other ticks.
    pub(super) fn run_guild_ticks(&mut self, pulse: u64) {
        self.guilds.invites.retain(|_, due| *due > pulse);
        // The DB server's `m_pqSkill` pump (`db/GuildManager.cpp:280-286`): a due
        // cooldown re-arms, quietly (the client never learns the cooldown — it just
        // answers 932 while unusable). A 0-second cooldown re-arms on the next tick
        // (`now + 0 <= now` on the tick after the cast), so a second cast on the
        // same Pulse still answers 932.
        for guild in self.guilds.by_id.values_mut() {
            for (index, due) in guild.usable_due.iter_mut().enumerate() {
                if due.is_some_and(|due| due <= pulse) {
                    *due = None;
                    guild.usable[index] = true;
                }
            }
        }
        let now = super::quests::quest_now();
        for &(lo, hi) in &self.guilds.wars.clone() {
            let start = self
                .guilds
                .by_id
                .get(&lo)
                .and_then(|guild| guild.enemies.get(&hi))
                .map_or(0, |enemy| enemy.war_start_time);
            if start.saturating_add(GUILD_WAR_DURATION) > now {
                continue;
            }
            if let Some(outcome) = self.manager_end_war(lo, hi, false, true, now) {
                self.guilds.pending_war_ends.push(outcome);
            }
        }
    }

    /// Drain the war ends the tick queued: the poll command's answer.
    pub(super) fn take_war_ends(&mut self) -> Vec<WarEndOutcome> {
        std::mem::take(&mut self.guilds.pending_war_ends)
    }

    /// The live wars, in pair order (`CGuildManager::ShowGuildWarList`,
    /// `G/guild_manager.cpp:785-800`): each pair's lower guild first, with its name and
    /// id, then the higher, and the age from the lower side's start stamp. A pair whose
    /// guilds are gone is skipped; legacy `TouchGuild` would ghost one, but a war's
    /// guilds are not removed while it lives.
    pub(super) fn guild_war_list(&self, now: u32) -> Vec<GuildWarListLine> {
        self.guilds
            .wars
            .iter()
            .filter_map(|&(lo, hi)| {
                let lo_guild = self.guilds.by_id.get(&lo)?;
                let hi_guild = self.guilds.by_id.get(&hi)?;
                let start = lo_guild
                    .enemies
                    .get(&hi)
                    .map_or(0, |enemy| enemy.war_start_time);
                Some(GuildWarListLine {
                    lo_name: lo_guild.name.clone(),
                    lo_id: lo,
                    hi_name: hi_guild.name.clone(),
                    hi_id: hi,
                    seconds: now.saturating_sub(start),
                })
            })
            .collect()
    }

    /// Judge one `CG_ANSWER_MAKE_GUILD` (`CInputMain::AnswerMakeGuild`,
    /// `G/input_main.cpp:2672-2730`, plus `CreateGuild`'s name and duplicate arms,
    /// `G/guild_manager.cpp:77-119`): the 200000 gold, the disband and withdraw delays,
    /// the has-guild arm, the name shape, and the in-memory duplicate. `now` is
    /// `get_global_time()` in seconds, which the descriptor reads; the quest and event
    /// flags come from the world's quests, defaulting to 0 when unset (`PC::GetFlag`
    /// returns 0, `G/questpc.cpp:92-102`; `GetEventFlag` returns 0,
    /// `G/questmanager.cpp:1691-1699`).
    ///
    /// The name is judged as legacy judges it: the bytes before the first NUL
    /// (`strlcpy(cp.name, p->guild_name, ...)` then `check_name`), as ASCII letters and
    /// digits of length 2-12 (`check_name_alphabet` on europe,
    /// `G/locale_service.cpp:317-337,1156`, plus the 13-byte field's length), refusing the
    /// banned words and the mob names (`check_name_independent`, `:86-99`), which the
    /// world's name rules hold. The `@fixme143` escape check (`:88-93`) is interpolation
    /// defence; bound parameters retire it, so no arm.
    pub(super) fn answer_make_guild(
        &self,
        vid: Vid,
        field: &[u8; GUILD_NAME_FIELD_SIZE],
        now: u32,
    ) -> GuildMakeAnswer {
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return GuildMakeAnswer::Refused { line: None };
        };
        if character.gold() < GUILD_CREATE_FEE {
            return GuildMakeAnswer::Refused { line: None };
        }
        let pid = character.player_id();
        let quest_flag = |name: &[u8]| {
            self.quests
                .as_ref()
                .and_then(|quests| quests.manager().quest_flag(pid, name))
                .unwrap_or(0)
        };
        let event_flag = |name: &[u8]| {
            self.quests
                .as_ref()
                .map_or(0, |quests| quests.manager().event_flag(name))
        };
        let disband_at = quest_flag(b"guild_manage.new_disband_time");
        if i64::from(now) - i64::from(disband_at)
            < i64::from(event_flag(b"guild_disband_delay")) * DAY_SECONDS
        {
            return GuildMakeAnswer::Refused {
                line: Some(self.guild_delay_line(
                    vid,
                    b"[LS;678;%d]",
                    event_flag(b"guild_disband_delay"),
                )),
            };
        }
        let withdraw_at = quest_flag(b"guild_manage.new_withdraw_time");
        if i64::from(now) - i64::from(withdraw_at)
            < i64::from(event_flag(b"guild_withdraw_delay")) * DAY_SECONDS
        {
            return GuildMakeAnswer::Refused {
                line: Some(self.guild_delay_line(
                    vid,
                    b"[LS;679;%d]",
                    event_flag(b"guild_withdraw_delay"),
                )),
            };
        }
        if self.guild_of(pid).is_some() {
            return GuildMakeAnswer::Refused { line: None };
        }
        let end = field
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(field.len());
        let name = &field[..end];
        if name.len() < 2
            || name.len() > GUILD_NAME_MAX_LEN
            || !name.iter().all(u8::is_ascii_alphanumeric)
            || !self.name_rules.guild_name_ok(name)
        {
            return GuildMakeAnswer::Refused {
                line: Some(self.guild_info_line(vid, b"[LS;680]")),
            };
        }
        if self
            .guilds
            .by_id
            .values()
            .any(|guild| guild.name.eq_ignore_ascii_case(name))
        {
            return GuildMakeAnswer::Refused {
                line: Some(self.guild_info_line(vid, b"[LS;473]")),
            };
        }
        GuildMakeAnswer::Store {
            name: String::from_utf8_lossy(name).into_owned(),
        }
    }

    /// Commit one judged creation under the id the store answered (`AnswerMakeGuild`'s
    /// `dwGuildID` arm, `G/input_main.cpp:2712-2729`, plus the constructor's sends,
    /// `G/guild.cpp:59-127`, and the master's `LoginMember`, `:270-296`): the registry and
    /// the master's link go in, the fee and one 70101 item leave the master's pockets, and
    /// the answer carries every record — to the master and to every PC — plus the fee and
    /// the item half for the descriptor to store with the guild rows.
    ///
    /// Nothing leaves through the outbox: the descriptor sends the records with the same
    /// `send_all` it sends any record with. One `SetGold` plus one item walk mutate the
    /// character; the quickslots are untouched (70101 is not a potion, so no slot can name
    /// it, and the master's slots were never synced into the world anyway).
    ///
    /// The master's own order is legacy's: the `GUILD_NAME` record (the broadcast reaches
    /// the master first, inside `CreateGuild`), the `[LS;682]` line, the fee
    /// (`PointChange(POINT_GOLD, -fee)`), the logs (unported: `sys.log` owns no sink), the
    /// item (`RemoveSpecifyItem(70101, 1)`), then the login walk (`RequestAddMember` →
    /// `AddMember` → `LoginMember`): the LIST to an empty online set sends nothing, and
    /// `LoginMember` sends the four self packets plus its own LOGIN.
    ///
    /// The store already holds the guild rows; a gone master or a master that joined a
    /// guild while the rows were written leaves the world unchanged and answers `[LS;683]`,
    /// as a zero `dwGuildID` answers it.
    pub(super) fn commit_make_guild(
        &mut self,
        vid: Vid,
        commit: &GuildMakeCommit,
        mover: Mover,
    ) -> GuildMakeAnswer {
        let Ok(master) = self.characters.find_by_vid(vid) else {
            return GuildMakeAnswer::Refused { line: None };
        };
        let pid = master.player_id();
        let name = master.name().to_owned();
        if self.guild_of(pid).is_some() {
            return GuildMakeAnswer::Refused {
                line: Some(self.guild_info_line(vid, b"[LS;683]")),
            };
        }
        let (job, level) = self
            .bodies
            .get(&vid)
            .map_or((0, 0), |body| (body.card.job, body.card.level));
        let mut grades: [(Vec<u8>, u8); GUILD_GRADE_COUNT] =
            std::array::from_fn(|_| (FOLLOWER_GRADE_NAME.to_vec(), 0));
        grades[0] = (LEADER_GRADE_NAME.to_vec(), LEADER_AUTH);
        let mut members = BTreeMap::new();
        members.insert(
            pid,
            GuildMember {
                grade: GUILD_LEADER_GRADE,
                is_general: 0,
                job,
                level,
                offer: 0,
                name: name.as_bytes().to_vec(),
                vid: Some(vid),
            },
        );
        self.guilds.by_id.insert(
            commit.id,
            Guild {
                id: commit.id,
                master_pid: pid,
                exp: 0,
                level: 1,
                name: commit.name.as_bytes().to_vec(),
                grades,
                skill_point: 0,
                skills: [0; GUILD_SKILL_COUNT],
                power: GUILD_BASE_POWER,
                max_power: GUILD_BASE_POWER,
                gold: 0,
                member_count_bonus: 0,
                members,
                ladder_point: 0,
                war_wins: 0,
                war_draws: 0,
                war_losses: 0,
                post_comment_due: 0,
                usable: [true; GUILD_SKILL_COUNT],
                usable_due: [None; GUILD_SKILL_COUNT],
                enemies: BTreeMap::new(),
            },
        );
        self.guilds.of.insert(pid, commit.id);
        let name_record = guild_name_record(commit.id, commit.name.as_bytes());
        let mut own = vec![
            name_record.clone(),
            self.guild_made_line(mover, commit.name.as_bytes()),
        ];
        let (due, changes, records) = self.take_guild_fee(vid, mover, pid);
        own.extend(records);
        own.extend(self.guild_login_records(commit.id));
        GuildMakeAnswer::Committed {
            id: commit.id,
            own,
            others: vec![name_record],
            gold: due,
            changes,
        }
    }

    /// Take the creation's fee and item from the master online under `vid`
    /// (`PointChange(POINT_GOLD, -200000)` and `RemoveSpecifyItem(70101, 1)`,
    /// `G/input_main.cpp:2717-2728`): the gold and item rows, and the records the
    /// master hears. Answers the stored-gold delta, the row changes, and the records.
    fn take_guild_fee(
        &mut self,
        vid: Vid,
        mover: Mover,
        pid: u32,
    ) -> (i64, Vec<db::items::RowChange>, Vec<Vec<u8>>) {
        let before = self
            .characters
            .find_by_vid(vid)
            .map_or(GUILD_CREATE_FEE, Character::gold);
        let after = before.saturating_sub(GUILD_CREATE_FEE);
        let mut item_done = MoveDone {
            kind: MoveKind::GuildMade,
            records: Vec::new(),
            changes: Vec::new(),
        };
        if let Ok(character) = self.characters.find_by_vid_mut(vid) {
            character.set_gold(after);
            item_done = remove_create_item(character);
        }
        let fee = MoveDone {
            kind: MoveKind::GuildMade,
            records: std::iter::once(MoveRecord::Gold {
                amount: 0,
                value: after,
            })
            .chain(item_done.records)
            .collect(),
            changes: item_done.changes,
        };
        let changes = fee.changes.clone();
        let mut packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            fee,
            mover,
            &self.protos,
            &self.locale,
        );
        packed.gold = Some(gold_delta(before, after));
        let due = packed.gold.unwrap_or(0);
        let records = packed.records.clone();
        (due, row_changes(pid, &changes), records)
    }

    /// An info line to the player under `vid`, built the way `CHARACTER::ChatPacket`
    /// builds one.
    fn guild_info_line(&self, vid: Vid, format: &[u8]) -> Vec<u8> {
        let Some(mover) = self.own_mover(vid) else {
            return Vec::new();
        };
        chat_packet(mover.recipient(&self.locale), CHAT_TYPE_INFO, format, &[])
    }

    /// An info line with one integer argument, for the delay gates (`[LS;678;%d]`,
    /// `[LS;679;%d]`, which format the raw event flag, `G/input_main.cpp:2683-2695`).
    fn guild_delay_line(&self, vid: Vid, format: &[u8], flag: i32) -> Vec<u8> {
        let Some(mover) = self.own_mover(vid) else {
            return Vec::new();
        };
        chat_packet(
            mover.recipient(&self.locale),
            CHAT_TYPE_INFO,
            format,
            &[Arg::Int(i64::from(flag))],
        )
    }

    /// The `[LS;682;%s]` line (`G/input_main.cpp:2716`): the new guild's name, formatted as
    /// a C string.
    fn guild_made_line(&self, mover: Mover, name: &[u8]) -> Vec<u8> {
        chat_packet(
            mover.recipient(&self.locale),
            CHAT_TYPE_INFO,
            b"[LS;682;%s]",
            &[Arg::Text(name)],
        )
    }

    /// The login walk as records, for the master's own answer (`LoginMember`,
    /// `G/guild.cpp:270-296`): the four self packets plus its own LOGIN. The LOGIN to the
    /// other already-online members is empty at creation — the master is the only member —
    /// so no fan is carried.
    fn guild_login_records(&self, id: u32) -> Vec<Vec<u8>> {
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return Vec::new();
        };
        let grades: [GcGuildGrade; GUILD_GRADE_COUNT] = std::array::from_fn(|index| {
            let (name, auth) = &guild.grades[index];
            GcGuildGrade {
                name: pad_grade_name(name),
                auth: *auth,
            }
        });
        let mut out = vec![
            guild_grade_record(&grades),
            guild_info_record(&guild.info()),
            guild_list_record(&guild.list()),
        ];
        for other_pid in guild
            .members
            .iter()
            .filter_map(|(pid, member)| member.vid.map(|_| pid))
        {
            out.push(guild_login_record(*other_pid));
        }
        out.push(guild_skill_info_record(&guild.skill_info()));
        out
    }

    /// Answer one `guild.*` quest call for the character online under `vid` (230s): the
    /// world's half of `quest::manager::Game::guild_call`. Readers answer from memory;
    /// writers judge, fan through the outboxes, and queue their store halves for the
    /// descriptor to drain after the step, in order.
    pub(super) fn answer_guild_quest(
        &mut self,
        vid: Vid,
        name: &[u8],
        args: &[quest::manager::GuildArg],
        now: u32,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let number = |value: u32| GuildAnswer::Number(f64::from(value));
        let signed = |value: i32| GuildAnswer::Number(f64::from(value));
        let first = args.first();
        let second = args.get(1);
        let third = args.get(2);
        let caller = self
            .characters
            .find_by_vid(vid)
            .map(world::character::Character::player_id)
            .ok();
        let own = caller.and_then(|pid| self.guild_of(pid));
        let quiet = GuildQuestJudged::silent;
        match name {
            b"get_rank" => quiet(match own {
                None => signed(-1),
                Some(id) => signed(self.guild_rank(id)),
            }),
            b"get_ladder_point" => quiet(match own.and_then(|id| self.guild_ladder(id)) {
                None => signed(-1),
                Some(point) => signed(point),
            }),
            b"high_ranking_string" => quiet(GuildAnswer::Text(self.guild_high_rank(own))),
            b"around_ranking_string" => quiet(GuildAnswer::Text(self.guild_around_rank(own))),
            b"name" | b"get_name" => {
                let id = match first {
                    // `lua_isnumber(L, 1)`: a number-like argument. A ` guild.get_name`
                    // non-number pushes `""`; a `guild.name` non-number logs and pushes
                    // nothing — both arrive here, and the soft arm wins (V19l).
                    Some(GuildArg::Number(id)) => *id,
                    _ => return quiet(GuildAnswer::Text(Vec::new())),
                };
                quiet(GuildAnswer::Text(self.guild_named(id)))
            }
            b"level" => {
                let id = match first {
                    Some(GuildArg::Number(id)) => *id,
                    // `luaL_checknumber` errors; the bridge answered the error.
                    _ => return quiet(GuildAnswer::Number(0.0)),
                };
                quiet(number(self.guild_level_of(id)))
            }
            b"get_member_count" => quiet(number(self.guild_member_count(own))),
            b"get_id0" => quiet(number(own.unwrap_or(0))),
            b"get_sp0" => quiet(number(self.guild_power(own))),
            b"get_maxsp0" => quiet(number(self.guild_max_power(own))),
            b"get_money0" => quiet(number(self.guild_gold(own))),
            b"get_max_member0" => quiet(number(u32::from(self.guild_max_members(own)))),
            b"get_total_member_level0" => quiet(number(self.guild_total_level(own))),
            b"has_land0" => quiet(GuildAnswer::Boolean(false)),
            b"get_win_count0" => quiet(signed(self.guild_war_tally(own, 0))),
            b"get_draw_count0" => quiet(signed(self.guild_war_tally(own, 1))),
            b"get_loss_count0" => quiet(signed(self.guild_war_tally(own, 2))),
            b"get_skill_level0" => {
                let vnum = match first {
                    Some(GuildArg::Number(vnum)) => *vnum,
                    _ => 0,
                };
                quiet(signed(self.guild_skill_level(own, vnum)))
            }
            b"get_skill_point0" => quiet(number(self.guild_skill_point(own))),
            b"get_exp_level0" => {
                let level = match first {
                    Some(GuildArg::Number(level)) => *level,
                    _ => 0,
                };
                let clamped = level.min(u32::from(GUILD_MAX_LEVEL));
                quiet(number(
                    GUILD_LEVELUP_EXP[usize::try_from(clamped).unwrap_or(0)],
                ))
            }
            b"change_master" => self.quest_change_master(vid, caller, own, first, now, None),
            b"change_master_with_limit" => self.quest_change_master(
                vid,
                caller,
                own,
                first,
                now,
                Some((second, third, args.get(3), args.get(4), args.get(5))),
            ),
            b"add_comment0" => self.quest_add_comment(vid, caller, own, first),
            b"set_ladder_point0" => self.quest_set_ladder(vid, own, first),
            b"set_skill_level0" => self.quest_set_skill_level(vid, own, first, second, third),
            b"set_skill_point0" => self.quest_set_skill_point(vid, own, first),
            b"offer_exp0" => self.quest_offer_exp(vid, caller, own, first),
            b"give_exp0" => self.quest_give_exp(vid, own, first),
            _ => quiet(GuildAnswer::None),
        }
    }

    /// `CGuildManager::GetRank` (`G/guild_manager.cpp:285-297`): one plus the guilds whose
    /// ladder point stands above the guild's.
    fn guild_rank(&self, id: u32) -> i32 {
        let Some(point) = self.guild_ladder(id) else {
            return 1;
        };
        let above = self
            .guilds
            .by_id
            .values()
            .filter(|guild| guild.ladder_point > point)
            .count();
        i32::try_from(above).unwrap_or(i32::MAX).saturating_add(1)
    }

    /// The guild's ladder point, when it holds one.
    fn guild_ladder(&self, id: u32) -> Option<i32> {
        self.guilds.by_id.get(&id).map(|guild| guild.ladder_point)
    }

    /// The guilds in `FGuildCompare` order (`G/guild_manager.cpp:299-318`): ladder point,
    /// then wins, then losses, then the name, each ascending — the ranking strings read
    /// the sorted vector from the top.
    fn guild_ranked(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.guilds.by_id.keys().copied().collect();
        ids.sort_by(|left, right| {
            let guilds = &self.guilds.by_id;
            let (left, right) = (&guilds[left], &guilds[right]);
            (
                left.ladder_point,
                left.war_wins,
                left.war_losses,
                &left.name,
            )
                .cmp(&(
                    right.ladder_point,
                    right.war_wins,
                    right.war_losses,
                    &right.name,
                ))
        });
        ids
    }

    /// One ranking line (`G/guild_manager.cpp:365-370`): `%3d | %-12s | %-8d | %4d |
    /// %4d | %4d` of the rank, the name, the ladder, the wins, the draws, the losses.
    fn guild_rank_line(&self, id: u32) -> Vec<u8> {
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return Vec::new();
        };
        // `%-12s` pads with spaces to 12; a longer name overflows the column, as
        // `snprintf` lets it.
        let mut name = guild.name.clone();
        name.resize(name.len().max(12), b' ');
        format!(
            "{:>3} | {} | {:<8} | {:>4} | {:>4} | {:>4}",
            self.guild_rank(id),
            String::from_utf8_lossy(&name),
            guild.ladder_point,
            guild.war_wins,
            guild.war_draws,
            guild.war_losses,
        )
        .into_bytes()
    }

    /// `CGuildManager::GetHighRankString` (`G/guild_manager.cpp:320-384`): the top eight
    /// guilds with a positive ladder, the caller's own wrapped in the yellow `[COLOR]`.
    fn guild_high_rank(&self, own: Option<u32>) -> Vec<u8> {
        let ranked = self.guild_ranked();
        let mut out = Vec::new();
        let mut shown = 0;
        for id in ranked.iter().rev() {
            if shown >= 8 {
                break;
            }
            let Some(guild) = self.guilds.by_id.get(id) else {
                continue;
            };
            if guild.ladder_point <= 0 {
                break;
            }
            if Some(*id) == own {
                out.extend_from_slice(b"[COLOR r;255|g;255|b;0]");
            }
            if shown > 0 {
                out.extend_from_slice(b"[ENTER]");
            }
            out.extend_from_slice(&self.guild_rank_line(*id));
            if Some(*id) == own {
                out.extend_from_slice(b"[/COLOR]");
            }
            shown += 1;
        }
        out
    }

    /// `CGuildManager::GetAroundRankString` (`G/guild_manager.cpp:386-470`): the seven
    /// around the caller's own — three above, it, three below — the own wrapped in the
    /// yellow `[COLOR]`. Empty when the caller holds no guild.
    fn guild_around_rank(&self, own: Option<u32>) -> Vec<u8> {
        let Some(own) = own else {
            return Vec::new();
        };
        let ranked = self.guild_ranked();
        let Some(position) = ranked.iter().position(|id| *id == own) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut shown = 0;
        for offset in -3..=3 {
            let Some(index) = position.checked_add_signed(offset) else {
                continue;
            };
            let Some(id) = ranked.get(index) else {
                continue;
            };
            if *id == own {
                out.extend_from_slice(b"[COLOR r;255|g;255|b;0]");
            }
            if shown > 0 {
                out.extend_from_slice(b"[ENTER]");
            }
            out.extend_from_slice(&self.guild_rank_line(*id));
            shown += 1;
            if *id == own {
                out.extend_from_slice(b"[/COLOR]");
            }
        }
        out
    }

    /// The name of guild `id` (`CGuildManager::FindGuild`, `G/guild_manager.cpp:238-246`),
    /// empty when no guild holds it.
    fn guild_named(&self, id: u32) -> Vec<u8> {
        self.guilds
            .by_id
            .get(&id)
            .map_or_else(Vec::new, |guild| guild.name.clone())
    }

    /// The level of guild `id`, 0 when no guild holds it.
    fn guild_level_of(&self, id: u32) -> u32 {
        self.guilds
            .by_id
            .get(&id)
            .map_or(0, |guild| u32::from(guild.level))
    }

    /// The member count of the caller's guild, 0 when it holds none.
    fn guild_member_count(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| {
                u32::try_from(guild.members.len()).unwrap_or(u32::MAX)
            })
    }

    /// The power of the caller's guild, 0 when it holds none.
    fn guild_power(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| guild.power)
    }

    /// The max power of the caller's guild, 0 when it holds none.
    fn guild_max_power(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| guild.max_power)
    }

    /// The gold of the caller's guild, 0 when it holds none.
    fn guild_gold(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| guild.gold)
    }

    /// The max members of the caller's guild, 0 when it holds none.
    fn guild_max_members(&self, own: Option<u32>) -> u16 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, Guild::max_member_count)
    }

    /// The summed member levels of the caller's guild (`GetTotalLevel`,
    /// `G/guild.cpp:1792-1801`), 0 when it holds none.
    fn guild_total_level(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| {
                guild
                    .members
                    .values()
                    .map(|member| u32::from(member.level))
                    .sum()
            })
    }

    /// One war tally of the caller's guild (`m_data.win/draw/loss`), 0 when it holds
    /// none. `which` is 0 for wins, 1 for draws, else losses.
    fn guild_war_tally(&self, own: Option<u32>, which: u8) -> i32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| match which {
                0 => guild.war_wins,
                1 => guild.war_draws,
                _ => guild.war_losses,
            })
    }

    /// The level of the caller's guild's skill `vnum` (`GetSkillLevel`,
    /// `G/guild.cpp:1446-1454`): the `(vnum - 151)` slot, 0 past the twelve or without
    /// a guild.
    fn guild_skill_level(&self, own: Option<u32>, vnum: u32) -> i32 {
        let real = vnum.wrapping_sub(GUILD_SKILL_START);
        let index = usize::try_from(real).unwrap_or(GUILD_SKILL_COUNT);
        if index >= GUILD_SKILL_COUNT {
            return 0;
        }
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| i32::from(guild.skills[index]))
    }

    /// The skill point of the caller's guild, 0 when it holds none.
    fn guild_skill_point(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.guilds.by_id.get(&id))
            .map_or(0, |guild| u32::from(guild.skill_point))
    }

    /// `guild_change_master` (`G/questlua_guild.cpp:305-340`) and `with_limit` (`:346-433`):
    /// the 0/1/2/3/4 ladder, plus the limit arm's 5/6/7. `limits` holds the `with_limit`
    /// arguments past the name — the min level, resign, other-leader, other-member and the
    /// ignore flag — when the call is it. The six stamps land through the manager now,
    /// as the withdraw stamp does; the new master is online, so `GetPC` returns its PC.
    fn quest_change_master(
        &mut self,
        vid: Vid,
        caller: Option<u32>,
        own: Option<u32>,
        name: Option<&quest::manager::GuildArg>,
        now: u32,
        limits: Option<WithLimitArgs<'_>>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let number = |value: u32| GuildQuestJudged::silent(GuildAnswer::Number(f64::from(value)));
        let (Some(caller), Some(id)) = (caller, own) else {
            return number(4);
        };
        let Some(guild) = self.guilds.by_id.get(&id) else {
            return number(4);
        };
        if guild.master_pid != caller {
            return number(1);
        }
        let Some(GuildArg::Text(want)) = name else {
            return number(0);
        };
        // `GetMemberPID` (`G/guild.cpp:694-703`): the first member in PID order whose
        // name matches byte-exact, 0 when none does — reached only when `FindPC` found
        // an online character of the name, whose PID then fails the member lookup.
        let member_pid = self.quest_member_pid(id, want);
        let Some(args) = limits else {
            if member_pid == 0 {
                return number(2);
            }
            self.commit_quest_master(vid, id, caller, member_pid);
            return GuildQuestJudged {
                answer: GuildAnswer::Number(3.0),
                own: Vec::new(),
                stores: vec![GuildQuestStore::Master {
                    guild_id: id,
                    from: caller,
                    to: member_pid,
                }],
            };
        };
        self.quest_change_master_limited(vid, caller, id, now, want, args)
    }

    /// `CGuild::GetMemberPID` for the quest master judges: the first member in PID order
    /// whose name matches byte-exact, 0 when none does.
    fn quest_member_pid(&self, id: u32, want: &[u8]) -> u32 {
        self.guilds
            .by_id
            .get(&id)
            .and_then(|guild| {
                guild
                    .members
                    .iter()
                    .find(|(_, member)| member.name == *want)
                    .map(|(pid, _)| *pid)
            })
            .unwrap_or(0)
    }

    /// The `with_limit` arm of [`Self::quest_change_master`] (`G/questlua_guild.cpp:346-433`):
    /// the 5/6/7 gates, the member check, and the six stamps on the 3.
    fn quest_change_master_limited(
        &mut self,
        vid: Vid,
        caller: u32,
        id: u32,
        now: u32,
        want: &[u8],
        args: WithLimitArgs<'_>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let number = |value: u32| GuildQuestJudged::silent(GuildAnswer::Number(f64::from(value)));
        let (min_level, resign, other_leader, other_member, ignore) = args;
        // `FindPC` (`G/char_manager.cpp:244-259`): the online character of the
        // lower-cased name, case-insensitively.
        let Ok(named) = String::from_utf8(want.to_vec()) else {
            return number(5);
        };
        let Ok(target) = self.characters.find_player_by_name(&named) else {
            return number(5);
        };
        let target_pid = target.player_id();
        let target_level = target.points().map_or(0, world::character::Points::level);
        let floor = match min_level {
            Some(GuildArg::Number(floor)) => *floor,
            _ => 0,
        };
        if u32::from(target_level) < floor {
            return number(6);
        }
        // `nBeOtherLeader > get_global_time()`: the new master's flag, read from its own
        // PC — it is online, so `GetPC` returns it, and a missing flag reads 0.
        let held = self
            .quests
            .as_ref()
            .and_then(|quests| {
                quests
                    .manager()
                    .quest_flag(target_pid, b"change_guild_master.be_other_leader")
            })
            .unwrap_or(0);
        // `lua_toboolean(L, 6)`: any live value but false and nil (numbers arrive
        // dworded, so 0 is the number 0, which is true), and a missing sixth is nil.
        let ignored = !matches!(ignore, None | Some(GuildArg::Text(_)));
        let held = if ignored { 0 } else { held };
        if u64::from(u32::try_from(held.max(0)).unwrap_or(u32::MAX)) > u64::from(now) {
            return number(7);
        }
        let member_pid = self.quest_member_pid(id, want);
        if member_pid == 0 || member_pid != target_pid {
            return number(2);
        }
        self.commit_quest_master(vid, id, caller, member_pid);
        if let Some(quests) = self.quests.as_mut() {
            // `(int)lua_tonumber`: a non-number's 0, else the dword re-read as signed.
            let stamp = |arg: Option<&quest::manager::GuildArg>| match arg {
                Some(GuildArg::Number(beats)) => i32::from_ne_bytes(beats.to_ne_bytes()),
                _ => 0,
            };
            let manager = quests.manager_mut();
            manager.set_quest_flag(target_pid, b"change_guild_master.be_other_leader", 0);
            manager.set_quest_flag(target_pid, b"change_guild_master.be_other_member", 0);
            manager.set_quest_flag(
                target_pid,
                b"change_guild_master.resign_limit",
                stamp(resign),
            );
            manager.set_quest_flag(
                caller,
                b"change_guild_master.be_other_leader",
                stamp(other_leader),
            );
            manager.set_quest_flag(
                caller,
                b"change_guild_master.be_other_member",
                stamp(other_member),
            );
            manager.set_quest_flag(caller, b"change_guild_master.resign_limit", 0);
        }
        GuildQuestJudged {
            answer: GuildAnswer::Number(3.0),
            own: Vec::new(),
            stores: vec![GuildQuestStore::Master {
                guild_id: id,
                from: caller,
                to: member_pid,
            }],
        }
    }

    /// Run the master change `quest_change_master` judged (`ChangeMasterTo`'s commit,
    /// `db/GuildManager.cpp:1175-1192`): the master's PID, the new master's grade 1, the
    /// old master's grade 15, and the INFO plus the grades to every online member
    /// (`SendGuildDataUpdateToAllMember`, `G/guild.cpp:2226-2235`).
    fn commit_quest_master(&mut self, _vid: Vid, id: u32, from: u32, to: u32) {
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return;
        };
        guild.master_pid = to;
        if let Some(row) = guild.members.get_mut(&to) {
            row.grade = GUILD_LEADER_GRADE;
        }
        if let Some(row) = guild.members.get_mut(&from) {
            row.grade = JOIN_GRADE;
        }
        let info = guild.info();
        let grades: [GcGuildGrade; GUILD_GRADE_COUNT] = std::array::from_fn(|index| {
            let (name, auth) = &guild.grades[index];
            GcGuildGrade {
                name: pad_grade_name(name),
                auth: *auth,
            }
        });
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_info_record(&info));
            let _sent = self.write_to_client(member, guild_grade_record(&grades));
        }
    }

    /// `guild_add_comment0` (`G/questlua_guild.cpp:538-546`, plus `AddComment`,
    /// `G/guild.cpp:1049-1072`): the wire post's judge without the length byte, the empty
    /// and overlong quiets, and the minute's Romanian countdown. The minute sets at the
    /// commit, after the insert — the descriptor drains the `Comment` store, reads the
    /// board, and sends `GuildCommentCommit::Post`, as the wire post's does.
    fn quest_add_comment(
        &self,
        vid: Vid,
        caller: Option<u32>,
        own: Option<u32>,
        body: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let quiet = GuildQuestJudged::silent(GuildAnswer::None);
        let (Some(speaker), Some(id)) = (caller, own) else {
            return quiet;
        };
        let Some(GuildArg::Text(body)) = body else {
            return quiet;
        };
        match self.answer_post_comment(vid, body) {
            GuildCommentAnswer::Store { content, .. } => GuildQuestJudged {
                answer: GuildAnswer::None,
                own: Vec::new(),
                stores: vec![GuildQuestStore::Comment {
                    guild_id: id,
                    pid: speaker,
                    content,
                }],
            },
            GuildCommentAnswer::Refused { line } => GuildQuestJudged {
                answer: GuildAnswer::None,
                own: line.into_iter().collect(),
                stores: Vec::new(),
            },
            GuildCommentAnswer::Done { .. } => quiet,
        }
    }

    /// `guild_set_ladder_point0` (`G/questlua_guild.cpp:548-556`, plus `ChangeLadderPoint`,
    /// `G/guild_war.cpp:753-758`): the delta lands on the ladder, saturating, and the
    /// judged ladder queues for the store. Without a guild, or without a number, nothing
    /// happens.
    fn quest_set_ladder(
        &mut self,
        _vid: Vid,
        own: Option<u32>,
        delta: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let quiet = GuildQuestJudged::silent(GuildAnswer::None);
        let (Some(id), Some(GuildArg::Number(delta))) = (own, delta) else {
            return quiet;
        };
        // `lua_tonumber` to `int`: the dword's low 32 bits, signed.
        let delta = i32::from_ne_bytes(delta.to_ne_bytes());
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return quiet;
        };
        guild.ladder_point = guild.ladder_point.saturating_add(delta);
        let ladder = guild.ladder_point;
        GuildQuestJudged {
            answer: GuildAnswer::None,
            own: Vec::new(),
            stores: vec![GuildQuestStore::Ladder {
                guild_id: id,
                ladder,
            }],
        }
    }

    /// `guild_set_skill_level0` (`G/questlua_guild.cpp:580-588`, plus `SetSkillLevel`,
    /// `G/guild.cpp:2251-2289`): the range gate, the proto gate, the max gate, the point
    /// debit, then the level lands, the points recompute, and the `SKILL_INFO` fans. The
    /// store half is the whole-row write. Without a guild, without numbers, or past a
    /// gate, nothing happens.
    fn quest_set_skill_level(
        &mut self,
        _vid: Vid,
        own: Option<u32>,
        vnum: Option<&quest::manager::GuildArg>,
        level: Option<&quest::manager::GuildArg>,
        point: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let quiet = GuildQuestJudged::silent(GuildAnswer::None);
        let (Some(id), Some(GuildArg::Number(vnum)), Some(GuildArg::Number(level))) =
            (own, vnum, level)
        else {
            return quiet;
        };
        let real = vnum.wrapping_sub(GUILD_SKILL_START);
        let index = usize::try_from(real).unwrap_or(GUILD_SKILL_COUNT);
        if index >= GUILD_SKILL_COUNT {
            return quiet;
        }
        let Some(row) = self.guild_skill_tables.get(*vnum) else {
            return quiet;
        };
        let max = row.record.max_level;
        // `BYTE level`: the dword's low 8 bits, not a saturating clamp.
        let level = level.to_le_bytes()[0];
        if level > max {
            return quiet;
        }
        let debit = match point {
            Some(GuildArg::Number(debit)) => debit.to_le_bytes()[0],
            _ => 0,
        };
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return quiet;
        };
        if debit != 0 {
            if guild.skill_point < debit {
                return quiet;
            }
            guild.skill_point -= debit;
        }
        guild.skills[index] = level;
        guild.max_power =
            GUILD_BASE_POWER + u32::from(guild.level.saturating_sub(1)) * GUILD_POWER_PER_LEVEL;
        guild.power = guild.power.min(guild.max_power);
        let skills = guild.skills;
        let skill_point = guild.skill_point;
        let info = guild.skill_info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_skill_info_record(&info));
        }
        GuildQuestJudged {
            answer: GuildAnswer::None,
            own: Vec::new(),
            stores: vec![GuildQuestStore::Skills {
                guild_id: id,
                skills,
                skill_point,
            }],
        }
    }

    /// `guild_set_skill_point0` (`G/questlua_guild.cpp:600-608`, plus `SetSkillPoint`,
    /// `G/guild.cpp:2299-2305`): the point lands and the `SKILL_INFO` fans. The store half
    /// is the whole-row write.
    fn quest_set_skill_point(
        &mut self,
        _vid: Vid,
        own: Option<u32>,
        point: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let quiet = GuildQuestJudged::silent(GuildAnswer::None);
        let (Some(id), Some(GuildArg::Number(point))) = (own, point) else {
            return quiet;
        };
        // `BYTE point`: the dword's low 8 bits, not a saturating clamp.
        let point = point.to_le_bytes()[0];
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return quiet;
        };
        guild.skill_point = point;
        let skills = guild.skills;
        let info = guild.skill_info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        for member in members {
            let _sent = self.write_to_client(member, guild_skill_info_record(&info));
        }
        GuildQuestJudged {
            answer: GuildAnswer::None,
            own: Vec::new(),
            stores: vec![GuildQuestStore::Skills {
                guild_id: id,
                skills,
                skill_point: point,
            }],
        }
    }

    /// `guild_offer_exp0` (`G/questlua_guild.cpp:616-649`, plus `OfferExp`,
    /// `G/guild.cpp:942-982`): the max-level refusal, the /100*100 floor, then the wire
    /// offer's judge — whose store halves the descriptor drains as a wire offer's: the
    /// `Offer` store, the `POINT_EXP` debit, and the commit's fans. Answers whether the
    /// offer landed.
    fn quest_offer_exp(
        &mut self,
        vid: Vid,
        caller: Option<u32>,
        own: Option<u32>,
        amount: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let refused = GuildQuestJudged::silent(GuildAnswer::Boolean(false));
        let (Some(caller), Some(id)) = (caller, own) else {
            return refused;
        };
        let Some(GuildArg::Number(amount)) = amount else {
            return refused;
        };
        if self
            .guilds
            .by_id
            .get(&id)
            .is_some_and(|guild| guild.level >= GUILD_MAX_LEVEL)
        {
            return refused;
        }
        // `offer /= 100; offer *= 100`: the floor, before the wire judge floors again
        // (idempotent) and debits the speaker's exp.
        match self.answer_guild_offer(vid, amount / 100 * 100) {
            GuildGradeAnswer::Store { set, .. } => {
                let GuildGradeSet::Offer {
                    floored,
                    offer,
                    exp,
                    level,
                    ladder,
                    ..
                } = set
                else {
                    return refused;
                };
                // The commit fans now, through the outboxes, as the wire offer's commit
                // does after the store.
                let _ = self.commit_guild_offer(id, caller, offer, exp, level, ladder);
                GuildQuestJudged {
                    answer: GuildAnswer::Boolean(true),
                    own: Vec::new(),
                    stores: vec![GuildQuestStore::Offer {
                        pid: caller,
                        floored,
                        offer,
                        guild_id: id,
                        exp,
                        level,
                        ladder,
                    }],
                }
            }
            GuildGradeAnswer::Refused { line } => GuildQuestJudged {
                answer: GuildAnswer::Boolean(false),
                own: line.into_iter().collect(),
                stores: Vec::new(),
            },
            GuildGradeAnswer::Committed { .. } => refused,
        }
    }

    /// `guild_give_exp0` (`G/questlua_guild.cpp:651-660`, plus `GuildPointChange`'s exp
    /// arm, `G/guild.cpp:1582-1662`): the /100 step runs the offer loop, the `CHANGE_EXP`
    /// and one INFO per level step fan, and the loop's whole answer queues for the
    /// store. No debit, no member row.
    fn quest_give_exp(
        &mut self,
        _vid: Vid,
        own: Option<u32>,
        amount: Option<&quest::manager::GuildArg>,
    ) -> GuildQuestJudged {
        use quest::manager::{GuildAnswer, GuildArg};
        let quiet = GuildQuestJudged::silent(GuildAnswer::None);
        let (Some(id), Some(GuildArg::Number(amount))) = (own, amount) else {
            return quiet;
        };
        let Some(guild) = self.guilds.by_id.get_mut(&id) else {
            return quiet;
        };
        let (exp, level, ladder) = {
            let step = amount / 100;
            let mut exp = guild.exp.saturating_add(step);
            let mut level = guild.level;
            let mut ladder = guild.ladder_point;
            let mut skill_point = guild.skill_point;
            while level < GUILD_MAX_LEVEL && exp >= GUILD_LEVELUP_EXP[usize::from(level)] {
                exp -= GUILD_LEVELUP_EXP[usize::from(level)];
                level += 1;
                skill_point = skill_point.saturating_add(1);
                ladder = ladder.saturating_add(GUILD_LADDER_POINT_PER_LEVEL);
            }
            if level == GUILD_MAX_LEVEL {
                exp = 0;
            }
            guild.exp = exp;
            guild.level = level;
            guild.ladder_point = ladder;
            guild.skill_point = skill_point;
            let max_power =
                GUILD_BASE_POWER + u32::from(level.saturating_sub(1)) * GUILD_POWER_PER_LEVEL;
            guild.max_power = max_power;
            guild.power = max_power;
            (exp, level, ladder)
        };
        let info = guild.info();
        let members: Vec<Vid> = guild
            .members
            .values()
            .filter_map(|member| member.vid)
            .collect();
        let steps = usize::from(level.saturating_sub(info.level));
        for member in members {
            let _sent = self.write_to_client(member, guild_change_exp_record(level, exp));
            for _ in 0..steps {
                let _sent = self.write_to_client(member, guild_info_record(&info));
            }
        }
        GuildQuestJudged {
            answer: GuildAnswer::None,
            own: Vec::new(),
            stores: vec![GuildQuestStore::Grant {
                guild_id: id,
                exp,
                level,
                ladder,
            }],
        }
    }
}

/// `/war`'s type word judged to a type: empty is field, otherwise `strtoul` with
/// anything at or past the max folded to field (`G/cmd_general.cpp:2060-2065`).
fn parse_war_type_or_field(word: &[u8]) -> u8 {
    if word.is_empty() {
        return GUILD_WAR_TYPE_FIELD;
    }
    let parsed = parse_war_type(word);
    if parsed >= u32::from(GUILD_WAR_TYPE_MAX_NUM) {
        GUILD_WAR_TYPE_FIELD
    } else {
        u8::try_from(parsed).unwrap_or(GUILD_WAR_TYPE_FIELD)
    }
}

/// `/war`'s type word as legacy's `strtoul` base 10 reads it (`str_to_number` on a
/// DWORD, `common/utils.h:44-50`): leading spaces skipped, an optional `+`/`-` taken
/// (a `-` negates into the unsigned wrap), digits read until the first non-digit,
/// garbage answering 0. The running answer is a `u32`, so the arithmetic wraps as
/// the C multiply does on every overflow; the judge folds anything past the max to
/// field. A word past the byte folds the same way below, so no clamp belongs here.
fn parse_war_type(word: &[u8]) -> u32 {
    let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[offset..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: u32 = 0;
    for byte in digits {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}
/// A `gwstop`/`gwcancel` id word as `str_to_number` on an `int` reads it
/// (`common/utils.h:38-43`): an empty word never reaches here (the caller guards —
/// `str_to_number` would leave the 0), else `strtol` base 10 cast to `int`. Leading
/// spaces skip, an optional sign takes, digits read until the first non-digit, and
/// the running answer wraps as the C multiply does — matching `parse_war_type`'s
/// wrap, signed.
fn parse_gm_guild_id(word: &[u8]) -> i32 {
    let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[offset..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    let mut value: i32 = 0;
    for byte in digits {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// `/war`'s type word as `strtol` base 10 reads it (`G/cmd_general.cpp:2058`): only the
/// sign matters — the FIX arm asks whether it is negative. A leading run of zeros
/// still signs it, as `strtol` returns `-0` is 0 — so a `-` followed only by zeros
/// answers non-negative, and only a nonzero digit past it answers negative.
fn parse_war_long(word: &[u8]) -> i64 {
    let Some(offset) = word.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    let rest = &word[offset..];
    let (digits, negative) = match rest.first() {
        Some(b'-') => (&rest[1..], true),
        Some(b'+') => (&rest[1..], false),
        _ => (rest, false),
    };
    if !negative {
        return 0;
    }
    if digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .any(|byte| *byte != b'0')
    {
        -1
    } else {
        0
    }
}

/// `RemoveSpecifyItem(GUILD_CREATE_ITEM_VNUM, 1)` (`CHARACTER::RemoveSpecifyItem`,
/// `G/char_item.cpp:8848-8900`): the first cell in slot order holding the vnum loses one;
/// a stack of one is destroyed (`SetCount(0)`, `G/item.cpp:296-344`), a deeper stack is
/// decremented. The scan covers `Inventory_Size()` cells (`:8859`) and then the custom
/// range (`:8902-8922`); the shop arms pass (no shops: nothing is on sale) and the gold-bar
/// log arm is dead (70101 is not 80003-80007). No quickslot sync is emitted: 70101 is
/// `ITEM_SPECIAL/SPECIAL_MAP` (`G/item.cpp:309`), which no slot can name.
fn remove_create_item(character: &mut Character) -> MoveDone {
    let usable = usable_inventory_cells(character.inven_point());
    let found = (0..usable)
        .chain(CUSTOM_INVENTORY_SLOT_START..CUSTOM_INVENTORY_SLOT_END)
        .find_map(|cell| {
            let pos = ItemPos::new(EWindows::Inventory as u8, cell);
            let item = character.items().item_at(pos)?;
            (item.vnum == GUILD_CREATE_ITEM_VNUM).then_some((item.id, item.count, pos))
        });
    let mut records = Vec::new();
    let mut changes = Vec::new();
    let Some((id, count, pos)) = found else {
        return MoveDone {
            kind: MoveKind::GuildMade,
            records,
            changes,
        };
    };
    if count <= 1 {
        if character.items_mut().release(id).is_ok() {
            records.push(MoveRecord::Item(ItemRecord::Set(gc_item_clear(pos))));
            changes.push(ItemChange::Destroyed { id });
        }
    } else if character
        .items_mut()
        .set_count(id, u32::from(count - 1))
        .is_ok()
    {
        if let Some(item) = character.items().item(id) {
            records.push(MoveRecord::Item(ItemRecord::Update(item.gc_item_update())));
        }
        changes.push(ItemChange::Count {
            id,
            count: count - 1,
        });
    }
    MoveDone {
        kind: MoveKind::GuildMade,
        records,
        changes,
    }
}

impl Guild {
    /// `GetMaxMemberCount` (`G/guild.cpp:1870-1878`): `32 + 2 * (level - 1) + bonus`,
    /// with the bonus clamped to 0-18. No infinite-members arm: `g_bGuildInfiniteMembers`
    /// is a test-server switch and the Rewrite has no test server.
    fn max_member_count(&self) -> u16 {
        let bonus = self.member_count_bonus.clamp(0, 18);
        let count = 32 + 2 * i32::from(self.level.saturating_sub(1)) + bonus;
        u16::try_from(count).unwrap_or(u16::MAX)
    }

    /// The auth of `pid`'s grade row (`HasGradeAuth`, `G/guild.h:242`), or `None` when
    /// the PID holds no row in this guild.
    fn grade_auth_of(&self, pid: u32) -> Option<u8> {
        let grade = self.members.get(&pid)?.grade;
        self.grades
            .get(usize::from(grade.saturating_sub(1)))
            .map(|(_, auth)| *auth)
    }

    /// The INFO body (`SendGuildInfoPacket`, `G/guild.cpp:910-939`). `has_land` is 0:
    /// `HasLand` asks the building manager (`:1990-1993`), which is not ported
    /// (`sys.world.objects`).
    fn info(&self) -> GcGuildInfo {
        GcGuildInfo {
            member_count: u16::try_from(self.members.len()).unwrap_or(u16::MAX),
            max_member_count: self.max_member_count(),
            guild_id: self.id,
            master_pid: self.master_pid,
            exp: self.exp,
            level: self.level,
            name: pad_guild_name(&self.name),
            gold: self.gold,
            has_land: 0,
        }
    }

    /// The LIST rows (`SendListPacket`, `G/guild.cpp:427-459`), in PID order
    /// (`m_member` is a `std::map`, `G/guild.h:383`).
    fn list(&self) -> Vec<GcGuildMember> {
        self.members
            .iter()
            .map(|(pid, member)| GcGuildMember {
                pid: *pid,
                grade: member.grade,
                is_general: member.is_general,
                job: member.job,
                level: member.level,
                offer: member.offer,
                name_flag: 1,
                name: pad_member_name(&member.name),
            })
            .collect()
    }

    /// The `SKILL_INFO` body (`SendSkillInfoPacket`, `G/guild.cpp:1419-1437`).
    fn skill_info(&self) -> GcGuildSkillInfo {
        GcGuildSkillInfo {
            skill_point: self.skill_point,
            skills: self.skills,
            power: u16::try_from(self.power).unwrap_or(u16::MAX),
            max_power: u16::try_from(self.max_power).unwrap_or(u16::MAX),
        }
    }
}

impl Guilds {
    /// Answer one `guild.*` quest call's pushed value from the shadow (230s): what
    /// [`GameState::answer_guild_quest`] would push for the same call on the live guilds.
    /// The Session calls this while the script waits, so the script's control flow sees
    /// the answer now; the world judges the same call again after the step for its
    /// records and stores.
    pub(super) fn judge_shadow(
        &self,
        caller: Option<u32>,
        name: &[u8],
        args: &[quest::manager::GuildArg],
    ) -> quest::manager::GuildAnswer {
        use quest::manager::{GuildAnswer, GuildArg};
        let number = |value: u32| GuildAnswer::Number(f64::from(value));
        let signed = |value: i32| GuildAnswer::Number(f64::from(value));
        let first = args.first();
        let own = caller.and_then(|pid| self.of.get(&pid).copied());
        let rank = |id: u32| {
            let point = self.by_id.get(&id).map_or(0, |guild| guild.ladder_point);
            let above = self
                .by_id
                .values()
                .filter(|guild| guild.ladder_point > point)
                .count();
            i32::try_from(above).unwrap_or(i32::MAX).saturating_add(1)
        };
        match name {
            b"get_rank" => signed(own.map_or(-1, rank)),
            b"get_ladder_point" => signed(
                own.and_then(|id| self.by_id.get(&id))
                    .map_or(-1, |guild| guild.ladder_point),
            ),
            b"high_ranking_string" => GuildAnswer::Text(self.shadow_high_rank(own, &rank)),
            b"around_ranking_string" => GuildAnswer::Text(self.shadow_around_rank(own, &rank)),
            b"name" | b"get_name" => {
                let Some(GuildArg::Number(id)) = first else {
                    return GuildAnswer::Text(Vec::new());
                };
                GuildAnswer::Text(
                    self.by_id
                        .get(id)
                        .map_or_else(Vec::new, |guild| guild.name.clone()),
                )
            }
            b"level" => {
                let Some(GuildArg::Number(id)) = first else {
                    return GuildAnswer::Number(0.0);
                };
                number(self.by_id.get(id).map_or(0, |guild| u32::from(guild.level)))
            }
            b"get_member_count" => {
                number(own.and_then(|id| self.by_id.get(&id)).map_or(0, |guild| {
                    u32::try_from(guild.members.len()).unwrap_or(u32::MAX)
                }))
            }
            b"get_id0" => number(own.unwrap_or(0)),
            b"get_sp0" => number(self.shadow_counter(own, |guild| guild.power)),
            b"get_maxsp0" => number(self.shadow_counter(own, |guild| guild.max_power)),
            b"get_money0" => number(self.shadow_counter(own, |guild| guild.gold)),
            b"get_max_member0" => number(u32::from(self.shadow_max_members(own))),
            b"get_total_member_level0" => number(self.shadow_total_level(own)),
            b"has_land0" => GuildAnswer::Boolean(false),
            b"get_win_count0" => signed(self.shadow_tally(own, |guild| guild.war_wins)),
            b"get_draw_count0" => signed(self.shadow_tally(own, |guild| guild.war_draws)),
            b"get_loss_count0" => signed(self.shadow_tally(own, |guild| guild.war_losses)),
            b"get_skill_level0" => {
                let vnum = match first {
                    Some(GuildArg::Number(vnum)) => *vnum,
                    _ => 0,
                };
                let real = vnum.wrapping_sub(GUILD_SKILL_START);
                let index = usize::try_from(real).unwrap_or(GUILD_SKILL_COUNT);
                if index >= GUILD_SKILL_COUNT {
                    return GuildAnswer::Number(0.0);
                }
                signed(
                    own.and_then(|id| self.by_id.get(&id))
                        .map_or(0, |guild| i32::from(guild.skills[index])),
                )
            }
            b"get_skill_point0" => number(
                own.and_then(|id| self.by_id.get(&id))
                    .map_or(0, |guild| u32::from(guild.skill_point)),
            ),
            b"get_exp_level0" => {
                let level = match first {
                    Some(GuildArg::Number(level)) => *level,
                    _ => 0,
                };
                let clamped = level.min(u32::from(GUILD_MAX_LEVEL));
                number(GUILD_LEVELUP_EXP[usize::try_from(clamped).unwrap_or(0)])
            }
            // Writers push nothing; their judges land after the step.
            _ => GuildAnswer::None,
        }
    }

    /// One unsigned counter of the caller's shadow guild, 0 when it holds none.
    fn shadow_counter(&self, own: Option<u32>, pick: impl Fn(&Guild) -> u32) -> u32 {
        own.and_then(|id| self.by_id.get(&id)).map_or(0, pick)
    }

    /// One signed tally of the caller's shadow guild, 0 when it holds none.
    fn shadow_tally(&self, own: Option<u32>, pick: impl Fn(&Guild) -> i32) -> i32 {
        own.and_then(|id| self.by_id.get(&id)).map_or(0, pick)
    }

    /// The max members of the caller's shadow guild, 0 when it holds none.
    fn shadow_max_members(&self, own: Option<u32>) -> u16 {
        own.and_then(|id| self.by_id.get(&id))
            .map_or(0, Guild::max_member_count)
    }

    /// The summed member levels of the caller's shadow guild, 0 when it holds none.
    fn shadow_total_level(&self, own: Option<u32>) -> u32 {
        own.and_then(|id| self.by_id.get(&id)).map_or(0, |guild| {
            guild
                .members
                .values()
                .map(|member| u32::from(member.level))
                .sum()
        })
    }

    /// The shadow's `FGuildCompare` order: ladder, wins, losses, name, ascending.
    fn shadow_ranked(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.by_id.keys().copied().collect();
        ids.sort_by(|left, right| {
            let (left, right) = (&self.by_id[left], &self.by_id[right]);
            (
                left.ladder_point,
                left.war_wins,
                left.war_losses,
                &left.name,
            )
                .cmp(&(
                    right.ladder_point,
                    right.war_wins,
                    right.war_losses,
                    &right.name,
                ))
        });
        ids
    }

    /// One shadow ranking line: `%3d | %-12s | %-8d | %4d | %4d | %4d`.
    fn shadow_rank_line(&self, id: u32, rank: &dyn Fn(u32) -> i32) -> Vec<u8> {
        let Some(guild) = self.by_id.get(&id) else {
            return Vec::new();
        };
        let mut name = guild.name.clone();
        name.resize(name.len().max(12), b' ');
        format!(
            "{:>3} | {} | {:<8} | {:>4} | {:>4} | {:>4}",
            rank(id),
            String::from_utf8_lossy(&name),
            guild.ladder_point,
            guild.war_wins,
            guild.war_draws,
            guild.war_losses,
        )
        .into_bytes()
    }

    /// The shadow's top eight with a positive ladder, the caller's own wrapped.
    fn shadow_high_rank(&self, own: Option<u32>, rank: &dyn Fn(u32) -> i32) -> Vec<u8> {
        let ranked = self.shadow_ranked();
        let mut out = Vec::new();
        let mut shown = 0;
        for id in ranked.iter().rev() {
            if shown >= 8 {
                break;
            }
            let Some(guild) = self.by_id.get(id) else {
                continue;
            };
            if guild.ladder_point <= 0 {
                break;
            }
            if Some(*id) == own {
                out.extend_from_slice(b"[COLOR r;255|g;255|b;0]");
            }
            if shown > 0 {
                out.extend_from_slice(b"[ENTER]");
            }
            out.extend_from_slice(&self.shadow_rank_line(*id, rank));
            if Some(*id) == own {
                out.extend_from_slice(b"[/COLOR]");
            }
            shown += 1;
        }
        out
    }

    /// The shadow's seven around the caller's own, the own wrapped.
    fn shadow_around_rank(&self, own: Option<u32>, rank: &dyn Fn(u32) -> i32) -> Vec<u8> {
        let Some(own) = own else {
            return Vec::new();
        };
        let ranked = self.shadow_ranked();
        let Some(position) = ranked.iter().position(|id| *id == own) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut shown = 0;
        for offset in -3..=3 {
            let Some(index) = position.checked_add_signed(offset) else {
                continue;
            };
            let Some(id) = ranked.get(index) else {
                continue;
            };
            if *id == own {
                out.extend_from_slice(b"[COLOR r;255|g;255|b;0]");
            }
            if shown > 0 {
                out.extend_from_slice(b"[ENTER]");
            }
            out.extend_from_slice(&self.shadow_rank_line(*id, rank));
            shown += 1;
            if *id == own {
                out.extend_from_slice(b"[/COLOR]");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc::UnboundedReceiver;

    use super::super::fixtures::{a_world, enter, TestClock};
    use super::*;
    use crate::game_loop::PulseProcessor;
    use gamedata::map_atlas::{MapAtlas, MapRegion};

    /// The master, at the origin.
    const MASTER_VID: u32 = 100;
    /// The guest, 100 east.
    const GUEST_VID: u32 = 101;
    /// A third player, far away.
    const THIRD_VID: u32 = 102;

    const HERE: (i32, i32) = (3200, 3200);
    const EAST: (i32, i32) = (3300, 3200);
    const FAR: (i32, i32) = (9000, 9000);

    const GUILD_ID: u32 = 7;

    /// `get_global_time()` in seconds, past every penalty: the quest flags default to 0,
    /// so `NOW - 0` clears the one-day withdraw and disband delays.
    const NOW: u32 = 10_000_000;

    fn a_guilded_pair() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let clock = TestClock::default();
        clock.set(1_000);
        let mut world = a_world(&clock);
        let master = enter(&mut world, MASTER_VID, HERE, 0);
        let guest = enter(&mut world, GUEST_VID, EAST, 0);
        let grades: Vec<db::guilds::StoredGrade> = (1..=15)
            .map(|grade| db::guilds::StoredGrade {
                guild_id: GUILD_ID,
                grade,
                name: if grade == 1 {
                    "Leader".to_owned()
                } else {
                    "...".to_owned()
                },
                auth: if grade == 1 { 15 } else { 0 },
            })
            .collect();
        world.install_guilds(
            &db::guilds::StoredGuilds {
                guilds: vec![db::guilds::StoredGuild {
                    id: GUILD_ID,
                    name: "Knights".to_owned(),
                    master_pid: MASTER_VID,
                    sp: 400,
                    level: 1,
                    exp: 0,
                    skill_point: 0,
                    skills: vec![0; GUILD_SKILL_COUNT],
                    ladder_point: 0,
                    win: 0,
                    draw: 0,
                    loss: 0,
                    gold: 0,
                }],
                grades,
                members: vec![db::guilds::StoredMember {
                    pid: MASTER_VID,
                    guild_id: GUILD_ID,
                    grade: 1,
                    is_general: false,
                    offer: 0,
                    level: 10,
                    job: 0,
                    name: format!("P{MASTER_VID}"),
                }],
            },
            &[],
            &[],
        );
        // `install_guilds` holds the row unlinked; the master's live enter ran before
        // the install, so the link lands here, as `enter_world`'s `link_guild` lands it
        // on a live enter after boot.
        world.link_guild(Vid::new(MASTER_VID));
        (clock, world, master, guest)
    }

    /// What `inbox` holds since the last look.
    fn heard(inbox: &mut UnboundedReceiver<Vec<u8>>) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| inbox.try_recv().ok()).collect()
    }

    /// The subheaders of `records`, in order.
    fn subheaders(records: &[Vec<u8>]) -> Vec<u8> {
        records.iter().map(|record| record[3]).collect()
    }

    /// Raise the fixture's guild to `level`: the install pins level 1, and the mark
    /// judge reads the live level.
    fn raise_guild(world: &mut GameState, level: u8) {
        world
            .guilds
            .by_id
            .get_mut(&GUILD_ID)
            .expect("installed")
            .level = level;
    }

    #[test]
    fn an_upload_saves_lists_and_fetches() {
        // The happy path: judged under the lowest id, the list names it, and the
        // diff answers the blocks the client's zero list lacks.
        let (_clock, mut world, _, _) = a_guilded_pair();
        raise_guild(&mut world, 3);
        let mut image = [0_u32; MARK_PIXELS];
        image[0] = 0x1122_3344;
        assert_eq!(
            world.judge_mark_upload(GUILD_ID, &image, 3),
            MarkUploadAnswer::Saved { mark_id: 0 }
        );
        assert_eq!(
            world.marks.idx_pairs(),
            vec![(u16::try_from(GUILD_ID).unwrap_or(u16::MAX), 0)]
        );
        let diff = world.marks.diff_blocks(0, &[0_u32; 80]);
        assert!(!diff.is_empty(), "the zero list lacks every live block");
        let held = world.marks.crc_list(0).expect("image 0 is live");
        let quiet = world.marks.diff_blocks(0, &held);
        assert!(quiet.is_empty(), "the held list matches every live block");
    }

    #[test]
    fn an_unknown_guild_or_a_short_level_is_quiet() {
        // Both refusals keep the manager empty and answer quiet, as the legacy
        // early returns log and send nothing.
        let (_clock, mut world, _, _) = a_guilded_pair();
        let mut image = [0_u32; MARK_PIXELS];
        image[0] = 1;
        assert_eq!(
            world.judge_mark_upload(4242, &image, 3),
            MarkUploadAnswer::Quiet
        );
        assert_eq!(
            world.judge_mark_upload(GUILD_ID, &image, 3),
            MarkUploadAnswer::Quiet,
            "the fixture sits at level 1"
        );
        assert!(world.marks.idx_pairs().is_empty());
    }

    #[test]
    fn a_zero_image_deletes_and_frees_the_id() {
        // `DeleteMark` zeroes the tile and returns the id to the free set, so the
        // next save reuses it; the symbol row goes with the mark.
        let (_clock, mut world, _, _) = a_guilded_pair();
        raise_guild(&mut world, 3);
        let mut image = [0_u32; MARK_PIXELS];
        image[0] = 1;
        assert_eq!(
            world.judge_mark_upload(GUILD_ID, &image, 3),
            MarkUploadAnswer::Saved { mark_id: 0 }
        );
        assert!(world.commit_symbol_upload(GUILD_ID, &[9; 16]));
        assert_eq!(
            world.judge_mark_upload(GUILD_ID, &[0_u32; MARK_PIXELS], 3),
            MarkUploadAnswer::Deleted
        );
        assert!(world.marks.idx_pairs().is_empty());
        assert!(!world.symbols.contains_key(&GUILD_ID));
        assert_eq!(
            world.judge_mark_upload(GUILD_ID, &image, 3),
            MarkUploadAnswer::Saved { mark_id: 0 },
            "the freed id is the lowest again"
        );
    }

    #[test]
    fn a_symbol_mismatch_serves_and_a_match_is_quiet() {
        // `GuildSymbolCRC` serves the bytes on a size or CRC miss and sends nothing
        // on a hit or an unknown guild; an upload to an unknown guild is refused.
        let (_clock, mut world, _, _) = a_guilded_pair();
        assert!(!world.commit_symbol_upload(4242, &[9; 16]));
        assert!(world.commit_symbol_upload(GUILD_ID, &[9; 16]));
        let crc = guild_mark::crc32(&[9_u8; 16]);
        assert_eq!(
            world.symbol_mismatch(GUILD_ID, crc, 16),
            None,
            "a hit sends nothing"
        );
        assert_eq!(
            world.symbol_mismatch(GUILD_ID, crc, 15).as_deref(),
            Some(&[9_u8; 16][..]),
            "a size miss serves"
        );
        assert_eq!(
            world.symbol_mismatch(GUILD_ID, crc ^ 1, 16).as_deref(),
            Some(&[9_u8; 16][..]),
            "a CRC miss serves"
        );
        assert_eq!(world.symbol_mismatch(4242, 0, 0), None);
    }

    #[test]
    fn the_boot_rows_seed_the_manager_and_the_map() {
        // `install_marks` records the ids, draws the tiles, and hashes the symbols;
        // rows for unknown guilds and short images are dropped.
        let (_clock, mut world, _, _) = a_guilded_pair();
        let mut image = vec![0_u8; MARK_PIXELS * 4];
        image[0] = 0x44;
        image[1] = 0x33;
        image[2] = 0x22;
        image[3] = 0x11;
        let marks = vec![
            db::guilds::StoredMark {
                guild_id: GUILD_ID,
                mark_id: 5,
                image: image.clone(),
            },
            db::guilds::StoredMark {
                guild_id: 4242,
                mark_id: 6,
                image: image.clone(),
            },
            db::guilds::StoredMark {
                guild_id: GUILD_ID,
                mark_id: 9000,
                image: vec![1_u8; 10],
            },
        ];
        let symbols = vec![
            db::guilds::StoredSymbol {
                guild_id: GUILD_ID,
                data: vec![7_u8; 16],
            },
            db::guilds::StoredSymbol {
                guild_id: 4242,
                data: vec![8_u8; 16],
            },
        ];
        world.install_marks(&marks, &symbols);
        assert_eq!(
            world.marks.idx_pairs(),
            vec![(u16::try_from(GUILD_ID).unwrap_or(u16::MAX), 5)]
        );
        let held = world.marks.crc_list(0).expect("image 0 drew the tile");
        assert!(
            world.marks.diff_blocks(0, &held).is_empty(),
            "the drawn tile compresses to the held list"
        );
        assert_eq!(
            world.symbols.get(&GUILD_ID).map(|symbol| &symbol.raw[..]),
            Some(&[7_u8; 16][..])
        );
        assert!(!world.symbols.contains_key(&4242));
    }

    #[test]
    fn an_invite_lands_and_lapses_quiet() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        assert!(heard(&mut master).is_empty());
        let invited = heard(&mut guest);
        assert_eq!(invited.len(), 1, "the guest hears the invite alone");
        assert_eq!(
            subheaders(&invited),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_GUILD_INVITE]
        );
        assert_eq!(
            u32::from_le_bytes(invited[0][4..8].try_into().unwrap()),
            GUILD_ID
        );
        let due = world.last_pulse.saturating_add(250);
        for pulse in 1..=due {
            world.process_pulse(pulse);
        }
        heard(&mut master);
        heard(&mut guest);
        // The lapsed invite answers nothing: the accept finds no row and stays quiet.
        let answer = world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW);
        assert_eq!(
            answer,
            GuildJoinAnswer::Refused {
                to: Vid::new(GUEST_VID),
                line: None
            }
        );
        assert!(heard(&mut guest).is_empty());
    }

    #[test]
    fn an_accept_joins_and_fans() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        heard(&mut guest);
        let answer = world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW);
        assert_eq!(answer, GuildJoinAnswer::Store { guild_id: GUILD_ID });
        let commit = GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        };
        heard(&mut master);
        let GuildJoinAnswer::Committed { own } = world.commit_guild_join(&commit) else {
            panic!("the commit fans");
        };
        assert_eq!(world.guild_of(GUEST_VID), Some(GUILD_ID));
        // The joiner's walk: grades, INFO, LIST, two LOGINs, skills.
        assert_eq!(own.len(), 3 + 2 + 1, "grades, INFO, LIST, LOGINs, skills");
        assert_eq!(
            subheaders(&own),
            [
                protocol::gc_guild::GUILD_SUBHEADER_GC_GRADE,
                protocol::gc_guild::GUILD_SUBHEADER_GC_INFO,
                protocol::gc_guild::GUILD_SUBHEADER_GC_LIST,
                protocol::gc_guild::GUILD_SUBHEADER_GC_LOGIN,
                protocol::gc_guild::GUILD_SUBHEADER_GC_LOGIN,
                protocol::gc_guild::GUILD_SUBHEADER_GC_SKILL_INFO,
            ]
        );
        // The fan through the outboxes: the LIST-one and the LOGIN to the
        // already-online master; the joiner hears no LIST-one of itself.
        let master_heard = heard(&mut master);
        assert_eq!(
            subheaders(&master_heard),
            [
                protocol::gc_guild::GUILD_SUBHEADER_GC_LIST,
                protocol::gc_guild::GUILD_SUBHEADER_GC_LOGIN,
            ]
        );
        assert!(heard(&mut guest).is_empty());
    }

    #[test]
    fn a_deny_removes_the_invite_quiet() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        heard(&mut guest);
        let answer = world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 0, NOW);
        assert_eq!(
            answer,
            GuildJoinAnswer::Refused {
                to: Vid::new(GUEST_VID),
                line: None
            }
        );
        // The deny removed the row: an accept after it stays quiet too.
        let answer = world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW);
        assert_eq!(
            answer,
            GuildJoinAnswer::Refused {
                to: Vid::new(GUEST_VID),
                line: None
            }
        );
        assert!(heard(&mut guest).is_empty());
    }

    #[test]
    fn an_offline_removal_fans_and_chats_forced_leave() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        heard(&mut guest);
        assert_eq!(
            world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        }) else {
            panic!("the join commits");
        };
        world.unlink_guild(Vid::new(GUEST_VID));
        heard(&mut master);
        let answer = world.answer_guild_remove(Vid::new(MASTER_VID), GUEST_VID, NOW);
        assert_eq!(
            answer,
            GuildRemoveAnswer::Store {
                pid: GUEST_VID,
                online: false,
                pre: None
            }
        );
        let commit = GuildRemoveCommit {
            speaker: Vid::new(MASTER_VID),
            pid: GUEST_VID,
            online: false,
        };
        let GuildRemoveAnswer::Committed { post } = world.commit_guild_remove(&commit) else {
            panic!("the removal commits");
        };
        assert_eq!(world.guild_of(GUEST_VID), None);
        let fanned = heard(&mut master);
        assert_eq!(fanned.len(), 1, "the master alone hears the REMOVE");
        assert_eq!(
            subheaders(&fanned),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_REMOVE]
        );
        assert_eq!(
            u32::from_le_bytes(fanned[0][4..8].try_into().unwrap()),
            GUEST_VID
        );
        assert!(
            post.is_some(),
            "the forced-leave line chats after the store"
        );
    }

    #[test]
    fn an_online_removal_without_auth_scolds_and_runs() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        heard(&mut guest);
        assert_eq!(
            world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        }) else {
            panic!("the join commits");
        };
        let mut third = enter(&mut world, THIRD_VID, FAR, 0);
        heard(&mut master);
        heard(&mut third);
        // The guest holds grade 15, whose auth is 0: removing the *guest* judges the
        // guest's own auth, so the master speaks and the judge scolds nothing. To take
        // the scold arm the guest removes the master — which the leader arm refuses
        // quiet — so instead the third player joins and removes the guest: its grade-15
        // row has no auth either.
        world.guild_add_member(Vid::new(MASTER_VID), THIRD_VID, NOW);
        assert_eq!(
            world.answer_guild_invite(Vid::new(THIRD_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: THIRD_VID,
        }) else {
            panic!("the third join commits");
        };
        let answer = world.answer_guild_remove(Vid::new(THIRD_VID), GUEST_VID, NOW);
        let GuildRemoveAnswer::Store { pid, online, pre } = answer else {
            panic!("the online scold judges Store, got the refusal");
        };
        assert_eq!(pid, GUEST_VID);
        assert!(online);
        assert!(pre.is_some(), "the scold chats before the store");
        heard(&mut master);
        heard(&mut guest);
        heard(&mut third);
        let GuildRemoveAnswer::Committed { post } = world.commit_guild_remove(&GuildRemoveCommit {
            speaker: Vid::new(THIRD_VID),
            pid: GUEST_VID,
            online: true,
        }) else {
            panic!("the removal commits");
        };
        assert_eq!(world.guild_of(GUEST_VID), None, "the scold runs anyway");
        assert_eq!(
            subheaders(&heard(&mut master)),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_REMOVE]
        );
        assert_eq!(
            subheaders(&heard(&mut third)),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_REMOVE]
        );
        assert!(heard(&mut guest).is_empty(), "the victim hears no REMOVE");
        assert!(post.is_none(), "the online arm chats nothing after");
    }

    #[test]
    fn the_leader_cannot_be_removed() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        heard(&mut master);
        let answer = world.answer_guild_remove(Vid::new(MASTER_VID), MASTER_VID, NOW);
        assert_eq!(answer, GuildRemoveAnswer::Refused { line: None });
        assert_eq!(world.guild_of(MASTER_VID), Some(GUILD_ID));
    }

    /// Judge, commit and drain: the guest joins at 15, then one grade-arm step runs
    /// end to end. Answers the commit and what the master heard.
    fn grade_step(
        world: &mut GameState,
        master: &mut UnboundedReceiver<Vec<u8>>,
        guest: &mut UnboundedReceiver<Vec<u8>>,
        judged: GuildGradeAnswer,
    ) -> (GuildGradeAnswer, Vec<Vec<u8>>) {
        let GuildGradeAnswer::Store { guild_id, set } = judged else {
            panic!("the arm judges Store, got the refusal");
        };
        assert_eq!(guild_id, GUILD_ID);
        heard(master);
        heard(guest);
        let committed = world.commit_guild_grade(&GuildGradeCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id: GUILD_ID,
            set,
        });
        (committed, heard(master))
    }

    /// A guild with the guest joined at 15, inboxes drained.
    fn a_joined_pair() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let (clock, mut world, mut master, mut guest) = a_guilded_pair();
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        assert_eq!(
            world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        }) else {
            panic!("the join commits");
        };
        heard(&mut master);
        heard(&mut guest);
        (clock, world, master, guest)
    }

    #[test]
    fn a_grade_is_renamed_and_the_fan_carries_it() {
        let (_clock, mut world, mut master, mut guest) = a_joined_pair();
        let field = b"Veteran\0\0";
        let judged = world.answer_grade_name(Vid::new(MASTER_VID), 15, field);
        assert_eq!(
            judged,
            GuildGradeAnswer::Store {
                guild_id: GUILD_ID,
                set: GuildGradeSet::GradeName {
                    grade: 15,
                    name: "Veteran".to_owned(),
                },
            }
        );
        let (committed, master_heard) = grade_step(&mut world, &mut master, &mut guest, judged);
        assert_eq!(committed, GuildGradeAnswer::Committed { post: None });
        assert_eq!(
            subheaders(&master_heard),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_GRADE_NAME]
        );
        assert_eq!(master_heard[0][4], 15);
        assert_eq!(&master_heard[0][5..12], b"Veteran");
        assert_eq!(
            subheaders(&heard(&mut guest)),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_GRADE_NAME]
        );
        assert_eq!(
            world.guilds.by_id[&GUILD_ID].grades[14].0, b"Veteran",
            "the name lands in memory"
        );
    }

    #[test]
    fn a_grade_rename_refuses_outside_the_leader() {
        let (_clock, world, mut master, _guest) = a_joined_pair();
        // The guest holds grade 15: not the leader.
        let answer = world.answer_grade_name(Vid::new(GUEST_VID), 15, b"Veteran\0\0");
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("a follower's rename is refused");
        };
        assert!(line.is_some(), "the leader gate chats");
        assert!(heard(&mut master).is_empty());
        // Grade 1 is untouchable even for the leader.
        let answer = world.answer_grade_name(Vid::new(MASTER_VID), 1, b"Bossman\0\0");
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("a grade-1 rename is refused");
        };
        assert!(line.is_some(), "the grade-1 gate chats");
        // A one-letter name fails the alphabet.
        let answer = world.answer_grade_name(Vid::new(MASTER_VID), 15, b"V\0\0\0\0\0\0\0\0");
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("a short name is refused");
        };
        assert!(line.is_some(), "the shape gate chats");
        // A NUL-less field names its first 8 bytes, as `strlcpy(..., 9)` reads them.
        let answer = world.answer_grade_name(Vid::new(MASTER_VID), 15, b"Veterans9");
        assert_eq!(
            answer,
            GuildGradeAnswer::Store {
                guild_id: GUILD_ID,
                set: GuildGradeSet::GradeName {
                    grade: 15,
                    name: "Veterans".to_owned(),
                },
            }
        );
        assert_eq!(
            world.guilds.by_id[&GUILD_ID].grades[14].0, b"...",
            "no refusal writes"
        );
    }

    #[test]
    fn a_grade_is_re_authed_and_followers_keep_quiet() {
        let (_clock, mut world, mut master, mut guest) = a_joined_pair();
        let judged = world.answer_grade_auth(Vid::new(MASTER_VID), 15, 7);
        assert_eq!(
            judged,
            GuildGradeAnswer::Store {
                guild_id: GUILD_ID,
                set: GuildGradeSet::GradeAuth { grade: 15, auth: 7 },
            }
        );
        let (committed, master_heard) = grade_step(&mut world, &mut master, &mut guest, judged);
        assert_eq!(committed, GuildGradeAnswer::Committed { post: None });
        assert_eq!(
            subheaders(&master_heard),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_GRADE_AUTH]
        );
        assert_eq!(master_heard[0][4], 15);
        assert_eq!(master_heard[0][5], 7);
        assert_eq!(world.guilds.by_id[&GUILD_ID].grades[14].1, 7);
        // Grade 1's auth is untouchable even for the leader.
        let answer = world.answer_grade_auth(Vid::new(MASTER_VID), 1, 0);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("a grade-1 re-auth is refused");
        };
        assert!(line.is_some(), "the grade-1 gate chats");
    }

    #[test]
    fn a_member_is_re_graded_and_the_own_row_refuses() {
        let (_clock, mut world, mut master, mut guest) = a_joined_pair();
        let judged = world.answer_member_grade(Vid::new(MASTER_VID), GUEST_VID, 5);
        assert_eq!(
            judged,
            GuildGradeAnswer::Store {
                guild_id: GUILD_ID,
                set: GuildGradeSet::MemberGrade {
                    pid: GUEST_VID,
                    grade: 5
                },
            }
        );
        let (committed, master_heard) = grade_step(&mut world, &mut master, &mut guest, judged);
        assert_eq!(committed, GuildGradeAnswer::Committed { post: None });
        assert_eq!(
            subheaders(&master_heard),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_CHANGE_MEMBER_GRADE]
        );
        assert_eq!(
            u32::from_le_bytes(master_heard[0][4..8].try_into().unwrap()),
            GUEST_VID
        );
        assert_eq!(master_heard[0][8], 5);
        assert_eq!(world.guilds.by_id[&GUILD_ID].members[&GUEST_VID].grade, 5);
        // The leader cannot re-grade its own row.
        let answer = world.answer_member_grade(Vid::new(MASTER_VID), MASTER_VID, 5);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("an own-row re-grade is refused");
        };
        assert!(line.is_some(), "the own-row gate chats");
        // Nobody can be raised to the leader grade.
        let answer = world.answer_member_grade(Vid::new(MASTER_VID), GUEST_VID, 1);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("a raise to 1 is refused");
        };
        assert!(line.is_some(), "the grade-1 gate chats");
    }

    #[test]
    fn a_general_is_flagged_and_the_second_is_refused() {
        let (_clock, mut world, mut master, mut guest) = a_joined_pair();
        let judged = world.answer_member_general(Vid::new(MASTER_VID), GUEST_VID, 9);
        assert_eq!(
            judged,
            GuildGradeAnswer::Store {
                guild_id: GUILD_ID,
                set: GuildGradeSet::MemberGeneral {
                    pid: GUEST_VID,
                    is_general: 1,
                },
            },
            "a nonzero byte judges to 1"
        );
        let (committed, master_heard) = grade_step(&mut world, &mut master, &mut guest, judged);
        assert_eq!(committed, GuildGradeAnswer::Committed { post: None });
        assert_eq!(
            subheaders(&master_heard),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_CHANGE_MEMBER_GENERAL]
        );
        assert_eq!(
            u32::from_le_bytes(master_heard[0][4..8].try_into().unwrap()),
            GUEST_VID
        );
        assert_eq!(master_heard[0][8], 1);
        // The house is full: a rival flag refuses at the judge.
        let mut third = enter(&mut world, THIRD_VID, FAR, 0);
        world.guild_add_member(Vid::new(MASTER_VID), THIRD_VID, NOW);
        assert_eq!(
            world.answer_guild_invite(Vid::new(THIRD_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: THIRD_VID,
        }) else {
            panic!("the third join commits");
        };
        heard(&mut third);
        let answer = world.answer_member_general(Vid::new(MASTER_VID), THIRD_VID, 1);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("the second general is refused");
        };
        assert!(line.is_some(), "the count gate chats");
        // And at the commit, when a rival flag landed between.
        let answer = world.commit_guild_grade(&GuildGradeCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id: GUILD_ID,
            set: GuildGradeSet::MemberGeneral {
                pid: THIRD_VID,
                is_general: 1,
            },
        });
        let GuildGradeAnswer::Committed { post } = answer else {
            panic!("the rival commit answers");
        };
        assert!(post.is_some(), "the rival commit chats the line");
        assert_eq!(
            world.guilds.by_id[&GUILD_ID].members[&THIRD_VID].is_general, 0,
            "the rival flag never lands"
        );
    }

    #[test]
    fn an_offer_floors_debits_and_fans() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        // The fixture's warrior holds no exp; bank some, as a kill would. Level 10
        // needs 33 000 for the next, so 50 000 banks 17 000 after the level-up — short
        // of the 20 400 offer. Bank 60 000: 27 000 rests, enough for the offer.
        world.gain_pc_exp(Vid::new(MASTER_VID), 60_000);
        heard(&mut master);
        let answer = world.answer_guild_offer(Vid::new(MASTER_VID), 20_450);
        let GuildGradeAnswer::Store { guild_id, set } = answer else {
            panic!("the offer judges");
        };
        assert_eq!(guild_id, GUILD_ID);
        let GuildGradeSet::Offer {
            pid,
            floored,
            offer,
            exp,
            level,
            ladder,
        } = set
        else {
            panic!("the offer stores");
        };
        assert_eq!(pid, MASTER_VID);
        assert_eq!(floored, 20_400, "the floor keeps the hundreds");
        assert_eq!(offer, 204, "the levelled delta is a hundredth");
        assert_eq!((exp, level, ladder), (204, 1, 0));
        let answer = world.commit_guild_grade(&GuildGradeCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id,
            set: GuildGradeSet::Offer {
                pid,
                floored,
                offer,
                exp,
                level,
                ladder,
            },
        });
        assert_eq!(answer, GuildGradeAnswer::Committed { post: None });
        let guild = &world.guilds.by_id[&GUILD_ID];
        assert_eq!(guild.exp, 204);
        assert_eq!(guild.members[&MASTER_VID].offer, 204);
        let master_heard = heard(&mut master);
        assert_eq!(
            master_heard.len(),
            3,
            "the POINT_EXP debit, the LIST-one, the CHANGE_EXP"
        );
        assert_eq!(
            master_heard[1][3],
            protocol::gc_guild::GUILD_SUBHEADER_GC_LIST
        );
        assert_eq!(master_heard[1].len(), 17);
        assert_eq!(
            master_heard[2][3],
            protocol::gc_guild::GUILD_SUBHEADER_GC_CHANGE_EXP
        );
        assert_eq!(master_heard[2][4], 1);
    }

    #[test]
    fn an_offer_short_of_exp_is_refused() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        let answer = world.answer_guild_offer(Vid::new(MASTER_VID), 10_000);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("the short offer is refused");
        };
        assert!(line.is_some(), "the [LS;929] arm chats");
        assert_eq!(world.guilds.by_id[&GUILD_ID].exp, 0);
    }

    #[test]
    fn an_offer_at_the_top_is_refused() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        world
            .guilds
            .by_id
            .get_mut(&GUILD_ID)
            .expect("the guild")
            .level = 20;
        let answer = world.answer_guild_offer(Vid::new(MASTER_VID), 10_000);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("the top-level offer is refused");
        };
        assert!(line.is_some(), "the max-level gate chats");
    }

    #[test]
    fn a_charge_caps_debits_and_fans() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        // The fixture enters broke; fund the charge, as a sale would.
        if let Ok(character) = world.characters.find_by_vid_mut(Vid::new(MASTER_VID)) {
            character.set_gold(50_000);
        }
        world
            .guilds
            .by_id
            .get_mut(&GUILD_ID)
            .expect("the guild")
            .power = 350;
        let answer = world.answer_guild_charge(Vid::new(MASTER_VID), 200);
        let GuildGradeAnswer::Store { guild_id, set } = answer else {
            panic!("the charge judges");
        };
        let GuildGradeSet::Charge {
            pid,
            debit,
            power,
            gained,
        } = set
        else {
            panic!("the charge stores");
        };
        assert_eq!(pid, MASTER_VID);
        assert_eq!(debit, 5_000, "the cap pays the headroom only");
        assert_eq!(power, 400);
        assert_eq!(gained, 50, "the count chats the capped SP");
        let answer = world.commit_guild_grade(&GuildGradeCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id,
            set: GuildGradeSet::Charge {
                pid,
                debit,
                power,
                gained,
            },
        });
        let GuildGradeAnswer::Committed { post } = answer else {
            panic!("the charge commits");
        };
        assert!(post.is_some(), "the [LS;938] count answers");
        assert_eq!(world.guilds.by_id[&GUILD_ID].power, 400);
        let master_heard = heard(&mut master);
        assert!(
            master_heard
                .iter()
                .any(|record| record[3] == protocol::gc_guild::GUILD_SUBHEADER_GC_SKILL_INFO),
            "the SKILL_INFO fans"
        );
    }

    #[test]
    fn a_post_inside_the_minute_chats_the_countdown() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        // A post sets the minute at depth 1500; run 25 Pulses in, so 1475 rest — 59
        // seconds — and the countdown chats 00 minutes and 59 seconds.
        let answer = world.answer_post_comment(Vid::new(MASTER_VID), b"hello all");
        let GuildCommentAnswer::Store {
            guild_id,
            pid,
            content,
        } = answer
        else {
            panic!("the first post judges");
        };
        assert_eq!(guild_id, GUILD_ID);
        assert_eq!(pid, MASTER_VID);
        assert_eq!(content, b"hello all");
        let answer = world.commit_post_comment(&GuildCommentCommit::Post {
            speaker: Vid::new(MASTER_VID),
            guild_id,
            board: vec![GuildCommentLine {
                id: 1,
                name: b"Master".to_vec(),
                content: b"hello all".to_vec(),
            }],
        });
        let GuildCommentAnswer::Done { board, line } = answer else {
            panic!("the post commits");
        };
        assert!(line.is_none());
        assert_eq!(board.expect("the board").len(), 1);
        world.run_guild_ticks(25);
        // `run_guild_ticks` lapses invites; step the pulse by hand, as the loop does.
        world.last_pulse = 25;
        let answer = world.answer_post_comment(Vid::new(MASTER_VID), b"again");
        let GuildCommentAnswer::Refused { line } = answer else {
            panic!("the second post is refused");
        };
        let line = line.expect("the countdown chats");
        // The countdown's text rides a GC_CHAT line; the minutes and seconds print
        // zero-padded, as `%02d` prints them: 1475 Pulses rest, which floor to 59
        // seconds, so the line ends `00 minute si 59 secunde!`.
        let text = line
            .windows(24)
            .any(|window| window == b"00 minute si 59 secunde!");
        assert!(text, "the countdown rests: {line:?}");
        heard(&mut master);
    }

    #[test]
    fn a_notice_without_auth_is_refused() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        // The guest holds no row: join it at grade 15, whose auth is 0.
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        assert_eq!(
            world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW),
            GuildJoinAnswer::Store { guild_id: GUILD_ID }
        );
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        }) else {
            panic!("the guest joins");
        };
        let answer = world.answer_post_comment(Vid::new(GUEST_VID), b"!war at nine");
        let GuildCommentAnswer::Refused { line } = answer else {
            panic!("the notice is refused");
        };
        assert!(line.is_some(), "the permission line chats");
        // The leader's notice judges: grade 1 holds the notice bit.
        let answer = world.answer_post_comment(Vid::new(MASTER_VID), b"!war at nine");
        assert!(
            matches!(answer, GuildCommentAnswer::Store { .. }),
            "the leader's notice judges"
        );
    }

    #[test]
    fn a_charge_short_of_gold_is_refused() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        let answer = world.answer_guild_charge(Vid::new(MASTER_VID), 10);
        let GuildGradeAnswer::Refused { line } = answer else {
            panic!("the short charge is refused");
        };
        assert!(line.is_some(), "the insufficient arm chats");
        assert_eq!(world.guilds.by_id[&GUILD_ID].power, 400);
    }

    /// The hosted guild rows, from the owner's dump.
    fn guild_dumps() -> std::sync::Arc<gamedata::guild_skill_data::GuildSkills> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/sql/gamedata");
        let player = std::fs::read(root.join("player.sql")).unwrap();
        std::sync::Arc::new(gamedata::guild_skill_data::GuildSkills::from_dump(&player).unwrap())
    }

    /// The pair with the guild rows shared and one skill point held.
    fn a_skilling_pair() -> (
        TestClock,
        GameState,
        UnboundedReceiver<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let (clock, mut world, master, guest) = a_guilded_pair();
        world = world.with_guild_skills(guild_dumps());
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skill_point = 1;
        // `link_guild`'s self packets already left; the tests below read only what
        // their own step sends.
        (clock, world, master, guest)
    }

    /// The `SKILL_INFO` body of `record`: point, levels, power, max power.
    fn skill_info_of(record: &[u8]) -> (u8, Vec<u8>, u16, u16) {
        use protocol::gc_guild::{GC_GUILD_ENVELOPE_WIRE_SIZE, GUILD_SUBHEADER_GC_SKILL_INFO};
        assert_eq!(
            record[3], GUILD_SUBHEADER_GC_SKILL_INFO,
            "the SKILL_INFO subheader"
        );
        let body = GC_GUILD_ENVELOPE_WIRE_SIZE;
        let point = record[body];
        let levels = record[body + 1..body + 1 + GUILD_SKILL_COUNT].to_vec();
        let power = u16::from_le_bytes(record[body + 13..body + 15].try_into().unwrap());
        let max = u16::from_le_bytes(record[body + 15..body + 17].try_into().unwrap());
        (point, levels, power, max)
    }

    #[test]
    fn a_guild_skill_up_climbs_and_fans() {
        let (_clock, mut world, mut master, _guest) = a_skilling_pair();
        heard(&mut master);
        let answer = world.guild_skill_up(Vid::new(MASTER_VID), b"152");
        let Some(climbed) = answer.climbed else {
            panic!("the up lands");
        };
        assert_eq!(
            (climbed.guild_id, climbed.vnum, climbed.level, climbed.point),
            (GUILD_ID, 152, 1, 0)
        );
        assert_eq!(climbed.skills[1], 1, "152 is the second byte");
        let fanned = heard(&mut master);
        assert_eq!(fanned.len(), 1, "one SKILL_INFO");
        assert_eq!(
            skill_info_of(&fanned[0]),
            (0, climbed.skills.to_vec(), 400, 400)
        );
    }

    #[test]
    fn a_guild_skill_up_refuses_every_gate() {
        let (_clock, mut world, mut master, _guest) = a_skilling_pair();
        heard(&mut master);
        // Empty: quiet.
        assert!(world
            .guild_skill_up(Vid::new(MASTER_VID), b"")
            .climbed
            .is_none());
        assert!(heard(&mut master).is_empty(), "the empty up is quiet");
        // Out of range: quiet.
        assert!(world
            .guild_skill_up(Vid::new(MASTER_VID), b"163")
            .climbed
            .is_none());
        assert!(world
            .guild_skill_up(Vid::new(MASTER_VID), b"150")
            .climbed
            .is_none());
        assert!(heard(&mut master).is_empty(), "the ranged-out up is quiet");
        // At max: quiet. 158 climbs to 3.
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skills[7] = 3;
        assert!(world
            .guild_skill_up(Vid::new(MASTER_VID), b"158")
            .climbed
            .is_none());
        assert!(heard(&mut master).is_empty(), "the maxed up is quiet");
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skills[7] = 0;
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skill_point = 0;
        // No point: quiet.
        assert!(world
            .guild_skill_up(Vid::new(MASTER_VID), b"152")
            .climbed
            .is_none());
        assert!(heard(&mut master).is_empty(), "the pointless up is quiet");
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skill_point = 1;
        // Non-leader: 524. Join the guest at grade 15 first.
        world.guild_add_member(Vid::new(MASTER_VID), GUEST_VID, NOW);
        let answer = world.answer_guild_invite(Vid::new(GUEST_VID), GUILD_ID, 1, NOW);
        let GuildJoinAnswer::Store { guild_id } = answer else {
            panic!("the invite judges");
        };
        assert_eq!(guild_id, GUILD_ID);
        let GuildJoinAnswer::Committed { .. } = world.commit_guild_join(&GuildJoinCommit {
            guild_id: GUILD_ID,
            pid: GUEST_VID,
        }) else {
            panic!("the guest joins");
        };
        let answer = world.guild_skill_up(Vid::new(GUEST_VID), b"152");
        assert!(answer.climbed.is_none(), "the guest climbs nothing");
        // Guildless: 523.
        let answer = world.guild_skill_up(Vid::new(THIRD_VID), b"152");
        assert!(answer.climbed.is_none(), "the guildless climbs nothing");
    }

    #[test]
    fn a_guild_cast_consumes_and_chats_937() {
        let (_clock, mut world, mut master, _guest) = a_skilling_pair();
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skills[1] = 1;
        heard(&mut master);
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 152, 0);
        let GuildSkillUseAnswer::Store {
            guild_id,
            vnum,
            power,
            ..
        } = answer
        else {
            panic!("the cast judges: {answer:?}");
        };
        assert_eq!(guild_id, GUILD_ID);
        assert_eq!(vnum, 152);
        // `150 + 150 * (1/7) = 171.4` truncates to 171; 400 - 171 = 229.
        assert_eq!(power, 229);
        let answer = world.commit_guild_use_skill(&GuildSkillUseCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id,
            vnum,
            pid: 0,
            power,
        });
        assert_eq!(answer, GuildSkillUseAnswer::Committed);
        assert_eq!(
            world.guilds.by_id[&GUILD_ID].power, 229,
            "the SP stays consumed"
        );
        let fanned = heard(&mut master);
        assert_eq!(fanned.len(), 2, "the SKILL_INFO and the 937 line");
        assert_eq!(skill_info_of(&fanned[0]).2, 229);
        assert!(
            fanned[1]
                .windows(b"[LS;937]".len())
                .any(|w| w == b"[LS;937]"),
            "the war gate chats 937"
        );
        // Still unusable: a second cast on the same Pulse answers 932.
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 152, 0);
        let GuildSkillUseAnswer::Refused { line } = answer else {
            panic!("the cooling cast refuses");
        };
        assert!(line.is_some(), "932 chats");
    }

    #[test]
    fn a_guild_cast_refuses_before_the_cost() {
        let (_clock, mut world, mut master, _guest) = a_skilling_pair();
        heard(&mut master);
        // Level 0: quiet.
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 152, 0);
        assert_eq!(answer, GuildSkillUseAnswer::Refused { line: None });
        assert!(heard(&mut master).is_empty());
        // Out of range: quiet.
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 163, 0);
        assert_eq!(answer, GuildSkillUseAnswer::Refused { line: None });
        // Guildless: 523.
        let answer = world.answer_guild_use_skill(Vid::new(GUEST_VID), 152, 0);
        let GuildSkillUseAnswer::Refused { line } = answer else {
            panic!("the guildless cast refuses");
        };
        assert!(line.is_some(), "523 chats");
        // Underfunded: 931. Level 1 of 152 costs 171; hold 170.
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skills[1] = 1;
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().power = 170;
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 152, 0);
        let GuildSkillUseAnswer::Refused { line } = answer else {
            panic!("the poor cast refuses");
        };
        let line = line.expect("931 chats");
        assert!(
            line.windows(b"[LS;931;170;171]".len())
                .any(|w| w == b"[LS;931;170;171]"),
            "931 carries (power, needed)"
        );
    }

    #[test]
    fn a_guild_teleport_orders_runs_and_rearms() {
        let (clock, mut world, mut master, _guest) = a_skilling_pair();
        world.guilds.by_id.get_mut(&GUILD_ID).unwrap().skills[7] = 1;
        heard(&mut master);
        // Offline PID: 935 chats at commit, nothing judged against the registry.
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 158, 999_999);
        let GuildSkillUseAnswer::Store { power, .. } = answer else {
            panic!("the teleport judges");
        };
        assert_eq!(power, 400, "the empty cost consumes nothing");
        let answer = world.commit_guild_use_skill(&GuildSkillUseCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id: GUILD_ID,
            vnum: 158,
            pid: 999_999,
            power,
        });
        assert_eq!(answer, GuildSkillUseAnswer::Committed);
        let fanned = heard(&mut master);
        assert_eq!(fanned.len(), 2, "the SKILL_INFO and the 935 line");
        assert!(
            fanned[1]
                .windows(b"[LS;935]".len())
                .any(|w| w == b"[LS;935]"),
            "the offline victim chats 935"
        );
        // The 0-second cooldown stamps `due = now` and re-arms on the next tick.
        assert!(!world.guilds.by_id[&GUILD_ID].usable[7]);
        let pulse = world.last_pulse();
        world.run_guild_ticks(pulse);
        assert!(
            world.guilds.by_id[&GUILD_ID].usable[7],
            "a 0 cooldown re-arms next tick"
        );
        // Same-process victim on the same Channel: the warp orders. The registry is
        // empty in the fixture, so the order warns and drops — the branch still ran
        // past the 934/935 chats.
        clock.set(1_001);
        let answer = world.answer_guild_use_skill(Vid::new(MASTER_VID), 158, GUEST_VID);
        let GuildSkillUseAnswer::Store { power, .. } = answer else {
            panic!("the re-armed cast judges");
        };
        let answer = world.commit_guild_use_skill(&GuildSkillUseCommit {
            speaker: Vid::new(MASTER_VID),
            guild_id: GUILD_ID,
            vnum: 158,
            pid: GUEST_VID,
            power,
        });
        assert_eq!(answer, GuildSkillUseAnswer::Committed);
        let fanned = heard(&mut master);
        assert_eq!(fanned.len(), 1, "the warp branch chats nothing");
    }

    /// The quest manager the `with_limit` judge reads and stamps through, built like
    /// the fight tests'.
    fn quests() -> super::super::quests::Quests {
        use gamedata::mob_locale_names::MobNamesByLanguage;
        let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe");
        let manager =
            quest::manager::Manager::load(&locale_dir).unwrap_or_else(|error| panic!("{error}"));
        super::super::quests::Quests::new(
            manager,
            std::sync::Arc::default(),
            MobNamesByLanguage::default(),
        )
    }

    /// One `with_limit` call against the joined pair, dwording numbers like the bridge.
    fn with_limit(
        world: &mut GameState,
        name: &[u8],
        floor: f64,
        resign: f64,
        other_leader: f64,
        other_member: f64,
        ignore: Option<quest::manager::GuildArg>,
    ) -> quest::manager::GuildAnswer {
        use quest::manager::GuildArg;
        let dword = |value: f64| {
            let truncated = value.trunc();
            let wide: i64 = format!("{truncated:.0}").parse().unwrap_or(0);
            u32::from_le_bytes(wide.to_le_bytes()[..4].try_into().unwrap())
        };
        let args = [
            GuildArg::Text(name.to_vec()),
            GuildArg::Number(dword(floor)),
            GuildArg::Number(dword(resign)),
            GuildArg::Number(dword(other_leader)),
            GuildArg::Number(dword(other_member)),
        ];
        let mut full = args.to_vec();
        if let Some(ignore) = ignore {
            full.push(ignore);
        }
        world
            .answer_guild_quest(
                Vid::new(MASTER_VID),
                b"change_master_with_limit",
                &full,
                NOW,
            )
            .answer
    }

    /// A joined pair with quests loaded: the guest is a grade-15 member named `P101`.
    fn a_limit_pair() -> (TestClock, GameState) {
        let (clock, mut world, mut master, mut guest) = a_joined_pair();
        world.quests = Some(quests());
        heard(&mut master);
        heard(&mut guest);
        (clock, world)
    }

    #[test]
    fn a_limit_change_lands_and_stamps_six() {
        use quest::manager::GuildAnswer;
        let (_clock, mut world) = a_limit_pair();
        let answer = with_limit(&mut world, b"P101", 1.0, 100.0, 200.0, 300.0, None);
        assert_eq!(answer, GuildAnswer::Number(3.0));
        assert_eq!(world.guilds.by_id[&GUILD_ID].master_pid, GUEST_VID);
        let flags = |pid: u32, name: &[u8]| {
            world
                .quests
                .as_ref()
                .expect("quests")
                .manager()
                .quest_flag(pid, name)
        };
        assert_eq!(
            flags(GUEST_VID, b"change_guild_master.be_other_leader"),
            Some(0)
        );
        assert_eq!(
            flags(GUEST_VID, b"change_guild_master.be_other_member"),
            Some(0)
        );
        assert_eq!(
            flags(GUEST_VID, b"change_guild_master.resign_limit"),
            Some(100)
        );
        assert_eq!(
            flags(MASTER_VID, b"change_guild_master.be_other_leader"),
            Some(200)
        );
        assert_eq!(
            flags(MASTER_VID, b"change_guild_master.be_other_member"),
            Some(300)
        );
        assert_eq!(
            flags(MASTER_VID, b"change_guild_master.resign_limit"),
            Some(0)
        );
    }

    #[test]
    fn a_limit_change_judges_each_gate() {
        use quest::manager::{GuildAnswer, GuildArg};
        let (_clock, mut world) = a_limit_pair();
        // Not the master: 1.
        let answer = world
            .answer_guild_quest(
                Vid::new(GUEST_VID),
                b"change_master_with_limit",
                &[
                    GuildArg::Text(b"P100".to_vec()),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                ],
                NOW,
            )
            .answer;
        assert_eq!(answer, GuildAnswer::Number(1.0));
        // A number name: 0.
        let answer = world
            .answer_guild_quest(
                Vid::new(MASTER_VID),
                b"change_master_with_limit",
                &[
                    GuildArg::Number(7),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                    GuildArg::Number(0),
                ],
                NOW,
            )
            .answer;
        assert_eq!(answer, GuildAnswer::Number(0.0));
        // Offline name: 5.
        let answer = with_limit(&mut world, b"Nobody", 0.0, 0.0, 0.0, 0.0, None);
        assert_eq!(answer, GuildAnswer::Number(5.0));
        // Under the floor: 6. The fixture enters at level 10.
        let answer = with_limit(&mut world, b"P101", 11.0, 0.0, 0.0, 0.0, None);
        assert_eq!(answer, GuildAnswer::Number(6.0));
        // The new master holds a live `be_other_leader`: 7.
        world
            .quests
            .as_mut()
            .expect("quests")
            .manager_mut()
            .set_quest_flag(
                GUEST_VID,
                b"change_guild_master.be_other_leader",
                i32::try_from(NOW + 1).unwrap(),
            );
        let answer = with_limit(&mut world, b"P101", 1.0, 0.0, 0.0, 0.0, None);
        assert_eq!(answer, GuildAnswer::Number(7.0));
        // The ignore flag clears the 7.
        let answer = with_limit(
            &mut world,
            b"P101",
            1.0,
            0.0,
            0.0,
            0.0,
            Some(GuildArg::Number(1)),
        );
        assert_eq!(answer, GuildAnswer::Number(3.0));
        // An online non-member past the gates: 2. The third player stands but never
        // joined.
        let (_clock, mut world) = a_limit_pair();
        let _third = enter(&mut world, THIRD_VID, FAR, 0);
        let answer = with_limit(&mut world, b"P102", 1.0, 0.0, 0.0, 0.0, None);
        assert_eq!(answer, GuildAnswer::Number(2.0));
    }

    /// `view_guild` names the master 3, a general 2, a plain member 1, and the guildless
    /// 0 (`G/char.cpp:1180-1199`).
    #[test]
    fn view_guild_names_master_general_member_and_none() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        assert_eq!(world.view_guild(MASTER_VID), (GUILD_ID, 3), "the master");
        assert_eq!(world.view_guild(GUEST_VID), (0, 0), "the guildless guest");
        assert_eq!(world.view_guild_of_vid(MASTER_VID), (GUILD_ID, 3), "by VID");
        assert_eq!(world.view_guild_of_vid(999), (0, 0), "none online");
        let guild = world.guilds.by_id.get_mut(&GUILD_ID).expect("the guild");
        guild.members.insert(
            GUEST_VID,
            GuildMember {
                grade: 2,
                is_general: 1,
                job: 0,
                level: 10,
                offer: 0,
                name: b"P101".to_vec(),
                vid: None,
            },
        );
        world.guilds.of.insert(GUEST_VID, GUILD_ID);
        assert_eq!(world.view_guild(GUEST_VID), (GUILD_ID, 2), "a general");
        let guild = world.guilds.by_id.get_mut(&GUILD_ID).expect("the guild");
        guild
            .members
            .get_mut(&GUEST_VID)
            .expect("the guest")
            .is_general = 0;
        assert_eq!(world.view_guild(GUEST_VID), (GUILD_ID, 1), "a plain member");
    }

    /// The master's spawn info carries its guild id and 3, and the viewer hears the NAME
    /// once (`G/char.cpp:1070`, `:1176-1199`): a second spawn of the same guild sends no
    /// NAME, while the info still names the id.
    #[test]
    fn a_viewer_hears_a_guilds_name_once_and_sees_its_id() {
        use super::super::fixtures::enter_seeing;
        use super::super::view::{Effect, EntityKey};
        let (clock, mut world, mut master, mut guest) = a_guilded_pair();
        heard(&mut master);
        heard(&mut guest);
        clock.set(2_000);
        let (_third, shown) = enter_seeing(&mut world, (1, 41), THIRD_VID, EAST, 0);
        let named: Vec<&Vec<u8>> = shown
            .iter()
            .filter(|record| record[0] == protocol::gc_guild::HEADER_GC_GUILD)
            .collect();
        assert_eq!(
            named.len(),
            1,
            "one NAME, before the master's add: {shown:?}"
        );
        assert_eq!(&named[0][4..8], &GUILD_ID.to_le_bytes(), "the guild id");
        assert_eq!(&named[0][8..15], b"Knights", "the guild name");
        let info = shown
            .iter()
            .find(|record| {
                record[0] == 136 && record.len() == 70 && record[1..5] == MASTER_VID.to_le_bytes()
            })
            .expect("the master's info");
        assert_eq!(&info[43..47], &GUILD_ID.to_le_bytes(), "dwGuildID");
        assert_eq!(info[63], 3, "dwNewIsGuildName names the master");
        // The guest's spawn re-sends the info but not the name: the viewer knows the id.
        world.guilds.of.insert(GUEST_VID, GUILD_ID);
        world
            .guilds
            .by_id
            .get_mut(&GUILD_ID)
            .expect("the guild")
            .members
            .insert(
                GUEST_VID,
                GuildMember {
                    grade: 2,
                    is_general: 0,
                    job: 0,
                    level: 10,
                    offer: 0,
                    name: b"P101".to_vec(),
                    vid: Some(Vid::new(GUEST_VID)),
                },
            );
        let effects = [Effect::Insert {
            of: EntityKey::Character(GUEST_VID),
            to: EntityKey::Character(THIRD_VID),
        }];
        let mut own = Vec::new();
        world.deliver(&effects, Some((THIRD_VID, &mut own)));
        assert!(
            own.iter()
                .all(|record| record[0] != protocol::gc_guild::HEADER_GC_GUILD),
            "no second NAME: {own:?}"
        );
        let info = own
            .iter()
            .find(|record| record[0] == 136 && record.len() == 70)
            .expect("the guest's info");
        assert_eq!(&info[43..47], &GUILD_ID.to_le_bytes(), "dwGuildID");
        assert_eq!(info[63], 1, "dwNewIsGuildName names a plain member");
    }

    /// An equip's update carries the master's guild id and 3 (`UpdatePacket`,
    /// `G/char.cpp:1327-1352`).
    #[test]
    fn an_update_carries_the_guild_id_and_master_flag() {
        let (_clock, world, _master, _guest) = a_guilded_pair();
        let frame = world
            .update_frame(Vid::new(MASTER_VID), [0; 2])
            .expect("the master is online");
        assert_eq!(frame.len(), 55, "the update's packed width");
        assert_eq!(&frame[28..32], &GUILD_ID.to_le_bytes(), "dwGuildID");
        assert_eq!(frame[48], 3, "dwNewIsGuildName names the master");
    }
    /// A second guild that can start a war: 8 member rows (the master by pid
    /// `SECOND_ID`, which the test enters and links), a positive ladder, and the
    /// leader grade.
    const SECOND_ID: u32 = 9;

    /// Install the second guild beside the fixture's, whose ladder and rows pass
    /// `CanStartWar`; the fixture's own guild gains the ladder and rows too.
    fn install_war_pair(world: &mut GameState) {
        let grades_for = |id: u32| {
            (1..=15)
                .map(|grade| db::guilds::StoredGrade {
                    guild_id: id,
                    grade,
                    name: if grade == 1 {
                        "Leader".to_owned()
                    } else {
                        "...".to_owned()
                    },
                    auth: if grade == 1 { 15 } else { 0 },
                })
                .collect::<Vec<_>>()
        };
        let mut grades = grades_for(SECOND_ID);
        grades.extend(grades_for(GUILD_ID));
        // The guest (vid 101) stays guildless for the gates test, so the rows skip
        // it; the seat past 7 keeps the eight `CanStartWar` needs.
        let members_for = |id: u32, master: u32| {
            (0..9)
                .filter(move |seat| {
                    *seat < 8 && master + seat != GUEST_VID || *seat == 8 && id == GUILD_ID
                })
                .map(|seat| db::guilds::StoredMember {
                    pid: master + seat,
                    guild_id: id,
                    grade: 1,
                    is_general: false,
                    offer: 0,
                    level: 10,
                    job: 0,
                    name: format!("P{}", master + seat),
                })
                .collect::<Vec<_>>()
        };
        let mut members = members_for(GUILD_ID, MASTER_VID);
        members.extend(members_for(SECOND_ID, 200));
        world.install_guilds(
            &db::guilds::StoredGuilds {
                guilds: vec![
                    db::guilds::StoredGuild {
                        id: GUILD_ID,
                        name: "Knights".to_owned(),
                        master_pid: MASTER_VID,
                        sp: 400,
                        level: 5,
                        exp: 0,
                        skill_point: 0,
                        skills: vec![0; GUILD_SKILL_COUNT],
                        ladder_point: 1000,
                        win: 0,
                        draw: 0,
                        loss: 0,
                        gold: 0,
                    },
                    db::guilds::StoredGuild {
                        id: SECOND_ID,
                        name: "Wolves".to_owned(),
                        master_pid: 200,
                        sp: 400,
                        level: 5,
                        exp: 0,
                        skill_point: 0,
                        skills: vec![0; GUILD_SKILL_COUNT],
                        ladder_point: 1000,
                        win: 0,
                        draw: 0,
                        loss: 0,
                        gold: 0,
                    },
                ],
                grades,
                members,
            },
            &[],
            &[],
        );
        world.link_guild(Vid::new(MASTER_VID));
    }

    /// The chat text of the first CHAT record in `records`.
    fn first_chat(records: &[Vec<u8>]) -> &[u8] {
        let line = records
            .iter()
            .find(|record| record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value())
            .expect("a chat line");
        &line[protocol::gc_chat::GC_CHAT_WIRE_SIZE..]
    }

    /// A `/war` landing: both enemies insert, each online member hears the opp's
    /// NAME then the WAR record, and the answer names the pair for the notice.
    #[test]
    fn a_declare_lands_names_and_fans() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        let answer = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert_eq!(
            answer.notice,
            Some(GuildWarNotice::Declare {
                from_name: b"Knights".to_vec(),
                to_name: b"Wolves".to_vec(),
            }),
            "the notice names declarer then declaree"
        );
        assert_eq!(
            world.guild_war_state(GUILD_ID, SECOND_ID),
            GUILD_WAR_SEND_DECLARE
        );
        assert_eq!(
            world.guild_war_state(SECOND_ID, GUILD_ID),
            GUILD_WAR_RECV_DECLARE
        );
        let held = heard(&mut master);
        // The NAME rode the spawn view already, so the fan is the WAR alone.
        assert_eq!(
            subheaders(&held),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_WAR],
            "the fan is the WAR: {held:?}"
        );
        let war = &held[0];
        assert_eq!(&war[4..8], &GUILD_ID.to_le_bytes(), "dwGuildSelf");
        assert_eq!(&war[8..12], &SECOND_ID.to_le_bytes(), "dwGuildOpp");
        assert_eq!(war[12], GUILD_WAR_TYPE_FIELD, "bType");
        assert_eq!(war[13], GUILD_WAR_SEND_DECLARE, "bWarState");
        drop(rival);
    }

    /// The declaree's `/war` back lands nothing yet (`sys.guild.war` starts it): the
    /// entries move to on-war, and the answer names the acceptor first.
    #[test]
    fn a_declare_back_starts_the_war() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the first declare lands");
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        let answer = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert_eq!(
            answer.notice,
            Some(GuildWarNotice::Accept {
                from_name: b"Wolves".to_vec(),
                to_name: b"Knights".to_vec(),
            }),
            "the notice names acceptor then declarer"
        );
        assert_eq!(world.guild_war_state(GUILD_ID, SECOND_ID), GUILD_WAR_ON_WAR);
        assert_eq!(world.guild_war_state(SECOND_ID, GUILD_ID), GUILD_WAR_ON_WAR);
        assert!(
            world.guilds.wars.contains(&(GUILD_ID, SECOND_ID))
                || world.guilds.wars.contains(&(SECOND_ID, GUILD_ID)),
            "the pair joins the war set"
        );
        let held = heard(&mut master);
        assert!(
            first_chat(&held)
                .windows(8)
                .any(|window| window == b"[LS;558]"),
            "the on-war fan chats [LS;558] first: {held:?}"
        );
        drop(rival);
    }

    /// A started war is listed on enter: the entrant hears the `WAR_LIST` envelope with
    /// the pair.
    #[test]
    fn a_started_war_is_listed_on_enter() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        let accept = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert!(accept.notice.is_some(), "the accept starts the war");
        let _ = heard(&mut master);
        // A raw enter: the shared fixture drains the enter WAR_LIST, so read it
        // straight from the admission.
        let (tx, mut late) = tokio::sync::mpsc::unbounded_channel();
        world
            .enter_world(
                Vid::new(300),
                300,
                "P300",
                crate::client_registry::ClientOutbox::new(tx),
            )
            .expect("the late player enters");
        let held = heard(&mut late);
        assert_eq!(
            subheaders(&held),
            [protocol::gc_guild::GUILD_SUBHEADER_GC_GUILD_WAR_LIST],
            "the entrant hears the WAR_LIST: {held:?}"
        );
        drop(rival);
    }

    /// `/gwlist` reads the live pair in set order, with both names and ids and the age.
    #[test]
    fn the_gwlist_reads_the_live_pair_and_its_age() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        assert_eq!(
            world.guild_war_list(NOW + 42),
            vec![GuildWarListLine {
                lo_name: b"Knights".to_vec(),
                lo_id: GUILD_ID,
                hi_name: b"Wolves".to_vec(),
                hi_id: SECOND_ID,
                seconds: 42,
            }]
        );
        assert_eq!(
            world.guild_war_list(NOW - 1)[0].seconds,
            0,
            "a clock behind the start clamps to zero"
        );
        drop(rival);
    }

    /// A kill across a begun field war scores the victim's level, and both guilds' online
    /// members hear the new total.
    #[test]
    fn a_kill_across_the_war_scores_the_level() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let mut rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        let _ = world.guild_war(Vid::new(200), b"Knights", NOW);
        let _ = heard(&mut master);
        let _ = heard(&mut rival);
        // The fixture enters at level 10.
        world.guild_war_kill(Vid::new(200), Vid::new(MASTER_VID));
        let score = world
            .guilds
            .by_id
            .get(&SECOND_ID)
            .and_then(|guild| guild.enemies.get(&GUILD_ID))
            .map_or(u32::MAX, |enemy| enemy.score);
        assert_eq!(score, 10, "the victim's level scores");
        for (held, side) in [
            (heard(&mut master), "declarer"),
            (heard(&mut rival), "gainer"),
        ] {
            assert_eq!(
                subheaders(&held),
                [protocol::gc_guild::GUILD_SUBHEADER_GC_WAR_SCORE],
                "the {side} hears the WAR_SCORE: {held:?}"
            );
        }
    }

    /// A kill across a mere declare scores nothing.
    #[test]
    fn a_kill_across_a_declare_scores_nothing() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let mut rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        let _ = heard(&mut master);
        let _ = heard(&mut rival);
        world.guild_war_kill(Vid::new(200), Vid::new(MASTER_VID));
        let score = world
            .guilds
            .by_id
            .get(&SECOND_ID)
            .and_then(|guild| guild.enemies.get(&GUILD_ID))
            .map_or(u32::MAX, |enemy| enemy.score);
        assert_eq!(score, 0, "a declare scores nothing");
        assert!(heard(&mut master).is_empty(), "the declarer hears nothing");
        assert!(heard(&mut rival).is_empty(), "the rival hears nothing");
    }

    /// A remove during a war chats the cannot-leave line even for a bogus PID.
    #[test]
    fn a_remove_during_a_war_chats_the_line_for_a_bogus_pid() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        let _ = world.guild_war(Vid::new(200), b"Knights", NOW);
        let answer = world.answer_guild_remove(Vid::new(MASTER_VID), 4242, NOW);
        let GuildRemoveAnswer::Refused { line } = answer else {
            panic!("a war-time remove refuses");
        };
        let line = line.expect("the line chats");
        assert!(
            line.windows(17)
                .any(|window| window == b"You cannot leave "),
            "the cannot-leave line chats: {line:?}"
        );
        drop(rival);
    }

    /// A relog mid-war hears the WAR record plus the score pair.
    #[test]
    fn a_relog_mid_war_hears_the_war_and_its_scores() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        let _ = world.guild_war(Vid::new(200), b"Knights", NOW);
        world.guild_war_kill(Vid::new(200), Vid::new(MASTER_VID));
        world.unlink_guild(Vid::new(MASTER_VID));
        let _ = heard(&mut master);
        world.link_guild(Vid::new(MASTER_VID));
        let held = heard(&mut master);
        let subs = subheaders(&held);
        assert!(
            subs.contains(&protocol::gc_guild::GUILD_SUBHEADER_GC_WAR),
            "the WAR record replays: {subs:?}"
        );
        let mut scores = 0;
        for sub in &subs {
            if *sub == protocol::gc_guild::GUILD_SUBHEADER_GC_WAR_SCORE {
                scores += 1;
            }
        }
        assert_eq!(scores, 2, "the score pair replays: {subs:?}");
        drop(rival);
    }

    /// A sent declare re-declared chats `[LS;780]` to the master, through the
    /// forwarded path (`RequestDeclareWar`'s `SEND_DECLARE` arm).
    #[test]
    fn a_second_declare_chats_the_master() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the first declare lands");
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        // The re-declare must pass the judge's `SEND_DECLARE` return to reach the
        // forwarded arm — so it goes straight to the request, as the DB forward
        // would carry it.
        let answer = world.request_declare_war(GUILD_ID, SECOND_ID, GUILD_WAR_TYPE_FIELD, NOW);
        assert_eq!(answer.notice, None);
        let held = heard(&mut master);
        assert!(
            first_chat(&held)
                .windows(8)
                .any(|window| window == b"[LS;780]"),
            "the master hears [LS;780]: {held:?}"
        );
        drop(rival);
    }

    /// A `/nowar` against a received declare refuses the pair: the declarer's master
    /// hears `[LS;476]` naming the refuser, and both ends fan WAR END.
    #[test]
    fn a_nowar_refuses_the_pair() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let mut rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the declare lands");
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        let _ = heard(&mut rival);
        world.guild_nowar(Vid::new(200), b"Knights");
        assert_eq!(world.guild_war_state(GUILD_ID, SECOND_ID), GUILD_WAR_NONE);
        assert_eq!(world.guild_war_state(SECOND_ID, GUILD_ID), GUILD_WAR_NONE);
        let held = heard(&mut master);
        assert!(
            first_chat(&held)
                .windows(15)
                .any(|window| window == b"[LS;476;Wolves]"),
            "the declarer hears who refused: {held:?}"
        );
        let end = held
            .iter()
            .find(|record| record[0] == protocol::gc_guild::HEADER_GC_GUILD)
            .expect("the END fan");
        assert_eq!(end[13], GUILD_WAR_END, "bWarState");
        let refused = heard(&mut rival);
        assert!(
            refused
                .iter()
                .filter(|record| record[0] == protocol::gc_guild::HEADER_GC_GUILD)
                .any(|record| record[13] == GUILD_WAR_END),
            "the refuser's END fans too: {refused:?}"
        );
    }

    /// A `/nowar` with no received declare stands quiet: a sent declare is not a
    /// refuse (`RequestRefuseWar`'s state gate).
    #[test]
    fn a_nowar_without_a_received_declare_is_quiet() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the declare lands");
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        // The declarer refuses its own sent declare: nothing holds, nothing fans.
        world.guild_nowar(Vid::new(MASTER_VID), b"Wolves");
        assert_eq!(
            world.guild_war_state(GUILD_ID, SECOND_ID),
            GUILD_WAR_SEND_DECLARE
        );
        assert_eq!(heard(&mut master), Vec::<Vec<u8>>::new(), "no line, no fan");
        drop(rival);
    }

    /// The battle map, `KOR_aGuildWarInfo[GUILD_WAR_TYPE_BATTLE].lMapIndex`
    /// (`G/constants.cpp:1132-1145`).
    const BATTLE_MAP: i32 = 110;

    /// A world whose map index holds the battle map and whose Channel hosts it, so a
    /// battle war has a map to be fought on: the map index
    /// (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`) and the Channel's map set
    /// (`map_allow_find`, `G/config.cpp:174-182`) both answer for it.
    fn with_the_battle_map(world: GameState) -> GameState {
        let atlas = MapAtlas::from_regions(vec![MapRegion {
            index: BATTLE_MAP,
            name: b"battle".to_vec(),
            sx: 0,
            sy: 0,
            ex: 6_400,
            ey: 6_400,
            spawn: (0, 0),
            empire_spawns: None,
        }]);
        let channel_maps: HashMap<u8, Vec<u32>> =
            HashMap::from([(super::super::fixtures::PLACE.0, vec![BATTLE_MAP as u32])]);
        world.with_recall(Arc::new(atlas), &channel_maps, &[])
    }

    /// A battle war whose map the map index does not hold is refused: the master hears
    /// `[LS;669]` and no declare lands (`GuildWar_IsWarMap`, `G/guild_war.cpp:260-271`).
    #[test]
    fn a_declare_of_a_battle_war_without_its_map_is_refused() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = heard(&mut master);
        let answer = world.guild_war(Vid::new(MASTER_VID), b"Wolves 1", NOW);
        assert!(answer.notice.is_none(), "the declare lands nothing");
        assert_eq!(
            world.guild_war_state(GUILD_ID, SECOND_ID),
            GUILD_WAR_NONE,
            "no entry is made"
        );
        let held = heard(&mut master);
        assert!(
            first_chat(&held)
                .windows(8)
                .any(|window| window == b"[LS;669]"),
            "the master hears [LS;669]: {held:?}"
        );
        drop(rival);
    }

    /// The same declare lands when the map index holds the battle map: the entry is a
    /// sent declare, and the rival holds the received one.
    #[test]
    fn a_declare_of_a_battle_war_with_its_map_lands() {
        let (_clock, world, mut master, _guest) = a_guilded_pair();
        let mut world = with_the_battle_map(world);
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = heard(&mut master);
        let answer = world.guild_war(Vid::new(MASTER_VID), b"Wolves 1", NOW);
        assert!(answer.notice.is_some(), "the declare lands");
        assert_eq!(
            world.guild_war_state(GUILD_ID, SECOND_ID),
            GUILD_WAR_SEND_DECLARE
        );
        assert_eq!(
            world.guild_war_state(SECOND_ID, GUILD_ID),
            GUILD_WAR_RECV_DECLARE
        );
        drop(rival);
    }

    /// A battle war that starts takes a private copy of its map: the war's index is the
    /// map's own index times 10,000, the map is hosted, and both sides learn it
    /// (`CGuild::WaitStartWar`, `G/guild_war.cpp:440-530`).
    #[test]
    fn a_started_battle_war_takes_a_private_copy_of_its_map() {
        let (_clock, world, _master, _guest) = a_guilded_pair();
        let mut world = with_the_battle_map(world);
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = world.guild_war(Vid::new(MASTER_VID), b"Wolves 1", NOW);
        let accept = world.guild_war(Vid::new(200), b"Knights 1", NOW);
        assert!(accept.notice.is_some(), "the accept starts the war");
        for (id, opp) in [(GUILD_ID, SECOND_ID), (SECOND_ID, GUILD_ID)] {
            let enemy = world
                .guilds
                .by_id
                .get(&id)
                .expect("installed")
                .enemies
                .get(&opp)
                .expect("the war's entry");
            assert_eq!(enemy.map_index, 1_100_000, "the private copy's index");
        }
        assert!(
            world
                .maps
                .contains_key(&(super::super::fixtures::PLACE.0, 1_100_000)),
            "the private map is hosted on the war's Channel"
        );
        drop(rival);
    }

    /// `/war`'s type word parses as `strtoul` does: garbage is field, a past-max
    /// number folds to field, and a `-` word refuses quiet after the fold.
    #[test]
    fn the_war_type_word_parses_as_strtoul() {
        assert_eq!(parse_war_type(b""), 0);
        assert_eq!(parse_war_type(b"garbage"), 0);
        assert_eq!(parse_war_type(b"1"), 1);
        assert_eq!(parse_war_type(b"1x"), 1);
        assert_eq!(parse_war_type(b" 2"), 2);
        assert_eq!(parse_war_type(b"+2"), 2);
        assert_eq!(parse_war_type(b"299"), 299);
        assert_eq!(parse_war_type_or_field(b""), GUILD_WAR_TYPE_FIELD);
        assert_eq!(parse_war_type_or_field(b"garbage"), GUILD_WAR_TYPE_FIELD);
        assert_eq!(parse_war_type_or_field(b"2"), 2);
        assert_eq!(parse_war_type_or_field(b"3"), GUILD_WAR_TYPE_FIELD);
        assert_eq!(parse_war_type_or_field(b"299"), GUILD_WAR_TYPE_FIELD);
        assert_eq!(parse_war_long(b"1"), 0);
        assert_eq!(parse_war_long(b"-1"), -1);
        assert_eq!(parse_war_long(b"-0"), 0);
        assert_eq!(parse_war_long(b"garbage"), 0);
    }

    /// The judge's gates in order: the guildless quiet, the empty-name quiet, the
    /// non-master error, the unknown-name error, and the unknown-name `[LS;535]` on
    /// the `/nowar` side.
    #[test]
    fn the_war_gates_judge_in_order() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        // The guest holds no guild: quiet.
        let answer = world.guild_war(Vid::new(GUEST_VID), b"Wolves", NOW);
        assert_eq!(answer.notice, None);
        assert_eq!(heard(&mut guest), Vec::<Vec<u8>>::new());
        // The empty name: quiet.
        let answer = world.guild_war(Vid::new(MASTER_VID), b"", NOW);
        assert_eq!(answer.notice, None);
        assert_eq!(heard(&mut master), Vec::<Vec<u8>>::new());
        // A non-master member (the war pair's grade-2 row at 102, entered): the
        // error.
        let mut follower = enter(&mut world, MASTER_VID + 2, EAST, 0);
        world.link_guild(Vid::new(MASTER_VID + 2));
        let _ = heard(&mut follower);
        let answer = world.guild_war(Vid::new(MASTER_VID + 2), b"Wolves", NOW);
        assert_eq!(answer.notice, None);
        assert!(
            first_chat(&heard(&mut follower))
                .windows(7)
                .any(|window| window == b"1116030"),
            "the non-master hears its error"
        );
        // The unknown name: the error.
        let answer = world.guild_war(Vid::new(MASTER_VID), b"Nobody", NOW);
        assert_eq!(answer.notice, None);
        assert!(
            first_chat(&heard(&mut master))
                .windows(7)
                .any(|window| window == b"1117030"),
            "the unknown name hears its error"
        );
        // The `/nowar` unknown name: `[LS;535]`.
        world.guild_nowar(Vid::new(MASTER_VID), b"Nobody");
        assert!(
            first_chat(&heard(&mut master))
                .windows(8)
                .any(|window| window == b"[LS;535]"),
            "the nowar unknown name hears [LS;535]"
        );
        drop(rival);
    }
    /// Start a war between the fixture pair: declare then accept, both landed.
    fn start_war_pair(world: &mut GameState) {
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the declare lands");
        let accept = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert!(accept.notice.is_some(), "the accept starts the war");
    }

    /// The ended war's bytes: both END fans, the over notice, and the `END_LIST` — and
    /// the pair leaves the war set with the end-time stamped.
    #[test]
    fn a_win_ends_with_notice_fans_and_end_list() {
        let (_clock, mut world, mut master, mut guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        // The kill scores the winner: level 10 pays 10.
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let _ = heard(&mut master);
        let _ = heard(&mut guest);
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, false, true, NOW)
            .expect("a live war ends");
        assert_eq!(outcome.lo.wins, 1, "the winner tallies a win");
        assert_eq!(outcome.hi.losses, 1, "the loser tallies a loss");
        assert_eq!(outcome.lo.ladder, 1050, "5% of the loser's 1000");
        assert_eq!(outcome.hi.ladder, 930, "7% off the loser's 1000");
        assert!(
            !world.guilds.wars.contains(&(GUILD_ID, SECOND_ID)),
            "the pair leaves the war set"
        );
        assert_eq!(
            world.guilds.war_end_times.get(&(GUILD_ID, SECOND_ID)),
            Some(&NOW),
            "the end-time stamps"
        );
        let held = heard(&mut master);
        assert!(
            held.iter().any(
                |record| record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()
                    && record[protocol::gc_chat::GC_CHAT_WIRE_SIZE..]
                        .windows(7)
                        .any(|window| window == b"[LS;480")
            ),
            "the winner hears the over notice: {held:?}"
        );
        assert!(
            held.iter().any(|record| record.len() == 12
                && record[3] == protocol::gc_guild::GUILD_SUBHEADER_GC_WAR_END_LIST
                && record[4..8] == GUILD_ID.to_le_bytes()
                && record[8..12] == SECOND_ID.to_le_bytes()),
            "the END_LIST names lo then hi: {held:?}"
        );
        let war = held
            .iter()
            .find(|record| {
                record[3] == protocol::gc_guild::GUILD_SUBHEADER_GC_WAR && record.len() > 13
            })
            .expect("an END fan");
        assert_eq!(war[13], GUILD_WAR_END, "the fan's state ends");
        drop(rival);
    }

    /// A forced draw tallies both draws and rewrites both ladders unchanged.
    #[test]
    fn a_forced_draw_tallies_draws_and_keeps_ladder() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, true, true, NOW)
            .expect("a live war ends");
        assert_eq!((outcome.lo.draws, outcome.hi.draws), (1, 1));
        assert_eq!((outcome.lo.ladder, outcome.hi.ladder), (1000, 1000));
        assert_eq!((outcome.lo.wins, outcome.hi.losses), (0, 0));
        drop(rival);
    }

    /// A forced draw turns the ladder math into a draw, but the over notice still
    /// follows the scores: the force flag is the DB's, and it never reaches the game
    /// side's `EndWar` (`G/guild_manager.cpp:697-703`).
    #[test]
    fn a_forced_draw_still_names_the_winner_when_the_scores_differ() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        // The master's kill scores the master's guild: 10 against 0.
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let _ = heard(&mut master);
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, true, true, NOW)
            .expect("a live war ends");
        assert_eq!(
            (outcome.lo.draws, outcome.hi.draws),
            (1, 1),
            "the math draws"
        );
        let held = heard(&mut master);
        let notice = first_chat(&held);
        assert!(
            notice
                .windows(23)
                .any(|window| window == b"[LS;480;Knights;Wolves]"),
            "the notice names the winner first: {notice:?}"
        );
        assert!(
            !notice.windows(7).any(|window| window == b"[LS;479"),
            "the notice is not a draw: {notice:?}"
        );
        drop(rival);
    }

    /// The half-win rule: a pair that ended within 12 hours halves the next gain.
    #[test]
    fn a_recent_end_halves_the_next_gain() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        world
            .manager_end_war(GUILD_ID, SECOND_ID, true, true, NOW)
            .expect("the first war ends");
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, false, true, NOW + 100)
            .expect("the second war ends");
        assert_eq!(outcome.lo.ladder, 1025, "the 50 gain halves to 25");
        assert_eq!(outcome.hi.ladder, 930, "the loss never halves");
        drop(rival);
    }

    /// The ladder math floors at 0: a broke loser pays nothing and keeps 0.
    #[test]
    fn the_ladder_math_floors_at_zero() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        world
            .guilds
            .by_id
            .get_mut(&SECOND_ID)
            .expect("installed")
            .ladder_point = 10;
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        // 10 * 0.05 truncates to 0, 10 * 0.07 truncates to 0.
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, false, true, NOW)
            .expect("a live war ends");
        assert_eq!((outcome.lo.ladder, outcome.hi.ladder), (1000, 10));
        world
            .guilds
            .by_id
            .get_mut(&SECOND_ID)
            .expect("installed")
            .ladder_point = 19;
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        // 19 * 0.05 truncates to 0, 19 * 0.07 truncates to 1.
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, false, true, NOW + 50_000)
            .expect("a live war ends");
        assert_eq!(outcome.lo.ladder, 1000, "no gain on 19");
        assert_eq!(outcome.hi.ladder, 18, "one off on 19");
        drop(rival);
    }

    /// `gwstop` echoes the sorted pair to the speaker and ends in a forced draw.
    #[test]
    fn gwstop_echoes_and_forces_a_draw() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let _ = heard(&mut master);
        let outcome = world
            .guild_war_stop(Vid::new(MASTER_VID), b"9 7", NOW)
            .expect("a live war ends");
        assert_eq!((outcome.lo.draws, outcome.hi.draws), (1, 1), "forced draw");
        let held = heard(&mut master);
        let echo = held
            .iter()
            .find(|record| {
                record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()
                    && record[3] == protocol::gc_chat::CHAT_TYPE_TALKING
            })
            .expect("the speaker hears its echo");
        assert!(
            echo[protocol::gc_chat::GC_CHAT_WIRE_SIZE..]
                .windows(3)
                .any(|window| window == b"7 9"),
            "the echo sorts the pair: {echo:?}"
        );
        // A zero id is quiet past nothing: no echo, no end.
        let _ = heard(&mut master);
        assert!(world
            .guild_war_stop(Vid::new(MASTER_VID), b"0 9", NOW)
            .is_none());
        assert!(heard(&mut master).is_empty(), "a zero id stays quiet");
        drop(rival);
    }

    /// `gwcancel` skips the math but still stamps, fans, and chats both masters.
    #[test]
    fn gwcancel_skips_the_math_and_chats_the_masters() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        let _ = heard(&mut master);
        assert!(world.guild_war_cancel(b"7 9", NOW));
        let knights = world.guilds.by_id.get(&GUILD_ID).expect("installed");
        assert_eq!(
            (knights.war_wins, knights.war_draws, knights.war_losses),
            (0, 0, 0),
            "no tallies on a cancel"
        );
        assert_eq!(knights.ladder_point, 1000, "no ladder move on a cancel");
        assert!(
            world
                .guilds
                .war_end_times
                .contains_key(&(GUILD_ID, SECOND_ID)),
            "the cancel still stamps the end-time map"
        );
        let held = heard(&mut master);
        assert!(
            held.iter().any(
                |record| record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()
                    && record[protocol::gc_chat::GC_CHAT_WIRE_SIZE..]
                        .windows(8)
                        .any(|window| window == b"[LS;482]")
            ),
            "the master hears [LS;482]: {held:?}"
        );
        assert!(
            held.iter().any(|record| record.len() == 12
                && record[3] == protocol::gc_guild::GUILD_SUBHEADER_GC_WAR_END_LIST),
            "the END_LIST still fans: {held:?}"
        );
        // A non-war cancels quiet: no end, no chat.
        let _ = heard(&mut master);
        assert!(!world.guild_war_cancel(b"7 9", NOW));
        assert!(heard(&mut master).is_empty(), "a non-war stays quiet");
        drop(rival);
    }

    /// `gwcancel` ends with both names to every PC (`CancelWar`'s last act,
    /// `G/guild_manager.cpp:763-768`), after the two masters' `[LS;482]`.
    #[test]
    fn gwcancel_names_the_pair_to_every_pc() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        let _ = heard(&mut master);
        assert!(world.guild_war_cancel(b"7 9", NOW));
        let held = heard(&mut master);
        assert!(
            held.iter().any(
                |record| record[0] == protocol::gc_inventory::HEADER_GC_CHAT.value()
                    && record[protocol::gc_chat::GC_CHAT_WIRE_SIZE..]
                        .windows(23)
                        .any(|window| window == b"[LS;483;Knights;Wolves]")
            ),
            "every PC hears the pair: {held:?}"
        );
        drop(rival);
    }

    /// The tick ends a war whose 30 minutes passed and queues its rows.
    #[test]
    fn the_tick_ends_an_expired_war() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        // Backdate the start past the duration: the wall clock is `quest_now`.
        let start = super::super::quests::quest_now().saturating_sub(GUILD_WAR_DURATION + 1);
        for id in [GUILD_ID, SECOND_ID] {
            let opp = if id == GUILD_ID { SECOND_ID } else { GUILD_ID };
            world
                .guilds
                .by_id
                .get_mut(&id)
                .expect("installed")
                .enemies
                .get_mut(&opp)
                .expect("on-war")
                .war_start_time = start;
        }
        world.run_guild_ticks(world.last_pulse());
        assert!(
            !world.guilds.wars.contains(&(GUILD_ID, SECOND_ID)),
            "the expired war leaves the set"
        );
        let queued = world.take_war_ends();
        assert_eq!(queued.len(), 1, "the tick queues one outcome");
        assert_eq!(queued[0].lo.draws, 1, "equal scores draw");
        assert!(
            world.take_war_ends().is_empty(),
            "the drain empties the queue"
        );
        drop(rival);
    }

    /// The end strips the war-casts and the bad affects from every online member, but
    /// only when the guild holds no more war.
    #[test]
    fn the_end_strips_the_war_casts_and_bad_affects() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        start_war_pair(&mut world);
        // The guild's war-casts on the master, and a poison beside them.
        for kind in [152, 153, 154, 155, 156, 157] {
            world.add_pc_affect(
                Vid::new(MASTER_VID),
                super::super::affect::Affect::buff(kind, 0, 10, 0, 100, 0),
                false,
            );
        }
        world.add_pc_affect(
            Vid::new(MASTER_VID),
            super::super::affect::Affect::buff(crate::loading_phase::AFFECT_STUN, 0, 0, 0, 100, 0),
            false,
        );
        assert!(
            world
                .affect_events
                .get(&Vid::new(MASTER_VID))
                .is_some_and(|event| event.affects.len() == 7),
            "the master holds the war-casts and the stun"
        );
        let _ = heard(&mut master);
        let outcome = world
            .manager_end_war(GUILD_ID, SECOND_ID, true, true, NOW)
            .expect("a live war ends");
        assert_eq!(outcome.lo.draws, 1);
        let left = world
            .affect_events
            .get(&Vid::new(MASTER_VID))
            .map_or(0, |event| event.affects.len());
        assert_eq!(left, 0, "the strip took every affect");
        let held = heard(&mut master);
        assert!(
            held.iter().any(|record| {
                record[0] == protocol::gc_inventory::HEADER_GC_AFFECT_REMOVE.value()
            }),
            "the strip sent its remove records: {held:?}"
        );
        drop(rival);
    }

    /// The war arm of `battle_is_attackable` (`G/battle.cpp:125-138`): two players whose
    /// guilds are under a war may fight whatever the PK policy says. A player of no guild
    /// has no war arm, and the arm answers before the castle, the arena and the policy.
    #[test]
    fn the_war_arm_opens_the_fight_between_two_guilds_at_war() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        // Before the war, the arm is shut: the two guilds are not at war.
        assert!(
            !world.guild_war_allows(Vid::new(MASTER_VID), Vid::new(200)),
            "no war, no arm"
        );
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the first declare lands");
        let answer = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert!(answer.notice.is_some(), "the declare back starts the war");
        assert!(
            world.guild_war_allows(Vid::new(MASTER_VID), Vid::new(200)),
            "the war arm lets the two fight"
        );
        assert!(
            world.guild_war_allows(Vid::new(200), Vid::new(MASTER_VID)),
            "the arm answers for the other side too"
        );
        assert!(
            !world.guild_war_allows(Vid::new(GUEST_VID), Vid::new(200)),
            "a guildless player has no war arm"
        );
        drop(rival);
    }

    /// `CWarMapManager::CreateWarMap` (`G/war_map.cpp:1042-1059`): the war's private map
    /// becomes a war map of the two guilds, each on a team of its own.
    #[test]
    fn the_war_takes_a_map_of_its_own() {
        let (_clock, mut world, _master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the first declare lands");
        let answer = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert!(answer.notice.is_some(), "the declare back starts the war");
        // `CWarMapManager::CreateWarMap` (`G/war_map.cpp:1042-1059`): the private copy
        // of the war's map type becomes the war's map, and the two guilds stand on it.
        // The war's map needs a Channel that hosts its map, which the world's fixture
        // does not, so the map is made here.
        world.create_war_map(1_100_000, GUILD_ID, SECOND_ID);
        let war = world.war_maps.get(&1_100_000).expect("the world's war map");
        assert_eq!(war.map_index, 1_100_000, "the map is the war's own");
        assert_eq!(
            war.base_map, 110,
            "the map was copied from the war's map type"
        );
        assert!(
            war.team_of(GUILD_ID).is_some(),
            "the first guild stands on it"
        );
        assert!(
            war.team_of(SECOND_ID).is_some(),
            "the second guild stands on it"
        );
        assert_ne!(
            war.team_of(GUILD_ID),
            war.team_of(SECOND_ID),
            "each guild has a team of its own"
        );
        drop(rival);
    }

    /// `CGuildManager::Kill` (`G/char_battle.cpp:1310-1311`): the kill scores the war with
    /// the victim's level, and the score reaches the gainer's members and the opp's
    /// (`SetWarScoreAgainstTo`'s field fan, `G/guild_war.cpp:198-218`).
    #[test]
    fn a_kill_scores_the_war_with_the_victims_level() {
        let (_clock, mut world, mut master, _guest) = a_guilded_pair();
        install_war_pair(&mut world);
        let mut rival = enter(&mut world, 200, FAR, 0);
        world.link_guild(Vid::new(200));
        let first = world.guild_war(Vid::new(MASTER_VID), b"Wolves", NOW);
        assert!(first.notice.is_some(), "the first declare lands");
        let answer = world.guild_war(Vid::new(200), b"Knights", NOW);
        assert!(answer.notice.is_some(), "the declare back starts the war");
        let _ = heard(&mut master);
        let _ = heard(&mut rival);
        world.guild_war_kill(Vid::new(MASTER_VID), Vid::new(200));
        // The score is the victim's level (10), and both sides' masters hear it.
        let score = world
            .guilds
            .by_id
            .get(&GUILD_ID)
            .and_then(|guild| guild.enemies.get(&SECOND_ID))
            .expect("the enemy entry")
            .score;
        assert_eq!(score, 10, "the victim's level scores the war");
        for (inbox, who) in [(&mut master, "the gainer"), (&mut rival, "the opp")] {
            let held = heard(inbox);
            let record = held
                .iter()
                .find(|record| {
                    record[0] == protocol::gc_inventory::HEADER_GC_GUILD.value()
                        && record[3] == protocol::gc_guild::GUILD_SUBHEADER_GC_WAR_SCORE
                })
                .unwrap_or_else(|| panic!("{who} hears the war's score: {held:?}"));
            assert_eq!(&record[4..8], &GUILD_ID.to_le_bytes(), "the gainer");
            assert_eq!(&record[8..12], &SECOND_ID.to_le_bytes(), "the opp");
            assert_eq!(&record[12..16], &10u32.to_le_bytes(), "the score");
        }
    }
}
