//! The reload line: `/reload` (`ACMD(do_reload)`, `G/cmd_gm.cpp:2639-2733`).
//!
//! The first word's first byte, lowered, names the table; an unknown byte answers nothing.
//! The `p` and `a` arms cross to the DB server in legacy and their effects come in later
//! slices; the rest run here:
//!
//! - `u` tells `Reloading state_user_count` and rereads the two thresholds from the
//!   `state_user_count` file beside the process (`LoadStateUserCount`, `G/config.cpp:1778-1788`),
//!   keeping both when the file is missing and zeroing both when it is empty, as `fscanf`
//!   does. The thresholds live on the status board, which the descriptor rereads beside the
//!   command; the world only tells the line.
//! - `p` tells `Reloading prototype tables` (the mirror, which rereads the text protos,
//!   rebuilds the motions, swaps the descriptor's and the world's tables and recomputes
//!   every PC, is a later slice).
//! - `s` tells `Reloading notice string` (the store has no `string` table yet, so nothing
//!   is read; a later slice reads it and the enter burst greets from it).
//! - `q` tells `Reloading quest` (the script reload itself, which rebuilds the manager's
//!   live Lua state over a fresh `load`, rides a later slice).
//! - `f` runs `fishing::Initialize` (`G/fishing.cpp:186-328`) over the reread bytes with
//!   no line of its own, swapping the table in only when it parses; a table the bytes
//!   refuse keeps the old one. `SendLog` tells every GM (test-server mode counts every
//!   character a GM): `Reloading fish table.` when the old first row was named, then on a
//!   failure `error! cannot open fishing.txt` or `error! parsing fishing.txt` with `
//!   restoring to backup` when the old first row was named.
//! - `t` runs the `/reload t` arm (`:2683-2698`): the chest groups fall away and the
//!   reread file loads in their place, or the caller hears `Nu pot relua
//!   special_item_group.txt..` and the old groups stand (legacy destroys first and tells
//!   the miss with the maps empty; the Rewrite reads first and keeps the old groups, a
//!   Divergence recorded below). On success the caller hears `special_item_group.txt
//!   reluat cu succes...` and, as legacy inserts them into nothing else, only the chest
//!   maps change: the `attr` groups, the unique map and the quest bag registrations stand
//!   as they were.
//! - `m` runs the `/reload m` arm (`:2700-2715`): the mob tables fall away and the
//!   reread file loads with its `drop` groups appended to the `drop_item_group.txt` read,
//!   or the caller hears `Nu pot relua mob_drop_item.txt.` and the old tables stand (the
//!   same read-first Divergence). On success the caller hears `mob_drop_item.txt reluat
//!   cu succes...`.
//! - `c` runs `Cube_init` over nothing: the cube is not ported (`sys.custom.cube` is
//!   `missing`), so the arm is silent.
//!
//! An empty line tells the three (`u`, `p`, `s`) in order. The payload rides the command:
//! the descriptor reads the files before it sends, so the game thread swaps tables without
//! touching the disk.
//!
//! # Divergences
//!
//! - The `t` and `m` arms read before they destroy. Legacy destroys the live maps first,
//!   so a bad file leaves the server with no groups at all until the next good reload;
//!   the Rewrite keeps the old tables on a miss. The caller hears the same lines either
//!   way.

use std::sync::Arc;

use common::vid::Vid;
use protocol::gc_chat::{CHAT_TYPE_INFO, CHAT_TYPE_NOTICE};

use super::GameState;
use crate::command::{GM_IMPLEMENTOR, GM_PLAYER};
use crate::item_move::Mover;

/// What the descriptor read for one `/reload` line: each file's bytes, or `None` when the
/// file could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReloadRead {
    /// `fishing.txt`'s bytes.
    pub fishing: Option<Vec<u8>>,
    /// `special_item_group.txt`'s bytes.
    pub groups: Option<Vec<u8>>,
    /// `mob_drop_item.txt`'s bytes.
    pub mob: Option<Vec<u8>>,
    /// `drop_item_group.txt`'s bytes, which the `m` arm rereads beside the mob file.
    pub drop_groups: Option<Vec<u8>>,
}

impl GameState {
    /// Runs one `/reload` line over the files the descriptor `read`: the tells above and
    /// the table swaps. The GM gate (`GM_IMPLEMENTOR`) is the dispatcher's.
    pub(super) fn reload_command(&mut self, vid: Vid, rest: &[u8], read: &ReloadRead) {
        let (first, _) = crate::command::one_argument(rest);
        let Some(mover) = self.own_mover(vid) else {
            return;
        };
        let Some(byte) = first.first().copied() else {
            self.reload_counts(vid, mover);
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Reloading prototype tables",
                &[],
            );
            self.tell(vid, mover, CHAT_TYPE_INFO, b"Reloading notice string", &[]);
            return;
        };
        match byte.to_ascii_lowercase() {
            b'u' => self.reload_counts(vid, mover),
            b'p' => self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Reloading prototype tables",
                &[],
            ),
            b's' => self.tell(vid, mover, CHAT_TYPE_INFO, b"Reloading notice string", &[]),
            b'q' => self.tell(vid, mover, CHAT_TYPE_INFO, b"Reloading quest", &[]),
            b'f' => self.reload_fishing(read.fishing.as_deref()),
            b't' => self.reload_groups(vid, mover, read.groups.as_deref()),
            b'm' => self.reload_mob(vid, mover, read),
            _ => {}
        }
    }

    /// The `u` arm and the empty line's first tell: `Reloading state_user_count`. The
    /// thresholds live on the status board, which the descriptor rereads beside the
    /// command; the world only tells the line.
    fn reload_counts(&self, vid: Vid, mover: Mover) {
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"Reloading state_user_count",
            &[],
        );
    }

    /// The `f` arm: `fishing::Initialize` over `bytes`, with no line of its own. A table
    /// the bytes refuse keeps the old one; `SendLog` tells every GM (test-server mode
    /// counts every character a GM).
    fn reload_fishing(&mut self, bytes: Option<&[u8]>) {
        let named = self
            .fishing
            .fish()
            .first()
            .is_some_and(|fish| !fish.name.is_empty());
        if named {
            self.send_log(b"Reloading fish table.");
        }
        let parsed = match bytes {
            Some(bytes) => gamedata::fishing::Fishing::read(bytes).ok(),
            None => None,
        };
        let Some(table) = parsed else {
            if bytes.is_none() {
                self.send_log(b"error! cannot open fishing.txt");
            } else {
                self.send_log(b"error! parsing fishing.txt");
            }
            if named {
                self.send_log(b"  restoring to backup");
            }
            return;
        };
        self.fishing = Arc::new(table);
    }

    /// The `t` arm: the reread `special_item_group.txt` replaces the chest groups, or the
    /// caller hears the miss and the old groups stand.
    fn reload_groups(&mut self, vid: Vid, mover: Mover, bytes: Option<&[u8]>) {
        let groups =
            bytes.and_then(|bytes| gamedata::special_item_group::read(bytes, &self.protos).ok());
        let Some(groups) = groups else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Nu pot relua special_item_group.txt..",
                &[],
            );
            return;
        };
        let standing = Arc::make_mut(&mut self.item_groups);
        standing.special = groups.special;
        standing.quest = groups.quest;
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"special_item_group.txt reluat cu succes...",
            &[],
        );
    }

    /// The `m` arm: the reread `mob_drop_item.txt` replaces the mob tables, its `drop`
    /// groups appended to the reread `drop_item_group.txt`, or the caller hears the miss
    /// and the old tables stand.
    fn reload_mob(&mut self, vid: Vid, mover: Mover, read: &ReloadRead) {
        let mut groups = gamedata::drop::DropGroups::default();
        let mob = read.drop_groups.as_deref().and_then(|drop| {
            gamedata::drop::DropGroups::read_group_bytes(drop, &self.protos, &mut groups).ok()
        });
        let mob = mob.and_then(|()| {
            read.mob.as_deref().and_then(|bytes| {
                gamedata::drop::MobDropGroups::read_mob_file(bytes, &self.protos, &mut groups).ok()
            })
        });
        let Some(mob) = mob else {
            self.tell(
                vid,
                mover,
                CHAT_TYPE_INFO,
                b"Nu pot relua mob_drop_item.txt.",
                &[],
            );
            return;
        };
        let standing = Arc::make_mut(&mut self.drops);
        standing.mob = mob;
        standing.groups = groups;
        self.tell(
            vid,
            mover,
            CHAT_TYPE_INFO,
            b"mob_drop_item.txt reluat cu succes...",
            &[],
        );
    }

    /// `SendLog` (`G/cmd_gm.cpp:1491-1514`): a `CHAT_TYPE_NOTICE` line to every character
    /// whose GM level is past `GM_PLAYER`, in VID order. Test-server mode counts every
    /// character a GM.
    fn send_log(&self, text: &[u8]) {
        let mut vids: Vec<Vid> = self.outboxes.keys().copied().collect();
        vids.sort_by_key(|vid| vid.raw());
        for vid in vids {
            let level = if self.test_server {
                GM_IMPLEMENTOR
            } else {
                self.characters
                    .find_by_vid(vid)
                    .ok()
                    .and_then(|character| {
                        character.points().map(world::character::Points::gm_level)
                    })
                    .unwrap_or(GM_PLAYER)
            };
            if level != GM_PLAYER {
                if let Some(mover) = self.own_mover(vid) {
                    self.tell(vid, mover, CHAT_TYPE_NOTICE, text, &[]);
                }
            }
        }
    }
}
