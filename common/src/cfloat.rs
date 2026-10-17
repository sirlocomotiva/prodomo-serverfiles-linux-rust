//! The two C conversions between `int` and `float` that legacy arithmetic relies on.
//!
//! The legacy target is 32-bit x86 built with SSE2 (`server/server/premake5.lua:12`), so a
//! `(float)i` of a 4-byte `int` rounds to the nearest `float`, ties to even, and a `(int)f` drops
//! the fraction toward zero. A formula such as the one `CHARACTER::ApplyPoint` uses to keep the
//! share of a pool when its maximum changes (`char.cpp:4879-4885`) depends on both roundings, so a
//! port must perform them the same way.
//!
//! Rust's `as` performs the same two conversions, but the workspace lints refuse a lossy `as`.
//! These helpers compute them from exact steps instead, and the tests witness each against the
//! 64-bit arithmetic, which holds every 32-bit `int` exactly.

/// `(float)value`: the nearest `f32` to `value`, ties to even.
///
/// The high 16 bits and the low 16 bits each convert exactly, and multiplying by 65536 only moves
/// the exponent, so the one addition is the only rounding, and IEEE 754 rounds it to nearest.
pub fn i32_to_f32(value: i32) -> f32 {
    let high = i16::try_from(value >> 16).unwrap_or(0);
    let low = u16::try_from(value & 0xffff).unwrap_or(0);
    f32::from(high) * 65_536.0 + f32::from(low)
}

/// `(float)value` of a `DWORD`: the nearest `f32` to `value`, ties to even.
///
/// The same exact steps as [`i32_to_f32`], with an unsigned high half.
#[must_use]
pub fn u32_to_f32(value: u32) -> f32 {
    let high = u16::try_from(value >> 16).unwrap_or(0);
    let low = u16::try_from(value & 0xffff).unwrap_or(0);
    f32::from(high) * 65_536.0 + f32::from(low)
}

/// `(int)value`: the `f32` with its fraction dropped toward zero.
///
/// C leaves a value outside the `int` range undefined, and SSE2 returns `INT_MIN` for it. No
/// ported formula can reach that range from valid points, so the Rewrite saturates, as Rust's
/// `as` does: a value at or above 2^31 is `i32::MAX`, one at or below -2^31 is `i32::MIN`, and a
/// NaN is 0.
pub fn f32_to_i32(value: f32) -> i32 {
    const TWO_POW_31: f32 = 2_147_483_648.0;
    if value.is_nan() {
        return 0;
    }
    if value >= TWO_POW_31 {
        return i32::MAX;
    }
    if value <= -TWO_POW_31 {
        return i32::MIN;
    }
    let bits = value.to_bits();
    let exponent = (bits >> 23) & 0xff;
    if exponent < 127 {
        // The magnitude is below 1, so the fraction is all there is.
        return 0;
    }
    // The range test above keeps the magnitude below 2^31, so the shift is at most 30.
    let shift = exponent - 127;
    let mantissa = (bits & 0x7f_ffff) | 0x80_0000;
    let magnitude = if shift >= 23 {
        i64::from(mantissa) << (shift - 23)
    } else {
        i64::from(mantissa >> (23 - shift))
    };
    let signed = if bits >> 31 == 0 {
        magnitude
    } else {
        -magnitude
    };
    i32::try_from(signed).unwrap_or(if signed < 0 { i32::MIN } else { i32::MAX })
}

/// `(int)value` of a `double`: the `f64` with its fraction dropped toward zero.
///
/// The same rules as [`f32_to_i32`]: a value at or above 2^31 is `i32::MAX`, one at or below
/// -2^31 is `i32::MIN`, and a NaN is 0. `CalcAttBonus` reaches it, where `/ 100.0` turns the
/// monster bonus into a `double` before it is added back to the `int` attack.
#[must_use]
pub fn f64_to_i32(value: f64) -> i32 {
    const TWO_POW_31: f64 = 2_147_483_648.0;
    if value.is_nan() {
        return 0;
    }
    if value >= TWO_POW_31 {
        return i32::MAX;
    }
    if value <= -TWO_POW_31 {
        return i32::MIN;
    }
    let bits = value.to_bits();
    let exponent = (bits >> 52) & 0x7ff;
    if exponent < 1023 {
        // The magnitude is below 1, so the fraction is all there is.
        return 0;
    }
    // The range test above keeps the magnitude below 2^31, so the shift is at most 30.
    let shift = exponent - 1023;
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    let magnitude = i64::try_from(mantissa >> (52 - shift)).unwrap_or(i64::MAX);
    let signed = if bits >> 63 == 0 {
        magnitude
    } else {
        -magnitude
    };
    i32::try_from(signed).unwrap_or(if signed < 0 { i32::MIN } else { i32::MAX })
}

/// `(float)value` of a `double`: the nearest `f32` to `value`, ties to even.
///
/// C casts a `double` to a `float` by rounding to the nearest `f32`, ties to even. The
/// conversion is performed from the exact bits, with no `as` cast, so the pedantic
/// `cast_precision_loss` lint stays quiet. Used by the lead's `fFollowSpeed`
/// (`G/char.cpp:6949`), which mixes `double` and `float` arithmetic.
/// # Panics
///
/// Panics only if an internal bit-width conversion fails, which cannot happen for valid
/// inputs.
#[must_use]
pub fn f64_to_f32(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign_bit = bits & (1u64 << 63);
    let exp64 = (bits >> 52) & 0x7ff;
    let frac64 = bits & 0x000f_ffff_ffff_ffff;

    // NaN.
    if exp64 == 0x7ff && frac64 != 0 {
        let nan32: u64 = (sign_bit >> 32) | 0x7fc0_0000;
        return f32::from_bits(u32::try_from(nan32).unwrap());
    }
    // Infinity.
    if exp64 == 0x7ff {
        let inf32: u64 = (sign_bit >> 32) | 0x7f80_0000;
        return f32::from_bits(u32::try_from(inf32).unwrap());
    }
    // Zero or subnormal f64: all below the f32 range.
    if exp64 == 0 {
        let zero32: u64 = sign_bit >> 32;
        return f32::from_bits(u32::try_from(zero32).unwrap());
    }

    let exp = i64::try_from(exp64).unwrap() - 1023;
    let mant = frac64 | (1u64 << 52);
    let e32 = exp + 127;

    if e32 >= 255 {
        let inf32: u64 = (sign_bit >> 32) | 0x7f80_0000;
        return f32::from_bits(u32::try_from(inf32).unwrap());
    }

    if e32 <= 0 {
        let shift = (1 - e32) + 29;
        if shift >= 63 {
            let zero32: u64 = sign_bit >> 32;
            return f32::from_bits(u32::try_from(zero32).unwrap());
        }
        let rshift = u32::try_from(shift).unwrap();
        let mant_shifted = mant >> rshift;
        let remainder = mant & ((1u64 << rshift) - 1);
        let half = 1u64 << (rshift - 1);
        let rounded = if remainder > half || (remainder == half && (mant_shifted & 1) == 1) {
            mant_shifted + 1
        } else {
            mant_shifted
        };
        if rounded >= (1u64 << 24) {
            let normal_bits: u64 = (sign_bit >> 32) | (1u64 << 23);
            return f32::from_bits(u32::try_from(normal_bits).unwrap());
        }
        let subnormal_bits: u64 = (sign_bit >> 32) | rounded;
        return f32::from_bits(u32::try_from(subnormal_bits).unwrap());
    }

    let rshift: u32 = 29;
    let mant_shifted = mant >> rshift;
    let remainder = mant & ((1u64 << rshift) - 1);
    let half = 1u64 << (rshift - 1);
    let mut rounded = if remainder > half || (remainder == half && (mant_shifted & 1) == 1) {
        mant_shifted + 1
    } else {
        mant_shifted
    };
    let mut e32_final = e32;
    if rounded >= (1u64 << 24) {
        rounded >>= 1;
        e32_final += 1;
        if e32_final >= 255 {
            let inf32: u64 = (sign_bit >> 32) | 0x7f80_0000;
            return f32::from_bits(u32::try_from(inf32).unwrap());
        }
    }
    let mant32 = rounded & 0x007f_ffff;
    let e32u = u64::try_from(e32_final).unwrap();
    let result_bits: u64 = (sign_bit >> 32) | (e32u << 23) | mant32;
    f32::from_bits(u32::try_from(result_bits).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The correctly rounded `f32` of `value`, computed through `f64`, which holds every `i32`.
    fn witness_f32(value: i32) -> u32 {
        let exact = f64::from(value);
        // Every f32 is an f64, so the nearest f32 is one of the two that bracket `exact`.
        let mut best = 0.0_f32;
        let mut best_gap = f64::INFINITY;
        let seed = i32_to_f32(value);
        for candidate in [seed, next_up(seed), next_down(seed)] {
            let gap = (f64::from(candidate) - exact).abs();
            let even = candidate.to_bits() & 1 == 0;
            if gap < best_gap || (gap.to_bits() == best_gap.to_bits() && even) {
                best = candidate;
                best_gap = gap;
            }
        }
        best.to_bits()
    }

    fn next_up(value: f32) -> f32 {
        if value == 0.0 {
            return f32::from_bits(1);
        }
        let bits = value.to_bits();
        f32::from_bits(if value > 0.0 { bits + 1 } else { bits - 1 })
    }

    fn next_down(value: f32) -> f32 {
        -next_up(-value)
    }

    fn samples() -> Vec<i32> {
        let mut values = vec![
            0,
            1,
            -1,
            2,
            -2,
            100,
            -100,
            65_535,
            65_536,
            -65_536,
            -65_537,
            16_777_215,
            16_777_216,
            16_777_217,
            16_777_218,
            16_777_219,
            -16_777_217,
            -16_777_219,
            33_554_435,
            0x1234_5678,
            -0x1234_5678,
            i32::MAX,
            i32::MAX - 1,
            i32::MIN,
            i32::MIN + 1,
        ];
        let mut state = 0x9e37_79b9_u32;
        for _ in 0..4096 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            values.push(i32::from_ne_bytes(state.to_ne_bytes()));
        }
        values
    }

    #[test]
    fn a_small_int_converts_exactly() {
        for value in [-16_777_216, -3, 0, 7, 12_345, 16_777_216] {
            let float = f64::from(i32_to_f32(value));
            assert_eq!(float.to_bits(), f64::from(value).to_bits(), "{value}");
        }
    }

    #[test]
    fn a_wide_int_rounds_to_nearest_with_ties_to_even() {
        // 2^24 + 1 is halfway between 2^24 and 2^24 + 2 and rounds to the even 2^24; 2^24 + 3
        // is halfway between 2^24 + 2 and 2^24 + 4 and rounds to the even 2^24 + 4.
        assert_eq!(i32_to_f32(16_777_217).to_bits(), 16_777_216.0_f32.to_bits());
        assert_eq!(i32_to_f32(16_777_219).to_bits(), 16_777_220.0_f32.to_bits());
        assert_eq!(
            i32_to_f32(-16_777_219).to_bits(),
            (-16_777_220.0_f32).to_bits()
        );
        assert_eq!(
            i32_to_f32(i32::MAX).to_bits(),
            2_147_483_648.0_f32.to_bits()
        );
        assert_eq!(
            i32_to_f32(i32::MIN).to_bits(),
            (-2_147_483_648.0_f32).to_bits()
        );
        for value in samples() {
            assert_eq!(i32_to_f32(value).to_bits(), witness_f32(value), "{value}");
        }
    }

    #[test]
    fn an_unsigned_count_converts_to_the_nearest_float_ties_to_even() {
        // 2^24 + 1 rounds down to the even 2^24, 2^24 + 3 up to the even 2^24 + 4, and the
        // largest DWORD to 2^32, which no i32 path reaches.
        assert_eq!(u32_to_f32(16_777_217).to_bits(), 16_777_216.0_f32.to_bits());
        assert_eq!(u32_to_f32(16_777_219).to_bits(), 16_777_220.0_f32.to_bits());
        assert_eq!(
            u32_to_f32(u32::MAX).to_bits(),
            4_294_967_296.0_f32.to_bits()
        );
        assert_eq!(
            u32_to_f32(0x8000_0001).to_bits(),
            2_147_483_648.0_f32.to_bits()
        );
        for value in samples() {
            let unsigned = u32::from_ne_bytes(value.to_ne_bytes());
            // The witness rounds the exact value through its two bracketing floats.
            let exact = f64::from(unsigned);
            let got = u32_to_f32(unsigned);
            let gap = (f64::from(got) - exact).abs();
            for neighbour in [next_up(got), next_down(got)] {
                let other = (f64::from(neighbour) - exact).abs();
                assert!(
                    gap < other || (gap.to_bits() == other.to_bits() && got.to_bits() & 1 == 0),
                    "{unsigned}"
                );
            }
        }
    }

    #[test]
    fn an_unsigned_count_below_two_to_the_24_converts_exactly() {
        for value in [
            0_u32, 1, 2, 999, 65_535, 65_536, 65_537, 5_333, 16_777_215, 16_777_216,
        ] {
            assert_eq!(
                f64::from(u32_to_f32(value)).to_bits(),
                f64::from(value).to_bits()
            );
        }
    }

    #[test]
    fn a_float_truncates_toward_zero() {
        for (value, expected) in [
            (0.0_f32, 0),
            (-0.0, 0),
            (0.999_999, 0),
            (-0.999_999, 0),
            (1.0, 1),
            (1.5, 1),
            (-1.5, -1),
            (2.5, 2),
            (-2.5, -2),
            (123.75, 123),
            (-123.75, -123),
            (8_388_607.5, 8_388_607),
            (-8_388_607.5, -8_388_607),
            (16_777_216.0, 16_777_216),
            (2_147_483_520.0, 2_147_483_520),
            (-2_147_483_520.0, -2_147_483_520),
            (f32::MIN_POSITIVE, 0),
        ] {
            assert_eq!(f32_to_i32(value), expected, "{value}");
        }
    }

    #[test]
    fn a_float_agrees_with_the_truncated_wide_value() {
        let mut state = 0x2545_f491_u32;
        for _ in 0..4096 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let value = f32::from_bits(state);
            if value.is_nan() || value.abs() >= 2_147_483_648.0 {
                continue;
            }
            // A fraction of either sign truncates to -0.0 or +0.0, and the int is plain 0.
            let truncated = f64::from(value).trunc();
            let expected = if truncated.abs() < 1.0 {
                0.0
            } else {
                truncated
            };
            let got = f64::from(f32_to_i32(value));
            assert_eq!(got.to_bits(), expected.to_bits(), "{value}");
        }
        for value in samples() {
            let float = i32_to_f32(value);
            if float.abs() >= 2_147_483_648.0 {
                continue;
            }
            let got = f64::from(f32_to_i32(float));
            assert_eq!(got.to_bits(), f64::from(float).to_bits(), "{value}");
        }
    }

    #[test]
    fn a_float_outside_the_int_range_saturates_and_nan_is_zero() {
        assert_eq!(f32_to_i32(2_147_483_648.0), i32::MAX);
        assert_eq!(f32_to_i32(1.0e20), i32::MAX);
        assert_eq!(f32_to_i32(f32::INFINITY), i32::MAX);
        assert_eq!(f32_to_i32(-2_147_483_648.0), i32::MIN);
        assert_eq!(f32_to_i32(-1.0e20), i32::MIN);
        assert_eq!(f32_to_i32(f32::NEG_INFINITY), i32::MIN);
        assert_eq!(f32_to_i32(f32::NAN), 0);
    }

    #[test]
    fn a_double_truncates_toward_zero_and_saturates() {
        for (value, expected) in [
            (0.0_f64, 0),
            (-0.0, 0),
            (0.999_999_999, 0),
            (-0.999_999_999, 0),
            (1.0, 1),
            (1.5, 1),
            (-1.5, -1),
            (61.999_999_999, 61),
            (-61.999_999_999, -61),
            (2_147_483_647.9, i32::MAX),
            (-2_147_483_647.9, -2_147_483_647),
            (f64::MIN_POSITIVE, 0),
        ] {
            assert_eq!(f64_to_i32(value), expected, "{value}");
        }
        assert_eq!(f64_to_i32(2_147_483_648.0), i32::MAX);
        assert_eq!(f64_to_i32(f64::INFINITY), i32::MAX);
        assert_eq!(f64_to_i32(-2_147_483_648.0), i32::MIN);
        assert_eq!(f64_to_i32(f64::NEG_INFINITY), i32::MIN);
        assert_eq!(f64_to_i32(f64::NAN), 0);
        for value in samples() {
            assert_eq!(f64_to_i32(f64::from(value)), value, "{value}");
            let fraction = f64::from(value) + if value < 0 { -0.75 } else { 0.75 };
            if fraction.abs() < 2_147_483_648.0 {
                assert_eq!(f64_to_i32(fraction), value, "{fraction}");
            }
        }
    }

    /// `f64_to_f32` rounds to nearest, ties to even, over the speeds and distances (U18).
    #[test]
    fn f64_to_f32_rounds_to_nearest_even() {
        // Exact ties: the midpoint between two adjacent floats rounds to the even one.
        for low in [1.0_f32, 2.0_f32, 100.0_f32, 300.0_f32, 10_000.0_f32] {
            for bits in [low.to_bits(), low.to_bits() | 1] {
                let even = f32::from_bits(bits & !1);
                let odd = f32::from_bits(bits | 1);
                let (below, above) = if even < odd { (even, odd) } else { (odd, even) };
                if above <= below {
                    continue;
                }
                let mid = (f64::from(below) + f64::from(above)) / 2.0;
                let want = if below.to_bits() & 1 == 0 {
                    below
                } else {
                    above
                };
                assert_eq!(f64_to_f32(mid).to_bits(), want.to_bits(), "{mid}");
            }
        }
        // The speeds and distances range: the nearest float, ties to even, witnessed
        // through the two bracketing floats, as the unsigned tests do.
        let mut values = vec![300.0, 394.7368, 0.1, 157.5, 201.25, 4800.0, 100_000.0];
        values.extend((0..20_000).map(|n| f64::from(n) / 2.0));
        values.extend((-5000..5000).map(|n| f64::from(n) * 1.5));
        for value in values {
            let got = f64_to_f32(value);
            let gap = (f64::from(got) - value).abs();
            for neighbour in [next_up(got), next_down(got)] {
                let other = (f64::from(neighbour) - value).abs();
                assert!(
                    gap < other || (gap.to_bits() == other.to_bits() && got.to_bits() & 1 == 0),
                    "{value}"
                );
            }
        }
    }

    #[test]
    fn f64_to_f32_known_values() {
        assert_eq!(f64_to_f32(0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(f64_to_f32(-0.0).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(f64_to_f32(1.5).to_bits(), 1.5_f32.to_bits());
        assert_eq!(f64_to_f32(1.0).to_bits(), 1.0_f32.to_bits());
        assert_eq!(f64_to_f32(f64::INFINITY).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(
            f64_to_f32(f64::NEG_INFINITY).to_bits(),
            f32::NEG_INFINITY.to_bits()
        );
        assert!(f64_to_f32(f64::NAN).is_nan());
        assert_eq!(f64_to_f32(1.0e300).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(f64_to_f32(-1.0e300).to_bits(), f32::NEG_INFINITY.to_bits());
    }
}
