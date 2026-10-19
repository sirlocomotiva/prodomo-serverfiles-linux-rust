//! The guild mark manager: `CGuildMarkManager` + `CGuildMarkImage` + `SGuildMarkBlock`
//! (`G/MarkManager.{h,cpp}`, `G/MarkImage.{h,cpp}`), minus the disk: the mark-id
//! allocator, the five 512x512 image slots, the block CRC lists, the LZO-compressed
//! block cache, and the guild symbol map.
//!
//! - **The pixels live in memory, not in TGA files.** Legacy draws each mark into a
//!   512x512 `DevIL` image (`ilSetPixels`) and reads the touched 64x48 block back out
//!   (`ilCopyPixels`) to compress it. The port keeps the same 512x512 `u32` pixels
//!   per image slot and copies the same rectangles itself; `DevIL` and the `mark/`
//!   directory are gone, and the pixel values the CRC and the compressor read are
//!   the same ones.
//! - **The CRC is the game's own table.** `GetCRC32` (`G/crc32.cpp`) is the
//!   byte-at-a-time CRC-32 with the table below, starting and ending at
//!   `0xffffffff`. Both the symbol map and the block cache hash with it.
//! - **The compressor is LZO1X-1.** `SGuildMarkBlock::Compress` calls
//!   `lzo1x_1_compress` on the block's 64x48 pixels. The port vendors that one
//!   algorithm from `G/minilzo.c` below; the 6.5k-line file's other codecs are not
//!   ported because nothing on the mark path calls them.
//! - **The index is memory-only.** Legacy persists `mark/<prefix>_index` and the TGA
//!   files; the port loads the marks from the store at boot (`db::guilds::marks`)
//!   and saves each upload through the descriptor, as guild creation judges, stores,
//!   then commits.
//!
//! The geometry constants are `MarkImage.h`'s: a mark is 16x12, a block holds 4x4
//! marks (64x48), an image holds 8x10 blocks (512x512, 1280 marks), and the manager
//! holds 5 images (6400 mark ids).

/// `SGuildMark::WIDTH` (`G/MarkImage.h`): 16.
pub(super) const MARK_WIDTH: usize = 16;
/// `SGuildMark::HEIGHT`: 12.
pub(super) const MARK_HEIGHT: usize = 12;
/// `SGuildMark::SIZE`: 192 pixels.
pub(super) const MARK_PIXELS: usize = MARK_WIDTH * MARK_HEIGHT;
/// `SGuildMarkBlock::MARK_PER_BLOCK_WIDTH`: 4.
pub(super) const BLOCK_MARKS_WIDE: usize = 4;
/// `SGuildMarkBlock::MARK_PER_BLOCK_HEIGHT`: 4.
pub(super) const BLOCK_MARKS_HIGH: usize = 4;
/// `SGuildMarkBlock::WIDTH`: 64.
pub(super) const BLOCK_WIDTH: usize = MARK_WIDTH * BLOCK_MARKS_WIDE;
/// `SGuildMarkBlock::HEIGHT`: 48.
pub(super) const BLOCK_HEIGHT: usize = MARK_HEIGHT * BLOCK_MARKS_HIGH;
/// `SGuildMarkBlock::SIZE`: 3072 pixels.
pub(super) const BLOCK_PIXELS: usize = BLOCK_WIDTH * BLOCK_HEIGHT;
/// `CGuildMarkImage::WIDTH`/`HEIGHT`: 512.
pub(super) const IMAGE_WIDTH: usize = 512;
pub(super) const IMAGE_HEIGHT: usize = 512;
/// `BLOCK_COL_COUNT`/`BLOCK_ROW_COUNT`: 8x10.
pub(super) const IMAGE_BLOCKS_WIDE: usize = 8;
pub(super) const IMAGE_BLOCKS_HIGH: usize = 10;
/// `BLOCK_TOTAL_COUNT`: 80 blocks. The live diff reads the block slice's own
/// length; the tests pin the width by name.
#[cfg(test)]
pub(super) const IMAGE_BLOCKS: usize = IMAGE_BLOCKS_WIDE * IMAGE_BLOCKS_HIGH;
/// `MARK_COL_COUNT`/`MARK_ROW_COUNT`: 32x40.
pub(super) const IMAGE_MARKS_WIDE: usize = 32;
/// The image height in marks. Only the test-only `empty_position` reads it.
#[cfg(test)]
pub(super) const IMAGE_MARKS_HIGH: usize = 40;
/// The row width as the `u32` position math takes.
#[cfg(test)]
const IMAGE_MARKS_WIDE_U32: u32 = 32;
/// `MARK_TOTAL_COUNT`: 1280 marks per image. The live math reads
/// [`IMAGE_MARKS_U32`]; the tests pin the width by name.
#[cfg(test)]
pub(super) const IMAGE_MARKS: usize = IMAGE_MARKS_WIDE * IMAGE_MARKS_HIGH;
/// The same count as the `u32` mark-id math takes.
const IMAGE_MARKS_U32: u32 = 32 * 40;
/// `MAX_IMAGE_COUNT`: 5 images.
pub(super) const IMAGE_COUNT: usize = 5;
/// The same count as the `u32` image-index math takes.
const IMAGE_COUNT_U32: u32 = 5;
/// `MAX_IMAGE_COUNT * MARK_TOTAL_COUNT`: 6400 mark ids. The live math reads
/// [`MARK_IDS_U32`]; the tests pin the width by name.
#[cfg(test)]
pub(super) const MARK_IDS: usize = IMAGE_COUNT * IMAGE_MARKS;
/// The same count as the `u32` mark-id math takes.
const MARK_IDS_U32: u32 = IMAGE_COUNT_U32 * IMAGE_MARKS_U32;
/// `INVALID_MARK_ID`: no mark.
pub(super) const INVALID_MARK_ID: u32 = 0xffff_ffff;
/// `SGuildMarkBlock::MAX_COMP_SIZE`: 12288 + 768 + 64 + 3 = 13123 bytes.
pub(super) const MAX_COMP_SIZE: usize = BLOCK_PIXELS * 4 + ((BLOCK_PIXELS * 4) >> 4) + 64 + 3;

/// The `CRCTable` of `G/crc32.cpp`: the CRC-32 table `GetCRC32` walks.
const CRC_TABLE: [u32; 256] = [
    0x0000_0000,
    0x7707_3096,
    0xee0e_612c,
    0x9909_51ba,
    0x076d_c419,
    0x706a_f48f,
    0xe963_a535,
    0x9e64_95a3,
    0x0edb_8832,
    0x79dc_b8a4,
    0xe0d5_e91e,
    0x97d2_d988,
    0x09b6_4c2b,
    0x7eb1_7cbd,
    0xe7b8_2d07,
    0x90bf_1d91,
    0x1db7_1064,
    0x6ab0_20f2,
    0xf3b9_7148,
    0x84be_41de,
    0x1ada_d47d,
    0x6ddd_e4eb,
    0xf4d4_b551,
    0x83d3_85c7,
    0x136c_9856,
    0x646b_a8c0,
    0xfd62_f97a,
    0x8a65_c9ec,
    0x1401_5c4f,
    0x6306_6cd9,
    0xfa0f_3d63,
    0x8d08_0df5,
    0x3b6e_20c8,
    0x4c69_105e,
    0xd560_41e4,
    0xa267_7172,
    0x3c03_e4d1,
    0x4b04_d447,
    0xd20d_85fd,
    0xa50a_b56b,
    0x35b5_a8fa,
    0x42b2_986c,
    0xdbbb_c9d6,
    0xacbc_f940,
    0x32d8_6ce3,
    0x45df_5c75,
    0xdcd6_0dcf,
    0xabd1_3d59,
    0x26d9_30ac,
    0x51de_003a,
    0xc8d7_5180,
    0xbfd0_6116,
    0x21b4_f4b5,
    0x56b3_c423,
    0xcfba_9599,
    0xb8bd_a50f,
    0x2802_b89e,
    0x5f05_8808,
    0xc60c_d9b2,
    0xb10b_e924,
    0x2f6f_7c87,
    0x5868_4c11,
    0xc161_1dab,
    0xb666_2d3d,
    0x76dc_4190,
    0x01db_7106,
    0x98d2_20bc,
    0xefd5_102a,
    0x71b1_8589,
    0x06b6_b51f,
    0x9fbf_e4a5,
    0xe8b8_d433,
    0x7807_c9a2,
    0x0f00_f934,
    0x9609_a88e,
    0xe10e_9818,
    0x7f6a_0dbb,
    0x086d_3d2d,
    0x9164_6c97,
    0xe663_5c01,
    0x6b6b_51f4,
    0x1c6c_6162,
    0x8565_30d8,
    0xf262_004e,
    0x6c06_95ed,
    0x1b01_a57b,
    0x8208_f4c1,
    0xf50f_c457,
    0x65b0_d9c6,
    0x12b7_e950,
    0x8bbe_b8ea,
    0xfcb9_887c,
    0x62dd_1ddf,
    0x15da_2d49,
    0x8cd3_7cf3,
    0xfbd4_4c65,
    0x4db2_6158,
    0x3ab5_51ce,
    0xa3bc_0074,
    0xd4bb_30e2,
    0x4adf_a541,
    0x3dd8_95d7,
    0xa4d1_c46d,
    0xd3d6_f4fb,
    0x4369_e96a,
    0x346e_d9fc,
    0xad67_8846,
    0xda60_b8d0,
    0x4404_2d73,
    0x3303_1de5,
    0xaa0a_4c5f,
    0xdd0d_7cc9,
    0x5005_713c,
    0x2702_41aa,
    0xbe0b_1010,
    0xc90c_2086,
    0x5768_b525,
    0x206f_85b3,
    0xb966_d409,
    0xce61_e49f,
    0x5ede_f90e,
    0x29d9_c998,
    0xb0d0_9822,
    0xc7d7_a8b4,
    0x59b3_3d17,
    0x2eb4_0d81,
    0xb7bd_5c3b,
    0xc0ba_6cad,
    0xedb8_8320,
    0x9abf_b3b6,
    0x03b6_e20c,
    0x74b1_d29a,
    0xead5_4739,
    0x9dd2_77af,
    0x04db_2615,
    0x73dc_1683,
    0xe363_0b12,
    0x9464_3b84,
    0x0d6d_6a3e,
    0x7a6a_5aa8,
    0xe40e_cf0b,
    0x9309_ff9d,
    0x0a00_ae27,
    0x7d07_9eb1,
    0xf00f_9344,
    0x8708_a3d2,
    0x1e01_f268,
    0x6906_c2fe,
    0xf762_575d,
    0x8065_67cb,
    0x196c_3671,
    0x6e6b_06e7,
    0xfed4_1b76,
    0x89d3_2be0,
    0x10da_7a5a,
    0x67dd_4acc,
    0xf9b9_df6f,
    0x8ebe_eff9,
    0x17b7_be43,
    0x60b0_8ed5,
    0xd6d6_a3e8,
    0xa1d1_937e,
    0x38d8_c2c4,
    0x4fdf_f252,
    0xd1bb_67f1,
    0xa6bc_5767,
    0x3fb5_06dd,
    0x48b2_364b,
    0xd80d_2bda,
    0xaf0a_1b4c,
    0x3603_4af6,
    0x4104_7a60,
    0xdf60_efc3,
    0xa867_df55,
    0x316e_8eef,
    0x4669_be79,
    0xcb61_b38c,
    0xbc66_831a,
    0x256f_d2a0,
    0x5268_e236,
    0xcc0c_7795,
    0xbb0b_4703,
    0x2202_16b9,
    0x5505_262f,
    0xc5ba_3bbe,
    0xb2bd_0b28,
    0x2bb4_5a92,
    0x5cb3_6a04,
    0xc2d7_ffa7,
    0xb5d0_cf31,
    0x2cd9_9e8b,
    0x5bde_ae1d,
    0x9b64_c2b0,
    0xec63_f226,
    0x756a_a39c,
    0x026d_930a,
    0x9c09_06a9,
    0xeb0e_363f,
    0x7207_6785,
    0x0500_5713,
    0x95bf_4a82,
    0xe2b8_7a14,
    0x7bb1_2bae,
    0x0cb6_1b38,
    0x92d2_8e9b,
    0xe5d5_be0d,
    0x7cdc_efb7,
    0x0bdb_df21,
    0x86d3_d2d4,
    0xf1d4_e242,
    0x68dd_b3f8,
    0x1fda_836e,
    0x81be_16cd,
    0xf6b9_265b,
    0x6fb0_77e1,
    0x18b7_4777,
    0x8808_5ae6,
    0xff0f_6a70,
    0x6606_3bca,
    0x1101_0b5c,
    0x8f65_9eff,
    0xf862_ae69,
    0x616b_ffd3,
    0x166c_cf45,
    0xa00a_e278,
    0xd70d_d2ee,
    0x4e04_8354,
    0x3903_b3c2,
    0xa767_2661,
    0xd060_16f7,
    0x4969_474d,
    0x3e6e_77db,
    0xaed1_6a4a,
    0xd9d6_5adc,
    0x40df_0b66,
    0x37d8_3bf0,
    0xa9bc_ae53,
    0xdebb_9ec5,
    0x47b2_cf7f,
    0x30b5_ffe9,
    0xbdbd_f21c,
    0xcaba_c28a,
    0x53b3_9330,
    0x24b4_a3a6,
    0xbad0_3605,
    0xcdd7_0693,
    0x54de_5729,
    0x23d9_67bf,
    0xb366_7a2e,
    0xc461_4ab8,
    0x5d68_1b02,
    0x2a6f_2b94,
    0xb40b_be37,
    0xc30c_8ea1,
    0x5a05_df1b,
    0x2d02_ef8d,
];

/// `GetCRC32` (`G/crc32.cpp`): the byte-at-a-time CRC-32 over `bytes`.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for byte in bytes {
        crc = CRC_TABLE[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_is_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(&[0_u8; 768]), 0xf5a0_f415);
        let bytes: Vec<u8> = (0..3).flat_map(|_| 0..=255_u8).collect();
        assert_eq!(crc32(&bytes), 0xb0c0_df2a);
    }

    #[test]
    fn the_lzo_matches_the_c_probe_on_zeros() {
        let input = [0_u8; 12_288];
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(&input, &mut out);
        assert_eq!(len, 76);
        let head: Vec<u8> = (0..32).map(|i| out[i]).collect();
        assert_eq!(
            head,
            vec![
                0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
            ]
        );
        assert_eq!(&out[len - 3..len], &[0x11, 0x00, 0x00]);
    }

    #[test]
    fn the_lzo_matches_the_c_probe_on_bytes() {
        let mut input = [0_u8; 12_288];
        for (i, slot) in input.iter_mut().enumerate() {
            *slot = u8::try_from(i % 256)
                .unwrap_or(0)
                .wrapping_mul(31)
                .wrapping_add(7);
        }
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(&input, &mut out);
        assert_eq!(len, 332);
        assert_eq!(&out[..8], &[0x00, 0xf3, 0x07, 0x26, 0x45, 0x64, 0x83, 0xa2]);
        assert_eq!(&out[len - 3..len], &[0x11, 0x00, 0x00]);
    }

    #[test]
    fn the_lzo_matches_the_c_probe_on_sixteen_marks() {
        let mut input = [0_u8; 12_288];
        for (m, chunk) in input.chunks_mut(768).enumerate() {
            chunk.fill(u8::try_from(m + 1).unwrap_or(u8::MAX));
        }
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(&input, &mut out);
        assert_eq!(len, 141);
        assert_eq!(
            &out[..16],
            &[
                0x02, 0x01, 0x01, 0x01, 0x01, 0x01, 0x20, 0x00, 0x00, 0xdc, 0x11, 0x00, 0x02, 0x20,
                0x00, 0x00,
            ]
        );
        assert_eq!(&out[len - 3..len], &[0x11, 0x00, 0x00]);
    }

    /// The probe's random input, byte for byte: glibc `rand()` seeded 12345, low
    /// byte each (`RAND_MAX` 2^31-1, nonlinear generator — not reproducible from
    /// the linear formula, hence the fixture). The test pins the compressed
    /// length and head/tail of the fixed array.
    #[test]
    fn the_lzo_matches_the_c_probe_on_random_bytes() {
        let input: &[u8; 12_288] = include_bytes!("guild_mark_rand.bin");
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(input, &mut out);
        assert_eq!(len, 12_341);
        assert_eq!(&out[..12], &[0; 12]);
        assert_eq!(&out[len - 4..len], &[0x9e, 0x11, 0x00, 0x00]);
    }

    #[test]
    fn the_lzo_matches_the_c_probe_on_periodic_bytes() {
        let mut input = [0_u8; 12_288];
        for (b, chunk) in input.chunks_mut(1024).enumerate() {
            assert!(b < 12);
            for (i, slot) in chunk.iter_mut().enumerate() {
                *slot = u8::try_from(i & 0xff).unwrap_or(0);
            }
        }
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(&input, &mut out);
        assert_eq!(len, 332);
        assert_eq!(
            &out[..12],
            &[0x00, 0xf3, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09]
        );
        assert_eq!(&out[len - 4..len], &[0xff, 0x11, 0x00, 0x00]);
    }

    #[test]
    fn the_lzo_matches_the_c_probe_on_sparse_bytes() {
        let mut input = [0_u8; 12_288];
        input[100] = 7;
        input[5000] = 9;
        input[12_000] = 13;
        let mut out = [0_u8; MAX_COMP_SIZE];
        let len = lzo1x_1_compress(&input, &mut out);
        assert_eq!(len, 101);
        assert_eq!(
            &out[..12],
            &[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x3e, 0x11, 0x00, 0x07, 0x20]
        );
        assert_eq!(&out[len - 4..len], &[0x00, 0x11, 0x00, 0x00]);
    }

    #[test]
    fn the_blank_image_matches_build_all_blocks() {
        // Every block of a blank image compresses the same 12288 zero bytes the
        // zeros probe pins: 76 bytes, same head and terminator.
        let image = MarkImage::blank();
        assert_eq!(image.blocks.len(), IMAGE_BLOCKS);
        for block in &image.blocks {
            assert_eq!(block.comp_len, 76);
            assert_eq!(&block.comp[..6], &[0x02, 0x00, 0x00, 0x00, 0x00, 0x00]);
            assert_eq!(&block.comp[73..76], &[0x11, 0x00, 0x00]);
        }
        // ... and the same CRC the sparse probe's zero stretches hash to.
        let zero_crc = crc32(&[0_u8; BLOCK_PIXELS * 4]);
        assert!(image.blocks.iter().all(|b| b.crc == zero_crc));
        assert_eq!(image.empty_position(), 0);
        assert_eq!(image.crc_list(), [zero_crc; IMAGE_BLOCKS]);
    }

    #[test]
    fn the_mark_lands_in_its_block() {
        let mut image = MarkImage::blank();
        let zero_crc = crc32(&[0_u8; BLOCK_PIXELS * 4]);
        // Mark 33 = row 1, col 1: the second block (block row 0, col 0 holds marks
        // 0..3 x rows 0..3, so mark 33 sits in block col 0, row 0 still? row 1 / 4
        // = 0, col 1 / 4 = 0: block 0).
        let mut mark = [0_u32; MARK_PIXELS];
        mark[0] = 0xff11_2233;
        assert!(image.save_mark(33, &mark));
        assert_ne!(image.blocks[0].crc, zero_crc);
        assert!(image.blocks[1..].iter().all(|b| b.crc == zero_crc));
        assert_eq!(image.empty_position(), 0);
        // Mark 0 dirties the same block; deleting both restores it.
        assert!(image.save_mark(0, &mark));
        assert_eq!(image.empty_position(), 1);
        assert!(image.delete_mark(33));
        assert!(image.delete_mark(0));
        assert_eq!(image.blocks[0].crc, zero_crc);
        assert_eq!(image.empty_position(), 0);
        // Past the 1280th the position is refused.
        assert!(!image.save_mark(IMAGE_MARKS_U32, &mark));
        assert!(!image.delete_mark(IMAGE_MARKS_U32));
    }

    #[test]
    fn the_manager_allocates_lowest_first() {
        let mut manager = MarkManager::new();
        assert_eq!(manager.mark_id(7), INVALID_MARK_ID);
        assert_eq!(manager.image_count(), 0);
        assert_eq!(manager.mark_count(), 0);
        let mark = [1_u32; MARK_PIXELS];
        // First upload allocates id 0 and touches image 0 only.
        assert_eq!(manager.save_mark(7, &mark), 0);
        assert_eq!(manager.mark_id(7), 0);
        assert_eq!(manager.image_count(), 1);
        assert_eq!(manager.mark_count(), 1);
        // Re-upload keeps the id.
        assert_eq!(manager.save_mark(7, &mark), 0);
        assert_eq!(manager.mark_count(), 1);
        // The next guild takes id 1.
        assert_eq!(manager.save_mark(8, &mark), 1);
        // Deleting frees the id for the next guild.
        manager.delete_mark(7);
        assert_eq!(manager.mark_id(7), INVALID_MARK_ID);
        assert_eq!(manager.save_mark(9, &mark), 0);
        // Deleting an unknown guild is a no-op.
        manager.delete_mark(77);
        assert_eq!(manager.mark_count(), 2);
        // Ids past 6399 are refused.
        assert!(!manager.add_mark_id(10, MARK_IDS_U32));
        // The diff of a fresh client (all-zero CRCs) is every block of image 0.
        let diff = manager.diff_blocks(0, &[0_u32; IMAGE_BLOCKS]);
        assert_eq!(diff.len(), IMAGE_BLOCKS);
        assert_eq!(diff[0].0, 0);
        assert_eq!(
            diff[0].1.len(),
            manager.images[0].as_ref().unwrap().blocks[0].comp_len
        );
        // ... and nothing once the client caught up.
        let crcs = manager.crc_list(0).unwrap();
        assert!(manager.diff_blocks(0, &crcs).is_empty());
        // Untouched and past-the-fifth slots yield nothing.
        assert!(manager.diff_blocks(1, &[0_u32; IMAGE_BLOCKS]).is_empty());
        assert!(manager
            .diff_blocks(IMAGE_COUNT_U32, &[0_u32; IMAGE_BLOCKS])
            .is_empty());
        assert!(manager.crc_list(1).is_none());
        assert!(manager.crc_list(IMAGE_COUNT_U32).is_none());
    }

    #[test]
    fn the_geometry_matches_the_headers() {
        assert_eq!(MARK_PIXELS, 192);
        assert_eq!((BLOCK_WIDTH, BLOCK_HEIGHT), (64, 48));
        assert_eq!(BLOCK_PIXELS, 3072);
        assert_eq!(IMAGE_BLOCKS, 80);
        assert_eq!(IMAGE_MARKS, 1280);
        assert_eq!(MARK_IDS, 6400);
        assert_eq!(MAX_COMP_SIZE, 13_123);
    }
}

// ---------------------------------------------------------------------------
// LZO1X-1 compression: `lzo1x_1_compress` + `do_compress` (`G/minilzo.c:4986-5362`).
//
// Only the byte-observable behaviour is ported: the dictionary hash, the match
// search, the literal and match encodings, and the tail. The port is an index port,
// not a pointer port: `ip`, `ii`, `op`, `m_pos` are `usize` offsets into `input`
// and `output`, and the dictionary holds input offsets the same way the C
// dictionary holds `DENTRY(p, in)` = `pd(p, in)` when `LZO_DICT_USE_PTR` is 0
// (`minilzo.c:4680`, `:4918`).
//
// Build flags, resolved for the game target the same file compiles under:
// - `LZO1X` is 1 (`:4701`), so the M2 arm writes the `LZO1X` form (`:5234`).
// - `LZO_DETERMINISTIC` is undefined (`:4710`), so the search is the non-deterministic
//   two-index arm (`:5011-5042`), with the work memory zeroed by the caller
//   (`LZOManager`'s constructor, `lzo_manager.cpp:5-13`) — not per block.
// - `LZO_DICT_USE_PTR` is 0 (`:4680`), so the dictionary is `lzo_uint16_t`
//   (`:4681`) and `GINDEX` reads an offset (`:4918`).
// - `D_BITS` is 14 (`:4689`), so the dictionary has 16384 entries and `D_MASK` is
//   `0x3fff`, `D_HIGH` is `0x2000`.
// - `LZO_OPT_UNALIGNED32` is 1 on x86, but every unaligned fast path copies or
//   compares the same bytes the scalar path does; the port takes the scalar path
//   everywhere, which is behaviour-identical on little-endian bytes.
// - `LZO_CHECK_MPOS_NON_DET` with offsets (`:4946-4949`) is `in_off(ip) <= dict_off
//   || { m_off = in_off(ip) - dict_off; m_off > max || { m_pos = ip - m_off } }`.
//   The `m_pos = ip - ...` assignment inside the macro both positions the match and
//   yields false; the port assigns `m_pos` and continues the same tests.
//
// `pd(a, b)` (`:4143`) is `a - b` as an unsigned value.

/// `D_BITS` (`G/minilzo.c:4689`): the dictionary is `1 << 14` offsets.
const LZO_DICT_BITS: usize = 14;
/// The dictionary entries: 16384.
const LZO_DICT_SIZE: usize = 1 << LZO_DICT_BITS;
/// `D_MASK`: `0x3fff`.
const LZO_DICT_MASK: usize = LZO_DICT_SIZE - 1;
/// `M2_MAX_OFFSET` (`:4715`): `0x0800`.
const LZO_M2_MAX_OFFSET: usize = 0x0800;
/// `M3_MAX_OFFSET` (`:4717`): `0x4000`.
const LZO_M3_MAX_OFFSET: usize = 0x4000;
/// `M2_MAX_LEN` (`:4726`): 8.
const LZO_M2_MAX_LEN: usize = 8;
/// `M3_MAX_LEN` (`:4729`): 33.
const LZO_M3_MAX_LEN: usize = 33;
/// `M4_MAX_LEN` (`:4731`): 9.
const LZO_M4_MAX_LEN: usize = 9;
/// `M3_MARKER` (`:4735`): 32.
const LZO_M3_MARKER: u8 = 32;
/// `M4_MARKER` (`:4736`): 16.
const LZO_M4_MARKER: u8 = 16;

/// `DINDEX` (`:4692`): `DM(DMUL(0x1824429d, dv) >> (32 - D_BITS))`, where `dv` is
/// the little-endian u32 at `ip` and `DMUL` multiplies 32-bit (`:3540`).
fn lzo_dindex(word: u32) -> usize {
    let product = 0x1824_429d_u32.wrapping_mul(word);
    ((product >> (32 - LZO_DICT_BITS)) as usize) & LZO_DICT_MASK
}

/// `do_compress`'s match extension (`:5151-5194`, the `LZO_OPT_UNALIGNED32` arm the
/// x86 game target compiles): from length 4, compare 4-byte words while whole words
/// agree and the end has not been reached, then count the agreeing prefix bytes of
/// the first disagreeing word. The word-at-a-time walk and the prefix count find the
/// same length a byte walk would; what matters is the stopping rule: the loop only
/// checks `ip + m_len >= ip_end` on the *second* word of each pair, and the prefix
/// count runs past `ip_end` without a bound check.
fn lzo_match_len(input: &[u8], ip: usize, m_pos: usize, ip_end: usize) -> usize {
    let word =
        |at: usize| u32::from_ne_bytes([input[at], input[at + 1], input[at + 2], input[at + 3]]);
    let mut len = 4;
    let mut v = word(ip + len) ^ word(m_pos + len);
    if v == 0 {
        loop {
            len += 4;
            v = word(ip + len) ^ word(m_pos + len);
            if v != 0 {
                break;
            }
            len += 4;
            v = word(ip + len) ^ word(m_pos + len);
            if ip + len >= ip_end {
                // `goto m_len_done`: the length stands, the last word uncounted.
                return len;
            }
            if v != 0 {
                break;
            }
        }
    }
    // Little-endian prefix count (`:5184-5190`): trailing zero bytes.
    len + (v.trailing_zeros() / 8) as usize
}

/// The match the search loop hands the emitter: the length and the offset.
#[derive(Clone, Copy)]
struct LzoMatch {
    /// `m_len`: the match length.
    len: usize,
    /// `m_off`: the match offset.
    off: usize,
}

/// Emit one match (`:5126-5297`): the M2, M3, or M4 form of `m`. Returns the new
/// `op`.
fn lzo_emit_match(out: &mut [u8], mut op: usize, m: LzoMatch) -> usize {
    let mut len = m.len;
    let mut off = m.off;
    if len <= LZO_M2_MAX_LEN && off <= LZO_M2_MAX_OFFSET {
        off -= 1;
        out[op] = u8::try_from(((len - 1) << 5) | ((off & 7) << 2)).unwrap_or(u8::MAX);
        op += 1;
        out[op] = u8::try_from(off >> 3).unwrap_or(u8::MAX);
        op += 1;
    } else if off <= LZO_M3_MAX_OFFSET {
        off -= 1;
        if len <= LZO_M3_MAX_LEN {
            out[op] = LZO_M3_MARKER | u8::try_from(len - 2).unwrap_or(u8::MAX);
            op += 1;
        } else {
            len -= LZO_M3_MAX_LEN;
            out[op] = LZO_M3_MARKER;
            op += 1;
            while len > 255 {
                len -= 255;
                out[op] = 0;
                op += 1;
            }
            out[op] = u8::try_from(len).unwrap_or(u8::MAX);
            op += 1;
        }
        out[op] = u8::try_from((off & 0x3f) << 2).unwrap_or(u8::MAX);
        op += 1;
        out[op] = u8::try_from((off >> 6) & 0xff).unwrap_or(u8::MAX);
        op += 1;
    } else {
        off -= 0x4000;
        if len <= LZO_M4_MAX_LEN {
            out[op] = LZO_M4_MARKER
                | u8::try_from((off >> 11) & 8).unwrap_or(u8::MAX)
                | u8::try_from(len - 2).unwrap_or(u8::MAX);
            op += 1;
        } else {
            len -= LZO_M4_MAX_LEN;
            out[op] = LZO_M4_MARKER | u8::try_from((off >> 11) & 8).unwrap_or(u8::MAX);
            op += 1;
            while len > 255 {
                len -= 255;
                out[op] = 0;
                op += 1;
            }
            out[op] = u8::try_from(len).unwrap_or(u8::MAX);
            op += 1;
        }
        out[op] = u8::try_from((off & 0x3f) << 2).unwrap_or(u8::MAX);
        op += 1;
        out[op] = u8::try_from((off >> 6) & 0xff).unwrap_or(u8::MAX);
        op += 1;
    }
    op
}

/// One `do_compress` call (`:4989-5298`): compress `input[..len]` into `out` from
/// `op`, with carry `ti` and the zeroed-at-construction dictionary. Returns the
/// trailing literal count `t` (`pd(in_end, ii - ti)`), and `op` advances past the
/// bytes written.
fn lzo_compress_core(
    input: &[u8],
    len: usize,
    out: &mut [u8],
    mut op: usize,
    mut ti: usize,
    dict: &mut [u32],
) -> (usize, usize) {
    let total = len;
    let ip_end = len.saturating_sub(20);
    let mut ip = 0_usize;
    let mut ii = 0_usize;
    // `ip += ti < 4 ? 4 - ti : 0` (`:5006`).
    ip += if ti < 4 { 4 - ti } else { 0 };
    // The deterministic search (`:5046-5063`): the loop *enters* at `literal:`, so
    // the first position is skipped before anything is hashed — the search starts
    // at `4 + 1 + ((4 - 0) >> 5)` = 5 for a fresh chunk. After a match the code
    // jumps to `next:` (`:5293`), bypassing the skip; only a word mismatch takes
    // the `literal:` skip. Each `next:` reads the little-endian word, hashes it,
    // reads `m_pos` from the slot (`GINDEX(m_off, m_pos, in+dict, ...)` with the
    // offset dictionary is `m_pos = in + dict[dindex]`, i.e. offset
    // `dict[dindex]`), stores the current position, and compares the words.
    ip += 1 + ((ip - ii) >> 5);
    loop {
        let mut m_pos;
        let mut dindex;
        loop {
            if ip >= ip_end {
                let t = total.wrapping_sub(ii.wrapping_sub(ti));
                return (t, op);
            }
            let word = u32::from_le_bytes([input[ip], input[ip + 1], input[ip + 2], input[ip + 3]]);
            dindex = lzo_dindex(word);
            m_pos = dict[dindex] as usize;
            dict[dindex] = u32::try_from(ip).unwrap_or(u32::MAX);
            let at_pos = u32::from_le_bytes([
                input[m_pos],
                input[m_pos + 1],
                input[m_pos + 2],
                input[m_pos + 3],
            ]);
            if word != at_pos {
                // `literal:` (`:5052-5053`).
                ip += 1 + ((ip - ii) >> 5);
                continue;
            }
            break;
        }
        // `try_match:` the 4-byte head compare (`:5036-5055`).
        if input[m_pos] != input[ip]
            || input[m_pos + 1] != input[ip + 1]
            || input[m_pos + 2] != input[ip + 2]
            || input[m_pos + 3] != input[ip + 3]
        {
            dict[dindex] = u32::try_from(ip).unwrap_or(u32::MAX);
            ip += 1 + ((ip - ii) >> 5);
            continue;
        }
        dict[dindex] = u32::try_from(ip).unwrap_or(u32::MAX);
        ii = ii.wrapping_sub(ti);
        ti = 0;
        // The pending literals (`:5058-5124`).
        let t = ip.wrapping_sub(ii);
        if t != 0 {
            if t <= 3 {
                out[op - 2] |= u8::try_from(t).unwrap_or(u8::MAX);
                for _ in 0..t {
                    out[op] = input[ii];
                    op += 1;
                    ii += 1;
                }
            } else {
                if t <= 18 {
                    out[op] = u8::try_from(t - 3).unwrap_or(u8::MAX);
                    op += 1;
                } else {
                    let mut tt = t - 18;
                    out[op] = 0;
                    op += 1;
                    while tt > 255 {
                        tt -= 255;
                        out[op] = 0;
                        op += 1;
                    }
                    out[op] = u8::try_from(tt).unwrap_or(u8::MAX);
                    op += 1;
                }
                for _ in 0..t {
                    out[op] = input[ii];
                    op += 1;
                    ii += 1;
                }
            }
        }
        let m = LzoMatch {
            len: lzo_match_len(input, ip, m_pos, ip_end),
            off: ip - m_pos,
        };
        ip += m.len;
        ii = ip;
        op = lzo_emit_match(out, op, m);
    }
}

/// `lzo1x_1_compress` (`:5301-5362`): compress `input` into `out`, which must hold
/// [`MAX_COMP_SIZE`] bytes. Returns the compressed length. The dictionary starts
/// zeroed, as `LZOManager`'s constructor zeroes it (`lzo_manager.cpp:5-13`).
///
/// Only inputs of at most 49152 bytes take the chunk loop's shape; a mark block is
/// 12288 bytes, so the loop runs once. The `ll_end` overflow guard cannot trip for
/// such an input (the sums stay far below `usize::MAX`), and the deterministic
/// `memset` is compiled out (`LZO_DETERMINISTIC` is undefined).
fn lzo1x_1_compress(input: &[u8], out: &mut [u8]) -> usize {
    let mut dict = vec![0_u32; LZO_DICT_SIZE].into_boxed_slice();
    let in_len = input.len();
    let mut op = 0_usize;
    let mut t = 0_usize;
    // The `while (l > 20)` chunk loop (`:5313-5333`): one pass for a block.
    if in_len > 20 {
        let (tt, next_op) = lzo_compress_core(input, in_len, out, op, t, &mut dict);
        t = tt;
        op = next_op;
    } else {
        t += in_len;
    }
    // The tail literals (`:5336-5360`).
    if t > 0 {
        let ii = in_len - t;
        if op == 0 && t <= 238 {
            out[op] = u8::try_from(17 + t).unwrap_or(u8::MAX);
            op += 1;
        } else if t <= 3 {
            out[op - 2] |= u8::try_from(t).unwrap_or(u8::MAX);
        } else if t <= 18 {
            out[op] = u8::try_from(t - 3).unwrap_or(u8::MAX);
            op += 1;
        } else {
            let mut tt = t - 18;
            out[op] = 0;
            op += 1;
            while tt > 255 {
                tt -= 255;
                out[op] = 0;
                op += 1;
            }
            out[op] = u8::try_from(tt).unwrap_or(u8::MAX);
            op += 1;
        }
        for i in 0..t {
            out[op] = input[ii + i];
            op += 1;
        }
    }
    out[op] = LZO_M4_MARKER | 1;
    op += 1;
    out[op] = 0;
    op += 1;
    out[op] = 0;
    op += 1;
    op
}

/// One `SGuildMarkBlock`: the compressed bytes, their length, and the CRC of the
/// 64x48 raw pixels they compress (`G/MarkImage.h`). A fresh block is the
/// compression of nothing — but legacy never reads a block before either
/// `BuildAllBlocks` or `SaveMark` writes it, and the port compresses every block
/// at boot the same way.
#[derive(Debug, Clone)]
pub(super) struct MarkBlock {
    /// `m_abCompBuf` + `m_sizeCompBuf`: the LZO bytes and their used length.
    /// Boxed: 80 of these per image at 13KB each would put 1MB on the stack.
    pub(super) comp: Box<[u8; MAX_COMP_SIZE]>,
    /// The used prefix of [`comp`](Self::comp).
    pub(super) comp_len: usize,
    /// `m_crc`: the CRC of the raw 64x48 pixels.
    pub(super) crc: u32,
}

impl MarkBlock {
    /// `SGuildMarkBlock::Compress` (`G/MarkImage.cpp`): compress `pixels` (64x48
    /// native-endian words, read as bytes) and hash the raw bytes.
    fn compress(pixels: &[u32; BLOCK_PIXELS]) -> Self {
        let mut comp = Box::new([0_u8; MAX_COMP_SIZE]);
        // `Pixel` is `unsigned long`, 4 bytes on the 32-bit game target; the
        // compressor reads the words' bytes in memory order, i.e. little-endian.
        let mut raw = [0_u8; BLOCK_PIXELS * 4];
        for (i, word) in pixels.iter().enumerate() {
            raw[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        let comp_len = lzo1x_1_compress(&raw, comp.as_mut());
        let crc = crc32(&raw);
        Self {
            comp,
            comp_len,
            crc,
        }
    }
}

/// One `CGuildMarkImage`: five of these live in the manager. The 512x512 pixels
/// are what `DevIL` held (`ilSetPixels`/`ilCopyPixels`); the 80 blocks are the
/// compressed cache `BuildAllBlocks` fills and `SaveMark` refreshes one of.
#[derive(Debug, Clone)]
pub(super) struct MarkImage {
    /// The 512x512 BGRA pixels, row-major. Boxed: 1MB of stack per image would
    /// overflow the test runner's 8MB stack the moment a `MarkImage` is moved.
    pixels: Box<[u32; IMAGE_WIDTH * IMAGE_HEIGHT]>,
    /// `m_aakBlock[row][col]`: the compressed cache, row-major over the 8x10
    /// block grid.
    blocks: Vec<MarkBlock>,
}

impl MarkImage {
    /// A black image with every block compressed (`Build` + `BuildAllBlocks`):
    /// what `__GetImage` leaves behind for a missing TGA.
    fn blank() -> Self {
        let pixels: Box<[u32; IMAGE_WIDTH * IMAGE_HEIGHT]> =
            vec![0_u32; IMAGE_WIDTH * IMAGE_HEIGHT]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| panic!("a {}-word vec converts", IMAGE_WIDTH * IMAGE_HEIGHT));
        let mut image = Self {
            pixels,
            blocks: Vec::new(),
        };
        image.compress_all();
        image
    }

    /// `BuildAllBlocks`: compress every 64x48 tile of the pixels.
    fn compress_all(&mut self) {
        self.blocks.clear();
        for row in 0..IMAGE_BLOCKS_HIGH {
            for col in 0..IMAGE_BLOCKS_WIDE {
                self.blocks
                    .push(MarkBlock::compress(&self.block_pixels(row, col)));
            }
        }
    }

    /// The 64x48 pixels at block (`row`, `col`), row-major.
    fn block_pixels(&self, row: usize, col: usize) -> [u32; BLOCK_PIXELS] {
        let mut tile = [0_u32; BLOCK_PIXELS];
        for y in 0..BLOCK_HEIGHT {
            let src = (row * BLOCK_HEIGHT + y) * IMAGE_WIDTH + col * BLOCK_WIDTH;
            let dst = y * BLOCK_WIDTH;
            tile[dst..dst + BLOCK_WIDTH].copy_from_slice(&self.pixels[src..src + BLOCK_WIDTH]);
        }
        tile
    }

    /// `SaveMark` (`G/MarkImage.cpp`): draw the 16x12 `mark` at mark position
    /// `pos`, then recompress the block that holds it. Positions past the 1280th
    /// are refused, as legacy refuses them.
    fn save_mark(&mut self, pos: u32, mark: &[u32; MARK_PIXELS]) -> bool {
        if pos >= IMAGE_MARKS_U32 {
            return false;
        }
        let col = (pos as usize) % IMAGE_MARKS_WIDE;
        let row = (pos as usize) / IMAGE_MARKS_WIDE;
        // `PutData(col * 16, row * 12, 16, 12, ...)`.
        for y in 0..MARK_HEIGHT {
            let dst = (row * MARK_HEIGHT + y) * IMAGE_WIDTH + col * MARK_WIDTH;
            let src = y * MARK_WIDTH;
            self.pixels[dst..dst + MARK_WIDTH].copy_from_slice(&mark[src..src + MARK_WIDTH]);
        }
        // `GetData` the touched block back out and `Compress` it.
        let block_row = row / BLOCK_MARKS_HIGH;
        let block_col = col / BLOCK_MARKS_WIDE;
        let tile = self.block_pixels(block_row, block_col);
        self.blocks[block_row * IMAGE_BLOCKS_WIDE + block_col] = MarkBlock::compress(&tile);
        true
    }

    /// `DeleteMark`: draw a zeroed mark (not `Clear`'s `0xff000000` — `DeleteMark`
    /// zeroes the image, while `Clear` is dead on the server path).
    fn delete_mark(&mut self, pos: u32) -> bool {
        self.save_mark(pos, &[0_u32; MARK_PIXELS])
    }

    /// `GetEmptyPosition`: the first mark position whose 192 pixels are all zero,
    /// or `INVALID_MARK_POSITION` (`0xffffffff`) when the image is full. No server
    /// path calls it (the allocator is the manager's free-id set); the tests pin it.
    #[cfg(test)]
    fn empty_position(&self) -> u32 {
        for row in 0..u32::try_from(IMAGE_MARKS_HIGH).unwrap_or(u32::MAX) {
            for col in 0..u32::try_from(IMAGE_MARKS_WIDE).unwrap_or(u32::MAX) {
                let mut empty = true;
                for y in 0..MARK_HEIGHT {
                    let base =
                        (row as usize * MARK_HEIGHT + y) * IMAGE_WIDTH + col as usize * MARK_WIDTH;
                    if self.pixels[base..base + MARK_WIDTH].iter().any(|p| *p != 0) {
                        empty = false;
                        break;
                    }
                }
                if empty {
                    return row * IMAGE_MARKS_WIDE_U32 + col;
                }
            }
        }
        0xffff_ffff
    }

    /// `GetBlockCRCList`: the 80 block CRCs in row-major block order. `CLIENT`-only
    /// in legacy (`MarkManager.h`); the tests pin it.
    #[cfg(test)]
    fn crc_list(&self) -> [u32; IMAGE_BLOCKS] {
        let mut list = [0_u32; IMAGE_BLOCKS];
        for (i, block) in self.blocks.iter().enumerate() {
            list[i] = block.crc;
        }
        list
    }
}

/// One `TGuildSymbol` (`G/MarkManager.h`): the CRC and the raw bytes.
#[derive(Debug, Clone, Default)]
pub(super) struct GuildSymbol {
    /// `crc`: the CRC of the stored bytes.
    pub(super) crc: u32,
    /// `raw`: the stored bytes.
    pub(super) raw: Vec<u8>,
}

/// `UploadSymbol` (`G/MarkManager.cpp:376-393`): store `data` under `guild_id`,
/// replacing any earlier bytes, and hash what was stored. Empty data clears the
/// bytes but keeps the row, as the map insert does.
pub(super) fn upload_symbol(
    symbols: &mut std::collections::BTreeMap<u32, GuildSymbol>,
    guild_id: u32,
    data: &[u8],
) {
    let symbol = symbols.entry(guild_id).or_default();
    symbol.raw.clear();
    if !data.is_empty() {
        symbol.raw.extend_from_slice(data);
        symbol.crc = crc32(data);
    }
}

/// `GetGuildSymbol` (`G/MarkManager.cpp:312-320`): the symbol of `guild_id`, if any.
pub(super) fn guild_symbol(
    symbols: &std::collections::BTreeMap<u32, GuildSymbol>,
    guild_id: u32,
) -> Option<&GuildSymbol> {
    symbols.get(&guild_id)
}

/// The mark-id allocator and image slots: `CGuildMarkManager`'s mark half
/// (`G/MarkManager.{h,cpp}`). The guild-id map and the free-id set are what the
/// `mark/<prefix>_index` file persisted; the images are what the TGA files held.
/// Both now live in memory only: the store seeds them at boot and each upload
/// commits through the descriptor.
#[derive(Debug, Clone)]
pub(super) struct MarkManager {
    /// `m_mapIdx_Image`: the live image slots by image index. Legacy inserts the
    /// slot on first touch (`__GetImage`) and never removes it; the port does the
    /// same (`Vec<Option<_>>` keeps the index stable).
    images: Vec<Option<MarkImage>>,
    /// `m_mapGID_MarkID`: the mark id per guild.
    guild_mark: std::collections::BTreeMap<u32, u32>,
    /// `m_setFreeMarkID`: the free ids, lowest first.
    free_ids: std::collections::BTreeSet<u32>,
}

impl Default for MarkManager {
    /// The constructor: every id from 0 to 6399 free, no image touched yet.
    fn default() -> Self {
        Self::new()
    }
}

impl MarkManager {
    /// The constructor: every id from 0 to 6399 free, no image touched yet.
    pub(super) fn new() -> Self {
        Self {
            images: (0..IMAGE_COUNT).map(|_| None).collect(),
            guild_mark: std::collections::BTreeMap::new(),
            free_ids: (0..MARK_IDS_U32).collect(),
        }
    }

    /// `AddMarkIDByGuildID`: record `mark_id` for `guild_id`. Ids past 6399 are
    /// refused; a duplicate guild row overwrites nothing (`insert` on the map
    /// keeps the first, as `std::map::insert` does).
    pub(super) fn add_mark_id(&mut self, guild_id: u32, mark_id: u32) -> bool {
        if mark_id >= MARK_IDS_U32 {
            return false;
        }
        if self.guild_mark.contains_key(&guild_id) {
            self.free_ids.remove(&mark_id);
            return true;
        }
        self.guild_mark.insert(guild_id, mark_id);
        self.free_ids.remove(&mark_id);
        true
    }

    /// `GetMarkID`: the mark id of `guild_id`, or [`INVALID_MARK_ID`].
    pub(super) fn mark_id(&self, guild_id: u32) -> u32 {
        self.guild_mark
            .get(&guild_id)
            .copied()
            .unwrap_or(INVALID_MARK_ID)
    }

    /// `__GetImage`: the image slot, touched into existence (blank = built + all
    /// blocks compressed) on first use. Indexes past the fifth return `None`, as
    /// `GetMarkImageFilename` fails them.
    fn image(&mut self, img_idx: u32) -> Option<&mut MarkImage> {
        if img_idx >= IMAGE_COUNT_U32 {
            return None;
        }
        let slot = &mut self.images[img_idx as usize];
        if slot.is_none() {
            *slot = Some(MarkImage::blank());
        }
        slot.as_mut()
    }

    /// `__AllocMarkID`: the lowest free id, with its image touched. Fails when no
    /// id is free or the guild row cannot be recorded.
    fn alloc_mark_id(&mut self, guild_id: u32) -> u32 {
        let Some(mark_id) = self.free_ids.iter().next().copied() else {
            return INVALID_MARK_ID;
        };
        let img_idx = mark_id / IMAGE_MARKS_U32;
        if self.image(img_idx).is_none() || !self.add_mark_id(guild_id, mark_id) {
            return INVALID_MARK_ID;
        }
        mark_id
    }

    /// Draw `pixels` at the mark position `mark_id` names, touching its image: the
    /// boot half of `LoadMarkImages` (`G/MarkManager.cpp:96-111`), which draws each
    /// TGA's tiles. The id must already be recorded; unrecorded ids draw nothing, as
    /// the TGA read draws only indexed tiles.
    pub(super) fn draw_mark(&mut self, mark_id: u32, pixels: &[u32; MARK_PIXELS]) {
        if mark_id >= MARK_IDS_U32 {
            return;
        }
        let img_idx = mark_id / IMAGE_MARKS_U32;
        if let Some(image) = self.image(img_idx) {
            image.save_mark(mark_id % IMAGE_MARKS_U32, pixels);
        }
    }

    /// `SaveMark`: keep the guild's id or allocate one, draw the mark, and return
    /// the id. The caller persists the upload; the TGA and index writes are gone
    /// (see the module doc). Fails with [`INVALID_MARK_ID`] when no id is free —
    /// legacy returns `false` (0) there, which collides with mark id 0; the port
    /// returns the invalid id instead, since no caller can use id 0 from a failure.
    pub(super) fn save_mark(&mut self, guild_id: u32, mark: &[u32; MARK_PIXELS]) -> u32 {
        let mut mark_id = self.mark_id(guild_id);
        if mark_id == INVALID_MARK_ID {
            mark_id = self.alloc_mark_id(guild_id);
            if mark_id == INVALID_MARK_ID {
                return INVALID_MARK_ID;
            }
        }
        let img_idx = mark_id / IMAGE_MARKS_U32;
        if let Some(image) = self.image(img_idx) {
            image.save_mark(mark_id % IMAGE_MARKS_U32, mark);
        }
        mark_id
    }

    /// `DeleteMark`: zero the guild's mark and free its id. Unknown guilds are a
    /// no-op, as legacy's early return is.
    pub(super) fn delete_mark(&mut self, guild_id: u32) {
        let Some(mark_id) = self.guild_mark.remove(&guild_id) else {
            return;
        };
        let img_idx = mark_id / IMAGE_MARKS_U32;
        if let Some(image) = self.image(img_idx) {
            image.delete_mark(mark_id % IMAGE_MARKS_U32);
        }
        self.free_ids.insert(mark_id);
    }

    /// `GetDiffBlocks`: the blocks whose CRC differs from the client's list, in
    /// row-major block order with the block position as key. Unknown image slots
    /// yield nothing, as legacy's early return does. The caller supplies the
    /// client's 80 CRCs; shorter lists read as zero past their end, since legacy
    /// walks a raw pointer and the client always sends 80.
    pub(super) fn diff_blocks(&mut self, img_idx: u32, crc_list: &[u32]) -> Vec<(u8, &[u8])> {
        if img_idx >= IMAGE_COUNT_U32 || self.images[img_idx as usize].is_none() {
            return Vec::new();
        }
        let image = self.images[img_idx as usize]
            .as_ref()
            .expect("checked live");
        let mut diff = Vec::new();
        for (pos, block) in image.blocks.iter().enumerate() {
            if block.crc != crc_list.get(pos).copied().unwrap_or(0) {
                diff.push((
                    u8::try_from(pos).unwrap_or(u8::MAX),
                    &block.comp[..block.comp_len],
                ));
            }
        }
        diff
    }

    /// `CopyMarkIdx`: the (guild, mark) pairs in guild-id order, each truncated to its
    /// low 16 bits as the `WORD` write does. The caller passes guild ids above 65535
    /// through knowingly: the truncation is the record's, not the read's.
    pub(super) fn idx_pairs(&self) -> Vec<(u16, u16)> {
        self.guild_mark
            .iter()
            .map(|(&guild_id, &mark_id)| {
                // The `WORD` write drops the high bits by record design; the mask
                // keeps the low 16 without a truncating cast.
                (
                    u16::try_from(guild_id & 0xffff).unwrap_or(u16::MAX),
                    u16::try_from(mark_id & 0xffff).unwrap_or(u16::MAX),
                )
            })
            .collect()
    }

    /// `GetBlockCRCList`: the 80 block CRCs of a live image slot, or `None` for an
    /// untouched or past-the-fifth slot, as legacy's `false` is. `CLIENT`-only in
    /// legacy; the tests pin it.
    #[cfg(test)]
    pub(super) fn crc_list(&self, img_idx: u32) -> Option<[u32; IMAGE_BLOCKS]> {
        if img_idx >= IMAGE_COUNT_U32 {
            return None;
        }
        self.images[img_idx as usize]
            .as_ref()
            .map(MarkImage::crc_list)
    }

    /// `GetMarkImageCount` / `GetMarkCount`: the live image slots and the guild rows.
    /// The index-list arm reads the pairs, not the counts; the tests pin these.
    #[cfg(test)]
    pub(super) fn image_count(&self) -> usize {
        self.images.iter().filter(|slot| slot.is_some()).count()
    }

    /// The rows `image_count` pairs with.
    #[cfg(test)]
    pub(super) fn mark_count(&self) -> usize {
        self.guild_mark.len()
    }
}
