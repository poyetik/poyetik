//! Tests of the exact errors gate.

use super::{INEXACT_ERRORS, exact_errors_gate, reviewed_errors_gate};
use crate::{steps::Failure, tests::parsed};

/// The gate on the test file `path` of audio-core holding `text`, with `reviewed`.
fn gate(path: &str, text: &str, reviewed: &[(&str, &str, &str)]) -> Result<(), Failure> {
    reviewed_errors_gate(&parsed(&[(path.to_owned(), text.to_owned())]), reviewed)
}

/// A test file of a crate.
const FILE: &str = "crates/audio-core/src/plan_tests.rs";

/// The refusal of `call` in the function `name`, at `line` of [`FILE`].
fn refused(line: usize, name: &str, call: &str) -> Result<(), Failure> {
    Err(Failure::from(format!(
        "{FILE}:{line}: `{name}` judges an error by `{call}`: compare it whole, or review the \
         function in INEXACT_ERRORS with why its text cannot be known \
         (crates/xtask/src/exact_errors.rs)"
    )))
}

#[test]
fn refuses_an_error_judged_by_a_part() {
    // The forms finding H5-6 of the fifth deep review of visual block 2 (#34) names.
    let of = |body: &str| format!("fn tells() {{\n    {body}\n}}\n");
    for (body, call) in [
        ("assert!(outcome.is_err() && written.is_empty());", "is_err"),
        (
            "assert!(run().is_err_and(|failure| failure.starts_with(\"x\")));",
            "is_err_and",
        ),
        (
            "assert!(run(1).expect_err(\"refused\").starts_with(\"x\"));",
            "expect_err(…).starts_with",
        ),
        (
            "assert!(run().unwrap_err().to_string().contains(\"x\"));",
            "unwrap_err(…).contains",
        ),
        (
            "assert!(run().expect_err(\"e\").summary.ends_with(\"x\"));",
            "expect_err(…).ends_with",
        ),
    ] {
        assert_eq!(
            gate(FILE, &of(body), &[]),
            refused(2, "tells", call),
            "{body}"
        );
        assert_eq!(
            gate("crates/audio-core/tests/a.rs", &of(body), &[]),
            refused(2, "tells", call).map_err(|failure| Failure::from(failure.summary.replacen(
                FILE,
                "crates/audio-core/tests/a.rs",
                1
            ))),
            "an integration test: {body}"
        );
    }
}

#[test]
fn refuses_an_error_bound_by_a_let_then_judged_by_a_part() {
    // Finding K5-3 of the sixth deep review of visual block 2 (#34): the chain of one
    // expression was read, never the variable that held the error.
    let taken = "let e = run().expect_err(\"x\");";
    for (body, call) in [
        (
            format!("{taken} assert!(e.starts_with(\"y\"));"),
            "e.starts_with",
        ),
        (
            format!("{taken} assert!(e.summary.ends_with(\"y\"));"),
            "e.ends_with",
        ),
        (
            format!("{taken} assert!(e.to_string().contains(\"y\"));"),
            "e.contains",
        ),
        (
            format!("{taken} assert!(e.details[0].contains(\"y\"));"),
            "e.contains",
        ),
        (
            format!("{taken} {{ if a {{ assert!(e.contains(\"y\")); }} }}"),
            "e.contains",
        ),
        (
            "let mut e: String = f(|a| { g(a); a.unwrap_err() }); e.contains(\"y\");".to_owned(),
            "e.contains",
        ),
        (
            format!("{taken} let f: fn(u8) = g; e.contains(\"y\");"),
            "e.contains",
        ),
        // A name bound to what a held one gives holds the error too, the same name included.
        (
            format!("{taken} let e = e.to_string(); let t = e; t.contains(\"y\");"),
            "t.contains",
        ),
        // Held once more after a name that held none.
        (
            format!("{taken} let e = 1; {taken} e.contains(\"y\");"),
            "e.contains",
        ),
    ] {
        assert_eq!(
            gate(FILE, &format!("fn tells() {{\n{body}\n}}\n"), &[]),
            refused(2, "tells", call),
            "{body}"
        );
    }
    assert_eq!(
        gate(
            FILE,
            &format!(
                "fn tells() {{\n{taken}\nassert!(\n  e\n    .to_string()\n.contains(\"y\"));\n}}"
            ),
            &[]
        ),
        refused(4, "tells", "e.contains"),
        "a chain over lines, at the line of the name"
    );
}

#[test]
fn names_the_function_a_call_is_in_whatever_function_pointer_it_takes() {
    // Finding L7-n4 of the same review: the token after a pointer's `fn` was the name.
    for text in [
        "fn tells(edit: fn(u8)) { run().is_err(); }",
        "fn tells() { let edit: fn(u8) = g; run().is_err(); }",
    ] {
        assert_eq!(
            gate(FILE, text, &[]),
            refused(1, "tells", "is_err"),
            "{text}"
        );
        assert_eq!(
            gate(FILE, text, &[(FILE, "tells", "git's own words")]),
            Ok(())
        );
    }
}

#[test]
fn accepts_an_error_compared_whole_and_code_that_is_no_test() {
    for text in [
        "fn a() { assert_eq!(run(), Err(\"x\".to_owned())); }",
        "fn a() { let error = run().expect_err(\"refused\"); }",
        "fn a() { let e = run().expect_err(\"x\"); assert_eq!(e.to_string(), \"x\"); }",
        "fn a() { let e = run().expect_err(\"x\"); let e = 1; e.contains(\"y\"); }",
        "fn a() { let e = run().expect_err(\"x\"); let t = f(); t.contains(\"y\"); o.e.contains(1); }",
        "fn a() { let e = run().expect_err(\"x\"); }\nfn b() { e.contains(\"y\"); }",
        "fn a() { let e = f(); let t = run().expect_err(\"x\"); e.contains(\"y\"); }",
        "fn a() { let e = f(unwrap_err); e.contains(\"y\"); let (p, q) = run().unwrap_err(); q.contains(1); }",
        "fn a() { let e = run().expect_err(\"x\"); e == f().contains(\"y\"); }",
        "fn a() { let e = run().expect_err(\"x\"); \"e\".contains(\"y\"); }",
        "fn a() { assert_eq!(run().expect_err(\"e\").to_string(), \"x\"); }",
        "fn a() { let is_err = 1; f(is_err); is_err_and(1); text.starts_with(\"x\"); }",
        "fn a() { // is_err()\n let x = \".is_err()\"; }",
        "use a::expect_err;",
        "fn a() { f(run().expect_err(\"e\"), contains(\"x\")); }",
        "fn a() { run().is_error(); x.expect_err }",
    ] {
        assert_eq!(gate(FILE, text, &[]), Ok(()), "{text}");
    }
    for path in [
        "crates/audio-core/src/plan.rs",
        "tooling/a_tests.rs",
        "crates/none/src/a_tests.rs",
    ] {
        assert_eq!(
            gate(path, "fn a() { run().is_err(); }", &[]),
            Ok(()),
            "{path}"
        );
    }
}

#[test]
fn accepts_a_reviewed_function_and_refuses_one_that_judges_no_error() {
    let text = "fn a() {}\nfn b() { x.is_err(); x.is_err(); }\nfn c() { x.is_err(); }\n";
    let review = |file, name| (file, name, "git's own words");
    assert_eq!(
        gate(FILE, text, &[review(FILE, "b"), review(FILE, "c")]),
        Ok(())
    );
    assert_eq!(
        gate(FILE, text, &[review(FILE, "b")]),
        refused(3, "c", "is_err")
    );
    let stale = |file: &str, name: &str| {
        format!(
            "{file}: `{name}`, reviewed in INEXACT_ERRORS, judges no error by a part: remove it \
             (crates/xtask/src/exact_errors.rs)"
        )
    };
    assert_eq!(
        gate(
            FILE,
            text,
            &[
                review(FILE, "a"),
                review(FILE, "b"),
                review(FILE, "c"),
                review("crates/audio-core/src/x_tests.rs", "b")
            ]
        ),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                stale(FILE, "a"),
                stale("crates/audio-core/src/x_tests.rs", "b")
            ],
            usage: false
        })
    );
    // A call of an item before any function names none.
    assert_eq!(
        gate(FILE, "const A: bool = x.is_err();", &[]),
        refused(1, "", "is_err")
    );
    // The same function of another file is another function.
    assert_eq!(
        gate(
            FILE,
            text,
            &[review("gone_tests.rs", "c"), review(FILE, "b")]
        ),
        refused(3, "c", "is_err")
    );
    // A directory read without its reviewed file; a directory not read.
    let gone = "crates/audio-core/src/gone_tests.rs";
    assert_eq!(
        gate(FILE, "", &[review(gone, "b")]),
        Err(Failure::from(stale(gone, "b")))
    );
    for other in ["crates/audio-cli/src/lib_tests.rs", "gone_tests.rs"] {
        assert_eq!(gate(FILE, "", &[review(other, "b")]), Ok(()), "{other}");
    }
    assert_eq!(exact_errors_gate(&[]), Ok(()), "nothing read");
    let (file, name, _) = INEXACT_ERRORS[0];
    assert_eq!(
        exact_errors_gate(&parsed(&[(
            "crates/audio-cli/src/a_tests.rs".to_owned(),
            String::new()
        )])),
        Err(Failure::from(stale(file, name))),
        "the reviewed functions of a directory read"
    );
    assert!(
        INEXACT_ERRORS
            .iter()
            .all(|(_, _, reason)| !reason.trim().is_empty()),
        "each with its reason"
    );
}
