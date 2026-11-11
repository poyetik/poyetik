//! Without the counting allocator, the no-allocation law cannot be checked: `check_laws`
//! must refuse instead of passing vacuously.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use engine_core::{
        AlgorithmVersion, Block, Create, Factory, Object, ObjectSpec, Params, Sample, SampleRate,
    };

    #[derive(Debug)]
    struct Silent;
    impl<S: Sample> Object<S> for Silent {
        fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
    }
    impl Create for Silent {
        fn create<S: Sample>(_rate: SampleRate, _params: Params<'_>) -> Box<dyn Object<S>> {
            Box::new(Self)
        }
    }

    static SILENT: ObjectSpec = ObjectSpec {
        name: "silent~",
        description: "does nothing",
        inlets: &[],
        outlets: &[],
        params: &[],
        capabilities: &[],
        create: Factory::new::<Silent>(),
        algorithm_version: AlgorithmVersion::FIRST,
    };

    #[test]
    fn refuses_to_check_without_the_counting_allocator() {
        assert_eq!(
            engine_testkit::check_laws(&SILENT),
            Err("CountingAllocator is not installed as the global allocator".to_owned()),
            "no vacuous pass"
        );
    }
}
