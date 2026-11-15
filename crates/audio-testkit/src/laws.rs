//! The laws every box must respect.

use alloc::collections::BTreeSet;

use audio_core::host::Raw;
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, Dispatch, Object, ObjectSpec, ParamSpec, ParamValues,
    Precision, SampleRate,
};
use proptest::array::uniform;
use proptest::collection::vec;
use proptest::test_runner::TestCaseError;

use audio_battery::{BLOCKS, BatterySample, param_sets, run_blocks, special_input};

use crate::heap_operations_during;
use crate::matrix::{PRECISIONS, sample_rates};

/// What the laws read of a sample, in either precision.
trait LawSample: BatterySample {
    /// Whether it is neither NaN nor infinite.
    fn finite(self) -> bool;

    /// Whether it is subnormal in its precision.
    fn subnormal(self) -> bool;
}

impl LawSample for Binary64 {
    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

impl LawSample for Binary32 {
    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

/// Blocks rendered by the long run of each sample rate and parameter set: 65,536 frames,
/// 1.4 s at 48 kHz and 0.17 s at 384 kHz. Enough to reveal state that overflows, becomes
/// non-finite or uses the heap late; the run is compared with no reference, so slow drift
/// is not detected here (a drift check against the exact phase is #12 step 20).
const LONG_RUN_BLOCKS: usize = 1024;

/// Random input cases tried per law.
const CASES: u32 = 32;

/// Checks that a list of names is free of empty and duplicate names.
///
/// Takes a trait object, not a generic iterator: one compiled function, so that the
/// coverage of each line is measured over every caller together.
fn unique_names(what: &str, names: &mut dyn Iterator<Item = &str>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for name in names {
        if name.trim().is_empty() {
            return Err(format!("{what} with an empty name"));
        }
        if !seen.insert(name) {
            return Err(format!("duplicate {what} \"{name}\""));
        }
    }
    Ok(())
}

/// Checks the static description of a box type.
///
/// # Errors
///
/// Returns the first problem found: missing name or description, empty or duplicate
/// port or parameter names. A parameter's range and default need no law: a
/// `audio_core::ParamSpec` is built only with a finite range holding its default.
pub fn check_metadata(spec: &ObjectSpec) -> Result<(), String> {
    if spec.name().trim().is_empty() || spec.name().contains(char::is_whitespace) {
        return Err(format!("invalid box name \"{}\"", spec.name()));
    }
    if spec.description().trim().is_empty() {
        return Err("missing description".to_owned());
    }
    unique_names("inlet", &mut spec.inlets().iter().map(|port| port.name))?;
    unique_names("outlet", &mut spec.outlets().iter().map(|port| port.name))?;
    unique_names("parameter", &mut spec.params().iter().map(ParamSpec::name))?;
    for port in spec.inlets().iter().chain(spec.outlets()) {
        if port.description.trim().is_empty() {
            return Err(format!("port \"{}\" has no description", port.name));
        }
    }
    for param in spec.params() {
        if param.description().trim().is_empty() {
            return Err(format!("parameter \"{}\" has no description", param.name()));
        }
    }
    Ok(())
}

/// Checks every box of a registry and that their names are unique.
///
/// # Errors
///
/// Returns the first problem found, prefixed by the box name.
pub fn check_registry(registry: &[&ObjectSpec]) -> Result<(), String> {
    unique_names("box", &mut registry.iter().map(|spec| spec.name()))?;
    for spec in registry {
        check_metadata(spec).map_err(|problem| format!("{}: {problem}", spec.name()))?;
    }
    Ok(())
}

/// Renders `blocks` blocks of a fresh instance computing in `S`, feeding consecutive input
/// blocks taken from `input` (then silence), every output sample set to `entry` before every
/// block ([`run_blocks`]). Returns the outputs and the heap operations made while
/// processing.
///
/// The parameters are given as a plan gives them, as `audio_core::Params`.
///
/// # Errors
///
/// Returns why the heap operations cannot be counted.
fn render<S: LawSample>(
    params: &ParamValues,
    rate: SampleRate,
    input: &[Block<S>],
    blocks: usize,
    entry: S,
) -> Result<(Vec<Block<S>>, usize), String> {
    let mut object = <Box<dyn Object<S>> as Dispatch<S>>::create(rate, params)
        .ok_or("the box type is not built for the default engine block")?;
    let mut operations = 0_usize;
    let rendered = run_blocks(params.spec(), input, blocks, entry, |given, outputs| {
        let ((), count) = heap_operations_during(|| object.process(given, outputs))?;
        operations = operations.saturating_add(count);
        Ok::<(), String>(())
    });
    Ok((rendered?, operations))
}

/// Bit patterns of rendered samples, so that `-0.0` and NaN payloads are compared exactly.
fn bits<S: LawSample>(blocks: &[Block<S>]) -> Vec<u64> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.bits())
        .collect()
}

/// The parameter values `params`, as the laws name them: each a binary32 value widened
/// exactly ([`ParamValues::within`]), so said in binary32.
fn shown(params: &ParamValues) -> Vec<f32> {
    params
        .params()
        .iter()
        .map(|&value| <Binary32 as audio_core::Sample>::from_binary64(value).raw())
        .collect()
}

/// Checks that rendered blocks are finite and hold no subnormal, and that rendering made
/// no heap operation (no allocation, reallocation or release). A sample that is not finite
/// is said as `non_finite`, which names every cause the render allows.
fn check_rendered<S: LawSample>(
    rendered: &[Block<S>],
    operations: usize,
    params: &ParamValues,
    non_finite: &str,
) -> Result<(), String> {
    let params = shown(params);
    if rendered.iter().flatten().any(|sample| !sample.finite()) {
        return Err(format!("{non_finite} with parameters {params:?}"));
    }
    if rendered.iter().flatten().any(|sample| sample.subnormal()) {
        return Err(format!("subnormal output with parameters {params:?}"));
    }
    if operations > 0 {
        return Err(format!(
            "uses the heap while processing ({operations} heap operations)"
        ));
    }
    Ok(())
}

/// What the second render of [`check_input`] and the long run ([`check_long_run`]) set
/// every output sample to before every block, where the first render sets +0: a quiet NaN
/// with a payload of its own, as close to no value a box computes as a sample can be.
const ENTRY_BITS: u64 = 0x7ff8_0000_dead_beef;

/// [`ENTRY_BITS`] in the precision `S`.
fn entry<S: LawSample>() -> S {
    S::from_binary64(Binary64::from_raw(f64::from_bits(ENTRY_BITS)))
}

/// Checks one input against the determinism, outputs-on-entry, finite-output,
/// no-subnormal and no-heap laws.
///
/// The determinism law renders each case twice, from two fresh instances: the first with
/// its outputs +0 on entry to every block, the second with them [`ENTRY_BITS`], which the
/// box contract leaves unspecified (a box writes every sample of every outlet,
/// `audio_core::Object::process`), so the two must give the same bits. Only when they do
/// not, a third render with +0, from a third instance, tells a box whose renders differ
/// with their entry alone from one that is not deterministic: when the third matches the
/// first, the box reads its outputs, leaves some of their samples unwritten, or differs
/// between instances in a way three renders cannot tell from those (every other instance
/// alike), and the law names the three. A box reading its outputs through a function
/// that maps both values alike (the larger of the output and its input, which NaN and +0
/// can both lose to) escapes the law: review only.
fn check_input<S: LawSample>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
) -> Result<(), String> {
    for params in param_sets(spec) {
        let (first, operations) = render(&params, rate, input, BLOCKS, S::ZERO)?;
        let (second, _) = render(&params, rate, input, BLOCKS, entry::<S>())?;
        if bits(&first) != bits(&second) {
            let (third, _) = render(&params, rate, input, BLOCKS, S::ZERO)?;
            let cause = if bits(&first) == bits(&third) {
                "reads its outputs, leaves some of their samples unwritten, or differs between \
                 instances,"
            } else {
                "not deterministic"
            };
            return Err(format!("{cause} with parameters {:?}", shown(&params)));
        }
        check_rendered(&first, operations, &params, "non-finite output")?;
    }
    Ok(())
}

/// Checks a long run of silence-fed blocks, per parameter set: finite, without subnormal
/// and without heap operations.
///
/// Every output sample is [`ENTRY_BITS`] on entry to every block, so that a box that stops
/// writing its outputs once fed silence (one skipping its work in silence, a tail ended),
/// which no block of the inputs [`check_input`] renders shows, leaves a NaN the finite law
/// sees: a box writing every sample of every outlet at every block gives the same bits
/// whatever its outputs hold on entry (`audio_core::Object::process`). A NaN found is said
/// with both causes it can have.
fn check_long_run<S: LawSample>(spec: &'static ObjectSpec, rate: SampleRate) -> Result<(), String> {
    for params in param_sets(spec) {
        let (rendered, operations) =
            render::<S>(&params, rate, &[], LONG_RUN_BLOCKS, entry::<S>())?;
        check_rendered(
            &rendered,
            operations,
            &params,
            "non-finite output, or outputs read or left unwritten (a NaN on entry),",
        )
        .map_err(|problem| format!("long run: {problem}"))?;
    }
    Ok(())
}

/// Checks every law at one sample rate, computing in `S`.
fn check_rate<S: LawSample>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    blocks: usize,
) -> Result<(), String> {
    check_input(spec, rate, &special_input::<S>(blocks))
        .map_err(|problem| format!("special inputs: {problem}"))?;
    check_long_run::<S>(spec, rate)?;
    crate::runner::runner(CASES)
        .run(
            &vec(uniform::<_, BLOCK_SIZE>(-1.0_f64..=1.0_f64), blocks),
            |input| {
                let samples: Vec<Block<S>> = input
                    .iter()
                    .map(|block| block.map(|value| S::from_binary64(Binary64::from_raw(value))))
                    .collect();
                check_input(spec, rate, &samples).map_err(TestCaseError::fail)
            },
        )
        .map_err(|error| error.to_string())
}

/// Checks every law at every sample rate of the [`crate::matrix`], computing in `S`.
fn check_precision<S: LawSample>(spec: &'static ObjectSpec, blocks: usize) -> Result<(), String> {
    for rate in sample_rates() {
        check_rate::<S>(spec, rate, blocks)
            .map_err(|problem| format!("at {} Hz: {problem}", rate.hz()))?;
    }
    Ok(())
}

/// Checks every law on a box type in both precisions and at every sample rate.
///
/// The precisions and rates of the [`crate::matrix`]: special inputs, a long run, and random
/// inputs from a fixed seed (in binary64, rounded to binary32 for the binary32 instance).
///
/// # Errors
///
/// Returns a description of the first broken law, prefixed by the precision and the sample
/// rate (`binary32, at 8000 Hz: …`). Also fails when [`crate::CountingAllocator`] is not
/// installed, since the no-heap law could not be checked.
pub fn check_laws(spec: &'static ObjectSpec) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    check_metadata(spec)?;
    let blocks = BLOCKS.saturating_mul(spec.inlets().len());
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => check_precision::<Binary64>(spec, blocks),
            Precision::Binary32 => check_precision::<Binary32>(spec, blocks),
        }
        .map_err(|problem| format!("binary{}, {problem}", precision.bits()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
