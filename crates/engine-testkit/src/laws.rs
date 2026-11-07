//! The laws every box must respect.

use alloc::collections::BTreeSet;

use engine_core::{BLOCK_SIZE, Block, Object, ObjectSpec, SILENCE, SampleRate};
use proptest::collection::vec;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};

use crate::allocations_during;

/// Blocks rendered per run.
const BLOCKS: usize = 8;

/// Random input cases tried per law.
const CASES: u32 = 32;

/// Checks that a list of names is free of empty and duplicate names.
fn unique_names<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> Result<(), String> {
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
    unique_names("inlet", spec.inlets.iter().map(|p| p.name))?;
    unique_names("outlet", spec.outlets.iter().map(|p| p.name))?;
    unique_names("parameter", spec.params.iter().map(|p| p.name))?;
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
    unique_names("box", registry.iter().map(|spec| spec.name))?;
    for spec in registry {
        check_metadata(spec).map_err(|problem| format!("{}: {problem}", spec.name))?;
    }
    Ok(())
}

/// Renders `blocks` blocks of a fresh instance, feeding consecutive input blocks taken
/// from `input`. Returns the outputs and the allocations made while processing.
fn render(
    spec: &ObjectSpec,
    rate: SampleRate,
    params: &[f32],
    input: &[f32],
) -> (Vec<Block>, usize) {
    let mut object: Box<dyn Object> = (spec.create)(rate, params);
    let (blocks, _remainder) = input.as_chunks::<BLOCK_SIZE>();
    let mut chunks = blocks.iter();
    let mut inputs = vec![SILENCE; spec.inlets.len()];
    let mut outputs = vec![SILENCE; spec.outlets.len()];
    let mut rendered = Vec::with_capacity(BLOCKS.saturating_mul(spec.outlets.len()));
    let mut allocations = 0_usize;
    for _ in 0..BLOCKS {
        for block in &mut inputs {
            for (sample, value) in block.iter_mut().zip(chunks.next().unwrap_or(&SILENCE)) {
                *sample = *value;
            }
        }
        let ((), count) = allocations_during(|| object.process(&inputs, &mut outputs));
        allocations = allocations.saturating_add(count);
        rendered.extend_from_slice(&outputs);
    }
    (rendered, allocations)
}

/// Bit patterns of rendered samples, so that `-0.0` and NaN payloads are compared exactly.
fn bits(blocks: &[Block]) -> Vec<u32> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.to_bits())
        .collect()
}

/// Parameter sets every law is checked with: all minimums, all defaults, all maximums.
fn param_sets(spec: &ObjectSpec) -> [Vec<f32>; 3] {
    [
        spec.params.iter().map(|p| p.min).collect(),
        spec.params.iter().map(|p| p.default).collect(),
        spec.params.iter().map(|p| p.max).collect(),
    ]
}

/// Checks one random input against the determinism, finite-output and no-allocation laws.
fn check_input(spec: &ObjectSpec, rate: SampleRate, input: &[f32]) -> Result<(), String> {
    for params in param_sets(spec) {
        let (first, allocations) = render(spec, rate, &params, input);
        let (second, _) = render(spec, rate, &params, input);
        if bits(&first) != bits(&second) {
            return Err(format!("not deterministic with parameters {params:?}"));
        }
        if first.iter().flatten().any(|sample| !sample.is_finite()) {
            return Err(format!("non-finite output with parameters {params:?}"));
        }
        if allocations > 0 {
            return Err(format!(
                "allocates while processing ({allocations} allocations)"
            ));
        }
    }
    Ok(())
}

/// Checks every law on a box type, with random inputs from a fixed seed.
///
/// # Errors
///
/// Returns a description of the first broken law. Also fails when
/// [`crate::CountingAllocator`] is not installed, since the no-allocation law could not
/// be checked.
pub fn check_laws(spec: &ObjectSpec) -> Result<(), String> {
    let ((), probe) = allocations_during(|| drop(Box::new(0_u8)));
    if probe == 0 {
        return Err("CountingAllocator is not installed as the global allocator".to_owned());
    }
    check_metadata(spec)?;
    let rate = SampleRate::HZ_48000;
    let length = BLOCKS
        .saturating_mul(BLOCK_SIZE)
        .saturating_mul(spec.inlets.len());
    let config = Config {
        cases: CASES,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha));
    runner
        .run(&vec(-1.0_f32..=1.0_f32, length), |input| {
            check_input(spec, rate, &input).map_err(TestCaseError::fail)
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
