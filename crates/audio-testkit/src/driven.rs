//! The laws of a driven box: its two bodies give the same bits.
//!
//! A box with parameters has two bodies, `audio_core::Object::process` for the blocks where
//! every parameter keeps one value and `audio_core::Driven::process_driven` for those where
//! one changes: nothing but these laws holds them together.

use alloc::collections::BTreeSet;

use audio_battery::{BLOCKS, param_sets, special_input, splitmix64};
use audio_core::host::ParamBounds as _;
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, BoxMode, Dispatch, ObjectSpec, ParamSpec, ParamValues,
    Precision, Sample as _, SampleRate,
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

/// How the parameters move over a run, frame by frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Motion {
    /// Not at all: one of the battery's parameter sets, by its place.
    Held(usize),
    /// From one end of the range to the other at each change, a parameter apart from the next.
    Ends,
    /// To a value drawn at each change.
    Random,
    /// Once per block, at a frame of its own in each: those from eight times this one on.
    Step(usize),
    /// Between the two zeros at each change, where the range accepts them.
    Zeros,
    /// To a value drawn at each change, for one parameter, the others staying at their default.
    Alone(usize),
}

impl Motion {
    /// Every motion tried on a box type of `parameters` parameters.
    fn all(parameters: usize) -> BTreeSet<Self> {
        let held = (0..6).map(Self::Held);
        let steps = (0..8).map(Self::Step);
        let alone = (0..parameters).map(Self::Alone);
        [Self::Ends, Self::Random, Self::Zeros]
            .into_iter()
            .chain(held)
            .chain(steps)
            .chain(alone)
            .collect()
    }

    /// The value of the parameter `param`, the `index`-th, at the change `change` of those
    /// every `R` frames, with the next draws of `state`: its range's, its default when the
    /// motion would leave it.
    fn value<const R: usize>(
        self,
        param: &ParamSpec,
        index: usize,
        change: usize,
        state: &mut u64,
    ) -> Binary64 {
        let (minimum, maximum) = (param.minimum(), param.maximum());
        let default = param.default_value();
        let end = |upper: bool| if upper { maximum } else { minimum };
        let frame = change.wrapping_mul(R);
        let block = frame.wrapping_div(BLOCK_SIZE);
        let value = match self {
            Self::Ends => end(!change.wrapping_add(index).is_multiple_of(2)),
            Self::Random => drawn(minimum, maximum, state),
            Self::Step(first) => {
                let at = first.wrapping_mul(8).wrapping_add(block);
                let after = frame.wrapping_rem(BLOCK_SIZE) >= at;
                end(block.is_multiple_of(2) == after)
            }
            Self::Zeros if change.is_multiple_of(2) => 0.0_f64,
            Self::Zeros => -0.0_f64,
            Self::Alone(moving) if moving == index => drawn(minimum, maximum, state),
            Self::Held(_) | Self::Alone(_) => default,
        };
        Binary64::new(if minimum <= value && value <= maximum {
            value
        } else {
            default
        })
    }
}

/// A value of `minimum` to `maximum`, drawn from `state` in steps of the range's width over
/// 2^53: with fewer bits, two formulas that round differently agree on every value drawn at
/// some rates (the probe of another formula is first refused at 8 kHz by these draws alone).
fn drawn(minimum: f64, maximum: f64, state: &mut u64) -> f64 {
    let [first, second, third, fourth, rest @ ..] = splitmix64(state).to_be_bytes();
    let high = f64::from(u32::from_be_bytes([first, second, third, fourth]));
    let low = f64::from(u32::from_be_bytes(rest).wrapping_shr(11));
    let part = (high * 2_097_152.0_f64 + low) / 9_007_199_254_740_992.0_f64;
    let value = minimum * (1.0_f64 - part) + maximum * part;
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

/// The parameters of one run: those the instances are created with, then those of each frame.
struct Run {
    /// How they move.
    motion: Motion,
    /// The blocks rendered by the steady body.
    steady: [bool; BLOCKS],
    /// The values the instances are created with.
    start: ParamValues,
    /// The values of each frame.
    frames: Vec<ParamValues>,
}

impl Run {
    /// The run of the box type `spec` by `motion`, whose values change every `R` frames at
    /// most: over a block of the steady body, they stay at the last ones.
    ///
    /// # Errors
    ///
    /// Returns why the box type refuses its own values.
    fn new<const R: usize>(
        spec: &'static ObjectSpec,
        motion: Motion,
        steady: [bool; BLOCKS],
    ) -> Result<Self, String> {
        let start = match motion {
            Motion::Held(set) => param_sets(spec).into_iter().nth(set),
            Motion::Ends | Motion::Random | Motion::Step(_) | Motion::Zeros | Motion::Alone(_) => {
                None
            }
        }
        .unwrap_or_else(|| ParamValues::defaults(spec));
        let mut state = 0_u64;
        let mut last = start.clone();
        let mut frames = Vec::new();
        for frame in 0..BLOCKS.wrapping_mul(BLOCK_SIZE) {
            let held = steady
                .get(frame.wrapping_div(BLOCK_SIZE))
                .is_some_and(|&held| held);
            let moves = !matches!(motion, Motion::Held(_));
            if moves && !held && frame.is_multiple_of(R) {
                let change = frame.checked_div(R).unwrap_or_default();
                let values: Vec<Binary64> = spec
                    .params()
                    .iter()
                    .enumerate()
                    .map(|(index, param)| motion.value::<R>(param, index, change, &mut state))
                    .collect();
                last = ParamValues::of(spec, &values).map_err(|error| error.to_string())?;
            }
            frames.push(last.clone());
        }
        Ok(Self {
            motion,
            steady,
            start,
            frames,
        })
    }
}

/// What a box is fed: one block per inlet, for each block of a run.
type Fed<S> = Vec<Vec<Block<S>>>;

/// `input` as a run feeds it to `inlets` inlets: its successive blocks to the inlets in turn.
fn fed<S: LawSample>(input: &[Block<S>], inlets: usize) -> Fed<S> {
    let mut blocks = input.iter().copied();
    (0..BLOCKS)
        .map(|_| blocks.by_ref().take(inlets).collect())
        .collect()
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

/// The box type is refused: an instance type does not create it.
const UNBUILT: &str = "the box type is not created at this engine block";

/// The box type is refused: it does not take its own number of parameters.
const REFUSED: &str = "refuses as many parameters as it has";

/// The reference: the run by the steady body alone, in blocks of `R` frames, each given its
/// values before it.
fn reference<S: LawSample, F: Dispatch<S, R>, const R: usize>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &Fed<S>,
    run: &Run,
) -> Result<Vec<Block<S>>, String> {
    let mut object = F::create(rate, &run.start).ok_or(UNBUILT)?;
    let mut rendered = Vec::new();
    for (given, values) in input.iter().zip(run.frames.chunks(BLOCK_SIZE)) {
        let mut whole = vec![S::SILENCE; spec.outlets().len()];
        for (part, held) in values.chunks(R).enumerate() {
            let first = held.first().ok_or(REFUSED)?;
            if !object.receive(first.params()) {
                return Err(REFUSED.to_owned());
            }
            let inputs: Vec<[S; R]> = given
                .iter()
                .filter_map(|block| block.as_chunks::<R>().0.get(part).copied())
                .collect();
            let mut outputs = vec![[S::ZERO; R]; spec.outlets().len()];
            object.process(&inputs, &mut outputs);
            for (block, output) in whole.iter_mut().zip(&outputs) {
                for place in block.as_chunks_mut::<R>().0.iter_mut().skip(part).take(1) {
                    *place = *output;
                }
            }
        }
        rendered.extend(whole);
    }
    Ok(rendered)
}

/// The run as the engine renders it, in blocks of 64 frames: by the steady body the blocks
/// where its values stay, given them before it, by the driven body the others. The outputs
/// are set to a NaN before each block, which a sample left unwritten keeps. Also returns the
/// heap operations.
fn driven<S: LawSample, W: Dispatch<S>>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &Fed<S>,
    run: &Run,
) -> Result<(Vec<Block<S>>, usize), String> {
    let mut object = W::create(rate, &run.start).ok_or(UNBUILT)?;
    let mut rendered = Vec::new();
    let mut operations = 0_usize;
    let blocks = input.iter().zip(run.frames.chunks(BLOCK_SIZE));
    for ((given, values), steady) in blocks.zip(run.steady) {
        let first = values.first().ok_or(REFUSED)?;
        let columns: Vec<Block<Binary64>> = (0..spec.params().len())
            .map(|index| {
                let mut column = [Binary64::ZERO; BLOCK_SIZE];
                for (slot, frame) in column.iter_mut().zip(values) {
                    *slot = frame.params().get(index).copied().unwrap_or(*slot);
                }
                column
            })
            .collect();
        let mut outputs = vec![S::SILENCE; spec.outlets().len()];
        for output in &mut outputs {
            output.fill(entry());
        }
        let (taken, count) = heap_operations_during(|| {
            if steady {
                let taken = object.receive(first.params());
                object.process(given, &mut outputs);
                taken
            } else {
                object.run_driven(given, &mut outputs, &columns)
            }
        })?;
        if !taken {
            return Err(REFUSED.to_owned());
        }
        operations = operations.saturating_add(count);
        rendered.extend(outputs);
    }
    Ok((rendered, operations))
}

/// Checks one run on one input.
fn check_run<S: LawSample, W: Dispatch<S>, F: Dispatch<S, R>, const R: usize>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &Fed<S>,
    run: &Run,
) -> Result<(), String> {
    let expected = reference::<S, F, R>(spec, rate, input, run)?;
    let (rendered, operations) = driven::<S, W>(spec, rate, input, run)?;
    if bits(&rendered) != bits(&expected) {
        return Err(
            "the driven body gives other bits than the steady one, or leaves outputs unwritten"
                .to_owned(),
        );
    }
    let samples = rendered.iter().flatten();
    if samples
        .clone()
        .any(|sample| !sample.finite() || sample.subnormal())
    {
        return Err("a sample is not finite, or is subnormal".to_owned());
    }
    if operations > 0 && spec.mode() == BoxMode::LiveCapable {
        return Err(format!(
            "uses the heap while driven ({operations} heap operations)"
        ));
    }
    Ok(())
}

/// Checks every run at every sample rate of the [`crate::matrix`], computing in `S`.
fn check_precision<S: LawSample, W: Dispatch<S>, F: Dispatch<S, R>, const R: usize>(
    spec: &'static ObjectSpec,
    runs: &[Run],
) -> Result<(), String> {
    let inlets = spec.inlets().len();
    let blocks = BLOCKS.saturating_mul(inlets);
    let inputs = [
        ("special inputs", fed::<S>(&special_input(blocks), inlets)),
        ("random inputs", fed::<S>(&random_input(blocks), inlets)),
    ];
    for rate in sample_rates() {
        for (name, input) in &inputs {
            for run in runs {
                check_run::<S, W, F, R>(spec, rate, input, run).map_err(|problem| {
                    format!(
                        "at {} Hz: {name}, {:?}, steady blocks {:?}: {problem}",
                        rate.hz(),
                        run.motion,
                        run.steady
                    )
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
/// - **back and forth**: driven blocks, the last values set, then steady ones.
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
/// - **a box in a module**: not created here (roadmap #12 step 40);
/// - **a difference the values tried do not show**: 512 frames a run, at the rates of the
///   matrix. The values of parameters are binary64 in both precisions, drawn over their range
///   in steps of its width over 2^53: the closer to zero, the fewer of a value's own bits are
///   drawn. The random inputs are drawn alike, then rounded to the 24 bits of binary32 in a
///   binary32 render, where a difference below the last bit of the outputs shows only once
///   the box's state has grown it. Two formulas that agree on all of these pass: that a box
///   has one arithmetic for both bodies is for its review;
/// - **an error both bodies share**: conformance and each box's own tests see it.
///
/// # Errors
///
/// Returns a description of the first broken law, prefixed by the precision, the sample rate
/// and the run. Also fails when [`crate::CountingAllocator`] is not installed, since the
/// no-heap law could not be checked.
pub fn check_driven_laws<W64, F64, W32, F32, const R: usize>(
    spec: &'static ObjectSpec,
) -> Result<(), String>
where
    W64: Dispatch<Binary64>,
    F64: Dispatch<Binary64, R>,
    W32: Dispatch<Binary32>,
    F32: Dispatch<Binary32, R>,
{
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
    let mut runs = Vec::new();
    for motion in Motion::all(spec.params().len()) {
        for steady in STEADY {
            runs.push(Run::new::<R>(spec, motion, steady)?);
        }
    }
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => check_precision::<Binary64, W64, F64, R>(spec, &runs),
            Precision::Binary32 => check_precision::<Binary32, W32, F32, R>(spec, &runs),
        }
        .map_err(|problem| format!("binary{}, {problem}", precision.bits()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "driven_tests.rs"]
mod tests;
