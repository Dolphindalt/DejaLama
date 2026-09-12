//! Emulate the x87 arithmetic the DLL runs: products rounded to a 64-bit mantissa and
//! truncating float-to-int conversion.

/// Truncating float-to-int conversion, like MSVC `_ftol`.
#[inline]
pub(super) fn ftol(x: f64) -> i32 {
    x as i32
}

/// An x87 extended-precision value: sign, a 64-bit mantissa and a power-of-two exponent.
///
/// Build one with [`x87`], multiply with [`X87::times`], and finish with [`X87::trunc`] (the
/// DLL's `_ftol`) or [`X87::to_f32`] (`FSTP float`). Every multiplication rounds the exact product
/// to 64 bits with round-to-nearest-even, exactly like `FMUL`. Expect different results from
/// plain f64 arithmetic when a product lies within half an f64 ulp of an integer: 96000·f64(0.013)
/// falls just short of 1248 because f64(0.013) < 0.013, so the FPU truncates it to 1247 while f64
/// rounds it to 1248.0. Several table sizes depend on this.
#[derive(Clone, Copy, Debug)]
pub(super) struct X87 {
    negative: bool,
    mantissa: u64,
    exponent: i32,
}

/// Start an x87 computation from an f32 or an f64 (both convert exactly).
pub(super) fn x87(value: impl Into<f64>) -> X87 {
    let (mantissa, exponent, negative) = decompose(value.into());
    let (mantissa, exponent) = round_to_bits(u128::from(mantissa), exponent, 64);
    X87 {
        negative,
        mantissa: mantissa as u64,
        exponent,
    }
}

/// Split an f64 into `mantissa · 2^exponent` with its sign.
fn decompose(value: f64) -> (u64, i32, bool) {
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let negative = bits >> 63 == 1;
    if biased == 0 {
        (fraction, -1074, negative)
    } else {
        (fraction | (1u64 << 52), biased - 1075, negative)
    }
}

/// Round `mantissa · 2^exponent` to `bits` significant bits, nearest even.
fn round_to_bits(mantissa: u128, exponent: i32, bits: u32) -> (u128, i32) {
    let width = 128 - mantissa.leading_zeros();
    if width <= bits {
        return (mantissa, exponent);
    }
    let shift = width - bits;
    let half = 1u128 << (shift - 1);
    let remainder = mantissa & ((1u128 << shift) - 1);
    let mut rounded = mantissa >> shift;
    if remainder > half || (remainder == half && rounded & 1 == 1) {
        rounded += 1;
    }
    (rounded, exponent + shift as i32)
}

impl X87 {
    /// `FMUL`: multiply by an exactly representable value and round to 64 bits.
    pub(super) fn times(self, value: impl Into<f64>) -> Self {
        let (mantissa, exponent, negative) = decompose(value.into());
        let product = u128::from(self.mantissa) * u128::from(mantissa);
        let (mantissa, exponent) = round_to_bits(product, self.exponent + exponent, 64);
        Self {
            negative: self.negative != negative,
            mantissa: mantissa as u64,
            exponent,
        }
    }

    /// `_ftol`: truncate toward zero.
    pub(super) fn trunc(self) -> i32 {
        let magnitude: u128 = if self.exponent >= 0 {
            if self.exponent >= 64 {
                u128::MAX
            } else {
                u128::from(self.mantissa) << self.exponent
            }
        } else if -self.exponent >= 128 {
            0
        } else {
            u128::from(self.mantissa) >> (-self.exponent)
        };
        let magnitude = magnitude.min(i32::MAX as u128) as i32;
        if self.negative { -magnitude } else { magnitude }
    }

    /// `FSTP float`: round the 64-bit mantissa to 24 bits, nearest even.
    pub(super) fn to_f32(self) -> f32 {
        if self.mantissa == 0 {
            return 0.0;
        }
        let (mantissa, exponent) = round_to_bits(u128::from(self.mantissa), self.exponent, 24);
        let value = (mantissa as f64 * 2f64.powi(exponent)) as f32; // exact: 24-bit mantissa
        if self.negative { -value } else { value }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;

    /// Pin the 96 kHz decay start from the type's doc (1247, not f64's 1248) and the chained
    /// product behind `vib_interval`.
    #[test]
    fn products_round_to_64_bits_before_truncation() {
        assert_eq!(x87(96000.0f32).times(0.013).trunc(), 1247);
        assert_eq!((96000.0f64 * 0.013) as i32, 1248);
        assert_eq!(x87(44100.0f32).times(0.02).trunc(), 882);
        // chained: 44100 · 104 · 0.001 = 4586.4
        assert_eq!(x87(44100.0f32).times(104.0f32).times(0.001).trunc(), 4586);
    }

    #[test]
    fn trunc_goes_toward_zero_and_saturates() {
        assert_eq!(x87(2.75f32).trunc(), 2);
        assert_eq!(x87(-2.75f32).trunc(), -2);
        assert_eq!(x87(0.0f32).trunc(), 0);
        assert_eq!(x87(1e12f64).trunc(), i32::MAX);
    }

    #[test]
    fn to_f32_rounds_to_nearest_even() {
        assert_eq!(x87(0.1f32).to_f32(), 0.1f32);
        assert_eq!(x87(1.5f64).times(2.0).to_f32(), 3.0);
        assert_eq!(x87(-0.5f32).times(3.0f32).to_f32(), -1.5);
        // ties: 1 + 2^-24 rounds down to 1.0, 1 + 3·2^-24 rounds up to 1 + 2^-22
        assert_eq!(x87(1.0 + 2f64.powi(-24)).to_f32(), 1.0);
        assert_eq!(
            x87(1.0 + 3.0 * 2f64.powi(-24)).to_f32(),
            1.0 + 2f32.powi(-22)
        );
        assert_eq!(x87(0.0f64).to_f32(), 0.0);
    }

    #[test]
    fn round_to_bits_is_nearest_even() {
        assert_eq!(round_to_bits(0b1011, 0, 4), (0b1011, 0)); // fits
        assert_eq!(round_to_bits(0b10_1101, 0, 4), (0b1011, 2)); // below half: down
        assert_eq!(round_to_bits(0b10_1111, 0, 4), (0b1100, 2)); // above half: up
        assert_eq!(round_to_bits(0b10111, 0, 4), (0b1100, 1)); // tie, odd: up
        assert_eq!(round_to_bits(0b10101, 0, 4), (0b1010, 1)); // tie, even: down
    }

    #[test]
    fn decompose_splits_sign_mantissa_and_exponent() {
        assert_eq!(decompose(1.0), (1 << 52, -52, false));
        assert_eq!(decompose(-0.5), (1 << 52, -53, true));
        assert_eq!(decompose(0.0), (0, -1074, false));
    }
}
