//! Reaches every function, from outside the crate.

#[test]
fn reaches_every_function() {
    assert_eq!(coverage_locality::by_unit_tests(9), 36, "large");
    assert_eq!(coverage_locality::by_integration_tests_only(9), 54, "large");
    assert_eq!(coverage_locality::by_integration_tests_only(1), 8, "small");
}
