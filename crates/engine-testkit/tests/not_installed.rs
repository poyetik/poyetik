//! Without the counting allocator, the no-allocation law cannot be checked: `check_laws`
//! must refuse instead of passing vacuously.

#[cfg(test)]
mod tests {
    use engine_core::{Block, Object, ObjectSpec};

    #[derive(Debug)]
    struct Silent;
    impl Object for Silent {
        fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {}
    }

    static SILENT: ObjectSpec = ObjectSpec {
        name: "silent~",
        description: "does nothing",
        inlets: &[],
        outlets: &[],
        params: &[],
        capabilities: &[],
        create: |_, _| Box::new(Silent),
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
