//! Incremental framing for source-verified variable-length client packets.
//!
//! The fixed [`crate::cg_wire::ClientFrameDecoder`] deliberately stops at the
//! packet-info prefix.  The legacy `CInputProcessor` then calls the active
//! input processor's `Analyze` method, which reads a length or count and
//! extends the frame.  This module implements that framing step for the
//! client packets whose length is unambiguous in the active x86 source build:
//! chat, whisper, sync-position, my-shop, shop, fish-event, messenger, and
//! guild-symbol prefixes. It does not execute commands, resolve players, or
//! reproduce any gameplay side effects.
//!
//! The guild-symbol layout is transcribed from
//! `server/server/game/packet.h:2272-2277` and
//! `client/Client/UserInterface/Packet.h:361-366`: one header byte, an explicit
//! little-endian 16-bit complete size, and an explicit little-endian 32-bit
//! guild ID, followed by opaque symbol bytes. The legacy login input path only
//! admits this header in `PHASE_LOGIN`, `PHASE_SELECT`, and `PHASE_LOADING`
//! (`server/server/game/desc.cpp:494-542`). This module is phase-neutral and
//! does not grant that admission; the handshake dispatcher remains a separate
//! fail-closed boundary.

use std::error::Error;
use std::fmt;

use crate::cg_inventory::{
    HEADER_CG_CHAT, HEADER_CG_FISH_EVENT_SEND, HEADER_CG_GUILD_SYMBOL_UPLOAD, HEADER_CG_MESSENGER,
    HEADER_CG_MYSHOP, HEADER_CG_PRIVATE_SHOP, HEADER_CG_SHOP, HEADER_CG_SYNC_POSITION,
    HEADER_CG_WHISPER,
};
use crate::cg_wire::{
    resolve_client_frame_size, ClientFrame, ClientFrameError, ClientFrameSize,
    CLIENT_FRAME_HEADER_SIZE, DEFAULT_MAX_CLIENT_FRAME_SIZE,
};

/// Wire size of one `TPacketCGSyncPositionElement` in the active packed x86
/// build: `DWORD`, `long`, and `long` fields without host-layout assumptions.
pub const SYNC_POSITION_ELEMENT_WIRE_SIZE: usize = 12;

/// Wire size of one active x86 `TShopItemTable` record.
///
/// The count-based my-shop extension is only enabled when the corresponding
/// packet prefix is the inventory's 35-byte active profile.  Keeping this value
/// explicit prevents a Rust `size_of` or a host `long` from changing the wire
/// contract.
pub const MY_SHOP_ITEM_WIRE_SIZE: usize = 68;

/// Active x86 `TPacketCGMyShop` prefix: header, 33-byte sign, and count.
pub const MY_SHOP_BASE_WIRE_SIZE: usize = 35;

/// Active x86 `TPacketCGFishEvent` prefix: one header and one subheader.
pub const FISH_EVENT_BASE_WIRE_SIZE: usize = 2;

/// Active x86 `TPacketCGShop` prefix: one header and one subheader.
pub const SHOP_BASE_WIRE_SIZE: usize = 2;

/// Active x86 `TPacketCGMessenger` prefix: one header and one subheader.
pub const MESSENGER_BASE_WIRE_SIZE: usize = 2;

/// Active packed x86 `TPacketCGGuildSymbolUpload` prefix: header, `WORD` size,
/// and `DWORD` guild ID.
pub const GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE: usize = 7;

/// Maximum raw symbol bytes accepted by the legacy guild-symbol handler.
///
/// The source stores the complete size in a `WORD`, so the representable wire
/// maximum is smaller than this semantic limit; both checks remain explicit.
pub const GUILD_SYMBOL_MAX_SOURCE_BYTES: usize = 64 * 1024;

/// `MESSENGER_ADD_BY_VID` extension: one packed 32-bit virtual ID.
pub const MESSENGER_ADD_BY_VID_EXTRA_WIRE_SIZE: usize = 4;

/// `MESSENGER_ADD_BY_NAME` and `MESSENGER_REMOVE` extension: the raw
/// 24-byte character-name field. The legacy handler does not interpret or
/// validate the field during framing.
pub const MESSENGER_NAME_EXTRA_WIRE_SIZE: usize = 24;

/// Legacy messenger subheader for adding a virtual ID.
pub const MESSENGER_SUBHEADER_ADD_BY_VID: u8 = 0;

/// Legacy messenger subheader for adding a character name.
pub const MESSENGER_SUBHEADER_ADD_BY_NAME: u8 = 1;

/// Legacy messenger subheader for removing a character name.
pub const MESSENGER_SUBHEADER_REMOVE: u8 = 2;

/// Legacy shop subheader for ending a shopping session.
pub const SHOP_SUBHEADER_END: u8 = 0;

/// Legacy shop subheader for buying.
pub const SHOP_SUBHEADER_BUY: u8 = 1;

/// Legacy shop subheader for selling one item.
pub const SHOP_SUBHEADER_SELL: u8 = 2;

/// Legacy shop subheader for selling a counted stack.
pub const SHOP_SUBHEADER_SELL2: u8 = 3;

/// Legacy fish-event subheader for using a box item.
pub const FISH_EVENT_SUBHEADER_BOX_USE: u8 = 0;

/// Legacy fish-event subheader for adding a shape.
pub const FISH_EVENT_SUBHEADER_SHAPE_ADD: u8 = 1;

/// Variable client headers covered by this source-verified resolver.
pub const RESOLVABLE_VARIABLE_CLIENT_HEADERS: &[u8] = &[
    HEADER_CG_CHAT.value(),
    HEADER_CG_WHISPER.value(),
    HEADER_CG_SYNC_POSITION.value(),
    HEADER_CG_MYSHOP.value(),
    HEADER_CG_SHOP.value(),
    HEADER_CG_GUILD_SYMBOL_UPLOAD.value(),
    HEADER_CG_FISH_EVENT_SEND.value(),
    HEADER_CG_MESSENGER.value(),
    HEADER_CG_PRIVATE_SHOP.value(),
];

// ---------------------------------------------------------------------------
// Private shop: a third sub-header-routed two-byte prefix
// ---------------------------------------------------------------------------
//
// `HEADER_CG_PRIVATE_SHOP = 236` at `packet.h:89` is routed on a *subheader*,
// not on a length, exactly like `HEADER_CG_SHOP` and
// `HEADER_CG_FISH_EVENT_SEND`. Its packet-table entry is
// `sizeof(TPacketCGPrivateShop)` at `packet_info.cpp:186`, which is two bytes:
// a `BYTE bHeader` and a `BYTE bSubHeader` at `packet.h:1226-1230`. The
// sub-record that follows is sized by the subheader alone.
//
// That is why a generic framer cannot handle this header. `input.cpp:83`
// resolves the registered length, `:92-93` checks only that many bytes are
// present, and only then does `:100-104` call `Analyze` and apply
// `iPacketLen += iExtraPacketSize`. The sub-record size is therefore unknown
// until `bSubHeader` has been read, which is precisely what the resolver below
// models.

/// Active x86 `TPacketCGPrivateShop` prefix: one header and one subheader.
pub const PRIVATE_SHOP_BASE_WIRE_SIZE: usize = 2;

/// Active x86 `TPacketCGPrivateShopBuild` fixed part: a 33-byte title, a
/// `DWORD` poly vnum, a title-type byte, a page-count byte, and a `WORD`
/// item count. `packet.h:1256-1263`.
pub const PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE: usize = 41;

/// Active x86 `TPrivateShopItem` element: a `TItemPos`, a `TItemPrice`, and a
/// display position word. `packet.h:1163-1168`.
pub const PRIVATE_SHOP_ITEM_WIRE_SIZE: usize = 17;

/// Wire offset of the little-endian `wItemCount` word inside a private-shop
/// build frame: the two base bytes, then 39 bytes into the sub-record, which
/// ends at `szTitle[33] + dwPolyVnum[4] + bTitleType[1] + bPageCount[1]`.
pub const PRIVATE_SHOP_BUILD_COUNT_OFFSET: usize =
    PRIVATE_SHOP_BASE_WIRE_SIZE + PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE - 2;

/// Legacy private-shop subheader for building a shop.
pub const PRIVATE_SHOP_SUBHEADER_BUILD: u8 = 0;
/// Legacy private-shop subheader for closing a shop.
pub const PRIVATE_SHOP_SUBHEADER_CLOSE: u8 = 1;
/// Legacy private-shop subheader for opening the shop panel.
pub const PRIVATE_SHOP_SUBHEADER_PANEL_OPEN: u8 = 2;
/// Legacy private-shop subheader for closing the shop panel.
pub const PRIVATE_SHOP_SUBHEADER_PANEL_CLOSE: u8 = 3;
/// Legacy private-shop subheader for starting a sale.
pub const PRIVATE_SHOP_SUBHEADER_START: u8 = 4;
/// Legacy private-shop subheader for ending a sale.
pub const PRIVATE_SHOP_SUBHEADER_END: u8 = 5;
/// Legacy private-shop subheader for buying.
pub const PRIVATE_SHOP_SUBHEADER_BUY: u8 = 6;
/// Legacy private-shop subheader for withdrawing an item.
pub const PRIVATE_SHOP_SUBHEADER_WITHDRAW: u8 = 7;
/// Legacy private-shop subheader for entering the modify state.
pub const PRIVATE_SHOP_SUBHEADER_MODIFY: u8 = 8;
/// Legacy private-shop subheader for a state update.
///
/// This enumerator is declared at `packet.h:1146` but the dispatch at
/// `input_main.cpp:3961-4081` has **no case arm** for it, so it falls to
/// `default:` at `:4078-4080`, which logs `Unknown private shop subheader` and
/// consumes the bare two-byte prefix. It is modelled as a zero-extension
/// subheader because that is what the source does. No payload is invented for
/// it.
pub const PRIVATE_SHOP_SUBHEADER_STATE_UPDATE: u8 = 9;
/// Legacy private-shop subheader for changing an item's price.
pub const PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE: u8 = 10;
/// Legacy private-shop subheader for moving an item slot.
pub const PRIVATE_SHOP_SUBHEADER_ITEM_MOVE: u8 = 11;
/// Legacy private-shop subheader for checking an item in.
pub const PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN: u8 = 12;
/// Legacy private-shop subheader for checking an item out.
pub const PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT: u8 = 13;
/// Legacy private-shop subheader for changing the shop title.
pub const PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE: u8 = 14;
/// Legacy private-shop subheader for a warp request.
pub const PRIVATE_SHOP_SUBHEADER_WARP_REQUEST: u8 = 15;
/// Legacy private-shop subheader for unlocking a shop slot.
pub const PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST: u8 = 16;
/// Legacy private-shop subheader for closing the search window.
pub const PRIVATE_SHOP_SUBHEADER_SEARCH_CLOSE: u8 = 17;
/// Legacy private-shop subheader for running a search.
pub const PRIVATE_SHOP_SUBHEADER_SEARCH: u8 = 18;
/// Legacy private-shop subheader for buying a search result.
pub const PRIVATE_SHOP_SUBHEADER_SEARCH_BUY: u8 = 19;
/// Legacy private-shop subheader for requesting market item price data.
pub const PRIVATE_SHOP_SUBHEADER_MARKET_ITEM_PRICE_DATA_REQUEST: u8 = 20;
/// Legacy private-shop subheader for requesting a market item price.
pub const PRIVATE_SHOP_SUBHEADER_MARKET_ITEM_PRICE_REQUEST: u8 = 21;

/// Extra bytes a fixed private-shop subheader appends **after** the two-byte
/// prefix.
///
/// This is the constant half of the table. [`PRIVATE_SHOP_SUBHEADER_BUILD`] is
/// deliberately absent because its length is count-derived; it is handled by
/// [`private_shop_extra_wire_size`]. The catch-all arm returns zero because an
/// unrecognised sub-header falls to `input_main.cpp:4078-4080` and consumes the
/// bare prefix without reading a payload.
///
/// Each size is a compile-time constant read out of the handler, never a value
/// taken from the client. The sizes are kept in separate arms even where two
/// arms agree, because each one documents a different legacy declaration.
const fn private_shop_constant_extra_wire_size(subheader: u8) -> usize {
    match subheader {
        // Three separate legacy declarations that happen to share a width:
        // `PrivateShopStart` and `PrivateShopMarketItemPriceRequest` each read
        // a bare `sizeof(DWORD)`, and `PrivateShopItemMove` is a whole
        // `sizeof(TPacketCGPrivateShopItemMove)`. They are grouped because the
        // width is what the framing layer sees, and the coincidence is not
        // evidence that they are the same request.
        PRIVATE_SHOP_SUBHEADER_START
        | PRIVATE_SHOP_SUBHEADER_MARKET_ITEM_PRICE_REQUEST
        | PRIVATE_SHOP_SUBHEADER_ITEM_MOVE => 4,
        // `PrivateShopBuy` and `PrivateShopSlotUnlockRequest` both read a bare
        // `sizeof(WORD)`.
        PRIVATE_SHOP_SUBHEADER_BUY | PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST => 2,
        // `sizeof(TPacketCGPrivateShopItemPriceChange)`, `packet.h:1265-1269`.
        PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE => 14,
        // `sizeof(TPacketCGPrivateShopItemCheckin)`, `packet.h:1380-1386`.
        PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN => 19,
        // `sizeof(TPacketCGPrivateShopItemCheckout)`, `packet.h:1388-1392`.
        PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT => 6,
        // `TITLE_MAX_LEN + 1`, `length.h:146`.
        PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE => 33,
        // `sizeof(TPacketCGPrivateShopSearch)`, `packet.h:1303-1307`.
        PRIVATE_SHOP_SUBHEADER_SEARCH => 85,
        // `sizeof(TPacketCGPrivateShopSearchBuy)`, `packet.h:1316-1319`, which is
        // `SELECTED_ITEM_MAX_NUM` (10, `length.h:140`) times an 18-byte
        // `SPrivateShopSearchSelectedItem` (`packet.h:1309-1314`).
        PRIVATE_SHOP_SUBHEADER_SEARCH_BUY => 180,
        // `CLOSE`, `PANEL_OPEN`, `PANEL_CLOSE`, `END`, `WITHDRAW`, `MODIFY`,
        // `WARP_REQUEST`, and `MARKET_ITEM_PRICE_DATA_REQUEST` all take no
        // `c_pData` at all. `STATE_UPDATE` is enumerated but has no dispatch
        // arm, and anything unrecognised falls through to `default:`; all of
        // them therefore extend by zero.
        _ => 0,
    }
}

/// Resolve the complete frame size of a header that declares its own length.
///
/// `HEADER_CG_CHAT`, `HEADER_CG_WHISPER`, and `HEADER_CG_SYNC_POSITION` all
/// carry a one-byte header followed by an explicit little-endian `WORD wSize`
/// at offset 1, so the complete frame length is read from the wire rather than
/// inferred from a table or from trailing bytes.
///
/// # Errors
///
/// Returns [`ClientFrameError::InvalidInventorySize`] if the registered base
/// size is too small to hold the three-byte prefix the length word lives in.
fn three_byte_prefix_declared_size(
    header: u8,
    base_size: usize,
    prefix: &[u8],
) -> Result<usize, ClientFrameError> {
    if base_size < CLIENT_FRAME_HEADER_SIZE + 2 {
        return Err(ClientFrameError::InvalidInventorySize {
            header,
            size: base_size,
        });
    }
    Ok(usize::from(u16::from_le_bytes([prefix[1], prefix[2]])))
}

/// Resolve the complete frame size of a private-shop frame.
///
/// This is the private-shop arm of
/// [`resolve_variable_client_frame_size`], split out so the shared resolver
/// stays within its size budget.
///
/// # Errors
///
/// Returns [`ClientFrameError::InvalidInventorySize`] if the registered base
/// size is not the two-byte `TPacketCGPrivateShop` prefix, or
/// [`ClientFrameError::SizeOverflow`] if the count-derived extension overflows.
fn private_shop_frame_size(
    header: u8,
    base_size: usize,
    prefix: &[u8],
) -> Result<Option<usize>, ClientFrameError> {
    if base_size != PRIVATE_SHOP_BASE_WIRE_SIZE {
        return Err(ClientFrameError::InvalidInventorySize {
            header,
            size: base_size,
        });
    }
    // The build subheader is the only count-derived one, so it is also the only
    // one that can still be undecidable from a two-byte prefix. `Ok(None)` is
    // the existing "need more bytes" signal, and `try_decode` re-offers the
    // frame once the count word lands.
    let Some(extra) = private_shop_extra_wire_size(prefix)? else {
        return Ok(None);
    };
    base_size
        .checked_add(extra)
        .ok_or(ClientFrameError::SizeOverflow)
        .map(Some)
}

/// Extra bytes a private-shop subheader appends **after** the two-byte prefix.
///
/// Every result is the extension only, never the whole frame, so the caller
/// adds [`PRIVATE_SHOP_BASE_WIRE_SIZE`] itself.
///
/// The `EPrivateShopCGSubheader` block at `packet.h:1135-1161` declares 22
/// enumerators with no explicit values, so they are 0 through 21. Twenty of them
/// are compile-time constants; only [`PRIVATE_SHOP_SUBHEADER_BUILD`] depends on
/// a count word, and it is the one subheader that can still be undecidable from
/// a two-byte prefix.
///
/// # Errors
///
/// Returns [`ClientFrameError::SizeOverflow`] if the item count would overflow
/// the frame size. A prefix too short to carry the count word yields `Ok(None)`,
/// the resolver's existing "need more bytes" signal, so a fragmented build
/// frame is not mistaken for a short one.
pub fn private_shop_extra_wire_size(prefix: &[u8]) -> Result<Option<usize>, ClientFrameError> {
    let subheader = prefix[PRIVATE_SHOP_BASE_WIRE_SIZE - 1];
    if subheader != PRIVATE_SHOP_SUBHEADER_BUILD {
        return Ok(Some(private_shop_constant_extra_wire_size(subheader)));
    }
    // `PrivateShopBuild`: the item count is the last two bytes of the 41-byte
    // fixed part, so a prefix shorter than that cannot decide the length.
    if prefix.len() < PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2 {
        return Ok(None);
    }
    let count = usize::from(u16::from_le_bytes([
        prefix[PRIVATE_SHOP_BUILD_COUNT_OFFSET],
        prefix[PRIVATE_SHOP_BUILD_COUNT_OFFSET + 1],
    ]));
    // The checked form is kept for consistency with the rest of the framing
    // layer, but it is provably equivalent to a plain multiply here: `count`
    // is widened from a `u16`, so the product is at most 65535 * 17 = 1_114_095
    // and cannot overflow a 64-bit `usize`. A mutation that replaces this with
    // `count * PRIVATE_SHOP_ITEM_WIRE_SIZE` is therefore undetectable, and is
    // recorded as equivalent rather than as a test gap.
    let items = count
        .checked_mul(PRIVATE_SHOP_ITEM_WIRE_SIZE)
        .ok_or(ClientFrameError::SizeOverflow)?;
    PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE
        .checked_add(items)
        .ok_or(ClientFrameError::SizeOverflow)
        .map(Some)
}

fn messenger_extra_wire_size(subheader: u8) -> usize {
    match subheader {
        MESSENGER_SUBHEADER_ADD_BY_VID => MESSENGER_ADD_BY_VID_EXTRA_WIRE_SIZE,
        MESSENGER_SUBHEADER_ADD_BY_NAME | MESSENGER_SUBHEADER_REMOVE => {
            MESSENGER_NAME_EXTRA_WIRE_SIZE
        }
        // INVITE_ANSWER and unknown subheaders have no extension in the
        // legacy Analyze switch. Do not guess a gameplay meaning for them.
        _ => 0,
    }
}

/// Resolve a complete frame size after the legacy packet-info prefix arrives.
///
/// `Ok(None)` means that `prefix` does not yet contain enough bytes to read
/// the variable length/count.  Fixed headers return their exact size even when
/// only the header byte is present.  Variable headers not listed in
/// [`RESOLVABLE_VARIABLE_CLIENT_HEADERS`] return
/// [`ClientFrameError::VariableLengthUnsupported`] rather than guessing from
/// arbitrary payload bytes.
///
/// One decoded `TPacketCGSyncPositionElement`.
///
/// Fields are decoded explicitly from the packed x86 wire order and do not
/// depend on Rust host layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPositionElement {
    /// `dwVID` as an unsigned 32-bit little-endian value.
    pub vid: u32,
    /// `lX` as a signed 32-bit little-endian value.
    pub x: i32,
    /// `lY` as a signed 32-bit little-endian value.
    pub y: i32,
}

/// A typed, aligned sync-position frame.
///
/// All declared elements are retained, including counts above the legacy
/// gameplay cap of 16. The game layer may apply that cap later; framing and
/// decoding must not discard bytes that the client actually sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPositionPacket {
    /// Total source-declared frame size, including the one-byte header.
    pub declared_size: usize,
    /// Elements in wire order.
    pub elements: Vec<SyncPositionElement>,
}

/// A failure while decoding a sync-position frame into typed elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncPositionDecodeError {
    /// The frame header was not `HEADER_CG_SYNC_POSITION`.
    UnexpectedHeader {
        /// Required one-byte header.
        expected: u8,
        /// Actual one-byte header.
        actual: u8,
    },
    /// The frame did not contain the complete two-byte size field after its
    /// header (the complete source prefix is three bytes).
    TruncatedPrefix {
        /// Payload bytes supplied after the header.
        available: usize,
    },
    /// The source-declared size did not equal the actual frame size.
    DeclaredLengthMismatch {
        /// Source-declared complete frame size.
        declared_size: usize,
        /// Actual complete frame size.
        actual_size: usize,
    },
    /// The declared trailing bytes were not a whole number of 12-byte
    /// elements. The legacy handler consumes such a frame and ignores its
    /// payload; this decoder reports it instead of reinterpreting it.
    NonElementAligned {
        /// Source-declared complete frame size.
        declared_size: usize,
        /// Bytes after the three-byte prefix.
        payload_len: usize,
    },
    /// The element vector could not reserve its source-sized allocation.
    AllocationFailed,
    /// A complete frame size could not be represented by `usize`.
    SizeOverflow,
}

impl fmt::Display for SyncPositionDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedHeader { expected, actual } => write!(
                formatter,
                "sync-position frame has header 0x{actual:02x}; expected 0x{expected:02x}"
            ),
            Self::TruncatedPrefix { available } => write!(
                formatter,
                "sync-position frame has {available} payload bytes; need the three-byte prefix"
            ),
            Self::DeclaredLengthMismatch {
                declared_size,
                actual_size,
            } => write!(
                formatter,
                "sync-position frame declares {declared_size} bytes but contains {actual_size}"
            ),
            Self::NonElementAligned {
                declared_size,
                payload_len,
            } => write!(
                formatter,
                "sync-position frame declares {declared_size} bytes with non-aligned payload length {payload_len}"
            ),
            Self::AllocationFailed => formatter.write_str("sync-position element allocation failed"),
            Self::SizeOverflow => formatter.write_str("sync-position frame size overflow"),
        }
    }
}

impl Error for SyncPositionDecodeError {}

/// Decode a complete sync-position frame into explicit 12-byte elements.
///
/// This function performs no count cap, player lookup, anti-cheat policy, or
/// gameplay action. A non-aligned declared length is returned as a distinct
/// error, matching the legacy consume-and-ignore behavior without silently
/// manufacturing elements.
///
/// # Errors
///
/// Returns [`SyncPositionDecodeError`] for a wrong header, short size field,
/// inconsistent declared size, non-element-aligned payload, checked-size
/// overflow, or allocation failure.
pub fn decode_sync_position(
    frame: &ClientFrame,
) -> Result<SyncPositionPacket, SyncPositionDecodeError> {
    let expected = HEADER_CG_SYNC_POSITION.value();
    if frame.header != expected {
        return Err(SyncPositionDecodeError::UnexpectedHeader {
            expected,
            actual: frame.header,
        });
    }
    if frame.payload.len() < 2 {
        return Err(SyncPositionDecodeError::TruncatedPrefix {
            available: frame.payload.len(),
        });
    }

    let declared_size = usize::from(u16::from_le_bytes([frame.payload[0], frame.payload[1]]));
    let actual_size = frame
        .payload
        .len()
        .checked_add(CLIENT_FRAME_HEADER_SIZE)
        .ok_or(SyncPositionDecodeError::SizeOverflow)?;
    if declared_size != actual_size {
        return Err(SyncPositionDecodeError::DeclaredLengthMismatch {
            declared_size,
            actual_size,
        });
    }

    let payload_len = frame.payload.len() - 2;
    if payload_len % SYNC_POSITION_ELEMENT_WIRE_SIZE != 0 {
        return Err(SyncPositionDecodeError::NonElementAligned {
            declared_size,
            payload_len,
        });
    }

    let element_count = payload_len / SYNC_POSITION_ELEMENT_WIRE_SIZE;
    let mut elements = Vec::new();
    elements
        .try_reserve_exact(element_count)
        .map_err(|_| SyncPositionDecodeError::AllocationFailed)?;
    for bytes in frame.payload[2..].chunks_exact(SYNC_POSITION_ELEMENT_WIRE_SIZE) {
        let vid = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let x = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let y = i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        elements.push(SyncPositionElement { vid, x, y });
    }

    Ok(SyncPositionPacket {
        declared_size,
        elements,
    })
}

/// A typed, raw-byte guild-symbol upload frame.
///
/// The legacy client sends a seven-byte packed prefix (`header`, explicit
/// little-endian `WORD size`, and explicit little-endian `DWORD guild_id`),
/// followed by the symbol bytes. The source size includes the prefix. The
/// symbol is intentionally opaque: this type does not decode an image, run a
/// CRC, look up a guild, or call a manager. The strict checks here are a
/// deliberate safety boundary: the legacy path computes `size - 7` only after
/// a byte-count check, while this codec rejects a declared total below seven,
/// an empty symbol, and a symbol over the source 64 KiB semantic limit before
/// allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildSymbolUpload {
    /// Complete source-declared frame size, including the header and prefix.
    pub declared_size: usize,
    /// Guild ID from the packed prefix.
    pub guild_id: u32,
    /// Raw symbol bytes after the seven-byte prefix.
    pub symbol: Vec<u8>,
}

/// A failure while validating or converting a guild-symbol upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildSymbolUploadError {
    /// The generic frame boundary rejected the size, completeness, or maximum.
    Frame(ClientFrameError),
    /// The frame did not carry the guild-symbol header.
    UnexpectedHeader {
        /// Required header.
        expected: u8,
        /// Actual header.
        actual: u8,
    },
    /// A zero-length symbol is not accepted by the legacy handler.
    EmptySymbol,
    /// The raw symbol exceeded the legacy semantic limit.
    SymbolTooLarge {
        /// Supplied symbol length.
        length: usize,
        /// Maximum accepted symbol length.
        maximum: usize,
    },
    /// A declared complete size could not fit the legacy `WORD` field.
    SizeOverflow,
    /// A checked allocation for an owned symbol or frame failed.
    AllocationFailed,
}

impl fmt::Display for GuildSymbolUploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(source) => write!(formatter, "guild-symbol frame rejected: {source}"),
            Self::UnexpectedHeader { expected, actual } => write!(
                formatter,
                "guild-symbol frame has header 0x{actual:02x}; expected 0x{expected:02x}"
            ),
            Self::EmptySymbol => formatter.write_str("guild-symbol upload has an empty symbol"),
            Self::SymbolTooLarge { length, maximum } => write!(
                formatter,
                "guild-symbol upload has {length} bytes; maximum is {maximum}"
            ),
            Self::SizeOverflow => formatter.write_str("guild-symbol upload size overflow"),
            Self::AllocationFailed => formatter.write_str("guild-symbol upload allocation failed"),
        }
    }
}

impl Error for GuildSymbolUploadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Frame(source) => Some(source),
            Self::UnexpectedHeader { .. }
            | Self::EmptySymbol
            | Self::SymbolTooLarge { .. }
            | Self::SizeOverflow
            | Self::AllocationFailed => None,
        }
    }
}

impl GuildSymbolUpload {
    /// Construct a validated upload from a guild ID and raw symbol bytes.
    ///
    /// # Errors
    ///
    /// Returns [`GuildSymbolUploadError::EmptySymbol`] for an empty symbol,
    /// `SymbolTooLarge` for more than 64 KiB, and `SizeOverflow` when the
    /// complete size cannot fit the source `WORD`.
    pub fn new(guild_id: u32, symbol: &[u8]) -> Result<Self, GuildSymbolUploadError> {
        validate_symbol_len(symbol.len())?;
        let declared_size = GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE
            .checked_add(symbol.len())
            .ok_or(GuildSymbolUploadError::SizeOverflow)?;
        if declared_size > usize::from(u16::MAX) {
            return Err(GuildSymbolUploadError::SizeOverflow);
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(symbol.len())
            .map_err(|_| GuildSymbolUploadError::AllocationFailed)?;
        owned.extend_from_slice(symbol);
        Ok(Self {
            declared_size,
            guild_id,
            symbol: owned,
        })
    }

    /// Decode one complete generic client frame without interpreting the symbol.
    ///
    /// # Errors
    ///
    /// Returns a wrapped [`ClientFrameError`] for a wrong prefix size, declared
    /// size below seven, incomplete declared payload, or declared/actual
    /// mismatch. It returns a typed error for an empty or oversized symbol and
    /// for checked allocation failure.
    pub fn decode(frame: &ClientFrame) -> Result<Self, GuildSymbolUploadError> {
        let expected_header = HEADER_CG_GUILD_SYMBOL_UPLOAD.value();
        if frame.header != expected_header {
            return Err(GuildSymbolUploadError::UnexpectedHeader {
                expected: expected_header,
                actual: frame.header,
            });
        }
        if frame.payload.len() < GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE - 1 {
            return Err(GuildSymbolUploadError::Frame(ClientFrameError::Truncated {
                header: expected_header,
                expected: GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE,
                available: frame
                    .payload
                    .len()
                    .checked_add(CLIENT_FRAME_HEADER_SIZE)
                    .ok_or(GuildSymbolUploadError::SizeOverflow)?,
            }));
        }
        let declared_size = usize::from(u16::from_le_bytes([frame.payload[0], frame.payload[1]]));
        if declared_size < GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE {
            return Err(GuildSymbolUploadError::Frame(
                ClientFrameError::InvalidVariableSize {
                    header: expected_header,
                    declared_size,
                    base_size: GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE,
                },
            ));
        }
        let actual_size = frame
            .payload
            .len()
            .checked_add(CLIENT_FRAME_HEADER_SIZE)
            .ok_or(GuildSymbolUploadError::SizeOverflow)?;
        if actual_size < declared_size {
            return Err(GuildSymbolUploadError::Frame(ClientFrameError::Truncated {
                header: expected_header,
                expected: declared_size,
                available: actual_size,
            }));
        }
        if actual_size != declared_size {
            return Err(GuildSymbolUploadError::Frame(
                ClientFrameError::LengthMismatch {
                    header: expected_header,
                    expected: declared_size,
                    actual: actual_size,
                },
            ));
        }
        let symbol_len = declared_size - GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE;
        validate_symbol_len(symbol_len)?;
        let symbol_start = GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE - 1;
        let mut symbol = Vec::new();
        symbol
            .try_reserve_exact(symbol_len)
            .map_err(|_| GuildSymbolUploadError::AllocationFailed)?;
        symbol.extend_from_slice(&frame.payload[symbol_start..]);
        let guild_id = u32::from_le_bytes([
            frame.payload[2],
            frame.payload[3],
            frame.payload[4],
            frame.payload[5],
        ]);
        Ok(Self {
            declared_size,
            guild_id,
            symbol,
        })
    }

    /// Convert this typed upload into a generic variable client frame.
    ///
    /// # Errors
    ///
    /// Returns a wrapped [`ClientFrameError::LengthMismatch`] if the public
    /// `declared_size` field disagrees with the owned symbol, or a typed
    /// semantic/allocation error otherwise.
    pub fn to_frame(&self) -> Result<ClientFrame, GuildSymbolUploadError> {
        let actual_size = GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE
            .checked_add(self.symbol.len())
            .ok_or(GuildSymbolUploadError::SizeOverflow)?;
        if self.declared_size != actual_size {
            return Err(GuildSymbolUploadError::Frame(
                ClientFrameError::LengthMismatch {
                    header: HEADER_CG_GUILD_SYMBOL_UPLOAD.value(),
                    expected: actual_size,
                    actual: self.declared_size,
                },
            ));
        }
        validate_symbol_len(self.symbol.len())?;
        if actual_size > usize::from(u16::MAX) {
            return Err(GuildSymbolUploadError::SizeOverflow);
        }
        let payload_len = actual_size
            .checked_sub(CLIENT_FRAME_HEADER_SIZE)
            .ok_or(GuildSymbolUploadError::SizeOverflow)?;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_len)
            .map_err(|_| GuildSymbolUploadError::AllocationFailed)?;
        payload.extend_from_slice(
            &u16::try_from(actual_size)
                .map_err(|_| GuildSymbolUploadError::SizeOverflow)?
                .to_le_bytes(),
        );
        payload.extend_from_slice(&self.guild_id.to_le_bytes());
        payload.extend_from_slice(&self.symbol);
        Ok(ClientFrame {
            header: HEADER_CG_GUILD_SYMBOL_UPLOAD.value(),
            payload,
        })
    }

    /// Encode the exact raw legacy bytes, including the one-byte header.
    ///
    /// # Errors
    ///
    /// Returns the same validation and allocation errors as
    /// [`GuildSymbolUpload::to_frame`].
    pub fn encode(&self) -> Result<Vec<u8>, GuildSymbolUploadError> {
        let frame = self.to_frame()?;
        let total = frame
            .payload
            .len()
            .checked_add(CLIENT_FRAME_HEADER_SIZE)
            .ok_or(GuildSymbolUploadError::SizeOverflow)?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(total)
            .map_err(|_| GuildSymbolUploadError::AllocationFailed)?;
        encoded.push(frame.header);
        encoded.extend_from_slice(&frame.payload);
        Ok(encoded)
    }

    /// Borrow the opaque symbol bytes.
    #[must_use]
    pub fn symbol_bytes(&self) -> &[u8] {
        &self.symbol
    }
}

/// Decode a complete guild-symbol upload frame.
///
/// # Errors
///
/// Returns a typed framing, semantic, header, or allocation error.
pub fn decode_guild_symbol_upload(
    frame: &ClientFrame,
) -> Result<GuildSymbolUpload, GuildSymbolUploadError> {
    GuildSymbolUpload::decode(frame)
}

/// Encode a guild ID and raw symbol as the exact legacy frame bytes.
///
/// # Errors
///
/// Returns a typed semantic, size, or allocation error.
pub fn encode_guild_symbol_upload(
    guild_id: u32,
    symbol: &[u8],
) -> Result<Vec<u8>, GuildSymbolUploadError> {
    GuildSymbolUpload::new(guild_id, symbol)?.encode()
}

fn validate_symbol_len(length: usize) -> Result<(), GuildSymbolUploadError> {
    if length == 0 {
        Err(GuildSymbolUploadError::EmptySymbol)
    } else if length > GUILD_SYMBOL_MAX_SOURCE_BYTES {
        Err(GuildSymbolUploadError::SymbolTooLarge {
            length,
            maximum: GUILD_SYMBOL_MAX_SOURCE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn resolve_guild_symbol_declared_size(
    header: u8,
    base_size: usize,
    prefix: &[u8],
) -> Result<usize, ClientFrameError> {
    if base_size != GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE {
        return Err(ClientFrameError::InvalidInventorySize {
            header,
            size: base_size,
        });
    }
    // `size` is the complete packed frame length, including the seven-byte
    // prefix. It is read explicitly little-endian rather than inferred from
    // Rust host layout or trailing bytes.
    Ok(usize::from(u16::from_le_bytes([prefix[1], prefix[2]])))
}

/// The active C++ code reads `WORD wSize` at offset 1 for chat, whisper, and
/// sync-position.  My-shop instead has a `BYTE bCount` at the end of its
/// 35-byte prefix and appends one 68-byte record per count.
///
/// # Errors
///
/// Returns a typed error for unknown or unsupported variable headers, invalid
/// packet-info sizes, short declared sizes, or checked-size overflow.
pub fn resolve_variable_client_frame_size(
    header: u8,
    prefix: &[u8],
) -> Result<Option<usize>, ClientFrameError> {
    let resolved = resolve_client_frame_size(header)?;
    let base_size = match resolved {
        ClientFrameSize::Fixed(size) => return Ok(Some(size)),
        ClientFrameSize::Variable(size) => size,
    };

    if !RESOLVABLE_VARIABLE_CLIENT_HEADERS.contains(&header) {
        return Err(ClientFrameError::VariableLengthUnsupported { header, base_size });
    }
    if prefix.len() < base_size {
        return Ok(None);
    }

    let size = match header {
        value
            if value == HEADER_CG_CHAT.value()
                || value == HEADER_CG_WHISPER.value()
                || value == HEADER_CG_SYNC_POSITION.value() =>
        {
            three_byte_prefix_declared_size(header, base_size, prefix)?
        }
        value if value == HEADER_CG_GUILD_SYMBOL_UPLOAD.value() => {
            resolve_guild_symbol_declared_size(header, base_size, prefix)?
        }
        value if value == HEADER_CG_MYSHOP.value() => {
            if base_size != MY_SHOP_BASE_WIRE_SIZE {
                return Err(ClientFrameError::InvalidInventorySize {
                    header,
                    size: base_size,
                });
            }
            let count = usize::from(prefix[base_size - 1]);
            let extra = count
                .checked_mul(MY_SHOP_ITEM_WIRE_SIZE)
                .ok_or(ClientFrameError::SizeOverflow)?;
            base_size
                .checked_add(extra)
                .ok_or(ClientFrameError::SizeOverflow)?
        }
        value if value == HEADER_CG_SHOP.value() => {
            if base_size != SHOP_BASE_WIRE_SIZE {
                return Err(ClientFrameError::InvalidInventorySize {
                    header,
                    size: base_size,
                });
            }
            let extra = match prefix[1] {
                // TPacketCGShopBuy is two BYTE cells. The source reads the
                // second cell as the position and consumes both bytes.
                SHOP_SUBHEADER_BUY => 2,
                SHOP_SUBHEADER_SELL => 1,
                // TfckOFF is declared outside packet.h's pack(1) region.
                // On the active x86 target it is four bytes: slot, padding,
                // then a WORD count. The padding byte is not initialized by
                // the legacy client sender and is preserved as wire data here.
                SHOP_SUBHEADER_SELL2 => 4,
                // END and unknown subheaders both consume only the prefix.
                _ => 0,
            };
            base_size
                .checked_add(extra)
                .ok_or(ClientFrameError::SizeOverflow)?
        }
        value if value == HEADER_CG_PRIVATE_SHOP.value() => {
            let Some(size) = private_shop_frame_size(header, base_size, prefix)? else {
                return Ok(None);
            };
            size
        }
        value if value == HEADER_CG_MESSENGER.value() => {
            if base_size != MESSENGER_BASE_WIRE_SIZE {
                return Err(ClientFrameError::InvalidInventorySize {
                    header,
                    size: base_size,
                });
            }
            base_size
                .checked_add(messenger_extra_wire_size(prefix[1]))
                .ok_or(ClientFrameError::SizeOverflow)?
        }
        value if value == HEADER_CG_FISH_EVENT_SEND.value() => {
            if base_size != FISH_EVENT_BASE_WIRE_SIZE {
                return Err(ClientFrameError::InvalidInventorySize {
                    header,
                    size: base_size,
                });
            }
            match prefix[1] {
                FISH_EVENT_SUBHEADER_BOX_USE => base_size
                    .checked_add(3)
                    .ok_or(ClientFrameError::SizeOverflow)?,
                FISH_EVENT_SUBHEADER_SHAPE_ADD => base_size
                    .checked_add(1)
                    .ok_or(ClientFrameError::SizeOverflow)?,
                // The legacy switch has no default action for unknown
                // subheaders. It consumes only the two-byte prefix.
                _ => base_size,
            }
        }
        _ => {
            return Err(ClientFrameError::VariableLengthUnsupported { header, base_size });
        }
    };

    if size < base_size {
        return Err(ClientFrameError::InvalidVariableSize {
            header,
            declared_size: size,
            base_size,
        });
    }
    Ok(Some(size))
}

/// Incremental decoder for fixed and source-verified variable client frames.
///
/// `feed` accepts arbitrary TCP fragments. `try_decode` returns `None` until a
/// complete frame is buffered and leaves coalesced following bytes for the
/// next call. Errors leave the input untouched so the caller can inspect or
/// explicitly clear it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableClientFrameDecoder {
    buffer: Vec<u8>,
    max_frame_size: usize,
}

impl VariableClientFrameDecoder {
    /// Create a decoder with [`DEFAULT_MAX_CLIENT_FRAME_SIZE`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_frame_size(DEFAULT_MAX_CLIENT_FRAME_SIZE)
    }

    /// Create a decoder with an explicit maximum complete frame size.
    #[must_use]
    pub fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_frame_size,
        }
    }

    /// Return the configured maximum complete frame size.
    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.max_frame_size
    }

    /// Return the number of bytes currently buffered.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Return whether no bytes are buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Peek at the next header without consuming input.
    #[must_use]
    pub fn peek_header(&self) -> Option<u8> {
        self.buffer.first().copied()
    }

    /// Borrow currently buffered bytes without consuming them.
    #[must_use]
    pub fn buffered_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Add an arbitrary TCP fragment to the decoder.
    ///
    /// A coalesced read may contain several complete frames and therefore may
    /// exceed the per-frame maximum. The maximum is applied to each frame when
    /// a frame is resolved.
    ///
    /// # Errors
    ///
    /// Returns a checked-size or allocation error while appending input.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), ClientFrameError> {
        self.buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(ClientFrameError::SizeOverflow)?;
        self.buffer
            .try_reserve(bytes.len())
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Try to consume one complete fixed or source-verified variable frame.
    ///
    /// # Errors
    ///
    /// Returns a typed error for unknown/unsupported headers, invalid declared
    /// sizes, oversized frames, checked-size failures, or allocation failure.
    /// A short prefix or short complete frame is represented by `Ok(None)`.
    pub fn try_decode(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        let Some(&header) = self.buffer.first() else {
            return Ok(None);
        };
        let Some(size) = resolve_variable_client_frame_size(header, &self.buffer)? else {
            return Ok(None);
        };
        if size > self.max_frame_size {
            return Err(ClientFrameError::FrameTooLarge {
                size,
                maximum: self.max_frame_size,
            });
        }
        if self.buffer.len() < size {
            return Ok(None);
        }

        let payload_len = size
            .checked_sub(CLIENT_FRAME_HEADER_SIZE)
            .ok_or(ClientFrameError::InvalidInventorySize { header, size })?;
        let mut payload = Vec::new();
        payload
            .try_reserve(payload_len)
            .map_err(|_| ClientFrameError::AllocationFailed)?;
        payload.extend_from_slice(&self.buffer[CLIENT_FRAME_HEADER_SIZE..size]);
        self.buffer.drain(..size);
        Ok(Some(ClientFrame { header, payload }))
    }

    /// Compatibility alias for [`VariableClientFrameDecoder::try_decode`].
    ///
    /// # Errors
    ///
    /// See [`VariableClientFrameDecoder::try_decode`].
    pub fn decode(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        self.try_decode()
    }

    /// Validate EOF without consuming complete buffered frames.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an incomplete prefix/frame, unknown or
    /// unsupported header, invalid declared size, or oversized frame.
    pub fn finish(&self) -> Result<(), ClientFrameError> {
        let mut offset = 0;
        while offset < self.buffer.len() {
            let header = self.buffer[offset];
            let available = self.buffer.len() - offset;
            let Some(size) = resolve_variable_client_frame_size(header, &self.buffer[offset..])?
            else {
                let prefix_size = match resolve_client_frame_size(header)? {
                    ClientFrameSize::Fixed(size) | ClientFrameSize::Variable(size) => size,
                };
                return Err(ClientFrameError::Truncated {
                    header,
                    expected: prefix_size,
                    available,
                });
            };
            if size > self.max_frame_size {
                return Err(ClientFrameError::FrameTooLarge {
                    size,
                    maximum: self.max_frame_size,
                });
            }
            let end = offset
                .checked_add(size)
                .ok_or(ClientFrameError::SizeOverflow)?;
            if end > self.buffer.len() {
                return Err(ClientFrameError::Truncated {
                    header,
                    expected: size,
                    available,
                });
            }
            offset = end;
        }
        Ok(())
    }

    /// Compatibility alias for [`VariableClientFrameDecoder::finish`].
    ///
    /// # Errors
    ///
    /// See [`VariableClientFrameDecoder::finish`].
    pub fn finish_eof(&self) -> Result<(), ClientFrameError> {
        self.finish()
    }

    /// Decode one frame, then validate EOF if no complete frame was available.
    ///
    /// # Errors
    ///
    /// See [`VariableClientFrameDecoder::try_decode`] and
    /// [`VariableClientFrameDecoder::finish`].
    pub fn decode_eof(&mut self) -> Result<Option<ClientFrame>, ClientFrameError> {
        if let Some(frame) = self.try_decode()? {
            return Ok(Some(frame));
        }
        self.finish()?;
        Ok(None)
    }

    /// Remove all buffered bytes after a protocol or connection error.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl Default for VariableClientFrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Compatibility alias for [`VariableClientFrameDecoder`].
pub type ClientFrameVariableDecoder = VariableClientFrameDecoder;

/// Compatibility alias for [`VariableClientFrameDecoder`].
pub type LegacyVariableClientFrameDecoder = VariableClientFrameDecoder;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_inventory::HEADER_CG_GUILD;
    use crate::cg_wire::ClientFrameDecoder;

    fn guild_symbol_raw(total: usize, symbol_len: usize) -> Vec<u8> {
        let symbol_len = symbol_len.min(total.saturating_sub(GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE));
        let mut bytes = vec![HEADER_CG_GUILD_SYMBOL_UPLOAD.value()];
        let total_u16 = u16::try_from(total).unwrap();
        bytes.extend_from_slice(&total_u16.to_le_bytes());
        bytes.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0xa5, symbol_len));
        bytes
    }

    // -----------------------------------------------------------------------
    // Private shop
    // -----------------------------------------------------------------------

    const PS: u8 = crate::cg_inventory::HEADER_CG_PRIVATE_SHOP.value();

    /// Every sub-header's extra size, transcribed from the handler constants in
    /// ledger Section 148. `None` means the sub-header takes no payload.
    fn private_shop_table() -> Vec<(u8, Option<usize>)> {
        vec![
            (0, Some(41)),   // BUILD: 41 + count * 17, checked separately
            (1, None),       // CLOSE
            (2, None),       // PANEL_OPEN
            (3, None),       // PANEL_CLOSE
            (4, Some(4)),    // START: sizeof(DWORD)
            (5, None),       // END
            (6, Some(2)),    // BUY: sizeof(WORD)
            (7, None),       // WITHDRAW
            (8, None),       // MODIFY
            (9, None),       // STATE_UPDATE: enumerated but never dispatched
            (10, Some(14)),  // ITEM_PRICE_CHANGE
            (11, Some(4)),   // ITEM_MOVE
            (12, Some(19)),  // ITEM_CHECKIN
            (13, Some(6)),   // ITEM_CHECKOUT
            (14, Some(33)),  // TITLE_CHANGE: TITLE_MAX_LEN + 1
            (15, None),      // WARP_REQUEST
            (16, Some(2)),   // SLOT_UNLOCK_REQUEST: sizeof(WORD)
            (17, None),      // SEARCH_CLOSE
            (18, Some(85)),  // SEARCH
            (19, Some(180)), // SEARCH_BUY: 10 * 18
            (20, None),      // MARKET_ITEM_PRICE_DATA_REQUEST
            (21, Some(4)),   // MARKET_ITEM_PRICE_REQUEST
        ]
    }

    #[test]
    fn private_shop_constants_match_the_source() {
        assert_eq!(PS, 236);
        assert_eq!(PRIVATE_SHOP_BASE_WIRE_SIZE, 2);
        assert_eq!(PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE, 41);
        assert_eq!(PRIVATE_SHOP_ITEM_WIRE_SIZE, 17);
        // The count word is the last two bytes of the 41-byte fixed part.
        assert_eq!(PRIVATE_SHOP_BUILD_COUNT_OFFSET, 41);
        assert_eq!(
            PRIVATE_SHOP_BUILD_COUNT_OFFSET,
            PRIVATE_SHOP_BASE_WIRE_SIZE + PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE - 2
        );
        // Sub-header values are sequential 0..=21 with no explicit values.
        let consts = [
            PRIVATE_SHOP_SUBHEADER_BUILD,
            PRIVATE_SHOP_SUBHEADER_CLOSE,
            PRIVATE_SHOP_SUBHEADER_PANEL_OPEN,
            PRIVATE_SHOP_SUBHEADER_PANEL_CLOSE,
            PRIVATE_SHOP_SUBHEADER_START,
            PRIVATE_SHOP_SUBHEADER_END,
            PRIVATE_SHOP_SUBHEADER_BUY,
            PRIVATE_SHOP_SUBHEADER_WITHDRAW,
            PRIVATE_SHOP_SUBHEADER_MODIFY,
            PRIVATE_SHOP_SUBHEADER_STATE_UPDATE,
            PRIVATE_SHOP_SUBHEADER_ITEM_PRICE_CHANGE,
            PRIVATE_SHOP_SUBHEADER_ITEM_MOVE,
            PRIVATE_SHOP_SUBHEADER_ITEM_CHECKIN,
            PRIVATE_SHOP_SUBHEADER_ITEM_CHECKOUT,
            PRIVATE_SHOP_SUBHEADER_TITLE_CHANGE,
            PRIVATE_SHOP_SUBHEADER_WARP_REQUEST,
            PRIVATE_SHOP_SUBHEADER_SLOT_UNLOCK_REQUEST,
            PRIVATE_SHOP_SUBHEADER_SEARCH_CLOSE,
            PRIVATE_SHOP_SUBHEADER_SEARCH,
            PRIVATE_SHOP_SUBHEADER_SEARCH_BUY,
            PRIVATE_SHOP_SUBHEADER_MARKET_ITEM_PRICE_DATA_REQUEST,
            PRIVATE_SHOP_SUBHEADER_MARKET_ITEM_PRICE_REQUEST,
        ];
        assert_eq!(consts.len(), 22);
        for (i, v) in consts.iter().enumerate() {
            let want = u8::try_from(i).expect("22 sub-headers fit in a u8");
            assert_eq!(*v, want, "sub-header {i} constant drifted");
        }
    }

    #[test]
    fn private_shop_resolves_every_subheader_to_its_source_size() {
        assert_eq!(private_shop_table().len(), 22);
        for (sub, extra) in private_shop_table() {
            if sub == PRIVATE_SHOP_SUBHEADER_BUILD {
                continue; // count-derived, covered by its own tests
            }
            let want = extra.unwrap_or(0);
            let mut frame = vec![PS, sub];
            frame.resize(PRIVATE_SHOP_BASE_WIRE_SIZE + want + 1, 0x5a);
            assert_eq!(
                private_shop_extra_wire_size(&frame),
                Ok(Some(want)),
                "sub-header {sub} extra size"
            );
            // The resolver is reached with only the 2-byte prefix too.
            let prefix = [PS, sub];
            assert_eq!(
                private_shop_extra_wire_size(&prefix),
                Ok(Some(want)),
                "sub-header {sub} from a bare prefix"
            );
            assert_eq!(
                resolve_variable_client_frame_size(PS, &prefix),
                Ok(Some(2 + want)),
                "sub-header {sub} full frame"
            );
        }
    }

    #[test]
    fn the_nine_zero_extension_subheaders_consume_only_the_prefix() {
        let zero: Vec<u8> = private_shop_table()
            .iter()
            .filter(|(_, e)| e.is_none())
            .map(|(s, _)| *s)
            .collect();
        // Nine sub-headers are dispatched with no `c_pData` (CLOSE,
        // PANEL_OPEN, PANEL_CLOSE, END, WITHDRAW, MODIFY, WARP_REQUEST,
        // SEARCH_CLOSE, MARKET_ITEM_PRICE_DATA_REQUEST) and STATE_UPDATE
        // has no arm at all, so ten sub-headers extend by zero.
        assert_eq!(zero.len(), 10, "ten sub-headers take no payload");
        for s in zero {
            assert_eq!(
                private_shop_extra_wire_size(&[PS, s]),
                Ok(Some(0)),
                "sub-header {s}"
            );
        }
    }

    /// `STATE_UPDATE` is declared but has no dispatch arm, so it must be framed
    /// as a bare prefix and must not be given an invented payload.
    /// The private-shop arm rejects a registered base size that is not the
    /// two-byte `TPacketCGPrivateShop` prefix. The guard is unreachable through
    /// `resolve_variable_client_frame_size`, because `cg_inventory` fixes the
    /// row at `base_size: 2`, so it is exercised directly here.
    #[test]
    fn the_private_shop_arm_rejects_a_wrong_registered_base_size() {
        for wrong in [0usize, 1, 3, 21, 2 + 41] {
            let err = private_shop_frame_size(PS, wrong, &[PS, 1])
                .expect_err("a wrong base size must be rejected");
            assert!(
                matches!(
                    err,
                    ClientFrameError::InvalidInventorySize { size, .. } if size == wrong
                ),
                "base_size {wrong} gave {err:?}"
            );
        }
        // The correct base size is still accepted for the same subheader.
        assert_eq!(
            private_shop_frame_size(PS, PRIVATE_SHOP_BASE_WIRE_SIZE, &[PS, 1]),
            Ok(Some(2))
        );
    }

    #[test]
    fn state_update_is_framed_as_a_bare_prefix_not_an_invented_payload() {
        assert_eq!(PRIVATE_SHOP_SUBHEADER_STATE_UPDATE, 9);
        assert_eq!(private_shop_extra_wire_size(&[PS, 9]), Ok(Some(0)));
        assert_eq!(
            resolve_variable_client_frame_size(PS, &[PS, 9]),
            Ok(Some(2))
        );
    }

    /// An unrecognised sub-header falls to `input_main.cpp:4078-4080` and
    /// consumes the bare prefix, so 22..=255 must behave like the zero cases.
    #[test]
    fn every_unrecognised_subheader_consumes_only_the_prefix() {
        for s in 22u8..=u8::MAX {
            assert_eq!(
                private_shop_extra_wire_size(&[PS, s]),
                Ok(Some(0)),
                "unrecognised sub-header {s} was given a payload"
            );
        }
    }

    #[test]
    fn private_shop_build_is_count_derived_and_waits_for_its_count_word() {
        // A two-byte prefix cannot decide the size, so it must ask for more.
        assert_eq!(private_shop_extra_wire_size(&[PS, 0]), Ok(None));
        assert_eq!(resolve_variable_client_frame_size(PS, &[PS, 0]), Ok(None));

        // Every prefix shorter than the count word must also wait.
        for len in 2..PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2 {
            let mut p = vec![PS, 0];
            p.resize(len, 0x00);
            assert_eq!(
                private_shop_extra_wire_size(&p),
                Ok(None),
                "prefix of {len} bytes should wait"
            );
        }
    }

    #[test]
    fn private_shop_build_sizes_from_the_count_at_offset_41() {
        for count in [0u16, 1, 2, 17, 100, 255, 256, 1000, u16::MAX] {
            let mut p = vec![PS, 0];
            p.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0x00);
            p[PRIVATE_SHOP_BUILD_COUNT_OFFSET] = (count & 0xff) as u8;
            p[PRIVATE_SHOP_BUILD_COUNT_OFFSET + 1] = (count >> 8) as u8;
            let want_extra = PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE
                + usize::from(count) * PRIVATE_SHOP_ITEM_WIRE_SIZE;
            assert_eq!(
                private_shop_extra_wire_size(&p),
                Ok(Some(want_extra)),
                "count {count} extra size"
            );
            let mut p2 = p.clone();
            p2.resize(2 + want_extra, 0x11);
            assert_eq!(
                resolve_variable_client_frame_size(PS, &p2),
                Ok(Some(2 + want_extra)),
                "count {count} full frame"
            );
        }
    }

    #[test]
    fn the_count_word_is_read_little_endian_from_offset_41() {
        // 0x0001 little-endian and 0x0100 little-endian must differ. A
        // big-endian read would make these two identical, so this test fails
        // if the byte order is ever wrong.
        let mut lo = vec![PS, 0];
        lo.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0);
        lo[PRIVATE_SHOP_BUILD_COUNT_OFFSET] = 0x01;
        let mut hi = vec![PS, 0];
        hi.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0);
        hi[PRIVATE_SHOP_BUILD_COUNT_OFFSET + 1] = 0x01;
        // Low byte set means count 1; high byte set means count 256. A
        // big-endian read would swap the two, so pinning both directions pins
        // the byte order.
        assert_eq!(
            private_shop_extra_wire_size(&lo),
            Ok(Some(
                PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE + PRIVATE_SHOP_ITEM_WIRE_SIZE
            ))
        );
        assert_eq!(
            private_shop_extra_wire_size(&hi),
            Ok(Some(
                PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE + 256 * PRIVATE_SHOP_ITEM_WIRE_SIZE
            ))
        );
        assert_ne!(
            private_shop_extra_wire_size(&lo),
            private_shop_extra_wire_size(&hi)
        );
    }

    #[test]
    fn a_maximum_count_is_not_an_overflow_and_the_decoder_cap_bounds_it() {
        let mut p = vec![PS, 0];
        p.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0);
        p[PRIVATE_SHOP_BUILD_COUNT_OFFSET] = 0xff;
        p[PRIVATE_SHOP_BUILD_COUNT_OFFSET + 1] = 0xff;
        // 65535 * 17 = 1 114 095 is perfectly representable, so the resolver
        // must NOT report SizeOverflow. What bounds a hostile count is the
        // decoder's own configured maximum, not checked arithmetic.
        let resolved = private_shop_extra_wire_size(&p).expect("resolves");
        assert_eq!(
            resolved,
            Some(PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE + 65_535 * PRIVATE_SHOP_ITEM_WIRE_SIZE)
        );

        // With a small explicit cap the same frame is refused as too large.
        let mut d = VariableClientFrameDecoder::with_max_frame_size(1024);
        d.feed(&p).expect("feed");
        let err = d.try_decode().expect_err("must exceed the frame cap");
        assert!(
            matches!(err, ClientFrameError::FrameTooLarge { size, maximum } if size > 1024 && maximum == 1024),
            "got {err:?}"
        );
        // And with a cap above the resolved size it merely waits for bytes.
        let mut ok = VariableClientFrameDecoder::with_max_frame_size(
            PRIVATE_SHOP_BASE_WIRE_SIZE
                + PRIVATE_SHOP_BUILD_FIXED_WIRE_SIZE
                + 65_535 * PRIVATE_SHOP_ITEM_WIRE_SIZE,
        );
        ok.feed(&p).expect("feed");
        assert_eq!(ok.try_decode().expect("no error"), None);
    }

    #[test]
    fn a_prefix_shorter_than_two_bytes_is_not_a_private_shop_frame() {
        // The resolver's own base-size guard keeps this out of the private-shop
        // arm entirely, so an empty or one-byte prefix is not a decision.
        assert_eq!(resolve_variable_client_frame_size(PS, &[]), Ok(None));
        assert_eq!(resolve_variable_client_frame_size(PS, &[PS]), Ok(None));
    }

    #[test]
    fn private_shop_frames_stream_through_the_variable_decoder() {
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for (sub, extra) in private_shop_table() {
            if sub == PRIVATE_SHOP_SUBHEADER_BUILD {
                continue;
            }
            let n = extra.unwrap_or(0);
            let mut f = vec![PS, sub];
            f.resize(2 + n, sub);
            frames.push(f);
        }
        // One build frame with three items, streamed last.
        let mut build = vec![PS, 0];
        build.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0x7f);
        build[PRIVATE_SHOP_BUILD_COUNT_OFFSET] = 3;
        build[PRIVATE_SHOP_BUILD_COUNT_OFFSET + 1] = 0;
        let build_len = 2 + 41 + 3 * 17;
        build.resize(build_len, 0x33);
        frames.push(build);

        let stream: Vec<u8> = frames.concat();
        for split in 0..stream.len() {
            let mut d = VariableClientFrameDecoder::new();
            d.feed(&stream[..split]).expect("prefix feed");
            d.feed(&stream[split..]).expect("suffix feed");
            let mut got: Vec<Vec<u8>> = Vec::new();
            while let Some(f) = d.try_decode().expect("decode") {
                assert_eq!(f.header, PS, "split {split} produced a foreign header");
                got.push(f.payload);
            }
            assert!(d.is_empty(), "split {split} left bytes buffered");
            let want: Vec<Vec<u8>> = frames.iter().map(|f| f[1..].to_vec()).collect();
            assert_eq!(got, want, "split at {split}");
        }
    }

    #[test]
    fn private_shop_bytes_at_a_time_still_reassemble() {
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for (sub, extra) in private_shop_table() {
            if sub == PRIVATE_SHOP_SUBHEADER_BUILD {
                continue;
            }
            let mut f = vec![PS, sub];
            f.resize(2 + extra.unwrap_or(0), 0x9c);
            frames.push(f);
        }
        let mut build = vec![PS, 0];
        build.resize(PRIVATE_SHOP_BUILD_COUNT_OFFSET + 2, 0);
        build[PRIVATE_SHOP_BUILD_COUNT_OFFSET] = 2;
        build.resize(2 + 41 + 2 * 17, 0x9c);
        frames.push(build);
        let stream: Vec<u8> = frames.concat();
        let mut d = VariableClientFrameDecoder::new();
        let mut n = 0usize;
        for b in &stream {
            d.feed(&[*b]).expect("byte feed");
            while let Some(f) = d.try_decode().expect("byte decode") {
                assert_eq!(f.header, PS);
                n += 1;
            }
        }
        assert_eq!(n, frames.len());
        assert!(d.is_empty());
    }

    #[test]
    fn guild_symbol_codec_uses_explicit_little_endian_prefix_and_raw_bytes() {
        let symbol = [0x00, 0xff, 0x80, 0x7f, 0x00];
        let upload = GuildSymbolUpload::new(0x1234_5678, &symbol).unwrap();
        let encoded = upload.encode().unwrap();
        assert_eq!(
            encoded,
            vec![0x70, 12, 0, 0x78, 0x56, 0x34, 0x12, 0x00, 0xff, 0x80, 0x7f, 0x00]
        );
        let frame = ClientFrame::new(HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), &encoded[1..]);
        let decoded = GuildSymbolUpload::decode(&frame).unwrap();
        assert_eq!(decoded, upload);
        assert_eq!(decoded.symbol_bytes(), symbol);
    }

    #[test]
    fn guild_symbol_decoder_waits_for_all_prefix_and_payload_bytes() {
        let raw = guild_symbol_raw(12, 5);
        let mut decoder = VariableClientFrameDecoder::new();
        for (index, byte) in raw.iter().enumerate() {
            decoder.feed(std::slice::from_ref(byte)).unwrap();
            let result = decoder.try_decode().unwrap();
            if index + 1 == raw.len() {
                let frame = result.expect("the final symbol byte completes the frame");
                assert_eq!(frame.header, HEADER_CG_GUILD_SYMBOL_UPLOAD.value());
                assert_eq!(frame.payload.len(), 11);
            } else {
                assert!(result.is_none());
            }
        }
        assert!(decoder.is_empty());
    }

    #[test]
    fn guild_symbol_decoder_consumes_exact_declared_size_before_next_frame() {
        let raw = guild_symbol_raw(9, 2);
        let mut stream = raw;
        stream.push(0); // legacy keepalive frame
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&stream).unwrap();
        let first = decoder.try_decode().unwrap().unwrap();
        assert_eq!(first.header, HEADER_CG_GUILD_SYMBOL_UPLOAD.value());
        assert_eq!(first.payload.len(), 8);
        let next = decoder.try_decode().unwrap().unwrap();
        assert_eq!(next.header, 0);
        assert!(next.payload.is_empty());
        assert!(decoder.is_empty());
    }

    #[test]
    fn guild_symbol_resolver_rejects_short_and_oversized_declarations() {
        let mut short = VariableClientFrameDecoder::new();
        short
            .feed(&[HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), 6, 0, 0, 0, 0, 0])
            .unwrap();
        assert!(matches!(
            short.try_decode(),
            Err(ClientFrameError::InvalidVariableSize {
                header: 0x70,
                declared_size: 6,
                base_size: 7,
            })
        ));

        let mut oversized = VariableClientFrameDecoder::with_max_frame_size(8);
        oversized.feed(&guild_symbol_raw(9, 2)).unwrap();
        assert!(matches!(
            oversized.try_decode(),
            Err(ClientFrameError::FrameTooLarge {
                size: 9,
                maximum: 8,
            })
        ));
    }

    #[test]
    fn guild_symbol_finish_reports_the_exact_expected_total() {
        let raw = guild_symbol_raw(12, 5);
        let mut decoder = VariableClientFrameDecoder::new();
        decoder
            .feed(&raw[..GUILD_SYMBOL_UPLOAD_BASE_WIRE_SIZE])
            .unwrap();
        assert!(matches!(
            decoder.finish(),
            Err(ClientFrameError::Truncated {
                header: 0x70,
                expected: 12,
                available: 7,
            })
        ));
    }

    #[test]
    fn guild_symbol_typed_decoder_preserves_frame_errors_and_semantic_limits() {
        let wrong_header = ClientFrame::new(HEADER_CG_MYSHOP.value(), [7, 0, 0, 0, 0, 0]);
        assert!(matches!(
            GuildSymbolUpload::decode(&wrong_header),
            Err(GuildSymbolUploadError::UnexpectedHeader {
                expected: 0x70,
                actual: 0x37,
            })
        ));

        let short = ClientFrame::new(HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), [7, 0]);
        assert!(matches!(
            GuildSymbolUpload::decode(&short),
            Err(GuildSymbolUploadError::Frame(ClientFrameError::Truncated {
                header: 0x70,
                expected: 7,
                available: 3,
            }))
        ));

        let declared_short =
            ClientFrame::new(HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), [6, 0, 0, 0, 0, 0]);
        assert!(matches!(
            GuildSymbolUpload::decode(&declared_short),
            Err(GuildSymbolUploadError::Frame(
                ClientFrameError::InvalidVariableSize {
                    header: 0x70,
                    declared_size: 6,
                    base_size: 7,
                }
            ))
        ));

        let mismatch = ClientFrame::new(
            HEADER_CG_GUILD_SYMBOL_UPLOAD.value(),
            [8, 0, 1, 2, 3, 4, 5, 6],
        );
        assert!(matches!(
            GuildSymbolUpload::decode(&mismatch),
            Err(GuildSymbolUploadError::Frame(
                ClientFrameError::LengthMismatch {
                    header: 0x70,
                    expected: 8,
                    actual: 9,
                }
            ))
        ));

        let empty = ClientFrame::new(HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), [7, 0, 1, 2, 3, 4]);
        assert_eq!(
            GuildSymbolUpload::decode(&empty),
            Err(GuildSymbolUploadError::EmptySymbol)
        );

        let too_large = vec![0_u8; GUILD_SYMBOL_MAX_SOURCE_BYTES + 1];
        assert!(matches!(
            GuildSymbolUpload::new(1, &too_large),
            Err(GuildSymbolUploadError::SymbolTooLarge {
                length: 65_537,
                maximum: GUILD_SYMBOL_MAX_SOURCE_BYTES,
            })
        ));
    }

    #[test]
    fn fixed_decoder_still_rejects_guild_symbol_until_variable_decoder_is_selected() {
        let mut decoder = ClientFrameDecoder::new();
        decoder
            .feed(&[HEADER_CG_GUILD_SYMBOL_UPLOAD.value(), 0, 0, 0, 0, 0, 0])
            .unwrap();
        assert!(matches!(
            decoder.try_decode(),
            Err(ClientFrameError::VariableLengthUnsupported {
                header: 0x70,
                base_size: 7,
            })
        ));
    }

    fn size_prefix(header: u8, size: u16) -> Vec<u8> {
        let mut bytes = vec![header, 0, 0];
        bytes[1..3].copy_from_slice(&size.to_le_bytes());
        bytes.resize(usize::from(size), 0);
        bytes
    }

    #[test]
    fn fixed_keepalive_and_fixed_frames_share_the_decoder() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder
            .feed(&[0, 0xff, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])
            .unwrap();
        assert_eq!(decoder.try_decode().unwrap().unwrap().header, 0);
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.header, 0xff);
        assert_eq!(frame.payload, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn chat_is_decoded_after_bytewise_fragmentation() {
        let mut decoder = VariableClientFrameDecoder::new();
        let frame_bytes = size_prefix(HEADER_CG_CHAT.value(), 7);
        for byte in &frame_bytes {
            decoder.feed(std::slice::from_ref(byte)).unwrap();
            if byte != frame_bytes.last().unwrap() {
                assert!(decoder.try_decode().unwrap().is_none());
            }
        }
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.header, HEADER_CG_CHAT.value());
        assert_eq!(frame.payload, vec![7, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn whisper_and_sync_sizes_are_little_endian_u16() {
        let mut decoder = VariableClientFrameDecoder::new();
        let whisper = size_prefix(HEADER_CG_WHISPER.value(), 30);
        let sync = size_prefix(HEADER_CG_SYNC_POSITION.value(), 15);
        decoder.feed(&whisper).unwrap();
        decoder.feed(&sync).unwrap();
        let whisper_frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(whisper_frame.payload.len(), 29);
        let sync_frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(sync_frame.payload.len(), 14);
    }

    #[test]
    fn shop_subheaders_resolve_to_the_source_extra_lengths() {
        let cases = [
            (SHOP_SUBHEADER_END, 2_usize),
            (SHOP_SUBHEADER_BUY, 4),
            (SHOP_SUBHEADER_SELL, 3),
            (SHOP_SUBHEADER_SELL2, 6),
            (0xff, 2),
        ];
        let mut stream = Vec::new();
        for (subheader, total) in cases {
            let mut frame = vec![HEADER_CG_SHOP.value(), subheader];
            frame.resize(total, 0xa5);
            stream.extend_from_slice(&frame);
        }
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&stream).unwrap();
        for (subheader, total) in cases {
            let frame = decoder.try_decode().unwrap().unwrap();
            assert_eq!(frame.header, HEADER_CG_SHOP.value());
            assert_eq!(frame.payload.len(), total - 1);
            assert_eq!(frame.payload[0], subheader);
            if subheader == SHOP_SUBHEADER_SELL2 {
                // The uninitialized legacy padding byte remains observable
                // wire data; this framing layer must not normalize it.
                assert_eq!(frame.payload[2], 0xa5);
            }
        }
        assert!(decoder.is_empty());
    }

    #[test]
    fn shop_frames_decode_after_bytewise_fragmentation() {
        let cases = [
            (SHOP_SUBHEADER_END, 2_usize),
            (SHOP_SUBHEADER_BUY, 4),
            (SHOP_SUBHEADER_SELL, 3),
            (SHOP_SUBHEADER_SELL2, 6),
            (0xff, 2),
        ];
        for (subheader, total) in cases {
            let mut bytes = vec![HEADER_CG_SHOP.value(), subheader];
            bytes.resize(total, 0x5a);
            let mut decoder = VariableClientFrameDecoder::new();
            for (index, byte) in bytes.iter().enumerate() {
                decoder.feed(std::slice::from_ref(byte)).unwrap();
                let result_frame = decoder.try_decode().unwrap();
                if index + 1 == total {
                    assert_eq!(result_frame.unwrap().payload.len(), total - 1);
                    assert!(decoder.is_empty());
                } else {
                    assert!(result_frame.is_none());
                }
            }
        }
    }

    #[test]
    fn shop_short_extension_is_retained_until_complete() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder
            .feed(&[HEADER_CG_SHOP.value(), SHOP_SUBHEADER_BUY, 0x11])
            .unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        assert!(matches!(
            decoder.finish(),
            Err(ClientFrameError::Truncated {
                header: 0x32,
                expected: 4,
                available: 3
            })
        ));
        assert_eq!(
            resolve_variable_client_frame_size(
                HEADER_CG_SHOP.value(),
                &[HEADER_CG_SHOP.value(), SHOP_SUBHEADER_BUY]
            ),
            Ok(Some(4))
        );
    }

    #[test]
    fn shop_unknown_subheader_consumes_only_the_legacy_prefix() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder
            .feed(&[HEADER_CG_SHOP.value(), 0xff, HEADER_CG_SHOP.value(), 0])
            .unwrap();
        let unknown = decoder.try_decode().unwrap().unwrap();
        assert_eq!(unknown.payload, vec![0xff]);
        let end = decoder.try_decode().unwrap().unwrap();
        assert_eq!(end.payload, vec![0]);
    }

    #[test]
    fn messenger_subheaders_resolve_to_source_lengths() {
        let header = HEADER_CG_MESSENGER.value();
        let cases = [
            (MESSENGER_SUBHEADER_ADD_BY_VID, 6_usize),
            (MESSENGER_SUBHEADER_ADD_BY_NAME, 26),
            (MESSENGER_SUBHEADER_REMOVE, 26),
            (3, 2),
            (0xff, 2),
        ];

        for (subheader, total) in cases {
            assert_eq!(
                resolve_variable_client_frame_size(header, &[header]),
                Ok(None)
            );
            assert_eq!(
                resolve_variable_client_frame_size(header, &[header, subheader]),
                Ok(Some(total))
            );
        }
    }

    #[test]
    fn messenger_extensions_wait_for_all_source_bytes() {
        let header = HEADER_CG_MESSENGER.value();
        for (subheader, total) in [
            (MESSENGER_SUBHEADER_ADD_BY_VID, 6_usize),
            (MESSENGER_SUBHEADER_ADD_BY_NAME, 26),
            (MESSENGER_SUBHEADER_REMOVE, 26),
        ] {
            let mut bytes = vec![header, subheader];
            bytes.resize(total, 0xa5);
            let mut decoder = VariableClientFrameDecoder::new();
            for (index, byte) in bytes.iter().enumerate() {
                decoder.feed(std::slice::from_ref(byte)).unwrap();
                let result = decoder.try_decode().unwrap();
                if index + 1 == total {
                    let frame = result.expect("the final extension byte completes the frame");
                    assert_eq!(frame.header, header);
                    assert_eq!(frame.payload.len(), total - 1);
                    assert_eq!(frame.payload[0], subheader);
                    assert!(decoder.is_empty());
                } else {
                    assert!(result.is_none());
                    assert_eq!(decoder.buffered_len(), index + 1);
                }
            }
        }
    }

    #[test]
    fn messenger_unknown_subheader_consumes_only_the_base_and_keeps_next_frame() {
        let header = HEADER_CG_MESSENGER.value();
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&[header, 0xff, 0]).unwrap();
        let unknown = decoder.try_decode().unwrap().unwrap();
        assert_eq!(unknown.header, header);
        assert_eq!(unknown.payload, vec![0xff]);
        let next = decoder.try_decode().unwrap().unwrap();
        assert_eq!(next.header, 0);
        assert!(next.payload.is_empty());
        assert!(decoder.is_empty());
    }

    #[test]
    fn messenger_supported_extension_drains_before_the_next_frame() {
        let header = HEADER_CG_MESSENGER.value();
        let mut bytes = vec![
            header,
            MESSENGER_SUBHEADER_ADD_BY_VID,
            0x78,
            0x56,
            0x34,
            0x12,
        ];
        bytes.extend_from_slice(&[0]);
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&bytes).unwrap();
        let first = decoder.try_decode().unwrap().unwrap();
        assert_eq!(first.header, header);
        assert_eq!(first.payload.len(), 5);
        assert_eq!(&first.payload[1..], &[0x78, 0x56, 0x34, 0x12]);
        let next = decoder.try_decode().unwrap().unwrap();
        assert_eq!(next.header, 0);
        assert!(next.payload.is_empty());
        assert!(decoder.is_empty());
    }

    #[test]
    fn messenger_eof_distinguishes_missing_base_and_suffix() {
        let header = HEADER_CG_MESSENGER.value();
        let cases = [
            (vec![header], 2),
            (vec![header, MESSENGER_SUBHEADER_ADD_BY_VID], 6),
            (vec![header, MESSENGER_SUBHEADER_ADD_BY_NAME], 26),
        ];
        for (bytes, expected) in cases {
            let mut decoder = VariableClientFrameDecoder::new();
            decoder.feed(&bytes).unwrap();
            assert!(matches!(
                decoder.finish(),
                Err(ClientFrameError::Truncated {
                    header: actual,
                    expected: size,
                    available,
                }) if actual == header && size == expected && available == bytes.len()
            ));
        }
    }

    #[test]
    fn fixed_decoder_still_rejects_messenger_until_transport_adapted() {
        let mut decoder = ClientFrameDecoder::new();
        decoder.feed(&[HEADER_CG_MESSENGER.value(), 0]).unwrap();
        assert!(matches!(
            decoder.try_decode(),
            Err(ClientFrameError::VariableLengthUnsupported {
                header: 0x43,
                base_size: 2
            })
        ));
    }

    #[test]
    fn fish_event_box_and_shape_subheaders_resolve_after_fragmentation() {
        let box_bytes = [HEADER_CG_FISH_EVENT_SEND.value(), 0, 7, 0x34, 0x12];
        let shape_bytes = [HEADER_CG_FISH_EVENT_SEND.value(), 1, 9];
        let mut decoder = VariableClientFrameDecoder::new();
        for byte in &box_bytes {
            decoder.feed(std::slice::from_ref(byte)).unwrap();
            if byte != box_bytes.last().unwrap() {
                assert!(decoder.try_decode().unwrap().is_none());
            }
        }
        let box_frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(box_frame.header, HEADER_CG_FISH_EVENT_SEND.value());
        assert_eq!(box_frame.payload, vec![0, 7, 0x34, 0x12]);

        decoder.feed(&shape_bytes).unwrap();
        let shape_frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(shape_frame.payload, vec![1, 9]);
    }

    #[test]
    fn unknown_fish_event_subheader_consumes_only_the_legacy_base_prefix() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder
            .feed(&[HEADER_CG_FISH_EVENT_SEND.value(), 0xff])
            .unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.payload, vec![0xff]);
        assert!(decoder.is_empty());
        assert_eq!(
            resolve_variable_client_frame_size(HEADER_CG_FISH_EVENT_SEND.value(), &[0xdb, 0xff]),
            Ok(Some(2))
        );
    }

    #[test]
    fn fish_event_short_prefix_and_coalesced_following_frame_are_preserved() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&[HEADER_CG_FISH_EVENT_SEND.value()]).unwrap();
        assert!(decoder.try_decode().unwrap().is_none());
        decoder.feed(&[0xff, 0x36, 0x11]).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.payload, vec![0xff]);
        let next = decoder.try_decode().unwrap().unwrap();
        assert_eq!(next.header, 0x36);
        assert_eq!(next.payload, vec![0x11]);

        let mut fixed = ClientFrameDecoder::new();
        fixed.feed(&[HEADER_CG_FISH_EVENT_SEND.value(), 0]).unwrap();
        assert!(matches!(
            fixed.try_decode(),
            Err(ClientFrameError::VariableLengthUnsupported {
                header: 0xdb,
                base_size: 2
            })
        ));
    }

    fn sync_frame(element_count: usize) -> ClientFrame {
        let total = 3 + element_count * SYNC_POSITION_ELEMENT_WIRE_SIZE;
        let mut payload = vec![0_u8; total - 1];
        let total_u16 = u16::try_from(total).unwrap();
        payload[0..2].copy_from_slice(&total_u16.to_le_bytes());
        for index in 0..element_count {
            let start = 2 + index * SYNC_POSITION_ELEMENT_WIRE_SIZE;
            let value = u32::try_from(index).unwrap().wrapping_add(1);
            let signed = i32::try_from(value).unwrap();
            payload[start..start + 4].copy_from_slice(&value.to_le_bytes());
            payload[start + 4..start + 8].copy_from_slice(&(-signed).to_le_bytes());
            payload[start + 8..start + 12].copy_from_slice(&(signed * 2).to_le_bytes());
        }
        ClientFrame::new(HEADER_CG_SYNC_POSITION.value(), payload)
    }

    #[test]
    fn typed_sync_decoder_keeps_all_elements_including_zero_and_large_counts() {
        for count in [0, 1, 16, 17] {
            let frame = sync_frame(count);
            let decoded = decode_sync_position(&frame).unwrap();
            assert_eq!(
                decoded.declared_size,
                3 + count * SYNC_POSITION_ELEMENT_WIRE_SIZE
            );
            assert_eq!(decoded.elements.len(), count);
            if count > 0 {
                assert_eq!(decoded.elements[0].vid, 1);
                assert_eq!(decoded.elements[0].x, -1);
                assert_eq!(decoded.elements[0].y, 2);
            }
        }
    }

    #[test]
    fn typed_sync_decoder_reports_nonaligned_and_inconsistent_frames() {
        let mut nonaligned = sync_frame(0);
        nonaligned.payload.push(0);
        nonaligned.payload[0..2].copy_from_slice(&4_u16.to_le_bytes());
        assert_eq!(
            decode_sync_position(&nonaligned),
            Err(SyncPositionDecodeError::NonElementAligned {
                declared_size: 4,
                payload_len: 1,
            })
        );

        let mut mismatch = sync_frame(1);
        mismatch.payload[0..2].copy_from_slice(&99_u16.to_le_bytes());
        assert_eq!(
            decode_sync_position(&mismatch),
            Err(SyncPositionDecodeError::DeclaredLengthMismatch {
                declared_size: 99,
                actual_size: 15,
            })
        );
    }

    #[test]
    fn typed_sync_decoder_checks_header_and_prefix_width() {
        let wrong_header = ClientFrame::new(HEADER_CG_CHAT.value(), [3, 0]);
        assert_eq!(
            decode_sync_position(&wrong_header),
            Err(SyncPositionDecodeError::UnexpectedHeader {
                expected: 0x08,
                actual: 0x03,
            })
        );
        let short = ClientFrame::new(HEADER_CG_SYNC_POSITION.value(), [3]);
        assert_eq!(
            decode_sync_position(&short),
            Err(SyncPositionDecodeError::TruncatedPrefix { available: 1 })
        );
    }

    #[test]
    fn myshop_count_uses_the_explicit_active_wire_record_size() {
        let mut prefix = vec![HEADER_CG_MYSHOP.value(); MY_SHOP_BASE_WIRE_SIZE];
        prefix[MY_SHOP_BASE_WIRE_SIZE - 1] = 2;
        let total = MY_SHOP_BASE_WIRE_SIZE + 2 * MY_SHOP_ITEM_WIRE_SIZE;
        prefix.resize(total, 0);
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&prefix).unwrap();
        let frame = decoder.try_decode().unwrap().unwrap();
        assert_eq!(frame.payload.len(), total - 1);
    }

    #[test]
    fn invalid_short_declarations_are_rejected_before_waiting() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&[HEADER_CG_CHAT.value(), 3, 0, 0]).unwrap();
        assert!(matches!(
            decoder.try_decode(),
            Err(ClientFrameError::InvalidVariableSize {
                header: 0x03,
                declared_size: 3,
                base_size: 4,
            })
        ));
        assert_eq!(decoder.buffered_len(), 4);
    }

    #[test]
    fn variable_headers_without_a_resolver_remain_explicitly_unsupported() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&[HEADER_CG_GUILD.value(), 0]).unwrap();
        assert!(matches!(
            decoder.try_decode(),
            Err(ClientFrameError::VariableLengthUnsupported { .. })
        ));
    }

    #[test]
    fn maximum_is_checked_before_allocating_a_large_payload() {
        let mut decoder = VariableClientFrameDecoder::with_max_frame_size(8);
        decoder
            .feed(&size_prefix(HEADER_CG_CHAT.value(), 9))
            .unwrap();
        assert!(matches!(
            decoder.try_decode(),
            Err(ClientFrameError::FrameTooLarge {
                size: 9,
                maximum: 8,
            })
        ));
    }

    #[test]
    fn finish_distinguishes_missing_prefix_from_missing_suffix() {
        let mut decoder = VariableClientFrameDecoder::new();
        decoder.feed(&[HEADER_CG_CHAT.value(), 7]).unwrap();
        assert!(matches!(
            decoder.finish(),
            Err(ClientFrameError::Truncated {
                header: 0x03,
                expected: 4,
                available: 2,
            })
        ));
        decoder.clear();
        decoder.feed(&[HEADER_CG_CHAT.value(), 7, 0, 0, 9]).unwrap();
        assert!(matches!(
            decoder.finish(),
            Err(ClientFrameError::Truncated {
                header: 0x03,
                expected: 7,
                available: 5,
            })
        ));
    }

    #[test]
    fn aliases_are_available_for_compatibility_callers() {
        let _: ClientFrameVariableDecoder = ClientFrameVariableDecoder::new();
        let _: LegacyVariableClientFrameDecoder = LegacyVariableClientFrameDecoder::default();
    }

    #[test]
    fn variable_resolver_handles_fixed_headers_without_a_prefix() {
        assert_eq!(resolve_variable_client_frame_size(0, &[]).unwrap(), Some(1));
        assert_eq!(
            resolve_variable_client_frame_size(HEADER_CG_CHAT.value(), &[3, 0]).unwrap(),
            None
        );
    }
}
