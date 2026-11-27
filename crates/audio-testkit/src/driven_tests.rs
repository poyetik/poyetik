//! Tests of the laws of a driven box: each must refuse a box whose bodies differ.

use core::hint::black_box;
use core::sync::atomic::{AtomicUsize, Ordering};

use audio_battery::{BLOCKS, splitmix64};
use audio_core::DrivenValues;
use audio_core::host::Raw as _;
use audio_core::{
    AlgorithmVersion, Binary32, Binary64, Block, BoxInstance, BoxMode, Create, Dispatch, Driven,
    Driving, Object, ObjectSpec, ParamSpec, ParamValues, PortSpec, Sample, SampleRate, Typed,
    Values,
};

use super::{check_driven_laws, drawn};

const IN: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};

/// Both bodies agree.
const RIGHT: u8 = 0;
/// The driven body takes the first values of a block for all of it.
const FIRST_FRAME: u8 = 1;
/// The driven body takes each value one frame late.
const LATE: u8 = 2;
/// The driven body derives its step by another formula.
const OTHER_FORMULA: u8 = 3;
/// Taking values restarts the phase.
const RESTARTS: u8 = 4;
/// Taking values does not derive the step again.
const KEEPS_STEP: u8 = 5;
/// The driven body does not advance the phase.
const FROZEN: u8 = 6;
/// The driven body swaps its parameters.
const SWAPPED: u8 = 7;
/// The driven body ignores its second parameter.
const ONE_OF_TWO: u8 = 8;
/// The driven body is the steady one.
const IGNORES: u8 = 9;
/// The driven body allocates.
const ALLOCATES: u8 = 10;
/// The driven body allocates, of an offline-only box.
const ALLOCATES_OFFLINE: u8 = 11;
/// The driven body leaves its first sample unwritten.
const UNWRITTEN: u8 = 12;
/// The driven body takes a value every 16 frames.
const EVERY_16: u8 = 13;
/// The instance refuses its values set.
const REFUSES: u8 = 14;
/// The instance refuses its second block of values.
const REFUSES_BLOCKS: u8 = 15;
/// Both bodies output an infinity, alike.
const INFINITE: u8 = 16;
/// The box type is not created.
const UNCREATED: u8 = 17;
/// The driven body advances a phase of its own, apart from the steady body's.
const SEPARATE_PHASE: u8 = 18;
/// Taking values allocates.
const ALLOCATES_IN_SET: u8 = 19;
/// The driven body keeps a value equal to the last one it took, which takes -0 for +0.
const EQUAL_SKIP: u8 = 20;
/// The driven body takes its second parameter only at a frame where its first changes.
const CHANGE_ON_FIRST: u8 = 21;
/// The driven body derives its step at a fixed rate of 8 kHz.
const FIXED_RATE: u8 = 22;
/// The driven body rounds its values to the precision it computes in.
const ROUNDED_VALUES: u8 = 23;
/// The driven body takes a block whose first two frames are equal as constant.
const FIRST_TWO: u8 = 24;
/// The driven body multiplies its input by 3, then divides it by 3, which gives back every
/// special input, a power of two or zero.
const THIRDS: u8 = 25;
/// Both bodies output a subnormal number, alike.
const SUBNORMAL: u8 = 26;
/// An instance created with values takes the default gain, whatever it is given.
const CREATED_OTHERWISE: u8 = 27;
/// The driven body advances a phase of its own until the steady body has rendered a block.
const SEPARATE_AT_FIRST: u8 = 28;
/// The driven body leaves the steady one the phase after the first block of its run.
const FIRST_OF_RUN: u8 = 29;
/// The driven body takes a block whose first eight frames are equal as constant.
const FIRST_EIGHT: u8 = 30;
/// The instance refuses its values set the sixth time.
const REFUSES_LATER: u8 = 31;
/// The steady body restarts the phase in a block it was not given values before, nor created
/// before.
const STALE: u8 = 32;

/// Outputs a ramp of `hertz` cycles per second, with a quarter of its input, times a gain;
/// `CHEAT` says how it goes wrong.
#[derive(Debug)]
struct Probe<const CHEAT: u8> {
    /// Its phase, in cycles.
    phase: f64,
    /// The phase of the driven body of [`SEPARATE_PHASE`], [`SEPARATE_AT_FIRST`] and
    /// [`FIRST_OF_RUN`], in cycles.
    separate: f64,
    /// Whether the steady body has rendered a block.
    steadied: bool,
    /// Whether the driven body rendered the last block.
    driven: bool,
    /// Whether it was created or given values since the steady body last rendered a block.
    fresh: bool,
    /// The bits of the first parameter the driven body of [`CHANGE_ON_FIRST`] last took.
    first: u64,
    /// Cycles per frame.
    step: f64,
    /// Its gain.
    gain: f64,
    /// The sample rate, in hertz.
    rate: f64,
}

impl<const CHEAT: u8> Probe<CHEAT> {
    /// At `rate`, from its parameters.
    fn of(rate: SampleRate, values: Values<2>) -> Self {
        let [hertz, gain] = values.get();
        let rate = f64::from(rate.hz());
        Self {
            phase: 0.0,
            separate: 0.0,
            steadied: false,
            driven: false,
            fresh: true,
            first: hertz.raw().to_bits(),
            step: hertz.raw() / rate,
            gain: if CHEAT == CREATED_OTHERWISE {
                0.5_f64
            } else {
                gain.raw()
            },
            rate,
        }
    }

    /// The next sample, of `input`: the arithmetic of both bodies.
    fn frame<S: Sample>(&mut self, input: S) -> S {
        let wide = input.to_binary64().raw();
        let sample = (self.phase + wide * 0.25_f64) * self.gain;
        self.phase = (self.phase + self.step).fract();
        match CHEAT {
            INFINITE => S::from_binary64(Binary64::new(f64::INFINITY)),
            SUBNORMAL => S::from_binary64(Binary64::new(f64::MIN_POSITIVE / 2.0_f64)),
            _ => S::from_binary64(Binary64::new(sample)).flush(),
        }
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Object<S, N> for Probe<CHEAT> {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.steadied = true;
        self.driven = false;
        if CHEAT == STALE && !self.fresh {
            self.phase = 0.0_f64;
        }
        self.fresh = false;
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                *out = self.frame(*value);
            }
        }
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Driven<S, N, 2> for Probe<CHEAT> {
    fn set(&mut self, values: Values<2>) {
        let [hertz, gain] = values.get();
        if CHEAT == ALLOCATES_IN_SET {
            drop(black_box(Vec::<u8>::with_capacity(8)));
        }
        self.first = hertz.raw().to_bits();
        if CHEAT != KEEPS_STEP {
            self.step = hertz.raw() / self.rate;
        }
        self.gain = gain.raw();
        self.fresh = true;
        if CHEAT == RESTARTS {
            self.phase = 0.0_f64;
        }
    }

    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        driving: Driving<'_, N, 2>,
    ) {
        let [hertz, gains] = driving.blocks();
        if CHEAT == IGNORES {
            self.process(inputs, outputs);
            return;
        }
        if CHEAT == ALLOCATES || CHEAT == ALLOCATES_OFFLINE {
            drop(black_box(Vec::<u8>::with_capacity(8)));
        }
        let first_of_run = !self.driven;
        self.driven = true;
        if CHEAT == FIRST_OF_RUN && first_of_run {
            self.separate = self.phase;
        }
        let apart = CHEAT == SEPARATE_PHASE
            || CHEAT == FIRST_OF_RUN
            || (CHEAT == SEPARATE_AT_FIRST && !self.steadied);
        if apart {
            core::mem::swap(&mut self.phase, &mut self.separate);
        }
        let opening = match CHEAT {
            FIRST_TWO => 2,
            FIRST_EIGHT => 8,
            _ => 0,
        };
        let same = |block: &Block<Binary64, N>| {
            block
                .iter()
                .take(opening)
                .all(|value| value.raw().to_bits() == block[0].raw().to_bits())
        };
        let constant = opening > 0 && same(hertz) && same(gains);
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (frame, (out, value)) in output.iter_mut().zip(input).enumerate() {
                let at = match CHEAT {
                    FIRST_FRAME => 0,
                    LATE => frame.saturating_sub(1),
                    EVERY_16 => frame.saturating_sub(frame % 16),
                    _ if constant => 0,
                    _ => frame,
                };
                let (mut tone, mut gain) = (hertz[at], gains[at]);
                if CHEAT == ROUNDED_VALUES {
                    (tone, gain) = (
                        S::from_binary64(tone).to_binary64(),
                        S::from_binary64(gain).to_binary64(),
                    );
                }
                let (mut tone, mut gain) = (tone.raw(), gain.raw());
                if CHEAT == SWAPPED {
                    (tone, gain) = (gain, tone);
                }
                let changed = tone.to_bits() != self.first;
                self.first = tone.to_bits();
                let late = CHEAT == LATE && frame == 0;
                if !late {
                    let rate = if CHEAT == FIXED_RATE {
                        8_000.0_f64
                    } else {
                        self.rate
                    };
                    self.step = if CHEAT == OTHER_FORMULA {
                        tone * (1.0_f64 / rate)
                    } else {
                        tone / rate
                    };
                    let kept = match CHEAT {
                        ONE_OF_TWO => true,
                        EQUAL_SKIP => Binary64::new(gain) == Binary64::new(self.gain),
                        CHANGE_ON_FIRST => !changed,
                        _ => false,
                    };
                    if !kept {
                        self.gain = gain;
                    }
                }
                let before = self.phase;
                let given = if CHEAT == THIRDS {
                    let wide = value.to_binary64().raw();
                    S::from_binary64(Binary64::new(wide * 3.0_f64 / 3.0_f64))
                } else {
                    *value
                };
                let sample = self.frame(given);
                if CHEAT == FROZEN {
                    self.phase = before;
                }
                if CHEAT != UNWRITTEN || frame > 0 {
                    *out = sample;
                }
            }
        }
        if apart {
            core::mem::swap(&mut self.phase, &mut self.separate);
        }
        if CHEAT == FIRST_OF_RUN && first_of_run {
            self.phase = self.separate;
        }
    }
}

impl<const CHEAT: u8> Create<2> for Probe<CHEAT> {
    const BOX_MODE: BoxMode = if CHEAT == ALLOCATES_OFFLINE {
        BoxMode::OfflineOnly
    } else {
        BoxMode::LiveCapable
    };
    const PARAMS: &'static [ParamSpec; 2] = &[
        audio_core::param!("hertz", "cycles per second", 100.0, 0.0, 1_000.0),
        audio_core::param!("gain", "how loud", 0.5, 0.0, 1.0),
    ];

    fn create<S: Sample, const N: usize>(
        rate: SampleRate,
        params: Values<2>,
    ) -> impl Driven<S, N, 2> + 'static {
        Self::of(rate, params)
    }
}

/// A [`Probe`] created at any engine block, as `audio_boxes::BuiltIn` creates a built-in box.
#[derive(Debug)]
struct Held<const CHEAT: u8> {
    /// The probe.
    probe: Typed<Probe<CHEAT>, 2>,
    /// How many times it was given values, or blocks of them.
    given: u32,
}

impl<S: Sample, const N: usize, const CHEAT: u8> Object<S, N> for Held<CHEAT> {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.probe.process(inputs, outputs);
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> BoxInstance<S, N> for Held<CHEAT> {
    fn receive(&mut self, values: &ParamValues) -> bool {
        self.given = self.given.saturating_add(1);
        CHEAT != REFUSES
            && !(CHEAT == REFUSES_LATER && self.given == 6)
            && BoxInstance::<S, N>::receive(&mut self.probe, values)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool {
        self.given = self.given.saturating_add(1);
        !(CHEAT == REFUSES_BLOCKS && self.given == 2)
            && self.probe.run_driven(inputs, outputs, values)
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Dispatch<S, N> for Held<CHEAT> {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        if CHEAT == UNCREATED {
            return None;
        }
        let params = values.params().array()?;
        Some(Self {
            probe: Typed::new(values.spec(), Probe::of(rate, params)),
            given: 0,
        })
    }
}

/// What a [`Counted`] counts, per precision, block and rate.
const COUNTED: usize = 4;

/// Its counts: per precision (binary64, binary32), engine block (1 frame, 64), rate (each of
/// the matrix, then any other) and what (created, given values, given blocks of values,
/// rendered by the steady body), in that order.
static COUNTS: [AtomicUsize; 2 * 2 * 9 * COUNTED] =
    [const { AtomicUsize::new(0) }; 2 * 2 * 9 * COUNTED];

/// A [`Held`] probe that counts what is done with it, in [`COUNTS`].
#[derive(Debug)]
struct Counted {
    /// The probe.
    held: Held<RIGHT>,
    /// Where its counts begin.
    first: usize,
}

impl Counted {
    /// Counts one more of `what` at its place.
    fn count(&self, what: usize) {
        if let Some(count) = COUNTS.get(self.first.saturating_add(what)) {
            let _: usize = count.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl<S: Sample, const N: usize> Object<S, N> for Counted {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.count(3);
        self.held.process(inputs, outputs);
    }
}

impl<S: Sample, const N: usize> BoxInstance<S, N> for Counted {
    fn receive(&mut self, values: &ParamValues) -> bool {
        self.count(1);
        BoxInstance::<S, N>::receive(&mut self.held, values)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool {
        self.count(2);
        self.held.run_driven(inputs, outputs, values)
    }
}

impl<S: Sample, const N: usize> Dispatch<S, N> for Counted {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        let precision = usize::from(size_of::<S>() == 4);
        let block = usize::from(N != 1);
        let hz = audio_battery::matrix::SAMPLE_RATES
            .iter()
            .position(|&hz| hz == rate.hz())
            .unwrap_or(8);
        let counted = Self {
            held: <Held<RIGHT> as Dispatch<S, N>>::create(rate, values)?,
            first: precision
                .wrapping_mul(2)
                .wrapping_add(block)
                .wrapping_mul(9)
                .wrapping_add(hz)
                .wrapping_mul(COUNTED),
        };
        counted.count(0);
        Some(counted)
    }
}

/// The spec of the box type `T`, with the outlets `outlets`, for the rest of the test process.
fn spec<T: Create<K>, const K: usize>(outlets: &'static [PortSpec]) -> &'static ObjectSpec {
    Box::leak(Box::new(ObjectSpec::new::<T, K>(
        "probe~",
        "a ramp",
        &[IN],
        outlets,
        &[],
        AlgorithmVersion::FIRST,
    )))
}

/// What the laws say of the probe that goes wrong as `CHEAT` says, every frame checked.
fn laws<const CHEAT: u8>() -> Result<(), String> {
    check_driven_laws::<Held<CHEAT>, Held<CHEAT>, Held<CHEAT>, Held<CHEAT>, 1>(spec::<
        Probe<CHEAT>,
        2,
    >(&[OUT]))
}

/// What they say of it every 16 frames.
fn coarse_laws<const CHEAT: u8>() -> Result<(), String> {
    check_driven_laws::<Held<CHEAT>, Held<CHEAT>, Held<CHEAT>, Held<CHEAT>, 16>(spec::<
        Probe<CHEAT>,
        2,
    >(&[OUT]))
}

/// What they say of it through its factory, which holds no engine block below 16 frames.
fn factory_laws<const CHEAT: u8>() -> Result<(), String> {
    check_driven_laws::<
        Box<dyn BoxInstance<Binary64>>,
        Box<dyn BoxInstance<Binary64, 16>>,
        Box<dyn BoxInstance<Binary32>>,
        Box<dyn BoxInstance<Binary32, 16>>,
        16,
    >(spec::<Probe<CHEAT>, 2>(&[OUT]))
}

#[test]
fn accepts_a_box_whose_bodies_agree() {
    assert_eq!(laws::<RIGHT>(), Ok(()), "every frame");
    assert_eq!(coarse_laws::<RIGHT>(), Ok(()), "every 16 frames");
    assert_eq!(factory_laws::<RIGHT>(), Ok(()), "through its factory");
}

/// The control of the laws: every run, held set and instance they create, at every rate of
/// the matrix, in both precisions, on both inputs, counted by a probe, against counts made
/// from the laws' documentation alone. Passed before it, and refused by it: a law reading
/// two rates only, checking how an instance is created for one held set only, or changing the
/// values once per block in its first two steps only.
#[test]
fn renders_every_run_at_every_rate_in_both_precisions() {
    assert_eq!(
        check_driven_laws::<Counted, Counted, Counted, Counted, 1>(spec::<Probe<RIGHT>, 2>(&[OUT])),
        Ok(()),
        "a box whose bodies agree"
    );
    // The motions: 6 parameter sets held, then the ends, a draw, 8 steps (64 frames a block
    // over 8 blocks), the zeros and each of the 2 parameters alone.
    let motions = 6 + 2 + 8 + 1 + 2;
    let runs = motions * 5;
    let inputs = 2;
    // The steady blocks of the 5 patterns of a run, 0, 4, 4, 4 and 6, of which 3 kept, given
    // no values; the others driven, 8, 4, 4, 4 and 2.
    let (steady, kept, driven) = (18, 3, 22);
    // The reference renders each run frame by frame, given values before each of its 512.
    let reference = [runs, runs * 512, 0, runs * 512].map(|count| count * inputs);
    // As the engine renders it: each run, then each held set created and given, both
    // instances rendering 8 blocks by the steady body.
    let engine = [
        runs + 6 * 2,
        motions * (steady - kept) + 6,
        motions * driven,
        motions * steady + 6 * 2 * 8,
    ]
    .map(|count| count * inputs);
    let read = |first: usize| -> Vec<usize> {
        COUNTS[first * COUNTED..(first + 1) * COUNTED]
            .iter()
            .map(|count| count.load(Ordering::Relaxed))
            .collect()
    };
    for (precision, bits) in [(0, 64_u32), (1, 32)] {
        for hz in 0..9 {
            let (expected, name) = if hz < 8 {
                (
                    [reference.to_vec(), engine.to_vec()],
                    audio_battery::matrix::SAMPLE_RATES[hz].to_string(),
                )
            } else {
                ([vec![0; 4], vec![0; 4]], "another rate".to_owned())
            };
            assert_eq!(
                [0, 1].map(|block| read((precision * 2 + block) * 9 + hz)),
                expected,
                "binary{bits} at {name}: by the reference, then as the engine renders it, \
                 created, given values, given blocks of values, rendered by the steady body"
            );
        }
    }
}

/// What the laws say of a box whose first broken one gives `problem`: in `precision` bits, at
/// `hertz`, on `input`, at the run of `motion` whose steady blocks are `steady`.
fn refused_at(
    precision: u32,
    hertz: u32,
    input: &str,
    motion: &str,
    steady: &str,
    problem: &str,
) -> Result<(), String> {
    Err(format!(
        "binary{precision}, at {hertz} Hz: {input}, {motion}, steady blocks {steady}: {problem}"
    ))
}

/// What the laws say of a box whose first broken one gives `problem`: in the first precision,
/// at the first rate of the matrix, on the first input, at the first run of `motion`.
fn refused(motion: &str, problem: &str) -> Result<(), String> {
    refused_at(64, 8_000, "special inputs", motion, "[]", problem)
}

/// What the laws say of a first differing sample: what the engine's run gives for what the
/// reference gives, where, and the parameters there.
fn other(samples: &str, place: &str, params: &str) -> String {
    format!(
        "the run as the engine renders it gives other bits than the reference, {samples}, at \
         {place}, with parameters {params}"
    )
}

/// The motion between the ends of the ranges.
const ENDS: &str = "every parameter from one end of its range to the other";

/// The parameter set held that is the defaults.
const DEFAULTS: &str = "parameter set 1 held";

/// The steady blocks of a run where every other block is steady, from the first.
const ALTERNATE: &str = "[0, 2, 4, 6]";

/// Those of one where every other block is steady, from the second.
const ALTERNATE_LATER: &str = "[1, 3, 5, 7]";

/// Those of the run of both bodies.
const RUNS: &str = "[2, 3, 4, 6]";

/// The second frame of a run.
const SECOND: &str = "block 0, outlet 0, frame 1";

/// The samples, places and parameters of these tests are those of a model of the probe, of the
/// values and of the inputs written apart, in another language, which the tree does not keep:
/// they are fixed by review.
#[test]
fn refuses_a_box_whose_bodies_differ() {
    const ONE_FRAME_OLD: &str = "0.0 (0x0) for 0.125 (0x3fc0000000000000)";
    let ends = "[0.0, 1.0]";
    for (name, motion, problem, result) in [
        // Right while its values stay, so first seen when they change at each frame.
        (
            "the first frame only",
            ENDS,
            other(ONE_FRAME_OLD, SECOND, ends),
            laws::<FIRST_FRAME>(),
        ),
        (
            "one frame late",
            ENDS,
            other(ONE_FRAME_OLD, SECOND, ends),
            laws::<LATE>(),
        ),
        (
            "a step kept",
            ENDS,
            other(
                "0.125 (0x3fc0000000000000) for 0.0125 (0x3f8999999999999a)",
                SECOND,
                ends,
            ),
            laws::<KEEPS_STEP>(),
        ),
        (
            "one parameter ignored",
            ENDS,
            other(
                "0.0625 (0x3fb0000000000000) for 0.125 (0x3fc0000000000000)",
                SECOND,
                ends,
            ),
            laws::<ONE_OF_TWO>(),
        ),
        (
            "every parameter ignored",
            ENDS,
            other(
                "0.00625 (0x3f7999999999999a) for 0.125 (0x3fc0000000000000)",
                SECOND,
                ends,
            ),
            laws::<IGNORES>(),
        ),
        (
            "a value every 16 frames",
            ENDS,
            other(ONE_FRAME_OLD, SECOND, ends),
            laws::<EVERY_16>(),
        ),
        (
            "the first frame only, through its factory",
            ENDS,
            other(
                "0.0 (0x0) for -0.0 (0x8000000000000000)",
                "block 0, outlet 0, frame 21",
                ends,
            ),
            factory_laws::<FIRST_FRAME>(),
        ),
    ] {
        assert_eq!(result, refused(motion, &problem), "{name}");
    }
}

#[test]
fn refuses_a_box_whose_bodies_differ_on_values_drawn_or_held() {
    for (name, motion, problem, result) in [
        // The two formulas agree at 8 kHz at both ends of the range: seen there by the values
        // drawn.
        (
            "another formula",
            "every parameter drawn",
            other(
                "0.10719881810038981 (0x3fbb7161b9e93912) for 0.1071988181003898 \
                 (0x3fbb7161b9e93911)",
                SECOND,
                "[26.433771592597743, 0.9708819781538285]",
            ),
            laws::<OTHER_FORMULA>(),
        ),
        // Wrong while its values stay, but silent at a gain of 0, the first set's: seen at the
        // second one, the defaults.
        (
            "a phase restarted",
            DEFAULTS,
            other(
                "0.00625 (0x3f7999999999999a) for 0.0 (0x0)",
                SECOND,
                "[100.0, 0.5]",
            ),
            laws::<RESTARTS>(),
        ),
        (
            "a phase frozen",
            DEFAULTS,
            other(
                "0.0 (0x0) for 0.00625 (0x3f7999999999999a)",
                SECOND,
                "[100.0, 0.5]",
            ),
            laws::<FROZEN>(),
        ),
        (
            "parameters swapped",
            DEFAULTS,
            other(
                "0.01875 (0x3f93333333333333) for 0.018750000000000003 (0x3f93333333333334)",
                "block 0, outlet 0, frame 3",
                "[100.0, 0.5]",
            ),
            laws::<SWAPPED>(),
        ),
        // Whatever its values.
        (
            "a sample unwritten",
            "parameter set 0 held",
            "the run as the engine renders it leaves a sample unwritten, for 0.0 (0x0), at block \
             0, outlet 0, frame 0, with parameters [0.0, 0.0]"
                .to_owned(),
            laws::<UNWRITTEN>(),
        ),
    ] {
        assert_eq!(result, refused(motion, &problem), "{name}");
    }
}

#[test]
fn refuses_a_box_wrong_in_one_dimension_of_the_runs_only() {
    const HEAP: &str = "uses the heap (8 heap operations)";
    let special = "special inputs";
    let start = "block 1, outlet 0, frame 0";
    let defaults = "[100.0, 0.5]";
    for (name, result, expected) in [
        // Right whenever the driven body follows itself: seen once the steady one renders.
        (
            "a phase of its own",
            laws::<SEPARATE_PHASE>(),
            refused_at(
                64,
                8_000,
                special,
                DEFAULTS,
                ALTERNATE,
                &other(
                    "0.0 (0x0) for 0.3999999999999996 (0x3fd9999999999992)",
                    start,
                    defaults,
                ),
            ),
        ),
        // Right whenever the steady body renders first, or never.
        (
            "a phase of its own until the steady body renders",
            laws::<SEPARATE_AT_FIRST>(),
            refused_at(
                64,
                8_000,
                special,
                DEFAULTS,
                ALTERNATE_LATER,
                &other(
                    "0.0 (0x0) for 0.3999999999999996 (0x3fd9999999999992)",
                    start,
                    defaults,
                ),
            ),
        ),
        // Right whenever the driven body renders one block at a time, or every block.
        (
            "the phase of the first block of a run",
            laws::<FIRST_OF_RUN>(),
            refused_at(
                64,
                8_000,
                special,
                DEFAULTS,
                RUNS,
                &other(
                    "0.3999999999999996 (0x3fd9999999999992) for 0.2999999999999992 \
                     (0x3fd3333333333325)",
                    "block 2, outlet 0, frame 0",
                    defaults,
                ),
            ),
        ),
        // Right whenever the steady body is given values before each block.
        (
            "a phase restarted in a block given no values",
            laws::<STALE>(),
            refused_at(
                64,
                8_000,
                special,
                DEFAULTS,
                "[0, 1, 3, 4, 5, 7], [1, 4, 5] given no values",
                &other(
                    "0.0 (0x0) for 0.3999999999999996 (0x3fd9999999999992)",
                    start,
                    defaults,
                ),
            ),
        ),
        (
            "the heap while taking values",
            laws::<ALLOCATES_IN_SET>(),
            refused_at(64, 8_000, special, "parameter set 0 held", ALTERNATE, HEAP),
        ),
    ] {
        assert_eq!(result, expected, "{name}");
    }
}

#[test]
fn refuses_a_box_wrong_on_some_changes_of_its_values_only() {
    for (name, result, expected) in [
        // Right while no value changes to an equal one.
        (
            "-0 taken for +0",
            laws::<EQUAL_SKIP>(),
            refused(
                "every parameter from one zero to the other",
                &other(
                    "0.0 (0x0) for -0.0 (0x8000000000000000)",
                    SECOND,
                    "[-0.0, -0.0]",
                ),
            ),
        ),
        // Right while every parameter changes with the first.
        (
            "a parameter taken with the first only",
            laws::<CHANGE_ON_FIRST>(),
            refused(
                "parameter 1 drawn alone",
                &other(
                    "0.00625 (0x3f7999999999999a) for 0.005394099963106375 (0x3f76181faeed2514)",
                    SECOND,
                    "[100.0, 0.43152799704850997]",
                ),
            ),
        ),
        // Right while the values do not change at the second frame of a block.
        (
            "a block of two equal frames taken as constant",
            laws::<FIRST_TWO>(),
            refused(
                "one change per block, from frame 0 in the first",
                &other(
                    "0.0 (0x0) for 0.125 (0x3fc0000000000000)",
                    "block 2, outlet 0, frame 2",
                    "[1000.0, 1.0]",
                ),
            ),
        ),
        // Right while the values change within the first eight frames of a block.
        (
            "a block of eight equal frames taken as constant",
            laws::<FIRST_EIGHT>(),
            refused(
                "one change per block, from frame 8 in the first",
                &other(
                    "0.0 (0x0) for 0.125 (0x3fc0000000000000)",
                    "block 0, outlet 0, frame 9",
                    "[1000.0, 1.0]",
                ),
            ),
        ),
    ] {
        assert_eq!(result, expected, "{name}");
    }
}

#[test]
fn refuses_a_box_wrong_on_some_rates_inputs_or_precisions_only() {
    let special = "special inputs";
    let random = "random inputs";
    let row = "[]";
    let defaults = "[100.0, 0.5]";
    for (name, result, expected) in [
        // Right at the first rate.
        (
            "a fixed rate",
            laws::<FIXED_RATE>(),
            refused_at(
                64,
                11_025,
                special,
                DEFAULTS,
                row,
                &other(
                    "0.00625 (0x3f7999999999999a) for 0.0045351473922902496 (0x3f7293725bb804a5)",
                    SECOND,
                    defaults,
                ),
            ),
        ),
        // Right on every special input.
        (
            "its input multiplied by 3, then divided by 3",
            laws::<THIRDS>(),
            refused_at(
                64,
                8_000,
                random,
                DEFAULTS,
                row,
                &other(
                    "0.13183717169104323 (0x3fc0e00a5a67b393) for 0.13183717169104325 \
                     (0x3fc0e00a5a67b394)",
                    "block 0, outlet 0, frame 6",
                    defaults,
                ),
            ),
        ),
        // Right in binary64, and on the parameter sets, which binary32 holds (500 and 0.75 away
        // from the defaults): seen by the values drawn.
        (
            "values rounded to binary32",
            laws::<ROUNDED_VALUES>(),
            refused_at(
                32,
                8_000,
                special,
                "every parameter drawn",
                row,
                &other(
                    "0.2659332752227783 (0x3e882868) for 0.2659333050251007 (0x3e882869)",
                    "block 0, outlet 0, frame 6",
                    "[523.9505916549513, 0.5551675161334325]",
                ),
            ),
        ),
        (
            "a subnormal output",
            laws::<SUBNORMAL>(),
            refused(
                "parameter set 0 held",
                "a sample is not finite, or is subnormal, 1.1125369292536007e-308 \
                 (0x8000000000000) at block 0, outlet 0, frame 0, with parameters [0.0, 0.0]",
            ),
        ),
        // Right by every run, since the reference gives values before every block.
        (
            "created with the default gain",
            laws::<CREATED_OTHERWISE>(),
            Err(
                "binary64, at 8000 Hz: special inputs, parameter set 0 held: an instance created \
                 with its values gives other bits than one created with the defaults, then given \
                 them, 0.125 (0x3fc0000000000000) for 0.0 (0x0), at block 0, outlet 0, frame 6, \
                 with parameters [0.0, 0.0]"
                    .to_owned(),
            ),
        ),
    ] {
        assert_eq!(result, expected, "{name}");
    }
}

/// The words of each motion and each place, which no refusal above needs all of.
#[test]
fn names_each_motion_and_place() {
    use super::{Motion, Place};
    for (motion, words) in [
        (Motion::Held(5), "parameter set 5 held"),
        (Motion::Ends, ENDS),
        (Motion::Random, "every parameter drawn"),
        (
            Motion::Step(7),
            "one change per block, from frame 56 in the first",
        ),
        (Motion::Zeros, "every parameter from one zero to the other"),
        (Motion::Alone(3), "parameter 3 drawn alone"),
    ] {
        assert_eq!(motion.to_string(), words, "{motion:?}");
    }
    // Two outlets, blocks of 64 frames.
    for (index, words) in [
        (0, "block 0, outlet 0, frame 0"),
        (63, "block 0, outlet 0, frame 63"),
        (64, "block 0, outlet 1, frame 0"),
        (323, "block 2, outlet 1, frame 3"),
    ] {
        assert_eq!(Place::of(index, 2).to_string(), words, "sample {index}");
    }
}

/// The values a law names are those of the place's frame, and of the part that holds it when
/// the reference's blocks are longer than a frame.
#[test]
fn names_the_values_at_a_place() {
    use super::{Motion, Place, Run};
    let spec = spec::<Probe<RIGHT>, 2>(&[OUT]);
    let motion = || (Motion::Ends, ParamValues::defaults(spec));
    let place = |frame| Place {
        block: 1,
        outlet: 0,
        frame,
    };
    let fine = Run::new::<1>(spec, motion(), [super::Body::Driven; BLOCKS]);
    let coarse = Run::new::<16>(spec, motion(), [super::Body::Driven; BLOCKS]);
    let (up, down) = ([1_000.0_f64, 0.0_f64], [0.0_f64, 1.0_f64]);
    for (frame, at_each, at_every_16) in [
        (0, up, up),
        (15, down, up),
        (17, down, down),
        (62, up, down),
    ] {
        assert_eq!(
            (
                fine.values_at::<1>(place(frame)),
                coarse.values_at::<16>(place(frame))
            ),
            (at_each.to_vec(), at_every_16.to_vec()),
            "frame {frame}"
        );
    }
}

/// The bits of the values of `run`'s frames `frames` of its block `block`, a frame a pair.
fn moved(run: &super::Run, block: usize, frames: core::ops::Range<usize>) -> Vec<[u64; 2]> {
    run.blocks[block].parts[frames]
        .iter()
        .map(|values| {
            let [hertz, gain] = values.params().array().expect("two parameters").get();
            [hertz.raw().to_bits(), gain.raw().to_bits()]
        })
        .collect()
}

#[test]
fn moves_each_parameter_as_its_motion_says() {
    use super::{Motion, Run};
    let spec = spec::<Probe<RIGHT>, 2>(&[OUT]);
    let defaults = ParamValues::defaults(spec);
    let run = |motion: Motion| {
        Run::new::<1>(
            spec,
            (motion, defaults.clone()),
            [super::Body::Driven; BLOCKS],
        )
    };
    let (top, zero) = (1_000.0_f64.to_bits(), 0.0_f64.to_bits());
    let one = 1.0_f64.to_bits();
    assert_eq!(
        moved(&run(Motion::Ends), 0, 0..3),
        [[top, zero], [zero, one], [top, zero]],
        "from the maximum, a parameter apart from the next, at each frame"
    );
    let step = run(Motion::Step(0));
    assert_eq!(
        (
            moved(&step, 0, 0..2),
            moved(&step, 1, 0..2),
            moved(&step, 2, 1..3)
        ),
        (
            vec![[top, one]; 2],
            vec![[top, one], [zero, zero]],
            vec![[zero, zero], [top, one]]
        ),
        "at frame 0 of block 0, 1 of block 1, 2 of block 2: down, then up"
    );
    let minus = (-0.0_f64).to_bits();
    assert_eq!(
        moved(&run(Motion::Zeros), 0, 0..2),
        [[zero, zero], [minus, minus]],
        "+0, then -0"
    );
    let alone = moved(&run(Motion::Alone(1)), 0, 0..2);
    assert_eq!(
        alone.iter().map(|[hertz, _]| *hertz).collect::<Vec<_>>(),
        [100.0_f64.to_bits(); 2],
        "the others at their default"
    );
    // Over the gain's range, 0 to 1, a draw is its number's 53 high bits over 2^53, exactly.
    let mut state = 0_u64;
    let drawn: Vec<u64> = (0_u8..2)
        .map(|_| (whole(splitmix64(&mut state) >> 11_u32) / TWO_TO_53).to_bits())
        .collect();
    assert_eq!(
        alone.iter().map(|[_, gain]| *gain).collect::<Vec<_>>(),
        drawn,
        "one parameter drawn at each frame, from the first number of the run's draws"
    );
    let held = Run::new::<1>(
        spec,
        (Motion::Held(3), audio_battery::param_sets(spec)[3].clone()),
        [super::Body::Driven; BLOCKS],
    );
    assert_eq!(
        moved(&held, 7, 62..64),
        [[top, one]; 2],
        "a set held to the end"
    );
}

/// The steps reach every frame of a block as the one where the values change, each once.
#[test]
fn steps_at_each_frame_of_a_block_once() {
    use super::{Motion, Run, STEPS};
    let spec = spec::<Probe<RIGHT>, 2>(&[OUT]);
    assert_eq!(STEPS, 8, "8 runs of 8 blocks");
    let mut frames = Vec::new();
    for first in 0..8 {
        let motion = (Motion::Step(first), ParamValues::defaults(spec));
        let run = Run::new::<1>(spec, motion, [super::Body::Driven; BLOCKS]);
        for block in 0..8 {
            let values = moved(&run, block, 0..64);
            let changes: Vec<usize> = (1..64)
                .filter(|&frame| values[frame] != values[frame - 1])
                .collect();
            // None inside the block: at its first frame.
            frames.push(match changes.as_slice() {
                [] => 0,
                [at] => *at,
                more => panic!("several changes in one block: {more:?}"),
            });
        }
    }
    assert_eq!(frames, (0..64).collect::<Vec<_>>(), "each frame once");
}

/// A block given no values follows a steady block, as a plan may render it: never the first
/// block, which [`super::check_created`] holds, nor one after a driven block, which a set
/// follows (`audio_core::Driven::set`).
#[test]
fn gives_no_values_only_after_a_steady_block() {
    use super::{Body, STEADY};
    let mut wrong = Vec::new();
    for (pattern, bodies) in STEADY.iter().enumerate() {
        let before = core::iter::once(None).chain(bodies.iter().map(Some));
        for (block, (body, last)) in bodies.iter().zip(before).enumerate() {
            if *body == Body::Kept && !matches!(last, Some(Body::Set | Body::Kept)) {
                wrong.push((pattern, block));
            }
        }
    }
    assert_eq!(wrong, [], "a block given no values after no steady block");
    assert_eq!(
        STEADY
            .iter()
            .flatten()
            .filter(|&&body| body == Body::Kept)
            .count(),
        3,
        "the control: blocks given no values"
    );
}

#[test]
fn does_not_see_inside_the_blocks_of_a_coarser_reference() {
    // The limit of a box type known only by its spec, as the laws document it: a body wrong
    // inside 16 frames passes a reference of 16, and fails a reference of 1 (above).
    assert_eq!(coarse_laws::<EVERY_16>(), Ok(()), "every 16 frames");
    assert_eq!(factory_laws::<EVERY_16>(), Ok(()), "through its factory");
}

#[test]
fn refuses_what_is_not_finite_and_the_heap() {
    assert_eq!(
        laws::<INFINITE>(),
        refused(
            "parameter set 0 held",
            "a sample is not finite, or is subnormal, inf (0x7ff0000000000000) at block 0, \
             outlet 0, frame 0, with parameters [0.0, 0.0]"
        ),
        "an infinity in both bodies"
    );
    // An allocation and a release in each of the 8 blocks.
    assert_eq!(
        laws::<ALLOCATES>(),
        refused("parameter set 0 held", "uses the heap (16 heap operations)"),
        "an allocation"
    );
    assert_eq!(laws::<ALLOCATES_OFFLINE>(), Ok(()), "offline-only");
}

#[test]
fn refuses_an_instance_that_is_not_created_or_takes_no_parameter() {
    for (result, problem) in [
        (
            laws::<REFUSES>(),
            "refuses as many parameters as it has, [0.0, 0.0] at block 0, frame 0",
        ),
        (
            laws::<REFUSES_BLOCKS>(),
            "refuses as many parameters as it has, [0.0, 0.0] at block 1, frame 0",
        ),
        (
            laws::<REFUSES_LATER>(),
            "refuses as many parameters as it has, [0.0, 0.0] at block 0, frame 5",
        ),
        // Four parts a block: the sixth is the second of the second block.
        (
            coarse_laws::<REFUSES_LATER>(),
            "refuses as many parameters as it has, [0.0, 0.0] at block 1, frame 16",
        ),
        (
            laws::<UNCREATED>(),
            "the box type is not built for engine blocks of 1 frames",
        ),
    ] {
        assert_eq!(result, refused("parameter set 0 held", problem), "refused");
    }
    assert_eq!(
        check_driven_laws::<Held<UNCREATED>, Held<RIGHT>, Held<UNCREATED>, Held<RIGHT>, 1>(spec::<
            Probe<RIGHT>,
            2,
        >(
            &[OUT]
        )),
        refused(
            "parameter set 0 held",
            "the box type is not built for engine blocks of 64 frames"
        ),
        "created for the reference only"
    );
}

#[test]
fn checks_nothing_without_parameters_and_refuses_parameters_without_outlet() {
    /// A box type without parameters.
    #[derive(Debug)]
    struct Plain;
    impl<S: Sample, const N: usize> Object<S, N> for Plain {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            for output in outputs {
                output.fill(S::ZERO);
            }
        }
    }
    impl Create<0> for Plain {
        const BOX_MODE: BoxMode = BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec; 0] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _: Values<0>,
        ) -> impl Driven<S, N, 0> + 'static {
            Self
        }
    }
    // No instance is created: an instance type that creates none is not seen.
    assert_eq!(
        check_driven_laws::<Held<UNCREATED>, Held<UNCREATED>, Held<UNCREATED>, Held<UNCREATED>, 1>(
            spec::<Plain, 0>(&[OUT])
        ),
        Ok(()),
        "one body"
    );
    assert_eq!(
        check_driven_laws::<Held<RIGHT>, Held<RIGHT>, Held<RIGHT>, Held<RIGHT>, 1>(spec::<
            Probe<RIGHT>,
            2,
        >(&[])),
        Err("has parameters and no outlet: nothing to compare".to_owned()),
        "nothing to compare"
    );
}

/// 2^53: over a range of that width from a whole number, a draw is its number's 53 high bits.
const TWO_TO_53: f64 = 9_007_199_254_740_992.0_f64;

/// The whole number `value`, below 2^53, as a binary64: exactly, in two parts each a `u32`.
fn whole(value: u64) -> f64 {
    let high = u32::try_from(value >> 21_u32).expect("below 2^32");
    let low = u32::try_from(value & 0x1f_ffff).expect("below 2^21");
    f64::from(high) * 2_097_152.0_f64 + f64::from(low)
}

/// A draw takes the 53 high bits of its number, in steps of the range's width over 2^53, from
/// either end: a range from 0 to 2^53 gives those bits, one from -2^53 to 0 those bits less
/// 2^53, both exactly.
#[test]
fn draws_the_53_high_bits_of_each_number_over_the_range() {
    for seed in [0_u64, 1, 0x0123_4567_89ab_cdef, u64::MAX] {
        let mut source = seed;
        let mut state = seed;
        for draw in 0_u8..64 {
            let high = splitmix64(&mut source) >> 11_u32;
            let mut below = state;
            assert_eq!(
                drawn(0.0_f64, TWO_TO_53, &mut state).to_bits(),
                whole(high).to_bits(),
                "seed {seed:#x}, draw {draw}, from 0"
            );
            assert_eq!(
                drawn(-TWO_TO_53, 0.0_f64, &mut below).to_bits(),
                (whole(high) - TWO_TO_53).to_bits(),
                "seed {seed:#x}, draw {draw}, to 0"
            );
            assert_eq!(below, state, "one number a draw");
        }
    }
}
