//! The quest targets (`CTargetManager`, `G/target.cpp`): the characters a quest marks on its
//! character's client with `target.vid`, each the character's own, which the client hears of as
//! `GC_TARGET_CREATE` (125) and `GC_TARGET_DELETE` (124).
//!
//! A target's ID comes from one counter for the whole process (`m_iID`), so the IDs one
//! character sees are consecutive only while no one else is marked. A target that replaces one
//! of the same quest and name keeps its place and takes a new ID; a character that leaves the
//! world drops its targets and its client hears nothing (`CTargetManager::Logout`, run from
//! `CHARACTER::Disconnect`, `G/char.cpp:1815`).
//!
//! What follows a target is not ported yet: the event that watches it (`target_event`,
//! `G/target.cpp:57-132`, with record 123 and the `arrive` and `die` events) and the click on a
//! marked character (`CQuestManager::Click`'s `GetTargetInfo`). Only `TARGET_TYPE_VID` targets
//! exist, because `target.pos` is not ported.
//!
//! # Divergences
//!
//! - **Long names match.** Legacy keeps the first 32 bytes of a name (`szTargetName[32+1]`) but
//!   compares the whole name a script passes, so a target named with more than 32 bytes never
//!   matches itself: each `target.vid` adds another, and `target.delete` and `target.id` miss
//!   it (a Defect). The Rewrite compares the kept 32 bytes with the first 32 of the name.
//! - **The name field is zero-filled.** `SendTargetCreatePacket` fills a stack record whose
//!   `szName` bytes past the `strlcpy` terminator are never written (a Defect); the Rewrite
//!   sends zeros there.
//! - **The VID is a `DWORD`.** Legacy stores `(int)lua_tonumber` in `iArg1`, which cannot hold
//!   the Rewrite's NPC VIDs from 2^31 up ([`quest::manager`]'s `target.vid`).

use std::collections::BTreeMap;

use protocol::gc_actors::{GcTargetCreateNew, TARGET_NAME_LEN};
use protocol::gc_inventory::HEADER_GC_TARGET_DELETE;
use protocol::gc_vid::GcHeaderAndDword;
use quest::manager::Target;

/// `TARGET_TYPE_VID` (`G/target.h:7`): a target on a character.
const TARGET_TYPE_VID: u8 = 1 << 1;

/// What `strlcpy` keeps of a name in `szTargetName[32+1]` and `szTargetDesc[32+1]`.
const KEPT: usize = TARGET_NAME_LEN - 1;

/// One target (`TargetInfo`, `G/target.h:13-51`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuestTarget {
    /// `iID`.
    id: i32,
    /// `dwQuestIndex`.
    quest: u32,
    /// `szTargetName`: the quest's name for it, its first 32 bytes.
    name: Vec<u8>,
    /// `szTargetDesc`: the name the client shows, its first 32 bytes.
    desc: Vec<u8>,
    /// `iArg1`: the VID it points at.
    vid: u32,
    /// `bSendToClient`.
    send: bool,
}

impl QuestTarget {
    /// `SendTargetCreatePacket` (`G/target.cpp:15-28`): nothing unless the client hears of it.
    fn create_record(&self) -> Option<Vec<u8>> {
        if !self.send {
            return None;
        }
        let mut name = [0; TARGET_NAME_LEN];
        name[..self.desc.len()].copy_from_slice(&self.desc);
        let mut out = Vec::new();
        GcTargetCreateNew::new(self.id, name, self.vid, TARGET_TYPE_VID).encode_into(&mut out);
        Some(out)
    }

    /// `SendTargetDeletePacket` (`G/target.cpp:41-47`), when the client heard of it.
    fn delete_record(&self) -> Option<Vec<u8>> {
        if !self.send {
            return None;
        }
        let id = u32::from_le_bytes(self.id.to_le_bytes());
        let mut out = Vec::new();
        GcHeaderAndDword::new(HEADER_GC_TARGET_DELETE.value(), id).encode_into(&mut out);
        Some(out)
    }
}

/// `strlcpy` into 33 bytes: the text up to its first zero, at most 32 bytes of it.
fn kept(text: &[u8]) -> Vec<u8> {
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    text[..end.min(KEPT)].to_vec()
}

/// Every character's quest targets (`m_map_kListEvent`) and the last ID given (`m_iID`).
#[derive(Debug, Default)]
pub(super) struct QuestTargets {
    last_id: i32,
    by_pid: BTreeMap<u32, Vec<QuestTarget>>,
}

impl QuestTargets {
    /// `CTargetManager::CreateTarget` (`G/target.cpp:134-238`) with `TARGET_TYPE_VID` for
    /// character `pid`: a target of the same quest and name is replaced in place, the client
    /// hearing of the old one's end first; else a new one is added. Answers the records for the
    /// character's client.
    pub(super) fn create(&mut self, pid: u32, target: &Target) -> Vec<Vec<u8>> {
        self.last_id = self.last_id.wrapping_add(1);
        let made = QuestTarget {
            id: self.last_id,
            quest: target.quest,
            name: kept(&target.name),
            desc: target.desc.as_deref().map(kept).unwrap_or_default(),
            vid: target.vid,
            send: target.send,
        };
        let mut records = Vec::new();
        let targets = self.by_pid.entry(pid).or_default();
        let same = targets
            .iter_mut()
            .find(|old| old.quest == made.quest && old.name == made.name);
        if let Some(old) = same {
            records.extend(old.delete_record());
            *old = made;
            records.extend(old.create_record());
        } else {
            records.extend(made.create_record());
            targets.push(made);
        }
        records
    }

    /// `CTargetManager::DeleteTarget` (`G/target.cpp:240-282`): character `pid`'s targets of
    /// quest `quest` named `name`, or all of them when `name` is `None`, end in the order they
    /// were made, each the client heard of with a delete record.
    pub(super) fn delete(&mut self, pid: u32, quest: u32, name: Option<&[u8]>) -> Vec<Vec<u8>> {
        let Some(targets) = self.by_pid.get_mut(&pid) else {
            return Vec::new();
        };
        let name = name.map(kept);
        let mut records = Vec::new();
        targets.retain(|target| {
            let ends =
                target.quest == quest && name.as_ref().is_none_or(|name| target.name == *name);
            if ends {
                records.extend(target.delete_record());
            }
            !ends
        });
        if targets.is_empty() {
            self.by_pid.remove(&pid);
        }
        records
    }

    /// `GetTargetEvent` (`G/target.cpp:284-315`) as `target_id` reads it: the VID character
    /// `pid`'s target of quest `quest` named `name` points at, and 0 without one.
    pub(super) fn vid(&self, pid: u32, quest: u32, name: &[u8]) -> u32 {
        let name = kept(name);
        self.by_pid
            .get(&pid)
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| target.quest == quest && target.name == name)
            })
            .map_or(0, |target| target.vid)
    }

    /// `CTargetManager::Logout` (`G/target.cpp:350-363`): character `pid`'s targets end
    /// with no record.
    pub(super) fn logout(&mut self, pid: u32) {
        self.by_pid.remove(&pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(quest: u32, name: &[u8], vid: u32, desc: Option<&[u8]>, send: bool) -> Target {
        Target {
            quest,
            name: name.to_vec(),
            vid,
            desc: desc.map(<[u8]>::to_vec),
            send,
        }
    }

    /// `TPacketGCTargetCreate` (`G/packet.h:2523-2530`) in its field order: the header, `lID`,
    /// the 33-byte `szName`, `dwVID` and `bType`.
    fn create(id: i32, name: &[u8], vid: u32) -> Vec<u8> {
        let mut out = vec![125];
        out.extend_from_slice(&id.to_le_bytes());
        let mut field = [0; 33];
        field[..name.len()].copy_from_slice(name);
        out.extend_from_slice(&field);
        out.extend_from_slice(&vid.to_le_bytes());
        out.push(2);
        out
    }

    fn delete(id: i32) -> Vec<u8> {
        let mut out = vec![124];
        out.extend_from_slice(&id.to_le_bytes());
        out
    }

    /// A new target takes the next ID of the one counter and the client hears of it; the same
    /// quest and name replace it in place with a new ID, the delete first; another quest or
    /// another name is another target.
    #[test]
    fn a_target_of_the_same_quest_and_name_is_replaced() {
        let mut targets = QuestTargets::default();
        let first = targets.create(
            7,
            &target(44, b"theowahdan", 0x8000_0102, Some(b"Th"), true),
        );
        assert_eq!(first, [create(1, b"Th", 0x8000_0102)]);
        assert_eq!(
            hex(&first[0]),
            "7d01000000\
             546800000000000000000000000000000000000000000000000000000000000000\
             0201008002"
        );
        let other = targets.create(9, &target(44, b"theowahdan", 0x0304_0506, None, true));
        assert_eq!(other, [create(2, b"", 0x0304_0506)], "another character's");
        let replaced = targets.create(7, &target(44, b"theowahdan", 0x8000_0103, None, true));
        assert_eq!(replaced, [delete(1), create(3, b"", 0x8000_0103)]);
        assert_eq!(targets.vid(7, 44, b"theowahdan"), 0x8000_0103);
        let quiet = targets.create(7, &target(45, b"theowahdan", 0x0a0b_0c0d, None, false));
        assert!(quiet.is_empty(), "a target the client does not hear of");
        assert_eq!(targets.vid(7, 45, b"theowahdan"), 0x0a0b_0c0d);
        let heard = targets.create(7, &target(45, b"theowahdan", 0x0a0b_0c0e, None, true));
        assert_eq!(
            heard,
            [create(5, b"", 0x0a0b_0c0e)],
            "the quiet one ends quietly"
        );
        assert_eq!(
            targets.vid(7, 44, b"Theowahdan"),
            0,
            "the name is case-sensitive"
        );
        assert_eq!(targets.by_pid[&7].len(), 2);
    }

    /// `strlcpy` keeps 32 bytes of the name and of the description, and both stop at a zero; a
    /// long name matches its own target (a Divergence from legacy's full-name compare).
    #[test]
    fn names_keep_32_bytes_and_long_names_match() {
        let long = [b'n'; 40];
        let desc = [b'd'; 40];
        let mut targets = QuestTargets::default();
        let made = targets.create(7, &target(1, &long, 5, Some(&desc), true));
        assert_eq!(made, [create(1, &desc[..32], 5)]);
        assert_eq!(made[0].len(), 43);
        assert_eq!(made[0][37], 0, "the 33rd byte is the terminator");
        let replaced = targets.create(7, &target(1, &long[..33], 6, Some(b"a\0b"), true));
        assert_eq!(replaced, [delete(1), create(2, b"a", 6)]);
        assert_eq!(targets.vid(7, 1, &long), 6);
        assert_eq!(targets.vid(7, 1, &long[..31]), 0);
        assert_eq!(targets.delete(7, 1, Some(&long[..32])), [delete(2)]);
        assert!(targets.by_pid.is_empty());
    }

    /// A named delete ends the quest's targets of that name; a clear ends every target of the
    /// quest, in the order they were made; another quest's and another character's stay. A
    /// target the client never heard of ends with no record, and a logout ends all with none.
    #[test]
    fn a_delete_ends_the_quests_targets_in_order() {
        let mut targets = QuestTargets::default();
        targets.create(7, &target(1, b"a", 10, None, true));
        targets.create(7, &target(1, b"b", 11, None, false));
        targets.create(7, &target(2, b"a", 12, None, true));
        targets.create(7, &target(1, b"c", 13, None, true));
        targets.create(8, &target(1, b"a", 14, None, true));
        assert!(targets.delete(7, 1, Some(b"z")).is_empty());
        assert!(targets.delete(6, 1, None).is_empty());
        assert_eq!(targets.delete(7, 1, Some(b"a")), [delete(1)]);
        assert_eq!(targets.vid(7, 1, b"a"), 0);
        assert_eq!(targets.vid(7, 2, b"a"), 12);
        assert_eq!(targets.delete(7, 1, None), [delete(4)], "b was never sent");
        assert_eq!(targets.vid(7, 1, b"b"), 0);
        assert_eq!(targets.vid(8, 1, b"a"), 14);
        targets.logout(7);
        assert_eq!(targets.vid(7, 2, b"a"), 0);
        assert!(targets.delete(7, 2, None).is_empty());
        assert_eq!(targets.vid(8, 1, b"a"), 14);
        let next = targets.create(8, &target(1, b"x", 15, None, true));
        assert_eq!(next, [create(6, b"", 15)], "the counter never goes back");
    }

    /// `++m_iID` past `INT_MAX` wraps (legacy's signed overflow); the record carries the bits.
    #[test]
    fn the_id_wraps_at_the_top_of_int() {
        let mut targets = QuestTargets {
            last_id: i32::MAX,
            ..QuestTargets::default()
        };
        let made = targets.create(7, &target(1, b"a", 1, None, true));
        assert_eq!(made, [create(i32::MIN, b"", 1)]);
        assert_eq!(targets.delete(7, 1, None), [delete(i32::MIN)]);
        assert_eq!(hex(&delete(i32::MIN)), "7c00000080");
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }
}
