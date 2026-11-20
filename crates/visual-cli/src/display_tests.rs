//! Tests of the display corpus: what it reaches, its file's form, and where a file differs.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use visual_core::{ALPHA8_THRESHOLDS, SRGB8_THRESHOLDS, pixel8};

use super::{
    END_BUCKET, EXACT_ALPHAS, FIRST_BUCKET, SPECIALS, display_corpus, display_text,
    first_difference,
};

#[test]
fn reaches_every_threshold_its_neighbors_and_every_special_value() {
    let pixels = display_corpus();
    assert_eq!(
        pixels.len(),
        255 + 3 * 255 + 555 + 1664 + 2 * 85 + 5 * 19,
        "the corpus's pixels"
    );
    let values: BTreeSet<u32> = pixels
        .iter()
        .flatten()
        .map(|value| value.to_bits())
        .collect();
    for threshold in SRGB8_THRESHOLDS {
        for value in [threshold.next_down(), threshold, threshold.next_up()] {
            assert!(
                values.contains(&value.to_bits()),
                "{value} around {threshold}"
            );
        }
    }
    let alphas: BTreeSet<u32> = pixels.iter().map(|pixel| pixel[3].to_bits()).collect();
    for threshold in ALPHA8_THRESHOLDS {
        for alpha in [threshold.next_down(), threshold, threshold.next_up()] {
            assert!(alphas.contains(&alpha.to_bits()), "alpha {alpha}");
        }
    }
    for special in SPECIALS {
        for channel in 0..4 {
            assert!(
                pixels.iter().any(|pixel| pixel
                    .get(channel)
                    .is_some_and(|value| value.to_bits() == special)),
                "{special:08x} in channel {channel}"
            );
        }
        assert!(
            pixels
                .iter()
                .any(|pixel| pixel.iter().all(|value| value.to_bits() == special)),
            "{special:08x} as all four"
        );
    }
    // Every code of the opaque encoding, and every alpha code, is reached.
    let bytes: Vec<[u8; 4]> = pixels
        .iter()
        .map(|&[red, green, blue, alpha]| pixel8(red, green, blue, alpha))
        .collect();
    let colors: BTreeSet<u8> = bytes
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let alpha_codes: BTreeSet<u8> = bytes.iter().map(|pixel| pixel[3]).collect();
    assert_eq!((colors.len(), alpha_codes.len()), (256, 256), "every code");
}

#[test]
fn reaches_every_bucket_of_the_codes_as_a_color_and_as_an_alpha() {
    // Each table entry of the codes' buckets is used: a color of an opaque pixel (its own
    // straight value) and an alpha in every bucket, from 2^-13 to 1.
    let pixels = display_corpus();
    let bucket = |value: &f32| {
        let high = value.to_bits() >> 16_u32;
        (FIRST_BUCKET..END_BUCKET).contains(&high).then_some(high)
    };
    let opaque_colors: BTreeSet<u32> = pixels
        .iter()
        .filter(|pixel| pixel[3].to_bits() == 1.0_f32.to_bits())
        .flat_map(|pixel| pixel.iter().take(3).filter_map(bucket))
        .collect();
    let alphas: BTreeSet<u32> = pixels
        .iter()
        .filter_map(|pixel| bucket(&pixel[3]))
        .collect();
    let every: BTreeSet<u32> = (FIRST_BUCKET..END_BUCKET).collect();
    assert_eq!(opaque_colors, every, "every bucket as an opaque color");
    assert_eq!(alphas, every, "every bucket as an alpha");
    assert_eq!(
        (FIRST_BUCKET << 16_u32, END_BUCKET << 16_u32),
        ((1.0_f32 / 8192.0).to_bits(), 1.0_f32.to_bits()),
        "from 2^-13 to 1, 0x680 buckets"
    );
    assert_eq!(
        END_BUCKET - FIRST_BUCKET,
        0x680,
        "the encoding's bucket count"
    );
}

#[test]
fn brackets_each_exact_product_with_colors_of_two_codes() {
    // The translucent pixels: over an alpha whose code is above 0, the colors below and above
    // the float nearest each product bracket the exact product (f64 holds the product of two
    // 32-bit floats exactly), so they straddle the code that rounds there.
    let pixels = display_corpus();
    let translucent = pixels
        .get(255..255 + 3 * 255)
        .expect("the translucent pixels");
    let mut bracketed = 0_usize;
    for (pixel, threshold) in translucent.iter().zip(
        SRGB8_THRESHOLDS
            .iter()
            .flat_map(|&threshold| [threshold; 3]),
    ) {
        let [below, _, above, alpha] = *pixel;
        let bytes = pixel8(below, 0.0, above, alpha);
        if bytes[3] == 0 {
            continue;
        }
        let exact = f64::from(threshold) * f64::from(alpha);
        assert!(
            f64::from(below) < exact && exact <= f64::from(above),
            "{threshold:e} × {alpha:e}: bracketed"
        );
        assert!(bytes[0] < bytes[2], "{threshold:e} × {alpha:e}: two codes");
        bracketed += 1;
    }
    assert_eq!(
        bracketed,
        3 * 255 - 1,
        "all but the alpha below the first threshold"
    );
}

#[test]
fn holds_colors_at_exact_ties_of_every_threshold() {
    // Over alphas that are powers of two, each threshold's product is a float exactly: the
    // color sits on the boundary itself, and encodes to the threshold's code.
    // After the opaque, translucent and bucket pixels: 85 pixels over each exact alpha.
    let pixels = display_corpus();
    let ties = pixels
        .get(255 + 3 * 255 + 555 + 1664..)
        .expect("the exact ties");
    for (alpha, section) in EXACT_ALPHAS.into_iter().zip(ties.chunks(85)) {
        let colors: Vec<f32> = section
            .iter()
            .filter(|pixel| pixel[3].to_bits() == alpha.to_bits())
            .flat_map(|pixel| pixel.iter().take(3).copied())
            .collect();
        assert_eq!(colors.len(), 255, "{alpha}: every threshold, once");
        for ((code, threshold), color) in (1..=255_u8).zip(SRGB8_THRESHOLDS).zip(colors) {
            assert_eq!(
                f64::from(color),
                f64::from(threshold) * f64::from(alpha),
                "{threshold:e} × {alpha}: exact"
            );
            assert_eq!(
                pixel8(color, color, color, alpha)[0],
                code,
                "{threshold:e} × {alpha}: its code"
            );
        }
    }
}

#[test]
fn writes_one_line_per_pixel_with_its_bits_and_its_bytes() {
    let pixels = [[1.0, 0.5, -0.0, 1.0], [0.0, 0.0, 0.0, 0.0]];
    assert_eq!(
        display_text(&pixels),
        "{\n  \"pixels\": [\n    [\"3f800000\", \"3f000000\", \"80000000\", \"3f800000\", \"ffbc00ff\"],\n    [\"00000000\", \"00000000\", \"00000000\", \"00000000\", \"00000000\"]\n  ]\n}\n"
    );
    assert_eq!(
        display_text(&[]),
        "{\n  \"pixels\": [\n  ]\n}\n",
        "no pixel"
    );
}

#[test]
fn names_where_a_file_first_differs_from_the_corpus() {
    let expected = display_text(&[[1.0, 0.5, 0.0, 1.0], [0.0; 4], [0.5; 4]]);
    assert_eq!(first_difference(&expected, &expected), None, "the same");
    let second = expected.replacen("00000000\"]", "00000001\"]", 1);
    assert_eq!(
        first_difference(&second, &expected).as_deref(),
        Some("pixel 1"),
        "a byte of the second pixel"
    );
    assert_eq!(
        first_difference(&expected.replacen('{', "[", 1), &expected).as_deref(),
        Some("line 1"),
        "the opening line"
    );
    let missing_last: Vec<&str> = expected
        .lines()
        .filter(|line| !line.contains("3f000000\", \"3f000000"))
        .collect();
    assert_eq!(
        first_difference(&(missing_last.join("\n") + "\n"), &expected).as_deref(),
        Some("pixel 2"),
        "a pixel missing: the closing line where it was"
    );
    let extra_pixel = expected.replacen(
        "\n  ]",
        "\n    [\"00000000\", \"00000000\", \"00000000\", \"00000000\", \"00000000\"]\n  ]",
        1,
    );
    assert_eq!(
        first_difference(&extra_pixel, &expected).as_deref(),
        Some("line 6"),
        "a pixel more: the closing line, where it was"
    );
    assert_eq!(
        first_difference(&(expected.clone() + "\n"), &expected).as_deref(),
        Some("line 8"),
        "a line past the end"
    );
    assert_eq!(
        first_difference("", &expected).as_deref(),
        Some("line 1"),
        "an empty file"
    );
    // Exactly as bless writes it: the same lines with other endings are another file.
    assert_eq!(
        first_difference(&expected.replace('\n', "\r\n"), &expected).as_deref(),
        Some("line 1"),
        "carriage returns"
    );
    let crlf_pixel = expected.replacen("\"],\n", "\"],\r\n", 1);
    assert_eq!(
        first_difference(&crlf_pixel, &expected).as_deref(),
        Some("pixel 0"),
        "a carriage return after a pixel"
    );
    assert_eq!(
        first_difference(expected.trim_end_matches('\n'), &expected).as_deref(),
        Some("line 7"),
        "the last line without its newline"
    );
}
