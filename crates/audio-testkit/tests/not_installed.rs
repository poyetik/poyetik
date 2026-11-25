//! Without the counting allocator, the no-allocation law cannot be checked: `check_laws`
//! must refuse instead of passing vacuously.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use audio_core::{
        AlgorithmVersion, Binary64, Block, Create, Driven, Object, ObjectSpec, ParamSpec, Sample,
        SampleRate,
    };

    #[derive(Debug)]
    struct Silent;
    impl<S: Sample, const N: usize> Object<S, N> for Silent {
        fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
    }
    impl Create<0> for Silent {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec; 0] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            []: [Binary64; 0],
        ) -> impl Driven<S, N, 0> + 'static {
            Self
        }
    }

    static SILENT: ObjectSpec = ObjectSpec::new::<Silent, 0>(
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
