//! LZO1X block decompression, as `lzo1x_decompress_safe` decodes a block.
//!
//! Legacy reads each sectree of a map's `server_attr` through `LZOManager::Decompress`
//! (`server/server/game/lzo_manager.cpp:32-39`), which calls the LZO library's
//! `lzo1x_decompress_safe`. This module is written from the LZO1X stream format; it holds no code
//! from the library.
//!
//! A stream is a sequence of instructions, each a literal run or a back-reference into the output
//! already written:
//!
//! - A first byte above 17 is a literal run of `byte - 17` bytes.
//! - At an instruction boundary, a byte below 16 is a literal run of `byte + 3` bytes; zero
//!   extends the length by 255 for each further zero byte, then by 15 plus the next byte.
//! - A byte of 64 or more is a match of 3 to 8 bytes at most 2048 bytes back.
//! - A byte of 32 to 63 is a match at most 16384 bytes back; a byte of 16 to 31 reaches up to
//!   49151 bytes back, and when its distance field is 0 it ends the stream.
//! - A byte below 16 straight after a literal run is a 3-byte match 2049 to 3072 bytes back;
//!   after a match it is a 2-byte match at most 1024 bytes back.
//! - The low two bits of a match's distance field count the 0 to 3 literals that follow it.
//!
//! A match copies byte by byte, so it may overlap the bytes it writes. The safe decoder checks,
//! before each step, that the input holds the bytes the step reads plus the lookahead the next
//! instruction needs, that the output has room, and that a match reaches no further back than
//! the output written; the checks run in the library's order, so a broken stream fails with the
//! library's [`LzoFault`].

use std::error::Error;
use std::fmt;

/// Why a block failed to decompress, as the library's `LZO_E_*` codes name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LzoFault {
    /// `LZO_E_INPUT_OVERRUN`: the stream reads past its end.
    InputOverrun,
    /// `LZO_E_OUTPUT_OVERRUN`: the output would pass the capacity.
    OutputOverrun,
    /// `LZO_E_LOOKBEHIND_OVERRUN`: a match reaches before the start of the output.
    LookbehindOverrun,
    /// `LZO_E_INPUT_NOT_CONSUMED`: bytes follow the end-of-stream marker.
    InputNotConsumed,
}

impl LzoFault {
    /// The library's `LZO_E_*` value.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            Self::InputOverrun => -4,
            Self::OutputOverrun => -5,
            Self::LookbehindOverrun => -6,
            Self::InputNotConsumed => -8,
        }
    }
}

/// A block that failed to decompress, and the bytes written before it failed (legacy's
/// `*out_len`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LzoError {
    /// The fault.
    pub fault: LzoFault,
    /// The output length when the decoder stopped.
    pub written: usize,
}

impl fmt::Display for LzoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.fault {
            LzoFault::InputOverrun => "the stream ends early",
            LzoFault::OutputOverrun => "the output is too large",
            LzoFault::LookbehindOverrun => "a match reaches before the output",
            LzoFault::InputNotConsumed => "bytes follow the end of the stream",
        };
        write!(f, "{what} (after {} bytes out)", self.written)
    }
}

impl Error for LzoError {}

/// Decompress one LZO1X block into at most `capacity` bytes.
///
/// # Errors
///
/// The [`LzoFault`] the library's safe decoder returns for the same stream, with the output
/// length it had reached.
pub fn decompress(input: &[u8], capacity: usize) -> Result<Vec<u8>, LzoError> {
    let mut decoder = Decoder {
        input,
        at: 0,
        out: Vec::new(),
        capacity,
    };
    match decoder.run() {
        Ok(()) => Ok(decoder.out),
        Err(fault) => Err(LzoError {
            fault,
            written: decoder.out.len(),
        }),
    }
}

/// Where the decoder goes next: the library's labels.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// The top of the outer loop: an instruction boundary after a match with no trailing
    /// literals.
    Instruction,
    /// `first_literal_run`: the instruction straight after a literal run.
    AfterLiterals,
    /// `match`: an instruction byte of 16 or more, or a 2-byte match after a match.
    Match(usize),
    /// `match_next`: 1 to 3 trailing literals, then an instruction.
    Trailing(usize),
    /// `eof_found`.
    End,
}

/// The largest distance a byte-below-16 match after a literal run adds (`M2_MAX_OFFSET`).
const M2_MAX_OFFSET: usize = 0x0800;

/// The distance a 16-to-31 match adds to its field, when it is not the end marker.
const M4_BASE: usize = 0x4000;

struct Decoder<'a> {
    input: &'a [u8],
    at: usize,
    out: Vec<u8>,
    capacity: usize,
}

impl Decoder<'_> {
    fn run(&mut self) -> Result<(), LzoFault> {
        let mut step = self.first()?;
        loop {
            step = match step {
                Step::Instruction => self.instruction()?,
                Step::AfterLiterals => self.after_literals()?,
                Step::Match(t) => self.matched(t)?,
                Step::Trailing(t) => self.trailing(t)?,
                Step::End => break,
            };
        }
        if self.at == self.input.len() {
            Ok(())
        } else {
            Err(LzoFault::InputNotConsumed)
        }
    }

    /// The first byte: above 17 it is a literal run of `byte - 17`, taken as trailing literals
    /// when shorter than 4.
    fn first(&mut self) -> Result<Step, LzoFault> {
        self.need_input(1)?;
        let byte = usize::from(self.input[self.at]);
        if byte <= 17 {
            return Ok(Step::Instruction);
        }
        self.at += 1;
        let t = byte - 17;
        if t < 4 {
            return Ok(Step::Trailing(t));
        }
        self.need_output(t)?;
        self.need_input(t + 3)?;
        self.literals(t);
        Ok(Step::AfterLiterals)
    }

    /// An instruction at the top of the outer loop: a match, or a literal run.
    fn instruction(&mut self) -> Result<Step, LzoFault> {
        self.need_input(3)?;
        let mut t = self.byte()?;
        if t >= 16 {
            return Ok(Step::Match(t));
        }
        if t == 0 {
            t = self.extended(15)?;
        }
        self.need_output(t + 3)?;
        self.need_input(t + 6)?;
        self.literals(t + 3);
        Ok(Step::AfterLiterals)
    }

    /// The instruction after a literal run: a byte below 16 is a 3-byte match 2049 to 3072
    /// bytes back.
    fn after_literals(&mut self) -> Result<Step, LzoFault> {
        let t = self.byte()?;
        if t >= 16 {
            return Ok(Step::Match(t));
        }
        let distance = 1 + M2_MAX_OFFSET + (t >> 2) + (self.byte()? << 2);
        self.lookbehind(distance)?;
        self.need_output(3)?;
        self.copy(distance, 3);
        Ok(self.match_done())
    }

    /// `match`: decode one back-reference and copy it.
    fn matched(&mut self, t: usize) -> Result<Step, LzoFault> {
        let (distance, length) = if t >= 64 {
            let distance = 1 + ((t >> 2) & 7) + (self.byte()? << 3);
            (distance, (t >> 5) + 1)
        } else if t >= 32 {
            let mut length = t & 31;
            if length == 0 {
                length = self.extended(31)?;
                self.need_input(2)?;
            }
            (1 + (self.le16()? >> 2), length + 2)
        } else if t >= 16 {
            let high = (t & 8) << 11;
            let mut length = t & 7;
            if length == 0 {
                length = self.extended(7)?;
                self.need_input(2)?;
            }
            let field = high + (self.le16()? >> 2);
            if field == 0 {
                return Ok(Step::End);
            }
            (field + M4_BASE, length + 2)
        } else {
            let distance = 1 + (t >> 2) + (self.byte()? << 2);
            self.lookbehind(distance)?;
            self.need_output(2)?;
            self.copy(distance, 2);
            return Ok(self.match_done());
        };
        self.lookbehind(distance)?;
        self.need_output(length)?;
        self.copy(distance, length);
        Ok(self.match_done())
    }

    /// `match_done`: the low two bits of the byte two back count the trailing literals.
    fn match_done(&self) -> Step {
        match usize::from(self.input[self.at - 2] & 3) {
            0 => Step::Instruction,
            t => Step::Trailing(t),
        }
    }

    /// `match_next`: copy 1 to 3 literals, then read the next instruction.
    fn trailing(&mut self, t: usize) -> Result<Step, LzoFault> {
        self.need_output(t)?;
        self.need_input(t + 3)?;
        self.literals(t);
        Ok(Step::Match(self.byte()?))
    }

    /// A length of 0 extended: 255 for each further zero byte, then `base` plus the next byte.
    ///
    /// The library also stops a length that nears `lzo_uint`'s range; a length here is at most
    /// 255 per input byte, so it cannot.
    fn extended(&mut self, base: usize) -> Result<usize, LzoFault> {
        let mut t: usize = 0;
        while self.input.get(self.at) == Some(&0) {
            t += 255;
            self.at += 1;
            self.need_input(1)?;
        }
        Ok(t + base + self.byte()?)
    }

    /// Read one byte. The lookahead checks have already made room for every read, so the
    /// check here only keeps a broken invariant from reading past the input.
    fn byte(&mut self) -> Result<usize, LzoFault> {
        let byte = *self.input.get(self.at).ok_or(LzoFault::InputOverrun)?;
        self.at += 1;
        Ok(usize::from(byte))
    }

    /// Read a little-endian 16-bit field.
    fn le16(&mut self) -> Result<usize, LzoFault> {
        let low = self.byte()?;
        Ok(low | (self.byte()? << 8))
    }

    fn literals(&mut self, count: usize) {
        let end = self.at + count;
        self.out.extend_from_slice(&self.input[self.at..end]);
        self.at = end;
    }

    /// Copy a match. A match longer than its distance repeats the bytes it has just written.
    fn copy(&mut self, distance: usize, length: usize) {
        let from = self.out.len() - distance;
        if distance >= length {
            self.out.extend_from_within(from..from + length);
        } else {
            for at in from..from + length {
                let byte = self.out[at];
                self.out.push(byte);
            }
        }
    }

    /// `NEED_IP`: the input holds `count` more bytes.
    fn need_input(&self, count: usize) -> Result<(), LzoFault> {
        if self.input.len().saturating_sub(self.at) < count {
            Err(LzoFault::InputOverrun)
        } else {
            Ok(())
        }
    }

    /// `NEED_OP`: the output has room for `count` more bytes.
    fn need_output(&self, count: usize) -> Result<(), LzoFault> {
        if self.capacity.saturating_sub(self.out.len()) < count {
            Err(LzoFault::OutputOverrun)
        } else {
            Ok(())
        }
    }

    /// `TEST_LB`: the match starts inside the output written.
    fn lookbehind(&self, distance: usize) -> Result<(), LzoFault> {
        if distance > self.out.len() {
            Err(LzoFault::LookbehindOverrun)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decompress, LzoError, LzoFault};

    /// liblzo2 2.10's `lzo1x_999_compress` of [`synthetic`]'s 47,403 bytes: 1,303 bytes that
    /// use every instruction kind: an extended first literal run and extended and short literal
    /// runs; the 3-byte match after literals and the 2-byte match after a match; 1 to 3 trailing
    /// literals; short matches; mid-distance matches, short and extended; far matches below and
    /// above 32,768 bytes back, short and extended; and the end marker.
    const SYNTHETIC: &str = concat!(
        "0000005a6924ade2356b611764b34b6d08a68b7d42a57897928a3d6783d6fdd792fb98b91f8d9645b27a",
        "14a985895426986cefe41e968e388cd7090b24bc72d4e319ad77f26edc3171ef06d81e3d1e2038f094a3",
        "6c1f938b180dc62cf2b34d7b5ae10491ecace5b5212ed3a2415aed1ba4ac9a887f8f9114a0614d958f40",
        "0e24294b721dbc1d42c393024e91846360cbfb74f0ec10eb16e85e3f359978dece2a51071644f9bad313",
        "4c4be3ec5fdc10932010a675d4251c5990c0cbefb4efd727787a47c22539c316b61fa2f0e55b66bcef87",
        "d04330247e2a1b8a4c8671991d5e7e646a34ae55ffd2ab7db2541ba28eed999bb5cb156c484c6d165ec0",
        "6bc949e5033f216cd370d9439071559f053be1ceba587101c4740d0104145ea3065abb891c907189a589",
        "4e5090d1333b195d858ed508e150ee2cb7d0ace5feee7d88e8caf36cf2a1785992677acfc6e2f16648da",
        "bbff49f8d275d700a13bfe4d42374fe0d56da6f8516a3773e77488055ef7e9c1f0669ad2f88123ff9724",
        "c4fc4a87db55e9b5a51c8807a5176569a6d128c793e6fa728cf4bb2adf60baad766ea5b6bd28cb2e4337",
        "13d495759d6f578e5014e2ff80a1ad2bc998cb273b51afb2c35bc7f95a6fe782c8733b06e66382e42869",
        "3d58956f1551939dafa5eab53245c7a4ae1542c34a5daf0fc8271fdb84054965eb1c40d029fa362bbf88",
        "1e9437ed013355feb84d10020ba758c76fc5c061fc827ea333cf1e6eb26f200b8373e53eb4cbc8f49a57",
        "7a9f8d5afaa6229c10536ccd64e939877a2dc179ec6b3eff217d29d8420d1932a79338e6317314b5066c",
        "8f9b2d13bd021de60908ae39c7614c5318054a045bbdb7d010e767352c52cf0dfb42aa002820ec3e0415",
        "e524a1a4d13e2000000000000000e6180029180002d6c92df24c057888040304a3d1756205b681200000",
        "0000000000e618002000000000000000e618002000000000000000e618002069180001c539455f20071c",
        "8b04b10d5c7ed06c542000000000000000e618002000000000000000e618002000000000000000e61800",
        "2000000000000000e618002000000000000000e61800200000a6180001df6c58a113703504e05df2fe25",
        "94ca2000000000000000e618002000000000000000e61800200000000000000046180001e486f6d71033",
        "a091045ded17a5d945c32000000000000000e618002000000000000000e618002000000000000000e618",
        "002000000000000000e618002000000000000000e618002000000000000000e61800200000000000004e",
        "1b00cffc991ccc6a04efdc99d334d9ce2000000000000000e618002000000000000000e6180020000000",
        "00000000e61800200000002a180001dedd0f2d1851a0d600001b266b60099b72e1ab85b166ed6828af88",
        "64e9353bf8a4d226873348eac52f7426eaca16233ffb6d424c91dfd5068aa8daecd7b0b6a30b60a40123",
        "7743e14b622d1daf6ebe8102abb84aa2010d3f6d4a1a1ccc6e6fb6ef7380238633d06c82c225d03b3c08",
        "55f9117fb4ba0e1bd3fd681cad03bcf4aacef2f23507a4fd9b353cb12025ad8f124223e32f16bb1245b8",
        "08db82956e5fa2d0fb53ae8b0ecba164cde8109c02537d267cb391936850f3dd09ac414640ee725d60b4",
        "b55c941bc20a28509d4bd17b064adeb611118581c5f52b1ca89367df64fe0c769b65f46da4803732d4e2",
        "4520b42b8cda0b0d401276997c29dc814e7b0e208ba7a802ae10090ef7919a1d783efe184e29612b3cca",
        "49e982678e6dcef5dd1660905877cf2f70353f868771fe9cd968f487acd0bbb951000b070092986e1100",
        "00",
    );

    fn unhex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("ASCII hex");
                u8::from_str_radix(pair, 16).expect("a hex byte")
            })
            .collect()
    }

    /// The bytes the synthetic stream was compressed from: a xorshift32 generator's low bytes,
    /// copies from a unique prefix at chosen distances, and period-7 runs between them.
    struct Synthetic {
        state: u32,
        out: Vec<u8>,
    }

    impl Synthetic {
        fn fresh(&mut self, count: usize) {
            for _ in 0..count {
                self.state ^= self.state << 13;
                self.state ^= self.state >> 17;
                self.state ^= self.state << 5;
                self.out.push(self.state.to_le_bytes()[0]);
            }
        }

        fn back(&mut self, distance: usize, length: usize) {
            for _ in 0..length {
                self.out.push(self.out[self.out.len() - distance]);
            }
        }

        fn pad(&mut self, to: usize) {
            self.fresh(7);
            while self.out.len() < to {
                self.back(7, 1);
            }
        }

        fn far(&mut self, from: usize, length: usize) {
            for at in from..from + length {
                self.out.push(self.out[at]);
            }
        }
    }

    fn synthetic() -> Vec<u8> {
        let mut data = Synthetic {
            state: 0x2290_0001,
            out: Vec::new(),
        };
        data.fresh(600);
        data.fresh(6);
        data.back(300, 2);
        data.fresh(1);
        data.fresh(9);
        data.back(3, 6);
        data.fresh(2);
        data.back(500, 8);
        data.pad(2700);
        data.fresh(5);
        data.back(2530, 3);
        data.fresh(1);
        data.back(700, 2);
        for (to, fresh, from, length) in [
            (9000, 4, 100, 40),
            (20000, 4, 200, 5),
            (26000, 4, 300, 60),
            (40000, 3, 400, 6),
            (47000, 4, 500, 90),
        ] {
            data.pad(to);
            data.fresh(fresh);
            data.far(from, length);
        }
        data.fresh(300);
        data.back(5, 3);
        data.fresh(1);
        data.back(2, 2);
        data.fresh(3);
        data.out
    }

    fn fault(input: &[u8], capacity: usize) -> (LzoFault, usize) {
        let error = decompress(input, capacity).expect_err("a broken stream");
        (error.fault, error.written)
    }

    #[test]
    fn decodes_every_instruction_kind() {
        let stream = unhex(SYNTHETIC);
        let data = synthetic();
        assert_eq!((stream.len(), data.len()), (1303, 47_403));
        assert_eq!(decompress(&stream, data.len()), Ok(data.clone()));
        assert_eq!(decompress(&stream, usize::MAX), Ok(data));
    }

    #[test]
    fn stops_at_the_capacity_as_the_library_does() {
        let stream = unhex(SYNTHETIC);
        assert_eq!(fault(&stream, 47_402), (LzoFault::OutputOverrun, 47_400));
        assert_eq!(fault(&stream, 0), (LzoFault::OutputOverrun, 0));
    }

    #[test]
    fn every_prefix_of_a_stream_overruns_the_input() {
        let stream = unhex(SYNTHETIC);
        let data = synthetic();
        for end in 0..stream.len() {
            let error = decompress(&stream[..end], data.len()).expect_err("a prefix");
            assert_eq!(error.fault, LzoFault::InputOverrun, "prefix {end}");
            assert!(error.written <= data.len(), "prefix {end}");
        }
        // The output the library had written, for prefixes that end inside the first literal
        // run, inside a later one, and one byte short of the end marker.
        for (end, written) in [(0, 0), (1, 0), (607, 0), (700, 6814), (1000, 47_094)] {
            assert_eq!(
                fault(&stream[..end], data.len()),
                (LzoFault::InputOverrun, written),
                "prefix {end}"
            );
        }
        assert_eq!(
            fault(&stream[..1302], data.len()),
            (LzoFault::InputOverrun, 47_400)
        );
    }

    #[test]
    fn decodes_the_library_answers_for_hand_streams() {
        assert_eq!(fault(b"", 16), (LzoFault::InputOverrun, 0));
        assert_eq!(decompress(b"\x11\x00\x00", 16), Ok(Vec::new()));
        assert_eq!(
            fault(b"\x11\x00\x00\x00", 16),
            (LzoFault::InputNotConsumed, 0)
        );
        assert_eq!(fault(b"\x11\x00", 16), (LzoFault::InputOverrun, 0));
        // A first byte above 17 is a literal run.
        assert_eq!(
            decompress(b"\x15abcd\x11\x00\x00", 16),
            Ok(b"abcd".to_vec())
        );
        assert_eq!(
            fault(b"\x15abcd\x11\x00\x00", 3),
            (LzoFault::OutputOverrun, 0)
        );
        assert_eq!(fault(b"\x15abc", 16), (LzoFault::InputOverrun, 0));
        // Below 21 it is 1 to 3 trailing literals, here before a 3-byte match 1 byte back.
        assert_eq!(
            decompress(b"\x12a\x40\x00\x11\x00\x00", 16),
            Ok(b"aaaa".to_vec())
        );
        assert_eq!(
            fault(b"\x12a\x40\x00\x11\x00\x00", 3),
            (LzoFault::OutputOverrun, 1)
        );
        // A match 9 bytes back, and a 3-byte match 2049 bytes back, after 4 bytes.
        assert_eq!(
            fault(b"\x15abcd\x40\x01\x11\x00\x00", 16),
            (LzoFault::LookbehindOverrun, 4)
        );
        assert_eq!(
            fault(b"\x15abcd\x01\x00\x11\x00\x00", 16),
            (LzoFault::LookbehindOverrun, 4)
        );
    }

    #[test]
    fn a_literal_run_needs_its_room_and_the_next_instructions_lookahead() {
        // The first byte's run, then an instruction's run of 1 + 3, each followed by the end
        // marker, cut short of it, and given too little room.
        assert_eq!(fault(b"\x15abcd", 16), (LzoFault::InputOverrun, 0));
        assert_eq!(fault(b"\x15abcd\x11\x00", 16), (LzoFault::InputOverrun, 0));
        assert_eq!(
            decompress(b"\x01abcd\x11\x00\x00", 16),
            Ok(b"abcd".to_vec())
        );
        assert_eq!(
            fault(b"\x01abcd\x11\x00\x00", 3),
            (LzoFault::OutputOverrun, 0)
        );
        assert_eq!(fault(b"\x01abcd", 16), (LzoFault::InputOverrun, 0));
        assert_eq!(fault(b"\x01abcd\x11\x00", 16), (LzoFault::InputOverrun, 0));
    }

    #[test]
    fn a_far_match_ends_the_stream_only_when_its_whole_distance_field_is_zero() {
        // 16384 from the high bit and 0 from the field: a 3-byte match 32768 bytes back.
        assert_eq!(
            fault(b"\x15abcd\x19\x00\x00\x11\x00\x00", 16),
            (LzoFault::LookbehindOverrun, 4)
        );
        let data: Vec<u8> = (0..32_768_u32)
            .map(|at| (at * 7 + 3).to_le_bytes()[0])
            .collect();
        // A literal run of 32640 + 110 + 15 + 3 bytes, the match, and the end marker.
        let mut stream = vec![0; 129];
        stream.push(110);
        stream.extend_from_slice(&data);
        stream.extend_from_slice(b"\x19\x00\x00\x11\x00\x00");
        let mut expected = data.clone();
        expected.extend_from_slice(&data[..3]);
        assert_eq!(decompress(&stream, 40_000), Ok(expected));
    }

    #[test]
    fn a_byte_below_16_after_trailing_literals_is_a_2_byte_match_within_the_output() {
        assert_eq!(
            decompress(b"\x12a\x00\x00\x11\x00\x00", 16),
            Ok(b"aaa".to_vec())
        );
        assert_eq!(
            fault(b"\x12a\x04\x00\x11\x00\x00", 16),
            (LzoFault::LookbehindOverrun, 1)
        );
        assert_eq!(
            fault(b"\x12a\x00\x00\x11\x00\x00", 2),
            (LzoFault::OutputOverrun, 1)
        );
    }

    #[test]
    fn every_match_form_checks_its_reach_before_its_room() {
        // A short, a mid-distance and a far match, and the 3-byte match after literals, each
        // too far back for the 4 bytes written and too long for the 1 byte of room left.
        for stream in [
            &b"\x15abcd\x40\x01\x11\x00\x00"[..],
            b"\x15abcd\x21\x20\x00\x11\x00\x00",
            b"\x15abcd\x11\x04\x00\x11\x00\x00",
            b"\x15abcd\x01\x00\x11\x00\x00",
        ] {
            assert_eq!(fault(stream, 5), (LzoFault::LookbehindOverrun, 4));
        }
        // The 2-byte match after trailing literals.
        assert_eq!(
            fault(b"\x12a\x04\x00\x11\x00\x00", 2),
            (LzoFault::LookbehindOverrun, 1)
        );
    }

    #[test]
    fn names_the_library_codes() {
        let codes = [
            LzoFault::InputOverrun,
            LzoFault::OutputOverrun,
            LzoFault::LookbehindOverrun,
            LzoFault::InputNotConsumed,
        ]
        .map(LzoFault::code);
        assert_eq!(codes, [-4, -5, -6, -8]);
        let error = LzoError {
            fault: LzoFault::LookbehindOverrun,
            written: 4,
        };
        assert_eq!(
            error.to_string(),
            "a match reaches before the output (after 4 bytes out)"
        );
    }
}
