//! The `HEADER_GC_MESSENGER` (74) records: the friend and team lists, the login and the
//! logout of a listed companion, and the remove-friend notice.
//!
//! # Provenance
//!
//! `server/server/game/packet.h:1877-1906`:
//!
//! ```ignore
//! enum
//! {
//!     MESSENGER_SUBHEADER_GC_TEAM_LIST,     // 0, ENABLE_MESSENGER_TEAM
//!     MESSENGER_SUBHEADER_GC_TEAM_LOGIN,    // 1
//!     MESSENGER_SUBHEADER_GC_TEAM_LOGOUT,   // 2
//!     MESSENGER_SUBHEADER_GC_LIST,          // 3
//!     MESSENGER_SUBHEADER_GC_LOGIN,         // 4
//!     MESSENGER_SUBHEADER_GC_LOGOUT,        // 5
//!     MESSENGER_SUBHEADER_GC_INVITE,        // 6, never sent
//!     MESSENGER_SUBHEADER_GC_REMOVE_FRIEND, // 7, __FIX_DELETE_FRIEND_REFRESH__
//! };
//!
//! typedef struct packet_messenger
//! {
//!     BYTE   header;
//!     WORD   size;
//!     BYTE   subheader;
//! } TPacketGCMessenger;
//! ```
//!
//! The envelope is **4 packed bytes**, and `size` counts it. Measured, not summed: 4 bytes
//! from a 32-bit compile of the verbatim struct body under `#pragma pack(1)` with the
//! repository's known-width controls (a packed two-`long` struct must be 8 bytes, which
//! fails on the 64-bit target). The i686 cross compiler (`i686-linux-gnu-g++-12`) is not
//! installed on this machine, so the probe used the native compiler's 32-bit mode.
//!
//! # The LIST entries
//!
//! `MessengerManager::SendList` (`G/messenger_manager.cpp:481-513`) writes, per companion,
//! either `TPacketGCMessengerListOnline` (`connected = 1`):
//!
//! ```ignore
//! typedef struct packet_messenger_list_online
//! {
//!     BYTE connected; // always 1
//!     BYTE length;
//! } TPacketGCMessengerListOnline;
//! ```
//!
//! or the offline twin (`connected = 0`), both **2 packed bytes** — and then the companion's
//! name bytes, as many as the length says, with no terminator
//! (`buf.write(it->c_str(), it->size())`). A list entry is therefore `2 + name.size()`
//! bytes, and the envelope's size is `4 + sum(2 + name.size())`.
//!
//! `ENABLE_MESSENGER_TEAM` is defined (`common/prodomodefines.h:155`), so the TEAM records
//! ride the same wire shapes under subheaders 0 to 2 (`packet.h:1882-1885`); their entry
//! structs (`TPacketGCMessengerTeamListOffline`/`Online`, `packet.h:1902-1919`) measure the
//! same 2 packed bytes. The same shape backs the friend list's records under subheaders 3
//! and 4 to 5 and 7.
//!
//! # The login, logout and remove-friend records
//!
//! Each is the envelope plus a `BYTE` length and the name bytes of that many
//! (`MessengerManager::SendLogin`, `G/messenger_manager.cpp:515-543`; `SendLogout`,
//! `:545-566`; the `REMOVE_FRIEND` arm of `__RemoveFromList`, `:421-451`).
//!
//! The `INVITE` subheader (6) is never sent by this tree: only the enum names it, so the
//! codec declares the byte and produces no record of it.
//!
//! # The names are raw
//!
//! Every producer writes the companion's `std::string` bytes with no conversion and no
//! terminator, so the codec carries `Vec<u8>` and bounds each name at
//! `CHARACTER_NAME_MAX_LEN` (24, `common/length.h:15`): a longer string could not fit the
//! entry's or the record's `BYTE` length, and no legacy producer can make one either.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use crate::gc_inventory::HEADER_GC_MESSENGER;

/// `sizeof(struct packet_messenger)`: the one-byte header, the size word and the subheader.
pub const GC_MESSENGER_ENVELOPE_WIRE_SIZE: usize = 4;

/// `CHARACTER_NAME_MAX_LEN` (`server/server/common/length.h:15`): the longest name a list
/// entry or a login record carries. A longer one could not fit the entry's `BYTE` length,
/// and every legacy producer writes `std::string::size()`, so none can make one.
pub const GC_MESSENGER_NAME_MAX: usize = 24;

/// `MESSENGER_SUBHEADER_GC_TEAM_LIST` (`packet.h:1882`): 0.
pub const MESSENGER_SUBHEADER_GC_TEAM_LIST: u8 = 0;
/// `MESSENGER_SUBHEADER_GC_TEAM_LOGIN` (`packet.h:1883`): 1.
pub const MESSENGER_SUBHEADER_GC_TEAM_LOGIN: u8 = 1;
/// `MESSENGER_SUBHEADER_GC_TEAM_LOGOUT` (`packet.h:1884`): 2.
pub const MESSENGER_SUBHEADER_GC_TEAM_LOGOUT: u8 = 2;
/// `MESSENGER_SUBHEADER_GC_LIST` (`packet.h:1886`): 3.
pub const MESSENGER_SUBHEADER_GC_LIST: u8 = 3;
/// `MESSENGER_SUBHEADER_GC_LOGIN` (`packet.h:1887`): 4.
pub const MESSENGER_SUBHEADER_GC_LOGIN: u8 = 4;
/// `MESSENGER_SUBHEADER_GC_LOGOUT` (`packet.h:1888`): 5.
pub const MESSENGER_SUBHEADER_GC_LOGOUT: u8 = 5;
/// `MESSENGER_SUBHEADER_GC_INVITE` (`packet.h:1889`): 6.
pub const MESSENGER_SUBHEADER_GC_INVITE: u8 = 6;
/// `MESSENGER_SUBHEADER_GC_REMOVE_FRIEND` (`packet.h:1892`): 7.
pub const MESSENGER_SUBHEADER_GC_REMOVE_FRIEND: u8 = 7;

/// One entry of a LIST record: the name and whether its owner is in the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcMessengerEntry {
    /// `connected`: `m_set_loginAccount`'s answer for this companion
    /// (`G/messenger_manager.cpp:492,503`).
    pub online: bool,
    /// The companion's name bytes, no terminator.
    pub name: Vec<u8>,
}

/// One game-to-client messenger record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcMessenger {
    /// `MESSENGER_SUBHEADER_GC_TEAM_LIST`: the GM companion list, online or not
    /// (`SendTeamList`, `G/messenger_manager.cpp:104-136`).
    TeamList(Vec<GcMessengerEntry>),
    /// `MESSENGER_SUBHEADER_GC_TEAM_LOGIN`: a GM the team list holds came into the game
    /// (`SendTeamLogin`, `:138-160`).
    TeamLogin(Vec<u8>),
    /// `MESSENGER_SUBHEADER_GC_TEAM_LOGOUT` (`SendTeamLogout`, `:162-184`).
    TeamLogout(Vec<u8>),
    /// `MESSENGER_SUBHEADER_GC_LIST`: the friend list, online or not
    /// (`SendList`, `:481-513`).
    List(Vec<GcMessengerEntry>),
    /// `MESSENGER_SUBHEADER_GC_LOGIN`: a companion that holds this owner in its list came
    /// into the game (`SendLogin`, `:515-543`).
    Login(Vec<u8>),
    /// `MESSENGER_SUBHEADER_GC_LOGOUT` (`SendLogout`, `:545-566`).
    Logout(Vec<u8>),
    /// `MESSENGER_SUBHEADER_GC_REMOVE_FRIEND`: the delete-friend refresh's own record,
    /// which names the character taken away (`G/messenger_manager.cpp:431-451`).
    RemoveFriend(Vec<u8>),
}

/// Why a messenger record could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcMessengerError {
    /// The record's size would not fit the `WORD` size field.
    Size {
        /// The size that was found.
        size: usize,
    },
    /// A name is longer than the `BYTE` length field can carry.
    Name {
        /// The name's length.
        length: usize,
    },
}

impl fmt::Display for GcMessengerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size { size } => {
                write!(
                    formatter,
                    "messenger record of {size} bytes overruns the size word"
                )
            }
            Self::Name { length } => write!(
                formatter,
                "a messenger name of {length} bytes is over the {GC_MESSENGER_NAME_MAX} a list entry carries"
            ),
        }
    }
}

impl Error for GcMessengerError {}

/// Append the 4-byte envelope (`TPacketGCMessenger`) to `out`. `size` counts the envelope.
fn envelope(out: &mut Vec<u8>, subheader: u8, size: usize) {
    out.push(HEADER_GC_MESSENGER.value());
    out.extend_from_slice(&u16::try_from(size).unwrap_or(u16::MAX).to_le_bytes());
    out.push(subheader);
}

/// Append one `connected + length + name` entry to `out`, as `SendList` writes it.
fn push_entry(out: &mut Vec<u8>, entry: &GcMessengerEntry) -> Result<(), GcMessengerError> {
    let length = u8::try_from(entry.name.len()).map_err(|_| GcMessengerError::Name {
        length: entry.name.len(),
    })?;
    out.push(u8::from(entry.online));
    out.push(length);
    out.extend_from_slice(&entry.name);
    Ok(())
}

/// Appends the whole packed record to `out`.
///
/// # Errors
///
/// Returns [`GcMessengerError::Size`] when the record would not fit the `WORD` size field,
/// and [`GcMessengerError::Name`] when a name does not fit its length byte.
pub fn encode_into(record: &GcMessenger, out: &mut Vec<u8>) -> Result<(), GcMessengerError> {
    let mut body: Vec<u8> = Vec::new();
    let (subheader, named) = match record {
        GcMessenger::TeamList(list) => {
            for item in list {
                push_entry(&mut body, item)?;
            }
            (MESSENGER_SUBHEADER_GC_TEAM_LIST, None)
        }
        GcMessenger::TeamLogin(name) => (MESSENGER_SUBHEADER_GC_TEAM_LOGIN, Some(name)),
        GcMessenger::TeamLogout(name) => (MESSENGER_SUBHEADER_GC_TEAM_LOGOUT, Some(name)),
        GcMessenger::List(list) => {
            for item in list {
                push_entry(&mut body, item)?;
            }
            (MESSENGER_SUBHEADER_GC_LIST, None)
        }
        GcMessenger::Login(name) => (MESSENGER_SUBHEADER_GC_LOGIN, Some(name)),
        GcMessenger::Logout(name) => (MESSENGER_SUBHEADER_GC_LOGOUT, Some(name)),
        GcMessenger::RemoveFriend(name) => (MESSENGER_SUBHEADER_GC_REMOVE_FRIEND, Some(name)),
    };
    if let Some(name) = named {
        let length =
            u8::try_from(name.len()).map_err(|_| GcMessengerError::Name { length: name.len() })?;
        body.push(length);
        body.extend_from_slice(name);
    }
    let size = GC_MESSENGER_ENVELOPE_WIRE_SIZE
        .checked_add(body.len())
        .ok_or(GcMessengerError::Size {
            size: GC_MESSENGER_ENVELOPE_WIRE_SIZE.saturating_add(body.len()),
        })?;
    u16::try_from(size).map_err(|_| GcMessengerError::Size { size })?;
    envelope(out, subheader, size);
    out.extend_from_slice(&body);
    Ok(())
}

/// Encodes to a fresh buffer.
///
/// # Errors
///
/// As [`encode_into`].
pub fn encode(record: &GcMessenger) -> Result<Vec<u8>, GcMessengerError> {
    let mut out = Vec::new();
    encode_into(record, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_is_four_packed_bytes() {
        let record = GcMessenger::Login(b"Alpha".to_vec());
        let bytes = encode(&record).expect("the login record encodes");
        assert_eq!(&bytes[..GC_MESSENGER_ENVELOPE_WIRE_SIZE], &[0x4a, 10, 0, 4]);
        assert_eq!(bytes.len(), GC_MESSENGER_ENVELOPE_WIRE_SIZE + 1 + 5);
    }

    #[test]
    fn the_size_word_counts_the_envelope_and_the_body() {
        // `pack.size = sizeof(TPacketGCMessenger) + sizeof(BYTE) + bLen`
        // (`G/messenger_manager.cpp:560-562`).
        let name = vec![0xaa; GC_MESSENGER_NAME_MAX];
        let bytes = encode(&GcMessenger::Login(name.clone())).expect("the record encodes");
        assert_eq!(&bytes[1..3], &29_u16.to_le_bytes());
        assert_eq!(usize::from(bytes[4]), GC_MESSENGER_NAME_MAX);
        assert_eq!(bytes.len(), 4 + 1 + GC_MESSENGER_NAME_MAX);
    }

    #[test]
    fn the_list_records_the_entries_the_legacy_writes() {
        // `SendList` writes online (1) or offline (0), then the length and the name bytes
        // with no terminator (`G/messenger_manager.cpp:499-511`).
        let record = GcMessenger::List(vec![
            GcMessengerEntry {
                online: true,
                name: b"Bob".to_vec(),
            },
            GcMessengerEntry {
                online: false,
                name: vec![0x00, 0x01],
            },
        ]);
        let bytes = encode(&record).expect("the list encodes");
        assert_eq!(bytes[0], 0x4a);
        assert_eq!(bytes[3], MESSENGER_SUBHEADER_GC_LIST);
        assert_eq!(bytes.len(), 4 + 2 + 3 + 2 + 2);
        assert_eq!(
            u16::from_le_bytes([bytes[1], bytes[2]]) as usize,
            bytes.len()
        );
        assert_eq!(&bytes[4..9], &[1, 3, b'B', b'o', b'b']);
        assert_eq!(&bytes[9..], &[0, 2, 0x00, 0x01]);
    }

    #[test]
    fn the_team_records_walk_the_lowest_subheaders() {
        let bytes = encode(&GcMessenger::TeamList(vec![GcMessengerEntry {
            online: false,
            name: b"Server".to_vec(),
        }]))
        .expect("the team list encodes");
        assert_eq!(bytes[3], MESSENGER_SUBHEADER_GC_TEAM_LIST);
        let login =
            encode(&GcMessenger::TeamLogin(b"Server".to_vec())).expect("the team login encodes");
        assert_eq!(login[3], MESSENGER_SUBHEADER_GC_TEAM_LOGIN);
        let logout =
            encode(&GcMessenger::TeamLogout(b"Server".to_vec())).expect("the team logout encodes");
        assert_eq!(logout[3], MESSENGER_SUBHEADER_GC_TEAM_LOGOUT);
    }

    #[test]
    fn a_remove_friend_record_names_the_companion() {
        let bytes =
            encode(&GcMessenger::RemoveFriend(b"Bob".to_vec())).expect("the record encodes");
        assert_eq!(bytes[3], MESSENGER_SUBHEADER_GC_REMOVE_FRIEND);
        assert_eq!(&bytes[4..], &[3, b'B', b'o', b'b']);
        assert_eq!(bytes.len(), 4 + 1 + 3);
    }

    #[test]
    fn a_name_past_the_byte_length_is_refused() {
        let bytes = encode(&GcMessenger::Login(vec![0xaa; 255])).expect("255 fits the length byte");
        assert_eq!(bytes[4], 255);
        assert!(matches!(
            encode(&GcMessenger::Login(vec![0xaa; 256])),
            Err(GcMessengerError::Name { length: 256 })
        ));
    }

    #[test]
    fn every_name_byte_survives_the_record() {
        let name: Vec<u8> = (0..=u8::MAX).collect();
        let bytes = encode(&GcMessenger::Logout(name[..u8::MAX as usize].to_vec()))
            .expect("the record encodes");
        assert_eq!(&bytes[5..], &name[..u8::MAX as usize]);
    }

    #[test]
    fn an_empty_list_record_never_exists_but_one_that_is_sent_carries_no_entry() {
        // `SendList` early-returns on an empty list
        // (`G/messenger_manager.cpp:495-499`), so the codec never has to produce one;
        // it still does, so a wrong caller is visible on the wire, not silent.
        let bytes = encode(&GcMessenger::List(vec![])).expect("the list encodes");
        assert_eq!(bytes, [0x4a, 4, 0, MESSENGER_SUBHEADER_GC_LIST]);
    }
}
