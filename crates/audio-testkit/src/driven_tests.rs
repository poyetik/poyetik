//! Tests of the laws of a driven box: each must refuse a box whose bodies differ.

use core::hint::black_box;

use audio_battery::{BLOCKS, splitmix64};
use audio_core::DrivenValues;
use audio_core::host::Raw as _;
use audio_core::{
    AlgorithmVersion, Binary32, Binary64, Block, BoxInstance, BoxMode, Create, Dispatch, Driven,
    Object, ObjectSpec, ParamSpec, ParamValues, PortSpec, Sample, SampleRate, Typed,
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
/// The instance refuses its blocks of values.
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

/// Outputs a ramp of `hertz` cycles per second, with a quarter of its input, times a gain;
/// `CHEAT` says how it goes wrong.
#[derive(Debug)]
struct Probe<const CHEAT: u8> {
    /// Its phase, in cycles.
    phase: f64,
    /// The phase of the driven body of [`SEPARATE_PHASE`], in cycles.
    separate: f64,
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
    fn of(rate: SampleRate, [hertz, gain]: [Binary64; 2]) -> Self {
        let rate = f64::from(rate.hz());
        Self {
            phase: 0.0,
            separate: 0.0,
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
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                *out = self.frame(*value);
            }
        }
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Driven<S, N, 2> for Probe<CHEAT> {
    fn set(&mut self, [hertz, gain]: [Binary64; 2]) {
        if CHEAT == ALLOCATES_IN_SET {
            drop(black_box(Vec::<u8>::with_capacity(8)));
        }
        self.first = hertz.raw().to_bits();
        if CHEAT != KEEPS_STEP {
            self.step = hertz.raw() / self.rate;
        }
        self.gain = gain.raw();
        if CHEAT == RESTARTS {
            self.phase = 0.0_f64;
        }
    }

    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        [hertz, gains]: &[Block<Binary64, N>; 2],
    ) {
        if CHEAT == IGNORES {
            self.process(inputs, outputs);
            return;
        }
        if CHEAT == ALLOCATES || CHEAT == ALLOCATES_OFFLINE {
            drop(black_box(Vec::<u8>::with_capacity(8)));
        }
        if CHEAT == SEPARATE_PHASE {
            core::mem::swap(&mut self.phase, &mut self.separate);
        }
        let same =
            |block: &Block<Binary64, N>| block[0].raw().to_bits() == block[1].raw().to_bits();
        let constant = CHEAT == FIRST_TWO && same(hertz) && same(gains);
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (frame, (out, value)) in output.iter_mut().zip(input).enumerate() {
                let at = match CHEAT {
                    FIRST_FRAME => 0,
                    LATE => frame.saturating_sub(1),
                    EVERY_16 => frame.saturating_sub(frame % 16),
                    FIRST_TWO if constant => 0,
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
        if CHEAT == SEPARATE_PHASE {
            core::mem::swap(&mut self.phase, &mut self.separate);
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
        params: [Binary64; 2],
    ) -> impl Driven<S, N, 2> + 'static {
        Self::of(rate, params)
    }
}

/// A [`Probe`] created at any engine block, as `audio_boxes::BuiltIn` creates a built-in box.
#[derive(Debug)]
struct Held<const CHEAT: u8>(Typed<Probe<CHEAT>, 2>);

impl<S: Sample, const N: usize, const CHEAT: u8> Object<S, N> for Held<CHEAT> {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.0.process(inputs, outputs);
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> BoxInstance<S, N> for Held<CHEAT> {
    fn receive(&mut self, values: &ParamValues) -> bool {
        CHEAT != REFUSES && BoxInstance::<S, N>::receive(&mut self.0, values)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        values: &DrivenValues<N>,
    ) -> bool {
        CHEAT != REFUSES_BLOCKS && self.0.run_driven(inputs, outputs, values)
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Dispatch<S, N> for Held<CHEAT> {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        if CHEAT == UNCREATED {
            return None;
        }
        let params = values.params().array()?;
        Some(Self(Typed::new(values.spec(), Probe::of(rate, params))))
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

/// What the laws say of a box whose first broken one gives `problem`: in `precision` bits, at
/// `hertz`, on `input`, at the run of `motion` whose steady blocks are `steady`.
fn refused_at(
    precision: u32,
    hertz: u32,
    input: &str,
    motion: &str,
    steady: [bool; BLOCKS],
    problem: &str,
) -> Result<(), String> {
    Err(format!(
        "binary{precision}, at {hertz} Hz: {input}, {motion}, steady blocks {steady:?}: {problem}"
    ))
}

/// What the laws say of a box whose first broken one gives `problem`: in the first precision,
/// at the first rate of the matrix, on the first input, at the first run of `motion`.
fn refused(motion: &str, problem: &str) -> Result<(), String> {
    refused_at(
        64,
        8_000,
        "special inputs",
        motion,
        [false; BLOCKS],
        problem,
    )
}

/// The run as the engine renders it differs from the reference.
const DIFFERS: &str = "the run as the engine renders it gives other bits than the reference, \
                       or leaves outputs unwritten";

/// Every other block steady, from the first.
const ALTERNATE: [bool; BLOCKS] = [true, false, true, false, true, false, true, false];

#[test]
fn refuses_a_box_whose_bodies_differ() {
    for (name, motion, result) in [
        // Right while its values stay, so first seen when they change at each frame.
        ("the first frame only", "Ends", laws::<FIRST_FRAME>()),
        ("one frame late", "Ends", laws::<LATE>()),
        ("a step kept", "Ends", laws::<KEEPS_STEP>()),
        ("one parameter ignored", "Ends", laws::<ONE_OF_TWO>()),
        ("every parameter ignored", "Ends", laws::<IGNORES>()),
        ("a value every 16 frames", "Ends", laws::<EVERY_16>()),
        (
            "the first frame only, through its factory",
            "Ends",
            factory_laws::<FIRST_FRAME>(),
        ),
        // The two formulas agree at 8 kHz at both ends of the range: seen there by the values
        // drawn.
        ("another formula", "Random", laws::<OTHER_FORMULA>()),
        // Wrong while its values stay, but silent at a gain of 0, the first set's: seen at the
        // second one, the defaults.
        ("a phase restarted", "Held(1)", laws::<RESTARTS>()),
        ("a phase frozen", "Held(1)", laws::<FROZEN>()),
        ("parameters swapped", "Held(1)", laws::<SWAPPED>()),
        // Whatever its values.
        ("a sample unwritten", "Held(0)", laws::<UNWRITTEN>()),
    ] {
        assert_eq!(result, refused(motion, DIFFERS), "{name}");
    }
}

#[test]
fn refuses_a_box_wrong_in_one_dimension_of_the_runs_only() {
    const HEAP: &str = "uses the heap (8 heap operations)";
    const SUBNORMAL_SAMPLE: &str = "a sample is not finite, or is subnormal";
    const CREATED: &str = "an instance created with its values gives other bits than one \
                           created with the defaults, then given them";
    let special = "special inputs";
    let random = "random inputs";
    let row = [false; BLOCKS];
    for (name, result, expected) in [
        // Right whenever the driven body follows itself: seen once the steady one renders.
        (
            "a phase of its own",
            laws::<SEPARATE_PHASE>(),
            refused_at(64, 8_000, special, "Held(1)", ALTERNATE, DIFFERS),
        ),
        (
            "the heap while taking values",
            laws::<ALLOCATES_IN_SET>(),
            refused_at(64, 8_000, special, "Held(0)", ALTERNATE, HEAP),
        ),
        // Right while no value changes to an equal one.
        (
            "-0 taken for +0",
            laws::<EQUAL_SKIP>(),
            refused_at(64, 8_000, special, "Zeros", row, DIFFERS),
        ),
        // Right while every parameter changes with the first.
        (
            "a parameter taken with the first only",
            laws::<CHANGE_ON_FIRST>(),
            refused_at(64, 8_000, special, "Alone(1)", row, DIFFERS),
        ),
        // Right while the values do not change at the second frame of a block.
        (
            "a block of two equal frames taken as constant",
            laws::<FIRST_TWO>(),
            refused_at(64, 8_000, special, "Step(0)", row, DIFFERS),
        ),
        // Right at the first rate.
        (
            "a fixed rate",
            laws::<FIXED_RATE>(),
            refused_at(64, 11_025, special, "Held(1)", row, DIFFERS),
        ),
        // Right on every special input.
        (
            "its input multiplied by 3, then divided by 3",
            laws::<THIRDS>(),
            refused_at(64, 8_000, random, "Held(1)", row, DIFFERS),
        ),
        // Right in binary64, and on the parameter sets, which binary32 holds (500 and 0.75 away
        // from the defaults): seen by the values drawn.
        (
            "values rounded to binary32",
            laws::<ROUNDED_VALUES>(),
            refused_at(32, 8_000, special, "Random", row, DIFFERS),
        ),
        (
            "a subnormal output",
            laws::<SUBNORMAL>(),
            refused_at(64, 8_000, special, "Held(0)", row, SUBNORMAL_SAMPLE),
        ),
        // Right by every run, since the reference gives values before every block.
        (
            "created with the default gain",
            laws::<CREATED_OTHERWISE>(),
            Err(format!(
                "binary64, at 8000 Hz: {special}, Held(0): {CREATED}"
            )),
        ),
    ] {
        assert_eq!(result, expected, "{name}");
    }
}

/// The bits of the values of `run`'s frames `frames` of its block `block`, a frame a pair.
fn moved(run: &super::Run, block: usize, frames: core::ops::Range<usize>) -> Vec<[u64; 2]> {
    run.blocks[block].parts[frames]
        .iter()
        .map(|values| {
            let [hertz, gain] = values.params().array().expect("two parameters");
            [hertz.raw().to_bits(), gain.raw().to_bits()]
        })
        .collect()
}

#[test]
fn moves_each_parameter_as_its_motion_says() {
    use super::{Motion, Run};
    let spec = spec::<Probe<RIGHT>, 2>(&[OUT]);
    let defaults = ParamValues::defaults(spec);
    let run = |motion: Motion| Run::new::<1>(spec, (motion, defaults.clone()), [false; BLOCKS]);
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
    assert!(
        alone[0][1] != alone[1][1]
            && alone
                .iter()
                .all(|[_, gain]| f64::from_bits(*gain) < 1.0_f64),
        "one parameter drawn at each frame, within its range: {alone:?}"
    );
    let held = Run::new::<1>(
        spec,
        (Motion::Held(3), audio_battery::param_sets(spec)[3].clone()),
        [false; BLOCKS],
    );
    assert_eq!(
        moved(&held, 7, 62..64),
        [[top, one]; 2],
        "a set held to the end"
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
        refused("Held(0)", "a sample is not finite, or is subnormal"),
        "an infinity in both bodies"
    );
    // An allocation and a release in each of the 8 blocks.
    assert_eq!(
        laws::<ALLOCATES>(),
        refused("Held(0)", "uses the heap (16 heap operations)"),
        "an allocation"
    );
    assert_eq!(laws::<ALLOCATES_OFFLINE>(), Ok(()), "offline-only");
}

#[test]
fn refuses_an_instance_that_is_not_created_or_takes_no_parameter() {
    for (result, problem) in [
        (laws::<REFUSES>(), "refuses as many parameters as it has"),
        (
            laws::<REFUSES_BLOCKS>(),
            "refuses as many parameters as it has",
        ),
        (
            laws::<UNCREATED>(),
            "the box type is not built for 1-frame engine blocks",
        ),
    ] {
        assert_eq!(result, refused("Held(0)", problem), "refused");
    }
    assert_eq!(
        check_driven_laws::<Held<UNCREATED>, Held<RIGHT>, Held<UNCREATED>, Held<RIGHT>, 1>(spec::<
            Probe<RIGHT>,
            2,
        >(
            &[OUT]
        )),
        refused(
            "Held(0)",
            "the box type is not built for 64-frame engine blocks"
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
            []: [Binary64; 0],
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
