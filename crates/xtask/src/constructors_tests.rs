//! Tests of the delegating constructor gate.

use super::delegating_new_gate;
use crate::steps::Failure;

/// The gate's verdict on one source of a crate at `crates/a/src/lib.rs`.
fn verdict(text: &str) -> Result<(), Failure> {
    delegating_new_gate(&[("crates/a/src/lib.rs".to_owned(), text.to_owned())])
}

/// The problem of a `new` holding more, at `line` of `crates/a/src/lib.rs`.
fn holding(line: usize) -> String {
    format!(
        "crates/a/src/lib.rs:{line}: a `new` holding more than one call to a function of another \
         name, its parameters passed on: cargo-mutants never mutates a function named `new`, so \
         move its body into one it mutates"
    )
}

#[test]
fn accepts_a_new_passing_its_parameters_on_to_another_function() {
    for text in [
        "impl A {\n    pub fn new(w: u32, h: u32) -> Self {\n        Self::checked(w, h)\n    }\n}\n",
        "impl A {\n    pub const fn new<T: B<K>, const K: usize>() -> Self {\n        Self::made::<T, K>()\n    }\n}\n",
        "impl A {\n    fn new(a: u8,) -> Self {\n        other::of(a,)\n    }\n}\n",
        "trait A {\n    fn new() -> Self;\n}\n",
        "fn renew() -> u8 {\n    1 + 1\n}\n",
    ] {
        assert_eq!(verdict(text), Ok(()), "{text}");
    }
}

#[test]
fn refuses_a_new_holding_more_naming_its_line() {
    for (text, line) in [
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self(a)\n    }\n}\n",
            2,
        ),
        (
            "\nimpl A {\n    fn new(a: u8) -> Self {\n        Self { a }\n    }\n}\n",
            3,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self::new(a)\n    }\n}\n",
            2,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self::of(a + 1)\n    }\n}\n",
            2,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self::of((a))\n    }\n}\n",
            2,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        let b = a;\n        Self::of(b)\n    }\n}\n",
            2,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self::of(a).into()\n    }\n}\n",
            2,
        ),
        (
            "impl A {\n    fn new(a: u8) -> Self {\n        Self::of::<u8(a)\n    }\n}\n",
            2,
        ),
        ("impl A {\n    fn new() -> Self {}\n}\n", 2),
        (
            "impl A {\n    fn new() -> Self {\n        S::of\n    }\n}\n",
            2,
        ),
    ] {
        assert_eq!(verdict(text), Err(Failure::from(holding(line))), "{text}");
    }
}

#[test]
fn reads_the_sources_of_crates_alone_and_lists_every_problem() {
    let held = "impl A {\n    fn new(a: u8) -> Self {\n        Self(a)\n    }\n}\n".to_owned();
    let sources = [
        ("crates/a/src/lib.rs".to_owned(), format!("{held}{held}")),
        ("crates/a/src/lib_tests.rs".to_owned(), held.clone()),
        ("crates/a/tests/laws.rs".to_owned(), held.clone()),
        ("crates/a/build.rs".to_owned(), held.clone()),
        ("apps/b/src/lib.rs".to_owned(), held.clone()),
        ("crates/a/src/notes.md".to_owned(), held),
    ];
    assert_eq!(
        delegating_new_gate(&sources),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![holding(2), holding(7)],
            usage: false,
        })
    );
    assert_eq!(
        delegating_new_gate(&[("crates/a/src/lib.rs".to_owned(), "fn new( {".to_owned())]),
        Err(Failure::from(
            "crates/a/src/lib.rs:1: does not lex as Rust (cannot parse string into token \
             stream): the gates read the tokens of every Rust source"
                .to_owned()
        )),
        "a source that does not lex"
    );
}
