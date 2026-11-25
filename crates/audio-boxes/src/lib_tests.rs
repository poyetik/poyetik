//! Tests of the registry.

/// The test binary counts heap operations, so that the no-heap law can be checked.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use audio_core::host::{ParamBounds as _, Raw};
use audio_core::{
    AlgorithmVersion, Binary32, Binary64, Block, BoxInstance, Dispatch, DrivenValues, Object,
    ObjectSpec, ParamValues, PortSpec, Sample,
};
use audio_testkit::matrix::{SPECIAL_INPUTS_BINARY64, sample_rates};

use super::{BuiltIn, Kind, REGISTRY};

#[test]
fn every_box_passes_the_generic_laws() {
    for spec in REGISTRY {
        assert_eq!(
            audio_testkit::check_laws(spec),
            Ok(()),
            "{} breaks a law",
            spec.name()
        );
    }
}

#[test]
fn every_box_gives_the_same_bits_by_either_body() {
    for spec in REGISTRY {
        assert_eq!(
            audio_testkit::check_driven_laws::<
                BuiltIn<Binary64>,
                BuiltIn<Binary64, 1>,
                BuiltIn<Binary32>,
                BuiltIn<Binary32, 1>,
                1,
            >(spec),
            Ok(()),
            "{}: a parameter changing at any frame",
            spec.name()
        );
        // Through its factory too: its smallest engine block is 16 frames.
        assert_eq!(
            audio_testkit::check_driven_laws::<
                Box<dyn BoxInstance<Binary64>>,
                Box<dyn BoxInstance<Binary64, 16>>,
                Box<dyn BoxInstance<Binary32>>,
                Box<dyn BoxInstance<Binary32, 16>>,
                16,
            >(spec),
            Ok(()),
            "{}: through its factory",
            spec.name()
        );
    }
}

#[test]
fn the_registry_is_consistent() {
    assert_eq!(
        audio_testkit::check_registry(&REGISTRY),
        Ok(()),
        "unique, valid names"
    );
    let names: Vec<&str> = REGISTRY.iter().map(|spec| spec.name()).collect();
    assert_eq!(names, ["osc~", "dac~"], "registered boxes");
}

#[test]
fn every_box_type_declares_its_algorithm_version() {
    // Each box type's version, as its algorithm stands (decision 29): a change of bits
    // takes a new one, and the conformance check refuses references blessed with another.
    let versions: Vec<(&str, u32)> = REGISTRY
        .iter()
        .map(|spec| (spec.name(), spec.algorithm_version().get()))
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
        REGISTRY.iter().map(|spec| spec.name()).collect();
    assert_eq!(
        measured, registered,
        "the box types of spec/benchmark-baseline.json are those of the registry"
    );
}

/// A box type outside the registry: a second spec of `osc~`'s type, the one copy its own
/// crate can write (its type, private, is named nowhere else), under another name, at
/// another address: its parameters and instances are still `osc~`'s.
static COPY: ObjectSpec = ObjectSpec::new::<crate::osc::Osc, 1>(
    "copy~",
    "a second spec of the sine oscillator",
    &[],
    &[PortSpec {
        name: "out",
        description: "The sine wave",
    }],
    &[],
    AlgorithmVersion::FIRST,
);

/// A second spec of `dac~`'s type, as [`COPY`] is of `osc~`'s: the box type of no
/// parameter whose values an instance of `dac~` refuses as another box type's.
static DAC_COPY: ObjectSpec = ObjectSpec::new::<crate::dac::Dac, 0>(
    "dac-copy~",
    "a second spec of the output",
    &[],
    &[],
    &[],
    AlgorithmVersion::FIRST,
);

/// A box type of as many parameters as `spec`, at another address: `osc~` and [`COPY`] each
/// the other's, [`DAC_COPY`] `dac~`'s, so that an instance of `spec` refusing its values
/// refuses them by their box type, their number the same (the cheat of finding D14-1 of the
/// second deep review of #12 block 3 ("3a")).
fn twin_of(spec: &'static ObjectSpec) -> &'static ObjectSpec {
    if core::ptr::eq(spec, &raw const COPY) {
        &crate::osc::OSC
    } else if spec.params().is_empty() {
        &DAC_COPY
    } else {
        &COPY
    }
}

/// How [`BuiltIn`] holds an instance of `spec`: `"osc~"`, `"dac~"`, or `"other"` for the
/// virtual table.
fn held_as(spec: &'static ObjectSpec) -> &'static str {
    let rate = sample_rates()[0];
    match BuiltIn::<Binary64>::create(rate, &ParamValues::defaults(spec))
        .expect("a test build creates every box type")
        .0
    {
        Kind::Osc(_) => "osc~",
        Kind::Dac(_) => "dac~",
        Kind::Other(_) => "other",
    }
}

#[test]
fn calls_every_box_type_of_the_registry_directly() {
    let held: Vec<(&str, &str)> = REGISTRY
        .iter()
        .map(|spec| (spec.name(), held_as(spec)))
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
                *sample = audio_testkit::sample_of(value);
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
        .params()
        .iter()
        .map(|param| Binary64::from_raw(param.default_value()))
        .collect();
    let mut lists = vec![ParamValues::defaults(spec)];
    for (index, param) in spec.params().iter().enumerate() {
        let (low, high) = (param.minimum(), param.maximum());
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

/// A box type of another number of parameters than `spec`'s: whose values and blocks of
/// values an instance of `spec` refuses.
fn other_than(spec: &'static ObjectSpec) -> &'static ObjectSpec {
    REGISTRY
        .iter()
        .copied()
        .find(|other| other.params().len() != spec.params().len())
        .expect("the registry holds box types of 0 and 1 parameters")
}

/// The blocks of values driving `list`'s box type over a block: each parameter at its value
/// of `list` on even frames, at its default on odd ones, so that a body reading one frame
/// for all of them, or none, differs.
fn driving(list: &ParamValues) -> DrivenValues {
    let spec = list.spec();
    let blocks = spec
        .params()
        .iter()
        .zip(list.params().iter())
        .map(|(param, &value)| {
            core::array::from_fn(|frame| {
                if frame % 2 == 0 {
                    value
                } else {
                    Binary64::from_raw(param.default_value())
                }
            })
        })
        .collect();
    DrivenValues::of(spec, blocks).expect("values of the box type")
}

/// Gives `direct` and `reference` the values of `list`, then drives a block of `given`
/// through both by [`driving`]; then the values and the blocks of another number of
/// parameters, and those of a box type of as many ([`twin_of`]), which both refuse, leaving
/// their outputs as they were. The same answers, and
/// the same bits, from NaN outputs on entry.
fn driven_as_the_factory<S: Raw>(
    direct: &mut BuiltIn<S>,
    reference: &mut Box<dyn BoxInstance<S>>,
    list: &ParamValues,
    given: &[Block<S>],
) {
    let spec = list.spec();
    let name = spec.name();
    assert!(direct.receive(list), "{name}: its values");
    assert!(
        reference.receive(list),
        "{name}: its values, by its factory"
    );
    let nan = S::from_binary64(Binary64::from_raw(f64::NAN));
    let mut out = vec![[nan; audio_core::BLOCK_SIZE]; spec.outlets().len()];
    let mut expected = out.clone();
    let values = driving(list);
    assert!(
        direct.run_driven(given, &mut out, &values),
        "{name}: driven"
    );
    assert!(
        reference.run_driven(given, &mut expected, &values),
        "{name}: driven by its factory"
    );
    assert_eq!(bits(&out), bits(&expected), "{name} driven, with {list:?}");
    let before = bits(&out);
    for other in [other_than(spec), twin_of(spec)] {
        let wrong = ParamValues::defaults(other);
        let of = other.name();
        assert!(!direct.receive(&wrong), "{name}: {of}'s values");
        assert!(
            !reference.receive(&wrong),
            "{name}: {of}'s values, by its factory"
        );
        let refused = driving(&wrong);
        assert!(
            !direct.run_driven(given, &mut out, &refused),
            "{name}: {of}'s blocks"
        );
        assert!(
            !reference.run_driven(given, &mut expected, &refused),
            "{name}: {of}'s blocks, by its factory"
        );
        assert_eq!(bits(&out), before, "{name}: nothing written");
        assert_eq!(
            bits(&expected),
            before,
            "{name}: nothing written by its factory"
        );
    }
}

/// Checks, at every sample rate and for several parameter lists ([`param_lists`]), that
/// `spec` held by [`BuiltIn`] gives the same blocks, bit for bit, as its factory's instance,
/// by either body ([`driven_as_the_factory`]), and that processing performs no heap
/// operation.
fn same_as_the_factory<S: Raw>(spec: &'static ObjectSpec) {
    let params = param_lists(spec);
    for rate in sample_rates() {
        for list in &params {
            let mut direct =
                BuiltIn::<S>::create(rate, list).expect("a test build creates every box type");
            let mut reference = <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, list)
                .expect("a test build creates every box type");
            for block in 0..3 {
                let given = inputs::<S>(spec.inlets().len(), block);
                let mut out = vec![S::SILENCE; spec.outlets().len()];
                let mut expected = out.clone();
                let ((), operations) = audio_testkit::heap_operations_during(|| {
                    direct.process(&given, &mut out);
                })
                .expect("the counting allocator is installed");
                assert_eq!(operations, 0, "{} allocates while processing", spec.name());
                reference.process(&given, &mut expected);
                // Bit for bit: equality of the samples would take -0 for +0.
                assert_eq!(
                    bits(&out),
                    bits(&expected),
                    "{} at {rate:?} with {list:?}, block {block}",
                    spec.name()
                );
            }
            let given = inputs::<S>(spec.inlets().len(), 3);
            driven_as_the_factory(&mut direct, &mut reference, list, &given);
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

/// `osc~` into both channels of `dac~`, and `copy~` (a box type outside the registry) when
/// `with_copy`, into the left channel too.
fn sine_patch(with_copy: bool) -> audio_core::Patch {
    let node = |id: u32, kind: &str| audio_core::Node {
        id: audio_core::NodeId(id),
        kind: kind.into(),
        params: alloc::collections::BTreeMap::new(),
    };
    let wire = |from: u32, to: u32, inlet: usize| audio_core::Connection {
        from: audio_core::NodeId(from),
        outlet: 0,
        to: audio_core::NodeId(to),
        inlet,
    };
    let mut nodes = vec![node(1, "osc~"), node(2, "dac~")];
    let mut cables = vec![wire(1, 2, 0), wire(1, 2, 1)];
    if with_copy {
        nodes.push(node(3, "copy~"));
        cables.push(wire(3, 2, 0));
    }
    audio_core::Patch::new(nodes, cables).expect("a valid patch")
}

/// The sample types, rendered as their own floats.
trait Rendered: Raw<Float: audio_core::host::Delivered<Self>> {}
impl Rendered for Binary64 {}
impl Rendered for Binary32 {}

/// The bits of `frames` frames of [`sine_patch`] (without `copy~`) at 44.1 kHz, on the
/// engine's plan in blocks of `N` frames.
fn engine_render<S: Rendered, const N: usize>(frames: usize) -> Vec<u64> {
    let rate = audio_core::SampleRate::new(44_100).expect("a valid rate");
    crate::EngineRenderer::<S, S::Float, N>::new(
        crate::EnginePlan::<S, N>::build(
            &sine_patch(false),
            &REGISTRY,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .expect("a plan of built-in box types, at any engine block"),
    )
    .render(frames)
    .expect("a small render")
    .into_iter()
    .map(|sample| S::from_raw(sample).bits())
    .collect()
}

/// [`engine_render`] at 1 frame and at 441 (an export size outside `ENGINE_BLOCKS`, 10 ms
/// at 44.1 kHz) against 64, the default: the same bits (decision 2).
fn runs_the_built_in_box_types_at_any_engine_block<S: Rendered>() {
    // Three blocks of 441 frames and a part: every size crosses block boundaries.
    let frames = 1_400;
    let default = engine_render::<S, 64>(frames);
    assert_eq!(default.len(), frames * 2, "stereo");
    assert_eq!(engine_render::<S, 1>(frames), default, "blocks of 1 frame");
    assert_eq!(
        engine_render::<S, 441>(frames),
        default,
        "blocks of 441 frames"
    );
}

#[test]
fn runs_the_built_in_box_types_at_any_engine_block_in_either_precision() {
    runs_the_built_in_box_types_at_any_engine_block::<Binary64>();
    runs_the_built_in_box_types_at_any_engine_block::<Binary32>();
}

/// A box type outside the registry, held at 441 frames, which no build holds through its
/// virtual table: refused by the plan, naming it (`CoreError::Unbuilt`).
fn refuses_a_box_type_of_its_virtual_table_outside_the_engine_blocks<S: Raw>() {
    let rate = audio_core::SampleRate::new(44_100).expect("a valid rate");
    assert!(
        BuiltIn::<S, 441>::create(rate, &ParamValues::defaults(&COPY)).is_none(),
        "no instance through the virtual table"
    );
    assert!(
        BuiltIn::<S, 441>::create(rate, &ParamValues::defaults(&crate::OSC)).is_some(),
        "a built-in one"
    );
    let mut registry = REGISTRY.to_vec();
    registry.push(&COPY);
    assert_eq!(
        crate::EnginePlan::<S, 441>::build(
            &sine_patch(true),
            &registry,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .map(|_| ()),
        Err(audio_core::CoreError::Unbuilt {
            node: audio_core::NodeId(3),
            kind: "copy~".into(),
            frames: 441,
        }),
        "the box type outside the registry"
    );
    // At 64 frames, a size of `ENGINE_BLOCKS` a test build holds: the same patch builds.
    assert!(
        crate::EnginePlan::<S, 64>::build(
            &sine_patch(true),
            &registry,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .is_ok(),
        "built at 64 frames"
    );
}

#[test]
fn refuses_a_box_type_of_its_virtual_table_outside_the_engine_blocks_in_either_precision() {
    refuses_a_box_type_of_its_virtual_table_outside_the_engine_blocks::<Binary64>();
    refuses_a_box_type_of_its_virtual_table_outside_the_engine_blocks::<Binary32>();
}
