//! The laws every box must respect.

use alloc::collections::BTreeSet;

use engine_core::{BLOCK_SIZE, Block, Object, ObjectSpec, ParamSpec, SILENCE, SampleRate};
use proptest::collection::vec;
use proptest::test_runner::TestCaseError;

use crate::heap_operations_during;
use crate::matrix::{SPECIAL_INPUTS, sample_rates};

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

/// Renders `blocks` blocks of a fresh instance, feeding consecutive input blocks taken
/// from `input` (then silence). Returns the outputs and the heap operations made while
/// processing.
///
/// # Errors
///
/// Returns why the heap operations cannot be counted.
fn render(
    spec: &ObjectSpec,
    rate: SampleRate,
    params: &[f32],
    input: &[f32],
    blocks: usize,
) -> Result<(Vec<Block>, usize), String> {
    let mut object: Box<dyn Object> = (spec.create)(rate, params);
    let (chunked, _remainder) = input.as_chunks::<BLOCK_SIZE>();
    let mut chunks = chunked.iter();
    let mut inputs = vec![SILENCE; spec.inlets.len()];
    let mut outputs = vec![SILENCE; spec.outlets.len()];
    let mut rendered = Vec::with_capacity(blocks.saturating_mul(spec.outlets.len()));
    let mut operations = 0_usize;
    for _ in 0..blocks {
        for block in &mut inputs {
            for (sample, value) in block.iter_mut().zip(chunks.next().unwrap_or(&SILENCE)) {
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
fn bits(blocks: &[Block]) -> Vec<u32> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.to_bits())
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
/// from their default, all maximums, then the special values a patch can hold (`-0.0`
/// and the smallest subnormal, `1e-45`) where the range accepts them.
fn param_sets(spec: &ObjectSpec) -> [Vec<f32>; 6] {
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
    ]
}

/// Checks that rendered blocks are finite and that rendering made no heap operation (no
/// allocation, reallocation or release).
fn check_finite_without_heap(
    rendered: &[Block],
    operations: usize,
    params: &[f32],
) -> Result<(), String> {
    if rendered.iter().flatten().any(|sample| !sample.is_finite()) {
        return Err(format!("non-finite output with parameters {params:?}"));
    }
    if operations > 0 {
        return Err(format!(
            "uses the heap while processing ({operations} heap operations)"
        ));
    }
    Ok(())
}

/// Checks one input against the determinism, finite-output and no-heap laws.
fn check_input(spec: &ObjectSpec, rate: SampleRate, input: &[f32]) -> Result<(), String> {
    for params in param_sets(spec) {
        let (first, operations) = render(spec, rate, &params, input, BLOCKS)?;
        let (second, _) = render(spec, rate, &params, input, BLOCKS)?;
        if bits(&first) != bits(&second) {
            return Err(format!("not deterministic with parameters {params:?}"));
        }
        check_finite_without_heap(&first, operations, &params)?;
    }
    Ok(())
}

/// Checks a long run of silence-fed blocks, per parameter set: finite and without heap
/// operations.
fn check_long_run(spec: &ObjectSpec, rate: SampleRate) -> Result<(), String> {
    for params in param_sets(spec) {
        let (rendered, operations) = render(spec, rate, &params, &[], LONG_RUN_BLOCKS)?;
        check_finite_without_heap(&rendered, operations, &params)
            .map_err(|problem| format!("long run: {problem}"))?;
    }
    Ok(())
}

/// The special inputs of the matrix, repeated to fill every inlet for every block.
fn special_input(length: usize) -> Vec<f32> {
    SPECIAL_INPUTS
        .iter()
        .copied()
        .cycle()
        .take(length)
        .collect()
}

/// Checks every law at one sample rate.
fn check_rate(spec: &ObjectSpec, rate: SampleRate, length: usize) -> Result<(), String> {
    check_input(spec, rate, &special_input(length))
        .map_err(|problem| format!("special inputs: {problem}"))?;
    check_long_run(spec, rate)?;
    crate::runner::runner(CASES)
        .run(&vec(-1.0_f32..=1.0_f32, length), |input| {
            check_input(spec, rate, &input).map_err(TestCaseError::fail)
        })
        .map_err(|error| error.to_string())
}

/// Checks every law on a box type at every sample rate of the [`crate::matrix`]: special
/// inputs, a long run, and random inputs from a fixed seed.
///
/// # Errors
///
/// Returns a description of the first broken law, prefixed by the sample rate. Also
/// fails when [`crate::CountingAllocator`] is not installed, since the no-heap law could
/// not be checked.
pub fn check_laws(spec: &ObjectSpec) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    check_metadata(spec)?;
    let length = BLOCKS
        .saturating_mul(BLOCK_SIZE)
        .saturating_mul(spec.inlets.len());
    for rate in sample_rates() {
        check_rate(spec, rate, length)
            .map_err(|problem| format!("at {} Hz: {problem}", rate.hz()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
