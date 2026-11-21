//! The event calendar's record: `TPacketGCEventInfo` followed by one `TPacketEventData` per
//! event, which `CEventManager::SendEventInfo` sends (`server/server/game/event_manager.cpp:
//! 613-646`).
//!
//! ```cpp
//! typedef struct SPacketGCEventInfo
//! {
//!     BYTE    bHeader;
//!     WORD    wSize;
//! } TPacketGCEventInfo;
//!
//! typedef struct SPacketEventData
//! {
//!     DWORD   dwID;
//!     BYTE    bType;
//!     long    startTime;
//!     long    endTime;
//!     int     iValue0;
//!     int     iValue1;
//!     bool    bCompleted;
//! } TPacketEventData;
//! ```
//!
//! (`server/server/game/packet.h:3459-3488`, under `#pragma pack(1)`.) `wSize` counts the
//! three header bytes and every entry. The widths, 3 and 22 bytes, were measured by compiling
//! the verbatim bodies under `#pragma pack(1)` with `g++ -m32 -fsyntax-only` `static_assert`s,
//! with a packed two-`long` control of 8; a 30-byte assertion failed as it should, and the
//! same file under `-m64` failed the entry and the control. `long` is 4 bytes on the legacy
//! target, so the two times are 32-bit.

/// `sizeof(TPacketGCEventInfo)`.
pub const GC_EVENT_INFO_HEADER_SIZE: usize = 3;

/// `sizeof(TPacketEventData)`.
pub const EVENT_DATA_WIRE_SIZE: usize = 22;

/// One `TPacketEventData`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventData {
    /// `dwID`: the event's row id.
    pub id: u32,
    /// `bType`: the `EEventTypes` member.
    pub event_type: u8,
    /// `startTime`: the start, in seconds after the Unix epoch.
    pub start: i32,
    /// `endTime`: the end, in seconds after the Unix epoch.
    pub end: i32,
    /// `iValue0`.
    pub value0: i32,
    /// `iValue1`.
    pub value1: i32,
    /// `bCompleted`.
    pub completed: bool,
}

impl EventData {
    /// Append the entry's 22 bytes to `out`.
    fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.id.to_le_bytes());
        out.push(self.event_type);
        out.extend_from_slice(&self.start.to_le_bytes());
        out.extend_from_slice(&self.end.to_le_bytes());
        out.extend_from_slice(&self.value0.to_le_bytes());
        out.extend_from_slice(&self.value1.to_le_bytes());
        out.push(u8::from(self.completed));
    }
}

/// Why the event calendar cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcEventInfoTooLong {
    /// How many entries were asked for.
    pub entries: usize,
}

impl std::fmt::Display for GcEventInfoTooLong {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} event entries do not fit the record's 16-bit size",
            self.entries
        )
    }
}

impl std::error::Error for GcEventInfoTooLong {}

/// The `GC_EVENT_INFO` record listing `events` in order.
///
/// # Errors
///
/// [`GcEventInfoTooLong`] when the record would pass 65535 bytes, which `wSize` cannot hold
/// (legacy truncates the size and the client misframes; more than 2978 entries).
pub fn encode_event_info(events: &[EventData]) -> Result<Vec<u8>, GcEventInfoTooLong> {
    let size = events
        .len()
        .checked_mul(EVENT_DATA_WIRE_SIZE)
        .and_then(|body| body.checked_add(GC_EVENT_INFO_HEADER_SIZE))
        .and_then(|size| u16::try_from(size).ok())
        .ok_or(GcEventInfoTooLong {
            entries: events.len(),
        })?;
    let mut out = Vec::with_capacity(usize::from(size));
    out.push(crate::gc_inventory::HEADER_GC_EVENT_INFO.value());
    out.extend_from_slice(&size.to_le_bytes());
    for event in events {
        event.encode_into(&mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{encode_event_info, EventData, GcEventInfoTooLong};

    /// The header, the size and each entry in the source field order.
    #[test]
    fn the_calendar_is_its_size_then_each_entry() {
        let event = EventData {
            id: 0x0403_0201,
            event_type: 0x05,
            start: 0x0908_0706,
            end: 0x0d0c_0b0a,
            value0: 0x1110_0f0e,
            value1: -2,
            completed: true,
        };
        assert_eq!(
            encode_event_info(&[event]),
            Ok(vec![
                0x9b, 25, 0, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x01,
            ])
        );
        assert_eq!(encode_event_info(&[]), Ok(vec![0x9b, 3, 0]));
    }

    /// 2978 entries fill the size to 65519; one more passes 65535 and is refused.
    #[test]
    fn a_calendar_past_the_size_word_is_refused() {
        let most = vec![EventData::default(); 2978];
        assert_eq!(
            encode_event_info(&most).map(|record| record.len()),
            Ok(65_519)
        );
        let over = vec![EventData::default(); 2979];
        assert_eq!(
            encode_event_info(&over),
            Err(GcEventInfoTooLong { entries: 2979 })
        );
    }
}
