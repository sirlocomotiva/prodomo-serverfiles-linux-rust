//! The biologist Chaegirab's ten missions (`__ENABLE_BIOLOGIST_RENEWAL_SYSTEM__`,
//! `G/Biologist.cpp`).
//!
//! Each mission asks for materials of one vnum, one at a time, with a wait between
//! takes; the 60% take becomes certain with the elixir (71035), and the forget book
//! (72350) lifts the wait. The finished mission pays its reward item and its lasting
//! affect; missions 8 and 9 pay only after the client picks one of the three bonuses.
//! Two flowers (30169, 30170) shorten the wait. The window is the `GC_BIOLOGIST`
//! record; every guard chats its line and sends nothing.
//!
//! The mission, the taken and the completed are the points' fields (`POINT_BIOLOGIST_*`,
//! stored with the row). The wait's end is the `biologist.timeout` quest flag, read
//! against `get_global_time()` (`SetTimeOut`/`GetTimeOut`, `:377-391`).
//!
//! # Defects not reproduced
//!
//! - The selective arm reads `byIndex` uninitialized for a choice past 2 (`:237-245`),
//!   and hands whatever byte to `arr_bonus[state][byIndex]`. The Rewrite ends a choice
//!   past 2 silently.
//! - The time arm indexes the two flowers unchecked (`:299-331`), so an index past 1
//!   reads past the table. The Rewrite ends an index past 1 silently.
//! - The open `memcpy`s 64 bytes from a title shorter than 64 (`:88`), reading past
//!   the literal. The Rewrite pads the title with zeroes.
//! - Mission 6 names `POINT_RESIST_*` (59-62) where `aApplyInfo` wants apply types, so
//!   its affect lands on `POINT_ATTBONUS_*` (54-57) instead (`constants.cpp:784-787`).
//!   The Rewrite keeps the landed points: the honest client shows what the server did.
//! - The close record leaves `dwLevelLimit` uninitialized (`SendBiologistClosePacket`,
//!   `:334-352` sets every field but it). The Rewrite sends 0.
//! - `DropObjects` has no caller (a whole-tree `command grep -a` for it finds only its
//!   definition and declaration; positive control `BiologistOpenPacket` hits
//!   `input_main.cpp:5350`, negative control `BiologistNoSuchSymbolXYZ` hits nothing),
//!   and its state guard can never fire (`state < 0 && state > 9` on a `DWORD`). The
//!   Rewrite ports no drop.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::item_slots::usable_inventory_cells;
use common::point_slot;
use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_COMMAND, CHAT_TYPE_INFO};
use protocol::gc_nested::{GcBiologist, GC_BIOLOGIST_TITLE_SIZE};
use world::character::{number, remove_specify_item, MoveDone, MoveKind};

use super::affect::Affect;
use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `BIOLOGIST_SUBHEADER_CG_OPEN`: open the window on the current mission.
pub const BIOLOGIST_OPEN: u8 = 0;
/// `BIOLOGIST_SUBHEADER_CG_CLOSE`: no arm reads it (`input_main.cpp:5344-5363`).
pub const BIOLOGIST_CLOSE: u8 = 1;
/// `BIOLOGIST_SUBHEADER_CG_PROVIDES`: hand over one material.
pub const BIOLOGIST_PROVIDES: u8 = 2;
/// `BIOLOGIST_SUBHEADER_CG_SELECTIVE`: pick one of the three bonuses.
pub const BIOLOGIST_SELECTIVE: u8 = 3;
/// `BIOLOGIST_SUBHEADER_CG_TIME`: spend a flower to shorten the wait.
pub const BIOLOGIST_TIME: u8 = 4;

/// `BIOLOGIST_SUBHEADER_GC_OPEN`.
pub const BIOLOGIST_GC_OPEN: u8 = 0;
/// `BIOLOGIST_SUBHEADER_GC_CLOSE`.
pub const BIOLOGIST_GC_CLOSE: u8 = 1;
/// `BIOLOGIST_SUBHEADER_GC_PROVIDES`.
pub const BIOLOGIST_GC_PROVIDES: u8 = 2;
/// `BIOLOGIST_SUBHEADER_GC_TIME`.
pub const BIOLOGIST_GC_TIME: u8 = 4;

/// One decoded `CG_BIOLOGIST` request, as `RecvBiologistPacket` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiologistStep {
    /// `BIOLOGIST_SUBHEADER_CG_OPEN`.
    Open,
    /// `BIOLOGIST_SUBHEADER_CG_PROVIDES`, with the elixir and forget-book bytes.
    Provides {
        /// `byIsElixirUse`: whether the elixir (71035) is spent.
        elixir: u8,
        /// `byIsBookTimeUse`: whether the forget book (72350) is spent.
        book: u8,
    },
    /// `BIOLOGIST_SUBHEADER_CG_SELECTIVE`, with the chosen bonus.
    Selective {
        /// `byChosenAffect`: 0, 1 or 2.
        chosen: u8,
    },
    /// `BIOLOGIST_SUBHEADER_CG_TIME`, with the flower index.
    Time {
        /// `byDecreaseTimeIndex`: 0 or 1.
        flower: u8,
    },
}

/// One mission's row of the `arr_*` tables (`G/Biologist.cpp:15-56`).
struct Mission {
    /// `arr_title`: the window title, in the file's bytes.
    title: &'static [u8],
    /// `arr_level_limit`: the level the guards ask for.
    level: u32,
    /// `arr_item_req`: the material vnum.
    material: u32,
    /// `arr_item_req_count`: the takes that finish the mission.
    takes: u32,
    /// `arr_item_reward`: the finished mission's item.
    reward: u32,
    /// `arr_timeout`: the seconds each take waits.
    wait: i32,
    /// `arr_affect`: the finished mission's affect kind.
    affect: u32,
    /// `arr_bonus`: the three (apply, value) pairs, as the record sends them.
    bonus: [(u32, u32); 3],
    /// `arr_is_selective_affect`: whether the finish waits for the client's pick.
    selective: bool,
}

/// The ten missions (`G/Biologist.cpp:15-56`).
const MISSIONS: [Mission; 10] = [
    Mission {
        title: b"Din\xfeii de Orc",
        level: 30,
        material: 30006,
        takes: 10,
        reward: 50109,
        wait: 1800,
        affect: 553,
        bonus: [(8, 10), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"C\xe3r\xfeile Blestemate",
        level: 40,
        material: 30047,
        takes: 15,
        reward: 50110,
        wait: 2700,
        affect: 554,
        bonus: [(7, 8), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"Suvenirul Demonului",
        level: 50,
        material: 30015,
        takes: 15,
        reward: 50111,
        wait: 3600,
        affect: 555,
        bonus: [(54, 60), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"Globurile de Ghea\xfe\xe3",
        level: 60,
        material: 30050,
        takes: 20,
        reward: 50112,
        wait: 4500,
        affect: 556,
        bonus: [(53, 50), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"Crengile Zelkova",
        level: 70,
        material: 30165,
        takes: 25,
        reward: 50113,
        wait: 5400,
        affect: 557,
        bonus: [(63, 5), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"T\xe3bli\xfeele lui Tugyi",
        level: 80,
        material: 30166,
        takes: 30,
        reward: 50114,
        wait: 7200,
        affect: 558,
        bonus: [(7, 8), (64, 10), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"Ramurile Ro\xbaii",
        level: 85,
        material: 30167,
        takes: 40,
        reward: 50115,
        wait: 9000,
        affect: 559,
        bonus: [(59, 10), (60, 10), (61, 10)],
        selective: false,
    },
    Mission {
        title: b"\xcensemnele Liderilor",
        level: 90,
        material: 30168,
        takes: 50,
        reward: 50114,
        wait: 10800,
        affect: 560,
        bonus: [(63, 8), (0, 0), (0, 0)],
        selective: false,
    },
    Mission {
        title: b"Bijuterie Rea Voint\xe3",
        level: 92,
        material: 30251,
        takes: 10,
        reward: 71107,
        wait: 12600,
        affect: 561,
        bonus: [(1, 1000), (53, 50), (54, 120)],
        selective: true,
    },
    Mission {
        title: b"Bijuterie \xcen\xfeelepciune",
        level: 94,
        material: 30252,
        takes: 20,
        reward: 71105,
        wait: 14400,
        affect: 562,
        bonus: [(1, 1100), (53, 60), (54, 140)],
        selective: true,
    },
];

/// `MAX_MISSION_STATE` (`G/Biologist.h:8`): the last mission index.
const LAST_MISSION: u32 = 9;

/// `arr_item_perc`: the elixir vnum, the plain take chance, the elixir chance.
const ELIXIR: (u32, i32, i32) = (71035, 60, 100);
/// `arr_item_forget_item`: the forget book vnum.
const FORGET_BOOK: u32 = 72350;
/// `arr_item_flower`: the two flowers with the seconds each takes off the wait.
const FLOWERS: [(u32, i32); 2] = [(30169, 15 * 60), (30170, 45 * 60)];
/// The finished mission's affect duration (`60*60*60*365` ticks).
const MISSION_AFFECT_DURATION: i32 = 60 * 60 * 60 * 365;

/// The `biologist.timeout` quest flag: the wait's end in `get_global_time()` seconds.
const TIMEOUT_FLAG: &[u8] = b"biologist.timeout";

/// The completed line, spoken by the open, provide and selective guards.
const BIO_DONE_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Misiunile Biologului Chaegirab au fost finalizate \xeen totalitate!";
/// The no-such-mission line.
const BIO_NO_MISSION_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Nu exist\xe3 nicio informa\xfeie pentru acest stadiu al Biologului Chaegirab. Contacta\xfei un membru din echip\xe3 noastr\xe3!";
/// The level line: `%d` is the mission's level.
const BIO_LEVEL_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Atinge nivelul %d pentru a putea \xeencepe acest stadiu al Biologului Chaegirab!";
/// The wait line: a take is still being processed.
const BIO_WAIT_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r \xcenc\xe3 prelucrez ultimul obiect oferit! Te rog s\xe3 mai a\xbatep\xfei o vreme!";
/// The pick-a-bonus line: the selective finish waits for the client.
const BIO_PICK_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Alege unul dintre bonusurile din lista de recompens\xe3!";
/// The overflow line: the taken count reached the takes outside the finish.
const BIO_OVERFLOW_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r OverFlow objects. Contacta\xfei un membru din echipa noastr\xe3!";
/// The no-material line.
const BIO_NO_MATERIAL_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Nu ai niciun material de oferit Biologului Chaegirab!";
/// The elixir-spent line.
const BIO_ELIXIR_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Elixirul Exploat\xe3rii a fost folosit\xe3 pentru a v\xe3 oferi o \xbaans\xe3 perfect\xe3 de oferire a obiectului!";
/// The elixir-missing line.
const BIO_NO_ELIXIR_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Elixirul Exploat\xe3rii nu a putut fi g\xe3sit \xeen inventarul t\xe3u!";
/// The take's success command (`BINARY_BiologistProvidesMaterialSuccesfully`, sic).
const BIO_TOOK_COMMAND: &[u8] = b"BINARY_BiologistProvidesMaterialSuccesfully";
/// The take's failure command.
const BIO_LOST_COMMAND: &[u8] = b"BINARY_BiologistProvidesMaterialFailed";
/// The forget-book-spent line.
const BIO_FORGOT_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Cartea Uit\xe3rii a fost folosit\xe3 pentru e elimina urm\xe3toarea limit\xe3 de timp!";
/// The forget-book-missing line.
const BIO_NO_BOOK_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Cartea Uit\xe3rii nu a putut fi g\xe3sit\xe3 \xeen inventarul t\xe3u!";
/// The all-missions-done line, spoken with the close on the last pick.
const BIO_ALL_DONE_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Toate misiune au fost completate cu succes! Felicit\xe3ri!";
/// The no-flower line.
const BIO_NO_FLOWER_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Ai nevoie de o floare pentru a putea reduce timpul de a\xbateptare!";
/// The no-wait line: the next take is already free.
const BIO_NO_WAIT_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Nu exist\xe3 nicio limit\xe3 de timp. Urm\xe3torul obiect poate fi oferit!";
/// The wait-shortened line: `%d` is the minutes the flower took off.
const BIO_SHORTER_NOTICE: &[u8] =
    b"|cff808000[Biologist]|h|r Timpul de a\xbateptare a fost redus cu %d de minute!";

impl GameState {
    /// A biologist line as an `INFO` chat.
    fn bio_line(&self, actor: Mover, key: &[u8], args: &[Arg<'_>]) -> Vec<u8> {
        crate::chat_line::chat_packet(actor.recipient(&self.locale), CHAT_TYPE_INFO, key, args)
    }

    /// Run one `CG_BIOLOGIST` for the character online under `vid`
    /// (`CInputMain::RecvBiologistPacket`, `G/input_main.cpp:5344-5363`), answered as a
    /// move is. The dispatch sits past the observer block (`:3951`), so no gate guards
    /// it, and an unknown sub-header is absorbed, as legacy's `default`-less `switch`
    /// does. `BIOLOGIST_SUBHEADER_CG_CLOSE` names no arm and is absorbed too.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when no character is online under `vid`.
    pub fn biologist(
        &mut self,
        vid: Vid,
        step: BiologistStep,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let mut moved = MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Given,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        match step {
            BiologistStep::Open => self.bio_open(vid, actor, &mut moved),
            BiologistStep::Provides { elixir, book } => {
                self.bio_provide(vid, actor, elixir, book, &mut moved);
            }
            BiologistStep::Selective { chosen } => self.bio_choose(vid, actor, chosen, &mut moved),
            BiologistStep::Time { flower } => self.bio_shorten(vid, actor, flower, &mut moved),
        }
        Ok(moved)
    }

    /// `GetTimeOut` (`:384-391`): the seconds left on the wait, or 0 and below when the
    /// next take is free. A flag the character never set reads 0
    /// (`quest::PC::GetFlag`, `G/questpc.cpp:92-103`).
    fn bio_left(&self, pid: u32) -> i32 {
        let end = self
            .quests
            .as_ref()
            .and_then(|quests| quests.manager().quest_flag(pid, TIMEOUT_FLAG))
            .unwrap_or(0);
        let now = super::quests::quest_now();
        end.saturating_sub_unsigned(now)
    }

    /// `SetTimeOut` (`:377-382`): the wait ends `secs` from now. A 0 writes now, which
    /// reads back 0 left; it never deletes the flag.
    fn bio_wait(&mut self, pid: u32, secs: i32) {
        let now = super::quests::quest_now();
        let end = i32::try_from(now).unwrap_or(i32::MAX).saturating_add(secs);
        if let Some(quests) = self.quests.as_mut() {
            quests.manager_mut().set_quest_flag(pid, TIMEOUT_FLAG, end);
        }
    }

    /// `IsSelectiveAffect` (`:365-372`): the taken reached the mission's takes and the
    /// mission is one of the two that wait for the client's pick. A state past the
    /// last mission answers false; every caller guards the state first.
    fn bio_selective(&self, vid: Vid) -> bool {
        let Some((state, taken)) = self.bio_state_of(vid) else {
            return false;
        };
        let Some(mission) = MISSIONS.get(usize::try_from(state).unwrap_or(usize::MAX)) else {
            return false;
        };
        taken >= mission.takes && mission.selective
    }

    /// The mission index and the taken count of the player under `vid`.
    fn bio_state_of(&self, vid: Vid) -> Option<(u32, u32)> {
        let character = self.characters.find_by_vid(vid).ok()?;
        let points = character.points()?;
        Some((points.bio_state(), points.bio_taken()))
    }

    /// `IsCompleted` (`:374-380` in the header's order): the completed counter is 1.
    fn bio_done(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(|points| points.bio_completed() == 1)
    }

    /// The shared guards of the open, provide and selective arms: the completed line,
    /// the no-such-mission line, and the level line. Answers the mission on pass.
    fn bio_guarded(
        &mut self,
        vid: Vid,
        actor: Mover,
        moved: &mut MovedItems,
    ) -> Option<(u32, &'static Mission)> {
        if self.bio_done(vid) {
            moved
                .records
                .push(self.bio_line(actor, BIO_DONE_NOTICE, &[]));
            return None;
        }
        let (state, _) = self.bio_state_of(vid)?;
        if state > LAST_MISSION {
            moved
                .records
                .push(self.bio_line(actor, BIO_NO_MISSION_NOTICE, &[]));
            return None;
        }
        let mission = MISSIONS.get(usize::try_from(state).unwrap_or(usize::MAX))?;
        let level = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(0, world::character::Points::level);
        if u32::from(level) < mission.level {
            moved.records.push(self.bio_line(
                actor,
                BIO_LEVEL_NOTICE,
                &[Arg::Int(i64::from(mission.level))],
            ));
            return None;
        }
        Some((state, mission))
    }

    /// The `GC_BIOLOGIST` window on `mission` with `sub` (`BiologistOpenPacket`,
    /// `:83-103`, and its three siblings): the selective byte, the title padded with
    /// zeroes, the five words, the bonus pairs, and the seconds left.
    fn bio_record(&self, vid: Vid, pid: u32, sub: u8, mission: &Mission) -> Vec<u8> {
        let mut title = [0u8; GC_BIOLOGIST_TITLE_SIZE];
        let len = mission.title.len().min(title.len());
        title[..len].copy_from_slice(&mission.title[..len]);
        let record = GcBiologist {
            header: GcBiologist::header(),
            sub_header: sub,
            selective: u8::from(self.bio_selective(vid)),
            title,
            level_limit: mission.level,
            item_req: mission.material,
            item_req_count: mission.takes,
            item_increase_rate: ELIXIR.0,
            item_forget_time: FORGET_BOOK,
            bonus_vnum: [mission.bonus[0].0, mission.bonus[1].0, mission.bonus[2].0],
            bonus_value: [mission.bonus[0].1, mission.bonus[1].1, mission.bonus[2].1],
            time: self.bio_left(pid),
        };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        out
    }

    /// The close record (`SendBiologistClosePacket`, `:334-352`): every field 0. Legacy
    /// leaves `dwLevelLimit` uninitialized; the Rewrite sends 0 (a Defect kept out).
    fn bio_close_record() -> Vec<u8> {
        let record = GcBiologist {
            header: GcBiologist::header(),
            sub_header: BIOLOGIST_GC_CLOSE,
            selective: 0,
            title: [0u8; GC_BIOLOGIST_TITLE_SIZE],
            level_limit: 0,
            item_req: 0,
            item_req_count: 0,
            item_increase_rate: 0,
            item_forget_time: 0,
            bonus_vnum: [0; 3],
            bonus_value: [0; 3],
            time: 0,
        };
        let mut out = Vec::new();
        record.encode_into(&mut out);
        out
    }

    /// `BiologistOpenPacket` (`:58-104`): the guards, then the window.
    fn bio_open(&mut self, vid: Vid, actor: Mover, moved: &mut MovedItems) {
        let pid = moved.owner_id;
        let Some((_, mission)) = self.bio_guarded(vid, actor, moved) else {
            return;
        };
        moved
            .records
            .push(self.bio_record(vid, pid, BIOLOGIST_GC_OPEN, mission));
    }

    /// One `RemoveSpecifyItem(vnum, 1)` for the player under `vid`, its records and
    /// changes folded into `moved`.
    fn bio_spend(&mut self, vid: Vid, actor: Mover, vnum: u32, moved: &mut MovedItems) {
        let protos = self.protos.clone();
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return;
        };
        let usable = usable_inventory_cells(character.inven_point());
        let (items, _, slots) = character.items_points_and_quickslots_mut();
        let done = remove_specify_item(items, slots, &protos, usable, (vnum, 1));
        let done = MoveDone {
            kind: MoveKind::Removed,
            records: done.records,
            changes: done.changes,
        };
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
    }

    /// One `AutoGiveItem(vnum)` for the player under `vid`, silent (`bMsg` is false):
    /// the pickup notice the shared give packs is left out, as the daily gift does.
    /// An unplaced reward lies at the caller's feet.
    fn bio_reward(&mut self, vid: Vid, actor: Mover, vnum: u32, moved: &mut MovedItems) {
        let questing = self.quest_running(vid);
        let Some((outcome, mut given)) = self.auto_give(vid, (vnum, 1), -1, questing) else {
            return;
        };
        if let world::character::AutoGiven::Unplaced(item) = outcome {
            self.lay_auto_given(vid, item);
        }
        given
            .records
            .retain(|record| !matches!(record, world::character::MoveRecord::PickedUp { .. }));
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            given,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        moved.changes.extend(packed.changes);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        moved.points = character.points().cloned();
        moved.quickslots = Some(character.quickslots().clone());
    }

    /// One `PointChange(kind, amount, true)` for the player under `vid`, its records
    /// folded into `moved`.
    fn bio_point(
        &mut self,
        vid: Vid,
        actor: Mover,
        kind: usize,
        amount: i32,
        moved: &mut MovedItems,
    ) {
        let Some(points) = self
            .characters
            .find_by_vid_mut(vid)
            .ok()
            .and_then(|character| character.items_and_points_mut().1)
        else {
            return;
        };
        let Ok(records) = points.point_change(kind, amount, true, false) else {
            return;
        };
        let done = MoveDone {
            kind: MoveKind::Given,
            records: records
                .into_iter()
                .map(world::character::MoveRecord::Point)
                .collect(),
            changes: Vec::new(),
        };
        let packed = MovedItems::new(
            moved.owner_id,
            vid.raw(),
            done,
            actor,
            &self.protos,
            &self.locale,
        );
        moved.records.extend(packed.records);
        let Ok(character) = self.characters.find_by_vid(vid) else {
            return;
        };
        moved.points = character.points().cloned();
    }

    /// The finished mission's affect (`:189-196`): one `AddAffect` per bonus pair whose
    /// value is above 0 (the first unconditionally), each for `60*60*60*365` ticks.
    /// The apply byte maps through `aApplyInfo` (`constants.cpp:717-810`); mission 6's
    /// `POINT_RESIST_*` ids (59-62) land on `POINT_ATTBONUS_*` (54-57), as legacy's own
    /// table lands them.
    fn bio_grant_affects(&mut self, vid: Vid, mission: &Mission) {
        for (index, &(apply, value)) in mission.bonus.iter().enumerate() {
            if index > 0 && value == 0 {
                continue;
            }
            let apply = u8::try_from(apply).unwrap_or(u8::MAX);
            let point = world::character::point_type(apply).unwrap_or(point_slot::POINT_NONE);
            let point = u8::try_from(point).unwrap_or(0);
            self.add_pc_affect(
                vid,
                Affect {
                    kind: mission.affect,
                    apply_on: point,
                    value: i32::try_from(value).unwrap_or(i32::MAX),
                    flag: 0,
                    remaining: MISSION_AFFECT_DURATION,
                    sp_cost: 0,
                },
                false,
            );
        }
    }

    /// `BiologistProvidesMaterialPacket` (`:106-235`): the guards, the elixir, the
    /// take's roll, the finish, the forget book, and the window.
    fn bio_provide(
        &mut self,
        vid: Vid,
        actor: Mover,
        elixir: u8,
        book: u8,
        moved: &mut MovedItems,
    ) {
        let pid = moved.owner_id;
        let Some((_, mission)) = self.bio_guarded(vid, actor, moved) else {
            return;
        };
        let mission: &Mission = mission;
        if self.bio_left(pid) > 0 {
            moved
                .records
                .push(self.bio_line(actor, BIO_WAIT_NOTICE, &[]));
            return;
        }
        if self.bio_selective(vid) {
            moved
                .records
                .push(self.bio_line(actor, BIO_PICK_NOTICE, &[]));
            return;
        }
        let taken = self.bio_state_of(vid).map_or(0, |(_, taken)| taken);
        if taken >= mission.takes {
            moved
                .records
                .push(self.bio_line(actor, BIO_OVERFLOW_NOTICE, &[]));
            return;
        }
        if self.count_specify_item(vid, mission.material) <= 0 {
            moved
                .records
                .push(self.bio_line(actor, BIO_NO_MATERIAL_NOTICE, &[]));
            return;
        }
        let roll = number(&mut self.dice, 1, 100);
        let mut chance = ELIXIR.1;
        if elixir != 0 {
            if self.count_specify_item(vid, ELIXIR.0) > 0 {
                moved
                    .records
                    .push(self.bio_line(actor, BIO_ELIXIR_NOTICE, &[]));
                chance = ELIXIR.2;
                self.bio_spend(vid, actor, ELIXIR.0, moved);
            } else {
                moved
                    .records
                    .push(self.bio_line(actor, BIO_NO_ELIXIR_NOTICE, &[]));
            }
        }
        if roll <= chance {
            moved.records.push(crate::chat_line::chat_packet(
                actor.recipient(&self.locale),
                CHAT_TYPE_COMMAND,
                BIO_TOOK_COMMAND,
                &[],
            ));
            self.bio_spend(vid, actor, mission.material, moved);
            self.bio_point(
                vid,
                actor,
                point_slot::POINT_BIOLOGIST_ITEMS_TAKEN,
                1,
                moved,
            );
        } else {
            moved.records.push(crate::chat_line::chat_packet(
                actor.recipient(&self.locale),
                CHAT_TYPE_COMMAND,
                BIO_LOST_COMMAND,
                &[],
            ));
            self.bio_spend(vid, actor, mission.material, moved);
        }
        self.bio_wait(pid, mission.wait);
        let taken = self.bio_state_of(vid).map_or(0, |(_, taken)| taken);
        if taken == mission.takes {
            self.bio_finish(vid, actor, pid, mission, moved);
        }
        if book != 0 {
            if self.count_specify_item(vid, FORGET_BOOK) > 0 {
                moved
                    .records
                    .push(self.bio_line(actor, BIO_FORGOT_NOTICE, &[]));
                self.bio_wait(pid, 0);
                self.bio_spend(vid, actor, FORGET_BOOK, moved);
            } else {
                moved
                    .records
                    .push(self.bio_line(actor, BIO_NO_BOOK_NOTICE, &[]));
            }
        }
        let Some((_, mission)) = self.bio_window_mission(vid) else {
            return;
        };
        moved
            .records
            .push(self.bio_record(vid, pid, BIOLOGIST_GC_PROVIDES, mission));
    }

    /// The provide's finish (`:185-216`): the taken reached the mission's takes, so a
    /// selective mission lifts its wait for the client's pick, and any other pays its
    /// reward item and lasting affect, rises the state, and clears the taken.
    fn bio_finish(
        &mut self,
        vid: Vid,
        actor: Mover,
        pid: u32,
        mission: &Mission,
        moved: &mut MovedItems,
    ) {
        if self.bio_selective(vid) {
            self.bio_wait(pid, 0);
            return;
        }
        let reward = mission.reward;
        self.bio_reward(vid, actor, reward, moved);
        self.bio_grant_affects(vid, mission);
        self.bio_point(vid, actor, point_slot::POINT_BIOLOGIST_STATE, 1, moved);
        let taken = self.bio_state_of(vid).map_or(0, |(_, taken)| taken);
        let back = i32::try_from(taken).unwrap_or(i32::MAX);
        self.bio_point(
            vid,
            actor,
            point_slot::POINT_BIOLOGIST_ITEMS_TAKEN,
            -back,
            moved,
        );
        self.bio_wait(pid, 0);
    }

    /// The mission the window shows after a provide or a pick: the state's own.
    fn bio_window_mission(&self, vid: Vid) -> Option<(u32, &'static Mission)> {
        let (state, _) = self.bio_state_of(vid)?;
        MISSIONS
            .get(usize::try_from(state).unwrap_or(usize::MAX))
            .map(|mission| (state, mission))
    }

    /// `BiologistChosenAffectPacket` (`:237-297`): the guards, then the pick. Below the
    /// last mission the taken clears first; the reward and the one picked affect land;
    /// the state rises; the wait clears. On the last mission with the taken at its
    /// takes, the all-done line chats, the completed sets, and the close goes out;
    /// else the window re-sends. A choice past 2 ends silently (legacy reads
    /// `byIndex` uninitialized there: a Defect kept out).
    fn bio_choose(&mut self, vid: Vid, actor: Mover, chosen: u8, moved: &mut MovedItems) {
        if chosen > 2 {
            return;
        }
        let pid = moved.owner_id;
        let Some((state, mission)) = self.bio_guarded(vid, actor, moved) else {
            return;
        };
        let pick = usize::from(chosen);
        if state < LAST_MISSION {
            let taken = self.bio_state_of(vid).map_or(0, |(_, taken)| taken);
            let back = i32::try_from(taken).unwrap_or(i32::MAX);
            self.bio_point(
                vid,
                actor,
                point_slot::POINT_BIOLOGIST_ITEMS_TAKEN,
                -back,
                moved,
            );
        }
        let reward = mission.reward;
        let (apply, value) = mission.bonus[pick];
        let affect = mission.affect;
        self.bio_reward(vid, actor, reward, moved);
        let apply = u8::try_from(apply).unwrap_or(u8::MAX);
        let point = world::character::point_type(apply).unwrap_or(point_slot::POINT_NONE);
        let point = u8::try_from(point).unwrap_or(0);
        self.add_pc_affect(
            vid,
            Affect {
                kind: affect,
                apply_on: point,
                value: i32::try_from(value).unwrap_or(i32::MAX),
                flag: 0,
                remaining: MISSION_AFFECT_DURATION,
                sp_cost: 0,
            },
            false,
        );
        self.bio_point(vid, actor, point_slot::POINT_BIOLOGIST_STATE, 1, moved);
        self.bio_wait(pid, 0);
        let (state, taken) = self.bio_state_of(vid).unwrap_or((0, 0));
        let need = MISSIONS
            .get(usize::try_from(state).unwrap_or(usize::MAX))
            .map_or(0, |mission| mission.takes);
        if taken == need && state == LAST_MISSION {
            moved
                .records
                .push(self.bio_line(actor, BIO_ALL_DONE_NOTICE, &[]));
            self.bio_point(vid, actor, point_slot::POINT_BIOLOGIST_COMPLETED, 1, moved);
            moved.records.push(Self::bio_close_record());
            return;
        }
        let Some((_, mission)) = self.bio_window_mission(vid) else {
            return;
        };
        moved
            .records
            .push(self.bio_record(vid, pid, BIOLOGIST_GC_PROVIDES, mission));
    }

    /// `BiologistDecreaseTime` (`:299-331`): the flower guards, then the wait shortens
    /// by the flower's seconds, the flower leaves, and the window re-sends with the
    /// time sub-header. An index past 1 ends silently (legacy indexes the two flowers
    /// unchecked: a Defect kept out). The spend is `RemoveSpecifyItem(vnum)` with the
    /// default count of 1 (`char.h:1230`).
    fn bio_shorten(&mut self, vid: Vid, actor: Mover, flower: u8, moved: &mut MovedItems) {
        let Some(&(vnum, secs)) = FLOWERS.get(usize::from(flower)) else {
            return;
        };
        let pid = moved.owner_id;
        if self.count_specify_item(vid, vnum) <= 0 {
            moved
                .records
                .push(self.bio_line(actor, BIO_NO_FLOWER_NOTICE, &[]));
            return;
        }
        if self.bio_left(pid) <= 0 {
            moved
                .records
                .push(self.bio_line(actor, BIO_NO_WAIT_NOTICE, &[]));
            return;
        }
        let left = self.bio_left(pid).saturating_sub(secs);
        self.bio_wait(pid, left);
        moved.records.push(self.bio_line(
            actor,
            BIO_SHORTER_NOTICE,
            &[Arg::Int(i64::from(secs / 60))],
        ));
        self.bio_spend(vid, actor, vnum, moved);
        let Some((_, mission)) = self.bio_window_mission(vid) else {
            return;
        };
        moved
            .records
            .push(self.bio_record(vid, pid, BIOLOGIST_GC_TIME, mission));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::fixtures::{a_world, enter_seeing_with, points_at, TestClock, PLACE};
    use protocol::item_pos::ItemPos;
    use world::character::Pcg32;
    use world::item::Item;

    /// One take's in-world aftermath: the taken count, the teeth left, and whether the
    /// success or the failure command went out.
    struct Take {
        taken: u32,
        teeth: i32,
        took: bool,
        lost: bool,
    }

    /// A level-30 world with one tooth at cell 0 (and one elixir at cell 1 when
    /// `elixir` holds), the dice seeded, and one provide taken with or without the
    /// elixir byte.
    fn take_with(seed: u64, elixir: bool) -> Take {
        let clock = TestClock::default();
        let mut world = a_world(&clock);
        let tooth = ItemPos::new(1, 0);
        let mut items = vec![{
            let mut tooth_item = Item::new(11, 30_006);
            tooth_item.set_size(1).expect("a positive footprint");
            (tooth, tooth_item)
        }];
        if elixir {
            let mut elixir_item = Item::new(12, 71_035);
            elixir_item.set_size(1).expect("a positive footprint");
            items.push((ItemPos::new(1, 1), elixir_item));
        }
        let vid = Vid::new(1);
        let _ = enter_seeing_with(&mut world, PLACE, 1, (5, 5), 10_000, &items);
        world
            .characters
            .find_by_vid_mut(vid)
            .expect("the player is in the world")
            .set_points(Some(points_at(30, 10_000, 0)));
        world.dice = Pcg32::new(seed, 0);
        let actor = world.own_mover(vid).expect("the body stands");
        let moved = world
            .biologist(
                vid,
                BiologistStep::Provides {
                    elixir: u8::from(elixir),
                    book: 0,
                },
                actor,
            )
            .expect("the player is in the world");
        let (_, taken) = world.bio_state_of(vid).expect("the player is in the world");
        let has = |needle: &[u8]| {
            moved
                .records
                .iter()
                .any(|record| record.windows(needle.len()).any(|at| at == needle))
        };
        Take {
            taken,
            teeth: world.count_specify_item(vid, 30_006),
            took: has(BIO_TOOK_COMMAND),
            lost: has(BIO_LOST_COMMAND),
        }
    }

    /// The take's roll is `<=` the chance: seed 137 draws 60, which the plain 60%
    /// take keeps; seed 10 draws 61, which it loses; seed 317 draws 100, which the
    /// elixir's certainty keeps. The tooth leaves either way; the taken rises only
    /// on a keep.
    #[test]
    fn the_take_keeps_the_roll_at_the_chance() {
        let kept = take_with(137, false);
        assert_eq!((kept.taken, kept.teeth), (1, 0), "60 of 60% lands");
        assert!(kept.took && !kept.lost, "the success command goes out");
        let lost = take_with(10, false);
        assert_eq!((lost.taken, lost.teeth), (0, 0), "61 of 60% misses");
        assert!(lost.lost && !lost.took, "the failure command goes out");
        let sure = take_with(317, true);
        assert_eq!((sure.taken, sure.teeth), (1, 0), "100 of 100% lands");
        assert!(sure.took && !sure.lost, "the elixir keeps the edge roll");
    }
}
