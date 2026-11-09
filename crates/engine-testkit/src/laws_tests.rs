//! Tests of the laws: each law must catch a box that breaks it, in either precision.

use core::hint::black_box;
use core::sync::atomic::{AtomicU16, Ordering};

use engine_core::host::Raw;
use engine_core::{
    Binary64, Block, Create, Factory, Object, ObjectSpec, ParamSpec, PortSpec, Precision, Sample,
    SampleRate,
};

use super::{check_laws, check_metadata, check_registry};

const PORT: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};
const GAIN: ParamSpec = ParamSpec::new("gain", "gain", 0.5, 0.0, 1.0);

/// The boxes of these tests, each written once for both precisions.
#[derive(Debug)]
enum Fixture {
    /// Multiplies its input by a gain, flushing subnormal products: respects every law.
    Gain(f64),
    /// Outputs NaN.
    Nan,
    /// Outputs NaN in one precision only, and silence in the other.
    NanIn(Precision),
    /// Outputs a constant, rounded to the precision of the render.
    Constant(f64),
    /// Outputs a constant in one precision only, and silence in the other.
    ConstantIn(Precision, f64),
    /// Outputs the smallest subnormal of the precision of the render.
    Subnormal,
    /// Allocates and frees while processing.
    Allocating,
    /// Frees memory allocated when it was created, on its first call only: it never
    /// allocates while processing.
    Freeing(Option<Vec<u8>>),
    /// NaN as soon as an input is negative.
    NegativeIsNan,
    /// NaN on its second outlet only when its second inlet carries a non-zero signal.
    SecondInlet,
    /// NaN only on a negative zero or a subnormal input.
    Special,
    /// NaN only on inputs strictly between 0.25 and 0.75.
    Middle,
    /// Infinity after 512 blocks.
    Drifts(u32),
}

impl<S: Sample> Object<S> for Fixture {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 multiplication, which neither overflows nor panics"
    )]
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        let nan = of::<S>(f64::NAN);
        let either = |condition: bool| if condition { nan } else { S::ZERO };
        let inputs_any =
            |test: &dyn Fn(f64) -> bool| inputs.iter().flatten().any(|sample| test(widen(*sample)));
        match self {
            Self::Gain(gain) => {
                for (output, input) in outputs.iter_mut().zip(inputs) {
                    for (out, value) in output.iter_mut().zip(input) {
                        *out = (*value * of::<S>(*gain)).flush();
                    }
                }
            }
            Self::Nan => fill(outputs, nan),
            Self::NanIn(precision) => fill(outputs, either(*precision == S::PRECISION)),
            Self::Constant(value) => fill(outputs, of::<S>(*value)),
            Self::ConstantIn(precision, value) => fill(
                outputs,
                of::<S>(if *precision == S::PRECISION {
                    *value
                } else {
                    0.0
                }),
            ),
            Self::Subnormal => fill(
                outputs,
                of::<S>(match S::PRECISION {
                    Precision::Binary64 => f64::from_bits(1),
                    Precision::Binary32 => f64::from(f32::from_bits(1)),
                }),
            ),
            Self::Allocating => drop(black_box(Vec::<u8>::with_capacity(8))),
            Self::Freeing(memory) => drop(black_box(memory.take())),
            Self::NegativeIsNan => fill(outputs, either(inputs_any(&|sample| sample < 0.0_f64))),
            Self::SecondInlet => {
                let second = inputs
                    .get(1)
                    .is_some_and(|block| block.iter().any(|sample| widen(*sample) != 0.0_f64));
                for output in outputs.iter_mut().skip(1) {
                    output.fill(either(second));
                }
            }
            Self::Special => {
                let special = inputs_any(&|sample| {
                    sample.to_bits() == (-0.0_f64).to_bits() || subnormal_in::<S>(sample)
                });
                fill(outputs, either(special));
            }
            Self::Middle => fill(
                outputs,
                either(inputs_any(&|sample| sample > 0.25_f64 && sample < 0.75_f64)),
            ),
            Self::Drifts(blocks) => {
                *blocks = blocks.saturating_add(1);
                let value = if *blocks > 512 {
                    f64::INFINITY
                } else {
                    0.0_f64
                };
                fill(outputs, of::<S>(value));
            }
        }
    }
}

/// Fills every output with `value`.
fn fill<S: Sample>(outputs: &mut [Block<S>], value: S) {
    for output in outputs {
        output.fill(value);
    }
}

/// `sample` in binary64, exactly.
fn widen<S: Sample>(sample: S) -> f64 {
    sample.to_binary64().raw()
}

/// `value` in the precision `S`.
fn of<S: Sample>(value: f64) -> S {
    S::from_binary64(Binary64::from_raw(value))
}

/// Whether `value`, a sample of `S` in binary64, is subnormal in `S`.
fn subnormal_in<S: Sample>(value: f64) -> bool {
    let smallest = match S::PRECISION {
        Precision::Binary64 => f64::MIN_POSITIVE,
        Precision::Binary32 => f64::from(f32::MIN_POSITIVE),
    };
    value != 0.0 && value.abs() < smallest
}

/// A [`Factory`] from a function of the sample rate and the parameters that chooses the
/// [`Fixture`] to create, in either precision.
macro_rules! factory {
    (|$rate:pat_param, $params:pat_param| $body:expr) => {{
        /// The fixture's box type.
        struct Chosen;
        impl Create for Chosen {
            fn create<S: Sample>($rate: SampleRate, given: &[Binary64]) -> Box<dyn Object<S>> {
                let raw: Vec<f64> = given.iter().map(|param| param.raw()).collect();
                let $params: &[f64] = &raw;
                Box::new($body)
            }
        }
        Factory::new::<Chosen>()
    }};
}

const GOOD: ObjectSpec = ObjectSpec {
    name: "gain~",
    description: "multiplies by a gain",
    inlets: &[PORT],
    outlets: &[OUT],
    params: &[GAIN],
    capabilities: &[],
    create: factory!(|_, params| Fixture::Gain(params.first().copied().unwrap_or(1.0))),
};

static INSTANCES: AtomicU16 = AtomicU16::new(0);

fn spec_with(create: Factory) -> ObjectSpec {
    ObjectSpec { create, ..GOOD }
}

#[test]
fn a_correct_box_passes_every_law() {
    assert_eq!(check_laws(&GOOD), Ok(()), "gain~ respects the laws");
    let no_ports = ObjectSpec {
        inlets: &[],
        outlets: &[],
        params: &[],
        ..GOOD
    };
    assert_eq!(check_laws(&no_ports), Ok(()), "a box without ports");
}

#[test]
fn catches_non_finite_output() {
    let error = check_laws(&spec_with(factory!(|_, _| Fixture::Nan))).expect_err("NaN is caught");
    assert!(
        error.starts_with(
            "binary64, at 8000 Hz: special inputs: non-finite output with parameters [0.0]"
        ),
        "{error}"
    );
}

#[test]
fn catches_a_broken_law_in_either_precision() {
    for precision in [Precision::Binary64, Precision::Binary32] {
        let spec = spec_with(match precision {
            Precision::Binary64 => factory!(|_, _| Fixture::NanIn(Precision::Binary64)),
            Precision::Binary32 => factory!(|_, _| Fixture::NanIn(Precision::Binary32)),
        });
        let error = check_laws(&spec).expect_err("NaN in one precision");
        assert!(
            error.starts_with(&format!(
                "binary{}, at 8000 Hz: special inputs: non-finite output",
                precision.bits()
            )),
            "{error}"
        );
    }
}

#[test]
fn catches_subnormal_output() {
    // The smallest subnormal of each precision: finite and deterministic, but not flushed.
    // Each precision is checked against its own smallest normal: 2^-149 is subnormal in
    // binary32 and normal in binary64, where 2^-1074 is subnormal.
    let error = check_laws(&spec_with(factory!(|_, _| Fixture::Subnormal)))
        .expect_err("a subnormal is caught");
    assert!(
        error.starts_with(
            "binary64, at 8000 Hz: special inputs: subnormal output with parameters [0.0]"
        ),
        "{error}"
    );
    let narrow = spec_with(factory!(|_, _| Fixture::Constant(f64::from(
        f32::from_bits(1)
    ))));
    let caught = check_laws(&narrow).expect_err("a binary32 subnormal is caught");
    assert!(
        caught.starts_with("binary32, at 8000 Hz: special inputs: subnormal output"),
        "{caught}"
    );
    let negative = spec_with(factory!(|_, _| Fixture::Constant(f64::from(
        f32::from_bits(0x8040_0000)
    ))));
    let failure = check_laws(&negative).expect_err("a negative subnormal is caught");
    assert!(
        failure.contains("subnormal output with parameters [0.0]"),
        "{failure}"
    );
    // The smallest normal and zeros of both signs are not subnormal.
    let not_subnormal = [
        factory!(|_, _| Fixture::Constant(f64::from(f32::MIN_POSITIVE))),
        factory!(|_, _| Fixture::Constant(-f64::from(f32::MIN_POSITIVE))),
        factory!(|_, _| Fixture::Constant(0.0)),
        factory!(|_, _| Fixture::Constant(-0.0)),
    ];
    for create in not_subnormal {
        let spec = ObjectSpec {
            inlets: &[],
            params: &[],
            create,
            ..GOOD
        };
        assert_eq!(check_laws(&spec), Ok(()), "not subnormal");
    }
}

#[test]
fn catches_non_deterministic_output() {
    let spec = spec_with(factory!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::Constant(f64::from(instance % 1000))
    }));
    let error = check_laws(&spec).expect_err("instances differ");
    assert!(
        error.contains("not deterministic with parameters [0.0]"),
        "{error}"
    );
    // In each precision on its own: the other one is silent, hence deterministic.
    let wide = spec_with(factory!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::ConstantIn(Precision::Binary64, f64::from(instance % 1000))
    }));
    let in_wide = check_laws(&wide).expect_err("binary64 instances differ");
    assert!(
        in_wide.starts_with("binary64, at 8000 Hz: special inputs: not deterministic"),
        "{in_wide}"
    );
    let narrow = spec_with(factory!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::ConstantIn(Precision::Binary32, f64::from(instance % 1000))
    }));
    let in_narrow = check_laws(&narrow).expect_err("binary32 instances differ");
    assert!(
        in_narrow.starts_with("binary32, at 8000 Hz: special inputs: not deterministic"),
        "{in_narrow}"
    );
}

#[test]
fn catches_allocation_while_processing() {
    let error =
        check_laws(&spec_with(factory!(|_, _| Fixture::Allocating))).expect_err("allocation");
    assert!(
        error.contains("uses the heap while processing (16 heap operations)"),
        "{error}"
    );
}

#[test]
fn catches_a_release_while_processing() {
    let spec = spec_with(factory!(|_, _| Fixture::Freeing(Some(vec![0_u8; 8]))));
    let error = check_laws(&spec).expect_err("a release");
    assert!(
        error.contains("uses the heap while processing (1 heap operations)"),
        "{error}"
    );
}

#[test]
fn checks_parameters_at_negative_zero_and_the_smallest_subnormal_and_normal() {
    // NaN only for a parameter of -0.0, then only for a binary32 subnormal parameter, then
    // only for the smallest normal binary32: none is a minimum, default, midpoint or maximum.
    // Parameters reach a box in binary64, as written.
    let negative_zero = spec_with(factory!(|_, params| {
        if params.first().map(|value| value.to_bits()) == Some((-0.0_f64).to_bits()) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    }));
    let error = check_laws(&negative_zero).expect_err("-0.0 is tried");
    assert!(error.contains("with parameters [-0.0]"), "{error}");
    let subnormal = spec_with(factory!(|_, params| {
        if params
            .first()
            .is_some_and(|value| *value != 0.0_f64 && value.abs() < f64::from(f32::MIN_POSITIVE))
        {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    }));
    let failure = check_laws(&subnormal).expect_err("a subnormal is tried");
    assert!(failure.contains("with parameters [1e-45]"), "{failure}");
    let normal = spec_with(factory!(|_, params| {
        if params.first().copied() == Some(f64::from(f32::MIN_POSITIVE)) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    }));
    let smallest = check_laws(&normal).expect_err("the smallest normal is tried");
    assert!(
        smallest.contains("with parameters [1.1754944e-38]"),
        "{smallest}"
    );
}

#[test]
fn tries_special_parameter_values_only_within_the_range() {
    const POSITIVE: ParamSpec = ParamSpec::new("gain", "gain", 2.0, 1.0, 3.0);
    const NEGATIVE: ParamSpec = ParamSpec::new("gain", "gain", -2.0, -3.0, -1.0);
    let bits = |param: &'static ParamSpec| {
        super::param_sets(&ObjectSpec {
            params: core::slice::from_ref(param),
            ..GOOD
        })
        .map(|set| set.iter().map(|value| value.to_bits()).collect::<Vec<_>>())
    };
    assert_eq!(
        bits(&GAIN),
        [
            vec![0.0_f32.to_bits()],
            vec![0.5_f32.to_bits()],
            vec![0.75_f32.to_bits()],
            vec![1.0_f32.to_bits()],
            vec![0x8000_0000],
            vec![0x0000_0001],
            vec![0x0080_0000],
        ],
        "[0, 1]: minimum, default, away from the default, maximum, -0.0, 1e-45, 2^-126"
    );
    assert_eq!(
        bits(&POSITIVE)[4..],
        [
            vec![2.0_f32.to_bits()],
            vec![2.0_f32.to_bits()],
            vec![2.0_f32.to_bits()]
        ],
        "above zero: the default instead"
    );
    assert_eq!(
        bits(&NEGATIVE)[4..],
        [
            vec![(-2.0_f32).to_bits()],
            vec![(-2.0_f32).to_bits()],
            vec![(-2.0_f32).to_bits()]
        ],
        "below zero: the default instead"
    );
}

#[test]
fn tries_every_case_at_every_rate_with_seven_parameter_sets() {
    static CREATED: AtomicU16 = AtomicU16::new(0);
    let spec = spec_with(factory!(|_, _| {
        let _: u16 = CREATED.fetch_add(1, Ordering::Relaxed);
        Fixture::Gain(1.0)
    }));
    assert_eq!(check_laws(&spec), Ok(()), "a correct box");
    // Per precision and rate: (special inputs + 32 random cases) x 7 parameter sets x 2
    // runs, plus a long run per parameter set.
    let per_rate = (1_usize + 32) * 7 * 2 + 7;
    let expected = per_rate
        .checked_mul(crate::matrix::SAMPLE_RATES.len())
        .and_then(|count| count.checked_mul(crate::matrix::PRECISIONS.len()));
    assert_eq!(
        Some(usize::from(CREATED.load(Ordering::Relaxed))),
        expected,
        "every precision, rate, case and parameter set"
    );
}

#[test]
fn feeds_inputs_across_the_whole_range() {
    // NaN as soon as an input is negative: the law must try negative inputs.
    let error =
        check_laws(&spec_with(factory!(|_, _| Fixture::NegativeIsNan))).expect_err("negatives");
    assert!(error.contains("non-finite output"), "{error}");
}

#[test]
fn checks_parameters_at_minimum_default_and_maximum() {
    // NaN only at the maximum gain: the law must try the maximum too.
    let spec = spec_with(factory!(|_, params| {
        if params.first().copied() == Some(1.0) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    }));
    let error = check_laws(&spec).expect_err("maximum is checked");
    assert!(
        error.contains("non-finite output with parameters [1.0]"),
        "{error}"
    );
}

#[test]
fn feeds_inputs_to_every_inlet_and_reads_every_outlet() {
    // NaN only when the second inlet carries a non-zero signal.
    const TWO: [PortSpec; 2] = [
        PortSpec {
            name: "a",
            description: "a",
        },
        PortSpec {
            name: "b",
            description: "b",
        },
    ];
    let spec = ObjectSpec {
        inlets: &TWO,
        outlets: &TWO,
        params: &[],
        create: factory!(|_, _| Fixture::SecondInlet),
        ..GOOD
    };
    let error = check_laws(&spec).expect_err("the second inlet and outlet are exercised");
    assert!(error.contains("non-finite output"), "{error}");
}

#[test]
fn checks_every_sample_rate_of_the_matrix() {
    // NaN at every rate but 48 kHz: the laws must try the other rates.
    let spec = spec_with(factory!(|rate, _| {
        Fixture::Constant(if rate.hz() == 48_000 { 0.0 } else { f64::NAN })
    }));
    let error = check_laws(&spec).expect_err("other rates are checked");
    assert!(error.starts_with("binary64, at 8000 Hz: "), "{error}");
    for hz in crate::matrix::SAMPLE_RATES {
        let only = spec_with(factory!(|rate, _| {
            Fixture::Constant(if rate.hz() == ONLY.load(Ordering::Relaxed) {
                f64::NAN
            } else {
                0.0
            })
        }));
        ONLY.store(hz, Ordering::Relaxed);
        let failure = check_laws(&only).expect_err("each rate is checked");
        assert!(
            failure.starts_with(&format!("binary64, at {hz} Hz: ")),
            "{failure}"
        );
    }
}

/// The only rate at which a fixture box fails, set by the test.
static ONLY: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[test]
fn feeds_the_special_inputs() {
    // NaN only on a negative zero or a subnormal input of the precision: random inputs never
    // hit them, in either precision.
    let error =
        check_laws(&spec_with(factory!(|_, _| Fixture::Special))).expect_err("special inputs");
    assert!(
        error.starts_with("binary64, at 8000 Hz: special inputs: non-finite output"),
        "{error}"
    );
}

#[test]
fn feeds_random_inputs() {
    // NaN only on inputs strictly between 0.25 and 0.75: no special input, no silence,
    // only the random inputs reach it.
    let error =
        check_laws(&spec_with(factory!(|_, _| Fixture::Middle))).expect_err("random inputs");
    assert!(error.starts_with("binary64, at 8000 Hz: "), "{error}");
    assert!(error.contains("non-finite output"), "{error}");
    assert!(
        !error.contains("special inputs") && !error.contains("long run"),
        "found by the random inputs: {error}"
    );
}

#[test]
fn runs_long_enough_to_reveal_late_failures() {
    // Breaks only after 512 blocks: the short runs never see it.
    let error = check_laws(&spec_with(factory!(|_, _| Fixture::Drifts(0)))).expect_err("long run");
    assert!(error.contains("long run: non-finite output"), "{error}");
}

#[test]
fn checks_parameters_away_from_their_default() {
    // NaN only at 0.5, away from the default 0.25 and from both limits.
    const QUARTER: ParamSpec = ParamSpec::new("gain", "gain", 0.25, 0.0, 1.0);
    let spec = ObjectSpec {
        params: &[QUARTER],
        create: factory!(|_, params| {
            if params.first().copied() == Some(0.5) {
                Fixture::Nan
            } else {
                Fixture::Gain(1.0)
            }
        }),
        ..GOOD
    };
    let error = check_laws(&spec).expect_err("the midpoint is checked");
    assert!(error.contains("with parameters [0.5]"), "{error}");
    // With the default at the midpoint (GAIN: 0.5 in [0, 1]), 0.75 is tried instead.
    let shifted = spec_with(factory!(|_, params| {
        if params.first().copied() == Some(0.75) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    }));
    let failure = check_laws(&shifted).expect_err("a value away from the default");
    assert!(failure.contains("with parameters [0.75]"), "{failure}");
    // A range not starting at zero: the midpoint of [-1, 1] is 0.
    let centered = ParamSpec::new("gain", "gain", 0.5, -1.0, 1.0);
    assert_eq!(
        super::away_from_default(&centered).to_bits(),
        0.0_f32.to_bits(),
        "midpoint of [-1, 1]"
    );
    assert_eq!(
        super::away_from_default(&ParamSpec::new("gain", "gain", 0.0, -1.0, 1.0)),
        0.5,
        "halfway from the default 0 to the maximum"
    );
}

/// Fixtures for the metadata cases.
static DUP_PORTS: [PortSpec; 2] = [PORT, PORT];
static DUP_PARAMS: [ParamSpec; 2] = [GAIN, GAIN];
static UNDOCUMENTED_PORT: [PortSpec; 1] = [PortSpec {
    name: "x",
    description: " ",
}];
static EMPTY_PORT: [PortSpec; 1] = [PortSpec {
    name: "",
    description: "x",
}];

/// Specs with one metadata problem each, and the expected message.
static INVALID_METADATA: [(ObjectSpec, &str); 13] = [
    (ObjectSpec { name: "", ..GOOD }, "invalid box name \"\""),
    (
        ObjectSpec {
            name: "two words",
            ..GOOD
        },
        "invalid box name \"two words\"",
    ),
    (
        ObjectSpec {
            description: "  ",
            ..GOOD
        },
        "missing description",
    ),
    (
        ObjectSpec {
            inlets: &DUP_PORTS,
            ..GOOD
        },
        "duplicate inlet \"in\"",
    ),
    (
        ObjectSpec {
            outlets: &DUP_PORTS,
            ..GOOD
        },
        "duplicate outlet \"in\"",
    ),
    (
        ObjectSpec {
            inlets: &EMPTY_PORT,
            ..GOOD
        },
        "inlet with an empty name",
    ),
    (
        ObjectSpec {
            params: &DUP_PARAMS,
            ..GOOD
        },
        "duplicate parameter \"gain\"",
    ),
    (
        ObjectSpec {
            inlets: &UNDOCUMENTED_PORT,
            ..GOOD
        },
        "port \"x\" has no description",
    ),
    (
        ObjectSpec {
            outlets: &UNDOCUMENTED_PORT,
            ..GOOD
        },
        "port \"x\" has no description",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec::new("gain", "", 0.5, 0.0, 1.0)],
            ..GOOD
        },
        "parameter \"gain\" has no description",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec::new("gain", "gain", 2.0, 0.0, 1.0)],
            ..GOOD
        },
        "parameter \"gain\": default 2 outside finite range [0, 1]",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec::new("gain", "gain", 0.5, f32::NEG_INFINITY, 1.0)],
            ..GOOD
        },
        "parameter \"gain\": default 0.5 outside finite range [-inf, 1]",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec::new("gain", "gain", 0.5, 0.0, f32::INFINITY)],
            ..GOOD
        },
        "parameter \"gain\": default 0.5 outside finite range [0, inf]",
    ),
];

#[test]
fn rejects_invalid_metadata() {
    for (spec, message) in INVALID_METADATA {
        assert_eq!(check_metadata(&spec), Err(message.to_owned()), "{message}");
        assert_eq!(
            check_laws(&spec),
            Err(message.to_owned()),
            "laws check metadata first"
        );
    }
    assert_eq!(check_metadata(&GOOD), Ok(()), "valid metadata");
}

#[test]
fn checks_a_registry() {
    let other = ObjectSpec {
        name: "other~",
        ..GOOD
    };
    assert_eq!(check_registry(&[&GOOD, &other]), Ok(()), "valid registry");
    assert_eq!(check_registry(&[]), Ok(()), "empty registry");
    assert_eq!(
        check_registry(&[&GOOD, &GOOD]),
        Err("duplicate box \"gain~\"".to_owned()),
        "unique names"
    );
    let bad = ObjectSpec {
        description: "",
        ..other
    };
    assert_eq!(
        check_registry(&[&GOOD, &bad]),
        Err("other~: missing description".to_owned()),
        "every box is checked"
    );
}
