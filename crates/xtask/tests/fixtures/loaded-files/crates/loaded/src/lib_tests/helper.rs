//! A file only the tests load, named as no test code.

#[test]
fn is_one() {
    assert_eq!(crate::one(), 1, "one");
}
