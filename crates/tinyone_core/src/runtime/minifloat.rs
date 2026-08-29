//! Precision-rounding for TinyOne's reduced-precision float kinds.
//!
//! `Value::Float { kind, bits }` always stores a full `f64` — there is no
//! native, bit-packed `f16`/`f8` storage. Precision is enforced instead by
//! rounding to the nearest value representable in the target format
//! whenever a value is cast (`fp8(...)`, `fp16(...)`, `fp32(...)`) or an
//! arithmetic op produces a result of that kind. This keeps one storage and
//! dispatch path for every float kind and avoids depending on Rust's
//! unstable nightly `f16`/`f8` primitive types.

use crate::TypeKind;

/// Rounds `value` to the nearest `f32`-representable value. Native and
/// exact — Rust's `f32` cast already implements round-to-nearest-even.
pub(crate) fn round_to_fp32(value: f64) -> f64 {
    value as f32 as f64
}

/// Rounds `value` to the nearest IEEE-754 binary16 (half precision) value:
/// 1 sign + 5 exponent + 10 mantissa bits, bias 15. Overflow saturates to
/// ±65504 (the largest finite half-precision value) rather than producing
/// an infinity.
pub(crate) fn round_to_fp16(value: f64) -> f64 {
    round_to_minifloat(value, 15, 10, 65504.0)
}

/// Rounds `value` to the nearest OCP/Nvidia E4M3FN fp8 value: 1 sign + 4
/// exponent + 3 mantissa bits, bias 7. Overflow saturates to ±448 (the
/// largest finite E4M3FN value) rather than producing an infinity — E4M3FN
/// has no infinity encoding.
pub(crate) fn round_to_fp8_e4m3(value: f64) -> f64 {
    round_to_minifloat(value, 7, 3, 448.0)
}

/// Rounds `value` to the precision implied by `kind`. `Fp64` and any
/// non-float kind pass through unchanged (already full `f64` precision, or
/// not a float at all).
pub(crate) fn round_to_kind(value: f64, kind: TypeKind) -> f64 {
    match kind {
        TypeKind::Fp8 => round_to_fp8_e4m3(value),
        TypeKind::Fp16 => round_to_fp16(value),
        TypeKind::Fp32 => round_to_fp32(value),
        _ => value,
    }
}

/// Rounds `value` to the nearest value representable by an IEEE-754-style
/// minifloat with `mantissa_bits` mantissa bits and the given exponent
/// `bias`, saturating overflow to `±max_finite` instead of producing an
/// infinity.
///
/// Pure `f64` arithmetic (`log2`/`powi`/`round`/`clamp`) rather than bit
/// manipulation: decompose `value` into a power-of-two exponent and a
/// mantissa in `[1.0, 2.0)`, round the mantissa to `mantissa_bits`
/// fractional bits (or, below the normal range, to a fixed subnormal
/// exponent of `1 - bias`), then clamp the reconstructed magnitude to
/// `max_finite`. The final clamp alone implements "saturate on overflow"
/// correctly regardless of exactly which bit patterns a given standard
/// reserves for special values, since this never bit-packs the result.
fn round_to_minifloat(value: f64, bias: i32, mantissa_bits: i32, max_finite: f64) -> f64 {
    if value.is_nan() {
        return f64::NAN;
    }
    if value == 0.0 {
        return value; // preserves signed zero
    }
    if value.is_infinite() {
        return value.signum() * max_finite;
    }

    let sign = value.signum();
    let magnitude = value.abs();

    let mut exp = magnitude.log2().floor() as i32;
    let mut mantissa_scale = magnitude / 2f64.powi(exp);
    if mantissa_scale >= 2.0 {
        exp += 1;
        mantissa_scale /= 2.0;
    } else if mantissa_scale < 1.0 {
        exp -= 1;
        mantissa_scale *= 2.0;
    }

    let min_normal_exp = 1 - bias;
    let quantized = if exp < min_normal_exp {
        // Subnormal range: fixed exponent, mantissa has no implicit leading 1.
        let scale = 2f64.powi(mantissa_bits - min_normal_exp);
        (magnitude * scale).round() / scale
    } else {
        let scale = 2f64.powi(mantissa_bits);
        let mut rounded_mantissa = (mantissa_scale * scale).round() / scale;
        if rounded_mantissa >= 2.0 {
            // Mantissa rounded up into the next exponent bracket.
            rounded_mantissa = 1.0;
            exp += 1;
        }
        rounded_mantissa * 2f64.powi(exp)
    };

    (sign * quantized).clamp(-max_finite, max_finite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_round_trip_in_every_format() {
        for &value in &[1.0, 0.5, -2.0, 0.0, -0.0, 1.5, -1.25] {
            assert_eq!(round_to_fp32(value), value, "fp32 round-trip of {value}");
            assert_eq!(round_to_fp16(value), value, "fp16 round-trip of {value}");
            assert_eq!(round_to_fp8_e4m3(value), value, "fp8 round-trip of {value}");
        }
    }

    #[test]
    fn fp8_overflow_saturates_to_448() {
        assert_eq!(round_to_fp8_e4m3(448.0), 448.0);
        assert_eq!(round_to_fp8_e4m3(449.0), 448.0);
        assert_eq!(round_to_fp8_e4m3(1_000_000.0), 448.0);
        assert_eq!(round_to_fp8_e4m3(-1_000_000.0), -448.0);
    }

    #[test]
    fn fp16_overflow_saturates_to_65504() {
        assert_eq!(round_to_fp16(65504.0), 65504.0);
        assert_eq!(round_to_fp16(70000.0), 65504.0);
        assert_eq!(round_to_fp16(-70000.0), -65504.0);
    }

    #[test]
    fn nan_is_preserved_in_every_format() {
        assert!(round_to_fp32(f64::NAN).is_nan());
        assert!(round_to_fp16(f64::NAN).is_nan());
        assert!(round_to_fp8_e4m3(f64::NAN).is_nan());
    }

    #[test]
    fn infinite_input_saturates_to_max_finite() {
        assert_eq!(round_to_fp8_e4m3(f64::INFINITY), 448.0);
        assert_eq!(round_to_fp8_e4m3(f64::NEG_INFINITY), -448.0);
        assert_eq!(round_to_fp16(f64::INFINITY), 65504.0);
    }

    #[test]
    fn fp8_quantizes_imprecise_values() {
        // 1.0 + 0.0625 (2^-4) is not representable with only 3 mantissa
        // bits at exponent 0 (smallest step there is 2^-3 = 0.125), so it
        // must round to a representable neighbor, not pass through exactly.
        let rounded = round_to_fp8_e4m3(1.0625);
        assert_ne!(rounded, 1.0625);
        assert!((rounded - 1.0625).abs() <= 0.0625);
    }

    #[test]
    fn round_to_kind_dispatches_by_type_kind() {
        assert_eq!(round_to_kind(449.0, TypeKind::Fp8), 448.0);
        assert_eq!(round_to_kind(70000.0, TypeKind::Fp16), 65504.0);
        assert_eq!(round_to_kind(1.0 / 3.0, TypeKind::Fp64), 1.0 / 3.0);
        assert_eq!(round_to_kind(5.0, TypeKind::I64), 5.0);
    }

    #[test]
    fn subnormals_near_zero_do_not_panic_or_produce_nan() {
        let tiny = 1e-300;
        assert!(!round_to_fp8_e4m3(tiny).is_nan());
        assert!(!round_to_fp16(tiny).is_nan());
        assert!(round_to_fp8_e4m3(tiny).abs() <= tiny.max(1.0));
    }
}
