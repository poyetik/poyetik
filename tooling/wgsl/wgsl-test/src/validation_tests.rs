//! Tests of the validation: a valid shader accepted, and each stage's refusal reported on one
//! line at the position of its problem.

use super::{Invalid, Location, Stage, addressable, narrowest, one_line, validate};

/// A compute shader valid under naga's default capabilities.
const VALID: &str = "@group(0) @binding(0) var<storage, read_write> data: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&data) {
        data[id.x] = data[id.x] * 2u + 1u;
    }
}
";

/// The refusal of `source`, which must be refused.
fn refusal(source: &str) -> Invalid {
    validate(source).expect_err("a refused source")
}

#[test]
fn accepts_a_valid_compute_shader() {
    assert_eq!(validate(VALID), Ok(()), "a valid shader");
    assert_eq!(validate(""), Ok(()), "an empty module is valid WGSL");
}

#[test]
fn refuses_a_parse_error_at_its_position() {
    // Line 2, column 10: the `+` with no operand on its right.
    let invalid = refusal("fn f() -> u32 {\n  return + ;\n}\n");
    assert_eq!(invalid.stage(), Stage::Parse, "{invalid}");
    assert_eq!(
        invalid.location(),
        Some(Location {
            line: 2,
            column: 10
        }),
        "{invalid}"
    );
    assert!(
        invalid.to_string().starts_with("2:10: parse error: "),
        "{invalid}"
    );
}

#[test]
fn refuses_a_module_breaking_a_rule_at_its_position() {
    // Parses, but returns a u32 from a function declared to return an f32.
    let source = "fn f() -> f32 {\n    return 1u;\n}\n";
    let invalid = refusal(source);
    assert_eq!(invalid.stage(), Stage::Validation, "{invalid}");
    let location = invalid.location().expect("a position");
    assert_eq!(location.line(), 2, "{invalid}");
    assert_eq!(
        location.column(),
        12,
        "the narrowest span, the expression: {invalid}"
    );
    assert!(
        invalid.to_string().starts_with("2:12: invalid: "),
        "{invalid}"
    );
}

#[test]
fn points_at_the_expression_at_fault_on_a_later_line() {
    // Line 3, column 16: the abstract float a u32 is added to.
    let invalid = refusal("fn f() -> u32 {\n    let x = 1u;\n    return x + 1.0;\n}\n");
    assert_eq!(invalid.stage(), Stage::Validation, "{invalid}");
    assert_eq!(
        invalid.location(),
        Some(Location {
            line: 3,
            column: 16
        }),
        "{invalid}"
    );
}

#[test]
fn refuses_an_optional_capability_of_a_device() {
    // 64-bit floats are an optional capability, outside naga's default ones.
    let invalid = refusal("fn f() -> f64 { return 1.0lf; }\n");
    assert_eq!(invalid.stage(), Stage::Validation, "{invalid}");
    assert!(invalid.message().contains("Capabilities"), "{invalid}");
}

#[test]
fn chains_every_cause_of_naga_on_one_line() {
    let invalid = refusal("fn f() -> f32 {\n    return 1u;\n}\n");
    let message = invalid.message();
    assert!(message.contains(": "), "causes joined: {message}");
    assert!(!message.contains('\n'), "one line: {message}");
}

/// An error whose text and cause are given.
#[derive(Debug)]
struct Chained(&'static str, Option<Box<Self>>);

impl core::fmt::Display for Chained {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl core::error::Error for Chained {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        let cause: &(dyn core::error::Error + 'static) = self.1.as_deref()?;
        Some(cause)
    }
}

#[test]
fn joins_causes_in_order_and_escapes_control_characters() {
    let error = Chained(
        "outer",
        Some(Box::new(Chained(
            "middle\nforged: line",
            Some(Box::new(Chained("inner\u{1b}[31m\u{202e}", None))),
        ))),
    );
    assert_eq!(
        one_line(&error),
        "outer: middle\\u{a}forged: line: inner\\u{1b}[31m\\u{202e}",
        "every cause, in order, made safe as the project's rule says (crate::safe_message)"
    );
}

#[test]
fn addresses_sources_up_to_naga_s_32_bit_positions() {
    let largest = usize::try_from(u32::MAX).expect("a 64-bit or 32-bit host");
    assert_eq!(addressable(0), Ok(()), "an empty source");
    assert_eq!(addressable(largest), Ok(()), "the largest 32-bit offset");
    if let Some(past) = largest.checked_add(1) {
        let invalid = addressable(past).expect_err("one byte past it");
        assert_eq!(
            invalid.to_string(),
            "too large: 4294967296 bytes, more than naga's 32-bit positions address",
            "refused before naga reads it, with no position"
        );
        assert_eq!(invalid.stage(), Stage::Size, "{invalid}");
        assert_eq!(invalid.location(), None, "{invalid}");
    }
}

#[test]
fn names_the_file_before_the_position_or_the_stage() {
    let positioned = refusal("fn f() -> f32 {\n    return 1u;\n}\n");
    assert!(
        positioned
            .of_file("a.wgsl")
            .starts_with("a.wgsl:2:12: invalid: "),
        "{positioned}"
    );
    let largest = usize::try_from(u32::MAX).expect("a 64-bit or 32-bit host");
    if let Some(past) = largest.checked_add(1) {
        let unplaced = addressable(past).expect_err("too large");
        assert_eq!(
            unplaced.of_file("a.wgsl"),
            "a.wgsl: too large: 4294967296 bytes, more than naga's 32-bit positions address",
            "a space after the name when there is no position"
        );
    }
}

#[test]
fn places_a_validation_error_at_its_narrowest_span_in_any_order() {
    let span = |start: u32, end: u32| naga::Span::new(start, end);
    for (spans, expected, why) in [
        (vec![], None, "no span"),
        (
            vec![naga::Span::UNDEFINED],
            None,
            "only a span that says nothing",
        ),
        (
            vec![span(0, 40), span(10, 14)],
            Some(span(10, 14)),
            "widest first",
        ),
        (
            vec![span(10, 14), span(0, 40)],
            Some(span(10, 14)),
            "narrowest first",
        ),
        (
            vec![span(0, 40), naga::Span::UNDEFINED],
            Some(span(0, 40)),
            "a span that says nothing skipped",
        ),
        (
            vec![span(0, 4), span(8, 12)],
            Some(span(8, 12)),
            "the last of a tie",
        ),
    ] {
        assert_eq!(narrowest(spans.into_iter()), expected, "{why}");
    }
}
