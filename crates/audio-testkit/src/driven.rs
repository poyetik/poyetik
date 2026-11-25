//! The laws of a driven box: its two bodies give the same bits.
//!
//! A box with parameters has two bodies, `audio_core::Object::process` for the blocks where
//! every parameter keeps one value and `audio_core::Driven::process_driven` for those where
//! one changes: nothing but these laws holds them together.

use audio_battery::{BLOCKS, param_sets, run_blocks, special_input, splitmix64};
use audio_core::host::{ParamBounds as _, Raw};
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, BoxInstance, BoxMode, Dispatch, DrivenValues,
    ObjectSpec, ParamSpec, ParamValues, Precision, Sample, SampleRate,
};

use crate::heap_operations_during;
use crate::laws::{LawSample, bits, entry};
use crate::matrix::{PRECISIONS, sample_rates};

/// Which blocks of a run are rendered by the steady body, the others by the driven one: none,
/// every other from the first or from the second, and runs of both, so that each body follows
/// the other and itself.
const STEADY: [[bool; BLOCKS]; 4] = [
    [false; BLOCKS],
    [true, false, true, false, true, false, true, false],
    [false, true, false, true, false, true, false, true],
    [false, false, true, true, true, false, true, false],
];

/// How many runs change once per block ([`Motion::Step`]): each of the [`BLOCKS`] blocks of
/// a run changes at a frame of its own in each, so that over them every frame of a block is
/// one where the values change.
const STEPS: usize = BLOCK_SIZE.wrapping_div(BLOCKS);

const _: () = assert!(
    STEPS.wrapping_mul(BLOCKS) == BLOCK_SIZE,
    "the steps of the runs reach every frame of a block once"
);

/// How the parameters move over a run, frame by frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    /// Not at all: one of the battery's parameter sets, by its place.
    Held(usize),
    /// From one end of the range to the other at each change, a parameter apart from the next.
    Ends,
    /// To a value drawn at each change.
    Random,
    /// Once per block, at a frame of its own in each: those from [`BLOCKS`] times this one on.
    Step(usize),
    /// Between the two zeros at each change, where the range accepts them.
    Zeros,
    /// To a value drawn at each change, for one parameter, the others staying at their default.
    Alone(usize),
}

impl Motion {
    /// The motions that hold the values of the box type `spec`, one per parameter set of the
    /// battery, each with that set, which its instances are created with.
    fn held(spec: &'static ObjectSpec) -> Vec<(Self, ParamValues)> {
        param_sets(spec)
            .into_iter()
            .enumerate()
            .map(|(set, values)| (Self::Held(set), values))
            .collect()
    }

    /// The motions that move the values of the box type `spec`, each with the defaults, which
    /// its instances are created with.
    fn moving(spec: &'static ObjectSpec) -> Vec<(Self, ParamValues)> {
        [Self::Ends, Self::Random]
            .into_iter()
            .chain((0..STEPS).map(Self::Step))
            .chain([Self::Zeros])
            .chain((0..spec.params().len()).map(Self::Alone))
            .map(|motion| (motion, ParamValues::defaults(spec)))
            .collect()
    }

    /// The value of the parameter `param`, the `index`-th, at the change `change` of those
    /// every `R` frames, with the next draws of `state`: [`ParamValues::within`] takes its
    /// parameter's default for a value its range refuses.
    fn value<const R: usize>(
        self,
        param: &ParamSpec,
        index: usize,
        change: usize,
        state: &mut u64,
    ) -> f64 {
        let (minimum, maximum) = (param.minimum(), param.maximum());
        let end = |upper: bool| if upper { maximum } else { minimum };
        let frame = change.wrapping_mul(R);
        let block = frame.wrapping_div(BLOCK_SIZE);
        match self {
            Self::Ends => end(change.wrapping_add(index).is_multiple_of(2)),
            Self::Random => drawn(minimum, maximum, state),
            Self::Step(first) => {
                let at = first.wrapping_mul(BLOCKS).wrapping_add(block);
                let after = frame.wrapping_rem(BLOCK_SIZE) >= at;
                end(block.is_multiple_of(2) == after)
            }
            Self::Zeros if change.is_multiple_of(2) => 0.0_f64,
            Self::Zeros => -0.0_f64,
            Self::Alone(moving) if moving == index => drawn(minimum, maximum, state),
            Self::Held(_) | Self::Alone(_) => param.default_value(),
        }
    }
}

/// A value of `minimum` to `maximum`, drawn from `state` in steps of the range's width over
/// 2^53: with fewer bits, two formulas that round differently agree on every value drawn at
/// some rates (the probe of another formula is first refused at 8 kHz by these draws alone).
/// Rounded past an end, it is the parameter's default, as [`ParamValues::within`] takes it.
fn drawn(minimum: f64, maximum: f64, state: &mut u64) -> f64 {
    let [first, second, third, fourth, rest @ ..] = splitmix64(state).to_be_bytes();
    let high = f64::from(u32::from_be_bytes([first, second, third, fourth]));
    let low = f64::from(u32::from_be_bytes(rest).wrapping_shr(11));
    let part = (high * 2_097_152.0_f64 + low) / 9_007_199_254_740_992.0_f64;
    minimum * (1.0_f64 - part) + maximum * part
}

/// One block of a run, as each body is given it.
struct RunBlock {
    /// Whether the steady body renders it, the driven one otherwise.
    steady: bool,
    /// The values the steady body is given before it: those of its frames, in a steady block.
    set: ParamValues,
    /// The values the reference is given before each part of `R` frames, in order.
    parts: Vec<ParamValues>,
    /// The values of each frame, as the driven body takes them.
    driving: DrivenValues,
}

/// The parameters of one run: those the instances are created with, then those of each block.
struct Run {
    /// How they move.
    motion: Motion,
    /// The blocks rendered by the steady body.
    steady: [bool; BLOCKS],
    /// The values the instances are created with.
    start: ParamValues,
    /// Its blocks, in order.
    blocks: Vec<RunBlock>,
}

impl Run {
    /// The run of the box type `spec` by `motion` from `start`, whose values change every
    /// `R` frames at most: over a block of the steady body, they stay at the last ones.
    fn new<const R: usize>(
        spec: &'static ObjectSpec,
        (motion, start): (Motion, ParamValues),
        steady: [bool; BLOCKS],
    ) -> Self {
        let moves = !matches!(motion, Motion::Held(_));
        let mut state = 0_u64;
        let mut last = start.clone();
        let mut frame = 0_usize;
        let blocks = steady
            .iter()
            .map(|&held| {
                let set = last.clone();
                let mut frames = Vec::with_capacity(BLOCK_SIZE);
                for _ in 0..BLOCK_SIZE {
                    if moves && !held && frame.is_multiple_of(R) {
                        let change = frame.checked_div(R).unwrap_or_default();
                        let mut index = 0_usize;
                        last = ParamValues::within(spec, |param| {
                            let value = motion.value::<R>(param, index, change, &mut state);
                            index = index.wrapping_add(1);
                            value
                        });
                    }
                    frames.push(last.clone());
                    frame = frame.wrapping_add(1);
                }
                let driving = DrivenValues::within(spec, |index, param, at| {
                    let default = param.default_value();
                    frames
                        .get(at)
                        .and_then(|values| values.params().get(index).copied())
                        .map_or(default, Raw::raw)
                });
                RunBlock {
                    steady: held,
                    set,
                    parts: frames.iter().step_by(R).cloned().collect(),
                    driving,
                }
            })
            .collect();
        Self {
            motion,
            steady,
            start,
            blocks,
        }
    }
}

/// `blocks` blocks of values in `[-1, 1)`, drawn from a fixed seed.
fn random_input<S: LawSample>(blocks: usize) -> Vec<Block<S>> {
    let mut state = 1_u64;
    (0..blocks)
        .map(|_| {
            core::array::from_fn(|_| {
                S::from_binary64(Binary64::new(drawn(-1.0_f64, 1.0_f64, &mut state)))
            })
        })
        .collect()
}

/// What creates the instances of one instance type, in engine blocks of `N` frames: so that
/// the laws are compiled once per precision and block whatever the instance type, and each
/// of their paths is held by a box that breaks it.
type Creator<S, const N: usize> =
    fn(SampleRate, &ParamValues) -> Option<Box<dyn BoxInstance<S, N>>>;

/// An instance of the instance type `T` at `rate`, of `values`, as [`Dispatch::create`]
/// creates it, if it does.
fn creator<S: Sample, T: Dispatch<S, N> + 'static, const N: usize>(
    rate: SampleRate,
    values: &ParamValues,
) -> Option<Box<dyn BoxInstance<S, N>>> {
    T::create(rate, values).map(|instance| -> Box<dyn BoxInstance<S, N>> { Box::new(instance) })
}

/// An instance of `make`'s instance type, in engine blocks of `N` frames, at `rate`, of
/// `values`.
///
/// # Errors
///
/// Returns [`unbuilt`]'s refusal when `make` creates none.
fn create<S: Sample, const N: usize>(
    make: Creator<S, N>,
    rate: SampleRate,
    values: &ParamValues,
) -> Result<Box<dyn BoxInstance<S, N>>, String> {
    make(rate, values).ok_or_else(|| unbuilt(N))
}

/// The refusal of a box type an instance type does not create in engine blocks of `frames`
/// frames.
fn unbuilt(frames: usize) -> String {
    format!("the box type is not built for {frames}-frame engine blocks")
}

/// Whether an instance took what it was given: [`REFUSED`] unless it did.
///
/// # Errors
///
/// Returns [`REFUSED`] unless `taken`.
fn taken(taken: bool) -> Result<(), String> {
    if taken {
        Ok(())
    } else {
        Err(REFUSED.to_owned())
    }
}

/// The box type is refused: it does not take its own number of parameters.
const REFUSED: &str = "refuses as many parameters as it has";

/// The run as the engine renders it differs from the reference.
const DIFFERS: &str = "the run as the engine renders it gives other bits than the reference, or leaves outputs \
     unwritten";

/// An instance created with values differs from one created with the defaults, then given them.
const CREATED_OTHERWISE: &str = "an instance created with its values gives other bits than one \
     created with the defaults, then given them";

/// What creates the instances of a run in one precision: as the engine does, in blocks of
/// 64 frames, and for the reference, in blocks of `R`.
struct Makers<S, const R: usize> {
    /// In blocks of 64 frames.
    engine: Creator<S, BLOCK_SIZE>,
    /// In blocks of `R` frames.
    reference: Creator<S, R>,
}

/// The reference: the run by the steady body alone, in blocks of `R` frames, each given its
/// values before it.
///
/// # Errors
///
/// Returns [`unbuilt`]'s refusal when `make` creates no instance, and [`REFUSED`] when the
/// instance refuses its values.
fn reference<S: LawSample, const R: usize>(
    make: Creator<S, R>,
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
    run: &Run,
) -> Result<Vec<Block<S>>, String> {
    let mut object = create(make, rate, &run.start)?;
    let mut blocks = run.blocks.iter();
    run_blocks(spec, input, BLOCKS, entry(), |given, outputs| {
        blocks.next().into_iter().try_for_each(|block| {
            for (part, values) in block.parts.iter().enumerate() {
                taken(object.receive(values.params()))?;
                let inputs: Vec<[S; R]> = given
                    .iter()
                    .filter_map(|whole| whole.as_chunks::<R>().0.get(part).copied())
                    .collect();
                let mut rendered = vec![[S::ZERO; R]; outputs.len()];
                object.process(&inputs, &mut rendered);
                for (output, chunk) in outputs.iter_mut().zip(&rendered) {
                    for place in output.as_chunks_mut::<R>().0.iter_mut().skip(part).take(1) {
                        *place = *chunk;
                    }
                }
            }
            Ok(())
        })
    })
}

/// The run as the engine renders it, in blocks of 64 frames: by the steady body the blocks
/// where its values stay, given them before it, by the driven body the others. The outputs
/// are set to a NaN before each block, which a sample left unwritten keeps. Also returns the
/// heap operations.
///
/// # Errors
///
/// Returns [`unbuilt`]'s refusal when `make` creates no instance, [`REFUSED`] when the
/// instance refuses its values or its blocks of values, and why the heap operations cannot be
/// counted.
fn driven<S: LawSample>(
    make: Creator<S, BLOCK_SIZE>,
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
    run: &Run,
) -> Result<(Vec<Block<S>>, usize), String> {
    let mut object = create(make, rate, &run.start)?;
    let mut operations = 0_usize;
    let mut blocks = run.blocks.iter();
    let rendered = run_blocks(spec, input, BLOCKS, entry(), |given, outputs| {
        blocks.next().into_iter().try_for_each(|block| {
            let render = || {
                if block.steady {
                    let took = object.receive(block.set.params());
                    object.process(given, outputs);
                    took
                } else {
                    object.run_driven(given, outputs, block.driving.params())
                }
            };
            let (took, count) = heap_operations_during(render)?;
            operations = operations.saturating_add(count);
            taken(took)
        })
    })?;
    Ok((rendered, operations))
}

/// Checks one run on one input.
///
/// # Errors
///
/// Returns the errors of [`reference()`] and [`driven`], then [`DIFFERS`], then the first broken
/// law of the samples and of the heap.
fn check_run<S: LawSample, const R: usize>(
    makers: &Makers<S, R>,
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
    run: &Run,
) -> Result<(), String> {
    let expected = reference(makers.reference, spec, rate, input, run)?;
    let (rendered, operations) = driven(makers.engine, spec, rate, input, run)?;
    if bits(&rendered) != bits(&expected) {
        return Err(DIFFERS.to_owned());
    }
    let samples = rendered.iter().flatten();
    if samples
        .clone()
        .any(|sample| !sample.finite() || sample.subnormal())
    {
        return Err("a sample is not finite, or is subnormal".to_owned());
    }
    if operations > 0 && spec.mode() == BoxMode::LiveCapable {
        return Err(format!("uses the heap ({operations} heap operations)"));
    }
    Ok(())
}

/// Checks that an instance created with `values` renders as one created with the defaults,
/// then given `values`, by the steady body: the reference gives values before every block, so
/// it does not see what an instance is created with.
///
/// # Errors
///
/// Returns [`unbuilt`]'s refusal when `make` creates no instance, [`REFUSED`] when the
/// instance refuses `values`, then [`CREATED_OTHERWISE`].
fn check_created<S: LawSample>(
    make: Creator<S, BLOCK_SIZE>,
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
    values: &ParamValues,
) -> Result<(), String> {
    let mut created = create(make, rate, values)?;
    let mut given = create(make, rate, &ParamValues::defaults(spec))?;
    taken(given.receive(values.params()))?;
    let render = |object: &mut Box<dyn BoxInstance<S>>| {
        run_blocks(spec, input, BLOCKS, entry(), |inputs, outputs| {
            object.process(inputs, outputs);
            Ok::<(), String>(())
        })
    };
    if bits(&render(&mut created)?) == bits(&render(&mut given)?) {
        Ok(())
    } else {
        Err(CREATED_OTHERWISE.to_owned())
    }
}

/// Checks every run at every sample rate of the [`crate::matrix`], computing in `S`, then
/// that each set of values held renders the same whether an instance is created with it or
/// given it.
///
/// # Errors
///
/// Returns the first error of [`check_run`] or [`check_created`], prefixed by the rate, the
/// input and the run.
fn check_precision<S: LawSample, const R: usize>(
    makers: &Makers<S, R>,
    spec: &'static ObjectSpec,
    runs: &[Run],
    held: &[(Motion, ParamValues)],
) -> Result<(), String> {
    let blocks = BLOCKS.saturating_mul(spec.inlets().len());
    let inputs = [
        ("special inputs", special_input::<S>(blocks)),
        ("random inputs", random_input::<S>(blocks)),
    ];
    for rate in sample_rates() {
        for (name, input) in &inputs {
            for run in runs {
                check_run(makers, spec, rate, input, run).map_err(|problem| {
                    format!(
                        "at {} Hz: {name}, {:?}, steady blocks {:?}: {problem}",
                        rate.hz(),
                        run.motion,
                        run.steady
                    )
                })?;
            }
            for (motion, values) in held {
                check_created(makers.engine, spec, rate, input, values).map_err(|problem| {
                    format!("at {} Hz: {name}, {motion:?}: {problem}", rate.hz())
                })?;
            }
        }
    }
    Ok(())
}

/// Checks that the two bodies of a box type give the same bits, in both precisions and at
/// every sample rate.
///
/// The steady body alone (`audio_core::Object::process`), in blocks of `R` frames, each given
/// its values before it (`audio_core::Driven::set`), is the reference. The same values are
/// then rendered as the engine does, in blocks of 64 frames, by the driven body
/// (`audio_core::Driven::process_driven`) and by the steady one in the blocks where they
/// stay: the same bits, for
/// - **values that stay**, each block by either body, so that neither forgets a state the
///   other keeps;
/// - **values that change at any frame**: drawn at each frame, from one end of the range to
///   the other, once per block at each frame of a block, between the two zeros, each
///   parameter alone; with constant values alone, a body reading the first frame of each
///   block would pass;
/// - **back and forth**: driven blocks, the last values set, then steady ones;
/// - **values an instance is created with**: each parameter set held, by the steady body
///   alone, from an instance created with it and from one created with the defaults then
///   given it, since the reference, given values before every block, does not see them.
///
/// It holds because a render does not depend on the engine block (`spec/engine-design.md`,
/// decision 2): a box that breaks that fails too. The outputs are a NaN on entry, the samples
/// are finite and never subnormal, and a live-capable box uses no heap.
///
/// `W64` and `W32` are the instance types at 64 frames in each precision, `F64` and `F32`
/// those at `R` frames, which divides 64 (it does not compile otherwise). With `R` at 1,
/// every frame is checked: the engine's `audio_boxes::BuiltIn` creates a built-in box there,
/// and a box type known by its own type is created by `audio_core::Create::create`.
///
/// A box type without parameters has one body: nothing is checked. What these laws do not
/// see, each with the step that treats it:
/// - **a box type known only by its spec** (its factory): its smallest engine block is 16
///   frames, so with `R` at 16 the values change every 16 frames only, and a body wrong
///   inside them passes (any box type loaded at run time: roadmap #12 step 40);
/// - **a box with parameters and no outlet**: its bits are read by the plan, so there is
///   nothing to compare: it is refused, until what it answers with can be read (roadmap #12
///   step 15);
/// - **a box that answers with messages**: only its signals are compared (roadmap #12
///   step 15);
/// - **a box in a module**: not created here; a module receives its parameters' changes while
///   it processes with roadmap #12 step 41;
/// - **a box that locks or waits**: only the heap is counted;
/// - **a difference the values tried do not show**: [`BLOCKS`] blocks of
///   [`audio_core::BLOCK_SIZE`] frames a run, at the rates of the matrix. The values of
///   parameters are binary64 in both precisions, drawn over their range in steps of its width
///   over 2^53: the closer to zero, the fewer of a value's own bits are
///   drawn. The random inputs are drawn alike, then rounded to the 24 bits of binary32 in a
///   binary32 render, where a difference below the last bit of the outputs shows only once
///   the box's state has grown it. Two formulas that agree on all of these pass: that a box
///   has one arithmetic for both bodies is for its review;
/// - **an error both bodies share**: conformance and each box's own tests see it.
///
/// # Errors
///
/// Fails when [`crate::CountingAllocator`] is not installed, since the no-heap law could not
/// be checked, then when the box type has parameters and no outlet. Otherwise returns the
/// first broken law, prefixed by the precision, the sample rate, the input and the run: an
/// instance type that creates no instance, an instance that refuses its values or its blocks
/// of values, a run whose bits differ from the reference's or that leaves an output
/// unwritten, a sample that is not finite or is subnormal, a live-capable box that uses the
/// heap, and an instance created with its values that renders otherwise than one given them.
pub fn check_driven_laws<W64, F64, W32, F32, const R: usize>(
    spec: &'static ObjectSpec,
) -> Result<(), String>
where
    W64: Dispatch<Binary64> + 'static,
    F64: Dispatch<Binary64, R> + 'static,
    W32: Dispatch<Binary32> + 'static,
    F32: Dispatch<Binary32, R> + 'static,
{
    laws_of::<R>(
        spec,
        &Makers {
            engine: creator::<Binary64, W64, BLOCK_SIZE>,
            reference: creator::<Binary64, F64, R>,
        },
        &Makers {
            engine: creator::<Binary32, W32, BLOCK_SIZE>,
            reference: creator::<Binary32, F32, R>,
        },
    )
}

/// [`check_driven_laws`] of the instance types `binary64` and `binary32` create, in each
/// precision.
///
/// # Errors
///
/// As [`check_driven_laws`].
fn laws_of<const R: usize>(
    spec: &'static ObjectSpec,
    binary64: &Makers<Binary64, R>,
    binary32: &Makers<Binary32, R>,
) -> Result<(), String> {
    const {
        assert!(
            R != 0 && BLOCK_SIZE.is_multiple_of(R),
            "the reference's block divides the engine's default"
        );
    }
    let ((), _) = heap_operations_during(|| ())?;
    if spec.params().is_empty() {
        return Ok(());
    }
    if spec.outlets().is_empty() {
        return Err("has parameters and no outlet: nothing to compare".to_owned());
    }
    let held = Motion::held(spec);
    let mut runs = Vec::new();
    for motion in held.iter().cloned().chain(Motion::moving(spec)) {
        for steady in STEADY {
            runs.push(Run::new::<R>(spec, motion.clone(), steady));
        }
    }
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => check_precision(binary64, spec, &runs, &held),
            Precision::Binary32 => check_precision(binary32, spec, &runs, &held),
        }
        .map_err(|problem| format!("binary{}, {problem}", precision.bits()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "driven_tests.rs"]
mod tests;
