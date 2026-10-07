//! The random numbers gameplay draws: legacy's `number(from, to)` over a source the caller
//! owns.
//!
//! Legacy draws from the C library's `random()` (`libthecore/utils.cpp:345-352`), one process-wide
//! sequence seeded at boot. The Rewrite takes the source as a [`Dice`] argument instead, so a
//! reducer stays deterministic under test and the game thread decides where the numbers come
//! from. [`Pcg32`] is the source the game thread uses; the client never sees the sequence, only
//! the numbers it produced, so which generator backs `random()` is not observable.

/// A source of the 31-bit numbers `random()` returns.
pub trait Dice {
    /// The next number, in `0..2^31`.
    fn random31(&mut self) -> u32;
}

/// `number(from, to)` (`libthecore/utils.cpp:355-375`): a number from `from` to `to`, both
/// included.
///
/// Legacy swaps the bounds when `from` is the larger. It computes the width `to - from + 1` in
/// `int`, and answers 0 without drawing when that width wraps to 0, which only the full `int`
/// range does. Otherwise it takes the draw, a `DWORD` (`thecore_random`, `:346-352`), modulo the
/// width, so the `%` converts the width to `unsigned int` and is unsigned; `from` is added in
/// `unsigned int` too, and the sum is stored back into an `int` (the 32-bit build's two's
/// complement wrap). A width past `INT_MAX` is therefore more than any 31-bit draw, which comes
/// back whole, plus `from`.
pub fn number(dice: &mut dyn Dice, from: i32, to: i32) -> i32 {
    let (from, to) = if from > to { (to, from) } else { (from, to) };
    let width = to.wrapping_sub(from).wrapping_add(1);
    if width == 0 {
        return 0;
    }
    let draw = dice.random31() & 0x7fff_ffff;
    let width = u32::from_ne_bytes(width.to_ne_bytes());
    let from = u32::from_ne_bytes(from.to_ne_bytes());
    i32::from_ne_bytes((draw % width).wrapping_add(from).to_ne_bytes())
}

/// PCG-XSH-RR with 64 bits of state and 32 of output (O'Neill, `pcg32_random_r`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcg32 {
    state: u64,
    increment: u64,
}

/// The LCG multiplier of the reference implementation.
const PCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;

impl Pcg32 {
    /// `pcg32_srandom_r(seed, stream)`: the generator the reference seeds with this state and
    /// stream.
    #[must_use]
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut pcg = Self {
            state: 0,
            increment: (stream << 1) | 1,
        };
        let _ = pcg.next_u32();
        pcg.state = pcg.state.wrapping_add(seed);
        let _ = pcg.next_u32();
        pcg
    }

    /// The next 32-bit output.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(PCG_MULTIPLIER)
            .wrapping_add(self.increment);
        // The reference keeps the low 32 bits; the mask makes the conversion total.
        let xorshifted = u32::try_from((((old >> 18) ^ old) >> 27) & 0xffff_ffff).unwrap_or(0);
        let rotation = u32::try_from(old >> 59).unwrap_or(0);
        xorshifted.rotate_right(rotation)
    }
}

impl Dice for Pcg32 {
    fn random31(&mut self) -> u32 {
        self.next_u32() >> 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that answers a fixed sequence, then repeats its last number.
    struct Fixed(Vec<u32>);

    impl Dice for Fixed {
        fn random31(&mut self) -> u32 {
            if self.0.len() > 1 {
                self.0.remove(0)
            } else {
                self.0[0]
            }
        }
    }

    #[test]
    fn the_generator_matches_the_reference_demo() {
        // `pcg32-demo`'s first six outputs for seed 42 and stream 54.
        let mut pcg = Pcg32::new(42, 54);
        let outputs: Vec<u32> = (0..6).map(|_| pcg.next_u32()).collect();
        assert_eq!(
            outputs,
            [
                0xa15c_02b7,
                0x7b47_f409,
                0xba1d_3330,
                0x83d2_f293,
                0xbfa4_784b,
                0xcbed_606e
            ]
        );
    }

    #[test]
    fn a_draw_is_31_bits() {
        let mut pcg = Pcg32::new(42, 54);
        assert_eq!(pcg.random31(), 0xa15c_02b7 >> 1);
        let mut pcg = Pcg32::new(7, 1);
        assert!((0..1000).all(|_| pcg.random31() < 1 << 31));
    }

    #[test]
    fn number_is_the_draw_modulo_the_width_plus_the_low_bound() {
        let mut dice = Fixed(vec![0, 8, 9, 1_234_567]);
        assert_eq!(number(&mut dice, 11, 19), 11);
        assert_eq!(number(&mut dice, 11, 19), 19);
        assert_eq!(number(&mut dice, 11, 19), 11);
        assert_eq!(number(&mut dice, 11, 19), 11 + 1_234_567 % 9);
    }

    #[test]
    fn number_swaps_bounds_given_the_wrong_way_round() {
        let mut dice = Fixed(vec![3]);
        assert_eq!(number(&mut dice, 19, 11), 14);
        assert_eq!(number(&mut dice, -5, -9), -6);
    }

    #[test]
    fn the_full_int_range_answers_zero_without_a_draw() {
        let mut dice = Fixed(vec![5, 6]);
        assert_eq!(number(&mut dice, i32::MIN, i32::MAX), 0);
        assert_eq!(dice.random31(), 5, "the draw was not taken");
    }

    #[test]
    fn a_width_past_int_max_takes_the_draw_whole_as_the_unsigned_modulus_does() {
        // The width 3_000_000_000 is -1_294_967_296 as an `int`, which the `DWORD` draw's `%`
        // reads back as 3_000_000_000: a draw of 1_300_000_000 is below it and comes back
        // whole, plus the low bound. A signed `%` would leave 5_032_704 over it.
        let mut dice = Fixed(vec![1_300_000_000, 2_147_483_647, 0]);
        let from = -1_000_000_000;
        let to = 1_999_999_999;
        assert_eq!(number(&mut dice, from, to), 300_000_000);
        assert_eq!(number(&mut dice, from, to), 1_147_483_647);
        assert_eq!(number(&mut dice, from, to), from);
    }

    #[test]
    fn a_negative_low_bound_wraps_through_the_unsigned_sum() {
        // `(draw % width) + from` is `unsigned int` arithmetic stored into an `int`; within an
        // `int` width the wrap lands where the signed sum does.
        let mut dice = Fixed(vec![1_999_999_999, 1_999_999_999]);
        assert_eq!(number(&mut dice, -10, 10), -10 + 4);
        assert_eq!(number(&mut dice, i32::MIN, -1), i32::MIN + 1_999_999_999);
    }

    #[test]
    fn a_single_value_range_answers_it() {
        let mut dice = Fixed(vec![123_456]);
        assert_eq!(number(&mut dice, 7, 7), 7);
    }
}
