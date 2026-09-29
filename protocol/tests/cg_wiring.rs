//! Public-surface wiring checks for the fixed legacy client records.
//!
//! These tests live in `protocol/tests/` on purpose, so they compile against
//! the crate's **public** API only. A codec module that exists in
//! `protocol/src/` but is never registered with `pub mod` in
//! `protocol/src/lib.rs` is dead code: its own `#[cfg(test)]` tests still
//! compile and still pass, because they sit inside the module and use `super::*`.
//! That is exactly the shape of the unregistered `protocol/src/cg.rs` deleted
//! in ledger Section 146, which held 91 record declarations and contributed
//! zero tests to any gate.
//!
//! A mutation that removes a `pub mod` line therefore does not fail a unit
//! test; it fails to compile this integration target, which `--no-fail-fast`
//! reports as a failing test binary. That is the weakest form of detection, so
//! every module added from here on should be named below.

use protocol::cg_account::{CgLogin, CgPlayerDelete};
use protocol::cg_attack::CgAttack;
use protocol::cg_inventory::{
    HEADER_CG_ITEM_DESTROY, HEADER_CG_ITEM_DROP, HEADER_CG_ITEM_DROP2, HEADER_CG_ITEM_GIVE,
    HEADER_CG_ITEM_MOVE, HEADER_CG_ITEM_PICKUP, HEADER_CG_ITEM_USE, HEADER_CG_ITEM_USE_TO_ITEM,
};
use protocol::cg_item_destroy::CgItemDestroy;
use protocol::cg_item_drop::CgItemDrop;
use protocol::cg_item_drop2::CgItemDrop2;
use protocol::cg_item_give::CgItemGive;
use protocol::cg_item_move::{CgItemMove, CgItemPos};
use protocol::cg_item_pickup::CgItemPickup;
use protocol::cg_item_use::CgItemUse;
use protocol::cg_item_use_to_item::CgItemUseToItem;
use protocol::cg_quickslot_add::CgQuickslotAdd;
use protocol::cg_quickslot_del::CgQuickslotDel;
use protocol::cg_quickslot_swap::CgQuickslotSwap;
use protocol::cg_use_skill::CgUseSkill;
use protocol::cg_variable::VariableClientFrameDecoder;
use protocol::cg_wire::ClientFrame;

/// Every item-family header the codecs in this crate claim to own.
const ITEM_FAMILY: [(u8, usize); 8] = [
    (
        HEADER_CG_ITEM_MOVE.value(),
        protocol::cg_item_move::CG_ITEM_MOVE_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_USE.value(),
        protocol::cg_item_use::CG_ITEM_USE_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_USE_TO_ITEM.value(),
        protocol::cg_item_use_to_item::CG_ITEM_USE_TO_ITEM_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_DESTROY.value(),
        protocol::cg_item_destroy::CG_ITEM_DESTROY_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_DROP.value(),
        protocol::cg_item_drop::CG_ITEM_DROP_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_DROP2.value(),
        protocol::cg_item_drop2::CG_ITEM_DROP2_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_PICKUP.value(),
        protocol::cg_item_pickup::CG_ITEM_PICKUP_WIRE_SIZE,
    ),
    (
        HEADER_CG_ITEM_GIVE.value(),
        protocol::cg_item_give::CG_ITEM_GIVE_WIRE_SIZE,
    ),
];

#[test]
fn every_item_family_codec_is_reachable_through_the_public_api() {
    for (header, wire_size) in ITEM_FAMILY {
        assert!(wire_size >= 2, "header {header:#x} wire size {wire_size}");
    }
    // A duplicate header would mean two codecs claim the same byte.
    let mut headers: Vec<u8> = ITEM_FAMILY.iter().map(|(h, _)| *h).collect();
    headers.sort_unstable();
    let before = headers.len();
    headers.dedup();
    assert_eq!(
        headers.len(),
        before,
        "two codecs claim the same header byte"
    );
}

#[test]
fn the_item_pickup_codec_is_reachable_and_agrees_with_the_inventory_row() {
    // Reaching the public path at all is the point of this test: it fails to
    // compile if `pub mod cg_item_pickup;` is removed from `lib.rs`.
    let rec = CgItemPickup::new(0x0102_0304);
    let bytes = rec.encode();
    assert_eq!(
        bytes.len(),
        protocol::cg_item_pickup::CG_ITEM_PICKUP_WIRE_SIZE
    );
    assert_eq!(bytes[0], HEADER_CG_ITEM_PICKUP.value());
    assert_eq!(CgItemPickup::decode(&bytes).expect("decodes"), rec);

    let frame = rec.to_frame();
    assert_eq!(frame.header, HEADER_CG_ITEM_PICKUP.value());
    assert_eq!(
        frame.payload.len(),
        protocol::cg_item_pickup::CG_ITEM_PICKUP_PAYLOAD_SIZE
    );
    assert_eq!(CgItemPickup::decode_frame(&frame).expect("decodes"), rec);
}

#[test]
fn every_item_family_record_decodes_through_one_public_decoder() {
    let records: Vec<Vec<u8>> = vec![
        CgItemMove::new(CgItemPos::new(1, 2), CgItemPos::new(3, 4), 1).encode(),
        CgItemUse::new(CgItemPos::new(1, 2)).encode(),
        CgItemUseToItem::new(CgItemPos::new(1, 2), CgItemPos::new(3, 4)).encode(),
        CgItemDestroy::new(CgItemPos::new(1, 2)).encode(),
        CgItemDrop::new(CgItemPos::new(1, 2), 500).encode(),
        CgItemDrop2::new(CgItemPos::new(1, 2), 500, 1).encode(),
        CgItemPickup::new(0xdead_beef).encode(),
        CgItemGive::new(9, CgItemPos::new(1, 2), 3).encode(),
    ];
    assert_eq!(records.len(), ITEM_FAMILY.len());

    for (record, (header, wire_size)) in records.iter().zip(ITEM_FAMILY) {
        assert_eq!(record.len(), wire_size, "header {header:#x}");
        assert_eq!(record[0], header, "header byte");
    }

    // All of them must also pass through the shared framing decoder, because
    // that is what a live descriptor would use.
    let mut stream = Vec::new();
    for r in &records {
        stream.extend_from_slice(r);
    }
    let mut dec = protocol::cg_wire::ClientFrameDecoder::new();
    dec.feed(&stream).expect("feed");
    let mut seen = 0;
    while let Some(frame) = dec.try_decode().expect("decode") {
        assert_eq!(frame.payload.len() + 1, frame_size_for(frame.header));
        seen += 1;
    }
    assert_eq!(seen, records.len());
    assert!(dec.is_empty());
}

fn frame_size_for(header: u8) -> usize {
    ITEM_FAMILY
        .iter()
        .find(|(h, _)| *h == header)
        .map(|(_, s)| *s)
        .expect("header must be claimed by a codec")
}

#[test]
fn the_non_item_cg_codecs_are_reachable_too() {
    let skill = CgUseSkill::new(1, 2);
    // The emitted header byte must be the one the inventory row claims.
    assert_eq!(
        skill.encode()[0],
        protocol::cg_inventory::HEADER_CG_USE_SKILL.value()
    );
    assert_eq!(
        skill.encode().len(),
        protocol::cg_use_skill::CG_USE_SKILL_WIRE_SIZE
    );
    assert_eq!(CgUseSkill::decode(&skill.encode()).expect("d"), skill);

    let attack = CgAttack::new(1, 3, 0, 0);
    assert_eq!(
        attack.encode()[0],
        protocol::cg_inventory::HEADER_CG_ATTACK.value()
    );
    assert_eq!(
        attack.encode().len(),
        protocol::cg_attack::CG_ATTACK_WIRE_SIZE
    );
    assert_eq!(CgAttack::decode(&attack.encode()).expect("d"), attack);

    let del = CgPlayerDelete::new(2, [7u8; protocol::cg_account::CG_PRIVATE_CODE_FIELD_BYTES]);
    assert_eq!(
        del.encode().len(),
        protocol::cg_account::CG_PLAYER_DELETE_WIRE_SIZE
    );
    assert_eq!(
        del.encode()[0],
        protocol::cg_inventory::HEADER_CG_CHARACTER_DELETE.value()
    );
    assert_eq!(CgPlayerDelete::decode(&del.encode()).expect("d"), del);

    let login = CgLogin::new(
        [3u8; protocol::cg_account::CG_LOGIN_FIELD_BYTES],
        [4u8; protocol::cg_account::CG_PASSWORD_FIELD_BYTES],
    );
    assert_eq!(
        login.encode().len(),
        protocol::cg_account::CG_LOGIN_WIRE_SIZE
    );
    assert_eq!(CgLogin::decode(&login.encode()).expect("d"), login);
}

#[test]
fn a_variable_frame_still_needs_the_variable_decoder() {
    // Private shop is not a fixed frame, so this is the boundary that must keep
    // the sub-header routing honest from the public side.
    //
    // Sub-header 12 is ITEM_CHECKIN, whose extension is exactly 19 bytes and is
    // not count-derived, so the complete frame is the two-byte prefix plus
    // nineteen. The build sub-header is the only count-derived one in the
    // family, and it must not be used to size any other arm.
    const CHECKIN_EXTRA: usize = 19;
    let complete = 2 + CHECKIN_EXTRA;

    let mut prefix = vec![protocol::cg_inventory::HEADER_CG_PRIVATE_SHOP.value(), 12u8];
    prefix.resize(complete, 0x5a);
    let mut dec = VariableClientFrameDecoder::new();
    dec.feed(&prefix).expect("feed");
    let frame = dec
        .try_decode()
        .expect("decode")
        .expect("one complete frame");
    assert_eq!(
        frame.header,
        protocol::cg_inventory::HEADER_CG_PRIVATE_SHOP.value()
    );
    // The sub-header byte rides in the payload, so the payload is one byte
    // longer than the declared extension.
    assert_eq!(
        frame.payload.len(),
        1 + CHECKIN_EXTRA,
        "sub-header byte plus extension"
    );
    // Exactly one frame was consumed and nothing was over-read.
    assert!(dec.is_empty(), "the decoder consumed more than one frame");
    assert!(dec.try_decode().expect("decode").is_none());

    // A frame one byte short is incomplete, not an error and not a short read.
    let mut short = VariableClientFrameDecoder::new();
    short.feed(&prefix[..complete - 1]).expect("feed");
    assert!(short.try_decode().expect("decode").is_none());

    // A sub-header that extends by zero needs only the two-byte prefix.
    let mut zero = VariableClientFrameDecoder::new();
    zero.feed(&[protocol::cg_inventory::HEADER_CG_PRIVATE_SHOP.value(), 1u8])
        .expect("feed");
    let frame = zero
        .try_decode()
        .expect("decode")
        .expect("close needs only the prefix");
    assert_eq!(frame.payload.len(), 1, "sub-header byte only");

    // An empty buffer is "need more bytes", never an error.
    let mut empty = VariableClientFrameDecoder::new();
    assert!(empty.try_decode().expect("decode").is_none());
}

#[test]
fn the_quickslot_add_codec_is_reachable_through_the_public_api() {
    // Reaching this public path is the point: it fails to compile if
    // `pub mod cg_quickslot_add;` is removed from `lib.rs`.
    let slot = protocol::TQuickslot {
        b_type: 1,
        b_pos: 12,
    };
    let rec = CgQuickslotAdd::new(3, slot);
    let bytes = rec.encode();
    assert_eq!(bytes, vec![16, 3, 1, 12]);
    assert_eq!(
        bytes[0],
        protocol::cg_inventory::HEADER_CG_QUICKSLOT_ADD.value()
    );
    assert_eq!(
        bytes.len(),
        protocol::cg_quickslot_add::CG_QUICKSLOT_ADD_WIRE_SIZE
    );
    assert_eq!(CgQuickslotAdd::decode(&bytes).expect("d"), rec);
    let frame = rec.to_frame();
    assert_eq!(
        frame.payload.len(),
        protocol::cg_quickslot_add::CG_QUICKSLOT_ADD_PAYLOAD_SIZE
    );
    assert_eq!(CgQuickslotAdd::decode_frame(&frame).expect("d"), rec);
    // The shared two-byte primitive is public too, so it is reachable by name.
    assert_eq!(
        <protocol::TQuickslot as protocol::PacketSerialize>::packed_size(),
        2
    );
}

#[test]
fn the_quickslot_del_codec_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_quickslot_del;` is removed from `lib.rs`.
    let rec = CgQuickslotDel::new(5);
    let bytes = rec.encode();
    assert_eq!(bytes, vec![17, 5]);
    assert_eq!(
        bytes[0],
        protocol::cg_inventory::HEADER_CG_QUICKSLOT_DEL.value()
    );
    assert_eq!(
        bytes.len(),
        protocol::cg_quickslot_del::CG_QUICKSLOT_DEL_WIRE_SIZE
    );
    assert_eq!(CgQuickslotDel::decode(&bytes).expect("d"), rec);
    let frame = rec.to_frame();
    assert_eq!(
        frame.payload.len(),
        protocol::cg_quickslot_del::CG_QUICKSLOT_DEL_PAYLOAD_SIZE
    );
    assert_eq!(CgQuickslotDel::decode_frame(&frame).expect("d"), rec);
}

#[test]
fn the_quickslot_swap_codec_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_quickslot_swap;` is removed from `lib.rs`.
    let rec = CgQuickslotSwap::new(2, 7);
    let bytes = rec.encode();
    assert_eq!(bytes, vec![18, 2, 7]);
    assert_eq!(
        bytes[0],
        protocol::cg_inventory::HEADER_CG_QUICKSLOT_SWAP.value()
    );
    assert_eq!(
        bytes.len(),
        protocol::cg_quickslot_swap::CG_QUICKSLOT_SWAP_WIRE_SIZE
    );
    assert_eq!(CgQuickslotSwap::decode(&bytes).expect("d"), rec);
    let frame = rec.to_frame();
    assert_eq!(
        frame.payload.len(),
        protocol::cg_quickslot_swap::CG_QUICKSLOT_SWAP_PAYLOAD_SIZE
    );
    assert_eq!(CgQuickslotSwap::decode_frame(&frame).expect("d"), rec);
}

#[test]
fn a_client_frame_built_by_hand_still_decodes() {
    let frame = ClientFrame::new(HEADER_CG_ITEM_PICKUP.value(), vec![1, 0, 0, 0]);
    assert_eq!(CgItemPickup::decode_frame(&frame).expect("d").vid, 1);
}

#[test]
fn the_party_family_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_party;` is removed from `lib.rs`. All five
    // records are covered by one test because one `pub mod` line gates them.
    use protocol::cg_party::{
        CgPartyError, CgPartyInvite, CgPartyInviteAnswer, CgPartyParameter, CgPartyRemove,
        CgPartySetState, CG_PARTY_INVITE_WIRE_SIZE, CG_PARTY_PARAMETER_WIRE_SIZE,
        CG_PARTY_SET_STATE_WIRE_SIZE,
    };

    let invite = CgPartyInvite::new(0x1122_3344);
    assert_eq!(invite.encode(), vec![72, 0x44, 0x33, 0x22, 0x11]);
    assert_eq!(invite.encode().len(), CG_PARTY_INVITE_WIRE_SIZE);
    assert_eq!(CgPartyInvite::decode(&invite.encode()).expect("d"), invite);
    assert_eq!(
        CgPartyInvite::decode_frame(&invite.to_frame()).expect("d"),
        invite
    );

    let answer = CgPartyInviteAnswer::new(9, 0xfe);
    assert_eq!(answer.encode(), vec![73, 9, 0, 0, 0, 0xfe]);
    assert_eq!(
        CgPartyInviteAnswer::decode(&answer.encode()).expect("d"),
        answer
    );

    let remove = CgPartyRemove::new(u32::MAX);
    assert_eq!(remove.encode(), vec![74, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(
        CgPartyRemove::decode_frame(&remove.to_frame()).expect("d"),
        remove
    );

    let set_state = CgPartySetState::new(0x0102_0304, 6, 1);
    assert_eq!(set_state.encode(), vec![75, 4, 3, 2, 1, 6, 1]);
    assert_eq!(set_state.encode().len(), CG_PARTY_SET_STATE_WIRE_SIZE);
    assert_eq!(
        CgPartySetState::decode(&set_state.encode()).expect("d"),
        set_state
    );

    let parameter = CgPartyParameter::new(0xab);
    assert_eq!(parameter.encode(), vec![78, 0xab]);
    assert_eq!(parameter.encode().len(), CG_PARTY_PARAMETER_WIRE_SIZE);
    assert_eq!(
        CgPartyParameter::decode_frame(&parameter.to_frame()).expect("d"),
        parameter
    );

    // The five headers stay distinct through the public header constants.
    assert_eq!(
        [
            CgPartyInvite::header().value(),
            CgPartyInviteAnswer::header().value(),
            CgPartyRemove::header().value(),
            CgPartySetState::header().value(),
            CgPartyParameter::header().value(),
        ],
        [72, 73, 74, 75, 78]
    );

    // The shared error type is nameable from outside the crate.
    let err = CgPartyRemove::decode(&[0, 0, 0, 0, 0, 0]).expect_err("must refuse");
    assert!(matches!(err, CgPartyError::LengthMismatch { .. }));
}

#[test]
fn the_party_records_agree_with_the_checked_in_inventory_widths() {
    // The dedicated codecs and the shared `cg_inventory` rows must agree, or the
    // shared fixed framer would disagree with these five records.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_PARTY_INVITE, HEADER_CG_PARTY_INVITE_ANSWER,
        HEADER_CG_PARTY_PARAMETER, HEADER_CG_PARTY_REMOVE, HEADER_CG_PARTY_SET_STATE,
    };
    use protocol::cg_party::{
        CG_PARTY_INVITE_ANSWER_WIRE_SIZE, CG_PARTY_INVITE_WIRE_SIZE, CG_PARTY_PARAMETER_WIRE_SIZE,
        CG_PARTY_REMOVE_WIRE_SIZE, CG_PARTY_SET_STATE_WIRE_SIZE,
    };

    for (header, expected) in [
        (HEADER_CG_PARTY_INVITE, CG_PARTY_INVITE_WIRE_SIZE),
        (
            HEADER_CG_PARTY_INVITE_ANSWER,
            CG_PARTY_INVITE_ANSWER_WIRE_SIZE,
        ),
        (HEADER_CG_PARTY_REMOVE, CG_PARTY_REMOVE_WIRE_SIZE),
        (HEADER_CG_PARTY_SET_STATE, CG_PARTY_SET_STATE_WIRE_SIZE),
        (HEADER_CG_PARTY_PARAMETER, CG_PARTY_PARAMETER_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
}
#[test]
fn the_micro_family_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_micro;` is removed from `lib.rs`. All four
    // records are covered by one test because one `pub mod` line gates them.
    use protocol::cg_micro::{
        CgFishing, CgMicroError, CgPosition, CgScriptAnswer, CgWarp, CG_FISHING_WIRE_SIZE,
        CG_POSITION_WIRE_SIZE, CG_WARP_WIRE_SIZE,
    };

    let position = CgPosition::new(3);
    assert_eq!(position.encode(), vec![0x1c, 0x03]);
    assert_eq!(position.encode().len(), CG_POSITION_WIRE_SIZE);
    assert_eq!(CgPosition::decode(&position.encode()).expect("d"), position);
    assert_eq!(
        CgPosition::decode_frame(&position.to_frame()).expect("d"),
        position
    );

    let answer = CgScriptAnswer::new(0xfb);
    assert_eq!(answer.encode(), vec![0x1d, 0xfb]);
    assert_eq!(CgScriptAnswer::decode(&answer.encode()).expect("d"), answer);

    let warp = CgWarp::new();
    assert_eq!(warp.encode(), vec![0x41]);
    assert_eq!(warp.encode().len(), CG_WARP_WIRE_SIZE);
    assert_eq!(CgWarp::decode_frame(&warp.to_frame()).expect("d"), warp);

    let fishing = CgFishing::new(0x80);
    assert_eq!(fishing.encode(), vec![0x52, 0x80]);
    assert_eq!(fishing.encode().len(), CG_FISHING_WIRE_SIZE);
    assert_eq!(
        CgFishing::decode_frame(&fishing.to_frame()).expect("d"),
        fishing
    );

    // The four headers stay distinct through the public header constants.
    assert_eq!(
        [
            CgPosition::header().value(),
            CgScriptAnswer::header().value(),
            CgWarp::header().value(),
            CgFishing::header().value(),
        ],
        [28, 29, 65, 82]
    );

    // The shared error type is nameable from outside the crate, and a
    // header-only record still refuses a payload.
    let err = CgWarp::decode_frame(&ClientFrame::new(0x41, [0x00])).expect_err("must refuse");
    assert_eq!(
        err,
        CgMicroError::LengthMismatch {
            expected: 0,
            actual: 1
        }
    );
}

#[test]
fn the_micro_records_agree_with_the_checked_in_inventory_widths() {
    // The dedicated codecs and the shared `cg_inventory` rows must agree, or the
    // shared fixed framer would disagree with these four records.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_CHARACTER_POSITION, HEADER_CG_FISHING,
        HEADER_CG_SCRIPT_ANSWER, HEADER_CG_WARP,
    };
    use protocol::cg_micro::{
        CG_FISHING_WIRE_SIZE, CG_POSITION_WIRE_SIZE, CG_SCRIPT_ANSWER_WIRE_SIZE, CG_WARP_WIRE_SIZE,
    };

    for (header, expected) in [
        (HEADER_CG_CHARACTER_POSITION, CG_POSITION_WIRE_SIZE),
        (HEADER_CG_SCRIPT_ANSWER, CG_SCRIPT_ANSWER_WIRE_SIZE),
        (HEADER_CG_WARP, CG_WARP_WIRE_SIZE),
        (HEADER_CG_FISHING, CG_FISHING_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
}

#[test]
fn the_mark_family_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_mark;` is removed from `lib.rs`. All three
    // records are covered by one test because one `pub mod` line gates them.
    use protocol::cg_mark::{
        CgMarkError, CgMarkIdxList, CgMarkLogin, CgSymbolCrc, CG_MARK_IDXLIST_WIRE_SIZE,
        CG_MARK_LOGIN_WIRE_SIZE, CG_SYMBOL_CRC_WIRE_SIZE,
    };

    let login = CgMarkLogin::new(0x1122_3344, 0xAABB_CCDD);
    assert_eq!(
        login.encode(),
        vec![0x64, 0x44, 0x33, 0x22, 0x11, 0xDD, 0xCC, 0xBB, 0xAA]
    );
    assert_eq!(login.encode().len(), CG_MARK_LOGIN_WIRE_SIZE);
    assert_eq!(CgMarkLogin::decode(&login.encode()).expect("d"), login);
    assert_eq!(
        CgMarkLogin::decode_frame(&login.to_frame()).expect("d"),
        login
    );

    let idx = CgMarkIdxList::new();
    assert_eq!(idx.encode(), vec![0x68]);
    assert_eq!(idx.encode().len(), CG_MARK_IDXLIST_WIRE_SIZE);
    assert_eq!(CgMarkIdxList::decode(&idx.encode()).expect("d"), idx);
    assert_eq!(
        CgMarkIdxList::decode_frame(&idx.to_frame()).expect("d"),
        idx
    );

    let crc = CgSymbolCrc::new(1, 2, 3);
    assert_eq!(crc.encode(), vec![0x71, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(crc.encode().len(), CG_SYMBOL_CRC_WIRE_SIZE);
    assert_eq!(CgSymbolCrc::decode(&crc.encode()).expect("d"), crc);
    assert_eq!(CgSymbolCrc::decode_frame(&crc.to_frame()).expect("d"), crc);

    // The three headers stay distinct through the public header constants. 113
    // is the value the client names `HEADER_CG_GUILD_SYMBOL_CRC`.
    assert_eq!(
        [
            CgMarkLogin::header().value(),
            CgMarkIdxList::header().value(),
            CgSymbolCrc::header().value(),
        ],
        [100, 104, 113]
    );

    // The shared error type is nameable from outside the crate, and the
    // header-only record still refuses a payload.
    let err =
        CgMarkIdxList::decode_frame(&ClientFrame::new(0x68, [0x00])).expect_err("must refuse");
    assert_eq!(
        err,
        CgMarkError::LengthMismatch {
            expected: 0,
            actual: 1
        }
    );
}

#[test]
fn the_mark_records_agree_with_the_checked_in_inventory_widths() {
    // The dedicated codecs and the shared `cg_inventory` rows must agree, or the
    // shared fixed framer would disagree with these three records.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_MARK_IDXLIST, HEADER_CG_MARK_LOGIN, HEADER_CG_SYMBOL_CRC,
    };
    use protocol::cg_mark::{
        CG_MARK_IDXLIST_WIRE_SIZE, CG_MARK_LOGIN_WIRE_SIZE, CG_SYMBOL_CRC_WIRE_SIZE,
    };

    for (header, expected) in [
        (HEADER_CG_MARK_LOGIN, CG_MARK_LOGIN_WIRE_SIZE),
        (HEADER_CG_MARK_IDXLIST, CG_MARK_IDXLIST_WIRE_SIZE),
        (HEADER_CG_SYMBOL_CRC, CG_SYMBOL_CRC_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
}

#[test]
fn the_pick_family_is_reachable_through_the_public_api() {
    // Fails to compile if `pub mod cg_pick;` is removed from `lib.rs`. All four
    // records are covered by one test because one `pub mod` line gates them.
    use protocol::cg_pick::{
        CgPickError, CgQuestConfirm, CgScriptButton, CgScriptSelectItem, CgTarget,
        CG_QUEST_CONFIRM_WIRE_SIZE, CG_SCRIPT_BUTTON_WIRE_SIZE, CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
        CG_TARGET_WIRE_SIZE,
    };

    // `TPacketCGQuestConfirm` puts the u8 before the u32 (packet.h:766-768), so
    // the golden below is the reverse of the obvious field order.
    let confirm = CgQuestConfirm::new(0x5a, 0x0102_0304);
    assert_eq!(confirm.encode(), vec![0x1f, 0x5a, 0x04, 0x03, 0x02, 0x01]);
    assert_eq!(confirm.encode().len(), CG_QUEST_CONFIRM_WIRE_SIZE);
    assert_eq!(
        CgQuestConfirm::decode(&confirm.encode()).expect("d"),
        confirm
    );
    assert_eq!(
        CgQuestConfirm::decode_frame(&confirm.to_frame()).expect("d"),
        confirm
    );

    let target = CgTarget::new(0x1122_3344);
    assert_eq!(target.encode(), vec![0x3d, 0x44, 0x33, 0x22, 0x11]);
    assert_eq!(target.encode().len(), CG_TARGET_WIRE_SIZE);
    assert_eq!(CgTarget::decode(&target.encode()).expect("d"), target);
    assert_eq!(
        CgTarget::decode_frame(&target.to_frame()).expect("d"),
        target
    );

    let button = CgScriptButton::new(0xa1b2_c3d4);
    assert_eq!(button.encode(), vec![0x42, 0xd4, 0xc3, 0xb2, 0xa1]);
    assert_eq!(button.encode().len(), CG_SCRIPT_BUTTON_WIRE_SIZE);
    assert_eq!(CgScriptButton::decode(&button.encode()).expect("d"), button);
    assert_eq!(
        CgScriptButton::decode_frame(&button.to_frame()).expect("d"),
        button
    );

    let select = CgScriptSelectItem::new(0xdead_beef);
    assert_eq!(select.encode(), vec![0x72, 0xef, 0xbe, 0xad, 0xde]);
    assert_eq!(select.encode().len(), CG_SCRIPT_SELECT_ITEM_WIRE_SIZE);
    assert_eq!(
        CgScriptSelectItem::decode(&select.encode()).expect("d"),
        select
    );
    assert_eq!(
        CgScriptSelectItem::decode_frame(&select.to_frame()).expect("d"),
        select
    );

    // The four headers stay distinct through the public header constants.
    assert_eq!(
        [
            CgQuestConfirm::header().value(),
            CgTarget::header().value(),
            CgScriptButton::header().value(),
            CgScriptSelectItem::header().value(),
        ],
        [31, 61, 66, 114]
    );

    // The shared error type is nameable from outside the crate, and the three
    // 5-byte records are separated by their header rather than their width.
    let err = CgScriptButton::decode(&[0x3d, 0x44, 0x33, 0x22, 0x11])
        .expect_err("a Target record must not decode as a ScriptButton");
    assert_eq!(
        err,
        CgPickError::InvalidHeader {
            expected: 0x42,
            actual: 0x3d
        }
    );
}

#[test]
fn the_pick_records_agree_with_the_checked_in_inventory_widths() {
    // The dedicated codecs and the shared `cg_inventory` rows must agree, or the
    // shared fixed framer would disagree with these four records.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_QUEST_CONFIRM, HEADER_CG_SCRIPT_BUTTON,
        HEADER_CG_SCRIPT_SELECT_ITEM, HEADER_CG_TARGET,
    };
    use protocol::cg_pick::{
        CG_QUEST_CONFIRM_WIRE_SIZE, CG_SCRIPT_BUTTON_WIRE_SIZE, CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
        CG_TARGET_WIRE_SIZE,
    };

    for (header, expected) in [
        (HEADER_CG_QUEST_CONFIRM, CG_QUEST_CONFIRM_WIRE_SIZE),
        (HEADER_CG_TARGET, CG_TARGET_WIRE_SIZE),
        (HEADER_CG_SCRIPT_BUTTON, CG_SCRIPT_BUTTON_WIRE_SIZE),
        (
            HEADER_CG_SCRIPT_SELECT_ITEM,
            CG_SCRIPT_SELECT_ITEM_WIRE_SIZE,
        ),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
}

#[test]
fn the_login_boundary_records_are_reachable_and_shaped_like_the_legacy_ones() {
    // `CgPong` and `CgStateChecker` are the two header-only CG records that the
    // handshake and login phases actually reach. Both are registered as
    // `sizeof(BYTE)` in `packet_info.cpp`, and both handlers take no payload.
    use protocol::cg_login::{CgEmpire, CgPong, CgStateChecker};

    assert_eq!(CgPong::new().encode(), vec![0xfe]);
    assert_eq!(CgStateChecker::new().encode(), vec![0xce]);
    assert_eq!(CgEmpire::new(3).encode(), vec![0x5a, 0x03]);

    assert_eq!(CgPong::new().to_frame().payload.len(), 0);
    assert_eq!(CgStateChecker::new().to_frame().payload.len(), 0);
    assert_eq!(CgEmpire::new(3).to_frame().payload.as_slice(), &[0x03]);
}

#[test]
fn the_login_boundary_records_agree_with_the_checked_in_inventory_widths() {
    // All three are `sizeof(BYTE)` or `sizeof(TPacketCGEmpire)` in
    // `packet_info.cpp`, so the shared fixed framer must agree with them.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_EMPIRE, HEADER_CG_PONG, HEADER_CG_STATE_CHECKER,
    };
    use protocol::cg_login::{CG_EMPIRE_WIRE_SIZE, CG_PONG_WIRE_SIZE, CG_STATE_CHECKER_WIRE_SIZE};

    for (header, expected) in [
        (HEADER_CG_PONG, CG_PONG_WIRE_SIZE),
        (HEADER_CG_STATE_CHECKER, CG_STATE_CHECKER_WIRE_SIZE),
        (HEADER_CG_EMPIRE, CG_EMPIRE_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
}

#[test]
fn the_state_checker_is_a_live_record_and_its_header_is_not_the_udp_literal_one() {
    // `input_udp.cpp:13` redefines the enumerator to `1` for that one
    // translation unit. The live record is dispatched at `input.cpp:227`
    // through the `else if` chain in `CInputHandshake::Analyze`, and it must
    // keep 206 -- `1` is `HEADER_CG_LOGIN`.
    use protocol::cg_inventory::HEADER_CG_LOGIN;
    use protocol::cg_login::CgStateChecker;

    assert_eq!(CgStateChecker::header().value(), 206);
    assert_ne!(CgStateChecker::header().value(), HEADER_CG_LOGIN.value());
    assert!(CgStateChecker::decode(&[0x01]).is_err());
}

#[test]
fn the_large_guild_mark_records_agree_with_the_checked_in_inventory_widths() {
    // `packet_info.cpp` registers these two as `sizeof(TPacketCGMarkCRCList)`
    // and `sizeof(TPacketCGMarkUpload)`, and they are the two widest CG records
    // in the table, so the shared fixed framer must agree with both.
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_MARK_CRCLIST, HEADER_CG_MARK_UPLOAD,
    };
    use protocol::cg_mark::{CG_MARK_CRCLIST_WIRE_SIZE, CG_MARK_UPLOAD_WIRE_SIZE};

    for (header, expected) in [
        (HEADER_CG_MARK_CRCLIST, CG_MARK_CRCLIST_WIRE_SIZE),
        (HEADER_CG_MARK_UPLOAD, CG_MARK_UPLOAD_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
    assert_eq!(CG_MARK_CRCLIST_WIRE_SIZE, 322);
    assert_eq!(CG_MARK_UPLOAD_WIRE_SIZE, 773);
}

#[test]
fn the_upload_image_is_readable_as_192_little_endian_pixels() {
    // `CInputLogin::GuildMarkUpload` steps a `DWORD *` across the 768-byte
    // image, and `SGuildMark::SIZE` is `16 * 12` = 192. The codec stores raw
    // bytes, so the grouping has to be recoverable from the record itself.
    use protocol::cg_mark::{CgMarkUpload, CG_MARK_UPLOAD_IMAGE_SIZE, CG_MARK_UPLOAD_PIXELS};

    assert_eq!(CG_MARK_UPLOAD_PIXELS * 4, CG_MARK_UPLOAD_IMAGE_SIZE);

    let mut image = [0_u8; CG_MARK_UPLOAD_IMAGE_SIZE];
    image[0..4].copy_from_slice(&0x1122_3344_u32.to_le_bytes());
    let encoded = CgMarkUpload::new(7, image).encode();
    assert_eq!(encoded.len(), 773);

    // Byte 5 is the first image byte on the wire, so pixel 0 starts there.
    assert_eq!(&encoded[5..9], &[0x44, 0x33, 0x22, 0x11]);

    let decoded = CgMarkUpload::decode(&encoded).expect("a 773-byte record must decode");
    assert_eq!(&decoded.image[0..4], &[0x44, 0x33, 0x22, 0x11]);
    assert_eq!(decoded.gid, 7);
}

#[test]
fn the_four_fixed_string_buffers_resolve_to_their_legacy_widths() {
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_CHANGE_NAME, HEADER_CG_INVENTORY_PROTECTED,
        HEADER_CG_WHISPER_DETAILS,
    };
    use protocol::cg_name::{
        CG_CHANGE_NAME_WIRE_SIZE, CG_INVENTORY_PROTECTED_WIRE_SIZE, CG_WHISPER_DETAILS_WIRE_SIZE,
    };

    for (header, expected) in [
        (HEADER_CG_CHANGE_NAME, CG_CHANGE_NAME_WIRE_SIZE),
        (
            HEADER_CG_INVENTORY_PROTECTED,
            CG_INVENTORY_PROTECTED_WIRE_SIZE,
        ),
        (HEADER_CG_WHISPER_DETAILS, CG_WHISPER_DETAILS_WIRE_SIZE),
    ] {
        assert_eq!(
            resolve_cg_base_size(header.value()),
            Some(expected),
            "inventory and codec disagree for header {}",
            header.value()
        );
    }
    assert_eq!(CG_CHANGE_NAME_WIRE_SIZE, 27);
    assert_eq!(CG_WHISPER_DETAILS_WIRE_SIZE, 26);
    assert_eq!(CG_INVENTORY_PROTECTED_WIRE_SIZE, 17);
}

#[test]
fn the_sash_record_resolves_to_23_bytes_with_a_three_byte_item_pos() {
    use protocol::cg_inventory::{resolve_cg_base_size, HEADER_CG_SASH};
    use protocol::cg_sash::{CgSash, ItemPos, CG_SASH_WIRE_SIZE};

    assert_eq!(
        resolve_cg_base_size(HEADER_CG_SASH.value()),
        Some(CG_SASH_WIRE_SIZE)
    );
    // sizeof(TPacketSash) in packet_info.cpp:182. The 3-byte TItemPos is what
    // makes the arithmetic close: 1+1+1+4+1+3+4+4+4 = 23.
    assert_eq!(CG_SASH_WIRE_SIZE, 23);
    assert_eq!(ItemPos::WIRE_SIZE, 3);
    assert_eq!(
        CG_SASH_WIRE_SIZE,
        1 + 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE + 4 + 4 + 4
    );

    // The two window fields are independent, and both must survive the frame.
    let pos = ItemPos::new(0x2A, 0xBEEF);
    let rec = CgSash::new(
        1,
        true,
        0x1122_3344,
        0x77,
        pos,
        0x5566_7788,
        0x99AA_BBCC,
        0xDDEE_FF00,
    );
    let frame = rec.to_frame();
    assert_eq!(frame.header, HEADER_CG_SASH.value());
    assert_eq!(frame.payload.len(), CG_SASH_WIRE_SIZE - 1);
    assert_eq!(
        CgSash::decode_frame(&frame).expect("23-byte record decodes"),
        rec
    );
    assert_eq!(rec.item_pos, pos);
}

#[test]
fn the_dragon_soul_record_resolves_to_47_bytes_with_a_three_byte_item_pos() {
    use protocol::cg_dragon_soul::{
        CgDragonSoulRefine, CG_DRAGON_SOUL_GRID_SIZE, CG_DRAGON_SOUL_REFINE_WIRE_SIZE,
    };
    use protocol::cg_inventory::{resolve_cg_base_size, HEADER_CG_DRAGON_SOUL_REFINE};
    use protocol::item_pos::{ItemPos, ITEM_POS_WIRE_SIZE};

    assert_eq!(
        resolve_cg_base_size(HEADER_CG_DRAGON_SOUL_REFINE.value()),
        Some(CG_DRAGON_SOUL_REFINE_WIRE_SIZE)
    );
    // sizeof(TPacketCGDragonSoulRefine) in packet_info.cpp:171. This record and
    // the 23-byte sash share no field, so the same 3-byte TItemPos is what
    // makes both arithmetic expressions close.
    assert_eq!(CG_DRAGON_SOUL_GRID_SIZE, 15);
    assert_eq!(ITEM_POS_WIRE_SIZE, 3);
    assert_eq!(CG_DRAGON_SOUL_REFINE_WIRE_SIZE, 47);
    assert_eq!(
        CG_DRAGON_SOUL_REFINE_WIRE_SIZE,
        1 + 1 + CG_DRAGON_SOUL_GRID_SIZE * ItemPos::WIRE_SIZE
    );

    let mut grid = [ItemPos::default(); CG_DRAGON_SOUL_GRID_SIZE];
    for (i, slot) in grid.iter_mut().enumerate() {
        *slot = ItemPos::new(
            u8::try_from(i).expect("grid index fits a u8"),
            0x0100 + u16::try_from(i).expect("grid index fits a u16"),
        );
    }
    let rec = CgDragonSoulRefine::new(12, grid);
    let frame = rec.to_frame();
    assert_eq!(frame.header, HEADER_CG_DRAGON_SOUL_REFINE.value());
    assert_eq!(frame.payload.len(), CG_DRAGON_SOUL_REFINE_WIRE_SIZE - 1);
    assert_eq!(
        CgDragonSoulRefine::decode_frame(&frame).expect("47-byte record decodes"),
        rec
    );
    // The shared type is reachable from the shared module, not only via cg_sash.
    assert_eq!(rec.grid[14], ItemPos::new(14, 0x010E));
}

#[test]
fn the_item_pos_type_is_shared_by_both_records_through_one_module() {
    use protocol::cg_dragon_soul::CgDragonSoulRefine;
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_DRAGON_SOUL_REFINE, HEADER_CG_SASH,
    };
    use protocol::cg_sash::{CgSash, ItemPos as SashItemPos};
    use protocol::item_pos::{ItemPos, ITEM_POS_WIRE_SIZE};

    // The same type, reached two ways. If these ever diverge the "one shared
    // TItemPos" claim is false and a record would be using the wrong width.
    assert_eq!(
        core::mem::size_of::<SashItemPos>(),
        core::mem::size_of::<ItemPos>()
    );
    assert_eq!(SashItemPos::WIRE_SIZE, ItemPos::WIRE_SIZE);
    assert_eq!(SashItemPos::WIRE_SIZE, ITEM_POS_WIRE_SIZE);

    // Both registrations close at the same width, and neither closes at 4.
    let sash = resolve_cg_base_size(HEADER_CG_SASH.value()).expect("sash registered");
    let dragon = resolve_cg_base_size(HEADER_CG_DRAGON_SOUL_REFINE.value()).expect("ds registered");
    assert_eq!(sash, 1 + 1 + 1 + 4 + 1 + ItemPos::WIRE_SIZE + 4 + 4 + 4);
    assert_eq!(dragon, 1 + 1 + 15 * ItemPos::WIRE_SIZE);
    assert_ne!(sash, 1 + 1 + 1 + 4 + 1 + 4 + 4 + 4 + 4);
    assert_ne!(dragon, 1 + 1 + 15 * 4);

    // A position written by one record is readable by the other's field type.
    let pos = ItemPos::new(0xFE, 0xFFFF);
    assert_eq!(
        ItemPos::decode(&pos.encode()).expect("3 bytes"),
        SashItemPos::new(0xFE, 0xFFFF)
    );
    let ds = CgDragonSoulRefine::new(0, [pos; 15]);
    assert_eq!(
        CgDragonSoulRefine::decode(&ds.encode())
            .expect("47 bytes")
            .grid[7],
        pos
    );
    let sash_rec = CgSash::new(0, false, 0, 0, pos, 0, 0, 0);
    assert_eq!(
        CgSash::decode(&sash_rec.encode())
            .expect("23 bytes")
            .item_pos,
        pos
    );
}

#[test]
fn the_two_quest_text_records_resolve_to_66_bytes_from_one_shared_field() {
    use protocol::cg_inventory::{
        resolve_cg_base_size, HEADER_CG_QUEST_INPUT_STRING, HEADER_CG_REQUEST_EVENT_QUEST,
    };
    use protocol::cg_quest_text::{
        quest_input_string_from, request_event_quest_from, CgQuestInputString, CgRequestEventQuest,
        CG_QUEST_INPUT_STRING_WIRE_SIZE, CG_QUEST_TEXT_FIELD_SIZE,
        CG_REQUEST_EVENT_QUEST_WIRE_SIZE,
    };

    assert_eq!(
        resolve_cg_base_size(HEADER_CG_QUEST_INPUT_STRING.value()),
        Some(66)
    );
    assert_eq!(
        resolve_cg_base_size(HEADER_CG_REQUEST_EVENT_QUEST.value()),
        Some(66)
    );

    // One constant, two records. The equality is a property of the active
    // legacy constants, not a shared declaration, so it is asserted rather than
    // assumed: 64+1 literal and QUEST_NAME_MAX_NUM+1 both evaluate to 65 today.
    assert_eq!(CG_QUEST_TEXT_FIELD_SIZE, 65);
    assert_eq!(
        CG_QUEST_INPUT_STRING_WIRE_SIZE,
        1 + CG_QUEST_TEXT_FIELD_SIZE
    );
    assert_eq!(
        CG_REQUEST_EVENT_QUEST_WIRE_SIZE,
        1 + CG_QUEST_TEXT_FIELD_SIZE
    );

    // A non-NUL-terminated 65-byte field is representable, because the legacy
    // RequestEventQuest handler hands the raw pointer to the quest manager.
    let full = [0xFF_u8; 65];
    let event = request_event_quest_from(full);
    let back = CgRequestEventQuest::decode(&event.encode()).expect("66-byte record decodes");
    assert_eq!(back.sz_name, full);
    assert!(!back.sz_name.contains(&0));

    let input = quest_input_string_from(full);
    let frame = input.to_frame();
    assert_eq!(frame.header, HEADER_CG_QUEST_INPUT_STRING.value());
    assert_eq!(frame.payload.len(), 65);
    assert_eq!(
        CgQuestInputString::decode_frame(&frame).expect("frame decodes"),
        input
    );
}

// ---------------------------------------------------------------------------
// Section 162: the twelve small and large records that closed the CG inventory.
// Each is named here so removing its `pub mod` fails this integration target.
// ---------------------------------------------------------------------------

use protocol::cg_arg_u8::{
    CgBiologist, CgChangeLanguage, CgDailyGift, CgPremiumPlayers, CgRefineElement,
    CgRequestEventData, CgWorldBoss, CG_BIOLOGIST_WIRE_SIZE, CG_DAILY_GIFT_WIRE_SIZE,
    CG_WORLD_BOSS_WIRE_SIZE,
};
use protocol::cg_change_look::{CgChangeLook, CG_CHANGE_LOOK_WIRE_SIZE};
use protocol::cg_cube_renewal::{CgCubeRenewal, CG_CUBE_RENEWAL_WIRE_SIZE};
use protocol::cg_exchange::{CgExchange, CG_EXCHANGE_DWORD_PROFILE_SIZE, CG_EXCHANGE_WIRE_SIZE};
use protocol::cg_fly_target::{
    CgFlyTarget, CgFlyTargetHeader, CG_FLY_TARGETING_PAYLOAD_SIZE, CG_FLY_TARGETING_WIRE_SIZE,
};
use protocol::cg_gaya_system::{
    CgGayaSystem, CG_GAYA_SYSTEM_SHADOWED_BY, CG_GAYA_SYSTEM_WIRE_SIZE,
};
use protocol::cg_guild_answer::{CgAnswerMakeGuild, CG_ANSWER_MAKE_GUILD_WIRE_SIZE};
use protocol::cg_hack::{CgHack, CG_HACK_WIRE_SIZE};
use protocol::cg_header_only::{
    CgEnvanterBlack, CgText, CG_TEXT_WIRE_SIZE, ENVANTER_BLACK_WIRE_SIZE,
};
use protocol::cg_inventory::{
    ENVANTER_BLACK, HEADER_CG_ADD_FLY_TARGETING, HEADER_CG_ANSWER_MAKE_GUILD, HEADER_CG_ATTR67_ADD,
    HEADER_CG_BIOLOGIST, HEADER_CG_CHANGE_LANGUAGE, HEADER_CG_CL, HEADER_CG_CUBE_RENEWAL,
    HEADER_CG_DAILY_GIFT, HEADER_CG_EXCHANGE, HEADER_CG_FLY_TARGETING, HEADER_CG_GAYA_SYSTEM,
    HEADER_CG_HACK, HEADER_CG_LOGIN3, HEADER_CG_ON_CLICK, HEADER_CG_PREMIUM_PLAYERS,
    HEADER_CG_REFINE, HEADER_CG_REFINE_ELEMENT, HEADER_CG_REQUEST_EVENT_DATA,
    HEADER_CG_TARGET_INFO_LOAD, HEADER_CG_TEXT, HEADER_CG_WORLD_BOSS,
};
use protocol::cg_login3::{CgLogin3, CgLogin3Error, CG_LOGIN3_CLIENT_WIDTH, CG_LOGIN3_WIRE_SIZE};
use protocol::cg_refine::{Attr67AddData, CgAttr67Add, CgRefine, ATTR67_ADD_DATA_WIRE_SIZE};
use protocol::cg_vid::{
    CgOnClick, CgTargetInfoLoad, CG_ON_CLICK_WIRE_SIZE, CG_TARGET_INFO_LOAD_WIRE_SIZE,
};
use protocol::item_pos::ItemPos;

#[test]
fn the_header_only_records_are_wired() {
    assert_eq!(CgText::header(), HEADER_CG_TEXT);
    assert_eq!(CgText::WIRE_SIZE, 1);
    assert_eq!(CG_TEXT_WIRE_SIZE, 1);
    let t = CgText;
    assert_eq!(t.encode(), vec![HEADER_CG_TEXT.value()]);

    // The keepalive byte is not a HEADER_CG_* constant in the legacy enum; it
    // is a separate fixed byte handled before the map lookup.
    assert_eq!(CgEnvanterBlack::header(), ENVANTER_BLACK);
    assert_eq!(CgEnvanterBlack::WIRE_SIZE, 1);
    assert_eq!(ENVANTER_BLACK_WIRE_SIZE, 1);
    assert_eq!(CgEnvanterBlack.encode(), vec![0xe2]);
}

#[test]
fn the_vid_records_are_wired() {
    let c = CgOnClick::new(0xDEAD_BEEF);
    assert_eq!(CgOnClick::header(), HEADER_CG_ON_CLICK);
    assert_eq!(CG_ON_CLICK_WIRE_SIZE, 5);
    assert_eq!(
        c.encode(),
        vec![HEADER_CG_ON_CLICK.value(), 0xEF, 0xBE, 0xAD, 0xDE]
    );
    assert_eq!(CgOnClick::decode(&c.encode()).expect("decodes"), c);

    let t = CgTargetInfoLoad::new(7);
    assert_eq!(CgTargetInfoLoad::header(), HEADER_CG_TARGET_INFO_LOAD);
    assert_eq!(CG_TARGET_INFO_LOAD_WIRE_SIZE, 5);
    assert_eq!(
        t.encode(),
        vec![HEADER_CG_TARGET_INFO_LOAD.value(), 7, 0, 0, 0]
    );
    assert_eq!(CgTargetInfoLoad::decode(&t.encode()).expect("decodes"), t);
}

#[test]
fn both_fly_target_headers_share_one_body_and_stay_distinct() {
    // One width constant, because both headers share one body.
    assert_eq!(CG_FLY_TARGETING_WIRE_SIZE, 13);
    assert_eq!(CG_FLY_TARGETING_PAYLOAD_SIZE, 12);
    assert_eq!(CgFlyTarget::WIRE_SIZE, 13);

    // The two 12-byte bodies are identical, so the header is the only thing
    // that tells the two actions apart. It must be retained, not checked away,
    // and `new` has no default-variant shortcut.
    let f = CgFlyTarget::new(CgFlyTargetHeader::FlyTarget, 0x0102_0304, -5, 7);
    let a = CgFlyTarget::new(CgFlyTargetHeader::AddFlyTarget, 0x0102_0304, -5, 7);
    assert_eq!(f.dw_target_vid, a.dw_target_vid);
    assert_eq!((f.x, f.y), (a.x, a.y));
    assert_eq!(&f.encode()[1..], &a.encode()[1..]);
    assert_ne!(f.header_byte(), a.header_byte());
    assert_eq!(f.header_byte(), HEADER_CG_FLY_TARGETING.value());
    assert_eq!(a.header_byte(), HEADER_CG_ADD_FLY_TARGETING.value());
    assert_eq!(CgFlyTarget::decode(&f.encode()).expect("decodes"), f);
    assert_eq!(CgFlyTarget::decode(&a.encode()).expect("decodes"), a);
    assert_eq!(
        CgFlyTargetHeader::from_value(f.header_byte()),
        Some(CgFlyTargetHeader::FlyTarget)
    );
    assert_eq!(CgFlyTargetHeader::ALL.len(), 2);
}

#[test]
fn the_opaque_u8_records_are_wired() {
    macro_rules! wired {
        ($ty:ty, $hdr:expr, $value:expr) => {{
            let r = <$ty>::new($value);
            assert_eq!(<$ty>::header(), $hdr, stringify!($ty));
            assert_eq!(
                <$ty>::decode(&r.encode()).expect("decodes"),
                r,
                stringify!($ty)
            );
            let f = r.to_frame();
            assert_eq!(
                <$ty>::decode_frame(&f).expect("frame decodes"),
                r,
                stringify!($ty)
            );
        }};
    }
    wired!(CgChangeLanguage, HEADER_CG_CHANGE_LANGUAGE, 9);
    wired!(CgRequestEventData, HEADER_CG_REQUEST_EVENT_DATA, 12);
    wired!(CgPremiumPlayers, HEADER_CG_PREMIUM_PLAYERS, 1);
    wired!(CgRefineElement, HEADER_CG_REFINE_ELEMENT, 255);
    wired!(CgWorldBoss, HEADER_CG_WORLD_BOSS, 2);
    // DailyGift is the only two-argument member of the family.
    let d = CgDailyGift::new(1, 3);
    assert_eq!(CgDailyGift::header(), HEADER_CG_DAILY_GIFT);
    assert_eq!(CgDailyGift::decode(&d.encode()).expect("decodes"), d);
    assert_eq!(
        CgDailyGift::decode_frame(&d.to_frame()).expect("frame decodes"),
        d
    );
    let b = CgBiologist::new(1, 2, 3, 4, 5);
    assert_eq!(CgBiologist::header(), HEADER_CG_BIOLOGIST);
    assert_eq!(CgBiologist::decode(&b.encode()).expect("decodes"), b);
    assert_eq!(CG_WORLD_BOSS_WIRE_SIZE, 2);
    assert_eq!(CG_DAILY_GIFT_WIRE_SIZE, 3);
    assert_eq!(CG_BIOLOGIST_WIRE_SIZE, 6);
}

#[test]
fn the_refinement_records_are_wired() {
    let r = CgRefine::new(3, 4);
    assert_eq!(CgRefine::header(), HEADER_CG_REFINE);
    assert_eq!(r.encode(), vec![HEADER_CG_REFINE.value(), 3, 4]);
    assert_eq!(CgRefine::decode(&r.encode()).expect("decodes"), r);

    let d = Attr67AddData::new(1, 2, 3, 4);
    assert_eq!(ATTR67_ADD_DATA_WIRE_SIZE, 6);
    assert_eq!(d.encode().len(), 6);
    let a = CgAttr67Add::new(0, d);
    assert_eq!(CgAttr67Add::header(), HEADER_CG_ATTR67_ADD);
    assert_eq!(a.encode().len(), 8);
    assert_eq!(CgAttr67Add::decode(&a.encode()).expect("decodes"), a);
}

#[test]
fn the_three_fourteen_byte_records_are_wired() {
    let c = CgCubeRenewal::new(0, 1, 2, 3);
    assert_eq!(CgCubeRenewal::header(), HEADER_CG_CUBE_RENEWAL);
    assert_eq!(CG_CUBE_RENEWAL_WIRE_SIZE, 14);
    assert_eq!(c.encode().len(), 14);
    assert_eq!(CgCubeRenewal::decode(&c.encode()).expect("decodes"), c);

    let g = CgAnswerMakeGuild::new([b'A'; 13]);
    assert_eq!(CgAnswerMakeGuild::header(), HEADER_CG_ANSWER_MAKE_GUILD);
    assert_eq!(CG_ANSWER_MAKE_GUILD_WIRE_SIZE, 14);
    assert_eq!(g.encode().len(), 14);
    assert_eq!(CgAnswerMakeGuild::decode(&g.encode()).expect("decodes"), g);

    let e = CgExchange::new(0, 1, 2, ItemPos::new(0, 0));
    assert_eq!(CgExchange::header(), HEADER_CG_EXCHANGE);
    assert_eq!(CG_EXCHANGE_WIRE_SIZE, 14);
    assert_eq!(e.encode().len(), 14);
    // The recorded inactive profile is exactly 4 bytes shorter.
    assert_eq!(CG_EXCHANGE_DWORD_PROFILE_SIZE, 10);
    assert_eq!(CgExchange::decode(&e.encode()).expect("decodes"), e);
}

#[test]
fn the_two_records_over_sixteen_bytes_are_wired() {
    let h = CgHack::new([0u8; 256]);
    assert_eq!(CgHack::header(), HEADER_CG_HACK);
    assert_eq!(CG_HACK_WIRE_SIZE, 257);
    assert_eq!(h.encode().len(), 257);
    assert_eq!(CgHack::decode(&h.encode()).expect("decodes"), h);

    let l = CgLogin3::default();
    assert_eq!(CgLogin3::header(), HEADER_CG_LOGIN3);
    assert_eq!(CG_LOGIN3_WIRE_SIZE, 66);
    assert_eq!(CG_LOGIN3_CLIENT_WIDTH, 69);
    assert_eq!(l.encode().len(), 66);
    assert_eq!(CgLogin3::decode(&l.encode()).expect("decodes"), l);

    // The client's 69 bytes are a named failure, not a silent acceptance.
    let mut over = l.encode();
    over.extend_from_slice(&[0, 0, 0]);
    assert_eq!(
        CgLogin3::decode(&over).expect_err("69 bytes rejected"),
        CgLogin3Error::ClientWidth
    );
}

#[test]
fn the_two_records_the_stricter_metric_revealed_are_wired() {
    // These two were silently counted as implemented by a substring metric.
    // Their `pub mod` lines are what keep that honest.
    let c = CgChangeLook::new(0, 0, 0, ItemPos::new(0, 0));
    assert_eq!(CgChangeLook::header(), HEADER_CG_CL);
    assert_eq!(CG_CHANGE_LOOK_WIRE_SIZE, 10);
    assert_eq!(c.encode().len(), 10);
    assert_eq!(CgChangeLook::decode(&c.encode()).expect("decodes"), c);

    let g = CgGayaSystem::new(0, -1);
    assert_eq!(CgGayaSystem::header(), HEADER_CG_GAYA_SYSTEM);
    assert_eq!(CG_GAYA_SYSTEM_WIRE_SIZE, 6);
    assert_eq!(g.encode().len(), 6);
    assert_eq!(CgGayaSystem::decode(&g.encode()).expect("decodes"), g);
    // The 241 collision is a code-visible fact.
    assert_eq!(CG_GAYA_SYSTEM_SHADOWED_BY, "TPacketCGClientVersion2");
}

#[test]
fn every_new_header_is_distinct_from_its_neighbours() {
    // The fly-target pair and the 241/233/69/105 neighbourhood all live in a
    // crowded header space; a copy-paste constant would be a silent collision.
    let headers = [
        HEADER_CG_TEXT,
        ENVANTER_BLACK,
        HEADER_CG_ON_CLICK,
        HEADER_CG_TARGET_INFO_LOAD,
        HEADER_CG_FLY_TARGETING,
        HEADER_CG_ADD_FLY_TARGETING,
        HEADER_CG_CHANGE_LANGUAGE,
        HEADER_CG_REQUEST_EVENT_DATA,
        HEADER_CG_PREMIUM_PLAYERS,
        HEADER_CG_REFINE_ELEMENT,
        HEADER_CG_WORLD_BOSS,
        HEADER_CG_DAILY_GIFT,
        HEADER_CG_BIOLOGIST,
        HEADER_CG_REFINE,
        HEADER_CG_ATTR67_ADD,
        HEADER_CG_CUBE_RENEWAL,
        HEADER_CG_ANSWER_MAKE_GUILD,
        HEADER_CG_EXCHANGE,
        HEADER_CG_HACK,
        HEADER_CG_LOGIN3,
        HEADER_CG_CL,
        HEADER_CG_GAYA_SYSTEM,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for h in headers {
        assert!(h.value() > 0, "header 0 is the framing special case");
        assert!(
            seen.insert(h.value()),
            "duplicate header {:#04x}",
            h.value()
        );
    }
    assert_eq!(seen.len(), 22);
    // `ENVANTER_BLACK` is the one entry with no `HEADER_CG_`-prefixed twin: the
    // legacy keepalive byte is not in the CG header enum. It is 0xe2 and it is
    // deliberately reachable from the same table.
    assert_eq!(ENVANTER_BLACK.value(), 0xe2);
    assert_eq!(ENVANTER_BLACK, CgEnvanterBlack::header());
    // And each is a real value, not the placeholder the old substring scan hit.
    let u8v: u8 = HEADER_CG_LOGIN3.into();
    assert_eq!(u8v, 111);
    assert_eq!(u8v, HEADER_CG_LOGIN3.value());
}

// ---------------------------------------------------------------------------
// Section 164: the small fixed-width game-to-client records.
// ---------------------------------------------------------------------------

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------

use protocol::gc_small::{
    GcDragonSoulChangeAttrResult, GcHeaderAndByte, GcHeaderOnly, GcQuickSlot, GcQuickSlotAdd,
    GcQuickSlotSwap, GcSkillCoolTimeEnd, GcSmallError, GcUnk213, GC_HEADER_AND_BYTE_WIRE_SIZE,
    GC_ONE_BYTE_WIRE_SIZE, GC_QUICK_SLOT_ADD_WIRE_SIZE, HEADER_GC_CHANGE_SKILL_GROUP,
    HEADER_GC_CHANNEL, HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN, HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
    HEADER_GC_EMPIRE, HEADER_GC_EVENT_RELOAD, HEADER_GC_LOVE_POINT_UPDATE, HEADER_GC_MALL_OPEN,
    HEADER_GC_PARTY_PARAMETER, HEADER_GC_QUICKSLOT_ADD, HEADER_GC_QUICKSLOT_DEL,
    HEADER_GC_QUICKSLOT_SWAP, HEADER_GC_REQUEST_CHANGE_LANGUAGE, HEADER_GC_REQUEST_MAKE_GUILD,
    HEADER_GC_SAFEBOX_SIZE, HEADER_GC_SAFEBOX_WRONG_PASSWORD, HEADER_GC_SKILL_COOLTIME_END,
    HEADER_GC_UNK_213,
};

#[test]
fn the_one_byte_game_to_client_records_round_trip_through_the_public_api() {
    assert_eq!(GC_ONE_BYTE_WIRE_SIZE, 1);
    for header in [
        HEADER_GC_REQUEST_MAKE_GUILD,
        HEADER_GC_SAFEBOX_WRONG_PASSWORD,
        HEADER_GC_EVENT_RELOAD,
        HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
    ] {
        let record = GcHeaderOnly::new(header);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire, vec![header]);
        assert_eq!(GcHeaderOnly::decode(&wire).unwrap(), record);
    }
}

#[test]
fn the_nine_header_and_byte_game_to_client_records_agree_with_the_inventory() {
    assert_eq!(GC_HEADER_AND_BYTE_WIRE_SIZE, 2);
    // Every header the shared two-byte shape covers must be a distinct wire byte,
    // because the header is the only thing separating these legacy records.
    let headers: BTreeSet<u8> = [
        HEADER_GC_QUICKSLOT_DEL,
        HEADER_GC_SKILL_COOLTIME_END,
        HEADER_GC_PARTY_PARAMETER,
        HEADER_GC_SAFEBOX_SIZE,
        HEADER_GC_EMPIRE,
        HEADER_GC_CHANGE_SKILL_GROUP,
        HEADER_GC_CHANNEL,
        HEADER_GC_MALL_OPEN,
        HEADER_GC_LOVE_POINT_UPDATE,
        HEADER_GC_UNK_213,
        HEADER_GC_REQUEST_CHANGE_LANGUAGE,
        HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
    ]
    .into_iter()
    .collect();
    assert_eq!(headers.len(), 12);
    for header in headers {
        let record = GcHeaderAndByte::new(header, 0xa5);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire, vec![header, 0xa5]);
        assert_eq!(GcHeaderAndByte::decode(&wire).unwrap().header, header);
    }
}

#[test]
fn the_quick_slot_family_shares_one_slot_type_and_keeps_its_own_widths() {
    let slot = GcQuickSlot {
        slot_type: 1,
        position: 5,
    };
    assert_eq!(GcQuickSlot::WIRE_SIZE, 2);

    let add = GcQuickSlotAdd { pos: 2, slot };
    let mut wire = Vec::new();
    add.encode_into(&mut wire);
    assert_eq!(wire.len(), GC_QUICK_SLOT_ADD_WIRE_SIZE);
    assert_eq!(GC_QUICK_SLOT_ADD_WIRE_SIZE, 4);
    assert_eq!(wire, vec![HEADER_GC_QUICKSLOT_ADD, 2, 1, 5]);
    assert_eq!(GcQuickSlotAdd::decode(&wire).unwrap(), add);

    let swap = GcQuickSlotSwap {
        pos: 2,
        change_pos: 7,
    };
    let mut wire = Vec::new();
    swap.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_QUICKSLOT_SWAP, 2, 7]);
    assert_eq!(GcQuickSlotSwap::decode(&wire).unwrap(), swap);
}

#[test]
fn the_client_only_game_to_client_records_are_usable_through_the_public_api() {
    // Bytes 73, 112, and 213 have no server producer, but the client registers
    // and decodes them, so the framing is real and must not be dropped.
    let cool = GcSkillCoolTimeEnd { skill: 12 };
    let mut wire = Vec::new();
    cool.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_SKILL_COOLTIME_END, 12]);
    assert_eq!(GcSkillCoolTimeEnd::decode(&wire).unwrap(), cool);

    let unk = GcUnk213 { value: 0xff };
    let mut wire = Vec::new();
    unk.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_UNK_213, 0xff]);
    assert_eq!(GcUnk213::decode(&wire).unwrap(), unk);

    let result = GcDragonSoulChangeAttrResult { result: 0x02 };
    let mut wire = Vec::new();
    result.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT, 0x02]);
    assert_eq!(GcDragonSoulChangeAttrResult::decode(&wire).unwrap(), result);

    let group = GcHeaderAndByte::new(HEADER_GC_CHANGE_SKILL_GROUP, 3);
    let mut wire = Vec::new();
    group.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_CHANGE_SKILL_GROUP, 3]);
}

#[test]
fn the_game_to_client_inventory_and_the_small_codec_agree() {
    use protocol::gc_inventory::{gc_missing_codec_count, resolve_gc_packet, GcFraming};

    assert_eq!(gc_missing_codec_count(), 30);
    // Every header this module decodes must be a registered client row that the
    // inventory marks implemented. This is the check that keeps the metric and
    // the codecs from drifting apart.
    let mut checked = 0usize;
    for header in [
        HEADER_GC_REQUEST_MAKE_GUILD,
        HEADER_GC_SAFEBOX_WRONG_PASSWORD,
        HEADER_GC_EVENT_RELOAD,
        HEADER_GC_DS_PLUS_CHANGE_ATTR_OPEN,
        HEADER_GC_DS_PLUS_CHANGE_ATTR_RESULT,
        HEADER_GC_QUICKSLOT_ADD,
        HEADER_GC_QUICKSLOT_DEL,
        HEADER_GC_QUICKSLOT_SWAP,
        HEADER_GC_SKILL_COOLTIME_END,
        HEADER_GC_PARTY_PARAMETER,
        HEADER_GC_SAFEBOX_SIZE,
        HEADER_GC_EMPIRE,
        HEADER_GC_CHANGE_SKILL_GROUP,
        HEADER_GC_CHANNEL,
        HEADER_GC_MALL_OPEN,
        HEADER_GC_LOVE_POINT_UPDATE,
        HEADER_GC_UNK_213,
        HEADER_GC_REQUEST_CHANGE_LANGUAGE,
    ] {
        let entry = resolve_gc_packet(header)
            .unwrap_or_else(|| panic!("byte {header:#04x} is not a registered client row"));
        assert!(
            entry.implemented_in_rust,
            "{} is not marked implemented",
            entry.client_name
        );
        assert_eq!(entry.framing, GcFraming::StaticSize);
        checked += 1;
    }
    assert_eq!(checked, 18);
}

#[test]
fn a_wrong_header_is_reported_as_a_named_error_not_a_silent_misread() {
    let err = GcQuickSlotAdd::decode(&[HEADER_GC_QUICKSLOT_DEL, 0, 0, 0]).unwrap_err();
    assert!(matches!(err, GcSmallError::Header { expected, actual, .. }
        if expected == HEADER_GC_QUICKSLOT_ADD && actual == HEADER_GC_QUICKSLOT_DEL));
    assert!(err.to_string().contains("GcQuickSlotAdd"));
}

// ---------------------------------------------------------------------------
// Section 165: the game-to-client records that carry one 32-bit value.
// ---------------------------------------------------------------------------

use protocol::gc_vid::GcSmallError as GcVidSmallError;
use protocol::gc_vid::{
    GcChangeSpeed, GcDragonSoulRefine, GcEventKwScore, GcFishing, GcFishingSubheader,
    GcHeaderAndDword, GcHeaderAndDwordAndByte, GcRefineElement, GcSpecialEffect,
    GC_CHANGE_SPEED_WIRE_SIZE, GC_DRAGON_SOUL_REFINE_WIRE_SIZE, GC_EVENT_KW_SCORE_WIRE_SIZE,
    GC_FISHING_WIRE_SIZE, GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE, GC_HEADER_AND_DWORD_WIRE_SIZE,
    GC_REFINE_ELEMENT_WIRE_SIZE, HEADER_GC_AFFECT_REMOVE, HEADER_GC_CHANGE_SPEED,
    HEADER_GC_CHARACTER_DEL, HEADER_GC_CHARACTER_POSITION, HEADER_GC_DEAD,
    HEADER_GC_DRAGON_SOUL_REFINE, HEADER_GC_EVENT_KW_SCORE, HEADER_GC_FISHING,
    HEADER_GC_ITEM_GROUND_DEL, HEADER_GC_MALL_DEL, HEADER_GC_PARTY_INVITE, HEADER_GC_PARTY_REMOVE,
    HEADER_GC_REFINE_ELEMENT, HEADER_GC_SAFEBOX_DEL, HEADER_GC_SAFEBOX_MONEY_CHANGE,
    HEADER_GC_SEPCIAL_EFFECT, HEADER_GC_STUN, HEADER_GC_TARGET_DELETE, HEADER_GC_TIME,
    HEADER_GC_WALK_MODE,
};

#[test]
fn the_eleven_header_and_dword_game_to_client_records_are_five_bytes() {
    assert_eq!(GC_HEADER_AND_DWORD_WIRE_SIZE, 5);
    let mut headers: Vec<u8> = vec![
        HEADER_GC_CHARACTER_DEL,
        HEADER_GC_STUN,
        HEADER_GC_DEAD,
        HEADER_GC_ITEM_GROUND_DEL,
        HEADER_GC_PARTY_INVITE,
        HEADER_GC_PARTY_REMOVE,
        HEADER_GC_SAFEBOX_MONEY_CHANGE,
        HEADER_GC_SAFEBOX_DEL,
        HEADER_GC_TIME,
        HEADER_GC_TARGET_DELETE,
        HEADER_GC_MALL_DEL,
    ];
    headers.sort_unstable();
    headers.dedup();
    assert_eq!(headers.len(), 11, "each record needs its own wire byte");
    for header in headers {
        let record = GcHeaderAndDword::new(header, 0xdead_beef);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 5, "byte {header:#04x}");
        assert_eq!(wire[0], header);
        assert_eq!(GcHeaderAndDword::decode(&wire).unwrap(), record);
    }
}

#[test]
fn the_three_header_dword_and_byte_game_to_client_records_are_six_bytes() {
    assert_eq!(GC_HEADER_AND_DWORD_AND_BYTE_WIRE_SIZE, 6);
    for header in [
        HEADER_GC_CHARACTER_POSITION,
        HEADER_GC_WALK_MODE,
        HEADER_GC_AFFECT_REMOVE,
    ] {
        let record = GcHeaderAndDwordAndByte::new(header, 7, 3);
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        assert_eq!(wire.len(), 6);
        assert_eq!(wire, vec![header, 7, 0, 0, 0, 3]);
        assert_eq!(GcHeaderAndDwordAndByte::decode(&wire).unwrap(), record);
    }
}

#[test]
fn the_four_single_field_game_to_client_records_keep_their_own_field_order() {
    // Each of these is 6 or 7 bytes with a different field layout, so each needs
    // its own type. Transposing any pair of them would still produce the right
    // length, which is why the round trips below are byte-exact.
    let effect = GcSpecialEffect {
        effect_type: 0x0a,
        vid: 0x0b0c_0d0e,
    };
    let mut wire = Vec::new();
    effect.encode_into(&mut wire);
    assert_eq!(
        wire,
        vec![HEADER_GC_SEPCIAL_EFFECT, 0x0a, 0x0e, 0x0d, 0x0c, 0x0b]
    );

    let refine = GcRefineElement {
        src_cell: 1,
        dst_cell: 2,
        element_type: 3,
    };
    let mut wire = Vec::new();
    refine.encode_into(&mut wire);
    assert_eq!(wire, vec![HEADER_GC_REFINE_ELEMENT, 1, 0, 2, 0, 3]);
    assert_eq!(GC_REFINE_ELEMENT_WIRE_SIZE, 6);

    let speed = GcChangeSpeed {
        vid: 0x1122_3344,
        moving_speed: 0x5566,
    };
    let mut wire = Vec::new();
    speed.encode_into(&mut wire);
    assert_eq!(
        wire,
        vec![HEADER_GC_CHANGE_SPEED, 0x44, 0x33, 0x22, 0x11, 0x66, 0x55]
    );
    assert_eq!(GC_CHANGE_SPEED_WIRE_SIZE, 7);

    let score = GcEventKwScore::new([0x1111, 0x2222, 0x3333]);
    let mut wire = Vec::new();
    score.encode_into(&mut wire);
    assert_eq!(
        wire,
        vec![HEADER_GC_EVENT_KW_SCORE, 0x11, 0x11, 0x22, 0x22, 0x33, 0x33]
    );
    assert_eq!(GC_EVENT_KW_SCORE_WIRE_SIZE, 7);

    // The last `wire` holds an EVENT_KW_SCORE frame. Reading it as a special
    // effect must be refused by the header check, not silently reinterpreted.
    assert_eq!(
        GcSpecialEffect::decode(&wire),
        Err(GcVidSmallError::Header {
            context: "GcSpecialEffect",
            expected: HEADER_GC_SEPCIAL_EFFECT,
            actual: HEADER_GC_EVENT_KW_SCORE,
        })
    );
}

#[test]
fn the_dragon_soul_refine_game_to_client_record_reuses_the_shared_grid_slot() {
    let record = GcDragonSoulRefine {
        sub_type: 1,
        pos: ItemPos::new(0x41, 7),
    };
    let mut wire = Vec::new();
    record.encode_into(&mut wire);
    assert_eq!(GC_DRAGON_SOUL_REFINE_WIRE_SIZE, 5);
    assert_eq!(wire, vec![HEADER_GC_DRAGON_SOUL_REFINE, 1, 0x41, 7, 0]);
    let decoded = GcDragonSoulRefine::decode(&wire).unwrap();
    assert_eq!(decoded.pos, ItemPos::new(0x41, 7));
}

#[test]
fn the_fishing_game_to_client_record_exposes_both_meanings_of_info() {
    assert_eq!(GC_FISHING_WIRE_SIZE, 7);
    let react = GcFishing {
        subheader: GcFishingSubheader::React,
        info: 999,
        dir: 1,
    };
    let mut wire = Vec::new();
    react.encode_into(&mut wire);
    assert_eq!(wire[1], 2, "REACT is enumerator position 2");
    let decoded = GcFishing::decode(&wire).unwrap();
    assert_eq!(decoded.actor_vid(), Some(999));
    assert_eq!(decoded.fish_vnum(), None);

    let fish = GcFishing {
        subheader: GcFishingSubheader::Fish,
        info: 50008,
        dir: 0,
    };
    let mut wire = Vec::new();
    fish.encode_into(&mut wire);
    let decoded = GcFishing::decode(&wire).unwrap();
    assert_eq!(decoded.fish_vnum(), Some(50008));
    assert_eq!(decoded.actor_vid(), None);
    assert_eq!(
        wire.len(),
        7,
        "the frame length does not depend on the subheader"
    );

    let unknown = GcFishing::decode(&[HEADER_GC_FISHING, 250, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(unknown.subheader, GcFishingSubheader::Unknown(250));
    assert!(!unknown.subheader.is_known());
}

#[test]
fn the_game_to_client_inventory_agrees_with_the_thirty_two_bit_value_codecs() {
    use protocol::gc_inventory::{gc_missing_codec_count, resolve_gc_packet, GcFraming};

    assert_eq!(gc_missing_codec_count(), 30);
    let mut checked = 0usize;
    for header in [
        HEADER_GC_CHARACTER_DEL,
        HEADER_GC_STUN,
        HEADER_GC_DEAD,
        HEADER_GC_ITEM_GROUND_DEL,
        HEADER_GC_PARTY_INVITE,
        HEADER_GC_PARTY_REMOVE,
        HEADER_GC_SAFEBOX_MONEY_CHANGE,
        HEADER_GC_SAFEBOX_DEL,
        HEADER_GC_TIME,
        HEADER_GC_TARGET_DELETE,
        HEADER_GC_MALL_DEL,
        HEADER_GC_CHARACTER_POSITION,
        HEADER_GC_WALK_MODE,
        HEADER_GC_AFFECT_REMOVE,
        HEADER_GC_SEPCIAL_EFFECT,
        HEADER_GC_REFINE_ELEMENT,
        HEADER_GC_CHANGE_SPEED,
        HEADER_GC_EVENT_KW_SCORE,
        HEADER_GC_DRAGON_SOUL_REFINE,
        HEADER_GC_FISHING,
    ] {
        let entry = resolve_gc_packet(header)
            .unwrap_or_else(|| panic!("byte {header:#04x} is not a registered client row"));
        assert!(
            entry.implemented_in_rust,
            "{} not marked implemented",
            entry.client_name
        );
        assert_eq!(entry.framing, GcFraming::StaticSize);
        checked += 1;
    }
    assert_eq!(checked, 20);
}

#[test]
fn a_truncated_32_bit_value_record_reports_the_length_it_needed() {
    let err = GcEventKwScore::decode(&[HEADER_GC_EVENT_KW_SCORE, 0, 0]).unwrap_err();
    assert_eq!(
        err,
        GcVidSmallError::Truncated {
            context: "GcEventKwScore",
            needed: 7,
            actual: 3,
        }
    );
    assert!(err.to_string().contains("need 7 bytes"));
}

// ---------------------------------------------------------------------------
// Section 166: the game-to-client records with named fields and raw char arrays.
// ---------------------------------------------------------------------------

use protocol::gc_fields::HEADER_GC_CHARACTER_GOLD as GcGoldHeader;
use protocol::gc_fields::{
    GcCreateFly, GcFieldsError, GcGold, GcLoverInfo, GcMotion, GcNamed, GcPickupItem, GcPoints,
    GcPvp, GcQuestConfirm, GcShamanSkill, GcShopSign, GcSpecificEffect, GcTargetUpdate, GcTwoWord,
    GC_CREATE_FLY_WIRE_SIZE, GC_GOLD_WIRE_SIZE, GC_LOVER_INFO_WIRE_SIZE, GC_MOTION_WIRE_SIZE,
    GC_NAMED_WIRE_SIZE, GC_NAME_FIELD_SIZE, GC_PICKUP_ITEM_WIRE_SIZE, GC_POINTS_WIRE_SIZE,
    GC_POINT_SLOT_COUNT, GC_PVP_WIRE_SIZE, GC_QUEST_CONFIRM_WIRE_SIZE, GC_QUEST_MESSAGE_FIELD_SIZE,
    GC_SHAMAN_SKILL_WIRE_SIZE, GC_SHOP_SIGN_FIELD_SIZE, GC_SHOP_SIGN_WIRE_SIZE,
    GC_SPECIFIC_EFFECT_WIRE_SIZE, GC_TARGET_UPDATE_WIRE_SIZE, GC_TWO_WORD_WIRE_SIZE,
    HEADER_GC_AUTO_SHAMAN_SKILL, HEADER_GC_CHANGE_NAME, HEADER_GC_CHARACTER_GOLD,
    HEADER_GC_CREATE_FLY, HEADER_GC_ITEM_OWNERSHIP, HEADER_GC_LOVER_INFO, HEADER_GC_MOTION,
    HEADER_GC_OWNERSHIP, HEADER_GC_PARTY_ADD, HEADER_GC_PARTY_LINK, HEADER_GC_PARTY_UNLINK,
    HEADER_GC_PICKUP_ITEM_SC, HEADER_GC_PLAYER_POINTS, HEADER_GC_PVP, HEADER_GC_QUEST_CONFIRM,
    HEADER_GC_SHOP_SIGN, HEADER_GC_SPECIFIC_EFFECT, HEADER_GC_TARGET_UPDATE,
};

#[test]
fn the_named_game_to_client_records_cover_three_headers_with_one_30_byte_type() {
    assert_eq!(GC_NAME_FIELD_SIZE, 25);
    assert_eq!(GC_NAMED_WIRE_SIZE, 30);
    for header in [
        HEADER_GC_ITEM_OWNERSHIP,
        HEADER_GC_PARTY_ADD,
        HEADER_GC_CHANGE_NAME,
    ] {
        let mut name = [0u8; GC_NAME_FIELD_SIZE];
        name[0] = b'm';
        let mut wire = Vec::new();
        GcNamed::new(header, 0x1122_3344, name).encode_into(&mut wire);
        assert_eq!(wire.len(), 30, "byte {header:#04x}");
        assert_eq!(wire[0], header);
        assert_eq!(&wire[1..5], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(wire[5], b'm');
        let decoded = GcNamed::decode(&wire).unwrap();
        assert_eq!(decoded.header, header);
        assert_eq!(decoded.id, 0x1122_3344);
        assert_eq!(decoded.name, name);
    }
}

#[test]
fn the_two_word_game_to_client_records_cover_three_headers_with_one_9_byte_type() {
    assert_eq!(GC_TWO_WORD_WIRE_SIZE, 9);
    // party.cpp:729 sends the unlink record through TPacketGCPartyLink, so all
    // three headers must be distinguishable by the header byte alone.
    let mut headers: Vec<u8> = Vec::new();
    for header in [
        HEADER_GC_OWNERSHIP,
        HEADER_GC_PARTY_LINK,
        HEADER_GC_PARTY_UNLINK,
    ] {
        let mut wire = Vec::new();
        GcTwoWord::new(header, 1, 2).encode_into(&mut wire);
        assert_eq!(wire.len(), 9);
        headers.push(wire[0]);
        let decoded = GcTwoWord::decode(&wire).unwrap();
        assert_eq!(
            (decoded.header, decoded.first, decoded.second),
            (header, 1, 2)
        );
    }
    headers.sort_unstable();
    headers.dedup();
    assert_eq!(headers.len(), 3);
}

#[test]
fn the_six_fixed_layout_game_to_client_records_keep_their_measured_widths() {
    assert_eq!(GC_PVP_WIRE_SIZE, 10);
    assert_eq!(GC_PICKUP_ITEM_WIRE_SIZE, 9);
    assert_eq!(GC_MOTION_WIRE_SIZE, 11);
    assert_eq!(GC_CREATE_FLY_WIRE_SIZE, 10);
    assert_eq!(GC_SHAMAN_SKILL_WIRE_SIZE, 10);
    assert_eq!(GC_TARGET_UPDATE_WIRE_SIZE, 13);

    let mut wire = Vec::new();
    GcPvp::new(0x0a0b_0c0d, 0x0102_0304, 0xff).encode_into(&mut wire);
    assert_eq!(wire.len(), 10);
    assert_eq!(
        GcPvp::decode(&wire).unwrap().mode,
        0xff,
        "an unknown mode survives"
    );

    let mut wire = Vec::new();
    GcPickupItem::new(-1, -2).encode_into(&mut wire);
    assert_eq!(wire.len(), 9);
    let decoded = GcPickupItem::decode(&wire).unwrap();
    assert_eq!(
        (decoded.item_vnum, decoded.item_count),
        (-1, -2),
        "int is signed"
    );

    let mut wire = Vec::new();
    GcMotion::new(1, 0, u16::MAX).encode_into(&mut wire);
    assert_eq!(wire.len(), 11);
    assert_eq!(GcMotion::decode(&wire).unwrap().motion, 65535);

    let mut wire = Vec::new();
    GcCreateFly::new(0x0a, 1, 2).encode_into(&mut wire);
    assert_eq!(
        wire,
        vec![HEADER_GC_CREATE_FLY, 0x0a, 1, 0, 0, 0, 2, 0, 0, 0]
    );

    let mut wire = Vec::new();
    GcShamanSkill::new(1, 2, 0x99).encode_into(&mut wire);
    assert_eq!(wire.len(), 10);
    assert_eq!(wire[9], 0x99, "dwLevel is declared BYTE, so it is one byte");

    let mut wire = Vec::new();
    GcTargetUpdate::new(-1, -2, -3).encode_into(&mut wire);
    assert_eq!(
        wire.len(),
        13,
        "long is 4 bytes on the 32-bit legacy target"
    );
    assert_eq!(
        GcTargetUpdate::decode(&wire).unwrap(),
        GcTargetUpdate::new(-1, -2, -3)
    );
}

#[test]
fn the_four_fixed_array_game_to_client_records_keep_their_measured_widths() {
    assert_eq!(GC_LOVER_INFO_WIRE_SIZE, 27);
    assert_eq!(GC_SHOP_SIGN_WIRE_SIZE, 38);
    assert_eq!(GC_SPECIFIC_EFFECT_WIRE_SIZE, 133);
    assert_eq!(GC_QUEST_CONFIRM_WIRE_SIZE, 74);

    let mut wire = Vec::new();
    GcLoverInfo::new([0u8; GC_NAME_FIELD_SIZE], 7).encode_into(&mut wire);
    assert_eq!(wire.len(), 27);
    assert_eq!(wire[26], 7, "name first, point byte last");

    let mut wire = Vec::new();
    GcShopSign::new(1, [0u8; GC_SHOP_SIGN_FIELD_SIZE]).encode_into(&mut wire);
    assert_eq!(wire.len(), 38);
    assert_eq!(
        GC_SHOP_SIGN_FIELD_SIZE, 33,
        "SHOP_SIGN_MAX_LEN + 1 == 32 + 1"
    );

    let mut wire = Vec::new();
    GcSpecificEffect::new(1, [0u8; 128]).encode_into(&mut wire);
    assert_eq!(wire.len(), 133, "MAX_EFFECT_FILE_NAME == 128");

    let mut wire = Vec::new();
    GcQuestConfirm::new([0u8; GC_QUEST_MESSAGE_FIELD_SIZE], -5, 7).encode_into(&mut wire);
    assert_eq!(wire.len(), 74);
    assert_eq!(
        &wire[66..70],
        &[0xfb, 0xff, 0xff, 0xff],
        "the long is 4 bytes"
    );
}

#[test]
fn the_two_64_bit_game_to_client_records_use_opposite_signedness() {
    assert_eq!(GC_POINT_SLOT_COUNT, 255);
    assert_eq!(GC_POINTS_WIRE_SIZE, 2041);
    assert_eq!(GC_GOLD_WIRE_SIZE, 9);

    let mut points = [0i64; GC_POINT_SLOT_COUNT];
    points[0] = i64::MIN;
    points[GC_POINT_SLOT_COUNT - 1] = i64::MAX;
    let mut wire = Vec::new();
    GcPoints::new(points).encode_into(&mut wire);
    assert_eq!(wire.len(), 2041);
    assert_eq!(
        GcPoints::decode(&wire).unwrap().points,
        points,
        "long long is signed"
    );

    let mut wire = Vec::new();
    GcGold::new(u64::MAX).encode_into(&mut wire);
    assert_eq!(wire.len(), 9);
    assert_eq!(wire[0], GcGoldHeader);
    assert_eq!(
        GcGold::decode(&wire).unwrap().gold,
        u64::MAX,
        "unsigned long long stays unsigned"
    );
}

#[test]
fn the_field_inventory_agrees_with_the_eighteen_field_records() {
    use protocol::gc_inventory::{gc_missing_codec_count, resolve_gc_packet, GcFraming};

    assert_eq!(gc_missing_codec_count(), 30);
    let mut checked = 0usize;
    for header in [
        HEADER_GC_PLAYER_POINTS,
        HEADER_GC_ITEM_OWNERSHIP,
        HEADER_GC_MOTION,
        HEADER_GC_SHOP_SIGN,
        HEADER_GC_PVP,
        HEADER_GC_QUEST_CONFIRM,
        HEADER_GC_AUTO_SHAMAN_SKILL,
        HEADER_GC_OWNERSHIP,
        HEADER_GC_PICKUP_ITEM_SC,
        HEADER_GC_CREATE_FLY,
        HEADER_GC_PARTY_ADD,
        HEADER_GC_PARTY_LINK,
        HEADER_GC_PARTY_UNLINK,
        HEADER_GC_CHANGE_NAME,
        HEADER_GC_TARGET_UPDATE,
        HEADER_GC_LOVER_INFO,
        HEADER_GC_SPECIFIC_EFFECT,
        HEADER_GC_CHARACTER_GOLD,
    ] {
        let entry = resolve_gc_packet(header)
            .unwrap_or_else(|| panic!("byte {header:#04x} is not a registered client row"));
        assert!(
            entry.implemented_in_rust,
            "{} not marked implemented",
            entry.client_name
        );
        assert_eq!(entry.framing, GcFraming::StaticSize);
        checked += 1;
    }
    assert_eq!(checked, 18);
}

#[test]
fn a_short_or_misheaded_field_record_reports_which_check_failed() {
    assert_eq!(
        GcPoints::decode(&vec![HEADER_GC_PLAYER_POINTS; 2040]).unwrap_err(),
        GcFieldsError::Truncated {
            context: "GcPoints",
            needed: 2041,
            actual: 2040,
        }
    );
    let mut buf = vec![0u8; 9];
    buf[0] = HEADER_GC_OWNERSHIP;
    assert_eq!(
        GcPickupItem::decode(&buf).unwrap_err(),
        GcFieldsError::Header {
            context: "GcPickupItem",
            expected: HEADER_GC_PICKUP_ITEM_SC,
            actual: HEADER_GC_OWNERSHIP,
        },
        "a 9-byte ownership frame must not be read as a 9-byte pickup frame"
    );
}

// ---------------------------------------------------------------------------
// Section 167: the nested and array game-to-client records in `gc_nested`.
// ---------------------------------------------------------------------------

/// The 12 records in `gc_nested` must be reachable through the public module.
#[test]
fn the_gc_nested_records_are_exported_and_widths_are_published() {
    use protocol::gc_nested::{
        GC_AFFECT_ADD_WIRE_SIZE, GC_AFFECT_ELEMENT_WIRE_SIZE, GC_CUBE_RENEWAL_DATE_WIRE_SIZE,
        GC_CUBE_RENEWAL_WIRE_SIZE, GC_DAILY_GIFT_WIRE_SIZE, GC_DAMAGE_INFO_WIRE_SIZE,
        GC_DIG_MOTION_WIRE_SIZE, GC_FLY_TARGETING_WIRE_SIZE, GC_PARTY_AFFECT_SLOT_COUNT,
        GC_PARTY_UPDATE_WIRE_SIZE, GC_POINT_CHANGE_WIRE_SIZE, GC_PREMIUM_PLAYERS_WIRE_SIZE,
        GC_SKILL_LEVEL_NEW_WIRE_SIZE, GC_SKILL_SLOT_COUNT, GC_SKILL_WIRE_SIZE, GC_WARP_WIRE_SIZE,
    };
    assert_eq!(GC_SKILL_WIRE_SIZE, 6);
    assert_eq!(GC_SKILL_SLOT_COUNT, 255);
    assert_eq!(GC_SKILL_LEVEL_NEW_WIRE_SIZE, 1531);
    assert_eq!(GC_AFFECT_ELEMENT_WIRE_SIZE, 21);
    assert_eq!(GC_AFFECT_ADD_WIRE_SIZE, 22);
    assert_eq!(GC_POINT_CHANGE_WIRE_SIZE, 25);
    assert_eq!(GC_DIG_MOTION_WIRE_SIZE, 10);
    assert_eq!(GC_DAMAGE_INFO_WIRE_SIZE, 10);
    assert_eq!(GC_PREMIUM_PLAYERS_WIRE_SIZE, 28);
    assert_eq!(GC_FLY_TARGETING_WIRE_SIZE, 17);
    assert_eq!(GC_WARP_WIRE_SIZE, 15);
    assert_eq!(GC_DAILY_GIFT_WIRE_SIZE, 111);
    assert_eq!(GC_PARTY_AFFECT_SLOT_COUNT, 7);
    assert_eq!(GC_PARTY_UPDATE_WIRE_SIZE, 21);
    assert_eq!(GC_CUBE_RENEWAL_DATE_WIRE_SIZE, 169);
    assert_eq!(GC_CUBE_RENEWAL_WIRE_SIZE, 171);
}

/// The 12 records must be registered in the inventory as implemented, and the
/// count must be reported honestly.
#[test]
fn the_gc_inventory_marks_the_twelve_nested_records_implemented() {
    use protocol::gc_inventory::{
        gc_missing_codec_count, resolve_gc_packet, LEGACY_GC_PACKET_INVENTORY,
    };
    let done: Vec<&str> = LEGACY_GC_PACKET_INVENTORY
        .iter()
        .filter(|e| e.implemented_in_rust)
        .map(|e| e.client_name)
        .collect();
    assert_eq!(done.len(), 104, "implemented game-to-client rows");
    assert_eq!(gc_missing_codec_count(), 30, "missing game-to-client rows");
    for name in [
        "HEADER_GC_AFFECT_ADD",
        "HEADER_GC_PLAYER_POINT_CHANGE",
        "HEADER_GC_DIG_MOTION",
        "HEADER_GC_DAMAGE_INFO",
        "HEADER_GC_PREMIUM_PLAYERS",
        "HEADER_GC_ADD_FLY_TARGETING",
        "HEADER_GC_FLY_TARGETING",
        "HEADER_GC_WARP",
        "HEADER_GC_SKILL_LEVEL_NEW",
        "HEADER_GC_DAILY_GIFT",
        "HEADER_GC_PARTY_UPDATE",
        "HEADER_GC_CUBE_RENEWAL",
    ] {
        assert!(done.contains(&name), "expected {name} to be implemented");
    }
    // Byte 76 is the live skill-level record. Byte 72 is the dead generation.
    assert!(resolve_gc_packet(76).is_some(), "byte 76 is registered");
    assert!(resolve_gc_packet(72).is_some(), "byte 72 is registered");
}

/// A 10-byte dig-motion frame must not be readable as a 10-byte damage frame.
#[test]
fn the_two_ten_byte_records_reject_each_other() {
    use protocol::gc_nested::{GcDamageInfo, GcDigMotion};
    let mut dig = Vec::new();
    GcDigMotion {
        header: protocol::gc::HEADER_GC_DIG_MOTION,
        vid: 0x0102_0304,
        target_vid: 0x0a0b_0c0d,
        count: 0x3d,
    }
    .encode_into(&mut dig);
    assert_eq!(dig.len(), 10, "the dig frame is 10 bytes");
    let damage = GcDamageInfo::decode(&dig).unwrap_err();
    assert_eq!(
        damage,
        protocol::gc_nested::GcNestedError::Header {
            context: "GcDamageInfo",
            expected: protocol::gc::HEADER_GC_DAMAGE_INFO,
            actual: protocol::gc::HEADER_GC_DIG_MOTION,
        },
        "a 10-byte dig frame must not be read as a damage frame"
    );
    assert_eq!(
        GcDigMotion::decode(&dig).unwrap().count,
        0x3d,
        "the dig frame"
    );
}

/// The two skill-level generations share a byte count but not a body, so byte
/// 72 must never be treated as the live 1531-byte record.
#[test]
fn the_dead_skill_generation_is_not_the_live_record() {
    use protocol::gc_nested::{
        GcSkillLevelNew, GC_SKILL_LEVEL_NEW_WIRE_SIZE, GC_SKILL_LEVEL_OLD_WIRE_SIZE,
    };
    assert_ne!(GC_SKILL_LEVEL_NEW_WIRE_SIZE, GC_SKILL_LEVEL_OLD_WIRE_SIZE);
    assert_eq!(GC_SKILL_LEVEL_NEW_WIRE_SIZE, 1531, "the live byte-76 body");
    assert_eq!(GC_SKILL_LEVEL_OLD_WIRE_SIZE, 256, "the dead byte-72 body");
    // A 256-byte frame is too short for the live record, not a valid one.
    let mut short = vec![0u8; GC_SKILL_LEVEL_OLD_WIRE_SIZE];
    short[0] = protocol::gc::HEADER_GC_SKILL_LEVEL_NEW;
    assert!(
        GcSkillLevelNew::decode(&short).is_err(),
        "a 256-byte prefix must not decode as the live record"
    );
}

/// The public header constants must match the legacy byte values.
#[test]
fn the_new_header_constants_match_the_legacy_byte_values() {
    use protocol::gc::{
        HEADER_GC_ADD_FLY_TARGETING, HEADER_GC_AFFECT_ADD, HEADER_GC_CUBE_RENEWAL,
        HEADER_GC_DAILY_GIFT, HEADER_GC_DAMAGE_INFO, HEADER_GC_DIG_MOTION, HEADER_GC_FLY_TARGETING,
        HEADER_GC_PARTY_UPDATE, HEADER_GC_PLAYER_POINT_CHANGE, HEADER_GC_PREMIUM_PLAYERS,
        HEADER_GC_SKILL_LEVEL_NEW, HEADER_GC_WARP,
    };
    assert_eq!(HEADER_GC_WARP, 65);
    assert_eq!(HEADER_GC_ADD_FLY_TARGETING, 69);
    assert_eq!(HEADER_GC_FLY_TARGETING, 71);
    assert_eq!(HEADER_GC_SKILL_LEVEL_NEW, 76);
    assert_eq!(HEADER_GC_PARTY_UPDATE, 79);
    assert_eq!(HEADER_GC_AFFECT_ADD, 126);
    assert_eq!(HEADER_GC_DIG_MOTION, 134);
    assert_eq!(HEADER_GC_DAMAGE_INFO, 135);
    assert_eq!(HEADER_GC_PREMIUM_PLAYERS, 141);
    assert_eq!(HEADER_GC_DAILY_GIFT, 180);
    assert_eq!(HEADER_GC_CUBE_RENEWAL, 221);
    assert_eq!(HEADER_GC_PLAYER_POINT_CHANGE, 17);
}

/// The 14 actor, target, and main-character records must be reachable through
/// the crate's public API, with the header bytes the legacy enums assign.
#[test]
fn the_gc_actor_records_are_wired_to_their_legacy_header_bytes() {
    use protocol::gc_actors::{
        GcCharacterAdd, GcCharacterAdditionalInfo, GcCharacterGoldChange, GcCharacterMove,
        GcCharacterUpdate, GcCharacterUpdate2, GcMainCharacter, GcMainCharacter2Empire,
        GcMainCharacter3Bgm, GcMainCharacter4BgmVol, GcSkillLevel, GcTarget, GcTargetCreateNew,
        GcTargetInfo,
    };
    let headers = [
        ("GcCharacterAdd", GcCharacterAdd::header()),
        ("GcCharacterMove", GcCharacterMove::header()),
        ("GcMainCharacter", GcMainCharacter::header()),
        ("GcCharacterUpdate", GcCharacterUpdate::header()),
        ("GcTargetInfo", GcTargetInfo::header()),
        ("GcTarget", GcTarget::header()),
        ("GcSkillLevel", GcSkillLevel::header()),
        ("GcMainCharacter2Empire", GcMainCharacter2Empire::header()),
        ("GcCharacterUpdate2", GcCharacterUpdate2::header()),
        ("GcTargetCreateNew", GcTargetCreateNew::header()),
        (
            "GcCharacterAdditionalInfo",
            GcCharacterAdditionalInfo::header(),
        ),
        ("GcMainCharacter3Bgm", GcMainCharacter3Bgm::header()),
        ("GcMainCharacter4BgmVol", GcMainCharacter4BgmVol::header()),
        ("GcCharacterGoldChange", GcCharacterGoldChange::header()),
    ];
    let expected = [
        ("GcCharacterAdd", 1u8),
        ("GcCharacterMove", 3),
        ("GcMainCharacter", 15),
        ("GcCharacterUpdate", 19),
        ("GcTargetInfo", 58),
        ("GcTarget", 63),
        ("GcSkillLevel", 72),
        ("GcMainCharacter2Empire", 113),
        ("GcCharacterUpdate2", 117),
        ("GcTargetCreateNew", 125),
        ("GcCharacterAdditionalInfo", 136),
        ("GcMainCharacter3Bgm", 137),
        ("GcMainCharacter4BgmVol", 138),
        ("GcCharacterGoldChange", 225),
    ];
    assert_eq!(headers.len(), expected.len());
    for ((name, got), (exp_name, want)) in headers.iter().zip(expected.iter()) {
        assert_eq!(name, exp_name);
        assert_eq!(*got, *want, "{name} is not its legacy header byte");
    }
}

/// The measured packed widths, which the C++ compilers produced on the legacy
/// 32-bit target under the active feature-gate set.
#[test]
fn the_gc_actor_wire_sizes_match_the_measured_legacy_widths() {
    use protocol::gc_actors::{
        GC_CHARACTER_ADD_WIRE_SIZE, GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE,
        GC_CHARACTER_MOVE_WIRE_SIZE, GC_CHARACTER_UPDATE2_WIRE_SIZE, GC_CHARACTER_UPDATE_WIRE_SIZE,
        GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE, GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE,
        GC_MAIN_CHARACTER3_BGM_WIRE_SIZE, GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE,
        GC_MAIN_CHARACTER_WIRE_SIZE, GC_SKILL_LEVEL_WIRE_SIZE, GC_TARGET_CREATE_NEW_WIRE_SIZE,
        GC_TARGET_INFO_WIRE_SIZE, GC_TARGET_WIRE_SIZE,
    };
    assert_eq!(GC_CHARACTER_ADD_WIRE_SIZE, 35);
    assert_eq!(GC_CHARACTER_MOVE_WIRE_SIZE, 24);
    assert_eq!(GC_MAIN_CHARACTER_WIRE_SIZE, 45);
    assert_eq!(GC_CHARACTER_UPDATE_WIRE_SIZE, 55);
    assert_eq!(GC_TARGET_INFO_WIRE_SIZE, 19);
    assert_eq!(GC_TARGET_WIRE_SIZE, 32);
    assert_eq!(GC_SKILL_LEVEL_WIRE_SIZE, 256);
    assert_eq!(GC_MAIN_CHARACTER2_EMPIRE_WIRE_SIZE, 46);
    assert_eq!(GC_CHARACTER_UPDATE2_WIRE_SIZE, 44);
    assert_eq!(GC_TARGET_CREATE_NEW_WIRE_SIZE, 43);
    assert_eq!(GC_CHAR_ADDITIONAL_INFO_WIRE_SIZE, 70);
    assert_eq!(GC_MAIN_CHARACTER3_BGM_WIRE_SIZE, 71);
    assert_eq!(GC_MAIN_CHARACTER4_BGM_VOL_WIRE_SIZE, 75);
    assert_eq!(GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE, 24);
}

/// The three distinct legacy name-array lengths must stay separate, because
/// the 33-byte target name shares a wire byte whose enumerator is named
/// differently on each side.
#[test]
fn the_gc_actor_name_array_lengths_stay_distinct() {
    use protocol::gc_actors::{BGM_NAME_LEN, NAME_LEN, TARGET_NAME_LEN};
    assert_eq!(NAME_LEN, 25);
    assert_eq!(BGM_NAME_LEN, 25);
    assert_eq!(TARGET_NAME_LEN, 33);
}

/// The gold-change header is the module's only four-byte header.
#[test]
fn the_gold_change_header_is_four_bytes() {
    use protocol::gc_actors::{GcCharacterGoldChange, GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE};
    assert_eq!(GcCharacterGoldChange::header(), 225);
    assert_eq!(GcCharacterGoldChange::HEADER, 225);
    let record = GcCharacterGoldChange::new(7, -1, 2);
    let mut wire = Vec::new();
    record.encode_into(&mut wire);
    assert_eq!(wire.len(), GC_CHARACTER_GOLD_CHANGE_WIRE_SIZE);
    assert_eq!(&wire[0..4], &[0xe1, 0x00, 0x00, 0x00]);
}

/// Every one of the 14 rows must be recorded as implemented in the inventory.
#[test]
fn the_gc_inventory_records_the_actor_rows_as_implemented() {
    use protocol::gc_inventory::LEGACY_GC_PACKET_INVENTORY;
    const IMPLEMENTED: [&str; 14] = [
        "HEADER_GC_CHARACTER_ADD",
        "HEADER_GC_CHARACTER_MOVE",
        "HEADER_GC_MAIN_CHARACTER",
        "HEADER_GC_CHARACTER_UPDATE",
        "HEADER_GC_TARGET_INFO",
        "HEADER_GC_TARGET",
        "HEADER_GC_SKILL_LEVEL",
        "HEADER_GC_MAIN_CHARACTER2_EMPIRE",
        "HEADER_GC_CHARACTER_UPDATE2",
        "HEADER_GC_TARGET_CREATE_NEW",
        "HEADER_GC_CHAR_ADDITIONAL_INFO",
        "HEADER_GC_MAIN_CHARACTER3_BGM",
        "HEADER_GC_MAIN_CHARACTER4_BGM_VOL",
        "HEADER_GC_CHARACTER_GOLD_CHANGE",
    ];
    for name in IMPLEMENTED {
        let row = LEGACY_GC_PACKET_INVENTORY
            .iter()
            .find(|row| row.client_name == name)
            .unwrap_or_else(|| panic!("{name} is not in the inventory"));
        assert!(row.implemented_in_rust, "{name} is not marked implemented");
    }
}
