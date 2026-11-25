//! Tests of the display corpus: what it reaches, its file's form, and where a file differs.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use visual_core::{
    ALPHA8_THRESHOLDS, DisplayImage, Holding, Image, ImageSize, SRGB8_THRESHOLDS, VisualError,
    display_bucket_floors, pixel8,
};

use super::{
    EXACT_ALPHAS, SPECIALS, display_corpus, display_text, display_text_with, first_difference,
};

#[test]
fn reaches_every_threshold_its_neighbors_and_every_special_value() {
    let pixels = display_corpus();
    assert_eq!(
        pixels.len(),
        255 + 3 * 255 + 555 + display_bucket_floors().count() + 2 * 85 + 5 * 19,
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
    // straight value) and an alpha at the smallest float of every bucket, as visual-core
    // lays them out (`display_bucket_floors`), whatever their width.
    let pixels = display_corpus();
    let every: BTreeSet<u32> = display_bucket_floors().map(f32::to_bits).collect();
    let opaque_colors: BTreeSet<u32> = pixels
        .iter()
        .filter(|pixel| pixel[3].to_bits() == 1.0_f32.to_bits())
        .flat_map(|pixel| pixel.iter().take(3).map(|value| value.to_bits()))
        .filter(|bits| every.contains(bits))
        .collect();
    let alphas: BTreeSet<u32> = pixels
        .iter()
        .map(|pixel| pixel[3].to_bits())
        .filter(|bits| every.contains(bits))
        .collect();
    assert_eq!(opaque_colors, every, "every bucket as an opaque color");
    assert_eq!(alphas, every, "every bucket as an alpha");
    // Not empty, so that neither equality above holds for want of buckets; the count itself
    // is visual-core's to pin (`BUCKETS`).
    assert!(!every.is_empty(), "some bucket to reach");
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
        .get(255 + 3 * 255 + 555 + display_bucket_floors().count()..)
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
        display_text(&pixels).as_deref(),
        Ok(
            "{\n  \"pixels\": [\n    [\"3f800000\", \"3f000000\", \"80000000\", \"3f800000\", \"ffbc00ff\"],\n    [\"00000000\", \"00000000\", \"00000000\", \"00000000\", \"00000000\"]\n  ]\n}\n"
        )
    );
    assert_eq!(
        display_text(&[]).as_deref(),
        Ok("{\n  \"pixels\": [\n  ]\n}\n"),
        "no pixel"
    );
}

#[test]
fn names_where_a_file_first_differs_from_the_corpus() {
    let expected = display_text(&[[1.0, 0.5, 0.0, 1.0], [0.0; 4], [0.5; 4]]).expect("encoded");
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

/// An allocation refused, naming what it was for.
const fn out_of_memory(size: ImageSize, of: Holding) -> VisualError {
    VisualError::OutOfMemory {
        width: size.width(),
        height: size.height(),
        of,
    }
}

/// An image the memory cannot hold.
const fn no_image(size: ImageSize) -> Result<Image, VisualError> {
    Err(out_of_memory(size, Holding::Image))
}

/// A display image the memory cannot hold.
const fn no_display(size: ImageSize) -> Result<DisplayImage, VisualError> {
    Err(out_of_memory(size, Holding::DisplayEncoding))
}

#[test]
fn refuses_a_corpus_whose_image_or_display_image_the_memory_cannot_hold() {
    let pixels = [[0.5; 4]; 3];
    let size = ImageSize::new(3, 1).expect("a size");
    assert_eq!(
        display_text_with(&pixels, no_image, DisplayImage::try_new),
        Err(out_of_memory(size, Holding::Image)),
        "the image, one row of the three pixels"
    );
    assert_eq!(
        display_text_with(&pixels, Image::try_new, no_display),
        Err(out_of_memory(size, Holding::DisplayEncoding)),
        "its display encoding"
    );
}

#[test]
fn refuses_a_display_image_of_another_size_than_the_corpus() {
    // A display image of one pixel for a corpus of three: the encoding refuses it.
    let one = |_size| DisplayImage::try_new(ImageSize::new(1, 1).expect("a size"));
    assert_eq!(
        display_text_with(&[[0.5; 4]; 3], Image::try_new, one),
        Err(VisualError::DisplaySizeMismatch {
            width: 3,
            height: 1
        }),
        "the image's size named"
    );
}

#[test]
fn refuses_a_corpus_of_more_pixels_than_an_image_holds() {
    // One pixel past the most an image holds, in rows of the widest side. The zeros are
    // allocated zeroed, untouched: refused from their count before any is read.
    let count = usize::try_from(ImageSize::MAX_PIXELS).expect("a count") + 1;
    let pixels = vec![[0.0_f32; 4]; count];
    assert_eq!(
        display_text_with(&pixels, Image::try_new, DisplayImage::try_new),
        Err(VisualError::ImageTooLarge {
            width: ImageSize::MAX_SIDE,
            height: 2049
        }),
        "2^25 + 1 pixels: 2048 full rows and one more"
    );
}

#[test]
fn encodes_a_corpus_longer_than_a_row_row_by_row() {
    // One pixel more than a row holds: two rows, the second holding one pixel, the rest of
    // it transparent black, which no line shows.
    let count = usize::try_from(ImageSize::MAX_SIDE).expect("a count") + 1;
    let pixels: Vec<[f32; 4]> = (0..count)
        .map(|at| {
            let value = f32::from(u16::try_from(at % 256).expect("a byte")) / 255.0;
            [value, value * 0.5, 0.0, if at % 2 == 0 { 1.0 } else { 0.5 }]
        })
        .collect();
    let text = display_text(&pixels).expect("encoded");
    let bytes: Vec<String> = text
        .lines()
        .filter_map(|line| {
            line.rsplit_once(", \"")
                .map(|(_, bytes)| bytes.get(..8).unwrap_or_default().to_owned())
        })
        .collect();
    let expected: Vec<String> = pixels
        .iter()
        .map(|&[red, green, blue, alpha]| {
            format!(
                "{:08x}",
                u32::from_be_bytes(pixel8(red, green, blue, alpha))
            )
        })
        .collect();
    assert_eq!(bytes.len(), count, "one line per pixel, no padding");
    assert_eq!(bytes, expected, "each pixel's bytes, in order");
}
