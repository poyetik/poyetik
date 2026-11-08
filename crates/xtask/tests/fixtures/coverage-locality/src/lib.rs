//! A crate whose unit tests reach one function, and whose integration test reaches both.

/// Reached by the unit tests.
#[must_use]
pub const fn by_unit_tests(x: u32) -> u32 {
    if x > 5 { x * 4 } else { x + 3 }
}

/// Reached only by the integration test: the coverage gate must report this file.
#[must_use]
pub const fn by_integration_tests_only(x: u32) -> u32 {
    if x > 5 { x * 6 } else { x + 7 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reaches_one_function() {
        assert_eq!(super::by_unit_tests(1), 4, "small");
        assert_eq!(super::by_unit_tests(9), 36, "large");
    }
}
