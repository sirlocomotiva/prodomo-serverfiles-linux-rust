//! The special item groups, `ReadSpecialDropItemFile`
//! (`server/server/game/item_manager_read_tables.cpp:119-302`).
//!
//! `locale/europe/special_item_group.txt` is a brace-delimited text file, read by
//! [`crate::text_file`]. Each group is a bag something opens: a vnum, a type, and a numbered list of
//! rows, each of which is an item, a count, and a weight. The weights are kept as a running total,
//! so drawing from the bag is a `lower_bound` over the totals.
//!
//! The owner's file has one group, `Cufar_Lumina_Lunii`, vnum 50011, no `type`, and 29 rows. Every
//! row names its item by vnum, and every one of those vnums must exist in `item_proto.txt` or
//! legacy refuses the whole file.
//!
//! These are [`crate::special_item_group`]'s legacy behaviours, reproduced rather than improved:
//!
//! - A row whose weight is **0** is dropped, because `AddItem` returns before it pushes
//!   (`item_manager.h:135-136`). A bag of nothing but zero-weight rows is then empty.
//! - The group vnum below 30000 also registers the group's item in a second map
//!   (`item_manager_read_tables.cpp:277-282`), which is what makes a group unique.
//! - A `type` of `attr` makes the group a set of bonuses rather than a set of items
//!   (`:168-210`), stored in a different map with a different key space.
//! - A group vnum written twice keeps the first group, because all three maps are filled with
//!   `std::map::insert`, and `insert` does not overwrite.
//!
//! Two are Defects and are **not** reproduced; see [`SpecialItemGroup::duplicate_tail_rows`] and
//! [`GroupError::ShortRow`].

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::item_proto::ItemProtos;
use crate::item_proto_value::{fn_get_apply_type, MAX_APPLY_NUM};
use crate::text_file::{self, TextFile};

/// The Korean word for "experience", as the bytes in `item_manager_read_tables.cpp:226`.
///
/// The legacy source is in a two-byte code page, so the comparison is against the file's own
/// bytes. A row is matched on the raw token, and the legacy `==` is byte for byte, so this is
/// case-sensitive and is not a UTF-8 string.
pub const EXP_KOREAN: &[u8] = b"\xb0\xe6\xc7\xe8\xc4\xa1";

/// The group vnum below which a group also registers its items as unique
/// (`item_manager_read_tables.cpp:278`).
pub const UNIQUE_GROUP_VNUM_LIMIT: i32 = 30000;

/// The last row key the legacy loop reads: `for (int k = 1; k < 1024; ++k)`.
pub const MAX_ROW: usize = 1023;

/// `CSpecialItemGroup::EGiveType` (`item_manager.h:105-115`).
///
/// These are **not** item vnums. A row that names a keyword instead of an item resolves to the
/// number below, and every consumer tests against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum GiveType {
    /// The row named neither a keyword nor a real item.
    None = 0,
    /// `GOLD`.
    ///
    /// The enum has this member, and no consumer is wrong for having it, but
    /// `ReadSpecialDropItemFile` has no `gold` keyword (`item_manager_read_tables.cpp:226-249`),
    /// so a row that writes `gold` falls through to the vnum read and is then refused as an
    /// unknown item. No group can hold a `GOLD` row; the value is here because the enum has it.
    Gold = 1,
    /// `exp`, or [`EXP_KOREAN`].
    Exp = 2,
    /// `mob`.
    Mob = 3,
    /// `slow`.
    Slow = 4,
    /// `drain_hp`.
    DrainHp = 5,
    /// `poison`.
    Poison = 6,
    /// `group`.
    MobGroup = 7,
}

impl GiveType {
    /// The token a row's first field may hold, as the file writes it.
    ///
    /// The legacy tests are `==` on the raw token (`item_manager_read_tables.cpp:226-249`), so a
    /// row is matched exactly and in the source's own bytes. Nothing is lowercased here, and a
    /// row writing `EXP` is not an experience row.
    pub fn from_token(token: &[u8]) -> Option<Self> {
        Some(match token {
            b"exp" | EXP_KOREAN => Self::Exp,
            b"mob" => Self::Mob,
            b"slow" => Self::Slow,
            b"drain_hp" => Self::DrainHp,
            b"poison" => Self::Poison,
            b"group" => Self::MobGroup,
            _ => return None,
        })
    }

    /// Whether a resolved vnum is one of these small values rather than a real item.
    pub fn from_vnum(vnum: u32) -> Option<Self> {
        match vnum {
            2 => Some(Self::Exp),
            3 => Some(Self::Mob),
            4 => Some(Self::Slow),
            5 => Some(Self::DrainHp),
            6 => Some(Self::Poison),
            7 => Some(Self::MobGroup),
            _ => None,
        }
    }
}

/// `CSpecialItemGroup::ESIGType` (`item_manager.h:116`), a four-member enum with no initialisers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum GroupType {
    /// No `type` key, or a `type` that is not one of the three names.
    #[default]
    Normal = 0,
    /// `type pct`: every row is rolled on its own, so a bag can give more than one row.
    Pct = 1,
    /// `type quest`: goes in the quest map, and the vnum is registered as an NPC vnum.
    Quest = 2,
    /// `type special`: the only type `GetAttrVnum` answers for.
    Special = 3,
}

impl GroupType {
    /// The value for a `type` the loader lowercased (`item_manager_read_tables.cpp:152-165`).
    ///
    /// `attr` is not one of the three: it selects the attribute group, which is a different
    /// struct, so it is refused here and handled by the reader.
    pub fn from_type(word: &[u8]) -> Option<Self> {
        Some(match word {
            b"pct" => Self::Pct,
            b"quest" => Self::Quest,
            b"special" => Self::Special,
            _ => return None,
        })
    }
}

/// `CSpecialItemGroup::CSpecialItemInfo` (`item_manager.h:118-127`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecialItemInfo {
    /// The resolved vnum, which is a [`GiveType`] as a number when the row was a keyword.
    pub vnum: u32,
    /// The second field, how many.
    pub count: i32,
    /// The fourth field, the rare chance.
    ///
    /// A row of exactly three fields leaves it 0, because legacy only reads it when
    /// `pTok->size() > 3` (`item_manager_read_tables.cpp:268-272`).
    pub rare: i32,
}

/// `CSpecialItemGroup` (`item_manager.h:102-228`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecialItemGroup {
    /// The `vnum` key, which is the map key rather than a member the reader looks up.
    pub vnum: i32,
    /// The `type` key, or [`GroupType::Normal`].
    pub group_type: GroupType,
    /// The rows whose weight was not 0, in key order.
    pub items: Vec<SpecialItemInfo>,
    /// The running total of the weights, which is what a draw searches.
    ///
    /// Legacy's `m_vecProbs` is a `std::vector<int>` (`item_manager.h:226`), so the total is a
    /// signed 32-bit value that wraps. A bag whose weights sum past `i32::MAX` therefore has a
    /// non-monotonic total here, and `lower_bound` over it is not a draw; the sum is kept wrapping
    /// here so the reader reports the same numbers legacy would, and
    /// [`SpecialItemGroup::weights_overflow`] names the condition.
    pub totals: Vec<i32>,
}

impl SpecialItemGroup {
    /// `IsEmpty` (`item_manager.h:143-146`): no row survived.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty()
    }

    /// `GetGroupSize` (`item_manager.h:219-222`).
    pub fn group_size(&self) -> usize {
        self.totals.len()
    }

    /// `AddItem` (`item_manager.h:133-141`): push a row, unless its weight is 0.
    ///
    /// The weight is added to the running total rather than stored, so `totals` is a prefix sum
    /// and a draw is a `lower_bound` over it.
    pub fn add_item(&mut self, vnum: u32, count: i32, weight: i32, rare: i32) {
        if weight == 0 {
            return;
        }
        let total = match self.totals.last() {
            Some(last) => last.wrapping_add(weight),
            None => weight,
        };
        self.totals.push(total);
        self.items.push(SpecialItemInfo { vnum, count, rare });
    }

    /// Whether the prefix sum has stopped increasing.
    ///
    /// A negative weight drops a total below the one before it, and a large enough sum wraps past
    /// `i32::MAX`. `GetOneIndex` calls `number(1, m_vecProbs.back())` (`item_manager.h:175-180`),
    /// which on a wrapped or non-monotonic total is a draw from a range `lower_bound` cannot
    /// answer. A bag where this is true is data the legacy server mis-handles.
    pub fn weights_overflow(&self) -> bool {
        self.totals.windows(2).any(|w| w[1] < w[0])
    }

    /// `GetOneIndex` (`item_manager.h:175-180`) for a draw in `1..=totals.last()`.
    ///
    /// `lower_bound` answers the first total that is not less than the draw, and the distance from
    /// the front is the index. An empty bag has no total to draw from, so there is no index; the
    /// legacy function reads `m_vecProbs.back()` on the empty vector, which is a Defect.
    pub fn one_index(&self, draw: i32) -> Option<usize> {
        if self.totals.is_empty() {
            return None;
        }
        Some(self.totals.partition_point(|&t| t < draw))
    }

    /// `GetMultiIndex` (`item_manager.h:147-173`) for a [`GroupType::Pct`] bag, given one
    /// `number(1, 100)` roll per row.
    ///
    /// A `Pct` bag rolls each row on its own weight, which for every row after the first is
    /// `totals[i] - totals[i - 1]` because the totals are a prefix sum. The first row is rolled
    /// against `totals[0]`, which is its own weight. Any other type draws exactly once, which is
    /// why a `Pct` bag is the only one that can give more than one row.
    pub fn multi_index(&self, rolls: &[i32]) -> Vec<usize> {
        if self.group_type != GroupType::Pct {
            return self
                .one_index(rolls.first().copied().unwrap_or(0))
                .into_iter()
                .collect();
        }
        let mut out = Vec::new();
        for (i, &roll) in rolls.iter().enumerate() {
            // Legacy draws `m_vecProbs.at(i) - (i ? m_vecProbs.at(i - 1) : 0)`, so the first row's
            // weight is its own total and no row reads one total below its own.
            let weight = match self.totals.get(i) {
                None => break,
                Some(&t) if i == 0 => t,
                Some(&t) => t - self.totals[i - 1],
            };
            if roll <= weight {
                out.push(i);
            }
        }
        out
    }

    /// `GetVnum` (`item_manager.h:182-185`), with the index bound-checked.
    pub fn vnum(&self, index: usize) -> Option<u32> {
        self.items.get(index).map(|i| i.vnum)
    }

    /// `GetCount` (`item_manager.h:187-190`).
    pub fn count(&self, index: usize) -> Option<i32> {
        self.items.get(index).map(|i| i.count)
    }

    /// `GetRarePct` (`item_manager.h:192-195`).
    pub fn rare_pct(&self, index: usize) -> Option<i32> {
        self.items.get(index).map(|i| i.rare)
    }

    /// `Contains` (`item_manager.h:197-205`).
    pub fn contains(&self, vnum: u32) -> bool {
        self.items.iter().any(|i| i.vnum == vnum)
    }

    /// `GetAttrVnum` (`item_manager.h:206-218`): the count of a row, and only for a
    /// [`GroupType::Special`] bag.
    pub fn attr_vnum(&self, vnum: u32) -> u32 {
        if self.group_type != GroupType::Special {
            return 0;
        }
        self.items
            .iter()
            .find(|i| i.vnum == vnum)
            .map_or(0, |i| widen_dword(i.count))
    }

    /// Whether the `char[4]` key buffer would have read a row twice.
    ///
    /// Legacy formats the row key into `char buf[4]`
    /// (`item_manager_read_tables.cpp:216-217`), so `snprintf` keeps three characters and a key of
    /// 1000 or more is truncated to its first three digits. The loop still runs to 1023, so keys
    /// 1000 to 1023 read as `100`, `101`, and `102` a second time, and a bag with 1000 or more
    /// rows gets those rows pushed again: ten times for `100` and `101` and four for `102`. This
    /// is a Defect and is not reproduced; the answer is here so a caller can report it. The
    /// owner's file has 29 rows and cannot reach it.
    pub fn duplicate_tail_rows(&self) -> bool {
        self.items.len() >= 1000
    }
}

/// `CSpecialAttrGroup::CSpecialAttrInfo` (`item_manager.h:88-95`) and the group around it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecialAttrGroup {
    /// The `vnum` key.
    pub vnum: i32,
    /// The `effect` key's first value, or empty when the group has none
    /// (`item_manager_read_tables.cpp:204-207`).
    pub effect_file: Vec<u8>,
    /// The numbered rows, in key order, as `(apply type, value)`.
    pub attrs: Vec<(u32, i32)>,
}

/// Everything `ReadSpecialDropItemFile` builds.
///
/// Each map is keyed by the group's own `vnum`, which is an `int` in legacy and is kept signed
/// here, so a negative `vnum` key is representable. A vnum written twice keeps the first group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecialItemGroups {
    /// The bags that are not a `quest` bag (`item_manager_read_tables.cpp:296`).
    pub special: BTreeMap<i32, SpecialItemGroup>,
    /// The `quest` bags (`:292`).
    pub quest: BTreeMap<i32, SpecialItemGroup>,
    /// The `type attr` groups (`:209`).
    pub attr: BTreeMap<i32, SpecialAttrGroup>,
    /// The item to unique-group map, which holds only groups below
    /// [`UNIQUE_GROUP_VNUM_LIMIT`] (`:280`).
    pub unique: BTreeMap<u32, i32>,
    /// The vnums a `quest` bag registered with the quest manager (`:160`), in file order and with
    /// a repeat for a repeated vnum, because `RegisterNPCVnum` is called per group and not per
    /// stored group.
    pub quest_npc_vnums: Vec<i32>,
}

/// A group the legacy reader refuses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupError {
    /// A group had no `vnum` key, or one with no value.
    ///
    /// `GetTokenInteger` returns false for both (`text_file_loader.cpp:335-342`) and the reader
    /// then returns false for the whole file (`:136-141`).
    NoVnum {
        /// The lowercased group name, which is what legacy logs.
        group: Vec<u8>,
    },
    /// A row had fewer than three fields.
    ///
    /// Legacy reads `pTok->at(1)` and `pTok->at(2)` with no size test
    /// (`item_manager_read_tables.cpp:264-266`), and `std::vector::at` past the end is undefined
    /// behaviour. This is a Defect and is refused here rather than guessed at.
    ShortRow {
        /// The lowercased group name.
        group: Vec<u8>,
        /// The row key, as the file wrote it.
        key: Vec<u8>,
        /// How many fields the row had.
        fields: usize,
    },
    /// A row named an item that is not in `item_proto.txt`
    /// (`item_manager_read_tables.cpp:252-259`).
    ///
    /// Legacy returns false from the whole load, so one bad row loses every group.
    UnknownItem {
        /// The lowercased group name.
        group: Vec<u8>,
        /// The row's first field, as the file wrote it.
        name: Vec<u8>,
        /// The value `str_to_number` read from it.
        vnum: u32,
    },
    /// An `attr` row named an apply type that is in neither the numbers nor the names
    /// (`item_manager_read_tables.cpp:183-188`).
    UnknownApply {
        /// The lowercased group name.
        group: Vec<u8>,
        /// The row's first field.
        name: Vec<u8>,
    },
    /// An `attr` row's apply type was above `MAX_APPLY_NUM`
    /// (`item_manager_read_tables.cpp:191-196`).
    ///
    /// The test is `> MAX_APPLY_NUM` and not `>=`, so [`MAX_APPLY_NUM`] itself is accepted even
    /// though no apply type has that value. That is an off-by-one in legacy and it is reproduced,
    /// because it decides what loads and a fix here would be a Divergence.
    ApplyOutOfRange {
        /// The lowercased group name.
        group: Vec<u8>,
        /// The resolved apply type.
        apply_type: u32,
    },
    /// The text file would have made legacy `exit(1)`.
    Text(text_file::TextFileError),
}

impl fmt::Display for GroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVnum { group } => {
                write!(
                    f,
                    "group {}: no vnum key; legacy refuses the whole file",
                    text(group)
                )
            }
            Self::ShortRow { group, key, fields } => write!(
                f,
                "group {} row {}: {fields} field(s), and legacy reads three of them; the read \
                 past the end is a Defect",
                text(group),
                text(key)
            ),
            Self::UnknownItem { group, name, vnum } => write!(
                f,
                "group {}: there is no item {} (read as {vnum}); legacy refuses the whole file",
                text(group),
                text(name)
            ),
            Self::UnknownApply { group, name } => write!(
                f,
                "group {}: invalid APPLY_TYPE {}; legacy refuses the whole file",
                text(group),
                text(name)
            ),
            Self::ApplyOutOfRange { group, apply_type } => write!(
                f,
                "group {}: apply type {apply_type} is above MAX_APPLY_NUM; legacy refuses the \
                 whole file",
                text(group)
            ),
            Self::Text(e) => write!(f, "{e}"),
        }
    }
}

impl Error for GroupError {}

/// A byte string for a message, with anything that is not printable shown as an escape.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `ReadSpecialDropItemFile` (`item_manager_read_tables.cpp:119-302`).
///
/// `protos` is the already-read `item_proto.txt`, which legacy holds as `ITEM_MANAGER`. A row names
/// its item by the **original** name first, and only then tries a keyword and a vnum, and a vnum
/// no row of `item_proto.txt` answers for stops the whole load.
///
/// # Errors
///
/// Every error here is a case where legacy refuses the whole file and returns `false`
/// (`item_manager_read_tables.cpp:123-124`, `:136-141`, `:168-210`, `:253-259`): a file the text
/// reader will not cut, a group with no `vnum`, a row that names no item, an apply group whose row
/// names no apply type, and an apply type above [`MAX_APPLY_NUM`]. The rewrite returns the error
/// instead of leaving a half-built group behind.
pub fn read(data: &[u8], protos: &ItemProtos) -> Result<SpecialItemGroups, GroupError> {
    let file: TextFile = text_file::parse(data).map_err(GroupError::Text)?;
    let mut out = SpecialItemGroups::default();
    for group in &file.groups {
        // `GetTokenInteger("vnum", &iVnum)` (`text_file_loader.cpp:332-349`) reads into a local
        // `int out = 0` and then writes it, so it answers 0 for a value that is not a number and
        // returns false only when the key is missing or holds no field at all. `iVnum` is an `int`
        // (`item_manager_read_tables.cpp:134`), so this is the signed read; the row vnum below is a
        // `DWORD` and is not.
        let values = group
            .get(b"vnum")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| GroupError::NoVnum {
                group: group.name.clone(),
            })?;
        let vnum = str_to_i32(&values[0]);
        // `stl_lowers(stType)` runs only when the key is there, so an absent `type` leaves the
        // empty string and neither the three names nor `attr` match.
        let type_word = group
            .get(b"type")
            .and_then(|v| v.first())
            .map(|w| w.to_ascii_lowercase())
            .unwrap_or_default();
        if type_word == b"attr" {
            insert_first_attr(&mut out.attr, vnum, read_attr_group(group, vnum)?);
            continue;
        }
        let group_type = GroupType::from_type(&type_word).unwrap_or_default();
        if group_type == GroupType::Quest {
            out.quest_npc_vnums.push(vnum);
        }
        let mut bag = SpecialItemGroup {
            vnum,
            group_type,
            ..SpecialItemGroup::default()
        };
        for k in 1..=MAX_ROW {
            let Some(row) = group.get(row_key(k).as_bytes()) else {
                break;
            };
            let (item, count, weight, rare) = read_row(group, k, row, protos)?;
            bag.add_item(item, count, weight, rare);
            if vnum < UNIQUE_GROUP_VNUM_LIMIT {
                out.unique.insert(item, vnum);
            }
        }
        if group_type == GroupType::Quest {
            insert_first(&mut out.quest, vnum, bag);
        } else {
            insert_first(&mut out.special, vnum, bag);
        }
    }
    Ok(out)
}

/// One bag row, resolved and read.
fn read_row(
    group: &text_file::TextGroup,
    key: usize,
    row: &[Vec<u8>],
    protos: &ItemProtos,
) -> Result<(u32, i32, i32, i32), GroupError> {
    if row.len() < 3 {
        return Err(GroupError::ShortRow {
            group: group.name.clone(),
            key: row_key(key).into_bytes(),
            fields: row.len(),
        });
    }
    let name = &row[0];
    // `GetVnumByOriginalName` first (`item_manager_read_tables.cpp:224`), so a real item whose
    // original name is a keyword still wins over the keyword.
    let vnum = if let Some(found) = protos.vnum_by_original_name(name) {
        found
    } else if let Some(give) = GiveType::from_token(name) {
        give as u32
    } else {
        // `dwVnum` is a `DWORD` (`:222`) and the read is `str_to_number(dwVnum, ...)` (`:252`),
        // so a row vnum goes through the unsigned scanner and never through a signed one.
        let vnum = str_to_u32(name);
        if protos.get(vnum).is_none() {
            return Err(GroupError::UnknownItem {
                group: group.name.clone(),
                name: name.clone(),
                vnum,
            });
        }
        vnum
    };
    Ok((
        vnum,
        str_to_i32(&row[1]),
        str_to_i32(&row[2]),
        read_rare(row),
    ))
}

/// The fourth field, or 0 when the row has exactly three.
///
/// Legacy reads it only when `pTok->size() > 3` (`item_manager_read_tables.cpp:268-272`), and the
/// local starts at 0, so a shorter row leaves 0.
fn read_rare(row: &[Vec<u8>]) -> i32 {
    if row.len() > 3 {
        str_to_i32(&row[3])
    } else {
        0
    }
}

/// One `type attr` group (`item_manager_read_tables.cpp:168-210`).
fn read_attr_group(
    group: &text_file::TextGroup,
    vnum: i32,
) -> Result<SpecialAttrGroup, GroupError> {
    let mut out = SpecialAttrGroup {
        vnum,
        ..SpecialAttrGroup::default()
    };
    for k in 1..=MAX_ROW {
        let Some(row) = group.get(row_key(k).as_bytes()) else {
            break;
        };
        if row.len() < 2 {
            return Err(GroupError::ShortRow {
                group: group.name.clone(),
                key: row_key(k).into_bytes(),
                fields: row.len(),
            });
        }
        // `str_to_number` into a `DWORD`, so a value above `i32::MAX` wraps here as it does there.
        let mut apply_type = str_to_u32(&row[0]);
        if apply_type == 0 {
            apply_type = fn_get_apply_type(&row[0]);
            if apply_type == 0 {
                return Err(GroupError::UnknownApply {
                    group: group.name.clone(),
                    name: row[0].clone(),
                });
            }
        }
        let value = str_to_i32(&row[1]);
        if apply_type > MAX_APPLY_NUM {
            return Err(GroupError::ApplyOutOfRange {
                group: group.name.clone(),
                apply_type,
            });
        }
        out.attrs.push((apply_type, value));
    }
    if let Some(effect) = group.get(b"effect").and_then(|v| v.first()) {
        out.effect_file.clone_from(effect);
    }
    Ok(out)
}

/// The low four bytes of a value, which is what every legacy narrowing cast keeps.
///
/// `(DWORD) x` and `(int) x` are truncations, not conversions: a field past 32 bits keeps its low
/// 32 bits. The bytes are taken apart rather than cast so the truncation is stated instead of
/// hidden, and so a reader cannot "fix" it by accident.
fn low_4(value: u64) -> [u8; 4] {
    let [a, b, c, d, ..] = value.to_le_bytes();
    [a, b, c, d]
}

/// `(DWORD) value` for a signed `value`: the bits are kept, so a negative count becomes a large
/// `DWORD`. Legacy's `GetAttrVnum` returns a `DWORD` from an `int` count
/// (`item_manager.h:206-218`), which is why a negative count is not 0 here.
fn widen_dword(value: i32) -> u32 {
    let [a, b, c, d] = value.to_le_bytes();
    u32::from_le_bytes([a, b, c, d])
}

/// The decimal row key, without legacy's `char[4]` truncation.
fn row_key(k: usize) -> String {
    k.to_string()
}

/// `std::map::insert`: a key already present keeps its first value.
fn insert_first<V>(map: &mut BTreeMap<i32, V>, key: i32, value: V) {
    map.entry(key).or_insert(value);
}

/// The `insert` for the attribute map, which has the same first-wins rule.
fn insert_first_attr(map: &mut BTreeMap<i32, SpecialAttrGroup>, key: i32, value: SpecialAttrGroup) {
    map.entry(key).or_insert(value);
}

/// `str_to_number` into an `int` (`server/server/common/utils.h:44-50`): `(int) strtol(in, 0, 10)`.
///
/// The function itself returns false and leaves the output alone for an empty string, but every
/// call site in the reader ignores the result and its own local already starts at 0
/// (`item_manager_read_tables.cpp:263-271`), so an empty field reads 0 and that is what this
/// answers. The cast **truncates**: a field past `i32` keeps its low 32 bits, so `9999999999`
/// reads 1410065407. `strtol` skips leading whitespace, takes an optional sign, and stops at the
/// first character that is not a decimal digit, so `0x1F` reads 0 and `007` reads 7.
fn str_to_i32(bytes: &[u8]) -> i32 {
    let (magnitude, negative) = scan_decimal(bytes);
    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    i32::from_le_bytes(low_4(value))
}

/// `str_to_number` into a `DWORD` (`server/server/common/utils.h:52-58`):
/// `(unsigned int) strtoul(in, 0, 10)`.
///
/// The same scan, but `strtoul` **negates** a signed value, so `-1` reads 4294967295 where
/// [`str_to_i32`] reads -1, and the narrowing keeps the low 32 bits either way. The two are kept
/// apart because the reader uses one for a vnum and the other for an apply type, and the
/// asymmetry is what the legacy file does.
fn str_to_u32(bytes: &[u8]) -> u32 {
    let (magnitude, negative) = scan_decimal(bytes);
    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    u32::from_le_bytes(low_4(value))
}

/// The shared `strtol`/`strtoul` front end: skip whitespace, take an optional sign, and read
/// decimal digits, stopping at the first character that is not one.
///
/// Returns the magnitude and whether it was signed. A string with no digits at all gives 0, which
/// is what `strtol` answers for `abc` and for a run of spaces. Past 19 digits `strtol` saturates
/// at the `long` limits and the narrowing then wraps, so the magnitude saturates here too.
fn scan_decimal(bytes: &[u8]) -> (u64, bool) {
    let text = String::from_utf8_lossy(bytes);
    let raw = text
        .trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
        .as_bytes();
    let (mut at, negative) = match raw.first() {
        Some(b'+') => (1, false),
        Some(b'-') => (1, true),
        _ => (0, false),
    };
    let digits = at;
    let mut magnitude: u64 = 0;
    let mut saturated = false;
    while at < raw.len() && raw[at].is_ascii_digit() {
        if !saturated {
            // Past 19 digits `strtol` saturates at `LONG_MAX`, and the cast to 32 bits makes the
            // exact saturation value invisible, so any saturated value gives the same result.
            if let Some(next) = magnitude
                .checked_mul(10)
                .and_then(|m| m.checked_add(u64::from(raw[at] - b'0')))
            {
                magnitude = next;
            } else {
                saturated = true;
            }
        }
        at += 1;
    }
    if at == digits {
        return (0, false);
    }
    (magnitude, negative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_proto::parse as parse_item_proto;

    fn owners_special_group() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../legacy/gamedata/locale/europe/special_item_group.txt");
        std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
    }

    fn owners_protos() -> ItemProtos {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        let proto = std::fs::read(dir.join("item_proto.txt")).expect("item_proto.txt");
        let names = std::fs::read(dir.join("item_names.txt")).expect("item_names.txt");
        parse_item_proto(&proto, &names).expect("the owner's item proto loads")
    }

    fn owners() -> SpecialItemGroups {
        read(&owners_special_group(), &owners_protos()).expect("the owner's groups load")
    }

    /// A file with one group whose rows are `name count weight`, and nothing else.
    fn one_group(body: &str) -> Vec<u8> {
        format!("group g\n{{\n\tvnum 50011\n{body}\n}}\n").into_bytes()
    }

    #[test]
    fn the_owners_file_is_one_group_of_twenty_nine_rows() {
        let groups = owners();
        assert_eq!(
            groups.special.len(),
            1,
            "only the one group, and no quest or attr bag"
        );
        assert!(groups.quest.is_empty());
        assert!(groups.attr.is_empty());
        assert!(groups.quest_npc_vnums.is_empty());
        let bag = &groups.special[&50011];
        assert_eq!(
            bag.group_type,
            GroupType::Normal,
            "the file writes no type key"
        );
        assert_eq!(bag.items.len(), 29, "the file has 29 numbered rows");
        assert_eq!(bag.totals.len(), 29);
        assert!(!bag.weights_overflow());
        assert!(!bag.duplicate_tail_rows());
    }

    #[test]
    fn the_group_name_is_the_lowercased_file_name() {
        let groups = owners();
        assert!(
            groups.special.contains_key(&50011),
            "the key is the vnum, not the name, and the name is not kept by legacy"
        );
    }

    #[test]
    fn the_owners_rows_are_the_files_own_vnums_counts_and_weights() {
        let bag = &owners().special[&50011];
        // The first three rows of the file, with the weights as running totals.
        assert_eq!(bag.items[0].vnum, 71084);
        assert_eq!(bag.items[0].count, 15);
        assert_eq!(bag.items[1].vnum, 50252);
        assert_eq!(bag.items[1].count, 1);
        assert_eq!(bag.items[2].vnum, 51505);
        assert_eq!(bag.items[2].count, 1);
        // The file's first four weights are 40, 75, 20, 15.
        assert_eq!(
            &bag.totals[..4],
            [40, 115, 135, 150],
            "a prefix sum, not the raw weights"
        );
        assert!(
            bag.items.iter().all(|i| i.rare == 0),
            "the file writes three columns"
        );
    }

    #[test]
    fn every_owners_vnum_exists_in_the_item_proto() {
        // This is the cross-check between two independently written readers: the special group
        // file names 29 items by vnum and legacy refuses to load the file unless every one of them
        // is in `item_proto.txt`, so all 29 must resolve.
        let protos = owners_protos();
        let bag = &owners().special[&50011];
        for item in &bag.items {
            assert!(
                protos.get(item.vnum).is_some(),
                "vnum {} is in the group file but not in item_proto.txt",
                item.vnum
            );
        }
    }

    #[test]
    fn a_group_vnum_above_the_limit_registers_no_unique_entry() {
        // The file's group vnum is 50011, which is above the 30000 limit, so nothing registers.
        assert!(owners().unique.is_empty());
    }

    #[test]
    fn a_group_below_the_limit_registers_every_row_as_unique() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t50252\t1\t60\n");
        let groups = read(&data, &protos).expect("the synthetic group loads");
        assert_eq!(groups.special[&50011].items.len(), 2);
        // The limit is on the *group* vnum, so a group of 50011 registers nothing whatever its
        // rows hold.
        assert!(groups.unique.is_empty());
        let low = b"group g\n{\n\tvnum 100\n\t1\t71084\t15\t40\n}\n";
        let groups = read(low, &protos).expect("the low-vnum group loads");
        assert_eq!(groups.unique.get(&71084), Some(&100));
    }

    #[test]
    fn a_zero_weight_row_is_dropped_and_leaves_the_total_alone() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t50252\t1\t0\n\t3\t51505\t1\t60\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(
            bag.items.len(),
            2,
            "AddItem returns before it pushes a zero weight"
        );
        assert_eq!(
            bag.totals,
            [40, 100],
            "the dropped row adds nothing to the total"
        );
    }

    #[test]
    fn a_bag_of_only_zero_weights_is_empty_and_has_no_draw() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t1\t0\n\t2\t50252\t1\t0\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert!(bag.is_empty());
        assert_eq!(bag.group_size(), 0);
        assert_eq!(
            bag.one_index(1),
            None,
            "legacy reads back() on the empty vector"
        );
    }

    #[test]
    fn a_draw_finds_the_first_total_that_is_not_below_it() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t50252\t1\t60\n\t3\t51505\t1\t20\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(bag.totals, [40, 100, 120]);
        assert_eq!(bag.one_index(1), Some(0));
        assert_eq!(
            bag.one_index(40),
            Some(0),
            "lower_bound finds the total equal to the draw"
        );
        assert_eq!(bag.one_index(41), Some(1));
        assert_eq!(bag.one_index(100), Some(1));
        assert_eq!(bag.one_index(120), Some(2));
    }

    #[test]
    fn a_pct_bag_rolls_every_row_and_any_other_bag_draws_once() {
        let protos = owners_protos();
        let data = b"group g\n{\n\tvnum 50011\n\ttype pct\n\t1\t71084\t15\t40\n\t2\t50252\t1\t60\n\t3\t51505\t1\t20\n}\n";
        let bag = &read(data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(bag.group_type, GroupType::Pct);
        // The three weights are 40, 60, and 20, so the totals are 40, 100, and 120 and each row
        // is rolled against its own weight rather than against a total.
        assert_eq!(
            bag.multi_index(&[10, 10, 10]),
            [0, 1, 2],
            "every roll is inside its weight"
        );
        assert_eq!(
            bag.multi_index(&[50, 10]),
            [1],
            "row 0 misses on 50, row 1 hits on 10"
        );
        assert_eq!(
            bag.multi_index(&[41, 61, 21]),
            [],
            "a roll one past every weight misses every row"
        );
        assert_eq!(
            bag.multi_index(&[40, 60, 20]),
            [0, 1, 2],
            "a roll equal to the weight hits"
        );
        assert_eq!(bag.multi_index(&[]), [], "no roll, no row");
        let plain =
            b"group g\n{\n\tvnum 50011\n\t1\t71084\t15\t40\n\t2\t50252\t1\t60\n\t3\t51505\t1\t20\n}\n";
        let bag = &read(plain, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(
            bag.multi_index(&[10, 50, 10]),
            [0],
            "a normal bag draws once"
        );
    }

    #[test]
    fn a_quest_bag_goes_to_the_quest_map_and_registers_its_vnum() {
        let protos = owners_protos();
        let data = b"group g\n{\n\tvnum 50011\n\ttype quest\n\t1\t71084\t15\t40\n}\n";
        let groups = read(data, &protos).expect("the synthetic group loads");
        assert!(groups.special.is_empty());
        assert!(groups.quest.contains_key(&50011));
        assert_eq!(groups.quest_npc_vnums, [50011]);
    }

    #[test]
    fn the_type_is_matched_after_lowercasing_and_anything_else_is_normal() {
        let protos = owners_protos();
        // `stl_lowers` runs on the type before it is compared, so the case of the file is free.
        for word in ["pct", "Pct", "PCT"] {
            let data =
                format!("group g\n{{\n\tvnum 50011\n\ttype {word}\n\t1\t71084\t15\t40\n}}\n");
            let bag = &read(data.as_bytes(), &protos)
                .expect("the group loads")
                .special[&50011];
            assert_eq!(
                bag.group_type,
                GroupType::Pct,
                "type {word:?} is the pct name"
            );
        }
        // A trailing space is the delimiter, not part of the value, so it does not hide the name.
        let data = b"group g\n{\n\tvnum 50011\n\ttype pct \n\t1\t71084\t15\t40\n}\n";
        let bag = &read(data, &protos).expect("the group loads").special[&50011];
        assert_eq!(
            bag.group_type,
            GroupType::Pct,
            "the delimiter is not part of the value"
        );
        for word in ["weird", "", "pc", "pctt", "pcts", "pctx"] {
            let data =
                format!("group g\n{{\n\tvnum 50011\n\ttype {word}\n\t1\t71084\t15\t40\n}}\n");
            let bag = &read(data.as_bytes(), &protos)
                .expect("the group loads")
                .special[&50011];
            assert_eq!(
                bag.group_type,
                GroupType::Normal,
                "type {word:?} is not a name"
            );
        }
    }

    #[test]
    fn a_group_without_a_vnum_is_refused() {
        let protos = owners_protos();
        let data = b"group g\n{\n\ttype pct\n\t1\t71084\t15\t40\n}\n";
        assert_eq!(
            read(data, &protos),
            Err(GroupError::NoVnum {
                group: b"g".to_vec()
            })
        );
    }

    #[test]
    fn a_vnum_token_that_is_not_a_number_becomes_the_group_key_zero() {
        // `GetTokenInteger` writes 0 and still answers true for a value that is not a number, so
        // the group is created under key 0 rather than refused. The rows are still resolved.
        let protos = owners_protos();
        let data = b"group g\n{\n\tvnum abc\n\t1\t71084\t15\t40\n}\n";
        let groups = read(data, &protos).expect("the group loads under key 0");
        assert_eq!(groups.special.len(), 1);
        assert!(
            groups.special.contains_key(&0),
            "the key is 0, not an error"
        );
        assert_eq!(groups.special[&0].items[0].vnum, 71084);
    }

    #[test]
    fn a_row_naming_an_absent_item_refuses_the_whole_file() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t999999\t1\t60\n");
        assert_eq!(
            read(&data, &protos),
            Err(GroupError::UnknownItem {
                group: b"g".to_vec(),
                name: b"999999".to_vec(),
                vnum: 999_999,
            })
        );
    }

    #[test]
    fn a_row_of_two_fields_is_refused_rather_than_read_past_the_end() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\n");
        assert_eq!(
            read(&data, &protos),
            Err(GroupError::ShortRow {
                group: b"g".to_vec(),
                key: b"1".to_vec(),
                fields: 2,
            })
        );
    }

    #[test]
    fn a_fourth_column_is_the_rare_pct_and_a_three_column_row_has_none() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t1\t40\t90\n\t2\t50252\t1\t60\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(bag.items[0].rare, 90);
        assert_eq!(bag.items[1].rare, 0);
    }

    #[test]
    fn a_keyword_row_resolves_to_its_small_number_and_never_needs_the_proto() {
        let protos = owners_protos();
        for (word, give) in [
            (&b"exp"[..], GiveType::Exp),
            (b"mob", GiveType::Mob),
            (b"slow", GiveType::Slow),
            (b"drain_hp", GiveType::DrainHp),
            (b"poison", GiveType::Poison),
            (b"group", GiveType::MobGroup),
            (EXP_KOREAN, GiveType::Exp),
        ] {
            // The token is spliced in as raw bytes: the Korean keyword is a legacy byte string
            // and nothing here is allowed to transcode it.
            let shown = String::from_utf8_lossy(word);
            let mut data = b"group g\n{\n\tvnum 50011\n\t1\t".to_vec();
            data.extend_from_slice(word);
            data.extend_from_slice(b"\t1\t40\n}\n");
            let bag = &read(&data, &protos).expect("the keyword row loads").special[&50011];
            assert_eq!(bag.items[0].vnum, give as u32, "{shown:?} is a GiveType");
            assert_eq!(GiveType::from_vnum(bag.items[0].vnum), Some(give));
        }
    }

    #[test]
    fn a_keyword_is_matched_by_bytes_so_an_upper_case_row_is_not_a_keyword() {
        // The legacy `==` is on the raw token and the loader lowercases only the key, never a
        // value, so `EXP` falls through to the vnum read and is then refused.
        let protos = owners_protos();
        for word in ["EXP", "Mob", "GOLD", "gold"] {
            let data = one_group(&format!("\t1\t{word}\t1\t40\n"));
            assert!(
                matches!(read(&data, &protos), Err(GroupError::UnknownItem { .. })),
                "{word:?} is not one of the loader's keywords"
            );
        }
    }

    #[test]
    fn gold_is_in_the_enum_but_no_row_can_hold_it() {
        assert_eq!(GiveType::Gold as u32, 1, "the enum has it at 1");
        assert_eq!(
            GiveType::from_token(b"gold"),
            None,
            "the loader has no gold keyword"
        );
        assert_eq!(GiveType::from_token(b"exp"), Some(GiveType::Exp));
    }

    #[test]
    fn an_attr_group_reads_applies_and_an_effect_file() {
        let protos = owners_protos();
        let data = b"group g\n{\n\tvnum 50011\n\ttype attr\n\t1\tMAX_HP\t100\n\t2\t1\t200\n\teffect\t50011.effect\n}\n";
        let groups = read(data, &protos).expect("the attr group loads");
        assert!(groups.special.is_empty());
        let group = &groups.attr[&50011];
        assert_eq!(group.attrs, [(1, 100), (1, 200)]);
        assert_eq!(group.effect_file, b"50011.effect");
    }

    #[test]
    fn an_attr_row_resolves_a_name_case_insensitively_and_answers_zero_for_none() {
        // `MAX_HP` is 1 and `POISON` aliases to the `_PCT` value 12.
        assert_eq!(fn_get_apply_type(b"MAX_HP"), 1);
        assert_eq!(
            fn_get_apply_type(b"max_hp"),
            1,
            "strcasecmp, so the case does not matter"
        );
        assert_eq!(fn_get_apply_type(b"POISON"), 12);
        assert_eq!(
            fn_get_apply_type(b"nope"),
            0,
            "0 is how a caller reads invalid"
        );
    }

    #[test]
    fn an_attr_row_of_one_field_is_refused() {
        let protos = owners_protos();
        let data = b"group g\n{\n\tvnum 50011\n\ttype attr\n\t1\tMAX_HP\n}\n";
        assert!(matches!(
            read(data, &protos),
            Err(GroupError::ShortRow { fields: 1, .. })
        ));
    }

    #[test]
    fn an_attr_row_above_max_apply_num_is_refused_and_the_limit_itself_is_not() {
        let protos = owners_protos();
        // The legacy test is `> MAX_APPLY_NUM`, so the limit itself is accepted.
        let data = format!("group g\n{{\n\tvnum 50011\n\ttype attr\n\t1\t{MAX_APPLY_NUM}\t5\n}}\n");
        let groups = read(data.as_bytes(), &protos).expect("the limit itself is accepted");
        assert_eq!(groups.attr[&50011].attrs, [(MAX_APPLY_NUM, 5)]);
        let data = format!(
            "group g\n{{\n\tvnum 50011\n\ttype attr\n\t1\t{}\t5\n}}\n",
            MAX_APPLY_NUM + 1
        );
        assert!(matches!(
            read(data.as_bytes(), &protos),
            Err(GroupError::ApplyOutOfRange { .. })
        ));
    }

    #[test]
    fn a_repeated_group_vnum_keeps_the_first_group() {
        let protos = owners_protos();
        let data = b"group a\n{\n\tvnum 50011\n\t1\t71084\t15\t40\n}\ngroup b\n{\n\tvnum 50011\n\t1\t50252\t1\t40\n}\n";
        let groups = read(data, &protos).expect("both groups load");
        assert_eq!(groups.special.len(), 1, "insert does not overwrite");
        assert_eq!(groups.special[&50011].items[0].vnum, 71084);
    }

    #[test]
    fn the_rows_stop_at_the_first_gap_in_the_keys() {
        // The legacy loop breaks when a key is missing, so a file cannot skip a number.
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t50252\t1\t40\n\t4\t51505\t1\t40\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(
            bag.items.len(),
            2,
            "row 4 is never reached once row 3 is missing"
        );
    }

    #[test]
    fn str_to_i32_matches_the_compiled_strtol_narrowing() {
        // Every expectation here was read out of a probe that calls the verbatim
        // `str_to_number(int&, const char*)` from `server/server/common/utils.h:44-50`.
        for (input, want) in [
            (&b"0"[..], 0),
            (b"1", 1),
            (b"-1", -1),
            (b"+7", 7),
            (b" 12", 12),
            (b"\t34", 34),
            (b"12abc", 12),
            (b"abc", 0),
            (b"0x1F", 0),
            (b"007", 7),
            (b"30000", 30000),
            (b"71084", 71084),
            (b"9999999999", 1_410_065_407),
            (b"2147483647", 2_147_483_647),
            (b"2147483648", -2_147_483_648),
            (b"-2147483648", -2_147_483_648),
            (b"-2147483649", 2_147_483_647),
            (b"4294967295", -1),
            (b"4294967296", 0),
            (b"3.9", 3),
            (b"1e3", 1),
            (b"  -5  ", -5),
            (b"5 %", 5),
            (b" ", 0),
            (b"", 0),
        ] {
            assert_eq!(str_to_i32(input), want, "int from {input:?}");
        }
    }

    #[test]
    fn str_to_u32_matches_the_compiled_strtoul_narrowing() {
        for (input, want) in [
            (&b"0"[..], 0),
            (b"1", 1),
            (b"-1", 4_294_967_295),
            (b"+7", 7),
            (b" 12", 12),
            (b"\t34", 34),
            (b"12abc", 12),
            (b"abc", 0),
            (b"0x1F", 0),
            (b"007", 7),
            (b"9999999999", 1_410_065_407),
            (b"2147483648", 2_147_483_648),
            (b"-2147483648", 2_147_483_648),
            (b"-2147483649", 2_147_483_647),
            (b"4294967295", 4_294_967_295),
            (b"4294967296", 0),
            (b"3.9", 3),
            (b"1e3", 1),
            (b"  -5  ", 4_294_967_291),
            (b"5 %", 5),
            (b" ", 0),
            (b"", 0),
        ] {
            assert_eq!(str_to_u32(input), want, "DWORD from {input:?}");
        }
    }

    #[test]
    fn the_two_numeric_reads_differ_only_in_a_negative_value() {
        // `strtoul` negates and `strtol` does not, so the same bytes give two different values.
        // Cast to 32 bits the two agree bit for bit, which is the point: the reader needs the
        // `DWORD` because a weight and a rare percentage are stored unsigned, not because the
        // low 32 bits differ.
        for (input, signed, unsigned) in [
            (&b"-1"[..], -1, 4_294_967_295),
            (b"-42", -42, 4_294_967_254),
            (b"  -5  ", -5, 4_294_967_291),
            (b"-2147483648", -2_147_483_648, 2_147_483_648),
        ] {
            assert_eq!(str_to_i32(input), signed, "int from {input:?}");
            assert_eq!(str_to_u32(input), unsigned, "DWORD from {input:?}");
            assert_eq!(
                i32::from_ne_bytes(str_to_u32(input).to_ne_bytes()),
                str_to_i32(input),
                "the low 32 bits agree, including the sign"
            );
        }
        for input in [&b"1"[..], b"30000", b"71084", b"abc", b"", b" 12"] {
            assert_eq!(
                i32::from_ne_bytes(str_to_u32(input).to_ne_bytes()),
                str_to_i32(input),
                "{input:?} reads the same"
            );
        }
    }

    #[test]
    fn a_negative_weight_makes_the_prefix_sum_non_monotonic_and_says_so() {
        let protos = owners_protos();
        let data = one_group("\t1\t71084\t15\t40\n\t2\t50252\t1\t-10\n");
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(bag.totals, [40, 30]);
        assert!(
            bag.weights_overflow(),
            "a lower_bound over this is not a draw"
        );
    }

    #[test]
    fn a_bag_over_a_thousand_rows_reports_the_key_truncation_defect() {
        let mut body = String::new();
        for k in 1..=1000 {
            body.push_str(&format!("\t{k}\t71084\t1\t1\n"));
        }
        let data = one_group(&body);
        let protos = owners_protos();
        let bag = &read(&data, &protos)
            .expect("the synthetic group loads")
            .special[&50011];
        assert_eq!(
            bag.items.len(),
            1000,
            "the reader does not reproduce the defect"
        );
        assert!(
            bag.duplicate_tail_rows(),
            "legacy would read keys 1000 to 1023 as 100, 101, and 102 again"
        );
        let short = one_group("\t1\t71084\t1\t1\n");
        let bag = &read(&short, &owners_protos()).expect("loads").special[&50011];
        assert!(
            !bag.duplicate_tail_rows(),
            "a group with fewer than 1000 rows has no key 1000, so the loop never re-reads it"
        );
    }

    #[test]
    fn get_attr_vnum_answers_only_for_a_special_bag() {
        let mut bag = SpecialItemGroup {
            vnum: 1,
            ..SpecialItemGroup::default()
        };
        bag.add_item(71084, 5, 40, 0);
        assert_eq!(bag.attr_vnum(71084), 0, "a normal bag answers nothing");
        assert!(bag.contains(71084));
        bag.group_type = GroupType::Special;
        assert_eq!(bag.attr_vnum(71084), 5, "the row's count, not its vnum");
        assert_eq!(bag.attr_vnum(50252), 0, "an absent row answers 0");
    }

    #[test]
    fn no_carriage_return_survives_into_a_field_of_the_owners_crlf_file() {
        // The owner's file is CRLF, and `bind` cuts a `\r\n` once, so the last field of a row has
        // no carriage return in it. If `bind` were wrong every count and weight would carry a
        // trailing byte and `str_to_number` would still read the same number, so this test pins the
        // field count instead.
        let file = owners_special_group();
        let lines = crate::text_file::bind(&file);
        let data = lines[3];
        assert_eq!(data, b"\t1\t71084\t15\t40", "no CR survives into the field");
    }
}
