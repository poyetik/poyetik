//! Tests of the box contract: parameter specifications and algorithm versions.

use alloc::format;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    AlgorithmVersion, Create, Dispatch, Factory, Object, ObjectSpec, ParamError, ParamSpec,
    ParamValues, Params,
};
use crate::host::{ParamBounds, Raw as _};
use crate::{Binary32, Binary64, Block, Sample, SampleRate};

const PARAM: ParamSpec = crate::param!("p", "p", 0.0, -1.0, 2.0);

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
    let param = crate::param!("q", "a q", 0.25, -1.5, 2.5);
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
    const GAIN: ParamSpec = crate::param!("gain", "how loud", 0.5, 0.0, 1.0);
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
        crate::param!("a", "a", 0.5, -1.0, 2.0),
        crate::param!("b", "b", 440.0, 0.0, 20_000.0),
    ],
    capabilities: &[],
    create: Factory::new::<Mute>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// [`TWO`] with one parameter whose default is below 2^-126.
static TINY: ObjectSpec = ObjectSpec {
    params: &[crate::param!("t", "t", -1e-40, -1.0, 1.0)],
    ..TWO
};

/// [`TWO`] with five parameters, each accepting -1,000 to 1,000.
static FIVE: ObjectSpec = ObjectSpec {
    params: &[WIDE, WIDE, WIDE, WIDE, WIDE],
    ..TWO
};

/// A parameter accepting -1,000 to 1,000.
const WIDE: ParamSpec = crate::param!("w", "w", 0.0, -1_000.0, 1_000.0);

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
    for (first, second, kept) in [
        (-1.0_f64, 0.0_f64, [0xbff0_0000_0000_0000, 0]),
        (
            2.0_f64,
            20_000.0_f64,
            [0x4000_0000_0000_0000, 0x40d3_8800_0000_0000],
        ),
        (
            -0.0_f64,
            -subnormal,
            [0x8000_0000_0000_0000, 0x8000_0000_0000_0000],
        ),
        (subnormal, 1e-300_f64, [0, 0]),
    ] {
        assert_eq!(
            ParamValues::of(&TWO, &wide([first, second])).map(|values| bits(&values)),
            Ok(kept.to_vec()),
            "{first} and {second}: within range once flushed"
        );
    }
    assert_eq!(
        ParamValues::of(&TWO, &wide([-subnormal, -1e-300_f64])).map(|values| bits(&values)),
        Ok(vec![0x8000_0000_0000_0000, 0x8000_0000_0000_0000]),
        "below 2^-126, zeros of their sign"
    );
    // A binary64 value no binary32 parameter holds, 1 + 2^-40, keeps its bits.
    let between = f64::from_bits(0x3ff0_0000_0000_1000);
    assert_eq!(
        ParamValues::of(&TWO, &wide([between, 0.0_f64])).map(|values| bits(&values)),
        Ok(vec![0x3ff0_0000_0000_1000, 0]),
        "1 + 2^-40, kept"
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

/// What [`ParamSpec::new`] gives for a default `default_value` and the range `minimum` to
/// `maximum`: the bits of the three as kept, or the error.
fn checked(default_value: f32, minimum: f32, maximum: f32) -> Result<[u32; 3], ParamError> {
    ParamSpec::new("p", "p", default_value, minimum, maximum)
        .map(|param| [param.default_value, param.minimum, param.maximum].map(f32::to_bits))
}

#[test]
fn takes_a_default_within_its_finite_range_only() {
    for (default_value, minimum, maximum) in [
        (-1.0_f32, -1.0_f32, 2.0_f32),
        (2.0_f32, -1.0_f32, 2.0_f32),
        (0.5_f32, 0.5_f32, 0.5_f32),
        (-0.0_f32, 0.0_f32, 1.0_f32),
        (f32::MAX, f32::MIN, f32::MAX),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Ok([default_value, minimum, maximum].map(f32::to_bits)),
            "{default_value} in [{minimum}, {maximum}], kept as written"
        );
    }
    // Flushed, as a plan flushes it: a subnormal default is a zero of its sign, kept as
    // written for hosts.
    let subnormal = f32::from_bits(1);
    assert_eq!(
        checked(subnormal, 0.0_f32, 1.0_f32),
        Ok([1, 0, 1.0_f32.to_bits()]),
        "a subnormal default, zero once flushed, in a range from 0"
    );
    assert_eq!(
        checked(-subnormal, -1.0_f32, 0.0_f32),
        Ok([0x8000_0001, (-1.0_f32).to_bits(), 0]),
        "a negative one, -0 once flushed, in a range up to 0"
    );
    for (default_value, minimum, maximum) in [
        ((-1.0_f32).next_down(), -1.0_f32, 2.0_f32),
        (2.0_f32.next_up(), -1.0_f32, 2.0_f32),
        (f32::NAN, -1.0_f32, 2.0_f32),
        (subnormal, f32::from_bits(2), 1.0_f32),
        (-subnormal, -1.0_f32, -f32::from_bits(2)),
        (f32::INFINITY, -1.0_f32, 2.0_f32),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Err(ParamError::Default { name: "p" }),
            "{default_value} outside [{minimum}, {maximum}] once flushed"
        );
    }
    for (minimum, maximum) in [
        (f32::NEG_INFINITY, 1.0_f32),
        (0.0_f32, f32::INFINITY),
        (f32::NAN, 1.0_f32),
        (0.0_f32, f32::NAN),
        (1.0_f32, 1.0_f32.next_down()),
    ] {
        assert_eq!(
            checked(1.0_f32, minimum, maximum),
            Err(ParamError::Range { name: "p" }),
            "[{minimum}, {maximum}]"
        );
    }
    assert_eq!(
        format!("{}", ParamError::Default { name: "gain" }),
        "parameter \"gain\": its default is outside its range",
        "said"
    );
    assert_eq!(
        format!("{}", ParamError::Range { name: "gain" }),
        "parameter \"gain\": its range is not finite, or its minimum is above its maximum",
        "said"
    );
}

#[test]
fn keeps_the_box_type_its_values_were_checked_against() {
    let given = ParamValues::of(&TWO, &wide([0.25_f64, 1000.0_f64])).expect("within range");
    assert!(core::ptr::eq(given.spec(), &raw const TWO), "of");
    assert!(
        core::ptr::eq(ParamValues::defaults(&FIVE).spec(), &raw const FIVE),
        "defaults"
    );
    assert!(
        core::ptr::eq(ParamValues::flushed(&NONE, vec![]).spec(), &raw const NONE),
        "flushed"
    );
}

/// A box type whose instance outputs its first parameter value then the sample rate in
/// hertz, frame by frame, so that a test sees what it was created with.
#[derive(Debug)]
struct Told([f64; 2]);

impl<S: Sample> Object<S> for Told {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs {
            for (sample, &value) in output.iter_mut().zip(self.0.iter()) {
                *sample = S::from_binary64(Binary64::new(value));
            }
        }
    }
}

impl Create for Told {
    fn create<S: Sample>(rate: SampleRate, params: Params<'_>) -> Box<dyn Object<S>> {
        let first = params.first().map_or(-1.0_f64, |value| value.raw());
        Box::new(Self([first, f64::from(rate.hz())]))
    }
}

/// The box type of [`Told`], of one parameter from 0 to 1,000.
static TOLD: ObjectSpec = ObjectSpec {
    name: "told~",
    params: &[crate::param!("v", "v", 1.0, 0.0, 1_000.0)],
    create: Factory::new::<Told>(),
    ..TWO
};

/// The first two frames of the instance computing in `S` that `values` create, at 8 kHz,
/// through the virtual table.
fn created<S: crate::host::Raw>(values: &ParamValues) -> [u64; 2] {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut out = [S::SILENCE];
    <Box<dyn Object<S>> as Dispatch<S>>::create(rate, values).process(&[], &mut out);
    let [first, second, ..] = out[0];
    [first.bits(), second.bits()]
}

#[test]
fn creates_the_box_type_of_its_values() {
    let given = ParamValues::of(&TOLD, &wide([250.0_f64])).expect("within range");
    assert_eq!(
        created::<Binary64>(&given),
        [250.0_f64.to_bits(), 8_000.0_f64.to_bits()],
        "binary64: its factory, its value and the rate"
    );
    assert_eq!(
        created::<Binary32>(&given),
        [250.0_f32.to_bits(), 8_000.0_f32.to_bits()].map(u64::from),
        "binary32"
    );
    assert_eq!(
        created::<Binary64>(&ParamValues::defaults(&TOLD)),
        [1.0_f64.to_bits(), 8_000.0_f64.to_bits()],
        "its default"
    );
}

#[test]
fn is_an_error_of_its_own() {
    let error: Box<dyn core::error::Error> = Box::new(ParamError::Default { name: "gain" });
    assert!(error.source().is_none(), "no cause of its own");
    assert_eq!(
        format!("{error}"),
        "parameter \"gain\": its default is outside its range",
        "said as it displays"
    );
}
