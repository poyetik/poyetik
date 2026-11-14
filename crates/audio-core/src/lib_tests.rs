//! What the unit tests of several modules of the crate share.

/// Runs each generic test in both precisions, each instantiation a test of its own: every
/// instantiation of the generic code is covered on its own.
macro_rules! in_both_precisions {
    ($($test:ident),* $(,)?) => {
        /// The tests in binary64, the default precision.
        mod binary64 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary64>();
                }
            )*
        }
        /// The tests in binary32.
        mod binary32 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary32>();
                }
            )*
        }
    };
}

pub(crate) use in_both_precisions;
