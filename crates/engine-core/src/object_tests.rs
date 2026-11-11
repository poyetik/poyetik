//! Tests of the box contract: parameter specifications and algorithm versions.

use alloc::format;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    AlgorithmVersion, Create, Factory, Object, ObjectSpec, ParamError, ParamSpec, ParamValues,
    Params,
};
use crate::host::{ParamBounds, Raw as _};
use crate::{Binary64, Block, Sample, SampleRate};

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
    let params = ParamValues::of(
        &FIVE,
        &[2.5_f64, -below, below, -0.0_f64, 440.0_f64].map(Binary64::new),
    )
    .expect("within range");
    let bits: Vec<u64> = params
        .params()
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
    assert!(
        ParamValues::of(&NONE, &[])
            .expect("none")
            .params()
            .is_empty(),
        "none"
    );
}

#[test]
fn names_and_describes_a_parameter() {
    const GAIN: ParamSpec = ParamSpec::new("gain", "how loud", 0.5, 0.0, 1.0);
    assert_eq!(
        (GAIN.name(), GAIN.description()),
        ("gain", "how loud"),
        "its name and description"
    );
}

/// A box type that outputs nothing, for the specs of these tests.
#[derive(Debug)]
struct Mute;

impl<S: Sample> Object<S> for Mute {
    fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
}

impl Create for Mute {
    fn create<S: Sample>(_rate: SampleRate, _params: Params<'_>) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
}

/// A box type of two parameters: `a` from -1 to 2 (default 0.5), `b` from 0 to 20,000
/// (default 440).
static TWO: ObjectSpec = ObjectSpec {
    name: "two~",
    description: "two parameters",
    inlets: &[],
    outlets: &[],
    params: &[
        ParamSpec::new("a", "a", 0.5, -1.0, 2.0),
        ParamSpec::new("b", "b", 440.0, 0.0, 20_000.0),
    ],
    capabilities: &[],
    create: Factory::new::<Mute>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// [`TWO`] with one parameter whose default is below 2^-126.
static TINY: ObjectSpec = ObjectSpec {
    params: &[ParamSpec::new("t", "t", -1e-40, -1.0, 1.0)],
    ..TWO
};

/// [`TWO`] with five parameters, each accepting -1,000 to 1,000.
static FIVE: ObjectSpec = ObjectSpec {
    params: &[WIDE, WIDE, WIDE, WIDE, WIDE],
    ..TWO
};

/// A parameter accepting -1,000 to 1,000.
const WIDE: ParamSpec = ParamSpec::new("w", "w", 0.0, -1_000.0, 1_000.0);

/// [`TWO`] without parameters.
static NONE: ObjectSpec = ObjectSpec { params: &[], ..TWO };

/// The bits of `values`, as a box is given them.
fn bits(values: &ParamValues) -> Vec<u64> {
    values
        .params()
        .iter()
        .map(|value| value.raw().to_bits())
        .collect()
}

/// `values`, in binary64.
fn wide<const N: usize>(values: [f64; N]) -> [Binary64; N] {
    values.map(Binary64::new)
}

#[test]
fn takes_one_value_per_parameter_of_the_box_type() {
    for given in [0, 1, 3] {
        let values = vec![Binary64::new(0.0_f64); given];
        assert_eq!(
            ParamValues::of(&TWO, &values).map(|values| bits(&values)),
            Err(ParamError::Count {
                kind: "two~",
                expected: 2,
                given
            }),
            "{given} value(s)"
        );
    }
    assert_eq!(
        ParamValues::of(&TWO, &wide([0.25_f64, 1000.0_f64])).map(|values| bits(&values)),
        Ok(vec![0.25_f64.to_bits(), 1000.0_f64.to_bits()]),
        "two, in order"
    );
    assert_eq!(
        format!(
            "{}",
            ParamError::Count {
                kind: "two~",
                expected: 2,
                given: 3
            }
        ),
        "box type \"two~\" takes 2 parameter value(s), not 3",
        "said"
    );
}

#[test]
fn takes_values_within_their_range_only_flushed_first() {
    let subnormal = f64::from_bits(f64::from(f32::MIN_POSITIVE).to_bits() - 1);
    for (first, second) in [
        (-1.0_f64, 0.0_f64),
        (2.0_f64, 20_000.0_f64),
        (-0.0_f64, -subnormal),
        (subnormal, 1e-300_f64),
    ] {
        assert!(
            ParamValues::of(&TWO, &wide([first, second])).is_ok(),
            "{first} and {second}: within range once flushed"
        );
    }
    assert_eq!(
        ParamValues::of(&TWO, &wide([-subnormal, -1e-300_f64])).map(|values| bits(&values)),
        Ok(vec![0x8000_0000_0000_0000, 0x8000_0000_0000_0000]),
        "below 2^-126, zeros of their sign"
    );
    // The smallest normal binary32 value is not flushed: below a range starting at 0.
    let smallest = f64::from(f32::MIN_POSITIVE);
    for (first, second, name, refused) in [
        ((-1.0_f64).next_down(), 0.0_f64, "a", (-1.0_f64).next_down()),
        (2.0_f64.next_up(), 0.0_f64, "a", 2.0_f64.next_up()),
        (0.0_f64, -smallest, "b", -smallest),
        (0.0_f64, 20_000.0_f64.next_up(), "b", 20_000.0_f64.next_up()),
        (f64::NAN, 0.0_f64, "a", f64::NAN),
        (0.0_f64, f64::INFINITY, "b", f64::INFINITY),
        (f64::NEG_INFINITY, f64::NAN, "a", f64::NEG_INFINITY),
    ] {
        assert_eq!(
            ParamValues::of(&TWO, &wide([first, second])).map(|values| bits(&values)),
            Err(ParamError::OutOfRange {
                kind: "two~",
                name,
                bits: refused.to_bits()
            }),
            "{first} and {second}"
        );
    }
    assert_eq!(
        format!(
            "{}",
            ParamError::OutOfRange {
                kind: "two~",
                name: "b",
                bits: 20_000.5_f64.to_bits()
            }
        ),
        "parameter \"b\" of box type \"two~\": 20000.5 is outside its range",
        "said"
    );
}

#[test]
fn gives_the_defaults_of_the_box_type() {
    assert_eq!(
        bits(&ParamValues::defaults(&TWO)),
        [0.5_f64.to_bits(), 440.0_f64.to_bits()],
        "each default, in order"
    );
    assert_eq!(
        bits(&ParamValues::defaults(&TINY)),
        [0x8000_0000_0000_0000],
        "flushed as a plan flushes it"
    );
}
