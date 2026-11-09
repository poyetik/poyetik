//! Tests of the registry.

use super::REGISTRY;

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
    let names: alloc::vec::Vec<&str> = REGISTRY.iter().map(|spec| spec.name).collect();
    assert_eq!(names, ["osc~", "dac~"], "registered boxes");
}

#[test]
fn every_box_type_declares_its_algorithm_version() {
    // Each box type's version, as its algorithm stands (decision 29): a change of bits
    // takes a new one, and the conformance check refuses references blessed with another.
    let versions: alloc::vec::Vec<(&str, u32)> = REGISTRY
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
