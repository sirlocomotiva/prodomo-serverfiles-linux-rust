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
}
