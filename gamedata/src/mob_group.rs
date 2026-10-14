//! The mob groups a regen entry spawns: `group.txt` and `group_group.txt`.
//!
//! `CMobManager::Initialize` reads both from the Locale folder after the mob protos
//! (`G/mob_manager.cpp:88-111`); a file it cannot open shuts the core down. Both go through
//! `CTextFileLoader` ([`crate::text_file`]), whose rules hold here: names and keys are lowercased,
//! and a key written twice in one group keeps its first value.
//!
//! - **`LoadGroup`** (`G/mob_manager.cpp:309-381`) reads each root group's `Vnum` with
//!   `GetTokenInteger` (`strtol` of the first value) and its `Leader` line, whose second value is
//!   the leader's vnum (`at(1)`). A group with no `Vnum`, no `Leader` or a one-value `Leader` is
//!   logged and skipped. The members are the keys `1`, `2`, ... up to the first one missing, at
//!   most 255, each read from its **second** value; a one-value member line throws
//!   `std::out_of_range` and ends the process. A member whose vnum is 0 is kept. A vnum written
//!   twice keeps its first group (`std::map::insert`).
//! - **`LoadGroupGroup`** (`:248-307`) reads `Vnum` the same way and its members from the
//!   **first** value of the keys `1` up. The second value, a probability, is parsed and dropped:
//!   `AddMember(dwMobVnum)` takes the default probability of 1 (`G/mob_manager.h:58-68`), so
//!   `GetMember` (`:71-83`) picks uniformly (Defect D1, reproduced). A member whose vnum is 0 is
//!   left out. A vnum written twice keeps its first group-group.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::mob_proto::{strtol, strtoul};
use crate::text_file::{self, TextFileError, TextGroup};

/// The group file, under the Locale folder.
pub const GROUP_FILE: &str = "group.txt";

/// The group-group file, under the Locale folder.
pub const GROUP_GROUP_FILE: &str = "group_group.txt";

/// The highest member key either loader reads: `for (int k = 1; k < 256; ++k)`.
const MEMBER_KEYS: u32 = 255;

/// One `CMobGroup`: its members in file order, the leader first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobGroup {
    /// The group's lowercased name.
    pub name: Vec<u8>,
    /// The leader's vnum, then each member's, as `AddMember` appended them. A vnum may be 0.
    pub members: Vec<u32>,
}

/// A group legacy logs and skips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedGroup {
    /// The file it is in.
    pub file: &'static str,
    /// Its lowercased name.
    pub name: Vec<u8>,
    /// Why it was skipped, as legacy's `sys_err` words it.
    pub reason: &'static str,
}

/// The groups and group-groups of the Locale, keyed by vnum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MobGroups {
    groups: BTreeMap<u32, MobGroup>,
    group_groups: BTreeMap<u32, Vec<u32>>,
    skipped: Vec<SkippedGroup>,
    duplicates: Vec<(&'static str, u32)>,
}

impl MobGroups {
    /// Read `group.txt` and `group_group.txt` from the Locale folder.
    ///
    /// # Errors
    ///
    /// [`MobGroupError::Io`] for a file that cannot be read, which shuts legacy down, and the
    /// other variants for a file legacy would exit on.
    pub fn load(locale_dir: &Path) -> Result<Self, MobGroupError> {
        let read = |name: &str| {
            let path = locale_dir.join(name);
            std::fs::read(&path).map_err(|source| MobGroupError::Io {
                path: path.clone(),
                message: source.to_string(),
            })
        };
        let groups = read(GROUP_FILE)?;
        let group_groups = read(GROUP_GROUP_FILE)?;
        Self::parse(&groups, &group_groups)
    }

    /// Read the two files' bytes.
    ///
    /// # Errors
    ///
    /// [`MobGroupError::Text`] for a file the text reader exits on, and
    /// [`MobGroupError::ShortMember`] for a group member line with one value.
    pub fn parse(groups: &[u8], group_groups: &[u8]) -> Result<Self, MobGroupError> {
        let mut out = Self::default();
        let file = text_file::parse(groups).map_err(|source| MobGroupError::Text {
            file: GROUP_FILE,
            source,
        })?;
        for group in &file.groups {
            out.load_group(group)?;
        }
        let file = text_file::parse(group_groups).map_err(|source| MobGroupError::Text {
            file: GROUP_GROUP_FILE,
            source,
        })?;
        for group in &file.groups {
            out.load_group_group(group);
        }
        Ok(out)
    }

    /// `CMobManager::GetGroup`: the group of `vnum`.
    #[must_use]
    pub fn group(&self, vnum: u32) -> Option<&MobGroup> {
        self.groups.get(&vnum)
    }

    /// The members of the group-group of `vnum`, the groups `GetMember` picks from.
    #[must_use]
    pub fn group_group(&self, vnum: u32) -> Option<&[u32]> {
        self.group_groups.get(&vnum).map(Vec::as_slice)
    }

    /// The number of groups and of group-groups.
    #[must_use]
    pub fn len(&self) -> (usize, usize) {
        (self.groups.len(), self.group_groups.len())
    }

    /// Whether neither file held a group.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.group_groups.is_empty()
    }

    /// The groups legacy logged and skipped, in file order.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedGroup] {
        &self.skipped
    }

    /// The vnums written more than once, with their file; the first one is kept.
    #[must_use]
    pub fn duplicates(&self) -> &[(&'static str, u32)] {
        &self.duplicates
    }

    fn skip(&mut self, file: &'static str, group: &TextGroup, reason: &'static str) {
        self.skipped.push(SkippedGroup {
            file,
            name: group.name.clone(),
            reason,
        });
    }

    /// `LoadGroup`'s work on one root group.
    fn load_group(&mut self, group: &TextGroup) -> Result<(), MobGroupError> {
        let Some(vnum) = vnum_of(group) else {
            self.skip(GROUP_FILE, group, "no vnum");
            return Ok(());
        };
        let Some(leader) = group.get(b"leader") else {
            self.skip(GROUP_FILE, group, "no leader");
            return Ok(());
        };
        let Some(leader) = leader.get(1) else {
            self.skip(GROUP_FILE, group, "no leader vnum");
            return Ok(());
        };
        let mut members = vec![strtoul(leader)];
        for key in 1..=MEMBER_KEYS {
            let Some(values) = group.get(key.to_string().as_bytes()) else {
                break;
            };
            let Some(member) = values.get(1) else {
                return Err(MobGroupError::ShortMember {
                    group: group.name.clone(),
                    key,
                });
            };
            members.push(strtoul(member));
        }
        match self.groups.entry(vnum) {
            Entry::Vacant(slot) => {
                slot.insert(MobGroup {
                    name: group.name.clone(),
                    members,
                });
            }
            Entry::Occupied(_) => self.duplicates.push((GROUP_FILE, vnum)),
        }
        Ok(())
    }

    /// `LoadGroupGroup`'s work on one root group.
    fn load_group_group(&mut self, group: &TextGroup) {
        let Some(vnum) = vnum_of(group) else {
            self.skip(GROUP_GROUP_FILE, group, "no vnum");
            return;
        };
        let mut members = Vec::new();
        for key in 1..=MEMBER_KEYS {
            let Some(values) = group.get(key.to_string().as_bytes()) else {
                break;
            };
            // `GetTokenVector` never answers an empty vector: a key with no value ends its group
            // in the text reader, so the first value is there.
            let member = values.first().map_or(0, |value| strtoul(value));
            if member != 0 {
                members.push(member);
            }
        }
        match self.group_groups.entry(vnum) {
            Entry::Vacant(slot) => {
                slot.insert(members);
            }
            Entry::Occupied(_) => self.duplicates.push((GROUP_GROUP_FILE, vnum)),
        }
    }
}

/// `GetTokenInteger("vnum")` as the `(DWORD)` both maps are keyed by: `strtol` of the first value.
fn vnum_of(group: &TextGroup) -> Option<u32> {
    let value = group.get(b"vnum")?.first()?;
    Some(u32::from_le_bytes(strtol(value).to_le_bytes()))
}

/// Why the group files were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobGroupError {
    /// A file could not be read; legacy calls `thecore_shutdown`.
    Io {
        /// The file.
        path: PathBuf,
        /// The reason.
        message: String,
    },
    /// The text reader exits on the file.
    Text {
        /// The file's name.
        file: &'static str,
        /// What the reader refused.
        source: TextFileError,
    },
    /// A `group.txt` member line has one value, whose `at(1)` throws `std::out_of_range` and ends
    /// the process (`G/mob_manager.cpp:368`).
    ShortMember {
        /// The group's lowercased name.
        group: Vec<u8>,
        /// The member key.
        key: u32,
    },
}

impl fmt::Display for MobGroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Text { file, source } => write!(f, "{file}: {source}"),
            Self::ShortMember { group, key } => write!(
                f,
                "{GROUP_FILE}: group {} member {key} has one value; legacy throws out_of_range",
                String::from_utf8_lossy(group)
            ),
        }
    }
}

impl Error for MobGroupError {}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: &[u8] =
        b"Group\tShark_Group01\r\n{\r\n\tVnum\t6233\r\n\tLeader\tshark_leader\t4301\r\n\
        \t1\tshark_monster1\t4301\r\n\t2\tshark_monster2\t4302\r\n\t2\tshark_monster3\t4303\r\n\
        \t4\tskipped\t4304\r\n}\r\n";

    const GROUP_GROUPS: &[u8] = b"Group\ta1_02\n{\n\tVnum\t102\n\t1\t101\t1\n\t2\t0\t5\n\
        \t3\t103\t20\n}\n";

    #[test]
    fn a_group_is_its_leader_then_its_members_up_to_the_first_missing_key() {
        let groups = MobGroups::parse(GROUPS, b"").expect("loads");
        let group = groups.group(6233).expect("6233");
        assert_eq!(group.name, b"shark_group01");
        assert_eq!(
            group.members,
            [4301, 4301, 4302],
            "the leader's second value, the first `2`, and nothing past the missing `3`"
        );
        assert_eq!(groups.len(), (1, 0));
    }

    /// Both loaders read member keys 1 to 255, and never 256.
    #[test]
    fn the_loaders_read_member_keys_up_to_255() {
        let mut file = b"Group a\n{\n\tVnum\t5\n\tLeader\tx\t1\n".to_vec();
        let mut gg = b"Group b\n{\n\tVnum\t6\n".to_vec();
        for key in 1..=256 {
            file.extend(format!("\t{key}\tx\t{}\n", 1000 + key).bytes());
            gg.extend(format!("\t{key}\t{}\t1\n", 1000 + key).bytes());
        }
        file.extend(b"}\n");
        gg.extend(b"}\n");
        let groups = MobGroups::parse(&file, &gg).expect("loads");
        let members: Vec<u32> = (1001..=1255).collect();
        let group = groups.group(5).expect("5");
        assert_eq!(group.members[0], 1);
        assert_eq!(group.members[1..], members[..]);
        assert_eq!(groups.group_group(6), Some(&members[..]));
    }

    #[test]
    fn a_group_group_keeps_its_first_values_and_drops_zero_and_the_probability() {
        let groups = MobGroups::parse(b"", GROUP_GROUPS).expect("loads");
        assert_eq!(groups.group_group(102), Some(&[101, 103][..]));
        assert_eq!(groups.group(102), None, "a group-group is not a group");
    }

    #[test]
    fn a_group_with_no_vnum_or_leader_is_skipped() {
        let file = b"Group a\n{\n\tLeader\tx\t1\n}\nGroup b\n{\n\tVnum\t2\n}\n\
            Group c\n{\n\tVnum\t3\n\tLeader\t7\n}\nGroup d\n{\n\tVnum\t4\n\tLeader\tx\t9\n}\n";
        let groups = MobGroups::parse(file, b"").expect("loads");
        let reasons: Vec<&str> = groups.skipped().iter().map(|s| s.reason).collect();
        assert_eq!(reasons, ["no vnum", "no leader", "no leader vnum"]);
        assert_eq!(groups.group(4).map(|g| g.members.clone()), Some(vec![9]));
    }

    #[test]
    fn the_first_of_two_vnums_wins() {
        let file =
            b"Group a\n{\n\tVnum\t5\n\tLeader\tx\t1\n}\nGroup b\n{\n\tVnum\t5\n\tLeader\tx\t2\n}\n";
        let gg = b"Group a\n{\n\tVnum\t5\n\t1\t10\n}\nGroup b\n{\n\tVnum\t5\n\t1\t11\n}\n";
        let groups = MobGroups::parse(file, gg).expect("loads");
        assert_eq!(groups.group(5).map(|g| g.members.clone()), Some(vec![1]));
        assert_eq!(groups.group_group(5), Some(&[10][..]));
        assert_eq!(
            groups.duplicates(),
            [(GROUP_FILE, 5), (GROUP_GROUP_FILE, 5)]
        );
    }

    #[test]
    fn a_one_value_member_line_is_refused() {
        let file = b"Group a\n{\n\tVnum\t5\n\tLeader\tx\t1\n\t1\t101\n}\n";
        assert_eq!(
            MobGroups::parse(file, b""),
            Err(MobGroupError::ShortMember {
                group: b"a".to_vec(),
                key: 1
            })
        );
    }

    #[test]
    fn a_vnum_zero_member_is_kept_in_a_group() {
        let file = b"Group a\n{\n\tVnum\t5\n\tLeader\tx\t1\n\t1\tx\t0\n\t2\tx\t3\n}\n";
        let groups = MobGroups::parse(file, b"").expect("loads");
        assert_eq!(
            groups.group(5).map(|g| g.members.clone()),
            Some(vec![1, 0, 3])
        );
    }

    /// The owner's files (R3 §8): 898 groups with 22 vnums written twice, so 876 kept, and 223
    /// group-groups with 2706 written twice, so 222 kept. The first group-group, 2410, is written
    /// in the group format, so its first values are names and it has no members.
    #[test]
    fn the_owners_group_files_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/locale/europe");
        if !dir.join(GROUP_FILE).exists() {
            return;
        }
        let groups = MobGroups::load(&dir).expect("the owner's group files load");
        assert_eq!(groups.len(), (876, 222));
        assert!(groups.skipped().is_empty());
        let twice: Vec<u32> = groups
            .duplicates()
            .iter()
            .filter(|(file, _)| *file == GROUP_FILE)
            .map(|(_, vnum)| *vnum)
            .collect();
        assert_eq!(twice.len(), 22);
        assert!(twice.contains(&2409) && twice.contains(&6111));
        assert_eq!(groups.duplicates().last(), Some(&(GROUP_GROUP_FILE, 2706)));
        assert_eq!(groups.group_group(2410), Some(&[][..]));
        assert_eq!(
            groups.group(6233).map(|g| g.members.clone()),
            Some(vec![4301, 4301, 4301])
        );
        assert_eq!(groups.group_group(101), Some(&[101, 171][..]));
    }
}
