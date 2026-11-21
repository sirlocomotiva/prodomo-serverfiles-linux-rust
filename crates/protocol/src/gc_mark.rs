//! The `HEADER_GC_MARK_BLOCK` (100), `HEADER_GC_MARK_IDXLIST` (102) and
//! `HEADER_GC_SYMBOL_DATA` (133) records: the mark index list, the compressed mark
//! blocks, and the guild symbol bytes
//! (`TPacketGCMarkIDXList`/`TPacketGCMarkBlock`, `packet.h:2255-2270`, and
//! `TPacketGCGuildSymbolData`, `packet.h:2289-2294`).
//!
//! All three are dynamic: each carries a `bufSize`/`size` that counts the header
//! itself. `TPacketGCMarkIDXList` is 7 bytes — header, DWORD `bufSize`, WORD
//! `count` — and the body is `count` `(WORD guild_id, WORD mark_id)` pairs, which
//! `CGuildMarkManager::CopyMarkIdx` writes in guild-id order (`MarkManager.cpp:203`).
//! `TPacketGCMarkBlock` is 10 bytes — header, DWORD `bufSize`, BYTE `imgIdx`, DWORD
//! `count` — and the body is `count` `(BYTE posBlock, DWORD compSize, compSize bytes)`
//! triples, in ascending `posBlock` order because the diff map is a
//! `std::map<BYTE, ...>` (`input_login.cpp:1129-1143`). `TPacketGCGuildSymbolData`
//! is 7 bytes — header, WORD `size`, DWORD `guild_id` — and the body is the stored
//! symbol bytes (`input_login.cpp:1041-1052`).
//!
//! None of the three is in the client's `PythonNetworkStream` decode table, so
//! none is in [`crate::gc_inventory`]: the table holds 134 rows and none is a
//! mark byte. The mark downloader decodes them on its own path. They are grouped
//! with the mark family by phase anyway: all three are sent by the login-phase
//! arms of `CInputLogin::Analyze` (`input_login.cpp:1214-1234`) to a client that
//! has no character yet.

/// Wire byte of `HEADER_GC_MARK_BLOCK` (100).
pub const HEADER_GC_MARK_BLOCK: u8 = 100;
/// Wire byte of `HEADER_GC_MARK_IDXLIST` (102).
pub const HEADER_GC_MARK_IDXLIST: u8 = 102;
/// Wire byte of `HEADER_GC_SYMBOL_DATA` (133).
pub const HEADER_GC_SYMBOL_DATA: u8 = 133;

/// The header of `TPacketGCMarkIDXList`: header + `bufSize` + `count` (7 bytes).
pub const GC_MARK_IDXLIST_HEAD_SIZE: usize = 7;
/// The header of `TPacketGCMarkBlock`: header + `bufSize` + `imgIdx` + `count` (10
/// bytes).
pub const GC_MARK_BLOCK_HEAD_SIZE: usize = 10;
/// The header of `TPacketGCGuildSymbolData`: header + `size` + `guild_id` (7 bytes).
pub const GC_SYMBOL_DATA_HEAD_SIZE: usize = 7;
/// One index-list pair: a WORD guild id and a WORD mark id (4 bytes).
pub const GC_MARK_IDX_PAIR_SIZE: usize = 4;
/// One block triple's lead: the BYTE position and the DWORD compressed size (5
/// bytes), before the compressed bytes themselves.
pub const GC_MARK_BLOCK_LEAD_SIZE: usize = 5;

/// Encode `TPacketGCMarkIDXList` plus `pairs` guild/mark pairs, in the order given.
///
/// `bufSize` counts the 7 head bytes and the body. The caller passes the pairs in
/// guild-id order, as `CopyMarkIdx` writes them; the encoder does not sort.
#[must_use]
pub fn mark_idxlist_record(pairs: &[(u16, u16)]) -> Vec<u8> {
    let body = pairs.len() * GC_MARK_IDX_PAIR_SIZE;
    let buf_size = u32::try_from(GC_MARK_IDXLIST_HEAD_SIZE + body).unwrap_or(u32::MAX);
    let count = u16::try_from(pairs.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(GC_MARK_IDXLIST_HEAD_SIZE + body);
    out.push(HEADER_GC_MARK_IDXLIST);
    out.extend_from_slice(&buf_size.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for (guild_id, mark_id) in pairs {
        out.extend_from_slice(&guild_id.to_le_bytes());
        out.extend_from_slice(&mark_id.to_le_bytes());
    }
    out
}

/// Encode `TPacketGCMarkBlock` plus `blocks` diff triples, in the order given.
///
/// Each triple is the block position and its compressed bytes. `bufSize` counts the
/// 10 head bytes and the body. The caller passes the blocks in ascending position
/// order, as the diff map's iteration does; the encoder does not sort.
#[must_use]
pub fn mark_block_record(img_idx: u8, blocks: &[(u8, &[u8])]) -> Vec<u8> {
    let body: usize = blocks
        .iter()
        .map(|(_, bytes)| GC_MARK_BLOCK_LEAD_SIZE + bytes.len())
        .sum();
    let buf_size = u32::try_from(GC_MARK_BLOCK_HEAD_SIZE + body).unwrap_or(u32::MAX);
    let count = u32::try_from(blocks.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(GC_MARK_BLOCK_HEAD_SIZE + body);
    out.push(HEADER_GC_MARK_BLOCK);
    out.extend_from_slice(&buf_size.to_le_bytes());
    out.push(img_idx);
    out.extend_from_slice(&count.to_le_bytes());
    for (pos, bytes) in blocks {
        out.push(*pos);
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// Encode `TPacketGCGuildSymbolData` plus the stored `symbol` bytes.
///
/// `size` is a WORD that counts the 7 head bytes and the body.
#[must_use]
pub fn symbol_data_record(guild_id: u32, symbol: &[u8]) -> Vec<u8> {
    let size = u16::try_from(GC_SYMBOL_DATA_HEAD_SIZE + symbol.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(GC_SYMBOL_DATA_HEAD_SIZE + symbol.len());
    out.push(HEADER_GC_SYMBOL_DATA);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&guild_id.to_le_bytes());
    out.extend_from_slice(symbol);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idxlist_is_pinned_byte_for_byte() {
        assert_eq!(mark_idxlist_record(&[]), vec![102, 7, 0, 0, 0, 0, 0]);
        assert_eq!(
            mark_idxlist_record(&[(3, 0), (7, 1)]),
            vec![102, 15, 0, 0, 0, 2, 0, 3, 0, 0, 0, 7, 0, 1, 0]
        );
    }

    #[test]
    fn the_block_is_pinned_byte_for_byte() {
        assert_eq!(
            mark_block_record(0, &[]),
            vec![100, 10, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            mark_block_record(2, &[(5, &[0x11, 0x22])]),
            vec![100, 17, 0, 0, 0, 2, 1, 0, 0, 0, 5, 2, 0, 0, 0, 0x11, 0x22]
        );
    }

    #[test]
    fn the_symbol_data_is_pinned_byte_for_byte() {
        assert_eq!(
            symbol_data_record(9, &[0xde, 0xad]),
            vec![133, 9, 0, 9, 0, 0, 0, 0xde, 0xad]
        );
    }
}
