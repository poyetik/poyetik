//! Poyetik boxes.
//!
//! Each box type lives in its own module and depends only on the `engine-core`
//! contract. [`REGISTRY`] lists every box type; being in the registry is enough for a
//! box to be checked against the generic laws of `engine-testkit`.
//!
//! Like the core, this crate is `no_std` and forbids `unsafe`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod dac;
mod osc;
mod sample;

use engine_core::ObjectSpec;

pub use dac::DAC;
pub use osc::OSC;

/// Every box type of the engine.
pub static REGISTRY: [&ObjectSpec; 2] = [&OSC, &DAC];

/// The test binary counts heap operations, so that the no-heap law can be checked.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
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
}
