//! Tests of the registry.

/// The test binary counts heap operations, so that the no-heap law can be checked.
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use engine_core::host::{ParamBounds as _, Raw};
use engine_core::{Binary32, Binary64, Block, Dispatch, Object, ObjectSpec, ParamValues, Sample};
use engine_testkit::matrix::{SPECIAL_INPUTS_BINARY64, sample_rates};

use super::{BuiltIn, Kind, OSC, REGISTRY};

#[test]
fn every_box_passes_the_generic_laws() {
    for spec in REGISTRY {
        assert_eq!(
            engine_testkit::check_laws(spec),
            Ok(()),
            "{} breaks a law",
            spec.name
        );
    }
}

#[test]
fn the_registry_is_consistent() {
    assert_eq!(
        engine_testkit::check_registry(&REGISTRY),
        Ok(()),
        "unique, valid names"
    );
    let names: Vec<&str> = REGISTRY.iter().map(|spec| spec.name).collect();
    assert_eq!(names, ["osc~", "dac~"], "registered boxes");
}

#[test]
fn every_box_type_declares_its_algorithm_version() {
    // Each box type's version, as its algorithm stands (decision 29): a change of bits
    // takes a new one, and the conformance check refuses references blessed with another.
    let versions: Vec<(&str, u32)> = REGISTRY
        .iter()
        .map(|spec| (spec.name, spec.algorithm_version.get()))
        .collect();
    assert_eq!(versions, [("osc~", 1), ("dac~", 1)], "registered versions");
}

/// The benchmark baseline, written by `cargo xtask bless-benchmarks` (spec/benchmarks.md).
const BASELINE: &str = include_str!("../../../spec/benchmark-baseline.json");

#[test]
fn every_box_type_has_a_benchmark_and_every_benchmark_a_box_type() {
    let baseline: serde_json::Value = serde_json::from_str(BASELINE).expect("valid JSON");
    let measured: alloc::collections::BTreeSet<&str> = baseline["boxes"]
        .as_array()
        .expect("a list of box benchmarks")
        .iter()
        .map(|entry| entry["box"].as_str().expect("a box type"))
        .collect();
    let registered: alloc::collections::BTreeSet<&str> =
        REGISTRY.iter().map(|spec| spec.name).collect();
    assert_eq!(
        measured, registered,
        "the box types of spec/benchmark-baseline.json are those of the registry"
    );
}

/// A box type outside the registry: `osc~`'s spec under another name, at another address.
static COPY: ObjectSpec = ObjectSpec {
    name: "copy~",
    ..OSC
};

/// How [`BuiltIn`] holds an instance of `spec`: `"osc~"`, `"dac~"`, or `"other"` for the
/// virtual table.
fn held_as(spec: &'static ObjectSpec) -> &'static str {
    let rate = sample_rates()[0];
    match BuiltIn::<Binary64>::create(rate, &ParamValues::defaults(spec)).0 {
        Kind::Osc(_) => "osc~",
        Kind::Dac(_) => "dac~",
        Kind::Other(_) => "other",
    }
}

#[test]
fn calls_every_box_type_of_the_registry_directly() {
    let held: Vec<(&str, &str)> = REGISTRY
        .iter()
        .map(|spec| (spec.name, held_as(spec)))
        .collect();
    assert_eq!(
        held,
        [("osc~", "osc~"), ("dac~", "dac~")],
        "every registered box type has its own arm, none the virtual table"
    );
    assert_eq!(
        held_as(&COPY),
        "other",
        "a box type outside the registry, even a copy of one, goes through its factory"
    );
}

/// One block per inlet of the special values of the test matrix, each inlet and each
/// block starting at a different one.
fn inputs<S: Sample>(inlets: usize, block: usize) -> Vec<Block<S>> {
    (0..inlets)
        .map(|inlet| {
            let mut input = S::SILENCE;
            let values = SPECIAL_INPUTS_BINARY64
                .iter()
                .cycle()
                .skip(inlet)
                .skip(block);
            for (sample, &value) in input.iter_mut().zip(values) {
                *sample = engine_testkit::sample_of(value);
            }
            input
        })
        .collect()
}

/// The bit pattern of every sample of `blocks`, block by block.
fn bits<S: Raw>(blocks: &[Block<S>]) -> Vec<u64> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.bits())
        .collect()
}

/// The parameter lists `spec` is tried with: its defaults, then each parameter at its
/// minimum, at its maximum and at three eighths of its range, the others at their default.
fn param_lists(spec: &'static ObjectSpec) -> Vec<ParamValues> {
    let defaults: Vec<Binary64> = spec
        .params
        .iter()
        .map(|param| Binary64::from_raw(f64::from(param.default_value())))
        .collect();
    let mut lists = vec![ParamValues::defaults(spec)];
    for (index, param) in spec.params.iter().enumerate() {
        let (low, high) = (f64::from(param.minimum()), f64::from(param.maximum()));
        for value in [low, high, low + (high - low) * 0.375_f64] {
            let values: Vec<Binary64> = defaults
                .iter()
                .enumerate()
                .map(|(at, default)| {
                    if at == index {
                        Binary64::from_raw(value)
                    } else {
                        *default
                    }
                })
                .collect();
            lists.push(ParamValues::of(spec, &values).expect("within range"));
        }
    }
    lists
}

/// Checks, at every sample rate and for several parameter lists ([`param_lists`]), that
/// `spec` held by [`BuiltIn`] gives the same blocks, bit for bit, as its factory's instance,
/// and that processing performs no heap operation.
fn same_as_the_factory<S: Raw>(spec: &'static ObjectSpec) {
    let params = param_lists(spec);
    for rate in sample_rates() {
        for list in &params {
            let mut direct = BuiltIn::<S>::create(rate, list);
            let mut reference = <Box<dyn Object<S>> as Dispatch<S>>::create(rate, list);
            for block in 0..3 {
                let given = inputs::<S>(spec.inlets.len(), block);
                let mut out = vec![S::SILENCE; spec.outlets.len()];
                let mut expected = out.clone();
                let ((), operations) = engine_testkit::heap_operations_during(|| {
                    direct.process(&given, &mut out);
                })
                .expect("the counting allocator is installed");
                assert_eq!(operations, 0, "{} allocates while processing", spec.name);
                reference.process(&given, &mut expected);
                // Bit for bit: equality of the samples would take -0 for +0.
                assert_eq!(
                    bits(&out),
                    bits(&expected),
                    "{} at {rate:?} with {list:?}, block {block}",
                    spec.name
                );
            }
        }
    }
}

#[test]
fn processes_every_box_type_as_its_factory_does() {
    for spec in REGISTRY.iter().copied().chain([&COPY]) {
        same_as_the_factory::<Binary64>(spec);
        same_as_the_factory::<Binary32>(spec);
    }
}
