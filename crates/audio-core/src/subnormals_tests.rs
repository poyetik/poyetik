//! Tests of the observation of subnormal arithmetic. The tests run in the default mode of
//! the processor (IEEE); `audio-fpenv`'s tests observe it under every flush mode forced.

use super::{SubnormalArithmetic, SubnormalsKept};

#[test]
fn observes_ieee_arithmetic_in_the_default_mode() {
    assert_eq!(
        SubnormalArithmetic::observe(),
        SubnormalArithmetic {
            binary32: SubnormalsKept {
                operands: true,
                results: true
            },
            binary64: SubnormalsKept {
                operands: true,
                results: true
            },
        }
    );
    assert!(SubnormalArithmetic::observe().is_ieee());
}

#[test]
fn is_ieee_only_when_every_subnormal_is_kept() {
    let kept = SubnormalsKept {
        operands: true,
        results: true,
    };
    for (operands, results) in [(false, true), (true, false), (false, false)] {
        let lost = SubnormalsKept { operands, results };
        assert!(
            !SubnormalArithmetic {
                binary32: lost,
                binary64: kept
            }
            .is_ieee(),
            "binary32 {lost:?}"
        );
        assert!(
            !SubnormalArithmetic {
                binary32: kept,
                binary64: lost
            }
            .is_ieee(),
            "binary64 {lost:?}"
        );
    }
}

/// `2^exponent` in binary64, by exact doublings or halvings of 1: an independent reference
/// for the probes' constants (exact down to the smallest subnormal).
fn power_of_two(exponent: i32) -> f64 {
    let step = if exponent < 0_i32 { 0.5_f64 } else { 2.0_f64 };
    (0..exponent.unsigned_abs()).fold(1.0_f64, |value, _| value * step)
}

#[test]
fn probes_with_the_exact_ieee_values() {
    assert_eq!(
        f64::from_bits(super::TWO_TO_60).to_bits(),
        power_of_two(60).to_bits()
    );
    assert_eq!(
        f64::from(f32::from_bits(super::TWO_TO_MINUS_125)).to_bits(),
        power_of_two(-125).to_bits()
    );
    assert_eq!(
        f64::from(f32::from_bits(super::TWO_TO_MINUS_128)).to_bits(),
        power_of_two(-128).to_bits()
    );
    assert_eq!(
        f64::from_bits(super::TWO_TO_MINUS_1014).to_bits(),
        power_of_two(-1014).to_bits()
    );
    assert_eq!(
        f64::from_bits(super::TWO_TO_MINUS_1024).to_bits(),
        power_of_two(-1024).to_bits()
    );
}
