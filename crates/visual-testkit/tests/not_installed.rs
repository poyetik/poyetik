//! Without the counting allocator, the no-allocation law cannot be checked: `check_laws`
//! must refuse instead of passing vacuously.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use visual_core::{Image, ImageIndex, VisualBox};

    /// A box that leaves its image black.
    #[derive(Debug)]
    struct Black;

    impl VisualBox for Black {
        fn render(&mut self, _index: ImageIndex, output: &mut Image) {
            for row in output.rows_mut() {
                row.fill(0.0);
            }
        }
    }

    #[test]
    fn refuses_to_check_without_the_counting_allocator() {
        assert_eq!(
            visual_testkit::check_laws(&|_| -> Box<dyn VisualBox> { Box::new(Black) }),
            Err("CountingAllocator is not installed as the global allocator".to_owned()),
            "no vacuous pass"
        );
    }
}
