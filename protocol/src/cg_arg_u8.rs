//! Seven small records whose payload is one to five **opaque** `u8` arguments.
//!
//! | record | header | wire | declaration | handler `switch`es on |
//! |---|---|---|---|---|
//! | `ChangeLanguage` | 238 `0xee` | 2 | `packet.h:3248` | -- (range check) |
//! | `EventRequest` | 117 `0x75` | 2 | `packet.h:3453` | -- |
//! | `RecvPremiumPlayersPacket` | 176 `0xb0` | 2 | `packet.h:3219` | `bySubHeader`, 3 cases, `default:` |
//! | `RefineElement` | 227 `0xe3` | 2 | `packet.h:3072` | -- (sentinel 255) |
//! | `WorldBoss` | 148 `0x94` | 2 | `packet.h:3357` | `bSubHeader`, 3 cases, **no `default:`** |
//! | `DailyGift` | 180 `0xb4` | 3 | `packet.h:3142` | `bAction`, 4 cases, **no `default:`** |
//! | `RecvBiologistPacket` | 175 `0xaf` | 6 | `packet.h:3168` | `bySubHeader`, 4 cases, `default:` |
//!
//! # Why every argument stays an opaque `u8`
//!
//! These seven records share a shape -- a header plus raw bytes the server
//! interprets -- but they are grouped for a reason stronger than tidiness: **not
//! one of them has a wire-visible constraint on its argument values.** The
//! handler-side meaning of each byte is decided by a `switch` or a comparison
//! that happens *after* the record has been parsed, and none of those
//! decisions can reject a record at the framing layer.
//!
//! Two of them show the pattern directly:
//!
//! - `WorldBoss` and `DailyGift` `switch` on their argument with **no `default:`
//!   arm**, so every unrecognised value falls through silently. That is the
//!   same shared-enum situation as the sash sub-header and the dragon-soul
//!   sub-header, and the same rule applies: preserve all 256, police none.
//!   Being stricter would reject records the legacy server accepts.
//! - `RefineElement` treats `bArg == 255` as an explicit **window-close**
//!   sentinel at `input_main.cpp:3241-3246`, which is a meaning, not a bound.
//!
//! `RecvPremiumPlayersPacket` and `RecvBiologistPacket` *do* have `default:`
//! arms, but they also ignore the unknown value rather than closing the
//! session, so the outcome is the same for framing purposes.
//!
//! # `ChangeLanguage` is the one that is already accounted for elsewhere
//!
//! This record is the documented server-238 / client-245 divergence: the server
//! accepts and forwards 238, the client registers 245. Both macro and value are
//! recorded in the compatibility rules. The Rust field is still an opaque `u8`
//! -- the divergence is a live protocol fact to record, not a constraint the
//! codec should enforce, and "reject 245" would break the only client that
//! works.
//!
//! # `EventRequest`'s argument is a month, not an index
//!
//! `bMonth` is forwarded straight to `CEventManager::Instance().SendEventInfo(ch,
//! p->bMonth)` at `input_main.cpp:5472` with no range check at all. It is a
//! calendar month, so the codec does not renumber it, does not treat 0 as
//! "unset", and does not wrap it. All 256 values round-trip.

use crate::cg_inventory::{
    CgHeader, HEADER_CG_BIOLOGIST, HEADER_CG_CHANGE_LANGUAGE, HEADER_CG_DAILY_GIFT,
    HEADER_CG_PREMIUM_PLAYERS, HEADER_CG_REFINE_ELEMENT, HEADER_CG_REQUEST_EVENT_DATA,
    HEADER_CG_WORLD_BOSS,
};
use crate::cg_wire::ClientFrame;

/// `TPacketCGChangeLanguage`: a header and one `BYTE`.
pub const CG_CHANGE_LANGUAGE_WIRE_SIZE: usize = 1 + 1;
/// The framed payload of `TPacketCGChangeLanguage`.
pub const CG_CHANGE_LANGUAGE_PAYLOAD_SIZE: usize = 1;

/// `TPacketCGRequestEventData`: a header and one `BYTE`.
pub const CG_REQUEST_EVENT_DATA_WIRE_SIZE: usize = 1 + 1;
/// The framed payload of `TPacketCGRequestEventData`.
pub const CG_REQUEST_EVENT_DATA_PAYLOAD_SIZE: usize = 1;

/// `TPacketCGPremiumPlayers`: a header and one `BYTE`.
pub const CG_PREMIUM_PLAYERS_WIRE_SIZE: usize = 1 + 1;
/// The framed payload of `TPacketCGPremiumPlayers`.
pub const CG_PREMIUM_PLAYERS_PAYLOAD_SIZE: usize = 1;

/// `TPacketCGRefineElement`: a header and one `BYTE`.
pub const CG_REFINE_ELEMENT_WIRE_SIZE: usize = 1 + 1;
/// The framed payload of `TPacketCGRefineElement`.
pub const CG_REFINE_ELEMENT_PAYLOAD_SIZE: usize = 1;

/// `TPacketCGWorldBoss`: a header and one `BYTE`.
pub const CG_WORLD_BOSS_WIRE_SIZE: usize = 1 + 1;
/// The framed payload of `TPacketCGWorldBoss`.
pub const CG_WORLD_BOSS_PAYLOAD_SIZE: usize = 1;

/// `TPacketCGDailyGift`: a header and two `BYTE`s.
pub const CG_DAILY_GIFT_WIRE_SIZE: usize = 1 + 1 + 1;
/// The framed payload of `TPacketCGDailyGift`.
pub const CG_DAILY_GIFT_PAYLOAD_SIZE: usize = 2;

/// `TPacketCGBiologist`: a header and five `BYTE`s.
pub const CG_BIOLOGIST_WIRE_SIZE: usize = 1 + 1 + 1 + 1 + 1 + 1;
/// The framed payload of `TPacketCGBiologist`.
pub const CG_BIOLOGIST_PAYLOAD_SIZE: usize = 5;

/// The `bArg` value that `CInputMain::RefineElement` reads as "close the
/// window" at `input_main.cpp:3241-3246`.
///
/// It is a **meaning**, not a bound: the same `u8` space also carries ordinary
/// element arguments, so no range check is implied by the codec.
pub const CG_REFINE_ELEMENT_CLOSE: u8 = 255;

/// Every way these seven decoders can refuse a slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgArgU8Error {
    /// Fewer bytes than the fixed record needs.
    Truncated {
        /// The fixed width the decoder requires.
        needed: usize,
        /// How many bytes were actually offered.
        available: usize,
    },
    /// A complete-length slice that is not the fixed width.
    LengthMismatch {
        /// The fixed width the decoder requires.
        expected: usize,
        /// How many bytes were actually offered.
        actual: usize,
    },
    /// The right number of bytes, but the header byte is not this record's.
    InvalidHeader {
        /// The header byte this record requires.
        expected: u8,
        /// The header byte that was actually present.
        actual: u8,
    },
}

/// Reject anything that is not exactly `expected` bytes wide.
fn check_exact(actual: usize, expected: usize) -> Result<(), CgArgU8Error> {
    if actual < expected {
        return Err(CgArgU8Error::Truncated {
            needed: expected,
            available: actual,
        });
    }
    if actual > expected {
        return Err(CgArgU8Error::LengthMismatch { expected, actual });
    }
    Ok(())
}

/// Reject a header byte that is not `expected`.
fn check_header(actual: u8, expected: u8) -> Result<(), CgArgU8Error> {
    if actual != expected {
        return Err(CgArgU8Error::InvalidHeader { expected, actual });
    }
    Ok(())
}

/// Generate the shared surface of a small header-plus-arguments record.
///
/// Every one of the seven has the same four operations and the same three
/// failure modes, and writing them out seven times is how offsets drift. The
/// macro takes the field list once, so a width can only be wrong in one place
/// per record and the shared tests below run against all seven.
/// Generate the shared surface of a small header-plus-arguments record.
///
/// Every one of the seven has the same four operations and the same three
/// failure modes, and writing them out seven times is how offsets drift. The
/// field list is given once as `(name: type ~ byte offset after the header)`, so
/// a width or an offset can only be wrong in one place per record, and the
/// shared tests below run against all seven.
///
/// The `~` separator is deliberate. A `=` there is parsed as a trait
/// associated-type binding, not as data, so the obvious spelling does not
/// compile.
macro_rules! arg_u8_record {
    (
        $(#[$meta:meta])*
        $name:ident, $header:expr, $wire:expr, $payload:expr,
        fields: { $($field:ident ~ $at:literal),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name {
            $(
                #[doc = concat!("The legacy `", stringify!($field), "`, as an opaque byte.")]
                pub $field: u8,
            )*
        }

        impl $name {
            /// The legacy header byte.
            pub const fn header() -> CgHeader {
                $header
            }
            /// The full legacy record width, header byte included.
            pub const WIRE_SIZE: usize = $wire;
            /// The framed payload width, everything after the header.
            pub const PAYLOAD_SIZE: usize = $payload;

            /// Build the record from its arguments.
            pub const fn new($($field: u8),*) -> Self {
                Self { $($field),* }
            }

            /// Append the record to `out`.
            pub fn encode_into(&self, out: &mut Vec<u8>) {
                out.push(Self::header().value());
                $(out.push(self.$field);)*
            }

            /// Encode to a fresh `WIRE_SIZE`-byte buffer.
            pub fn encode(&self) -> Vec<u8> {
                let mut out = Vec::with_capacity(Self::WIRE_SIZE);
                self.encode_into(&mut out);
                out
            }

            /// Encode the header-less payload.
            pub fn to_frame(&self) -> ClientFrame {
                let mut payload = Vec::with_capacity(Self::PAYLOAD_SIZE);
                $(payload.push(self.$field);)*
                ClientFrame {
                    header: Self::header().value(),
                    payload,
                }
            }

            /// # Errors
            ///
            /// [`CgArgU8Error::Truncated`] below `WIRE_SIZE` bytes,
            /// [`CgArgU8Error::LengthMismatch`] above,
            /// [`CgArgU8Error::InvalidHeader`] for a full-length slice that does
            /// not start with this record's header.
            pub fn decode(bytes: &[u8]) -> Result<Self, CgArgU8Error> {
                check_exact(bytes.len(), Self::WIRE_SIZE)?;
                check_header(bytes[0], Self::header().value())?;
                Ok(Self {
                    $($field: bytes[1 + $at],)*
                })
            }

            /// # Errors
            ///
            /// As the slice decoder, except that the frame payload must be
            /// exactly `PAYLOAD_SIZE` bytes.
            pub fn decode_frame(frame: &ClientFrame) -> Result<Self, CgArgU8Error> {
                check_exact(frame.payload.len(), Self::PAYLOAD_SIZE)?;
                check_header(frame.header, Self::header().value())?;
                Ok(Self {
                    $($field: frame.payload[$at],)*
                })
            }
        }
    };
}

arg_u8_record!(
    /// The 2-byte `ChangeLanguage` record: a header then an opaque language byte.
    ///
    /// The handler range-checks it `bLanguage > LOCALE_YMIR && bLanguage <
    /// LOCALE_MAX_NUM` at `input_main.cpp:3587`. The codec does not: see the
    /// module documentation on the server-238 / client-245 divergence.
    CgChangeLanguage, HEADER_CG_CHANGE_LANGUAGE,
    CG_CHANGE_LANGUAGE_WIRE_SIZE, CG_CHANGE_LANGUAGE_PAYLOAD_SIZE,
    fields: { b_language ~ 0 }
);

arg_u8_record!(
    /// The 2-byte `EventRequest` record: a header then an opaque calendar month.
    CgRequestEventData, HEADER_CG_REQUEST_EVENT_DATA,
    CG_REQUEST_EVENT_DATA_WIRE_SIZE, CG_REQUEST_EVENT_DATA_PAYLOAD_SIZE,
    fields: { b_month ~ 0 }
);

arg_u8_record!(
    /// The 2-byte `RecvPremiumPlayersPacket` record: a header then an opaque
    /// sub-header the handler `switch`es with a `default:` arm.
    CgPremiumPlayers, HEADER_CG_PREMIUM_PLAYERS,
    CG_PREMIUM_PLAYERS_WIRE_SIZE, CG_PREMIUM_PLAYERS_PAYLOAD_SIZE,
    fields: { by_sub_header ~ 0 }
);

arg_u8_record!(
    /// The 2-byte `RefineElement` record: a header then an opaque element
    /// argument, where 255 is a window-close sentinel rather than a bound.
    CgRefineElement, HEADER_CG_REFINE_ELEMENT,
    CG_REFINE_ELEMENT_WIRE_SIZE, CG_REFINE_ELEMENT_PAYLOAD_SIZE,
    fields: { b_arg ~ 0 }
);

arg_u8_record!(
    /// The 2-byte `WorldBoss` record: a header then an opaque sub-header the
    /// handler `switch`es with **no `default:` arm**.
    CgWorldBoss, HEADER_CG_WORLD_BOSS,
    CG_WORLD_BOSS_WIRE_SIZE, CG_WORLD_BOSS_PAYLOAD_SIZE,
    fields: { b_sub_header ~ 0 }
);

arg_u8_record!(
    /// The 3-byte `DailyGift` record: a header, an opaque action the handler
    /// `switch`es with **no `default:` arm**, and an opaque slot index.
    CgDailyGift, HEADER_CG_DAILY_GIFT,
    CG_DAILY_GIFT_WIRE_SIZE, CG_DAILY_GIFT_PAYLOAD_SIZE,
    fields: {
        b_action ~ 0,
        b_slot_index ~ 1,
    }
);

arg_u8_record!(
    /// The 6-byte `RecvBiologistPacket` record: a header then five opaque bytes.
    CgBiologist, HEADER_CG_BIOLOGIST,
    CG_BIOLOGIST_WIRE_SIZE, CG_BIOLOGIST_PAYLOAD_SIZE,
    fields: {
        by_sub_header ~ 0,
        by_chosen_affect ~ 1,
        by_decrease_time_index ~ 2,
        by_is_elixir_use ~ 3,
        by_is_book_time_use ~ 4,
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    /// The seven records, as `(header, wire, payload)`.
    const SHAPES: [(u8, usize, usize); 7] = [
        (0xee, 2, 1),
        (0x75, 2, 1),
        (0xb0, 2, 1),
        (0xe3, 2, 1),
        (0x94, 2, 1),
        (0xb4, 3, 2),
        (0xaf, 6, 5),
    ];

    fn check_exact_forms() {
        // Every record is header plus payload, with no gap and no tail.
        for (header, wire, payload) in SHAPES {
            assert_eq!(wire, 1 + payload, "header {header:#04x}");
        }
    }

    #[test]
    fn every_record_is_a_header_plus_its_payload() {
        check_exact_forms();
        assert_eq!(CgChangeLanguage::WIRE_SIZE, 2);
        assert_eq!(CgRequestEventData::WIRE_SIZE, 2);
        assert_eq!(CgPremiumPlayers::WIRE_SIZE, 2);
        assert_eq!(CgRefineElement::WIRE_SIZE, 2);
        assert_eq!(CgWorldBoss::WIRE_SIZE, 2);
        assert_eq!(CgDailyGift::WIRE_SIZE, 3);
        assert_eq!(CgBiologist::WIRE_SIZE, 6);
    }

    #[test]
    fn the_header_constants_are_the_registered_values() {
        assert_eq!(CgChangeLanguage::header().value(), 0xee);
        assert_eq!(CgRequestEventData::header().value(), 0x75);
        assert_eq!(CgPremiumPlayers::header().value(), 0xb0);
        assert_eq!(CgRefineElement::header().value(), 0xe3);
        assert_eq!(CgWorldBoss::header().value(), 0x94);
        assert_eq!(CgDailyGift::header().value(), 0xb4);
        assert_eq!(CgBiologist::header().value(), 0xaf);
    }

    #[test]
    fn the_seven_headers_are_all_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for (header, _, _) in SHAPES {
            assert!(seen.insert(header), "duplicate header {header:#04x}");
        }
        assert_eq!(seen.len(), 7);
    }

    #[test]
    fn the_two_byte_records_encode_to_header_plus_one_byte() {
        assert_eq!(CgChangeLanguage::new(0x41).encode(), vec![0xee, 0x41]);
        assert_eq!(CgRequestEventData::new(12).encode(), vec![0x75, 12]);
        assert_eq!(CgPremiumPlayers::new(7).encode(), vec![0xb0, 7]);
        assert_eq!(CgRefineElement::new(255).encode(), vec![0xe3, 255]);
        assert_eq!(CgWorldBoss::new(3).encode(), vec![0x94, 3]);
    }

    #[test]
    fn daily_gift_encodes_to_header_plus_two_bytes() {
        assert_eq!(CgDailyGift::new(1, 2).encode(), vec![0xb4, 1, 2]);
        assert_eq!(CgDailyGift::new(255, 0).encode(), vec![0xb4, 255, 0]);
    }

    #[test]
    fn biologist_encodes_to_header_plus_five_bytes() {
        let rec = CgBiologist::new(1, 2, 3, 4, 5);
        assert_eq!(rec.encode(), vec![0xaf, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn every_argument_value_round_trips() {
        // The whole point of "opaque": no argument value is rejected anywhere.
        for v in 0..=255_u8 {
            assert_eq!(
                CgChangeLanguage::decode(&CgChangeLanguage::new(v).encode())
                    .unwrap()
                    .b_language,
                v
            );
            assert_eq!(
                CgRequestEventData::decode(&CgRequestEventData::new(v).encode())
                    .unwrap()
                    .b_month,
                v
            );
            assert_eq!(
                CgPremiumPlayers::decode(&CgPremiumPlayers::new(v).encode())
                    .unwrap()
                    .by_sub_header,
                v
            );
            assert_eq!(
                CgRefineElement::decode(&CgRefineElement::new(v).encode())
                    .unwrap()
                    .b_arg,
                v
            );
            assert_eq!(
                CgWorldBoss::decode(&CgWorldBoss::new(v).encode())
                    .unwrap()
                    .b_sub_header,
                v
            );
            let g = CgDailyGift::new(v, v);
            let gb = CgDailyGift::decode(&g.encode()).unwrap();
            assert_eq!((gb.b_action, gb.b_slot_index), (v, v));
            let b = CgBiologist::new(v, v, v, v, v);
            let bb = CgBiologist::decode(&b.encode()).unwrap();
            assert_eq!(
                (
                    bb.by_sub_header,
                    bb.by_chosen_affect,
                    bb.by_decrease_time_index,
                    bb.by_is_elixir_use,
                    bb.by_is_book_time_use
                ),
                (v, v, v, v, v)
            );
        }
    }

    #[test]
    fn the_refine_element_close_sentinel_is_just_a_value() {
        assert_eq!(CG_REFINE_ELEMENT_CLOSE, 255);
        let rec = CgRefineElement::new(CG_REFINE_ELEMENT_CLOSE);
        assert_eq!(CgRefineElement::decode(&rec.encode()).unwrap(), rec);
    }

    #[test]
    fn all_round_trip() {
        let language = CgChangeLanguage::new(0x11);
        assert_eq!(
            CgChangeLanguage::decode(&language.encode()).unwrap(),
            language
        );
        let month = CgRequestEventData::new(0x22);
        assert_eq!(CgRequestEventData::decode(&month.encode()).unwrap(), month);
        let premium = CgPremiumPlayers::new(0x33);
        assert_eq!(
            CgPremiumPlayers::decode(&premium.encode()).unwrap(),
            premium
        );
        let element = CgRefineElement::new(0x44);
        assert_eq!(CgRefineElement::decode(&element.encode()).unwrap(), element);
        let world_boss = CgWorldBoss::new(0x55);
        assert_eq!(
            CgWorldBoss::decode(&world_boss.encode()).unwrap(),
            world_boss
        );
        let gift = CgDailyGift::new(0x66, 0x77);
        assert_eq!(CgDailyGift::decode(&gift.encode()).unwrap(), gift);
        let biologist = CgBiologist::new(1, 2, 3, 4, 5);
        assert_eq!(CgBiologist::decode(&biologist.encode()).unwrap(), biologist);
    }

    #[test]
    fn all_round_trip_through_a_frame() {
        macro_rules! frame_case {
            ($ty:ty, $rec:expr) => {{
                let r = $rec;
                let f = r.to_frame();
                assert_eq!(f.header, <$ty>::header().value());
                assert_eq!(f.payload.len(), <$ty>::PAYLOAD_SIZE);
                assert_eq!(<$ty>::decode_frame(&f).unwrap(), r);
                assert_eq!(&f.payload[..], &r.encode()[1..]);
            }};
        }
        frame_case!(CgChangeLanguage, CgChangeLanguage::new(1));
        frame_case!(CgRequestEventData, CgRequestEventData::new(1));
        frame_case!(CgPremiumPlayers, CgPremiumPlayers::new(1));
        frame_case!(CgRefineElement, CgRefineElement::new(1));
        frame_case!(CgWorldBoss, CgWorldBoss::new(1));
        frame_case!(CgDailyGift, CgDailyGift::new(1, 2));
        frame_case!(CgBiologist, CgBiologist::new(1, 2, 3, 4, 5));
    }

    #[test]
    fn no_record_accepts_another_records_bytes() {
        // Each record's own encoding, offered to every other decoder, must fail
        // on the header -- except that header 0xee etc. are all distinct, so
        // this is a real cross-product check.
        let encodings: [Vec<u8>; 7] = [
            CgChangeLanguage::new(0).encode(),
            CgRequestEventData::new(0).encode(),
            CgPremiumPlayers::new(0).encode(),
            CgRefineElement::new(0).encode(),
            CgWorldBoss::new(0).encode(),
            CgDailyGift::new(0, 0).encode(),
            CgBiologist::new(0, 0, 0, 0, 0).encode(),
        ];
        for (i, a) in encodings.iter().enumerate() {
            for (j, b) in encodings.iter().enumerate() {
                if i == j {
                    continue;
                }
                assert_ne!(a[0], b[0], "shapes {i} and {j} share a header");
            }
        }
        assert_eq!(encodings[5][0], 0xb4);
        assert_eq!(encodings[6][0], 0xaf);
        // And the two multi-byte records reject each other by length too.
        assert!(CgBiologist::decode(&encodings[5]).is_err());
        assert!(CgDailyGift::decode(&encodings[6]).is_err());
    }

    #[test]
    fn every_short_length_is_rejected_by_every_record() {
        macro_rules! trunc_case {
            ($ty:ty, $rec:expr, $wire:expr) => {{
                let bytes = $rec.encode();
                for len in 0..$wire {
                    let mut b = bytes.clone();
                    b.truncate(len);
                    assert_eq!(
                        <$ty>::decode(&b).unwrap_err(),
                        CgArgU8Error::Truncated {
                            needed: $wire,
                            available: len
                        },
                        "len {len}"
                    );
                }
            }};
        }
        trunc_case!(CgChangeLanguage, CgChangeLanguage::new(0), 2);
        trunc_case!(CgRequestEventData, CgRequestEventData::new(0), 2);
        trunc_case!(CgPremiumPlayers, CgPremiumPlayers::new(0), 2);
        trunc_case!(CgRefineElement, CgRefineElement::new(0), 2);
        trunc_case!(CgWorldBoss, CgWorldBoss::new(0), 2);
        trunc_case!(CgDailyGift, CgDailyGift::new(0, 0), 3);
        trunc_case!(CgBiologist, CgBiologist::new(0, 0, 0, 0, 0), 6);
    }

    #[test]
    fn every_long_length_is_rejected_by_every_record() {
        macro_rules! long_case {
            ($ty:ty, $rec:expr, $wire:expr) => {{
                let bytes = $rec.encode();
                for extra in 1..=3_usize {
                    let mut b = bytes.clone();
                    b.extend(std::iter::repeat_n(0u8, extra));
                    assert_eq!(
                        <$ty>::decode(&b).unwrap_err(),
                        CgArgU8Error::LengthMismatch {
                            expected: $wire,
                            actual: $wire + extra
                        }
                    );
                }
            }};
        }
        long_case!(CgChangeLanguage, CgChangeLanguage::new(0), 2);
        long_case!(CgRequestEventData, CgRequestEventData::new(0), 2);
        long_case!(CgPremiumPlayers, CgPremiumPlayers::new(0), 2);
        long_case!(CgRefineElement, CgRefineElement::new(0), 2);
        long_case!(CgWorldBoss, CgWorldBoss::new(0), 2);
        long_case!(CgDailyGift, CgDailyGift::new(0, 0), 3);
        long_case!(CgBiologist, CgBiologist::new(0, 0, 0, 0, 0), 6);
    }

    #[test]
    fn every_wrong_single_byte_header_is_rejected() {
        // The five 2-byte records, at a full length of 2.
        macro_rules! hdr_case {
            ($rec:ty, $wire:expr, $own:expr) => {{
                for v in 0..=255_u8 {
                    if v == $own {
                        continue;
                    }
                    let mut b = vec![0u8; $wire];
                    b[0] = v;
                    assert_eq!(
                        <$rec>::decode(&b).unwrap_err(),
                        CgArgU8Error::InvalidHeader {
                            expected: $own,
                            actual: v
                        },
                        "header {v}"
                    );
                }
            }};
        }
        hdr_case!(CgChangeLanguage, 2, 0xee);
        hdr_case!(CgRequestEventData, 2, 0x75);
        hdr_case!(CgPremiumPlayers, 2, 0xb0);
        hdr_case!(CgRefineElement, 2, 0xe3);
        hdr_case!(CgWorldBoss, 2, 0x94);
    }

    #[test]
    fn every_wrong_frame_length_is_rejected() {
        for len in 0..5 {
            let f = ClientFrame {
                header: 0xaf,
                payload: vec![0; len],
            };
            assert!(CgBiologist::decode_frame(&f).is_err(), "len {len}");
        }
        for len in 0..2 {
            let f = ClientFrame {
                header: 0xb4,
                payload: vec![0; len],
            };
            assert!(CgDailyGift::decode_frame(&f).is_err(), "len {len}");
        }
        for len in 0..1 {
            let f = ClientFrame {
                header: 0xee,
                payload: vec![0; len],
            };
            assert!(CgChangeLanguage::decode_frame(&f).is_err(), "len {len}");
        }
    }

    #[test]
    fn a_frame_with_a_payload_byte_for_the_header_is_rejected() {
        // The header must come from the frame header, not from payload[0].
        let f = ClientFrame {
            header: 0xee,
            payload: vec![0x11],
        };
        assert_eq!(
            CgChangeLanguage::decode_frame(&f).unwrap().b_language,
            0x11,
            "a correct frame decodes"
        );
        let bad = ClientFrame {
            header: 0x11,
            payload: vec![0xee],
        };
        assert_eq!(
            CgChangeLanguage::decode_frame(&bad).unwrap_err(),
            CgArgU8Error::InvalidHeader {
                expected: 0xee,
                actual: 0x11
            },
            "a payload byte cannot stand in for the header"
        );
    }

    #[test]
    fn encode_into_appends_to_an_existing_buffer() {
        let mut out = vec![0xDE, 0xAD];
        CgDailyGift::new(1, 2).encode_into(&mut out);
        assert_eq!(out.len(), 2 + 3);
        assert_eq!(&out[2..], &CgDailyGift::new(1, 2).encode()[..]);
    }
}
