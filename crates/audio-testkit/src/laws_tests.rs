//! Tests of the laws: each law must catch a box that breaks it, in either precision.

use core::hint::black_box;
use core::sync::atomic::{AtomicU16, Ordering};

use audio_core::host::Raw;
use audio_core::{
    AlgorithmVersion, Binary64, Block, Capability, Create, Driven, Driving, Object, ObjectSpec,
    ParamSpec, PortSpec, Precision, Sample, SampleRate, Values,
};

use super::{check_description, check_laws, check_registry};
use crate::sample_of as of;

const PORT: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};
const GAIN: ParamSpec = audio_core::param!("gain", "gain", 0.5, 0.0, 1.0);

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
    /// In one precision, adds a millionth of its input to what its outputs hold on entry;
    /// silence in the other.
    ReadsOutputsIn(Precision),
    /// Writes silence to its outputs on every other block only, from its first.
    Skips(u32),
    /// Leaves its outputs as they are when every input sample of the block is a zero, and
    /// writes 0.25 to every sample otherwise: a box that skips its work in silence.
    SkipsWhenSilent,
}

impl<S: Sample, const N: usize> Object<S, N> for Fixture {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 multiplication, which neither overflows nor panics"
    )]
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
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
            Self::Allocating => {
                drop(black_box(Vec::<u8>::with_capacity(8)));
                fill(outputs, S::ZERO);
            }
            Self::Freeing(memory) => {
                drop(black_box(memory.take()));
                fill(outputs, S::ZERO);
            }
            Self::NegativeIsNan => fill(outputs, either(inputs_any(&|sample| sample < 0.0_f64))),
            Self::SecondInlet => {
                let second = inputs
                    .get(1)
                    .is_some_and(|block| block.iter().any(|sample| widen(*sample) != 0.0_f64));
                fill(outputs, S::ZERO);
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
            Self::ReadsOutputsIn(precision) => {
                if *precision == S::PRECISION {
                    for (output, input) in outputs.iter_mut().zip(inputs) {
                        for (out, value) in output.iter_mut().zip(input) {
                            *out = (*out + *value * of::<S>(1e-6)).flush();
                        }
                    }
                } else {
                    fill(outputs, S::ZERO);
                }
            }
            Self::Skips(blocks) => {
                if *blocks % 2 == 0 {
                    fill(outputs, S::ZERO);
                }
                *blocks = blocks.saturating_add(1);
            }
            Self::SkipsWhenSilent => {
                if inputs_any(&|sample| sample != 0.0_f64) {
                    fill(outputs, of::<S>(0.25));
                }
            }
        }
    }
}

impl<S: Sample, const N: usize> Driven<S, N, 1> for Fixture {
    fn set(&mut self, values: Values<1>) {
        let [value] = values.get();
        if let Self::Gain(gain) = self {
            *gain = value.raw();
        }
    }

    /// Renders as it would undriven: these laws drive no parameter.
    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        _params: Driving<'_, N, 1>,
    ) {
        self.process(inputs, outputs);
    }
}

impl<S: Sample, const N: usize> Driven<S, N, 2> for Fixture {
    fn set(&mut self, _params: Values<2>) {}

    /// Renders as it would undriven: these laws drive no parameter.
    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        _params: Driving<'_, N, 2>,
    ) {
        self.process(inputs, outputs);
    }
}

/// Fills every output with `value`.
fn fill<S: Sample, const N: usize>(outputs: &mut [Block<S, N>], value: S) {
    for output in outputs {
        output.fill(value);
    }
}

/// `sample` in binary64, exactly.
fn widen<S: Sample>(sample: S) -> f64 {
    sample.to_binary64().raw()
}

/// Whether `value`, a sample of `S` in binary64, is subnormal in `S`.
fn subnormal_in<S: Sample>(value: f64) -> bool {
    let smallest = match S::PRECISION {
        Precision::Binary64 => f64::MIN_POSITIVE,
        Precision::Binary32 => f64::from(f32::MIN_POSITIVE),
    };
    value != 0.0 && value.abs() < smallest
}

/// How many parameters `params` holds, none included, whose element nothing else tells.
const fn count(params: &[ParamSpec]) -> usize {
    params.len()
}

/// The box type `$name` of the parameters `$params`, whose instance is the [`Fixture`] a
/// function of the sample rate and the parameters chooses, in either precision.
macro_rules! box_type {
    ($name:ident, $params:expr, |$rate:pat_param, $given:pat_param| $body:expr) => {
        /// A fixture's box type.
        struct $name;
        impl Create<{ count($params) }> for $name {
            const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
            const PARAMS: &'static [ParamSpec; count($params)] = $params;

            fn create<S: Sample, const N: usize>(
                $rate: SampleRate,
                given: Values<{ count($params) }>,
            ) -> impl Driven<S, N, { count($params) }> + 'static {
                let raw: Vec<f64> = given.get().iter().map(|param| param.raw()).collect();
                let $given: &[f64] = &raw;
                $body
            }
        }
    };
}

/// A `gain~` spec, as [`GOOD`]'s description but for the parameters and ports given (by
/// default [`GAIN`], [`PORT`] and [`OUT`]), whose instance is the [`Fixture`] a function of
/// the sample rate and the parameters chooses: a static of its own box type.
macro_rules! gain {
    ($(params: $params:expr,)? $(inlets: $inlets:expr,)? $(outlets: $outlets:expr,)?
     |$rate:pat_param, $given:pat_param| $body:expr) => {{
        box_type!(Chosen, gain!(@or $($params)? ; &[GAIN]), |$rate, $given| $body);
        leaked(good::<Chosen, _>(
            "gain~",
            GAIN_DESCRIPTION,
            gain!(@or $($inlets)? ; &[PORT]),
            gain!(@or $($outlets)? ; &[OUT]),
        ))
    }};
    (@or $value:expr ; $default:expr) => { $value };
    (@or ; $default:expr) => { $default };
}

/// The spec `name` of the box type `T`, described by `description`, with the ports `inlets`
/// and `outlets`, needing no capability, at the first algorithm version.
const fn good<T: Create<K>, const K: usize>(
    name: &'static str,
    description: &'static str,
    inlets: &'static [PortSpec],
    outlets: &'static [PortSpec],
) -> ObjectSpec {
    ObjectSpec::new::<T, K>(
        name,
        description,
        inlets,
        outlets,
        &[],
        AlgorithmVersion::FIRST,
    )
}

box_type!(Good, &[GAIN], |_, params| Fixture::Gain(
    params.first().copied().unwrap_or(1.0)
));

/// A correct box: multiplies its input by its gain.
static GOOD: ObjectSpec = good::<Good, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &[OUT]);

static INSTANCES: AtomicU16 = AtomicU16::new(0);

/// `spec`, for the rest of the test process: the laws take a static box type, as a plan's
/// registry holds them.
fn leaked(spec: ObjectSpec) -> &'static ObjectSpec {
    Box::leak(Box::new(spec))
}

/// What [`check_laws`] says of a box whose first broken law gives `problem`: in the first
/// precision, at the first rate of the matrix.
fn refused(problem: &str) -> Result<(), String> {
    Err(format!("binary64, at 8000 Hz: {problem}"))
}

#[test]
fn a_correct_box_passes_every_law() {
    assert_eq!(check_laws(&GOOD), Ok(()), "gain~ respects the laws");
    let no_ports = gain!(params: &[], inlets: &[], outlets: &[], |_, _| Fixture::Gain(1.0));
    assert_eq!(check_laws(no_ports), Ok(()), "a box without ports");
}

#[test]
fn catches_non_finite_output() {
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Nan)),
        refused("special inputs: non-finite output with parameters [0.0]"),
        "NaN is caught"
    );
}

#[test]
fn catches_a_broken_law_in_either_precision() {
    for precision in [Precision::Binary64, Precision::Binary32] {
        let spec = match precision {
            Precision::Binary64 => gain!(|_, _| Fixture::NanIn(Precision::Binary64)),
            Precision::Binary32 => gain!(|_, _| Fixture::NanIn(Precision::Binary32)),
        };
        assert_eq!(
            check_laws(spec),
            Err(format!(
                "binary{}, at 8000 Hz: special inputs: non-finite output with parameters [0.0]",
                precision.bits()
            )),
            "NaN in one precision"
        );
    }
}

#[test]
fn catches_subnormal_output() {
    // The smallest subnormal of each precision: finite and deterministic, but not flushed.
    // Each precision is checked against its own smallest normal: 2^-149 is subnormal in
    // binary32 and normal in binary64, where 2^-1074 is subnormal.
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Subnormal)),
        refused("special inputs: subnormal output with parameters [0.0]"),
        "a subnormal is caught"
    );
    let narrow = gain!(|_, _| Fixture::Constant(f64::from(f32::from_bits(1))));
    assert_eq!(
        check_laws(narrow),
        Err(
            "binary32, at 8000 Hz: special inputs: subnormal output with parameters [0.0]"
                .to_owned()
        ),
        "a binary32 subnormal is caught"
    );
    let negative = gain!(|_, _| Fixture::Constant(f64::from(f32::from_bits(0x8040_0000))));
    assert_eq!(
        check_laws(negative),
        Err(
            "binary32, at 8000 Hz: special inputs: subnormal output with parameters [0.0]"
                .to_owned()
        ),
        "a negative subnormal is caught"
    );
    // The smallest normal and zeros of both signs are not subnormal.
    let not_subnormal = [
        gain!(params: &[], inlets: &[], |_, _| Fixture::Constant(f64::from(f32::MIN_POSITIVE))),
        gain!(params: &[], inlets: &[], |_, _| Fixture::Constant(-f64::from(f32::MIN_POSITIVE))),
        gain!(params: &[], inlets: &[], |_, _| Fixture::Constant(0.0)),
        gain!(params: &[], inlets: &[], |_, _| Fixture::Constant(-0.0)),
    ];
    for spec in not_subnormal {
        assert_eq!(check_laws(spec), Ok(()), "not subnormal");
    }
}

#[test]
fn catches_non_deterministic_output() {
    let spec = gain!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::Constant(f64::from(instance % 1000))
    });
    assert_eq!(
        check_laws(spec),
        refused("special inputs: not deterministic with parameters [0.0]"),
        "instances differ"
    );
    // In each precision on its own: the other one is silent, hence deterministic.
    let wide = gain!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::ConstantIn(Precision::Binary64, f64::from(instance % 1000))
    });
    assert_eq!(
        check_laws(wide),
        refused("special inputs: not deterministic with parameters [0.0]"),
        "binary64 instances differ"
    );
    let narrow = gain!(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Fixture::ConstantIn(Precision::Binary32, f64::from(instance % 1000))
    });
    assert_eq!(
        check_laws(narrow),
        Err(
            "binary32, at 8000 Hz: special inputs: not deterministic with parameters [0.0]"
                .to_owned()
        ),
        "binary32 instances differ"
    );
}

#[test]
fn catches_a_box_reading_its_outputs_in_either_precision() {
    for precision in [Precision::Binary64, Precision::Binary32] {
        let spec = match precision {
            Precision::Binary64 => gain!(|_, _| Fixture::ReadsOutputsIn(Precision::Binary64)),
            Precision::Binary32 => gain!(|_, _| Fixture::ReadsOutputsIn(Precision::Binary32)),
        };
        assert_eq!(
            check_laws(spec),
            Err(format!(
                "binary{}, at 8000 Hz: special inputs: reads its outputs, leaves some of their \
                 samples unwritten, or differs between instances, with parameters [0.0]",
                precision.bits()
            )),
            "its outputs on entry are read"
        );
    }
}

#[test]
fn catches_a_box_leaving_samples_of_its_outputs_unwritten() {
    // The first block is written, then every other one: an outputs fill set once before
    // the first block would not see it.
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Skips(0))),
        refused(
            "special inputs: reads its outputs, leaves some of their \
             samples unwritten, or differs between instances, with parameters [0.0]"
        ),
        "unwritten blocks are seen"
    );
}

/// The instances created by [`names_no_cause_its_renders_cannot_tell`], alone.
static PARITY: AtomicU16 = AtomicU16::new(0);

#[test]
fn names_no_cause_its_renders_cannot_tell() {
    // Every other instance outputs another constant: the first and third renders agree,
    // the second differs, as for a box reading its outputs, though this one never does.
    let spec = gain!(|_, _| {
        let instance = PARITY.fetch_add(1, Ordering::Relaxed);
        Fixture::Constant(f64::from(instance % 2) / 2.0)
    });
    assert_eq!(
        check_laws(spec),
        refused(
            "special inputs: reads its outputs, leaves some of their \
             samples unwritten, or differs between instances, with parameters [0.0]"
        ),
        "every cause the renders allow"
    );
}

#[test]
fn catches_a_box_leaving_its_outputs_unwritten_in_silence() {
    // Every block of the special and random inputs holds a nonzero sample, so only a run
    // fed silence sees the box skip its work: the long run, its outputs NaN on entry.
    let error = check_laws(gain!(|_, _| Fixture::SkipsWhenSilent))
        .expect_err("outputs left unwritten in silence are seen");
    assert_eq!(
        error,
        "binary64, at 8000 Hz: long run: non-finite output, or outputs read or left \
         unwritten (a NaN on entry), with parameters [0.0]",
        "the long run's entry"
    );
}

#[test]
fn holds_an_offline_only_box_to_every_law_but_the_heap() {
    /// An allocating box type, declared offline-only, which may allocate while processing.
    struct OfflineAllocating;
    impl Create<1> for OfflineAllocating {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::OfflineOnly;
        const PARAMS: &'static [ParamSpec; 1] = &[GAIN];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _given: Values<1>,
        ) -> impl Driven<S, N, 1> + 'static {
            Fixture::Allocating
        }
    }
    /// A box type outputting NaN, declared offline-only: still refused.
    struct OfflineNan;
    impl Create<1> for OfflineNan {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::OfflineOnly;
        const PARAMS: &'static [ParamSpec; 1] = &[GAIN];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _given: Values<1>,
        ) -> impl Driven<S, N, 1> + 'static {
            Fixture::Nan
        }
    }
    let allocating = leaked(good::<OfflineAllocating, 1>(
        "gain~",
        GAIN_DESCRIPTION,
        &[PORT],
        &[OUT],
    ));
    assert_eq!(check_laws(allocating), Ok(()), "offline-only, allocating");
    let nan = leaked(good::<OfflineNan, 1>(
        "gain~",
        GAIN_DESCRIPTION,
        &[PORT],
        &[OUT],
    ));
    assert_eq!(
        check_laws(nan),
        refused("special inputs: non-finite output with parameters [0.0]"),
        "offline-only, not finite"
    );
}

#[test]
fn catches_allocation_while_processing() {
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Allocating)),
        refused("special inputs: uses the heap while processing (16 heap operations)"),
        "allocation"
    );
}

#[test]
fn catches_a_release_while_processing() {
    let spec = gain!(|_, _| Fixture::Freeing(Some(vec![0_u8; 8])));
    assert_eq!(
        check_laws(spec),
        refused("special inputs: uses the heap while processing (1 heap operations)"),
        "a release"
    );
}

#[test]
fn checks_parameters_at_negative_zero_and_the_smallest_normal_and_never_below() {
    // NaN only for a parameter of -0.0, then only for the smallest normal binary32: none is
    // a minimum, default, midpoint or maximum. Parameters reach a box as `Params`, so a
    // box breaking on a nonzero parameter below 2^-126 in magnitude passes: no plan can give
    // it one.
    let negative_zero = gain!(|_, params| {
        if params.first().map(|value| value.to_bits()) == Some((-0.0_f64).to_bits()) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(negative_zero),
        refused("special inputs: non-finite output with parameters [-0.0]"),
        "-0.0 is tried"
    );
    let subnormal = gain!(|_, params| {
        if params
            .first()
            .is_some_and(|value| *value != 0.0_f64 && value.abs() < f64::from(f32::MIN_POSITIVE))
        {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(subnormal),
        Ok(()),
        "no parameter below 2^-126 in magnitude but zero reaches a box"
    );
    let normal = gain!(|_, params| {
        if params.first().copied() == Some(f64::from(f32::MIN_POSITIVE)) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(normal),
        refused("special inputs: non-finite output with parameters [1.1754943508222875e-38]"),
        "the smallest normal is tried"
    );
}

#[test]
fn tries_every_case_at_every_rate_with_six_parameter_sets() {
    static CREATED: AtomicU16 = AtomicU16::new(0);
    let spec = gain!(|_, _| {
        let _: u16 = CREATED.fetch_add(1, Ordering::Relaxed);
        Fixture::Gain(1.0)
    });
    assert_eq!(check_laws(spec), Ok(()), "a correct box");
    // Per precision and rate: (special inputs + 32 random cases) x 6 parameter sets x 2
    // runs, plus a long run per parameter set.
    let per_rate = (1_usize + 32) * 6 * 2 + 6;
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
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::NegativeIsNan)),
        refused("special inputs: non-finite output with parameters [0.0]"),
        "negatives"
    );
}

#[test]
fn checks_parameters_at_minimum_default_and_maximum() {
    // NaN only at the maximum gain: the law must try the maximum too.
    let spec = gain!(|_, params| {
        if params.first().copied() == Some(1.0) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(spec),
        refused("special inputs: non-finite output with parameters [1.0]"),
        "maximum is checked"
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
    let spec = gain!(params: &[], inlets: &TWO, outlets: &TWO, |_, _| Fixture::SecondInlet);
    assert_eq!(
        check_laws(spec),
        refused("special inputs: non-finite output with parameters []"),
        "the second inlet and outlet are exercised"
    );
}

#[test]
fn checks_every_sample_rate_of_the_matrix() {
    // NaN at every rate but 48 kHz: the laws must try the other rates.
    let spec = gain!(|rate, _| Fixture::Constant(if rate.hz() == 48_000 { 0.0 } else { f64::NAN }));
    assert_eq!(
        check_laws(spec),
        refused("special inputs: non-finite output with parameters [0.0]"),
        "other rates are checked"
    );
    for hz in crate::matrix::SAMPLE_RATES {
        let only = gain!(|rate, _| {
            Fixture::Constant(if rate.hz() == ONLY.load(Ordering::Relaxed) {
                f64::NAN
            } else {
                0.0
            })
        });
        ONLY.store(hz, Ordering::Relaxed);
        assert_eq!(
            check_laws(only),
            Err(format!(
                "binary64, at {hz} Hz: special inputs: non-finite output with parameters [0.0]"
            )),
            "each rate is checked"
        );
    }
}

/// The only rate at which a fixture box fails, set by the test.
static ONLY: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[test]
fn feeds_the_special_inputs() {
    // NaN only on a negative zero or a subnormal input of the precision: random inputs never
    // hit them, in either precision.
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Special)),
        refused("special inputs: non-finite output with parameters [0.0]"),
        "special inputs"
    );
}

#[test]
fn feeds_random_inputs() {
    // NaN only on inputs strictly between 0.25 and 0.75: no special input, no silence,
    // only the random inputs reach it. The input said is the one the fixed seed draws and the
    // locked proptest shrinks, which nothing of the environment changes
    // (`crate::runner::config`).
    let mut input = [[0.0_f64; audio_core::BLOCK_SIZE]; audio_battery::BLOCKS];
    input[7][60] = 0.746_097_994_397_458_3_f64;
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Middle)),
        refused(&format!(
            "Test failed: non-finite output with parameters [0.0].\n\
             minimal failing input: {input:#?}"
        )),
        "random inputs"
    );
}

#[test]
fn runs_long_enough_to_reveal_late_failures() {
    // Breaks only after 512 blocks: the short runs never see it.
    assert_eq!(
        check_laws(gain!(|_, _| Fixture::Drifts(0))),
        refused(
            "long run: non-finite output, or outputs read or left unwritten (a NaN on entry), \
             with parameters [0.0]"
        ),
        "long run"
    );
}

#[test]
fn checks_parameters_away_from_their_default() {
    // NaN only at 0.5, away from the default 0.25 and from both limits.
    const QUARTER: ParamSpec = audio_core::param!("gain", "gain", 0.25, 0.0, 1.0);
    let spec = gain!(params: &[QUARTER], |_, params| {
        if params.first().copied() == Some(0.5) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(spec),
        refused("special inputs: non-finite output with parameters [0.5]"),
        "the midpoint is checked"
    );
    // With the default at the midpoint (GAIN: 0.5 in [0, 1]), 0.75 is tried instead.
    let shifted = gain!(|_, params| {
        if params.first().copied() == Some(0.75) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(shifted),
        refused("special inputs: non-finite output with parameters [0.75]"),
        "a value away from the default"
    );
}

/// The parameters a law names are binary64, as a box is given them.
#[test]
fn names_parameters_in_binary64() {
    // A maximum binary32 does not hold, which it would show as 0.3.
    const FINE: ParamSpec = audio_core::param!("gain", "gain", 0.0, 0.0, 0.300_000_000_000_000_04);
    let spec = gain!(params: &[FINE], |_, params| {
        if params.first().copied() == Some(0.300_000_000_000_000_04) {
            Fixture::Nan
        } else {
            Fixture::Gain(1.0)
        }
    });
    assert_eq!(
        check_laws(spec),
        refused("special inputs: non-finite output with parameters [0.30000000000000004]"),
        "the maximum"
    );
}

/// Fixtures for the description cases.
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

static SPACED_PORT: [PortSpec; 1] = [PortSpec {
    name: "in ",
    description: "x",
}];
static BLANK_PORT: [PortSpec; 1] = [PortSpec {
    name: " ",
    description: "x",
}];
static HIDDEN_PORT: [PortSpec; 1] = [PortSpec {
    name: "in\u{202e}",
    description: "x",
}];
static LINES_PORT: [PortSpec; 1] = [PortSpec {
    name: "x",
    description: "a\nb",
}];

box_type!(DupParams, &DUP_PARAMS, |_, _| Fixture::Gain(1.0));
box_type!(
    HiddenParam,
    &[audio_core::param!("g\u{feff}ain", "x", 0.5, 0.0, 1.0)],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    SeparatedParam,
    &[audio_core::param!("gain", "\u{2028}", 0.5, 0.0, 1.0)],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    FineDefault,
    &[
        audio_core::param!("gain", "x", 0.5, 0.0, 1.0),
        audio_core::param!("tone", "x", 0.1, 0.0, 1.0)
    ],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    TinyDefault,
    &[audio_core::param!("gain", "x", 1e-45, 0.0, 1.0)],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    NormalDefault,
    &[audio_core::param!(
        "gain",
        "x",
        1.175_494_350_823_356_6e-38,
        0.0,
        1.0
    )],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    FineMinimum,
    &[audio_core::param!("gain", "x", 0.5, -0.1, 1.0)],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    FineMaximum,
    &[audio_core::param!("gain", "x", 0.5, 0.0, 1e300)],
    |_, _| Fixture::Gain(1.0)
);
box_type!(
    Undescribed,
    &[audio_core::param!("gain", "", 0.5, 0.0, 1.0)],
    |_, _| Fixture::Gain(1.0)
);

/// The description of [`GOOD`].
const GAIN_DESCRIPTION: &str = "multiplies by a gain";

/// Specs with one description problem each, and the expected message.
static INVALID_DESCRIPTIONS: [(ObjectSpec, &str); 21] = [
    (
        good::<Good, 1>("osc\u{200b}~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "box name \"osc\\u{200b}~\" holds U+200B",
    ),
    (
        good::<Good, 1>("gain~", "erase\u{1b}[2J", &[PORT], &[OUT]),
        "description text \"erase\\u{1b}[2J\" holds U+001B",
    ),
    (
        good::<Good, 1>("gain~", "\u{200b} ", &[PORT], &[OUT]),
        "description text is blank",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &HIDDEN_PORT, &[OUT]),
        "inlet name \"in\\u{202e}\" holds U+202E",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &LINES_PORT),
        "outlet \"x\": its description text \"a\\nb\" holds U+000A",
    ),
    (
        good::<HiddenParam, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "parameter name \"g\\u{feff}ain\" holds U+FEFF",
    ),
    (
        good::<SeparatedParam, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "parameter \"gain\": its description text \"\\u{2028}\" holds U+2028",
    ),
    (
        good::<Good, 1>("", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "box name is blank",
    ),
    (
        good::<Good, 1>("two words", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "box name \"two words\" holds whitespace",
    ),
    (
        good::<Good, 1>("two\u{a0}words", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "box name \"two\\u{a0}words\" holds U+00A0",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &SPACED_PORT, &[OUT]),
        "inlet name \"in \" starts or ends with a space, or holds two in a row",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &BLANK_PORT),
        "outlet name is blank",
    ),
    (
        ObjectSpec::new::<Good, 1>(
            "gain~",
            GAIN_DESCRIPTION,
            &[PORT],
            &[OUT],
            &[Capability::AudioOutput, Capability::AudioOutput],
            AlgorithmVersion::FIRST,
        ),
        "duplicate capability AudioOutput",
    ),
    (
        good::<Good, 1>("gain~", "  ", &[PORT], &[OUT]),
        "description text is blank",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &DUP_PORTS, &[OUT]),
        "duplicate inlet \"in\"",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &DUP_PORTS),
        "duplicate outlet \"in\"",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &EMPTY_PORT, &[OUT]),
        "inlet name is blank",
    ),
    (
        good::<DupParams, 2>("gain~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "duplicate parameter \"gain\"",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &UNDOCUMENTED_PORT, &[OUT]),
        "inlet \"x\": its description text is blank",
    ),
    (
        good::<Good, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &UNDOCUMENTED_PORT),
        "outlet \"x\": its description text is blank",
    ),
    (
        good::<Undescribed, 1>("gain~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
        "parameter \"gain\": its description text is blank",
    ),
];

#[test]
fn rejects_invalid_descriptions() {
    for (spec, message) in &INVALID_DESCRIPTIONS {
        assert_eq!(
            check_description(spec),
            Err((*message).to_owned()),
            "{message}"
        );
        assert_eq!(
            check_laws(spec),
            Err((*message).to_owned()),
            "laws check the description first"
        );
    }
    assert_eq!(check_description(&GOOD), Ok(()), "a valid description");
    // A capability asked for once is accepted: only the second asking of one is refused.
    let output = ObjectSpec::new::<Good, 1>(
        "gain~",
        GAIN_DESCRIPTION,
        &[PORT],
        &[OUT],
        &[Capability::AudioOutput],
        AlgorithmVersion::FIRST,
    );
    assert_eq!(check_description(&output), Ok(()), "one capability");
}

#[test]
fn checks_a_registry() {
    let other = good::<Good, 1>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]);
    assert_eq!(check_registry(&[&GOOD, &other]), Ok(()), "valid registry");
    assert_eq!(check_registry(&[]), Ok(()), "empty registry");
    assert_eq!(
        check_registry(&[&GOOD, &GOOD]),
        Err("duplicate box \"gain~\"".to_owned()),
        "unique names"
    );
    // A name a port could have, but not a box type.
    let spaced = good::<Good, 1>("a b", GAIN_DESCRIPTION, &[PORT], &[OUT]);
    assert_eq!(
        check_registry(&[&GOOD, &spaced]),
        Err("box name \"a b\" holds whitespace".to_owned()),
        "each name checked as a box type's, before every box"
    );
    let bad = good::<Good, 1>("other~", "", &[PORT], &[OUT]);
    assert_eq!(
        check_registry(&[&GOOD, &bad]),
        Err("other~: description text is blank".to_owned()),
        "every box is checked"
    );
    // A patch writes binary32 values until its format holds binary64 ones.
    for (spec, problem) in [
        (
            good::<FineDefault, 2>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
            "parameter \"tone\": its default 0.1",
        ),
        (
            good::<NormalDefault, 1>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
            "parameter \"gain\": its default 1.1754943508233566e-38",
        ),
        (
            good::<FineMinimum, 1>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
            "parameter \"gain\": its minimum -0.1",
        ),
        (
            good::<FineMaximum, 1>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]),
            "parameter \"gain\": its maximum 1e300",
        ),
    ] {
        assert_eq!(
            check_registry(&[&GOOD, &spec]),
            Err(format!(
                "other~: {problem} is not a binary32, as a patch of format version 1 writes a \
                 value (roadmap #3 step 7)"
            )),
            "{problem}"
        );
        assert_eq!(
            check_description(&spec),
            Ok(()),
            "{problem}: valid otherwise"
        );
    }
    // A default below 2^-126 in magnitude is held as a zero, which a patch writes exactly
    // (finding D2-n3 of the third deep review of #12 block 3): 2^-126 (1 + 2^-40) above is
    // not a binary32 and stays refused, being of the binade at 2^-126.
    let tiny = good::<TinyDefault, 1>("other~", GAIN_DESCRIPTION, &[PORT], &[OUT]);
    assert_eq!(check_registry(&[&GOOD, &tiny]), Ok(()), "a flushed default");
}

#[test]
fn renders_a_fresh_instance_of_the_box_type_of_its_values() {
    // The laws give each box type its own parameter sets, as `ParamValues`: values of
    // another box type cannot be given (`audio_core::Dispatch`).
    let rate = SampleRate::new(48_000).expect("valid rate");
    let values = audio_core::ParamValues::of(&GOOD, &[Binary64::new(0.5)]).expect("within range");
    assert_eq!(
        super::render::<Binary64>(&values, rate, &[], 1, Binary64::ZERO)
            .map(|(rendered, operations)| (super::bits(&rendered), operations)),
        Ok((vec![0_u64; 64], 0)),
        "its own: one block of +0 from silence, without the heap"
    );
}
