//! `HEADER_GC_QUEST_INFO` (81): what the client's quest window shows of one quest.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1868-1877`, inside the file's `#pragma pack(1)`, with
//! `__QUEST_RENEWAL__` defined (`server/server/common/prodomodefines.h:18`):
//!
//! ```ignore
//! struct packet_quest_info
//! {
//!     BYTE header;
//!     WORD size;
//!     WORD index;
//! #ifdef __QUEST_RENEWAL__
//!     WORD c_index;
//! #endif
//!     BYTE flag;
//! };
//! ```
//!
//! The width is 8. It was measured by compiling the verbatim struct body for i686 with
//! `clang++-19 -target i686-linux-gnu` under `static_assert`, beside `packet_script` (6) and a
//! packed two-`long` struct (8) as controls; an assertion of 7 failed, and the same file for
//! x86-64 failed the `long` controls.
//!
//! The producer is `quest::PC::SendQuestInfoPakcet` (`server/server/game/questpc.cpp:231-308`).
//! `flag` is the script's `m_iSendToClient`, and each of its bits appends one part after the
//! fixed eight bytes, in bit order (`QUEST_SEND_*`, `server/server/game/questpc.h:35-44`):
//!
//! | bit | part | bytes |
//! |---|---|---|
//! | 0 `ISBEGIN` | `bStart ? 1 : 0` | 1 |
//! | 1 `TITLE` | `_title` | 31 |
//! | 2 `CLOCK_NAME` | `_clock_name` | 17 |
//! | 3 `CLOCK_VALUE` | `_clock_value`, an `int` | 4 |
//! | 4 `COUNTER_NAME` | `_counter_name` | 17 |
//! | 5 `COUNTER_VALUE` | `_counter_value`, an `int` | 4 |
//! | 6 `ICON_FILE` | `_icon_file` | 25 |
//!
//! # Three legacy Defects the encoder does not reproduce
//!
//! - **The size word.** Legacy copies the struct into its buffer before it adds each part's
//!   length to `qi.size`, so every record it sends says 8, however many parts follow. The
//!   client reads the parts by `flag`, so it survives that; but its framing waits only for
//!   the eight bytes, and a part that arrives later would be read as the next record. The
//!   encoder writes the whole record's length.
//! - **The bytes past a name's end.** Legacy writes each name as `reserve(N)` and then `N`
//!   bytes from `c_str()`, so after the terminating zero it sends whatever the string's
//!   buffer held. The encoder sends zeros there.
//! - **A name as long as its field.** The same `N` bytes from `c_str()` hold no terminator
//!   when the name is `N` bytes or longer, so the client is handed a name that does not end
//!   in its field. The encoder keeps `N - 1` bytes and ends the field with a zero
//!   ([`c_field`]).

use std::fmt;

use crate::gc_inventory::HEADER_GC_QUEST_INFO;

/// `sizeof(struct packet_quest_info)`: header, size, index, category and flag.
pub const GC_QUEST_INFO_WIRE_SIZE: usize = 8;

/// `QUEST_SEND_ISBEGIN`: the begin byte follows.
pub const QUEST_SEND_ISBEGIN: u8 = 1 << 0;
/// `QUEST_SEND_TITLE`: the title follows.
pub const QUEST_SEND_TITLE: u8 = 1 << 1;
/// `QUEST_SEND_CLOCK_NAME`: the clock's name follows.
pub const QUEST_SEND_CLOCK_NAME: u8 = 1 << 2;
/// `QUEST_SEND_CLOCK_VALUE`: the clock's value follows.
pub const QUEST_SEND_CLOCK_VALUE: u8 = 1 << 3;
/// `QUEST_SEND_COUNTER_NAME`: the counter's name follows.
pub const QUEST_SEND_COUNTER_NAME: u8 = 1 << 4;
/// `QUEST_SEND_COUNTER_VALUE`: the counter's value follows.
pub const QUEST_SEND_COUNTER_VALUE: u8 = 1 << 5;
/// `QUEST_SEND_ICON_FILE`: the icon's file name follows.
pub const QUEST_SEND_ICON_FILE: u8 = 1 << 6;

/// The bits a record may carry; legacy defines no eighth part.
const QUEST_SEND_ALL: u8 = 0x7f;

/// The title's width, `30 + 1`.
pub const QUEST_TITLE_FIELD_SIZE: usize = 31;
/// The clock's and the counter's name width, `16 + 1`.
pub const QUEST_NAME_FIELD_SIZE: usize = 17;
/// The icon file's width, `24 + 1`.
pub const QUEST_ICON_FIELD_SIZE: usize = 25;

/// `HEADER_GC_QUEST_INFO` (81): a quest's number and category, and the parts of its window
/// the script changed. A part that is `None` is not on the wire, and its bit is clear.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GcQuestInfo {
    /// `index`: the quest's number.
    pub index: u16,
    /// `c_index`: the quest's category (`GetQuestCategoryByQuestIndex`).
    pub category: u16,
    /// `bStart`: whether the quest begins (`q.start`) or ends (`q.done`).
    pub begin: Option<bool>,
    /// `_title`.
    pub title: Option<[u8; QUEST_TITLE_FIELD_SIZE]>,
    /// `_clock_name`.
    pub clock_name: Option<[u8; QUEST_NAME_FIELD_SIZE]>,
    /// `_clock_value`.
    pub clock_value: Option<i32>,
    /// `_counter_name`.
    pub counter_name: Option<[u8; QUEST_NAME_FIELD_SIZE]>,
    /// `_counter_value`.
    pub counter_value: Option<i32>,
    /// `_icon_file`.
    pub icon_file: Option<[u8; QUEST_ICON_FIELD_SIZE]>,
}

/// A name in a field of `N` bytes: at most `N - 1` bytes of `text`, up to its first zero,
/// then zeros to the end of the field, so the field always ends the name (legacy's `N` bytes
/// from `c_str()` do not, for a name of `N` bytes or more).
#[must_use]
pub fn c_field<const N: usize>(text: &[u8]) -> [u8; N] {
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len())
        .min(N.saturating_sub(1));
    let mut field = [0; N];
    field[..end].copy_from_slice(&text[..end]);
    field
}

impl GcQuestInfo {
    /// The fixed header this record always carries.
    #[must_use]
    pub const fn header() -> u8 {
        HEADER_GC_QUEST_INFO.value()
    }

    /// `m_iSendToClient`: one bit for each part the record carries.
    #[must_use]
    pub fn flag(&self) -> u8 {
        let bit = |present: bool, bit: u8| if present { bit } else { 0 };
        bit(self.begin.is_some(), QUEST_SEND_ISBEGIN)
            | bit(self.title.is_some(), QUEST_SEND_TITLE)
            | bit(self.clock_name.is_some(), QUEST_SEND_CLOCK_NAME)
            | bit(self.clock_value.is_some(), QUEST_SEND_CLOCK_VALUE)
            | bit(self.counter_name.is_some(), QUEST_SEND_COUNTER_NAME)
            | bit(self.counter_value.is_some(), QUEST_SEND_COUNTER_VALUE)
            | bit(self.icon_file.is_some(), QUEST_SEND_ICON_FILE)
    }

    /// The whole record's length on the wire, which is what `size` holds.
    #[must_use]
    pub fn wire_size(&self) -> usize {
        part_sizes(self.flag()) + GC_QUEST_INFO_WIRE_SIZE
    }

    /// Appends the whole packed record to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        // The longest record is 107 bytes, so the size always fits its word.
        let size = u16::try_from(self.wire_size()).unwrap_or(u16::MAX);
        out.push(Self::header());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&self.index.to_le_bytes());
        out.extend_from_slice(&self.category.to_le_bytes());
        out.push(self.flag());
        if let Some(begin) = self.begin {
            out.push(u8::from(begin));
        }
        if let Some(title) = &self.title {
            out.extend_from_slice(title);
        }
        if let Some(name) = &self.clock_name {
            out.extend_from_slice(name);
        }
        if let Some(value) = self.clock_value {
            out.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(name) = &self.counter_name {
            out.extend_from_slice(name);
        }
        if let Some(value) = self.counter_value {
            out.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(icon) = &self.icon_file {
            out.extend_from_slice(icon);
        }
    }

    /// Decodes the whole packed record from the start of `bytes`.
    ///
    /// # Errors
    ///
    /// [`GcQuestInfoError::Truncated`] when the buffer ends before the fixed part or before
    /// the parts `flag` names, [`GcQuestInfoError::Header`] when the leading byte is not
    /// [`HEADER_GC_QUEST_INFO`], [`GcQuestInfoError::Flag`] when `flag` names a part legacy
    /// does not define, [`GcQuestInfoError::Size`] when `size` is not the length `flag`
    /// makes, and [`GcQuestInfoError::Begin`] when the begin byte is neither 0 nor 1.
    pub fn decode(bytes: &[u8]) -> Result<Self, GcQuestInfoError> {
        if bytes.len() < GC_QUEST_INFO_WIRE_SIZE {
            return Err(GcQuestInfoError::Truncated {
                needed: GC_QUEST_INFO_WIRE_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0] != Self::header() {
            return Err(GcQuestInfoError::Header {
                expected: Self::header(),
                actual: bytes[0],
            });
        }
        let flag = bytes[7];
        if flag & !QUEST_SEND_ALL != 0 {
            return Err(GcQuestInfoError::Flag { flag });
        }
        let size = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
        let wanted = GC_QUEST_INFO_WIRE_SIZE + part_sizes(flag);
        if size != wanted {
            return Err(GcQuestInfoError::Size { size, wanted });
        }
        if bytes.len() < size {
            return Err(GcQuestInfoError::Truncated {
                needed: size,
                actual: bytes.len(),
            });
        }
        let mut parts = Parts {
            bytes: &bytes[GC_QUEST_INFO_WIRE_SIZE..size],
            flag,
        };
        let begin = match parts.take::<1>(QUEST_SEND_ISBEGIN) {
            None => None,
            Some([0]) => Some(false),
            Some([1]) => Some(true),
            Some([byte]) => return Err(GcQuestInfoError::Begin { byte }),
        };
        Ok(Self {
            index: u16::from_le_bytes([bytes[3], bytes[4]]),
            category: u16::from_le_bytes([bytes[5], bytes[6]]),
            begin,
            title: parts.take(QUEST_SEND_TITLE),
            clock_name: parts.take(QUEST_SEND_CLOCK_NAME),
            clock_value: parts.take(QUEST_SEND_CLOCK_VALUE).map(i32::from_le_bytes),
            counter_name: parts.take(QUEST_SEND_COUNTER_NAME),
            counter_value: parts.take(QUEST_SEND_COUNTER_VALUE).map(i32::from_le_bytes),
            icon_file: parts.take(QUEST_SEND_ICON_FILE),
        })
    }
}

/// The bytes the parts `flag` names take after the fixed eight.
fn part_sizes(flag: u8) -> usize {
    [
        (QUEST_SEND_ISBEGIN, 1),
        (QUEST_SEND_TITLE, QUEST_TITLE_FIELD_SIZE),
        (QUEST_SEND_CLOCK_NAME, QUEST_NAME_FIELD_SIZE),
        (QUEST_SEND_CLOCK_VALUE, 4),
        (QUEST_SEND_COUNTER_NAME, QUEST_NAME_FIELD_SIZE),
        (QUEST_SEND_COUNTER_VALUE, 4),
        (QUEST_SEND_ICON_FILE, QUEST_ICON_FIELD_SIZE),
    ]
    .iter()
    .filter(|(bit, _)| flag & bit != 0)
    .map(|(_, size)| size)
    .sum()
}

/// The parts after the fixed eight bytes, read in bit order.
struct Parts<'b> {
    bytes: &'b [u8],
    flag: u8,
}

impl Parts<'_> {
    /// The next `N` bytes when `flag` carries `bit`; the size check has already proved
    /// they are there.
    fn take<const N: usize>(&mut self, bit: u8) -> Option<[u8; N]> {
        if self.flag & bit == 0 {
            return None;
        }
        let (part, rest) = self.bytes.split_at_checked(N)?;
        self.bytes = rest;
        part.try_into().ok()
    }
}

/// What a [`GcQuestInfo`] decode can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcQuestInfoError {
    /// The buffer ended before the record did.
    Truncated {
        /// Bytes the record needs.
        needed: usize,
        /// Bytes the buffer actually had.
        actual: usize,
    },
    /// The leading byte is not this record's header.
    Header {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was present.
        actual: u8,
    },
    /// `flag` names a part past the seven legacy defines.
    Flag {
        /// The flag the record carried.
        flag: u8,
    },
    /// `size` is not the length the parts `flag` names make.
    Size {
        /// The `size` the record carried.
        size: usize,
        /// The length its `flag` makes.
        wanted: usize,
    },
    /// The begin byte is neither 0 nor 1.
    Begin {
        /// The byte the record carried.
        byte: u8,
    },
}

impl fmt::Display for GcQuestInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, actual } => {
                write!(f, "GcQuestInfo: need {needed} bytes, buffer held {actual}")
            }
            Self::Header { expected, actual } => write!(
                f,
                "GcQuestInfo: header {actual:#04x}, expected {expected:#04x}"
            ),
            Self::Flag { flag } => {
                write!(
                    f,
                    "GcQuestInfo: flag {flag:#04x} names a part past the seventh"
                )
            }
            Self::Size { size, wanted } => {
                write!(f, "GcQuestInfo: size {size}, but its flag makes {wanted}")
            }
            Self::Begin { byte } => write!(f, "GcQuestInfo: begin byte {byte} is not 0 or 1"),
        }
    }
}

impl std::error::Error for GcQuestInfoError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A title field from the source order: the text, then zeros to 31.
    fn padded<const N: usize>(text: &[u8]) -> Vec<u8> {
        let mut field = text.to_vec();
        field.resize(N, 0);
        field
    }

    #[test]
    fn the_letters_title_is_the_header_eight_bytes_and_31_title_bytes() {
        let info = GcQuestInfo {
            index: 0x0128,
            category: 0x0304,
            title: Some(c_field(b"Sixth and Seventh Bonus")),
            ..GcQuestInfo::default()
        };
        let mut want = vec![81, 39, 0, 0x28, 0x01, 0x04, 0x03, 0b10];
        want.extend(padded::<31>(b"Sixth and Seventh Bonus"));
        let mut out = Vec::new();
        info.encode_into(&mut out);
        assert_eq!(out, want);
        assert_eq!(info.wire_size(), 39);
        assert_eq!(GcQuestInfo::decode(&want), Ok(info));
    }

    #[test]
    fn every_part_follows_in_bit_order() {
        let info = GcQuestInfo {
            index: 0x1234,
            category: 0x5678,
            begin: Some(true),
            title: Some(c_field(b"Title")),
            clock_name: Some(c_field(b"Clock")),
            clock_value: Some(0x0102_0304),
            counter_name: Some(c_field(b"Count")),
            counter_value: Some(-2),
            icon_file: Some(c_field(b"icon.tga")),
        };
        let mut want = vec![81, 107, 0, 0x34, 0x12, 0x78, 0x56, 0x7f, 1];
        want.extend(padded::<31>(b"Title"));
        want.extend(padded::<17>(b"Clock"));
        want.extend([0x04, 0x03, 0x02, 0x01]);
        want.extend(padded::<17>(b"Count"));
        want.extend([0xfe, 0xff, 0xff, 0xff]);
        want.extend(padded::<25>(b"icon.tga"));
        assert_eq!(want.len(), 107);
        let mut out = Vec::new();
        info.encode_into(&mut out);
        assert_eq!(out, want);
        assert_eq!(GcQuestInfo::decode(&want), Ok(info));
    }

    #[test]
    fn the_done_byte_is_zero_and_a_bare_record_is_eight_bytes() {
        let done = GcQuestInfo {
            index: 7,
            begin: Some(false),
            ..GcQuestInfo::default()
        };
        let mut out = Vec::new();
        done.encode_into(&mut out);
        assert_eq!(out, [81, 9, 0, 7, 0, 0, 0, 0b1, 0]);
        let mut bare = Vec::new();
        GcQuestInfo::default().encode_into(&mut bare);
        assert_eq!(bare, [81, 8, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_field_keeps_one_byte_short_of_its_width_up_to_the_first_zero() {
        assert_eq!(c_field::<4>(b"abcdef"), *b"abc\0");
        assert_eq!(c_field::<4>(b"a\0bc"), *b"a\0\0\0");
        assert_eq!(c_field::<4>(b""), [0; 4]);
        let title: [u8; 31] = c_field(&[b'x'; 40]);
        assert_eq!(&title[..30], &[b'x'; 30][..]);
        assert_eq!(title[30], 0);
    }

    #[test]
    fn a_decode_refuses_what_legacy_never_sends() {
        let mut good = Vec::new();
        GcQuestInfo {
            begin: Some(true),
            ..GcQuestInfo::default()
        }
        .encode_into(&mut good);
        assert_eq!(
            GcQuestInfo::decode(&good[..7]),
            Err(GcQuestInfoError::Truncated {
                needed: 8,
                actual: 7
            })
        );
        assert_eq!(
            GcQuestInfo::decode(&good[..8]),
            Err(GcQuestInfoError::Truncated {
                needed: 9,
                actual: 8
            })
        );
        let mut header = good.clone();
        header[0] = 45;
        assert_eq!(
            GcQuestInfo::decode(&header),
            Err(GcQuestInfoError::Header {
                expected: 81,
                actual: 45
            })
        );
        let mut flag = good.clone();
        flag[7] = 0x81;
        assert_eq!(
            GcQuestInfo::decode(&flag),
            Err(GcQuestInfoError::Flag { flag: 0x81 })
        );
        // Legacy's own size word: 8 for a record that carries a part.
        let mut legacy = good.clone();
        legacy[1] = 8;
        assert_eq!(
            GcQuestInfo::decode(&legacy),
            Err(GcQuestInfoError::Size { size: 8, wanted: 9 })
        );
        let mut begin = good;
        begin[8] = 2;
        assert_eq!(
            GcQuestInfo::decode(&begin),
            Err(GcQuestInfoError::Begin { byte: 2 })
        );
    }

    #[test]
    fn each_bit_alone_sizes_its_own_part() {
        for (bit, size) in [
            (QUEST_SEND_ISBEGIN, 1),
            (QUEST_SEND_TITLE, 31),
            (QUEST_SEND_CLOCK_NAME, 17),
            (QUEST_SEND_CLOCK_VALUE, 4),
            (QUEST_SEND_COUNTER_NAME, 17),
            (QUEST_SEND_COUNTER_VALUE, 4),
            (QUEST_SEND_ICON_FILE, 25),
        ] {
            assert_eq!(part_sizes(bit), size, "bit {bit:#04x}");
            let mut record = vec![81, 0, 0, 0, 0, 0, 0, bit];
            record.resize(8 + size, 0);
            record[1] = u8::try_from(8 + size).expect("short");
            let decoded = GcQuestInfo::decode(&record).expect("one part");
            assert_eq!(decoded.flag(), bit);
        }
    }
}
