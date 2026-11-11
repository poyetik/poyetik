//! Tests of the box contract: parameter specifications and algorithm versions.

use alloc::format;

use super::{AlgorithmVersion, ParamSpec, Params};
use crate::Binary64;
use crate::host::ParamBounds;

const PARAM: ParamSpec = ParamSpec::new("p", "p", 0.0, -1.0, 2.0);

#[test]
fn accepts_values_within_range_inclusive() {
    for value in [-1.0_f32, 0.0, 2.0] {
        assert!(PARAM.accepts(value), "{value} is accepted");
    }
}

#[test]
fn rejects_values_outside_range_and_nan() {
    for value in [-1.5_f32, 2.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!PARAM.accepts(value), "{value} is rejected");
    }
}

#[test]
fn gives_hosts_its_values_as_written() {
    let param = ParamSpec::new("q", "a q", 0.25, -1.5, 2.5);
    assert_eq!(
        (param.name, param.description),
        ("q", "a q"),
        "name and description"
    );
    assert_eq!(
        [param.default_value(), param.minimum(), param.maximum()].map(f32::to_bits),
        [0.25_f32, -1.5, 2.5].map(f32::to_bits),
        "default, minimum, maximum"
    );
}

#[test]
fn numbers_algorithm_versions_from_one() {
    let first = AlgorithmVersion::FIRST;
    assert_eq!(first.get(), 1, "the first version");
    assert_eq!(first.next().get(), 2, "the next one");
    assert_eq!(first.next().next().get(), 3, "and the one after");
    assert!(first < first.next(), "ordered");
    assert_eq!(format!("{}", first.next()), "2", "shown as its number");
    let last = AlgorithmVersion(core::num::NonZeroU32::MAX);
    assert_eq!(last.get(), u32::MAX, "the largest version");
    assert_eq!(
        last.next().get(),
        u32::MAX,
        "stays the largest, never wraps to zero"
    );
}

#[test]
fn gives_parameters_in_order_none_below_the_smallest_normal_binary32() {
    let below = f64::from_bits(f64::from(f32::MIN_POSITIVE).to_bits() - 1);
    let params = Params::new([2.5_f64, -below, below, -0.0_f64, 440.0_f64].map(Binary64::new));
    let bits: alloc::vec::Vec<u64> = params
        .iter()
        .map(|value| crate::host::Raw::raw(*value).to_bits())
        .collect();
    assert_eq!(
        bits,
        [
            2.5_f64.to_bits(),
            0x8000_0000_0000_0000,
            0,
            0x8000_0000_0000_0000,
            440.0_f64.to_bits(),
        ],
        "in order, every value kept but those below 2^-126, zeros of their sign"
    );
    assert!(Params::new([]).is_empty(), "none");
}
