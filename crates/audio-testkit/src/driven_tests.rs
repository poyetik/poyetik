//! Tests of the laws of a driven box: each must refuse a box whose bodies differ.

use core::hint::black_box;

use audio_battery::BLOCKS;
use audio_core::DrivenParams;
use audio_core::host::Raw as _;
use audio_core::{
    AlgorithmVersion, Binary32, Binary64, Block, BoxInstance, BoxMode, Create, Dispatch, Driven,
    Object, ObjectSpec, ParamSpec, ParamValues, Params, PortSpec, Sample, SampleRate, Typed,
};

use super::check_driven_laws;

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

/// Outputs a ramp of `hertz` cycles per second, with a quarter of its input, times a gain;
/// `CHEAT` says how it goes wrong.
#[derive(Debug)]
struct Probe<const CHEAT: u8> {
    /// Its phase, in cycles.
    phase: f64,
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
            step: hertz.raw() / rate,
            gain: gain.raw(),
            rate,
        }
    }

    /// The next sample, of `input`: the arithmetic of both bodies.
    fn frame<S: Sample>(&mut self, input: S) -> S {
        let wide = input.to_binary64().raw();
        let sample = (self.phase + wide * 0.25_f64) * self.gain;
        self.phase = (self.phase + self.step).fract();
        let sample = if CHEAT == INFINITE {
            f64::INFINITY
        } else {
            sample
        };
        S::from_binary64(Binary64::new(sample)).flush()
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
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (frame, (out, value)) in output.iter_mut().zip(input).enumerate() {
                let at = match CHEAT {
                    FIRST_FRAME => 0,
                    LATE => frame.saturating_sub(1),
                    EVERY_16 => frame.saturating_sub(frame % 16),
                    _ => frame,
                };
                let (mut tone, mut gain) = (hertz[at].raw(), gains[at].raw());
                if CHEAT == SWAPPED {
                    (tone, gain) = (gain, tone);
                }
                let late = CHEAT == LATE && frame == 0;
                if !late {
                    self.step = if CHEAT == OTHER_FORMULA {
                        tone * (1.0_f64 / self.rate)
                    } else {
                        tone / self.rate
                    };
                    if CHEAT != ONE_OF_TWO {
                        self.gain = gain;
                    }
                }
                let before = self.phase;
                let sample = self.frame(*value);
                if CHEAT == FROZEN {
                    self.phase = before;
                }
                if CHEAT != UNWRITTEN || frame > 0 {
                    *out = sample;
                }
            }
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
    fn receive(&mut self, params: Params<'_>) -> bool {
        CHEAT != REFUSES && BoxInstance::<S, N>::receive(&mut self.0, params)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        params: DrivenParams<'_, N>,
    ) -> bool {
        CHEAT != REFUSES_BLOCKS && self.0.run_driven(inputs, outputs, params)
    }
}

impl<S: Sample, const N: usize, const CHEAT: u8> Dispatch<S, N> for Held<CHEAT> {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        if CHEAT == UNCREATED {
            return None;
        }
        let params = values.params().array()?;
        Some(Self(Typed::new(Probe::of(rate, params))))
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

/// What the laws say of a box whose first broken one gives `problem`: in the first precision,
/// at the first rate of the matrix, on the first input, at the first run of `motion`.
fn refused(motion: &str, problem: &str) -> Result<(), String> {
    Err(format!(
        "binary64, at 8000 Hz: special inputs, {motion}, steady blocks {:?}: {problem}",
        [false; BLOCKS]
    ))
}

#[test]
fn refuses_a_box_whose_bodies_differ() {
    const DIFFERS: &str =
        "the driven body gives other bits than the steady one, or leaves outputs unwritten";
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
        // The two formulas agree at 8 kHz at both ends of the range, and on every value of 32
        // random bits: seen there by the bits below.
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
        refused("Held(0)", "uses the heap while driven (16 heap operations)"),
        "an allocation"
    );
    assert_eq!(laws::<ALLOCATES_OFFLINE>(), Ok(()), "offline-only");
}

#[test]
fn refuses_an_instance_that_is_not_created_or_takes_no_parameter() {
    for (result, problem) in [
        (laws::<REFUSES>(), super::REFUSED),
        (laws::<REFUSES_BLOCKS>(), super::REFUSED),
        (laws::<UNCREATED>(), super::UNBUILT),
    ] {
        assert_eq!(result, refused("Held(0)", problem), "refused");
    }
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
