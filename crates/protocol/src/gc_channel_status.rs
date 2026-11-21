//! `HEADER_GC_RESPOND_CHANNELSTATUS` (210): the Channel status list.
//!
//! The answer to `HEADER_CG_STATE_CHECKER` (0xce), which the client sends in the handshake phase
//! to fill the Channel list on its login screen. The record is not in the client's
//! `PythonNetworkStream` registration table, so it has no row in [`crate::gc_inventory`].
//!
//! # Layout
//!
//! `CInputDB::RespondChannelStatus` (`server/server/game/input_db.cpp:2412-2429`) writes one
//! record in four pieces: three `BufferedPacket` calls and a closing `Packet`, which
//! `DESC::Packet` joins into one output unit:
//!
//! | offset | width | field |
//! |---|---|---|
//! | 0 | 1 | header, 210 |
//! | 1 | 4 | `int nSize`, the entry count, little-endian |
//! | 5 | 3 × count | `TChannelStatus` entries |
//! | 5 + 3 × count | 1 | `bSuccess`, always 1 |
//!
//! `TChannelStatus` (`server/server/common/tables.h:1677-1681`, inside the `#pragma pack(1)`
//! block that opens at line 345) is `{ short nPort; BYTE bStatus; }`: 3 bytes. The legacy DB
//! server encodes each entry field by field (`D/ClientManager.cpp:4455-4466`), which agrees.
//! `nPort` is a signed `short`; a port above 32767 has the same two bytes as the `u16` here.

/// `HEADER_GC_RESPOND_CHANNELSTATUS` (`server/server/game/packet.h:224`).
pub const HEADER_GC_RESPOND_CHANNELSTATUS: u8 = 210;

/// Width of one `TChannelStatus` entry.
pub const CHANNEL_STATUS_ENTRY_WIRE_SIZE: usize = 3;

/// The trailing `bSuccess` byte legacy always writes.
pub const CHANNEL_STATUS_SUCCESS: u8 = 1;

/// One `TChannelStatus` entry: a Channel port and its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelStatusEntry {
    /// Legacy `nPort`: the port the client connects to.
    pub port: u16,
    /// Legacy `bStatus`. Kept opaque here; the values are chosen above the codec.
    pub status: u8,
}

/// Encode the whole record for `entries`, in the order given.
///
/// # Panics
///
/// Panics if `entries` has more than `i32::MAX` entries, which no Channel layout can produce.
#[must_use]
pub fn encode_channel_status(entries: &[ChannelStatusEntry]) -> Vec<u8> {
    let count = i32::try_from(entries.len()).expect("a Channel status count fits an int");
    let mut out = Vec::with_capacity(6 + CHANNEL_STATUS_ENTRY_WIRE_SIZE * entries.len());
    out.push(HEADER_GC_RESPOND_CHANNELSTATUS);
    out.extend_from_slice(&count.to_le_bytes());
    for entry in entries {
        out.extend_from_slice(&entry.port.to_le_bytes());
        out.push(entry.status);
    }
    out.push(CHANNEL_STATUS_SUCCESS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_is_the_header_a_zero_count_and_the_success_byte() {
        assert_eq!(encode_channel_status(&[]), [0xd2, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn entries_follow_the_source_field_order() {
        let entries = [
            ChannelStatusEntry {
                port: 0x7533,
                status: 0x02,
            },
            ChannelStatusEntry {
                port: 0x1234,
                status: 0xab,
            },
        ];
        assert_eq!(
            encode_channel_status(&entries),
            [
                0xd2, // header
                0x02, 0x00, 0x00, 0x00, // int nSize
                0x33, 0x75, 0x02, // short nPort, BYTE bStatus
                0x34, 0x12, 0xab, //
                0x01, // bSuccess
            ]
        );
    }

    #[test]
    fn a_port_above_the_short_range_keeps_its_bytes() {
        let entry = ChannelStatusEntry {
            port: 0x8001,
            status: 1,
        };
        let bytes = encode_channel_status(&[entry]);
        assert_eq!(&bytes[5..8], [0x01, 0x80, 0x01]);
        assert_eq!(bytes.len(), 6 + CHANNEL_STATUS_ENTRY_WIRE_SIZE);
    }
}
