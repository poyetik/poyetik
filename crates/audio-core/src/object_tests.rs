//! Tests of the box contract: parameter specifications and algorithm versions.

use alloc::format;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    AlgorithmVersion, Capability, Create, Dispatch, Object, ObjectSpec, ParamError, ParamSpec,
    ParamSpecError, ParamValues, Params, PortSpec,
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
fn accepts_a_value_below_the_smallest_normal_binary32_as_the_zero_it_is() {
    let from_zero = crate::param!("z", "z", 0.5, 0.0, 1.0);
    let to_zero = crate::param!("z", "z", -0.5, -1.0, 0.0);
    for bits in [0x0000_0001_u32, 0x0001_16c2, 0x007f_ffff] {
        for sign in [0_u32, 0x8000_0000] {
            let value = f32::from_bits(sign | bits);
            assert!(from_zero.accepts(value), "{value:e} in [0, 1]");
            assert!(to_zero.accepts(value), "{value:e} in [-1, 0]");
        }
    }
    let above = crate::param!("a", "a", 0.5, f32::MIN_POSITIVE, 1.0);
    assert!(above.accepts(f32::MIN_POSITIVE), "the smallest normal");
    assert!(
        !above.accepts(f32::from_bits(0x007f_ffff)),
        "the largest subnormal is zero"
    );
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

/// A box type of the parameters `$params` that outputs nothing, `$name`, for the specs of
/// these tests.
macro_rules! mute {
    ($name:ident, $params:expr) => {
        /// A box type that outputs nothing, of its own parameters.
        #[derive(Debug)]
        struct $name;

        impl<S: Sample, const N: usize> Object<S, N> for $name {
            fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
        }

        impl Create for $name {
            const PARAMS: &'static [ParamSpec] = $params;

            fn create<S: Sample, const N: usize>(
                _rate: SampleRate,
                _params: Params<'_>,
            ) -> Box<dyn Object<S, N>> {
                Box::new(Self)
            }
        }
    };
}

/// The spec `name` of the box type `T`, without ports.
const fn mute<T: Create>(name: &'static str) -> ObjectSpec {
    ObjectSpec::new::<T>(name, "parameters", &[], &[], &[], AlgorithmVersion::FIRST)
}

mute!(
    Two,
    &[
        crate::param!("a", "a", 0.5, -1.0, 2.0),
        crate::param!("b", "b", 440.0, 0.0, 20_000.0),
    ]
);
mute!(Tiny, &[crate::param!("t", "t", -1e-40, -1.0, 1.0)]);
mute!(Five, &[WIDE, WIDE, WIDE, WIDE, WIDE]);
mute!(Silent, &[]);

/// A box type of two parameters: `a` from -1 to 2 (default 0.5), `b` from 0 to 20,000
/// (default 440).
static TWO: ObjectSpec = mute::<Two>("two~");

/// A box type of one parameter whose default is below 2^-126.
static TINY: ObjectSpec = mute::<Tiny>("tiny~");

/// A box type of five parameters, each accepting -1,000 to 1,000.
static FIVE: ObjectSpec = mute::<Five>("five~");

/// A parameter accepting -1,000 to 1,000.
const WIDE: ParamSpec = crate::param!("w", "w", 0.0, -1_000.0, 1_000.0);

/// A box type without parameters.
static NONE: ObjectSpec = mute::<Silent>("none~");

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
        let (minimum, maximum) = if name == "a" {
            (-1.0_f32, 2.0_f32)
        } else {
            (0.0_f32, 20_000.0_f32)
        };
        assert_eq!(
            ParamValues::of(&TWO, &wide([first, second])).map(|values| bits(&values)),
            Err(ParamError::OutOfRange {
                kind: "two~",
                name,
                bits: refused.to_bits(),
                minimum,
                maximum,
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
                bits: 20_000.5_f64.to_bits(),
                minimum: -0.5,
                maximum: 20_000.0,
            }
        ),
        "parameter \"b\" of box type \"two~\": 20000.5 is outside [-0.5, 20000]",
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
fn checked(default_value: f32, minimum: f32, maximum: f32) -> Result<[u32; 3], ParamSpecError> {
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
        (subnormal, f32::MIN_POSITIVE, 1.0_f32),
        (-subnormal, -1.0_f32, -f32::MIN_POSITIVE),
        (f32::INFINITY, -1.0_f32, 2.0_f32),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Err(ParamSpecError::DefaultOutsideRange { name: "p" }),
            "{default_value} outside [{minimum}, {maximum}] once flushed"
        );
    }
    // A nonzero bound below 2^-126, which a plan would flush to zero: no value can reach it.
    for bound in [0x0000_0001_u32, 0x0001_16c2, 0x007f_ffff] {
        let positive = f32::from_bits(bound);
        let negative = f32::from_bits(0x8000_0000 | bound);
        for (minimum, maximum) in [
            (positive, 1.0_f32),
            (-1.0_f32, positive),
            (negative, 1.0_f32),
            (-1.0_f32, negative),
            (negative, positive),
        ] {
            assert_eq!(
                checked(maximum, minimum, maximum),
                Err(ParamSpecError::InvalidRange { name: "p" }),
                "[{minimum:e}, {maximum:e}]"
            );
        }
    }
    for (default_value, minimum, maximum) in [
        (f32::MIN_POSITIVE, f32::MIN_POSITIVE, 1.0_f32),
        (-1.0_f32, -1.0_f32, -f32::MIN_POSITIVE),
        (0.0_f32, -0.0_f32, 0.0_f32),
        (0.0_f32, 0.0_f32, -0.0_f32),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Ok([default_value, minimum, maximum].map(f32::to_bits)),
            "zeros and the smallest normal are bounds: [{minimum:e}, {maximum:e}]"
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
            Err(ParamSpecError::InvalidRange { name: "p" }),
            "[{minimum}, {maximum}]"
        );
    }
    assert_eq!(
        format!("{}", ParamSpecError::DefaultOutsideRange { name: "gain" }),
        "parameter \"gain\": its default, flushed, is outside its range",
        "said"
    );
    assert_eq!(
        format!("{}", ParamSpecError::InvalidRange { name: "gain" }),
        "parameter \"gain\": its range is not finite, its minimum is above its maximum, or a bound \
         is below 2^-126 but not zero",
        "said"
    );
}

#[test]
fn takes_each_value_picked_its_range_accepts_else_the_default() {
    // `a` accepts -1 to 2 (default 0.5), `b` 0 to 20,000 (default 440).
    let picked = |pick: &dyn Fn(&ParamSpec) -> f32| bits(&ParamValues::within(&TWO, pick));
    assert_eq!(
        picked(&|param| if param.name() == "a" { -0.0 } else { 3.0e4 }),
        [0x8000_0000_0000_0000, 440.0_f64.to_bits()],
        "-0 kept bit for bit, 30,000 beyond b's range: its default"
    );
    assert_eq!(
        picked(&|_| f32::MIN_POSITIVE),
        [f64::from(f32::MIN_POSITIVE).to_bits(); 2],
        "the smallest normal, within both"
    );
    assert_eq!(
        picked(&|_| -f32::MIN_POSITIVE),
        [f64::from(-f32::MIN_POSITIVE).to_bits(), 440.0_f64.to_bits()],
        "below b's range: its default"
    );
    assert_eq!(
        picked(&|_| -1e-40),
        [0x8000_0000_0000_0000; 2],
        "a subnormal is a zero of its sign, which both accept"
    );
    assert_eq!(
        picked(&|_| f32::NAN),
        [0.5_f64.to_bits(), 440.0_f64.to_bits()],
        "NaN: the defaults"
    );
    assert_eq!(
        bits(&ParamValues::within(&TINY, |_| 2.0)),
        [0x8000_0000_0000_0000],
        "a default below 2^-126, flushed"
    );
    assert!(
        core::ptr::eq(ParamValues::within(&TWO, |_| 0.0).spec(), &raw const TWO),
        "of its box type"
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

impl<S: Sample, const N: usize> Object<S, N> for Told {
    fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        for output in outputs {
            for (sample, &value) in output.iter_mut().zip(self.0.iter()) {
                *sample = S::from_binary64(Binary64::new(value));
            }
        }
    }
}

impl Create for Told {
    const PARAMS: &'static [ParamSpec] = &[crate::param!("v", "v", 1.0, 0.0, 1_000.0)];

    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        params: Params<'_>,
    ) -> Box<dyn Object<S, N>> {
        let first = params.first().map_or(-1.0_f64, |value| value.raw());
        Box::new(Self([first, f64::from(rate.hz())]))
    }
}

/// The box type of [`Told`], of one parameter from 0 to 1,000.
static TOLD: ObjectSpec = mute::<Told>("told~");

/// The first two frames of the instance computing in `S` that `values` create, at 8 kHz,
/// through the virtual table.
fn created<S: crate::host::Raw>(values: &ParamValues) -> [u64; 2] {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut out = [S::SILENCE];
    <Box<dyn Object<S>> as Dispatch<S>>::create(rate, values)
        .expect("a test build holds every engine block")
        .process(&[], &mut out);
    let [first, second, ..] = out[0];
    [first.bits(), second.bits()]
}

#[test]
fn takes_its_parameters_and_instances_from_its_type() {
    const IN: PortSpec = PortSpec {
        name: "in",
        description: "a signal",
    };
    const OUT: PortSpec = PortSpec {
        name: "out",
        description: "the signal",
    };
    // Built at run time, not in a static, so that the constructor runs in the tests.
    let spec: &'static ObjectSpec = Box::leak(Box::new(ObjectSpec::new::<Told>(
        "told2~",
        "tells its value",
        &[IN],
        &[OUT, IN],
        &[Capability::AudioOutput],
        AlgorithmVersion::FIRST.next(),
    )));
    assert_eq!(
        (spec.name(), spec.description()),
        ("told2~", "tells its value"),
        "name and description as given"
    );
    assert_eq!(
        (spec.inlets(), spec.outlets()),
        (&[IN][..], &[OUT, IN][..]),
        "ports as given, in order"
    );
    assert_eq!(
        spec.capabilities(),
        [Capability::AudioOutput],
        "capabilities"
    );
    assert_eq!(spec.algorithm_version().get(), 2, "its version");
    assert_eq!(
        spec.params(),
        [crate::param!("v", "v", 1.0, 0.0, 1_000.0)],
        "the parameters of its type"
    );
    assert_eq!(
        created::<Binary64>(&ParamValues::of(spec, &wide([250.0_f64])).expect("within range")),
        [250.0_f64.to_bits(), 8_000.0_f64.to_bits()],
        "the instances of its type"
    );
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
fn are_errors_of_their_own() {
    let errors: [Box<dyn core::error::Error>; 2] = [
        Box::new(ParamSpecError::DefaultOutsideRange { name: "gain" }),
        Box::new(ParamError::Count {
            kind: "two~",
            expected: 2,
            given: 1,
        }),
    ];
    for error in &errors {
        assert!(error.source().is_none(), "{error}: no cause of its own");
    }
    assert_eq!(
        errors.map(|error| format!("{error}")),
        [
            "parameter \"gain\": its default, flushed, is outside its range",
            "box type \"two~\" takes 2 parameter value(s), not 1",
        ],
        "said as they display"
    );
}

#[test]
fn constructs_every_engine_block_offered() {
    // The factory's constructors cover exactly the engine blocks a build offers, in order:
    // a size added to one list and not the other fails here.
    assert_eq!(
        super::CONSTRUCTED,
        crate::ENGINE_BLOCKS,
        "the factory's sizes"
    );
}
