//! In-memory integration coverage for the legacy DB peer wire path.

use protocol::db_boot::{
    parse_db_boot_payload, BootFeatureProfile, DbBootRequest, DB_BOOT_RESPONSE_HANDLE,
    HEADER_DG_BOOT, HEADER_GD_BOOT,
};
use protocol::db_records::{
    decode_quest_load, encode_quest_load, ChannelChangeRequest, ChannelResultRecord,
    HorseNameRecord, HorseNameRequest, LoginAccountRecord, LoginByKeyRequest, PlayerLoadRequest,
    PlayerResultRecord, QuestRecord, HEADER_DG_ACK_HORSE_NAME, HEADER_DG_CHANNEL_RESULT,
    HEADER_DG_LOGIN_SUCCESS, HEADER_DG_PLAYER_LOAD_FAILED, HEADER_DG_PLAYER_LOAD_SUCCESS,
    HEADER_DG_QUEST_LOAD, HEADER_GD_FIND_CHANNEL, HEADER_GD_LOGIN_BY_KEY, HEADER_GD_PLAYER_LOAD,
    HEADER_GD_REQ_HORSE_NAME,
};
use protocol::db_wire::{DbFrame, DbFrameDecoder};

const LOGIN_HANDLE: u32 = 0x1020_3040;
const PLAYER_HANDLE: u32 = 0xa1b2_c3d4;
const QUEST_EMPTY_HANDLE: u32 = 0x5566_7788;
const QUEST_ONE_HANDLE: u32 = 0x89ab_cdef;

fn c_array<const N: usize>(value: &str) -> [u8; N] {
    let mut bytes = [0; N];
    bytes[..value.len()].copy_from_slice(value.as_bytes());
    bytes
}

fn assert_legacy_header(encoded: &[u8], header: u8, handle: u32, payload_length: usize) {
    let payload_length = u32::try_from(payload_length).unwrap();
    let mut expected = Vec::with_capacity(9);
    expected.push(header);
    expected.extend_from_slice(&handle.to_le_bytes());
    expected.extend_from_slice(&payload_length.to_le_bytes());
    assert_eq!(&encoded[..9], expected);
}

fn minimal_boot_payload() -> Vec<u8> {
    let profile = BootFeatureProfile::minimal();
    let mut body = vec![6_u8];
    for _ in profile.section_kinds() {
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
    }
    body.extend_from_slice(&0_i32.to_le_bytes());
    body.extend_from_slice(&12_u16.to_le_bytes());
    body.extend_from_slice(&1_u16.to_le_bytes());
    body.extend_from_slice(&[0_u8; 24]);
    body.extend_from_slice(&16_u16.to_le_bytes());
    body.extend_from_slice(&0_u16.to_le_bytes());
    body.extend_from_slice(&104_u16.to_le_bytes());
    body.extend_from_slice(&0_u16.to_le_bytes());
    body.extend_from_slice(&304_u16.to_le_bytes());
    body.extend_from_slice(&1_u16.to_le_bytes());
    body.extend_from_slice(&[0_u8; 304]);
    body.extend_from_slice(&68_u16.to_le_bytes());
    body.extend_from_slice(&0_u16.to_le_bytes());
    body.extend_from_slice(&0xffff_u16.to_le_bytes());

    let mut payload = Vec::with_capacity(body.len() + 4);
    payload.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_le_bytes());
    payload.extend_from_slice(&body);
    payload
}

#[test]
fn login_and_player_requests_round_trip_with_preserved_handles() {
    let login = LoginByKeyRequest {
        login: c_array("alice"),
        login_key: 0x1020_3041,
        client_key: [1, 0x0102_0304, 0x0506_0708, 0x090a_0b0c],
        ip: c_array("127.0.0.1"),
    };
    let login_payload = login.encode();
    assert_eq!(login_payload.len(), 67);

    let login_wire = DbFrame::new(HEADER_GD_LOGIN_BY_KEY, LOGIN_HANDLE, login_payload.clone())
        .encode()
        .unwrap();
    assert_legacy_header(
        &login_wire,
        HEADER_GD_LOGIN_BY_KEY,
        LOGIN_HANDLE,
        login_payload.len(),
    );

    let load = PlayerLoadRequest {
        account_id: 0x0102_0304,
        player_id: 0x1112_1314,
        account_index: 2,
    };
    let load_payload = load.encode();
    assert_eq!(load_payload.len(), 9);

    let load_wire = DbFrame::new(HEADER_GD_PLAYER_LOAD, PLAYER_HANDLE, load_payload.clone())
        .encode()
        .unwrap();
    assert_legacy_header(
        &load_wire,
        HEADER_GD_PLAYER_LOAD,
        PLAYER_HANDLE,
        load_payload.len(),
    );

    let mut request_stream = login_wire.clone();
    request_stream.extend_from_slice(&load_wire);

    // Split inside both the peer header and the first payload. The final
    // fragment also coalesces the complete player-load request.
    let first_frame_complete = login_wire.len();
    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&request_stream[..5]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder
        .feed(&request_stream[5..first_frame_complete - 1])
        .unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder
        .feed(&request_stream[first_frame_complete - 1..])
        .unwrap();

    let decoded_login = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_login.header, HEADER_GD_LOGIN_BY_KEY);
    assert_eq!(decoded_login.handle, LOGIN_HANDLE);
    assert_eq!(
        LoginByKeyRequest::decode(&decoded_login.payload).unwrap(),
        login
    );

    let decoded_load = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_load.header, HEADER_GD_PLAYER_LOAD);
    assert_eq!(decoded_load.handle, PLAYER_HANDLE);
    assert_eq!(
        PlayerLoadRequest::decode(&decoded_load.payload).unwrap(),
        load
    );
    assert!(decoder.try_decode().unwrap().is_none());
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn horse_name_request_and_ack_round_trip_through_the_legacy_headers() {
    let request = HorseNameRequest::new(0x1020_3040);
    let request_payload = request.encode();
    let request_wire = DbFrame::new(
        HEADER_GD_REQ_HORSE_NAME,
        0x5566_7788,
        request_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_legacy_header(
        &request_wire,
        HEADER_GD_REQ_HORSE_NAME,
        0x5566_7788,
        request_payload.len(),
    );

    let response = HorseNameRecord::missing(request.player_id);
    let response_payload = response.encode();
    let response_wire = DbFrame::new(HEADER_DG_ACK_HORSE_NAME, 0, response_payload.clone())
        .encode()
        .unwrap();
    assert_eq!(response_wire.len(), 9 + 29);
    assert_legacy_header(
        &response_wire,
        HEADER_DG_ACK_HORSE_NAME,
        0,
        response_payload.len(),
    );

    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&request_wire[..5]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&request_wire[5..]).unwrap();
    decoder.feed(&response_wire).unwrap();
    let decoded_request = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_request.header, HEADER_GD_REQ_HORSE_NAME);
    assert_eq!(decoded_request.handle, 0x5566_7788);
    assert_eq!(
        HorseNameRequest::decode(&decoded_request.payload).unwrap(),
        request
    );

    let decoded_response = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_response.header, HEADER_DG_ACK_HORSE_NAME);
    assert_eq!(decoded_response.handle, 0);
    assert_eq!(
        HorseNameRecord::decode(&decoded_response.payload).unwrap(),
        response
    );
    assert!(decoder.try_decode().unwrap().is_none());
}

#[test]
fn channel_request_and_result_round_trip_through_legacy_headers() {
    let request = ChannelChangeRequest::new(0x0102_0304, 2);
    let request_payload = request.encode();
    let request_wire = DbFrame::new(HEADER_GD_FIND_CHANNEL, 0x1234_5678, request_payload.clone())
        .encode()
        .unwrap();
    assert_eq!(request_wire.len(), 17);
    assert_legacy_header(
        &request_wire,
        HEADER_GD_FIND_CHANNEL,
        0x1234_5678,
        request_payload.len(),
    );

    let result = ChannelResultRecord::new(0x0100_007f, 0x1234);
    let result_payload = result.encode();
    let result_wire = DbFrame::new(
        HEADER_DG_CHANNEL_RESULT,
        0x1234_5678,
        result_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_eq!(result_wire.len(), 15);
    assert_legacy_header(
        &result_wire,
        HEADER_DG_CHANNEL_RESULT,
        0x1234_5678,
        result_payload.len(),
    );

    let mut stream = request_wire;
    stream.extend_from_slice(&result_wire);
    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&stream[..4]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&stream[4..]).unwrap();

    let decoded_request = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_request.header, HEADER_GD_FIND_CHANNEL);
    assert_eq!(decoded_request.handle, 0x1234_5678);
    assert_eq!(
        ChannelChangeRequest::decode(&decoded_request.payload).unwrap(),
        request
    );

    let decoded_result = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_result.header, HEADER_DG_CHANNEL_RESULT);
    assert_eq!(decoded_result.handle, 0x1234_5678);
    assert_eq!(
        ChannelResultRecord::decode(&decoded_result.payload).unwrap(),
        result
    );
    assert!(decoder.try_decode().unwrap().is_none());
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn account_and_player_responses_round_trip_when_coalesced() {
    let account = LoginAccountRecord {
        id: 0x0102_0304,
        login: c_array("alice"),
        ..LoginAccountRecord::default()
    };
    let account_payload = account.encode();
    assert_eq!(account_payload.len(), 362);
    let account_wire = DbFrame::new(
        HEADER_DG_LOGIN_SUCCESS,
        LOGIN_HANDLE,
        account_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_legacy_header(
        &account_wire,
        HEADER_DG_LOGIN_SUCCESS,
        LOGIN_HANDLE,
        account_payload.len(),
    );

    let player = PlayerResultRecord {
        id: 0x1112_1314,
        name: c_array("hero"),
        gold: 0x0102_0304_0506_0708,
        ..PlayerResultRecord::default()
    };
    let player_payload = player.encode();
    assert_eq!(player_payload.len(), 2007);
    let player_wire = DbFrame::new(
        HEADER_DG_PLAYER_LOAD_SUCCESS,
        PLAYER_HANDLE,
        player_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_legacy_header(
        &player_wire,
        HEADER_DG_PLAYER_LOAD_SUCCESS,
        PLAYER_HANDLE,
        player_payload.len(),
    );

    let failed_wire = DbFrame::new(HEADER_DG_PLAYER_LOAD_FAILED, PLAYER_HANDLE, Vec::new())
        .encode()
        .unwrap();
    assert_legacy_header(&failed_wire, HEADER_DG_PLAYER_LOAD_FAILED, PLAYER_HANDLE, 0);

    let mut response_stream = account_wire;
    response_stream.extend_from_slice(&player_wire);
    response_stream.extend_from_slice(&failed_wire);
    let before_last_byte = response_stream.len() - 1;

    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&response_stream[..4]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&response_stream[4..before_last_byte]).unwrap();

    let decoded_account = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_account.header, HEADER_DG_LOGIN_SUCCESS);
    assert_eq!(decoded_account.handle, LOGIN_HANDLE);
    assert_eq!(
        LoginAccountRecord::decode(&decoded_account.payload).unwrap(),
        account
    );

    let decoded_player = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_player.header, HEADER_DG_PLAYER_LOAD_SUCCESS);
    assert_eq!(decoded_player.handle, PLAYER_HANDLE);
    assert_eq!(
        PlayerResultRecord::decode(&decoded_player.payload).unwrap(),
        player
    );
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&response_stream[before_last_byte..]).unwrap();
    let decoded_failed = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_failed.header, HEADER_DG_PLAYER_LOAD_FAILED);
    assert_eq!(decoded_failed.handle, PLAYER_HANDLE);
    assert!(decoded_failed.payload.is_empty());
    assert!(decoder.try_decode().unwrap().is_none());
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn zero_and_one_row_quest_responses_round_trip_when_coalesced() {
    let empty_quest_payload = encode_quest_load(&[]).unwrap();
    assert_eq!(empty_quest_payload, [0, 0, 0, 0]);
    let empty_quest_wire = DbFrame::new(
        HEADER_DG_QUEST_LOAD,
        QUEST_EMPTY_HANDLE,
        empty_quest_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_legacy_header(
        &empty_quest_wire,
        HEADER_DG_QUEST_LOAD,
        QUEST_EMPTY_HANDLE,
        empty_quest_payload.len(),
    );

    let quest = QuestRecord {
        pid: 0x0102_0304,
        name: c_array("first_quest"),
        state: c_array("state_a"),
        value: -1234,
    };
    let one_quest_payload = encode_quest_load(std::slice::from_ref(&quest)).unwrap();
    assert_eq!(one_quest_payload.len(), 110);
    assert_eq!(&one_quest_payload[..4], &1_u32.to_le_bytes());
    let one_quest_wire = DbFrame::new(
        HEADER_DG_QUEST_LOAD,
        QUEST_ONE_HANDLE,
        one_quest_payload.clone(),
    )
    .encode()
    .unwrap();
    assert_legacy_header(
        &one_quest_wire,
        HEADER_DG_QUEST_LOAD,
        QUEST_ONE_HANDLE,
        one_quest_payload.len(),
    );

    let mut response_stream = empty_quest_wire;
    response_stream.extend_from_slice(&one_quest_wire);
    let before_last_byte = response_stream.len() - 1;
    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&response_stream[..4]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&response_stream[4..before_last_byte]).unwrap();

    let decoded_empty_quest = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_empty_quest.header, HEADER_DG_QUEST_LOAD);
    assert_eq!(decoded_empty_quest.handle, QUEST_EMPTY_HANDLE);
    assert!(decode_quest_load(&decoded_empty_quest.payload)
        .unwrap()
        .is_empty());
    assert!(decoder.try_decode().unwrap().is_none());

    decoder.feed(&response_stream[before_last_byte..]).unwrap();
    let decoded_one_quest = decoder.try_decode().unwrap().unwrap();
    assert_eq!(decoded_one_quest.header, HEADER_DG_QUEST_LOAD);
    assert_eq!(decoded_one_quest.handle, QUEST_ONE_HANDLE);
    assert_eq!(decoded_one_quest.payload, one_quest_payload);
    assert_eq!(
        decode_quest_load(&decoded_one_quest.payload).unwrap(),
        vec![quest]
    );
    assert!(decoder.try_decode().unwrap().is_none());
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn boot_response_frame_preserves_inner_and_outer_lengths() {
    let profile = BootFeatureProfile::minimal();
    let payload = minimal_boot_payload();
    let parsed = parse_db_boot_payload(&payload, profile).unwrap();
    let frame = parsed.encode_frame(profile).unwrap();
    assert_eq!(frame.header, HEADER_DG_BOOT);
    assert_eq!(frame.handle, DB_BOOT_RESPONSE_HANDLE);
    assert_eq!(frame.payload, payload);

    let wire = frame.encode().unwrap();
    assert_legacy_header(&wire, HEADER_DG_BOOT, 0, payload.len());
    assert_eq!(
        usize::try_from(u32::from_le_bytes(wire[5..9].try_into().unwrap())).unwrap(),
        payload.len()
    );
    assert_eq!(
        usize::try_from(u32::from_le_bytes(payload[..4].try_into().unwrap())).unwrap(),
        payload.len()
    );

    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&wire[..4]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&wire[4..]).unwrap();
    let received_frame = decoder.try_decode().unwrap().unwrap();
    assert_eq!(received_frame, frame);
    assert_eq!(
        parse_db_boot_payload(&received_frame.payload, profile).unwrap(),
        parsed
    );
    assert!(decoder.try_decode().unwrap().is_none());
}

#[test]
fn boot_request_round_trips_through_the_legacy_db_peer_header() {
    let mut ip = [0_u8; 16];
    ip[..9].copy_from_slice(b"127.0.0.1");
    let request = DbBootRequest::new([0x0102_0304, 0x0506_0708], ip);
    let payload = request.encode();
    let wire = DbFrame::new(HEADER_GD_BOOT, 0, payload.to_vec())
        .encode()
        .unwrap();

    assert_legacy_header(&wire, HEADER_GD_BOOT, 0, payload.len());
    let mut decoder = DbFrameDecoder::new();
    decoder.feed(&wire[..7]).unwrap();
    assert!(decoder.try_decode().unwrap().is_none());
    decoder.feed(&wire[7..]).unwrap();
    let frame = decoder.try_decode().unwrap().unwrap();

    assert_eq!(frame.header, HEADER_GD_BOOT);
    assert_eq!(frame.handle, 0);
    assert_eq!(DbBootRequest::decode(&frame.payload).unwrap(), request);
    assert!(decoder.try_decode().unwrap().is_none());
}
