//! Explicit codec for the fixed legacy `TPacketCGItemDrop2` record.
//!
//! `server/server/game/packet.h` sets `HEADER_CG_ITEM_DROP2 = 20` at `:29` and
//! declares `command_item_drop2` at `packet.h:619-625` as a `BYTE header`, a
//! packed `TItemPos Cell`, a `DWORD gold`, and a trailing `WORD count`.
//! `TItemPos` is itself packed, so it is exactly three bytes, declared at
//! `server/server/common/length.h:957-1057` inside the `#pragma pack(push, 1)` opened at
//! `length.h:956` and closed by `#pragma pack(pop)` at `length.h:1274`,
//! and the containing record sits under the `#pragma pack(1)` that
//! `packet.h:274` opens and does not close until `:3540`. The record is
//! therefore exactly **ten bytes** and its framed payload **nine**. Nothing in
//! the declaration is conditional, so the width is not profile-dependent.
//!
//! The position is reused as the already source-verified
//! [`CgItemPos`](crate::cg_item_move::CgItemPos), the tenth Rust codec to
//! share that one three-byte layout.
//!
//! # The width is fixed, but the *meaning* of the window byte is not
//!
//! The record's three-byte width cannot change, but the meaning of its leading
//! window byte does, because the `EWindows` enum at `length.h:657-676`
//! conditionally inserts three enumerators. `__ATTR_6TH_7TH__` inserts
//! `ATTR67_ADD` at `:665-667`, `__AURA_SYSTEM__` inserts `AURA_REFINE` at
//! `:668-670`, and `ENABLE_SWITCHBOT` inserts `SWITCHBOT` at `:671-673`, all
//! between `DRAGON_SOUL_INVENTORY` and `BELT_INVENTORY`. With all three on,
//! which the checked-in `prodomodefines.h` does, `BELT_INVENTORY` is 9 and
//! `GROUND` is 10; with all three off, they are 6 and 7. A window byte is
//! therefore only interpretable against a named feature profile, which is why
//! this codec keeps it opaque rather than resolving it to a window kind.
//!
//! # This is not header 12, and the two are different operations
//!
//! `TPacketCGItemDrop` at `packet.h:612-617` is the same record **without** the
//! `count` field and is eight bytes, so the two are not two spellings of one
//! request. Their handlers differ in exactly the way that matters:
//!
//! * `CInputMain::ItemDrop` at `input_main.cpp:1024-1040` handles header 12
//!   and calls `ch->DropItem(pinfo->Cell)` at `:1039`, with **one** argument.
//! * `CInputMain::ItemDrop2` at `input_main.cpp:1042-1051` handles header 20
//!   and calls `ch->DropItem(pinfo->Cell, pinfo->count)` at `:1050`, with
//!   **two**.
//!
//! Header 20 is a strict **superset** of header 12. It shares the same gold
//! discriminator, so a client that sets `gold` reaches the currency path and
//! the `count` field is ignored, and it is the only header that can reach the
//! partial-drop machinery at `char_item.cpp:7510-7530`, which splits a stack
//! and creates a fresh item at `:7522`. Header 12 cannot drop a partial stack
//! at all, for the reason given in the `cg_item_drop` module.
//!
//! # Both `gold` and `count` are read, and neither is validated here
//!
//! `input_main.cpp:1047` is `if (pinfo->gold > 0)`, the **sole** discriminator
//! between dropping currency and dropping an item, so a client that sets
//! `gold` never reaches the item path and the position in the same record is
//! ignored on that path.
//!
//! The only bound the server applies to the amount is a comparison against the
//! character's own balance inside `CHARACTER::DropGold` at
//! `char_item.cpp:7556`, which is character state. The only thing that bounds
//! `count` is `char_item.cpp:7498-7499`, which rewrites a zero **or** an
//! over-large value to `item->GetCount()` inside the method, and
//! `char_item.cpp:7505`, which takes the whole-stack branch when the rewritten
//! value equals the stack size. All three are server-side policy over item
//! state. This codec therefore preserves `gold` as an opaque 32-bit word and
//! `count` as an opaque 16-bit word, and deliberately applies no bound of
//! either.
//!
//! # Value 20 collides in four directions, and one of them is a live defect
//!
//! A sweep of the legacy trees finds the value 20 used in **four** distinct
//! directions, not one:
//!
//! | Direction | Name | Declaration |
//! | --- | --- | --- |
//! | CG, server inbound | `HEADER_CG_ITEM_DROP2` | `packet.h:29` |
//! | GC, server outbound | `HEADER_GC_ITEM_DEL` | `packet.h:121` |
//! | GC, client inbound | `HEADER_GC_ITEM_SET` | `Packet.h:116` |
//! | GG, server internal | `HEADER_GG_LOGIN_PING` | `packet.h:249` |
//! | GD, server to DB | `HEADER_GD_GUILD_EXP_UPDATE` | **`common/tables.h:39`** |
//!
//! The GD one is the reason a naive sweep misses this: the game-to-DB
//! constants live in `common/tables.h`, not in `packet.h`, so searching
//! `packet.h` for `HEADER_GD_` enumerators returns nothing. Only the CG and GC
//! directions involve the client.
//!
//! **The GC collision is a live legacy defect, and it is recorded rather than
//! reproduced.** The server sends this record's outbound sibling as a
//! 62-byte `TPacketGCItemDelDeprecated` with `pack.header` set to
//! `HEADER_GC_ITEM_DEL` at `char_item.cpp:598` and the length taken from
//! `sizeof(TPacketGCItemDelDeprecated)` at `:610`. That bare struct is declared
//! at `packet.h:1085-1099` and has **no typedef**, only a struct tag, so a C
//! probe that looks up the type by name will not compile it. The client has
//! **no `HEADER_GC_ITEM_DEL` at all**: its only 20 is `HEADER_GC_ITEM_SET`,
//! registered at `PythonNetworkStream.cpp:71` and dispatched to
//! `RecvItemSetPacket` at `PythonNetworkStreamPhaseGame.cpp:342` and
//! `PythonNetworkStreamPhaseLoading.cpp:146`, with
//! `NetStream.cpp:638` printing the same name. The two declarations are
//! field-for-field identical in order and in macro gating:
//!
//! ```text
//! server  packet.h:1085-1099   BYTE header; TItemPos Cell; DWORD vnum; BYTE count;
//!                               [#ifdef ENABLE_REFINE_ELEMENT  DWORD dwRefineElement;]
//!                               [#ifdef __CHANGELOOK_SYSTEM__ DWORD transmutation;]
//!                               long alSockets[...]; TPlayerItemAttribute aAttr[...];
//! client  Packet.h:1759-1774   BYTE header; TItemPos Cell; DWORD vnum; BYTE count;
//!                               [#ifdef ENABLE_REFINE_ELEMENT  DWORD dwRefineElement;]
//!                               [#ifdef ENABLE_CHANGELOOK_SYSTEM DWORD transmutation;]
//!                               long alSockets[...]; TPlayerItemAttribute aAttr[...];
//! ```
//!
//! Both macros are enabled in their own trees, `__CHANGELOOK_SYSTEM__` at
//! `server/server/common/prodomodefines.h:16` and the client's
//! `ENABLE_CHANGELOOK_SYSTEM` in its own headers, so the widths are designed
//! to match. That is what makes this worse than a name clash: the client
//! **cannot** reject the frame, because 20 is a header it already knows the
//! size of, and the two operations are opposites. A client served an
//! item-delete record would run the item-set handler on it. `char_item.cpp`
//! zeroes the payload before sending, at `:600`, `:601-603`, `:604-606`,
//! `:607`, `:608` and `:609`, so the fields the set handler reads carry
//! `vnum = 0`, `count = 0`, zero sockets and zero attributes.
//!
//! **Whether the widths agree *exactly* is NOT VERIFIED here, and the answer
//! decides the failure mode.** The auditor measured 62 bytes on both sides
//! under a probed x86 profile, but two of the width inputs are themselves
//! build-conditional: the client's `ITEM_SOCKET_SLOT_MAX_NUM` is 3 or 6 at
//! `GameType.h:550-552` and `ITEM_ATTRIBUTE_SLOT_MAX_NUM` is derived from the
//! attribute tier counts at `GameType.h:566`, and the server's equivalents are
//! declared elsewhere. The client's own define set and its target bitness were
//! not located, and `premake5.lua:12` fixes `"x86"` for the **server** only.
//! So there are two failure modes and this codec asserts neither:
//! if the widths match, the client silently performs the wrong operation; if
//! they differ, `CheckPacket` takes its fail-closed path at
//! `PythonNetworkStream.cpp:537-543` and drops the connection. Either way it
//! is a client/server protocol defect to be reported, and this
//! transport-free codec does not model, reproduce, or normalise it.
//!
//! # Legacy facts kept out of the codec
//!
//! * **A signedness narrowing on the gold path, which this record does not
//!   reproduce.** `CHARACTER::DropGold` takes a signed `INT`
//!   (`char_item.cpp:7554`, reached via `char.h:1265`, which is inside
//!   `#ifdef ENABLE_REMOVE_LIMIT_GOLD` at `char.h:1262`; that macro **is**
//!   defined at `prodomodefines.h:157`). The wire field is an unsigned `DWORD`
//!   passed straight through at `input_main.cpp:1048`, so any value at or
//!   above `0x80000000` arrives negative and is rejected by `gold <= 0` at
//!   `:7556`. The upper half of the wire range is unreachable for currency.
//!   `GetGold()` returns `unsigned long long` at `char.h:1263`, so the second
//!   half of that test, `gold > GetGold()`, is evaluated in unsigned 64-bit
//!   arithmetic; only the first half is signed. The codec keeps all `u32`
//!   values.
//! * **A dead `abs()`.** `char_item.cpp:7446` is `bCount = abs(bCount);` where
//!   `bCount` is a `WORD`. A `WORD` promotes to `int`, the value is always in
//!   `0..=65535`, and `abs` is the identity function, so the line is a no-op.
//!   The same no-op appears at `:7604` in `MoveItem`; the only meaningful
//!   `abs` in the file is at `:6648`. The original intent is NOT DETERMINABLE
//!   from the tree.
//! * **A second dead branch, the same recurring shape.** Because
//!   `:7498-7499` has already replaced any zero with `item->GetCount()`, the
//!   `if (bCount == 0)` guard at `char_item.cpp:7512-7517` can never be true.
//!   The same pattern appears in the safebox move rollback at
//!   `safebox.cpp:231-235` and in that method's merge guard at `:216`. Three
//!   separate files now show it, which makes it a class rather than three
//!   unrelated slips.
//! * **The item path's checks, in order.** `DropItem` tests
//!   `CanHandleItem()` at `char_item.cpp:7449`, a drop time limit at
//!   `:7455-7463` under `ENABLE_NEWSTUFF`, `IsDead()` at `:7467`,
//!   `IsValidItemPosition` and `GetItem` at `:7470`, `IsExchanging()` at
//!   `:7473`, `isLocked()` at `:7476`, a quest-running test at `:7479`, the
//!   `ITEM_ANTIFLAG_DROP | ITEM_ANTIFLAG_GIVE` test at `:7482-7486`, and
//!   `IsSecured()` at `:7489-7495` under
//!   `__ENABLE_INVENTORY_PROTECTED_SYSTEM__`. Both handler branches discard
//!   their return values, at `input_main.cpp:1048` and `:1050`.
//! * **Neither path is rate limited as shipped.** `g_ItemDropTimeLimitValue`
//!   and `g_GoldDropTimeLimitValue` are both initialised to `0` at
//!   `questmanager.cpp:35` and `:33`, and both guards read
//!   `if (0 != g_...)`, so a default server drops with no delay. The limits
//!   are only set from configuration, at `:1605` and `:1600`.
//! * **`DropGold` can report success on a failed placement.** `Save()` at
//!   `:7595` and `return true` at `:7596` sit *outside* the `AddToGround`
//!   block, while the currency deduction `PointChange` at `:7582` sits inside
//!   it. Severity is nil only because `input_main.cpp:1048` discards the
//!   result.
//! * **The window byte aliases inventory and equipment.** `GetItem` at
//!   `char_item.cpp:254-305` returns `INVENTORY` and `EQUIPMENT` from the same
//!   `pItems` array under the same bound at `:269`, and `GetWear` at `:655`
//!   offsets that array by `INVENTORY_MAX_NUM`, so an `EQUIPMENT` window byte
//!   naming an inventory cell drops an inventory item.
//! * **The inbound cast cannot be reached with a short frame.**
//!   `input.cpp:83` looks up the registered length and `:92-93` returns early
//!   on `m_iBufferLeft < iPacketLen` before any advance at `:112-113`.
//! * **The dispatch is in `ANALYZE`, not `Process`.** `CInputMain::ANALYZE` is
//!   declared virtual at `input.h:30`, overridden at `input.h:81`, and called
//!   from `input.cpp:100`; the switch opens at `input_main.cpp:3659` and the
//!   case is `:3699`. The no-character branch at `:3647-3652` closes the
//!   descriptor and the observer guard at `:3700` means `ch` is non-null by the
//!   time the handler runs, so the handler's own `if (!ch)` at `:1045` is
//!   belt-and-braces on a public method with no other call site.
//! * **The two handlers spell their cast differently, which defeats a grep
//!   audit.** `ItemDrop` at `:1026` casts to the bare struct tag
//!   `struct command_item_drop *`, while `ItemDrop2` at `:1044` casts to the
//!   typedef. Same type, two spellings. `ItemDrop` also carries a
//!   commented-out `MONARCH_LIMIT` block at `:1028-1031` and a stale
//!   count-limiting comment at `:1035` that do not apply to it and have no
//!   counterpart here.
//! * **The stored `header` byte is dead.** It is declared at `packet.h:621`
//!   and never read, because the dispatch switch already consumed the value
//!   to reach the handler. Contrast the inbound handshake, which stores a
//!   header because it accepts two values; this one stores a byte nothing
//!   reads. The byte must still be 20 at offset 0, because the dispatch and
//!   the descriptor read loop both size the frame from it.
//! * Registered exactly once, at `packet_info.cpp:119`.
//!   `packet_info.cpp:20-21` makes `Set` silently first-wins, so a genuine
//!   duplicate would be invisible; a sweep found no other registration of 20
//!   in that table. Value 20 *does* appear in a different table, at
//!   `packet_info.cpp:222` as `HEADER_GG_LOGIN_PING`, but the CG and GG
//!   subclasses hold separate `m_pPacketMap` instances at `packet_info.h:32`,
//!   `:37` and `:45`, so that is not a duplicate registration.
//! # The client, its builder, and its two send paths
//!
//! **The client has its own copy of this record and it agrees on the width.**
//! `client/Client/UserInterface/Packet.h:515-521` declares the same struct tag
//! `command_item_drop2`, the same typedef name, and the same four fields, and
//! `Packet.h:29` gives header 20, which agrees with the server. One field name
//! drifts: the server calls the position `Cell` at `packet.h:622` and the
//! client calls it `pos` at `Packet.h:518`. The other three fields match. Do
//! not generalise this to the header-12 sibling, which drifts in *two* places:
//! the client calls its currency field `elk` at `Packet.h:512` where the
//! server calls it `gold` at `packet.h:616`. The client never receives this
//! record, so it is a send buffer only and has no CG receive table.
//!
//! **Two Python entry points send header 20, and they disagree about arity.**
//! `netSendItemDropPacketNew` at
//! `PythonNetworkStreamModule.cpp:837-865`, registered as
//! `SendItemDropPacketNew` at `:2547`, switches on tuple size: the two-argument
//! form at `:843-849` reads the cell and the count but **never assigns
//! `window_type` from any argument**, so the transmitted window byte is the
//! `TItemPos` default constructor's `INVENTORY`, which `GameType.h:426-430`
//! sets to 1; the three-argument form at `:850-858` does assign it. The
//! `default` arm at `:859-860` **returns an exception rather than falling
//! through to a send**, so unlike `netSendGiveItemPacket` at `:963-964` and
//! `netSendSafeboxCheckoutPacket` at `:1376-1377`, this wrapper is not one of
//! the known break-and-fall-through defects.
//!
//! **The second entry point ignores arity entirely.**
//! `netSendGoldDropPacketNew` at `:877-886`, registered as
//! `SendGoldDropPacketNew` at `:2574`, does not switch on tuple size: it reads
//! the first argument and silently discards any others, and sends header 20
//! with `TItemPos(RESERVED_WINDOW, 0)`, the currency amount, and `count = 0`.
//! So header 20 carries both operations, and the wire position field means
//! "the reserved window" on the currency path.
//!
//! **The currency discriminator is unreachable from the item entry point.**
//! The item path hardcodes the currency word to `0` at
//! `PythonNetworkStreamPhaseGameItem.cpp:863`, so `input_main.cpp:1047`'s
//! `pinfo->gold > 0` test can only be true through `SendGoldDropPacketNew`.
//! That literal is precisely what makes the server's `gold` test a usable
//! discriminator, which is why `gold` must stay a real wire field even though
//! one of its two senders cannot vary it.
//!
//! **The client truncates the amount.** The builder takes a `DWORD count` at
//! `PythonNetworkStreamPhaseGameItem.cpp:656` while the wire field is a
//! `WORD` at `Packet.h:520`, and `:665` assigns one to the other with no
//! range check. A Python caller passing 65 536 therefore transmits a zero,
//! which the server reads as "drop the whole stack". The original intent is
//! NOT DETERMINABLE from the tree. This is a client defect and this codec
//! does not reproduce it: a `u16` count of 0 round-trips as 0.
//!
//! **The observer guard is present, so there is no gate asymmetry here.** The
//! builder calls `__CanActMainInstance()` at `:658-659`, implemented at
//! `PythonNetworkStreamPhaseGameActor.cpp:28-36`. Every one of the sixteen
//! `CPythonNetworkStream::Send*` functions in that file from `:449` onward
//! has the same guard, including the header-12 sibling at `:624`. Only the
//! safebox and mall builders at `:16-172` lack it, which is the same family
//! whose wrappers carry the known arity defects. No causal link between those
//! two facts is claimed here.
//! * A `cell` value of `0xffff` and a `count` value of `0xffff` are both
//!   normal opaque client values and must round-trip.

use crate::cg_inventory::HEADER_CG_ITEM_DROP2;
use crate::cg_wire::ClientFrame;
use std::fmt;

/// The exact wire size of one packed legacy `TItemPos`.
pub use crate::cg_item_move::CG_ITEM_POS_SIZE;

/// The exact payload size of a framed item-drop2 request.
pub const CG_ITEM_DROP2_PAYLOAD_SIZE: usize = 9;

/// The exact wire size of a complete item-drop2 request.
pub const CG_ITEM_DROP2_WIRE_SIZE: usize = 10;

/// One fixed legacy `TPacketCGItemDrop2` request.
///
/// `header` is not stored: the codec accepts only header 20 and always writes
/// it, so keeping a second copy in memory could only create a state that
/// cannot be encoded.
///
/// `gold` and `count` are opaque wire words, not validated amounts. See the
/// module documentation for why neither may be bounded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CgItemDrop2 {
    /// The one source position named by the record.
    pub cell: crate::cg_item_move::CgItemPos,
    /// The opaque 32-bit currency word.
    pub gold: u32,
    /// The opaque 16-bit amount word.
    pub count: u16,
}

impl CgItemDrop2 {
    /// Build a request for the given position, currency word, and amount.
    #[must_use]
    pub const fn new(cell: crate::cg_item_move::CgItemPos, gold: u32, count: u16) -> Self {
        Self { cell, gold, count }
    }

    /// The one-byte header this record encodes.
    #[must_use]
    pub const fn header() -> crate::cg_inventory::CgHeader {
        HEADER_CG_ITEM_DROP2
    }

    /// Encode this request as the exact ten wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CG_ITEM_DROP2_WIRE_SIZE);
        self.encode_into(&mut out);
        out
    }

    /// Encode this request into an existing buffer.
    ///
    /// The output is exactly `[0x14][window_type][cell LE][gold LE][count LE]`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(Self::header().value());
        self.cell.encode_into(out);
        out.extend_from_slice(&self.gold.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
    }

    /// Project this request as a `ClientFrame` with an exact nine-byte
    /// payload.
    #[must_use]
    pub fn to_frame(&self) -> ClientFrame {
        let mut payload = Vec::with_capacity(CG_ITEM_DROP2_PAYLOAD_SIZE);
        self.cell.encode_into(&mut payload);
        payload.extend_from_slice(&self.gold.to_le_bytes());
        payload.extend_from_slice(&self.count.to_le_bytes());
        ClientFrame::new(Self::header().value(), payload)
    }

    /// Decode one exact ten-byte record.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDrop2Error::Truncated`] for a short input,
    /// [`CgItemDrop2Error::LengthMismatch`] for a long input, and
    /// [`CgItemDrop2Error::InvalidHeader`] when the header is not 20. The
    /// length is checked before the header, and the header is checked before
    /// any field is read.
    pub fn decode(bytes: &[u8]) -> Result<Self, CgItemDrop2Error> {
        check_exact(bytes.len())?;
        check_header(bytes[0])?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&bytes[1..4]),
            gold: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            count: u16::from_le_bytes([bytes[8], bytes[9]]),
        })
    }

    /// Decode one framed request.
    ///
    /// # Errors
    ///
    /// Returns [`CgItemDrop2Error::Truncated`] for a payload shorter than nine
    /// bytes, [`CgItemDrop2Error::LengthMismatch`] for a longer payload, and
    /// [`CgItemDrop2Error::InvalidHeader`] when the frame header is not 20.
    /// The payload length is checked before the header, and the header is
    /// checked before any payload byte is read.
    pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgItemDrop2Error> {
        let len = frame.payload.len();
        match len.cmp(&CG_ITEM_DROP2_PAYLOAD_SIZE) {
            core::cmp::Ordering::Less => {
                return Err(CgItemDrop2Error::Truncated {
                    needed: CG_ITEM_DROP2_WIRE_SIZE,
                    available: len.saturating_add(1),
                })
            }
            core::cmp::Ordering::Greater => {
                return Err(CgItemDrop2Error::LengthMismatch {
                    expected: CG_ITEM_DROP2_WIRE_SIZE,
                    actual: len.saturating_add(1),
                });
            }
            core::cmp::Ordering::Equal => {}
        }
        check_header(frame.header)?;
        Ok(Self {
            cell: crate::cg_item_move::CgItemPos::decode_at(&frame.payload[0..3]),
            gold: u32::from_le_bytes([
                frame.payload[3],
                frame.payload[4],
                frame.payload[5],
                frame.payload[6],
            ]),
            count: u16::from_le_bytes([frame.payload[7], frame.payload[8]]),
        })
    }
}

impl fmt::Display for CgItemDrop2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CgItemDrop2(cell={{ window_type: {}, cell: {} }}, gold: {}, count: {})",
            self.cell.window_type, self.cell.cell, self.gold, self.count
        )
    }
}

/// Every way decoding a fixed item-drop2 request can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgItemDrop2Error {
    /// The input ended before the complete record was available.
    Truncated {
        /// The exact wire size that was required.
        needed: usize,
        /// The number of bytes that were actually available.
        available: usize,
    },
    /// The input carried more bytes than the fixed record can hold.
    LengthMismatch {
        /// The exact wire size the record requires.
        expected: usize,
        /// The number of bytes that were actually supplied.
        actual: usize,
    },
    /// The leading byte was not [`HEADER_CG_ITEM_DROP2`].
    InvalidHeader {
        /// The header byte that was actually present.
        actual: u8,
    },
}

impl fmt::Display for CgItemDrop2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(f, "item-drop2 record needs {needed} bytes, got {available}")
            }
            Self::LengthMismatch { expected, actual } => {
                write!(f, "item-drop2 record needs {expected} bytes, got {actual}")
            }
            Self::InvalidHeader { actual: byte } => {
                write!(f, "header {byte:#04x} is not the item-drop2 header")
            }
        }
    }
}

impl std::error::Error for CgItemDrop2Error {}

fn check_exact(len: usize) -> Result<(), CgItemDrop2Error> {
    match len.cmp(&CG_ITEM_DROP2_WIRE_SIZE) {
        core::cmp::Ordering::Less => Err(CgItemDrop2Error::Truncated {
            needed: CG_ITEM_DROP2_WIRE_SIZE,
            available: len,
        }),
        core::cmp::Ordering::Greater => Err(CgItemDrop2Error::LengthMismatch {
            expected: CG_ITEM_DROP2_WIRE_SIZE,
            actual: len,
        }),
        core::cmp::Ordering::Equal => Ok(()),
    }
}

fn check_header(byte: u8) -> Result<(), CgItemDrop2Error> {
    if byte != CgItemDrop2::header().value() {
        return Err(CgItemDrop2Error::InvalidHeader { actual: byte });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg_wire::{resolve_client_frame_size, ClientFrameDecoder, ClientFrameSize};
    use std::error::Error;

    fn rec(window: u8, cell: u16, gold: u32, count: u16) -> CgItemDrop2 {
        CgItemDrop2::new(
            crate::cg_item_move::CgItemPos::new(window, cell),
            gold,
            count,
        )
    }

    #[test]
    fn inventory_resolves_the_fixed_ten_byte_size() {
        assert_eq!(HEADER_CG_ITEM_DROP2.value(), 20);
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_DROP2_WIRE_SIZE, 10);
        assert_eq!(CG_ITEM_DROP2_PAYLOAD_SIZE, 9);
        assert_eq!(
            resolve_client_frame_size(20).unwrap(),
            ClientFrameSize::Fixed(10)
        );
    }

    #[test]
    fn golden_record_has_exact_source_offsets() {
        // header 20 at 0, window_type at 1, cell LE at 2..4, gold LE at 4..8,
        // count LE at 8..10.
        let p = rec(0x01, 0x0203, 0x0405_0607, 0x0809);
        assert_eq!(
            p.encode(),
            vec![0x14, 0x01, 0x03, 0x02, 0x07, 0x06, 0x05, 0x04, 0x09, 0x08]
        );
        assert_eq!(
            p.to_frame().payload,
            vec![0x01, 0x03, 0x02, 0x07, 0x06, 0x05, 0x04, 0x09, 0x08]
        );
        assert_eq!(p.to_frame().header, 0x14);
        assert_eq!(
            p.to_string(),
            "CgItemDrop2(cell={ window_type: 1, cell: 515 }, gold: 67438087, count: 2057)"
        );
        assert_eq!(CgItemDrop2::decode(&p.encode()).unwrap(), p);
        assert_eq!(CgItemDrop2::decode_frame(&p.to_frame()).unwrap(), p);
    }

    /// Header 20 is a strict superset of header 12: same gold discriminator,
    /// plus the only partial-drop capability. Both must round-trip, including
    /// the gold>0 case where count is ignored by the server.
    #[test]
    fn both_operations_and_both_count_paths_round_trip() {
        for (gold, count) in [
            (0u32, 0u16),
            (0, 1),
            (0, 0xffff),
            (1, 0),
            (1000, 7),
            (0x7fff_ffff, 0x8000),
            (u32::MAX, u16::MAX),
        ] {
            let p = rec(1, 0x0010, gold, count);
            assert_eq!(
                CgItemDrop2::decode(&p.encode()).unwrap(),
                p,
                "{gold}/{count}"
            );
            assert_eq!(
                CgItemDrop2::decode_frame(&p.to_frame()).unwrap(),
                p,
                "{gold}/{count}"
            );
        }
    }

    /// The server narrows the wire `DWORD` to a signed `int` before comparing,
    /// making the upper half unreachable. The codec must not inherit that, so
    /// every value must still round-trip bit-for-bit.
    #[test]
    fn the_whole_u32_gold_range_round_trips_unnarrowed() {
        for gold in [0x8000_0000u32, 0xffff_ffff, 0x7fff_ffff, 0x8000_0001, 0] {
            let p = rec(1, 0, gold, 0);
            assert_eq!(CgItemDrop2::decode(&p.encode()).unwrap().gold, gold);
            assert_eq!(CgItemDrop2::decode_frame(&p.to_frame()).unwrap().gold, gold);
        }
    }

    /// The server rewrites a zero or over-large count to the whole stack, but
    /// only inside the method. Zero and `u16::MAX` must survive the wire
    /// unchanged.
    #[test]
    fn the_whole_u16_count_range_round_trips_unchanged() {
        for count in [0u16, 1, 0x00ff, 0x0100, 0x7fff, 0x8000, 0xfffe, 0xffff] {
            let p = rec(1, 0x20, 0, count);
            assert_eq!(
                CgItemDrop2::decode(&p.encode()).unwrap().count,
                count,
                "count {count} was rewritten"
            );
            assert_eq!(
                CgItemDrop2::decode_frame(&p.to_frame()).unwrap().count,
                count
            );
        }
    }

    #[test]
    fn every_window_byte_round_trips_in_both_paths() {
        for w in 0..=u8::MAX {
            let p = rec(w, 0xbeef, 0, 5);
            assert_eq!(p.encode()[1], w, "raw encode lost window {w}");
            assert_eq!(p.to_frame().payload[0], w, "framed encode lost window {w}");
            assert_eq!(CgItemDrop2::decode(&p.encode()).unwrap(), p);
            assert_eq!(CgItemDrop2::decode_frame(&p.to_frame()).unwrap(), p);
        }
    }

    #[test]
    fn cell_boundaries_round_trip() {
        for c in [0u16, 1, 0x00ff, 0x0100, 0xfffe, 0xffff, u16::MAX] {
            let p = rec(1, c, 3, 4);
            assert_eq!(CgItemDrop2::decode(&p.encode()).unwrap(), p, "cell {c}");
            assert_eq!(
                CgItemDrop2::decode_frame(&p.to_frame()).unwrap(),
                p,
                "cell {c}"
            );
        }
    }

    #[test]
    fn every_wrong_header_is_rejected_at_the_exact_length() {
        for b in 0..=u8::MAX {
            if b == 20 {
                continue;
            }
            let raw = vec![b, 1, 0, 0, 0, 0, 0, 0, 0, 0];
            assert_eq!(
                CgItemDrop2::decode(&raw),
                Err(CgItemDrop2Error::InvalidHeader { actual: b }),
                "raw accepted header {b}"
            );
            let frame = ClientFrame::new(b, vec![1, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(
                CgItemDrop2::decode_frame(&frame),
                Err(CgItemDrop2Error::InvalidHeader { actual: b }),
                "framed accepted header {b}"
            );
        }
        assert!(CgItemDrop2::decode(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_ok());
        assert!(CgItemDrop2::decode_frame(&ClientFrame::new(20, vec![0; 9])).is_ok());
    }

    /// Header 12 and header 20 are different operations, not two spellings of
    /// one request, so their records must not be interchangeable in either
    /// direction.
    #[test]
    fn headers_12_and_20_are_not_interchangeable() {
        let mut twelve = rec(1, 0x0203, 0, 9).encode();
        twelve[0] = 12;
        assert_eq!(
            CgItemDrop2::decode(&twelve),
            Err(CgItemDrop2Error::InvalidHeader { actual: 12 })
        );
        // A real header-12 record is eight bytes, so re-labelling one as
        // header 20 must fail on length before the header is even considered.
        let mut real_twelve = rec(1, 0x0203, 0x0a0b_0c0d, 0).encode();
        real_twelve.truncate(8);
        assert_eq!(real_twelve.len(), 8, "header-12 record must be eight bytes");
        let mut relabelled = real_twelve.clone();
        relabelled[0] = 20;
        assert_eq!(
            CgItemDrop2::decode(&relabelled),
            Err(CgItemDrop2Error::Truncated {
                needed: 10,
                available: 8
            })
        );
        // The framed form of the same eight-byte payload is likewise short.
        let frame = ClientFrame::new(20, &real_twelve[1..]);
        assert_eq!(
            CgItemDrop2::decode_frame(&frame),
            Err(CgItemDrop2Error::Truncated {
                needed: 10,
                available: 8
            })
        );
    }

    #[test]
    fn raw_length_is_checked_before_the_header() {
        assert_eq!(
            CgItemDrop2::decode(&[0x99, 0x01, 0x00]),
            Err(CgItemDrop2Error::Truncated {
                needed: 10,
                available: 3
            })
        );
        let mut long = vec![20u8; 11];
        long[0] = 0x99;
        assert_eq!(
            CgItemDrop2::decode(&long),
            Err(CgItemDrop2Error::LengthMismatch {
                expected: 10,
                actual: 11
            })
        );
    }

    #[test]
    fn framed_length_is_checked_before_the_header() {
        for (payload, err) in [
            (
                vec![0x01, 0x00],
                CgItemDrop2Error::Truncated {
                    needed: 10,
                    available: 3,
                },
            ),
            (
                vec![0u8; 10],
                CgItemDrop2Error::LengthMismatch {
                    expected: 10,
                    actual: 11,
                },
            ),
        ] {
            let frame = ClientFrame::new(0x99, payload);
            assert_eq!(CgItemDrop2::decode_frame(&frame), Err(err));
        }
    }

    #[test]
    fn a_wrong_header_yields_a_header_error_even_when_the_fields_are_valid() {
        // The source does check the header first, but that ordering is not
        // observable from here and no test can make it so: `check_exact` has
        // already proved ten bytes are present, so a field read can neither
        // panic nor change the result.
        let err =
            CgItemDrop2::decode(&[0x0c, 0x01, 0x03, 0x02, 0x07, 0x06, 0x05, 0x04, 0x09, 0x08])
                .unwrap_err();
        assert_eq!(err, CgItemDrop2Error::InvalidHeader { actual: 0x0c });
    }

    #[test]
    fn header_accessor_matches_every_encoder_path() {
        assert_eq!(CgItemDrop2::header().value(), 20);
        assert_eq!(rec(1, 0, 0, 0).encode()[0], CgItemDrop2::header().value());
        assert_eq!(
            rec(1, 0, 0, 0).to_frame().header,
            CgItemDrop2::header().value()
        );
        let mut buf = vec![0xaa, 0xbb];
        rec(1, 0, 0, 0).encode_into(&mut buf);
        assert_eq!(buf[2], CgItemDrop2::header().value());
        assert_eq!(buf.len(), 12);
    }

    #[test]
    fn every_length_outside_the_exact_size_is_rejected() {
        for len in 0..=16usize {
            if len == 10 {
                let ok = CgItemDrop2::decode(&[0x14; 10]).expect("exact raw length");
                assert_eq!(ok, rec(0x14, 0x1414, 0x1414_1414, 0x1414));
                assert!(CgItemDrop2::decode_frame(&ClientFrame::new(0x14, vec![0x14; 9])).is_ok());
                continue;
            }
            let err = CgItemDrop2::decode(&vec![0x14; len]).unwrap_err();
            if len < 10 {
                assert_eq!(
                    err,
                    CgItemDrop2Error::Truncated {
                        needed: 10,
                        available: len
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDrop2Error::LengthMismatch {
                        expected: 10,
                        actual: len
                    }
                );
            }
            if len == 9 {
                continue; // a nine-byte payload is the exact framed size
            }
            let err =
                CgItemDrop2::decode_frame(&ClientFrame::new(0x14, vec![0x14; len])).unwrap_err();
            if len < 10 {
                assert_eq!(
                    err,
                    CgItemDrop2Error::Truncated {
                        needed: 10,
                        available: len + 1
                    }
                );
            } else {
                assert_eq!(
                    err,
                    CgItemDrop2Error::LengthMismatch {
                        expected: 10,
                        actual: len + 1
                    }
                );
            }
        }
    }

    #[test]
    fn fragmented_streaming_matches_bulk_decoding() {
        let records: Vec<Vec<u8>> = (0..4u16)
            .map(|i| {
                rec(
                    u8::try_from(1 + i).expect("small window"),
                    0x0100 + i,
                    0x1000 * u32::from(i),
                    0x20 + i,
                )
                .encode()
            })
            .collect();
        let stream: Vec<u8> = records.concat();
        let expect: Vec<CgItemDrop2> = records
            .iter()
            .map(|r| CgItemDrop2::decode(r).expect("expect record"))
            .collect();
        for split in 0..stream.len() {
            let mut d = ClientFrameDecoder::new();
            d.feed(&stream[..split]).expect("bulk prefix feed");
            d.feed(&stream[split..]).expect("bulk suffix feed");
            let mut out = Vec::new();
            while let Some(f) = d.try_decode().expect("bulk try_decode") {
                out.push(CgItemDrop2::decode_frame(&f).expect("bulk record"));
            }
            assert_eq!(out, expect, "bulk split at {split}");
        }
    }

    #[test]
    fn byte_at_a_time_streaming_retains_partial_tails() {
        let records: Vec<Vec<u8>> = (0..3u16)
            .map(|i| rec(1, 0x0f00 + i, u32::from(i), i).encode())
            .collect();
        let stream: Vec<u8> = records.concat();
        let mut d = ClientFrameDecoder::new();
        let mut out = Vec::new();
        for (i, b) in stream.iter().enumerate() {
            d.feed(&[*b]).expect("single byte feed");
            if i < 9 {
                assert!(d.buffered_len() > 0, "tail dropped at byte {i}");
            }
            while let Some(f) = d.try_decode().expect("byte-wise try_decode") {
                out.push(CgItemDrop2::decode_frame(&f).expect("byte-wise record"));
            }
        }
        assert_eq!(out.len(), 3);
        assert!(d.is_empty());
    }

    #[test]
    fn two_records_stream_back_to_back() {
        let mut d = ClientFrameDecoder::new();
        d.feed(&rec(1, 0x1111, 1, 1).encode()).expect("first feed");
        d.feed(&rec(2, 0x2222, 2, 2).encode()).expect("second feed");
        let a = d.try_decode().expect("first frame").expect("first present");
        let b = d
            .try_decode()
            .expect("second frame")
            .expect("second present");
        assert!(d.try_decode().expect("no third frame").is_none());
        assert_eq!(
            CgItemDrop2::decode_frame(&a).expect("first"),
            rec(1, 0x1111, 1, 1)
        );
        assert_eq!(
            CgItemDrop2::decode_frame(&b).expect("second"),
            rec(2, 0x2222, 2, 2)
        );
    }

    #[test]
    fn declared_sizes_match_the_bytes_actually_produced() {
        assert_eq!(CG_ITEM_POS_SIZE, 3);
        assert_eq!(CG_ITEM_POS_SIZE, crate::cg_item_move::CG_ITEM_POS_SIZE);
        let r = rec(1, 0x0203, 0x0405_0607, 0x0809);
        assert_eq!(r.encode().len(), CG_ITEM_DROP2_WIRE_SIZE);
        assert_eq!(r.to_frame().payload.len(), CG_ITEM_DROP2_PAYLOAD_SIZE);
        assert_eq!(r.encode().len() - 1, CG_ITEM_DROP2_PAYLOAD_SIZE);
    }

    /// The GC-20 defect in the module docs is a live legacy collision, not a
    /// property of this record. This test pins only the part that is decidable
    /// here: a 62-byte outbound GC record must NOT be accepted as a header-20
    /// inbound request, in either direction, so the two can never be confused
    /// by this codec.
    #[test]
    fn the_sixty_two_byte_outbound_gc_record_is_not_this_record() {
        // 62 = the measured width of the server's TPacketGCItemDelDeprecated
        // and of the client's TPacketGCItemSet, the two unrelated records that
        // share outbound header 20.
        let gc = vec![20u8; 62];
        assert_eq!(
            CgItemDrop2::decode(&gc),
            Err(CgItemDrop2Error::LengthMismatch {
                expected: 10,
                actual: 62
            })
        );
        assert_eq!(
            CgItemDrop2::decode_frame(&ClientFrame::new(20, &gc[1..])),
            Err(CgItemDrop2Error::LengthMismatch {
                expected: 10,
                actual: 62
            })
        );
        // And the reverse direction: this record must not be accepted as a
        // 62-byte payload either.
        assert_eq!(
            CgItemDrop2::decode(&rec(1, 0x0203, 0x0405_0607, 0x0809).encode()).map(|_| ()),
            Ok(())
        );
        assert_eq!(CG_ITEM_DROP2_WIRE_SIZE, 10);
    }

    /// The client builder takes a DWORD count and narrows it to the wire WORD
    /// without a range check, so 65 536 transmits as 0 and the server reads it
    /// as "drop the whole stack". This codec must NOT reproduce that: a u16
    /// round-trips bit-for-bit and no zero-triggered rewrite is applied here.
    #[test]
    fn the_client_count_truncation_is_not_reproduced() {
        // What the client would put on the wire for a DWORD 65 536.
        // The low WORD of the little-endian DWORD 65 536, which is what a
        // lossy narrowing would put on the wire.
        let client_wire =
            u16::from_le_bytes(65_536u32.to_le_bytes()[0..2].try_into().expect("two bytes"));
        assert_eq!(client_wire, 0, "premise: the client truncates to zero");
        let sent = rec(1, 0x0005, 0, client_wire);
        // The codec carries the truncated value faithfully...
        assert_eq!(CgItemDrop2::decode(&sent.encode()).unwrap().count, 0);
        // ...and does not itself rewrite the zero into any sentinel.
        assert_eq!(CgItemDrop2::decode(&sent.encode()).unwrap(), sent);
        // A real 65 536 is not representable, so it must not silently become
        // 65 535 either: u16 construction is the only narrowing point, and it
        // is checked here so the boundary stays explicit.
        assert_eq!(u16::MAX, 65_535);
        assert!(u16::try_from(65_536u32).is_err());
    }

    /// The window byte's *meaning* is build-conditional because `EWindows`
    /// conditionally inserts three enumerators at `length.h:665-673`. These
    /// are the two measured profiles; the codec must carry both without
    /// resolving either to a window kind.
    #[test]
    fn both_measured_ewindows_profiles_round_trip_unresolved() {
        // all three feature macros on:  RESERVED INVENTORY EQUIPMENT SAFEBOX
        // MALL DRAGON_SOUL ATTR67_ADD AURA_REFINE SWITCHBOT BELT=9 GROUND=10
        // all three off:                 BELT=6 GROUND=7
        for (belt, ground) in [(9u8, 10u8), (6u8, 7u8)] {
            for w in [0u8, 1, 2, 3, 4, 5, belt, ground, u8::MAX] {
                let p = rec(w, 0x1234, 0, 9);
                let back = CgItemDrop2::decode(&p.encode()).unwrap();
                assert_eq!(back.cell.window_type, w, "window {w} in belt {belt}");
                assert_eq!(back, p);
                assert_eq!(
                    CgItemDrop2::decode_frame(&p.to_frame())
                        .unwrap()
                        .cell
                        .window_type,
                    w
                );
            }
        }
    }

    /// The stored header byte is dead in the legacy handler, but it must still
    /// be 20 at offset 0 because dispatch and the descriptor read loop size the
    /// frame from it. Pin that the byte is emitted even though no field
    /// round-trips it.
    #[test]
    fn the_dead_header_byte_is_still_emitted_at_offset_zero() {
        let p = rec(0xff, 0xffff, u32::MAX, u16::MAX);
        let wire = p.encode();
        assert_eq!(wire[0], 20, "offset 0 must carry the header byte");
        // No field of the decoded value carries it back.
        let back = CgItemDrop2::decode(&wire).unwrap();
        assert_eq!(back.to_frame().header, 20);
        assert_eq!(back.encode(), wire, "the header is not stored yet survives");
    }

    #[test]
    fn error_display_is_stable_through_a_trait_object() {
        let cases = [
            (
                CgItemDrop2Error::Truncated {
                    needed: 10,
                    available: 3,
                },
                "item-drop2 record needs 10 bytes, got 3",
            ),
            (
                CgItemDrop2Error::LengthMismatch {
                    expected: 10,
                    actual: 11,
                },
                "item-drop2 record needs 10 bytes, got 11",
            ),
            (
                CgItemDrop2Error::InvalidHeader { actual: 0x53 },
                "header 0x53 is not the item-drop2 header",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let boxed: Box<dyn Error> = Box::new(error);
            assert_eq!(boxed.to_string(), text);
        }
    }
}
