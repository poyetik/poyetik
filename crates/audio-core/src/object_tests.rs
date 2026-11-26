//! Tests of the box contract: parameter specifications and algorithm versions.

use alloc::format;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::{
    AlgorithmVersion, BoxInstance, Capability, Create, Dispatch, Driven, DrivenValues, Object,
    ObjectSpec, ParamError, ParamSpec, ParamSpecError, ParamValues, PortSpec, Typed,
};
use crate::host::{ParamBounds, Raw as _};
use crate::{Binary32, Binary64, Block, Sample, SampleRate};

const PARAM: ParamSpec = crate::param!("p", "p", 0.0, -1.0, 2.0);

/// 2^-126, the smallest normal binary32 value, below which a parameter is a zero of its sign
/// (`spec/engine-design.md`, decisions 5 and 11), written as a literal so that a `const`
/// spec can take it.
const SMALLEST: f64 = 1.175_494_350_822_287_5e-38;

#[test]
fn writes_the_smallest_normal_binary32_value_exactly() {
    assert_eq!(
        SMALLEST.to_bits(),
        f64::from(f32::MIN_POSITIVE).to_bits(),
        "2^-126"
    );
}

#[test]
fn accepts_values_within_range_inclusive() {
    for value in [-1.0_f64, 0.0_f64, 2.0_f64, 1.0_f64 + f64::EPSILON] {
        assert!(PARAM.accepts(value), "{value} is accepted");
    }
}

#[test]
fn accepts_a_value_below_the_smallest_normal_binary32_as_the_zero_it_is() {
    let from_zero = crate::param!("z", "z", 0.5, 0.0, 1.0);
    let to_zero = crate::param!("z", "z", -0.5, -1.0, 0.0);
    for magnitude in [
        f64::from_bits(1),
        1e-300_f64,
        f64::from(f32::from_bits(1)),
        f64::from(f32::from_bits(0x007f_ffff)),
        SMALLEST.next_down(),
    ] {
        for value in [magnitude, -magnitude] {
            assert!(from_zero.accepts(value), "{value:e} in [0, 1]");
            assert!(to_zero.accepts(value), "{value:e} in [-1, 0]");
        }
    }
    let above = crate::param!("a", "a", 0.5, SMALLEST, 1.0);
    assert!(above.accepts(SMALLEST), "the smallest normal binary32");
    assert!(
        !above.accepts(SMALLEST.next_down()),
        "the largest binary64 below it is zero"
    );
}

#[test]
fn rejects_values_outside_range_and_nan() {
    for value in [
        (-1.0_f64).next_down(),
        2.0_f64.next_up(),
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(!PARAM.accepts(value), "{value} is rejected");
    }
}

#[test]
fn gives_hosts_its_values_as_written() {
    let param = crate::param!("q", "a q", 0.1, -1.5, 2.5e300);
    assert_eq!(
        (param.name, param.description),
        ("q", "a q"),
        "name and description"
    );
    assert_eq!(
        [param.default_value(), param.minimum(), param.maximum()].map(f64::to_bits),
        [
            0x3fb9_9999_9999_999a,
            0xbff8_0000_0000_0000,
            0x7e4d_dd4b_aa00_9303
        ],
        "default, minimum, maximum, in binary64"
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
    ($name:ident, $count:literal, $params:expr) => {
        /// A box type that outputs nothing, of its own parameters.
        #[derive(Debug)]
        struct $name;

        impl<S: Sample, const N: usize> Object<S, N> for $name {
            fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
        }

        impl Create<$count> for $name {
            const BOX_MODE: crate::BoxMode = crate::BoxMode::LiveCapable;
            const PARAMS: &'static [ParamSpec; $count] = $params;

            fn create<S: Sample, const N: usize>(
                _rate: SampleRate,
                _params: [Binary64; $count],
            ) -> impl Driven<S, N, $count> + 'static {
                Self
            }
        }
    };
}

/// What the box type `$name` of `$count` parameters, which outputs nothing, does when they
/// are driven: nothing.
macro_rules! mute_driven {
    ($name:ident, $count:literal) => {
        impl<S: Sample, const N: usize> Driven<S, N, $count> for $name {
            fn set(&mut self, _params: [Binary64; $count]) {}

            fn process_driven(
                &mut self,
                _inputs: &[Block<S, N>],
                _outputs: &mut [Block<S, N>],
                _params: &[Block<Binary64, N>; $count],
            ) {
            }
        }
    };
}
mute_driven!(Two, 2);
mute_driven!(Tiny, 1);
mute_driven!(Five, 5);
mute_driven!(Far, 1);

/// The spec `name` of the box type `T`, without ports.
const fn mute<T: Create<K>, const K: usize>(name: &'static str) -> ObjectSpec {
    ObjectSpec::new::<T, K>(name, "parameters", &[], &[], &[], AlgorithmVersion::FIRST)
}

mute!(
    Two,
    2,
    &[
        crate::param!("a", "a", 0.5, -1.0, 2.0),
        crate::param!("b", "b", 440.0, 0.0, 20_000.0),
    ]
);
mute!(Tiny, 1, &[crate::param!("t", "t", -1e-40, -1.0, 1.0)]);
mute!(Five, 5, &[WIDE, WIDE, WIDE, WIDE, WIDE]);
mute!(Far, 1, &[crate::param!("r", "r", 1.0, 1.0, 1e300)]);
mute!(Silent, 0, &[]);

/// A box type of two parameters: `a` from -1 to 2 (default 0.5), `b` from 0 to 20,000
/// (default 440).
static TWO: ObjectSpec = mute::<Two, 2>("two~");

/// A box type of one parameter whose default is below 2^-126.
static TINY: ObjectSpec = mute::<Tiny, 1>("tiny~");

/// A box type of one parameter from 1 to 1e300, whose range holds no value below 2^-126.
static FAR: ObjectSpec = mute::<Far, 1>("far~");

/// A box type of five parameters, each accepting -1,000 to 1,000.
static FIVE: ObjectSpec = mute::<Five, 5>("five~");

/// A parameter accepting -1,000 to 1,000.
const WIDE: ParamSpec = crate::param!("w", "w", 0.0, -1_000.0, 1_000.0);

/// A box type without parameters.
static NONE: ObjectSpec = mute::<Silent, 0>("none~");

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
            (-1.0_f64, 2.0_f64)
        } else {
            (0.0_f64, 20_000.0_f64)
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
                minimum: -0.5_f64,
                maximum: 20_000.0_f64,
            }
        ),
        "parameter \"b\" of box type \"two~\": 20000.5 is not within [-0.5, 20000.0]",
        "said"
    );
}

/// The refusals as a user reads them: the value as it was given (below 2^-126, what it is
/// flushed to would be another), a NaN, which is outside nothing, a wide range in its
/// shortest form, and, for driven values, blocks counted as blocks and the frame named.
#[test]
fn says_each_refusal_as_it_was_given() {
    let nothing = [Binary64::ZERO; 64];
    let mut block = [Binary64::new(0.25); 64];
    // Two frames refused: the first in frame order is named, whatever their values.
    block[40] = Binary64::new(f64::NAN);
    block[41] = Binary64::new(-3.0);
    for (said, error) in [
        (
            "box type \"two~\" takes 2 parameter value(s), not 1",
            ParamValues::of(&TWO, &[Binary64::ZERO]).map(|_| ()),
        ),
        (
            "parameter \"a\" of box type \"two~\": NaN is not within [-1.0, 2.0]",
            ParamValues::of(&TWO, &[Binary64::new(f64::NAN), Binary64::ZERO]).map(|_| ()),
        ),
        (
            "parameter \"r\" of box type \"far~\": 1e-40 is not within [1.0, 1e300]",
            ParamValues::of(&FAR, &[Binary64::new(1e-40)]).map(|_| ()),
        ),
        (
            "box type \"two~\" takes 2 block(s) of values, one per parameter, not 3",
            DrivenValues::of(&TWO, vec![nothing; 3]).map(|_| ()),
        ),
        (
            "parameter \"a\" of box type \"two~\", frame 40: NaN is not within [-1.0, 2.0]",
            DrivenValues::of(&TWO, vec![block, nothing]).map(|_| ()),
        ),
        (
            "parameter \"r\" of box type \"far~\", frame 0: 1e-40 is not within [1.0, 1e300]",
            DrivenValues::of(&FAR, vec![[Binary64::new(1e-40); 64]]).map(|_| ()),
        ),
    ] {
        assert_eq!(
            error.map_err(|refused| format!("{refused}")),
            Err(alloc::string::String::from(said))
        );
    }
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
fn checked(default_value: f64, minimum: f64, maximum: f64) -> Result<[u64; 3], ParamSpecError> {
    ParamSpec::new("p", "p", default_value, minimum, maximum)
        .map(|param| [param.default_value, param.minimum, param.maximum].map(f64::to_bits))
}

#[test]
fn takes_a_default_within_its_finite_range_only() {
    for (default_value, minimum, maximum) in [
        (-1.0_f64, -1.0_f64, 2.0_f64),
        (2.0_f64, -1.0_f64, 2.0_f64),
        (0.5_f64, 0.5_f64, 0.5_f64),
        (-0.0_f64, 0.0_f64, 1.0_f64),
        (0.1_f64, 0.0_f64, 1.0_f64),
        (f64::MAX, 0.0_f64, f64::MAX),
        (f64::MIN, f64::MIN, 0.0_f64),
        (f64::MAX, f64::MAX, f64::MAX),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Ok([default_value, minimum, maximum].map(f64::to_bits)),
            "{default_value} in [{minimum}, {maximum}], kept as written"
        );
    }
    // Flushed, as a plan flushes it: a default below 2^-126 is a zero of its sign, kept as
    // written for hosts.
    let below = SMALLEST.next_down();
    assert_eq!(
        checked(below, 0.0_f64, 1.0_f64),
        Ok([below.to_bits(), 0, 1.0_f64.to_bits()]),
        "a default below 2^-126, zero once flushed, in a range from 0"
    );
    assert_eq!(
        checked(-1e-300, -1.0_f64, 0.0_f64),
        Ok([(-1e-300_f64).to_bits(), (-1.0_f64).to_bits(), 0]),
        "a negative one, -0 once flushed, in a range up to 0"
    );
    for (default_value, minimum, maximum) in [
        ((-1.0_f64).next_down(), -1.0_f64, 2.0_f64),
        (2.0_f64.next_up(), -1.0_f64, 2.0_f64),
        (f64::NAN, -1.0_f64, 2.0_f64),
        (below, SMALLEST, 1.0_f64),
        (-below, -1.0_f64, -SMALLEST),
        (f64::INFINITY, -1.0_f64, 2.0_f64),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Err(ParamSpecError::DefaultOutsideRange { name: "p" }),
            "{default_value} outside [{minimum}, {maximum}] once flushed"
        );
    }
}

#[test]
fn takes_a_finite_range_in_order_of_reachable_bounds_only() {
    let below = SMALLEST.next_down();
    // A nonzero bound below 2^-126, which a plan would flush to zero: no value can reach it.
    for positive in [
        f64::from_bits(1),
        1e-300_f64,
        f64::from(f32::from_bits(1)),
        below,
    ] {
        let negative = -positive;
        for (minimum, maximum) in [
            (positive, 1.0_f64),
            (-1.0_f64, positive),
            (negative, 1.0_f64),
            (-1.0_f64, negative),
            (negative, positive),
        ] {
            assert_eq!(
                checked(maximum, minimum, maximum),
                Err(ParamSpecError::Unreachable { name: "p" }),
                "[{minimum:e}, {maximum:e}]"
            );
        }
        // Before the order, which a host taking subnormal numbers as zeros would not see.
        for (minimum, maximum) in [(1.0_f64, positive), (positive, negative)] {
            assert_eq!(
                checked(0.0_f64, minimum, maximum),
                Err(ParamSpecError::Unreachable { name: "p" }),
                "[{minimum:e}, {maximum:e}]"
            );
        }
    }
    for (default_value, minimum, maximum) in [
        (SMALLEST, SMALLEST, 1.0_f64),
        (-1.0_f64, -1.0_f64, -SMALLEST),
        (0.0_f64, -0.0_f64, 0.0_f64),
        (0.0_f64, 0.0_f64, -0.0_f64),
    ] {
        assert_eq!(
            checked(default_value, minimum, maximum),
            Ok([default_value, minimum, maximum].map(f64::to_bits)),
            "zeros and the smallest normal binary32 are bounds: [{minimum:e}, {maximum:e}]"
        );
    }
    for (minimum, maximum) in [
        (f64::NEG_INFINITY, 1.0_f64),
        (0.0_f64, f64::INFINITY),
        (f64::NAN, 1.0_f64),
        (0.0_f64, f64::NAN),
        (f64::INFINITY, 0.0_f64),
    ] {
        assert_eq!(
            checked(1.0_f64, minimum, maximum),
            Err(ParamSpecError::NotFinite { name: "p" }),
            "[{minimum}, {maximum}]"
        );
    }
    for (minimum, maximum) in [
        (1.0_f64, 1.0_f64.next_down()),
        (1.0_f64, -1.0_f64),
        (f64::MAX, f64::MIN),
        (1.0_f64, 0.0_f64),
    ] {
        assert_eq!(
            checked(1.0_f64, minimum, maximum),
            Err(ParamSpecError::Reversed { name: "p" }),
            "[{minimum}, {maximum}]"
        );
    }
}

/// A width is finite or refused at its exact edge, in the order of the rules.
#[test]
fn takes_a_range_of_finite_width_only() {
    // A finite range whose width, the maximum less the minimum, is beyond binary64: from
    // -2^1023 to 2^1023 (width 2^1024) the smallest, the whole of binary64 the largest.
    let half = f64::from_bits(0x7fe0_0000_0000_0000);
    for (minimum, maximum) in [(-half, half), (f64::MIN, f64::MAX), (-1e308_f64, 1e308_f64)] {
        assert_eq!(
            checked(0.0_f64, minimum, maximum),
            Err(ParamSpecError::InfiniteWidth { name: "p" }),
            "[{minimum:e}, {maximum:e}]"
        );
    }
    // From -2^1023 to the binary64 below 2^1023 the width, 2^1024 - 2^970, is a tie between
    // the largest finite binary64 and 2^1024, rounded to even: infinite. One binary64
    // lower, it is the largest finite binary64.
    let (below_half, lower) = (half.next_down(), half.next_down().next_down());
    assert_eq!(
        checked(0.0_f64, -half, below_half),
        Err(ParamSpecError::InfiniteWidth { name: "p" }),
        "a width that rounds to 2^1024"
    );
    assert_eq!(
        checked(0.0_f64, -half, lower),
        Ok([0, (-half).to_bits(), lower.to_bits()]),
        "the widest range, its width the largest finite binary64"
    );
    assert_eq!(
        checked(0.0_f64, -1.0_f64, f64::MAX),
        Ok([0, (-1.0_f64).to_bits(), f64::MAX.to_bits()]),
        "a width that rounds down to the largest finite binary64"
    );
    // The largest binary64, an odd multiple of 2^971: 2^970 from zero is a tie again, and
    // rounds to 2^1024, whatever is added below 2^970.
    let tie = f64::from_bits(0x7c90_0000_0000_0000);
    for (other, infinite) in [
        (tie, true),
        (tie.next_up(), true),
        (tie.next_down(), false),
        (SMALLEST, false),
        (0.0_f64, false),
    ] {
        let (under, over) = (
            checked(0.0_f64, -other, f64::MAX),
            checked(0.0_f64, f64::MIN, other),
        );
        if infinite {
            let refused = Err(ParamSpecError::InfiniteWidth { name: "p" });
            assert_eq!((under, over), (refused, refused), "{other:e}");
        } else {
            assert_eq!(
                (under, over),
                (
                    Ok([0, (-other).to_bits(), f64::MAX.to_bits()]),
                    Ok([0, f64::MIN.to_bits(), other.to_bits()])
                ),
                "{other:e}"
            );
        }
    }
    assert_eq!(
        checked(f64::NAN, f64::NAN, 1.0_f64),
        Err(ParamSpecError::NotFinite { name: "p" }),
        "the range refused first"
    );
    assert_eq!(
        checked(f64::NAN, f64::MIN, f64::MAX),
        Err(ParamSpecError::InfiniteWidth { name: "p" }),
        "the width before the default"
    );
}

/// The width decided in integers is the subtraction of a host that rounds to nearest, as a
/// test's does, over the edges of every step it takes and over drawn bounds.
#[test]
fn decides_the_width_as_the_subtraction_rounded_to_nearest() {
    let mut bounds = vec![0.0_f64, SMALLEST, 1.0_f64, f64::MAX];
    // Each exponent near the two shifts' ends and the halfway's, and the largest, with the
    // edges of its significands.
    for exponent in (1_988_u64..=1_998).chain(2_040..=2_046) {
        for fraction in [
            0,
            1,
            2,
            0x7_ffff_ffff_ffff,
            0x8_0000_0000_0000,
            0xf_ffff_ffff_fffe,
            0xf_ffff_ffff_ffff,
        ] {
            bounds.push(f64::from_bits((exponent << 52) | fraction));
        }
    }
    let mut state = 1_u64;
    for _ in 0_u8..200 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let drawn = f64::from_bits((2_044 + state % 3) << 52 | state >> 12);
        bounds.push(drawn);
    }
    let mut counted = [0_u32; 2];
    for &below in &bounds {
        for &above in &bounds {
            let infinite = !(above + below).is_finite();
            let expected = if infinite {
                Err(ParamSpecError::InfiniteWidth { name: "p" })
            } else {
                Ok([0, (-below).to_bits(), above.to_bits()])
            };
            assert_eq!(
                checked(0.0_f64, -below, above),
                expected,
                "[-{below:e}, {above:e}]"
            );
            counted[usize::from(infinite)] += 1;
        }
    }
    assert!(
        counted.iter().all(|&count| count > 5_000),
        "the control: both answers, {counted:?}"
    );
    // Bounds of one sign are never too far apart.
    for &bound in &bounds {
        assert!(
            checked(bound, bound, f64::MAX).is_ok(),
            "[{bound:e}, the largest]"
        );
    }
}

/// Each refusal's one text, whole: what `param!` fails to compile with and what the error
/// displays after the parameter's name.
#[test]
fn says_why_a_parameter_is_refused() {
    for (refused, reason) in [
        (
            ParamSpecError::NotFinite { name: "gain" },
            "its minimum or its maximum is not finite",
        ),
        (
            ParamSpecError::Reversed { name: "gain" },
            "its minimum is above its maximum",
        ),
        (
            ParamSpecError::Unreachable { name: "gain" },
            "its minimum or its maximum is below 2^-126 in magnitude but not zero: no value \
             reaches it",
        ),
        (
            ParamSpecError::InfiniteWidth { name: "gain" },
            "its width, its maximum less its minimum, is not finite in binary64",
        ),
        (
            ParamSpecError::DefaultOutsideRange { name: "gain" },
            "its default, flushed, is not within its range",
        ),
    ] {
        assert_eq!(refused.reason(), reason, "{refused:?}");
        assert_eq!(
            format!("{refused}"),
            format!("parameter \"gain\": {reason}"),
            "{refused:?}"
        );
    }
}

#[test]
fn takes_each_value_picked_its_range_accepts_else_the_default() {
    // `a` accepts -1 to 2 (default 0.5), `b` 0 to 20,000 (default 440).
    let picked = |pick: &dyn Fn(&ParamSpec) -> f64| bits(&ParamValues::within(&TWO, pick));
    assert_eq!(
        picked(&|param| if param.name() == "a" {
            -0.0_f64
        } else {
            3.0e4_f64
        }),
        [0x8000_0000_0000_0000, 440.0_f64.to_bits()],
        "-0 kept bit for bit, 30,000 beyond b's range: its default"
    );
    assert_eq!(
        picked(&|_| SMALLEST),
        [SMALLEST.to_bits(); 2],
        "the smallest normal binary32, within both"
    );
    assert_eq!(
        picked(&|_| 0.1_f64),
        [0x3fb9_9999_9999_999a; 2],
        "a binary64 value no binary32 holds, kept bit for bit"
    );
    assert_eq!(
        picked(&|_| -SMALLEST),
        [(-SMALLEST).to_bits(), 440.0_f64.to_bits()],
        "below b's range: its default"
    );
    assert_eq!(
        picked(&|_| -1e-300_f64),
        [0x8000_0000_0000_0000; 2],
        "below 2^-126 a zero of its sign, which both accept"
    );
    assert_eq!(
        picked(&|_| f64::NAN),
        [0.5_f64.to_bits(), 440.0_f64.to_bits()],
        "NaN: the defaults"
    );
    assert_eq!(
        bits(&ParamValues::within(&TINY, |_| 2.0_f64)),
        [0x8000_0000_0000_0000],
        "a default below 2^-126, flushed"
    );
    let mut names = Vec::new();
    let mut count = 0.0_f64;
    let counted = ParamValues::within(&TWO, |param| {
        names.push(param.name());
        count += 1.0_f64;
        count
    });
    assert_eq!(
        (names, bits(&counted)),
        (vec!["a", "b"], vec![1.0_f64.to_bits(), 2.0_f64.to_bits()]),
        "picked once per parameter, in order"
    );
    assert!(
        core::ptr::eq(
            ParamValues::within(&TWO, |_| 0.0_f64).spec(),
            &raw const TWO
        ),
        "of its box type"
    );
}

#[test]
fn drives_with_values_within_their_range_only_flushed_first() {
    let below = SMALLEST.next_down();
    let mut first: [Binary64; 64] = [Binary64::new(0.25); 64];
    first[0] = Binary64::new(-0.0);
    first[1] = Binary64::new(below);
    first[2] = Binary64::new(-1e-300);
    first[3] = Binary64::new(0.1);
    first[63] = Binary64::new(2.0);
    let mut second = [Binary64::new(20_000.0); 64];
    second[5] = Binary64::new(SMALLEST);
    let driven = DrivenValues::of(&TWO, vec![first, second]).expect("within range");
    assert!(
        core::ptr::eq(driven.spec(), &raw const TWO),
        "of its box type"
    );
    let given: Vec<Vec<u64>> = driven
        .blocks
        .iter()
        .map(|block| block.iter().map(|value| value.raw().to_bits()).collect())
        .collect();
    let mut expected_a = vec![0.25_f64.to_bits(); 64];
    expected_a[..4].copy_from_slice(&[
        0x8000_0000_0000_0000,
        0,
        0x8000_0000_0000_0000,
        0x3fb9_9999_9999_999a,
    ]);
    expected_a[63] = 2.0_f64.to_bits();
    let mut expected_b = vec![20_000.0_f64.to_bits(); 64];
    expected_b[5] = SMALLEST.to_bits();
    assert_eq!(
        given,
        vec![expected_a, expected_b],
        "below 2^-126 a zero of its sign, every other value kept bit for bit"
    );
    for (count, blocks) in [
        (0, vec![]),
        (1, vec![first]),
        (3, vec![first, second, second]),
    ] {
        assert_eq!(
            DrivenValues::of(&TWO, blocks).map(|_| ()),
            Err(ParamError::Blocks {
                kind: "two~",
                expected: 2,
                given: count,
            }),
            "{count} blocks"
        );
    }
    for (frame, value, refused) in [
        (0, f64::NAN, f64::NAN),
        (63, 2.0_f64.next_up(), 2.0_f64.next_up()),
        (17, (-1.0_f64).next_down(), (-1.0_f64).next_down()),
        (40, f64::INFINITY, f64::INFINITY),
    ] {
        let mut wrong = first;
        wrong[frame] = Binary64::new(value);
        let mut also = second;
        also[0] = Binary64::new(-1.0);
        assert_eq!(
            DrivenValues::of(&TWO, vec![wrong, also]).map(|_| ()),
            Err(ParamError::DrivenOutOfRange {
                kind: "two~",
                name: "a",
                frame,
                bits: refused.to_bits(),
                minimum: -1.0_f64,
                maximum: 2.0_f64,
            }),
            "{value} at frame {frame}: the first parameter's refusal first"
        );
    }
}

/// Two frames refused: the first is told.
#[test]
fn refuses_the_first_of_two_driven_frames_out_of_range() {
    let first = [Binary64::new(0.25); 64];
    let second = [Binary64::new(20_000.0); 64];
    let mut twice = first;
    twice[40] = Binary64::new(f64::INFINITY);
    twice[17] = Binary64::new(-3.0);
    assert_eq!(
        DrivenValues::of(&TWO, vec![twice, second]).map(|_| ()),
        Err(ParamError::DrivenOutOfRange {
            kind: "two~",
            name: "a",
            frame: 17,
            bits: (-3.0_f64).to_bits(),
            minimum: -1.0_f64,
            maximum: 2.0_f64,
        }),
        "the first of two frames refused"
    );
    let mut wrong = second;
    wrong[9] = Binary64::new(-SMALLEST);
    assert_eq!(
        DrivenValues::of(&TWO, vec![first, wrong]).map(|_| ()),
        Err(ParamError::DrivenOutOfRange {
            kind: "two~",
            name: "b",
            frame: 9,
            bits: (-SMALLEST).to_bits(),
            minimum: 0.0_f64,
            maximum: 20_000.0_f64,
        }),
        "-2^-126, below b's range"
    );
}

/// On a test's thread, whose stack is smaller than one block of the largest engine block.
#[test]
fn drives_with_picked_values_at_the_largest_engine_block() {
    const LAST: usize = crate::MAX_ENGINE_BLOCK - 1;
    let driven =
        DrivenValues::<{ crate::MAX_ENGINE_BLOCK }>::within(&TOLD, |_, _, frame| match frame {
            0 => 2.0_f64,
            LAST => 3.0_f64,
            _ => 4.0_f64,
        });
    assert_eq!(
        driven.blocks.first().map(|block| {
            [block[0], block[1], block[LAST - 1], block[LAST]].map(|value| value.raw().to_bits())
        }),
        Some([2.0_f64, 4.0_f64, 4.0_f64, 3.0_f64].map(f64::to_bits)),
        "frame by frame, to the last"
    );
}

#[test]
fn drives_with_each_value_picked_its_range_accepts_else_the_default() {
    // `a` accepts -1 to 2 (default 0.5), `b` 0 to 20,000 (default 440).
    let mut calls = Vec::new();
    let driven = DrivenValues::<4>::within(&TWO, |index, param, frame| {
        calls.push((index, param.name(), frame));
        match (index, frame) {
            (0, 0) => -0.0_f64,
            (0, 1) => 2.0_f64.next_up(),
            (0, 2) => -1e-300_f64,
            (0, _) => 0.1_f64,
            (_, 0) => f64::NAN,
            (_, 1) => SMALLEST,
            (_, 2) => -SMALLEST,
            (_, _) => 20_000.0_f64,
        }
    });
    assert_eq!(
        calls,
        [
            (0, "a", 0),
            (0, "a", 1),
            (0, "a", 2),
            (0, "a", 3),
            (1, "b", 0),
            (1, "b", 1),
            (1, "b", 2),
            (1, "b", 3),
        ],
        "picked once per parameter and frame, parameter by parameter"
    );
    let given: Vec<Vec<u64>> = driven
        .blocks
        .iter()
        .map(|block| block.iter().map(|value| value.raw().to_bits()).collect())
        .collect();
    assert_eq!(
        given,
        [
            vec![
                0x8000_0000_0000_0000,
                0.5_f64.to_bits(),
                0x8000_0000_0000_0000,
                0x3fb9_9999_9999_999a,
            ],
            vec![
                440.0_f64.to_bits(),
                SMALLEST.to_bits(),
                440.0_f64.to_bits(),
                20_000.0_f64.to_bits(),
            ],
        ],
        "-0 and 0.1 kept bit for bit, -1e-300 a zero of its sign, NaN and values beyond a \
         range their default"
    );
    assert!(
        core::ptr::eq(driven.spec(), &raw const TWO),
        "of its box type"
    );
    let tiny = DrivenValues::<2>::within(&TINY, |_, _, _| 2.0_f64);
    let flushed: Vec<[u64; 2]> = tiny
        .blocks
        .iter()
        .map(|block| block.map(|value| value.raw().to_bits()))
        .collect();
    assert_eq!(
        flushed,
        [[0x8000_0000_0000_0000; 2]],
        "a default below 2^-126, flushed to a zero of its sign"
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

/// A box type whose instance outputs its parameter value then the sample rate in
/// hertz, frame by frame, so that a test sees what it was created with. Its driven body
/// outputs the values it is driven by instead: a probe whose bodies differ on purpose, never
/// held to the driven laws.
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

impl<S: Sample, const N: usize> Driven<S, N, 1> for Told {
    fn set(&mut self, [value]: [Binary64; 1]) {
        let [held, _rate] = &mut self.0;
        *held = value.raw();
    }

    /// Outputs the values it is driven by, frame by frame.
    fn process_driven(
        &mut self,
        _inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        [values]: &[Block<Binary64, N>; 1],
    ) {
        for output in outputs {
            *output = values.map(S::from_binary64);
        }
    }
}

impl Create<1> for Told {
    const BOX_MODE: crate::BoxMode = crate::BoxMode::LiveCapable;
    const PARAMS: &'static [ParamSpec; 1] = &[crate::param!("v", "v", 1.0, 0.0, 1_000.0)];

    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        [value]: [Binary64; 1],
    ) -> impl Driven<S, N, 1> + 'static {
        Self([value.raw(), f64::from(rate.hz())])
    }
}

/// The box type of [`Told`], of one parameter from 0 to 1,000.
static TOLD: ObjectSpec = mute::<Told, 1>("told~");

/// The first two frames of the instance computing in `S` that `values` create, at 8 kHz,
/// through the virtual table.
fn created<S: crate::host::Raw>(values: &ParamValues) -> [u64; 2] {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut out = [S::SILENCE];
    <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, values)
        .expect("a test build holds every engine block")
        .process(&[], &mut out);
    let [first, second, ..] = out[0];
    [first.bits(), second.bits()]
}

#[test]
fn an_instance_takes_its_parameters_by_either_way_and_no_other_number_of_them() {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut told = <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(
        rate,
        &ParamValues::defaults(&TOLD),
    )
    .expect("a test build holds every engine block");
    let given = ParamValues::flushed(&TOLD, vec![Binary64::new(3.0)]);
    assert!(told.receive(&given), "its one value");
    let mut out = [Binary64::SILENCE];
    told.process(&[], &mut out);
    assert_eq!(
        out[0][0].raw().to_bits(),
        3.0_f64.to_bits(),
        "the value set"
    );
    let ramp: [Binary64; 64] = core::array::from_fn(|frame| {
        Binary64::new(f64::from(u8::try_from(frame).expect("below 64")))
    });
    let driven = DrivenValues::of(&TOLD, vec![ramp]).expect("within range");
    assert!(
        told.run_driven(&[], &mut out, &driven),
        "its one block of values"
    );
    assert_eq!(
        out[0].map(|sample| sample.raw().to_bits()),
        ramp.map(|value| value.raw().to_bits()),
        "a value per frame"
    );
    // Another number of values, or of blocks (those of another box type): refused, with
    // nothing done.
    for (count, spec) in [(0, &NONE), (2, &TWO)] {
        let wrong = ParamValues::flushed(&TOLD, vec![Binary64::new(9.0); count]);
        assert!(!told.receive(&wrong), "{count} values");
        let before = out;
        let blocks = DrivenValues::of(spec, vec![[Binary64::ZERO; 64]; count]).expect("zeros");
        assert!(!told.run_driven(&[], &mut out, &blocks), "{count} blocks");
        assert_eq!(
            out[0].map(|sample| sample.raw().to_bits()),
            before[0].map(|sample| sample.raw().to_bits()),
            "nothing written"
        );
    }
    told.process(&[], &mut out);
    assert_eq!(
        out[0][0].raw().to_bits(),
        3.0_f64.to_bits(),
        "the value it had"
    );
    // A box without parameters has nothing to be driven by: it renders as it would.
    let mut none = <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(
        rate,
        &ParamValues::defaults(&NONE),
    )
    .expect("a test build holds every engine block");
    assert!(none.receive(&ParamValues::defaults(&NONE)), "no value");
    let nothing = DrivenValues::of(&NONE, vec![]).expect("no parameter");
    assert!(
        none.run_driven(&[], &mut [], &nothing),
        "no block of values"
    );
    assert!(
        !none.run_driven(&[], &mut [], &driven),
        "a block it has no parameter for"
    );
}

/// The cheat of the second deep review of #12 block 3 ("3a"), finding D14-1: values checked
/// against another box type of as many parameters, outside the instance's own ranges, taken
/// by its bodies as its own.
#[test]
fn an_instance_refuses_the_values_of_another_box_type_of_as_many_parameters() {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let mut told = <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(
        rate,
        &ParamValues::defaults(&TOLD),
    )
    .expect("a test build holds every engine block");
    let mut out = [Binary64::SILENCE];
    told.process(&[], &mut out);
    let before = out[0].map(|sample| sample.raw().to_bits());
    // -0.5: a value of tiny~'s range, outside told~'s, as many as told~ has.
    let other = ParamValues::of(&TINY, &[Binary64::new(-0.5)]).expect("within tiny~'s range");
    assert!(!told.receive(&other), "tiny~'s value");
    let blocks = DrivenValues::of(&TINY, vec![[Binary64::new(-0.5); 64]]).expect("within range");
    assert!(
        !told.run_driven(&[], &mut out, &blocks),
        "tiny~'s block of values"
    );
    assert_eq!(
        out[0].map(|sample| sample.raw().to_bits()),
        before,
        "nothing written"
    );
    told.process(&[], &mut out);
    assert_eq!(
        out[0].map(|sample| sample.raw().to_bits()),
        before,
        "the value it had"
    );
}

#[test]
fn an_instance_shows_the_name_of_its_box_type_and_its_state() {
    assert_eq!(
        format!("{:?}", Typed::<_, 1>::new(&TOLD, Told([1.0_f64, 2.0_f64]))),
        "Typed { box_type: \"told~\", instance: Told([1.0, 2.0]) }"
    );
}

/// A box type without parameters that writes its input's frame number into its outlet,
/// plus the input: what a body that writes nothing, or another body, would not give.
#[derive(Debug)]
struct Counting(f64);

impl<S: Sample, const N: usize> Object<S, N> for Counting {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                *out = S::from_binary64(Binary64::new(self.0 + value.to_binary64().raw()));
                self.0 += 1.0_f64;
            }
        }
    }
}

impl Create<0> for Counting {
    const BOX_MODE: crate::BoxMode = crate::BoxMode::LiveCapable;
    const PARAMS: &'static [ParamSpec; 0] = &[];

    fn create<S: Sample, const N: usize>(
        _rate: SampleRate,
        []: [Binary64; 0],
    ) -> impl Driven<S, N, 0> + 'static {
        Self(0.5)
    }
}

/// The box type of [`Counting`], of an inlet and an outlet.
static COUNTING: ObjectSpec = ObjectSpec::new::<Counting, 0>(
    "counting~",
    "counts its frames",
    &[PortSpec {
        name: "in",
        description: "a signal",
    }],
    &[PortSpec {
        name: "out",
        description: "a count",
    }],
    &[],
    AlgorithmVersion::FIRST,
);

#[test]
fn drives_a_box_without_parameters_by_its_one_body() {
    let rate = SampleRate::new(8_000).expect("valid rate");
    let create = || {
        <Box<dyn BoxInstance<Binary64>> as Dispatch<Binary64>>::create(
            rate,
            &ParamValues::defaults(&COUNTING),
        )
        .expect("a test build holds every engine block")
    };
    let (mut driven, mut steady) = (create(), create());
    let input: [Binary64; 64] = core::array::from_fn(|frame| {
        Binary64::new(f64::from(u8::try_from(frame).expect("below 64")) * 0.25_f64)
    });
    let none = DrivenValues::of(&COUNTING, vec![]).expect("no parameter");
    for block in 0..2_u8 {
        let mut out = [[Binary64::new(f64::NAN); 64]];
        let mut expected = out;
        assert!(
            driven.run_driven(&[input], &mut out, &none),
            "block {block}"
        );
        steady.process(&[input], &mut expected);
        let first = f64::from(block) * 64.0_f64 + 0.5_f64;
        assert_eq!(
            out[0].map(|sample| sample.raw().to_bits()),
            core::array::from_fn::<u64, 64, _>(|frame| {
                let at = f64::from(u8::try_from(frame).expect("below 64"));
                (first + at + at * 0.25_f64).to_bits()
            }),
            "block {block}: its count and its input, every frame written"
        );
        assert_eq!(
            out[0].map(|sample| sample.raw().to_bits()),
            expected[0].map(|sample| sample.raw().to_bits()),
            "block {block}: the bits of its steady body"
        );
    }
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
    let spec: &'static ObjectSpec = Box::leak(Box::new(ObjectSpec::new::<Told, 1>(
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
            "parameter \"gain\": its default, flushed, is not within its range",
            "box type \"two~\" takes 2 parameter value(s), not 1",
        ],
        "said as they display"
    );
}
