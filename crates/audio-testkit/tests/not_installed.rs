//! Without the counting allocator, the no-allocation law cannot be checked: `check_laws`
//! must refuse instead of passing vacuously.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use audio_core::{
        AlgorithmVersion, Block, Create, Object, ObjectSpec, ParamSpec, Params, Sample, SampleRate,
    };

    #[derive(Debug)]
    struct Silent;
    impl<S: Sample> Object<S> for Silent {
        fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
    }
    impl Create for Silent {
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample>(_rate: SampleRate, _params: Params<'_>) -> Box<dyn Object<S>> {
            Box::new(Self)
        }
    }

    static SILENT: ObjectSpec = ObjectSpec::new::<Silent>(
        "silent~",
        "does nothing",
        &[],
        &[],
        &[],
        AlgorithmVersion::FIRST,
    );

    #[test]
    fn refuses_to_check_without_the_counting_allocator() {
        assert_eq!(
            audio_testkit::check_laws(&SILENT),
            Err("CountingAllocator is not installed as the global allocator".to_owned()),
            "no vacuous pass"
        );
    }
}
