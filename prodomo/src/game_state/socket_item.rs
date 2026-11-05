//! `/socketitem` (`do_socket_item`, `G/cmd_gm.cpp:3231-3268`).
//!
//! [`GameState::run_socket_item`] gives the caller one item of the vnum by `AutoGiveItem`
//! (the stacks, a free cell, or the ground) and writes 1 into its first sockets, one
//! `SetSocket` each. Each write sends `GC_ITEM_UPDATE` while the caller holds the item
//! (`CItem::UpdatePacket`, `G/item.cpp:249-276`); an item laid on the ground has no owner,
//! so its writes send nothing. A give that makes nothing answers
//! `#%d cannot create item`. The descriptor reads the vnum and the socket count.

use common::vid::Vid;

use protocol::gc_chat::CHAT_TYPE_INFO;

use world::character::{set_item_socket, AutoGiven, MoveDone, MoveKind};

use crate::chat_line::Arg;
use crate::game_loop_messages::GameCommand;
use crate::game_state::{GameState, LettersTrade};
use crate::item_move::Mover;

impl GameState {
    /// Whether `command` is one [`Self::apply_gm_item_line`] runs.
    pub(super) const fn is_gm_item_line(command: &GameCommand) -> bool {
        matches!(
            command,
            GameCommand::SocketItem { .. } | GameCommand::GmLine { .. }
        )
    }

    /// The GM item lines whose rows ride back to the caller's descriptor for storing.
    pub(super) fn apply_gm_item_line(&mut self, command: GameCommand) {
        match command {
            GameCommand::SocketItem {
                vid,
                vnum,
                sockets,
                mover,
                reply,
            } => {
                let given = self.run_socket_item(vid, (vnum, sockets), mover);
                if reply.send(given).is_err() {
                    tracing::debug!(?vid, "nobody was left to hear a socket item's answer");
                }
            }
            GameCommand::GmLine {
                vid,
                line,
                rest,
                mover,
                reply,
            } => {
                let rows = self.run_gm_line(vid, line, &rest, mover);
                if reply.send(rows).is_err() {
                    tracing::debug!(?vid, "nobody was left to hear a GM line's answer");
                }
            }
            _ => debug_assert!(false, "the dispatch routes only GM item lines here"),
        }
    }

    /// `do_socket_item`'s give: one `vnum` with its first `sockets` sockets set to 1.
    /// Answers the stored rows and the records, or `None` when nothing was made, which
    /// the caller already heard.
    pub(super) fn run_socket_item(
        &mut self,
        vid: Vid,
        (vnum, sockets): (u32, usize),
        mover: Mover,
    ) -> Option<LettersTrade> {
        let questing = self.quest_running(vid);
        let given = self.auto_give(vid, (vnum, 1), -1, questing);
        let (id, mut done) = match given {
            Some((AutoGiven::Merged(id) | AutoGiven::Placed { id, .. }, done)) => (Some(id), done),
            Some((AutoGiven::Unplaced(mut item), done)) => {
                for index in 0..sockets {
                    if item.set_socket(index, 1).is_err() {
                        break;
                    }
                }
                self.lay_auto_given(vid, item);
                (None, done)
            }
            Some((AutoGiven::NoProto | AutoGiven::NotMade, _)) | None => {
                self.tell(
                    vid,
                    mover,
                    CHAT_TYPE_INFO,
                    b"#%d cannot create item",
                    &[Arg::Int(i64::from(vnum))],
                );
                return None;
            }
        };
        let Ok(character) = self.characters.find_by_vid_mut(vid) else {
            return None;
        };
        let pid = character.player_id();
        if let Some(id) = id {
            for index in 0..sockets {
                let step = set_item_socket(character.items_mut(), id, (index, 1));
                done.records.extend(step.records);
                done.changes.extend(step.changes);
            }
        }
        let packed = crate::item_move::MovedItems::new(
            pid,
            vid.raw(),
            MoveDone {
                kind: MoveKind::Socketed,
                records: done.records,
                changes: done.changes,
            },
            mover,
            &self.protos,
            &self.locale,
        );
        Some(LettersTrade {
            pid,
            changes: packed.changes,
            records: packed.records,
        })
    }
}
