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

/// The larger of two numbers, the first when they compare equal: `f64::max` is banned
/// (`clippy.toml`), since it may return either zero when they are `+0` and `-0`, a sign that
/// would differ between targets.
pub(crate) fn larger(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

#[test]
fn takes_the_larger_number_and_the_first_of_equal_ones() {
    assert_eq!(
        larger(1.0_f64, 2.0_f64).to_bits(),
        2.0_f64.to_bits(),
        "second"
    );
    assert_eq!(
        larger(2.0_f64, 1.0_f64).to_bits(),
        2.0_f64.to_bits(),
        "first"
    );
    assert_eq!(
        larger(0.0_f64, -0.0_f64).to_bits(),
        0.0_f64.to_bits(),
        "+0 first"
    );
    assert_eq!(
        larger(-0.0_f64, 0.0_f64).to_bits(),
        (-0.0_f64).to_bits(),
        "-0 first"
    );
}
