//! The laws every box must respect.

use alloc::collections::BTreeSet;

use engine_core::host::Raw;
use engine_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, Object, ObjectSpec, ParamSpec, Precision, SampleRate,
};
use proptest::collection::vec;
use proptest::test_runner::TestCaseError;

use crate::heap_operations_during;
use crate::matrix::{PRECISIONS, SPECIAL_INPUTS, SPECIAL_INPUTS_BINARY64, sample_rates};

/// What the laws read of a sample, in either precision.
trait LawSample: Raw {
    /// The special inputs of the matrix in this precision.
    fn special_inputs() -> [Self; 8];

    /// Whether it is neither NaN nor infinite.
    fn finite(self) -> bool;

    /// Whether it is subnormal in its precision.
    fn subnormal(self) -> bool;
}

impl LawSample for Binary64 {
    fn special_inputs() -> [Self; 8] {
        SPECIAL_INPUTS_BINARY64.map(Self::from_raw)
    }

    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

impl LawSample for Binary32 {
    fn special_inputs() -> [Self; 8] {
        SPECIAL_INPUTS.map(Self::from_raw)
    }

    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

/// Blocks rendered per run.
const BLOCKS: usize = 8;

/// Blocks rendered by the long run of each sample rate and parameter set: 65,536 frames,
/// 1.4 s at 48 kHz and 0.17 s at 384 kHz. Enough to reveal state that overflows, becomes
/// non-finite or uses the heap late; the run is compared with no reference, so slow drift
/// is not detected here (a drift check against the exact phase is planned with roadmap
/// #12).
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
/// port or parameter names, a non-finite range, or a default outside its range.
pub fn check_metadata(spec: &ObjectSpec) -> Result<(), String> {
    if spec.name.trim().is_empty() || spec.name.contains(char::is_whitespace) {
        return Err(format!("invalid box name \"{}\"", spec.name));
    }
    if spec.description.trim().is_empty() {
        return Err("missing description".to_owned());
    }
    unique_names("inlet", &mut spec.inlets.iter().map(|port| port.name))?;
    unique_names("outlet", &mut spec.outlets.iter().map(|port| port.name))?;
    unique_names("parameter", &mut spec.params.iter().map(|param| param.name))?;
    for port in spec.inlets.iter().chain(spec.outlets) {
        if port.description.trim().is_empty() {
            return Err(format!("port \"{}\" has no description", port.name));
        }
    }
    for param in spec.params {
        if param.description.trim().is_empty() {
            return Err(format!("parameter \"{}\" has no description", param.name));
        }
        let finite = param.min.is_finite() && param.max.is_finite();
        if !finite || !param.accepts(param.default) {
            return Err(format!(
                "parameter \"{}\": default {} outside finite range [{}, {}]",
                param.name, param.default, param.min, param.max
            ));
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
    unique_names("box", &mut registry.iter().map(|spec| spec.name))?;
    for spec in registry {
        check_metadata(spec).map_err(|problem| format!("{}: {problem}", spec.name))?;
    }
    Ok(())
}

/// Renders `blocks` blocks of a fresh instance computing in `S`, feeding consecutive input
/// blocks taken from `input` (then silence). Returns the outputs and the heap operations
/// made while processing.
///
/// The parameters are given as a plan gives them, in binary64 (`Binary64`), but as written:
/// not flushed, so that a box is shown not to break on a subnormal one.
///
/// # Errors
///
/// Returns why the heap operations cannot be counted.
fn render<S: LawSample>(
    spec: &ObjectSpec,
    rate: SampleRate,
    params: &[f32],
    input: &[S],
    blocks: usize,
) -> Result<(Vec<Block<S>>, usize), String> {
    let wide: Vec<Binary64> = params
        .iter()
        .map(|&param| Binary64::from_raw(f64::from(param)))
        .collect();
    let mut object: Box<dyn Object<S>> = spec.create.create(rate, &wide);
    let (chunked, _remainder) = input.as_chunks::<BLOCK_SIZE>();
    let mut chunks = chunked.iter();
    let mut inputs = vec![S::SILENCE; spec.inlets.len()];
    let mut outputs = vec![S::SILENCE; spec.outlets.len()];
    let mut rendered = Vec::with_capacity(blocks.saturating_mul(spec.outlets.len()));
    let mut operations = 0_usize;
    for _ in 0..blocks {
        for block in &mut inputs {
            for (sample, value) in block.iter_mut().zip(chunks.next().unwrap_or(&S::SILENCE)) {
                *sample = *value;
            }
        }
        let ((), count) = heap_operations_during(|| object.process(&inputs, &mut outputs))?;
        operations = operations.saturating_add(count);
        rendered.extend_from_slice(&outputs);
    }
    Ok((rendered, operations))
}

/// Bit patterns of rendered samples, so that `-0.0` and NaN payloads are compared exactly.
fn bits<S: LawSample>(blocks: &[Block<S>]) -> Vec<u64> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.bits())
        .collect()
}

/// A value of the range that differs from the default whenever the range allows it: the
/// midpoint, or the point halfway between the default and the maximum when the default
/// is the midpoint. So that a box hard-coding its default cannot pass.
fn away_from_default(param: &ParamSpec) -> f32 {
    let midpoint = param.min / 2.0 + param.max / 2.0;
    if midpoint.to_bits() == param.default.to_bits() {
        param.default / 2.0 + param.max / 2.0
    } else {
        midpoint
    }
}

/// `value` when the range of `param` accepts it (bit for bit, `-0.0` included), else
/// the default, which another set already tries.
fn within(param: &ParamSpec, value: f32) -> f32 {
    if param.accepts(value) {
        value
    } else {
        param.default
    }
}

/// Parameter sets every law is checked with: all minimums, all defaults, all values away
/// from their default, all maximums, then special values where the range accepts them:
/// `-0.0`, the smallest subnormal, `1e-45` (a plan flushes it to zero, but a box must not
/// break on it), and the smallest normal, `2^-126` (the smallest magnitude above zero a
/// plan passes on).
fn param_sets(spec: &ObjectSpec) -> [Vec<f32>; 7] {
    let special = |value: f32| {
        spec.params
            .iter()
            .map(|param| within(param, value))
            .collect()
    };
    [
        spec.params.iter().map(|param| param.min).collect(),
        spec.params.iter().map(|param| param.default).collect(),
        spec.params.iter().map(away_from_default).collect(),
        spec.params.iter().map(|param| param.max).collect(),
        special(-0.0),
        special(f32::from_bits(1)),
        special(f32::MIN_POSITIVE),
    ]
}

/// Checks that rendered blocks are finite and hold no subnormal, and that rendering made
/// no heap operation (no allocation, reallocation or release).
fn check_rendered<S: LawSample>(
    rendered: &[Block<S>],
    operations: usize,
    params: &[f32],
) -> Result<(), String> {
    if rendered.iter().flatten().any(|sample| !sample.finite()) {
        return Err(format!("non-finite output with parameters {params:?}"));
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

/// Checks one input against the determinism, finite-output, no-subnormal and no-heap laws.
fn check_input<S: LawSample>(
    spec: &ObjectSpec,
    rate: SampleRate,
    input: &[S],
) -> Result<(), String> {
    for params in param_sets(spec) {
        let (first, operations) = render(spec, rate, &params, input, BLOCKS)?;
        let (second, _) = render(spec, rate, &params, input, BLOCKS)?;
        if bits(&first) != bits(&second) {
            return Err(format!("not deterministic with parameters {params:?}"));
        }
        check_rendered(&first, operations, &params)?;
    }
    Ok(())
}

/// Checks a long run of silence-fed blocks, per parameter set: finite, without subnormal
/// and without heap operations.
fn check_long_run<S: LawSample>(spec: &ObjectSpec, rate: SampleRate) -> Result<(), String> {
    for params in param_sets(spec) {
        let (rendered, operations) = render::<S>(spec, rate, &params, &[], LONG_RUN_BLOCKS)?;
        check_rendered(&rendered, operations, &params)
            .map_err(|problem| format!("long run: {problem}"))?;
    }
    Ok(())
}

/// The special inputs of the matrix in the precision `S`, repeated to fill every inlet for
/// every block.
fn special_input<S: LawSample>(length: usize) -> Vec<S> {
    S::special_inputs()
        .iter()
        .copied()
        .cycle()
        .take(length)
        .collect()
}

/// Checks every law at one sample rate, computing in `S`.
fn check_rate<S: LawSample>(
    spec: &ObjectSpec,
    rate: SampleRate,
    length: usize,
) -> Result<(), String> {
    check_input(spec, rate, &special_input::<S>(length))
        .map_err(|problem| format!("special inputs: {problem}"))?;
    check_long_run::<S>(spec, rate)?;
    crate::runner::runner(CASES)
        .run(&vec(-1.0_f64..=1.0_f64, length), |input| {
            let samples: Vec<S> = input
                .into_iter()
                .map(|value| S::from_binary64(Binary64::from_raw(value)))
                .collect();
            check_input(spec, rate, &samples).map_err(TestCaseError::fail)
        })
        .map_err(|error| error.to_string())
}

/// Checks every law at every sample rate of the [`crate::matrix`], computing in `S`.
fn check_precision<S: LawSample>(spec: &ObjectSpec, length: usize) -> Result<(), String> {
    for rate in sample_rates() {
        check_rate::<S>(spec, rate, length)
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
pub fn check_laws(spec: &ObjectSpec) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    check_metadata(spec)?;
    let length = BLOCKS
        .saturating_mul(BLOCK_SIZE)
        .saturating_mul(spec.inlets.len());
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => check_precision::<Binary64>(spec, length),
            Precision::Binary32 => check_precision::<Binary32>(spec, length),
        }
        .map_err(|problem| format!("binary{}, {problem}", precision.bits()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
