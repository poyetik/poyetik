//! Tests of the comparison with a reference: bit for bit, each tolerance at its edges and
//! around the special values, each refusal in its order, and every report on one line.

use core::cell::Cell;

use super::{Mismatch, Tolerance, Value, Values, compare, place, ulps_apart};

/// The tolerance the tests of a bound use, with its derivation.
fn two_ulps() -> Tolerance {
    Tolerance::ulps(2, "two roundings, each within one ulp")
}

/// The first difference of `actual` from `expected`, as the report reads.
fn reported(actual: Values<'_>, expected: Values<'_>, tolerance: &Tolerance) -> String {
    compare(actual, expected, tolerance, None)
        .expect_err("a difference")
        .to_string()
}

#[test]
fn equal_values_of_each_kind_match_bit_for_bit() {
    let exact = Tolerance::Exact;
    for (actual, expected) in [
        (
            Values::U32(&[0, 7, u32::MAX]),
            Values::U32(&[0, 7, u32::MAX]),
        ),
        (
            Values::I32(&[-1_i32, 0_i32, i32::MIN]),
            Values::I32(&[-1_i32, 0_i32, i32::MIN]),
        ),
        (
            Values::F32(&[-0.0, f32::from_bits(0x7fc0_1234), f32::INFINITY]),
            Values::F32(&[-0.0, f32::from_bits(0x7fc0_1234), f32::INFINITY]),
        ),
        (Values::Bytes(&[0, 255]), Values::Bytes(&[0, 255])),
        (Values::F32(&[]), Values::F32(&[])),
    ] {
        assert_eq!(
            compare(actual, expected, &exact, None),
            Ok(()),
            "{actual:?}"
        );
    }
}

#[test]
fn bit_for_bit_tells_the_bits_apart() {
    let exact = Tolerance::Exact;
    assert_eq!(
        reported(Values::F32(&[1.0, -0.0]), Values::F32(&[1.0, 0.0]), &exact),
        "first difference at index 1: expected 0.0 (0x00000000), actual -0.0 (0x80000000), bit \
         for bit expected",
        "the two zeros"
    );
    assert_eq!(
        reported(
            Values::F32(&[f32::from_bits(0x7fc0_0001)]),
            Values::F32(&[f32::from_bits(0x7fc0_0002)]),
            &exact
        ),
        "first difference at index 0: expected NaN (0x7fc00002), actual NaN (0x7fc00001), bit \
         for bit expected",
        "two payloads"
    );
    assert_eq!(
        reported(Values::U32(&[5, 6, 7]), Values::U32(&[5, 9, 8]), &exact),
        "first difference at index 1: expected 9 (0x00000009), actual 6 (0x00000006), bit for \
         bit expected",
        "the first of two differences"
    );
    assert_eq!(
        reported(Values::I32(&[-3_i32]), Values::I32(&[3_i32]), &exact),
        "first difference at index 0: expected 3, actual -3, bit for bit expected",
        "an i32"
    );
    assert_eq!(
        reported(Values::Bytes(&[1, 2]), Values::Bytes(&[1, 0xab]), &exact),
        "first difference at index 1: expected 0xab, actual 0x02, bit for bit expected",
        "a byte"
    );
}

#[test]
fn refuses_values_of_another_kind_first() {
    let all = [
        Values::U32(&[1]),
        Values::I32(&[1_i32]),
        Values::F32(&[1.0]),
        Values::Bytes(&[1]),
    ];
    let names = ["u32", "i32", "f32", "bytes"];
    for (actual, actual_name) in all.iter().zip(names) {
        for (expected, expected_name) in all.iter().zip(names) {
            let found = compare(*actual, *expected, &Tolerance::Exact, None);
            if actual_name == expected_name {
                assert_eq!(found, Ok(()), "{actual_name}");
            } else {
                assert_eq!(
                    found,
                    Err(Mismatch::Kinds {
                        actual: actual_name,
                        expected: expected_name
                    }),
                    "{actual_name} against {expected_name}"
                );
            }
        }
    }
    assert_eq!(
        compare(Values::U32(&[]), Values::F32(&[1.0]), &two_ulps(), None)
            .map_err(|mismatch| mismatch.to_string()),
        Err("different kinds: u32 values compared with a reference of f32".to_owned()),
        "before the tolerance and the lengths"
    );
}

#[test]
fn refuses_a_tolerance_that_cannot_apply_before_the_lengths() {
    for (values, kind) in [
        (Values::U32(&[1]), "u32"),
        (Values::I32(&[1_i32]), "i32"),
        (Values::Bytes(&[1]), "bytes"),
    ] {
        for tolerance in [two_ulps(), Tolerance::absolute(1.0, "a bound")] {
            assert_eq!(
                compare(values, values, &tolerance, None),
                Err(Mismatch::BadTolerance(format!(
                    "a tolerance on {kind}: only f32 values take one, the others are compared \
                     bit for bit"
                ))),
                "{kind} under {tolerance:?}"
            );
        }
    }
    for derivation in ["", " \t\n"] {
        for tolerance in [
            Tolerance::ulps(1, derivation),
            Tolerance::absolute(1.0, derivation),
        ] {
            assert_eq!(
                compare(Values::F32(&[1.0]), Values::F32(&[]), &tolerance, None),
                Err(Mismatch::BadTolerance(
                    "a tolerance with no derivation: say why the bound holds".to_owned()
                )),
                "{derivation:?}"
            );
        }
    }
    for (max, shown) in [
        (f32::NAN, "NaN"),
        (f32::INFINITY, "inf"),
        (f32::NEG_INFINITY, "-inf"),
        (-1.0e-30, "-1e-30"),
    ] {
        assert_eq!(
            compare(
                Values::F32(&[1.0]),
                Values::F32(&[]),
                &Tolerance::absolute(max, "a bound"),
                None
            )
            .map_err(|mismatch| mismatch.to_string()),
            Err(format!(
                "bad tolerance: an absolute tolerance of {shown}: give a finite bound, not \
                 negative"
            )),
            "{shown}"
        );
    }
    for max in [0.0, -0.0, f32::MAX] {
        assert_eq!(
            compare(
                Values::F32(&[1.0]),
                Values::F32(&[1.0]),
                &Tolerance::absolute(max, "a bound"),
                None
            ),
            Ok(()),
            "a bound of {max:?}"
        );
    }
}

#[test]
fn refuses_different_lengths_before_any_value() {
    for (actual, expected) in [(2, 3), (3, 2), (0, 1)] {
        let values = [1.0_f32, 2.0, 3.0];
        let found = compare(
            Values::F32(values.get(..actual).expect("in range")),
            Values::F32([9.0, 9.0, 9.0].get(..expected).expect("in range")),
            &Tolerance::Exact,
            None,
        );
        assert_eq!(
            found.as_ref().map_err(ToString::to_string),
            Err(format!(
                "different lengths: {actual} values compared with a reference of {expected}"
            )),
            "{actual} against {expected}"
        );
    }
}

#[test]
fn a_bound_in_ulps_holds_to_its_edge() {
    let tolerance = two_ulps();
    let one = 1.0_f32.to_bits();
    for apart in [0, 1, 2] {
        assert_eq!(
            compare(
                Values::F32(&[f32::from_bits(one.wrapping_add(apart))]),
                Values::F32(&[1.0]),
                &tolerance,
                None
            ),
            Ok(()),
            "{apart} ulps above"
        );
        assert_eq!(
            compare(
                Values::F32(&[f32::from_bits(one.wrapping_sub(apart))]),
                Values::F32(&[1.0]),
                &tolerance,
                None
            ),
            Ok(()),
            "{apart} ulps below"
        );
    }
    assert_eq!(
        reported(
            Values::F32(&[f32::from_bits(one.wrapping_add(3))]),
            Values::F32(&[1.0]),
            &tolerance
        ),
        "first difference at index 0: expected 1.0 (0x3f800000), actual 1.0000004 (0x3f800003), \
         3 ulps apart, at most 2 allowed: two roundings, each within one ulp",
        "one ulp past the bound"
    );
    assert_eq!(
        reported(
            Values::F32(&[f32::from_bits(one.wrapping_sub(3))]),
            Values::F32(&[1.0]),
            &tolerance
        ),
        "first difference at index 0: expected 1.0 (0x3f800000), actual 0.9999998 \
         (0x3f7ffffd), 3 ulps apart, at most 2 allowed: two roundings, each within one ulp",
        "one ulp past the bound, below"
    );
}

#[test]
fn places_each_float_in_the_order_of_the_number_line() {
    let smallest = f32::from_bits(1);
    assert_eq!(
        [
            -f32::MAX,
            -1.0,
            -smallest,
            -0.0,
            0.0,
            smallest,
            1.0,
            f32::MAX
        ]
        .map(place),
        [
            -0x7f7f_ffff_i64,
            -0x3f80_0000_i64,
            -1_i64,
            0_i64,
            0_i64,
            1_i64,
            0x3f80_0000_i64,
            0x7f7f_ffff_i64
        ],
        "negative below zero, positive above, each float one step from the next"
    );
}

#[test]
fn ulps_count_across_zero_and_to_the_largest_floats() {
    let smallest = f32::from_bits(1);
    assert_eq!(ulps_apart(0.0, -0.0), 0, "the two zeros are one");
    assert_eq!(ulps_apart(smallest, -smallest), 2, "across zero");
    assert_eq!(ulps_apart(-smallest, smallest), 2, "either way");
    assert_eq!(ulps_apart(smallest, 0.0), 1, "the smallest subnormal");
    assert_eq!(
        ulps_apart(f32::MAX, -f32::MAX),
        4_278_190_078,
        "twice 0x7f7fffff, beyond i32 and without overflow"
    );
    assert_eq!(
        ulps_apart(f32::MIN_POSITIVE, 0.0),
        0x0080_0000,
        "the subnormals"
    );
    assert_eq!(
        compare(
            Values::F32(&[-0.0, smallest]),
            Values::F32(&[0.0, -smallest]),
            &two_ulps(),
            None
        ),
        Ok(()),
        "within two ulps across zero"
    );
    assert_eq!(
        compare(
            Values::F32(&[f32::MAX]),
            Values::F32(&[-f32::MAX]),
            &Tolerance::ulps(4_278_190_078, "the widest finite span"),
            None
        ),
        Ok(()),
        "the largest floats of both signs, at the bound"
    );
    assert_eq!(
        compare(
            Values::F32(&[f32::MAX]),
            Values::F32(&[-f32::MAX]),
            &Tolerance::ulps(4_278_190_077, "one short"),
            None
        )
        .map_err(|mismatch| mismatch.to_string()),
        Err(
            "first difference at index 0: expected -3.4028235e38 (0xff7fffff), actual \
             3.4028235e38 (0x7f7fffff), 4278190078 ulps apart, at most 4278190077 allowed: \
             one short"
                .to_owned()
        ),
        "one ulp past the bound"
    );
}

#[test]
fn a_tolerance_matches_a_not_a_number_with_any_other_and_an_infinity_with_itself() {
    let nan = f32::from_bits(0x7fc0_0001);
    let other_nan = f32::from_bits(0xffc0_0002);
    for tolerance in [
        Tolerance::ulps(u32::MAX, "every float"),
        Tolerance::absolute(f32::MAX, "every float"),
    ] {
        assert_eq!(
            compare(
                Values::F32(&[nan, f32::INFINITY, f32::NEG_INFINITY]),
                Values::F32(&[other_nan, f32::INFINITY, f32::NEG_INFINITY]),
                &tolerance,
                None
            ),
            Ok(()),
            "any payload, and each infinity itself, under {tolerance:?}"
        );
        for (actual, expected) in [
            (nan, 1.0),
            (1.0, nan),
            (nan, f32::INFINITY),
            (f32::INFINITY, f32::MAX),
            (f32::MAX, f32::INFINITY),
            (f32::NEG_INFINITY, f32::INFINITY),
            (f32::NEG_INFINITY, -f32::MAX),
        ] {
            assert!(
                compare(
                    Values::F32(&[actual]),
                    Values::F32(&[expected]),
                    &tolerance,
                    None
                )
                .is_err(),
                "{actual:?} against {expected:?} under {tolerance:?}"
            );
        }
    }
}

#[test]
fn an_absolute_bound_holds_to_its_edge() {
    let tolerance = Tolerance::absolute(0.25, "a quarter, the step of the grid");
    assert_eq!(
        compare(
            Values::F32(&[1.25, 0.75, 1.0, -0.125]),
            Values::F32(&[1.0, 1.0, 1.0, 0.125]),
            &tolerance,
            None
        ),
        Ok(()),
        "at the bound either way, and within it across zero"
    );
    assert_eq!(
        reported(Values::F32(&[1.5]), Values::F32(&[1.0]), &tolerance),
        "first difference at index 0: expected 1.0 (0x3f800000), actual 1.5 (0x3fc00000), 0.5 \
         apart, at most 0.25 allowed: a quarter, the step of the grid",
        "past the bound"
    );
    assert_eq!(
        reported(
            Values::F32(&[f32::from_bits(1.25_f32.to_bits().wrapping_add(1))]),
            Values::F32(&[1.0]),
            &tolerance
        ),
        "first difference at index 0: expected 1.0 (0x3f800000), actual 1.2500001 (0x3fa00001), \
         0.25000011920928955 apart, at most 0.25 allowed: a quarter, the step of the grid",
        "one ulp past the bound"
    );
    assert_eq!(
        reported(
            Values::F32(&[f32::MAX]),
            Values::F32(&[-f32::MAX]),
            &Tolerance::absolute(f32::MAX, "the largest")
        ),
        "first difference at index 0: expected -3.4028235e38 (0xff7fffff), actual 3.4028235e38 \
         (0x7f7fffff), 6.805646932770577e38 apart, at most 3.4028235e38 allowed: the largest",
        "a distance beyond f32, in f64"
    );
    assert_eq!(
        compare(
            Values::F32(&[-0.0]),
            Values::F32(&[0.0]),
            &Tolerance::absolute(0.0, "none"),
            None
        ),
        Ok(()),
        "the two zeros are one number"
    );
}

#[test]
fn describes_the_inputs_of_the_first_difference_only() {
    let calls = Cell::new(0_u32);
    let describe = |index: usize| {
        calls.set(calls.get().wrapping_add(1));
        format!("x = {index}\nforged: line")
    };
    assert_eq!(
        compare(
            Values::U32(&[1, 2, 3, 4]),
            Values::U32(&[1, 0, 3, 0]),
            &Tolerance::Exact,
            Some(&describe)
        )
        .map_err(|mismatch| mismatch.to_string()),
        Err(
            "first difference at index 1: expected 0 (0x00000000), actual 2 (0x00000002), bit \
             for bit expected; inputs: x = 1\\u{a}forged: line"
                .to_owned()
        ),
        "the inputs at its index, on one line"
    );
    assert_eq!(calls.get(), 1, "described once");
    assert_eq!(
        compare(
            Values::U32(&[1]),
            Values::U32(&[1]),
            &Tolerance::Exact,
            Some(&describe)
        ),
        Ok(()),
        "a match"
    );
    assert_eq!(calls.get(), 1, "nothing described when nothing differs");
}

#[test]
fn says_each_mismatch_on_one_safe_line() {
    let lines = [
        Mismatch::NotWritten {
            group: 1,
            binding: 2,
        }
        .to_string(),
        Mismatch::BadTolerance("why\nnot".to_owned()).to_string(),
        reported(
            Values::F32(&[2.0]),
            Values::F32(&[1.0]),
            &Tolerance::ulps(0, "none\u{202e}"),
        ),
    ];
    assert_eq!(
        lines,
        [
            "not written: group 1 binding 2 is no read_write buffer of the kernel",
            "bad tolerance: why\\u{a}not",
            "first difference at index 0: expected 1.0 (0x3f800000), actual 2.0 (0x40000000), \
             8388608 ulps apart, at most 0 allowed: none\\u{202e}",
        ],
        "each line"
    );
    assert_eq!(
        [
            Value::U32(10),
            Value::I32(-10),
            Value::F32(-1.5),
            Value::Byte(10),
        ]
        .map(|value| value.to_string()),
        ["10 (0x0000000a)", "-10", "-1.5 (0xbfc00000)", "0x0a"],
        "each value"
    );
}
