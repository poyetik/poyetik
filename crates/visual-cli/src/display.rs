//! The display corpus: values chosen to reach every path of the display encoding, and the
//! bytes the native encoding gives them, which `bless` writes to `display.json` beside the
//! references and `check` compares, so that a target encoding them otherwise (WebAssembly,
//! `bindings/visual-wasm`'s conformance tests) is caught.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use visual_core::{
    ALPHA8_THRESHOLDS, DisplayImage, Image, ImageSize, SRGB8_THRESHOLDS, VisualError,
    display_bucket_floors,
};

/// The file of the display corpus in a conformance directory.
pub const DISPLAY_FILE: &str = "display.json";

/// Special values, as bits: both zeros, the smallest and largest subnormals and the
/// smallest normal (each of both signs), a half, 1 and the float above it, 2, the largest
/// float, both infinities, a quiet and a signaling NaN, a NaN of negative sign, and -1.
const SPECIALS: [u32; 19] = [
    0x0000_0000,
    0x8000_0000,
    0x0000_0001,
    0x8000_0001,
    0x007f_ffff,
    0x807f_ffff,
    0x0080_0000,
    0x8080_0000,
    0x3f00_0000,
    0x3f80_0000,
    0x3f80_0001,
    0x4000_0000,
    0x7f7f_ffff,
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_0000,
    0x7f80_0001,
    0xffc0_0000,
    0xbf80_0000,
];

/// The alphas whose product with each sRGB threshold is a float exactly (powers of two,
/// the products normal: the thresholds are at least 2^-13): a color at that product is a
/// tie between a code and the one below, on every target.
const EXACT_ALPHAS: [f32; 2] = [0.25, 0.003_906_25];

/// `value` and the floats just below and just above it.
const fn around(value: f32) -> [f32; 3] {
    [value.next_down(), value, value.next_up()]
}

/// The pixels of the corpus, four values each (red, green, blue, alpha), in its order.
///
/// - Opaque: every sRGB threshold, the float below it and the float above it, one in each
///   color channel.
/// - Translucent: every alpha threshold and its two neighbors as the alpha, each with the
///   float nearest the product of that alpha and an sRGB threshold (a code that rounds
///   there) and its two neighbors, which bracket the exact product, the sRGB threshold taken
///   in turn.
/// - Buckets: a value in each bucket of the codes (`visual_core::display_bucket_floors`), three
///   buckets to an opaque pixel as its colors; then each as the alpha, over colors at a
///   quarter, a half and all of it.
/// - Exact ties: over alphas 0.25 and 2⁻⁸, powers of two, each sRGB threshold's exact product
///   as a color, three thresholds to a pixel.
/// - Special: each special value in each channel of a translucent pixel, and as all four;
///   last, so that the corpus ends on pixels encoded to 0.
#[must_use]
pub fn display_corpus() -> Vec<[f32; 4]> {
    let mut pixels = Vec::new();
    for threshold in SRGB8_THRESHOLDS {
        let [below, at, above] = around(threshold);
        pixels.push([below, at, above, 1.0]);
    }
    for (alpha_threshold, color_threshold) in ALPHA8_THRESHOLDS.iter().zip(SRGB8_THRESHOLDS) {
        for alpha in around(*alpha_threshold) {
            let [below, at, above] = around(alpha * color_threshold);
            pixels.push([below, at, above, alpha]);
        }
    }
    let buckets: Vec<f32> = display_bucket_floors().collect();
    for colors in buckets.chunks(3) {
        // The last chunk is shorter: its missing colors are the first buckets again.
        let [red, green, blue] = [0, 1, 2].map(|at| {
            colors
                .get(at)
                .or_else(|| buckets.get(at))
                .copied()
                .unwrap_or_default()
        });
        pixels.push([red, green, blue, 1.0]);
    }
    for &alpha in &buckets {
        pixels.push([alpha * 0.25, alpha * 0.5, alpha, alpha]);
    }
    for alpha in EXACT_ALPHAS {
        for thresholds in SRGB8_THRESHOLDS.chunks(3) {
            let [red, green, blue] = [0, 1, 2].map(|at| {
                thresholds
                    .get(at)
                    .map_or(0.0, |&threshold| threshold * alpha)
            });
            pixels.push([red, green, blue, alpha]);
        }
    }
    for special in SPECIALS.map(f32::from_bits) {
        pixels.push([special, 0.25, 0.125, 0.5]);
        pixels.push([0.25, special, 0.125, 0.5]);
        pixels.push([0.25, 0.125, special, 0.5]);
        pixels.push([0.25, 0.125, 0.0625, special]);
        pixels.push([special; 4]);
    }
    pixels
}

/// The display corpus file of `pixels`, exactly as `visual-cli bless` writes it.
///
/// A JSON object whose `pixels` hold one line per pixel, its four values' bits then its four
/// bytes encoded for display, each as lowercase hexadecimal. The bytes are those of the
/// pixels as an image encoded by [`DisplayImage::encode`] ([`display_bytes`]): the encoding
/// of a rendered image as the hosts run it, its loop included, not only that of one pixel.
///
/// # Errors
///
/// Returns why the image or its display encoding cannot be held ([`display_bytes`]).
pub(crate) fn display_text(pixels: &[[f32; 4]]) -> Result<String, VisualError> {
    display_text_with(pixels, Image::try_new, DisplayImage::try_new)
}

/// [`display_text`], its image made by `image` ([`Image::try_new`]) and its display image by
/// `display` ([`DisplayImage::try_new`]): the seams through which the tests refuse either
/// allocation, which no test machine refuses otherwise.
///
/// # Errors
///
/// As [`display_text`].
pub(crate) fn display_text_with(
    pixels: &[[f32; 4]],
    image: fn(ImageSize) -> Result<Image, VisualError>,
    display: fn(ImageSize) -> Result<DisplayImage, VisualError>,
) -> Result<String, VisualError> {
    let encoded = display_bytes(pixels, image, display)?;
    let mut text = String::from("{\n  \"pixels\": [\n");
    let last = pixels.len().saturating_sub(1);
    for (at, (&[red, green, blue, alpha], &pixel)) in pixels.iter().zip(&encoded).enumerate() {
        let bytes = u32::from_be_bytes(pixel);
        let comma = if at == last { "" } else { "," };
        // Writing to a string cannot fail.
        let _: core::fmt::Result = writeln!(
            text,
            "    [\"{:08x}\", \"{:08x}\", \"{:08x}\", \"{:08x}\", \"{bytes:08x}\"]{comma}",
            red.to_bits(),
            green.to_bits(),
            blue.to_bits(),
            alpha.to_bits(),
        );
    }
    text.push_str("  ]\n}\n");
    Ok(text)
}

/// The display bytes of `pixels`, one group of four a pixel, in order: the pixels laid out as
/// an image, row by row, at most [`ImageSize::MAX_SIDE`] a row (the rest of the last row
/// transparent black), made by `image`, then encoded into a display image made by `display`
/// ([`DisplayImage::encode`]). None for no pixel.
///
/// # Errors
///
/// Returns why the image or the display image cannot be made: more pixels than an image
/// holds ([`ImageSize::new`]), or memory refused.
fn display_bytes(
    pixels: &[[f32; 4]],
    image: fn(ImageSize) -> Result<Image, VisualError>,
    display: fn(ImageSize) -> Result<DisplayImage, VisualError>,
) -> Result<Vec<[u8; 4]>, VisualError> {
    let widest = usize::try_from(ImageSize::MAX_SIDE).unwrap_or(usize::MAX);
    let columns = pixels.len().min(widest);
    if columns == 0 {
        return Ok(Vec::new());
    }
    let rows = pixels.len().div_ceil(columns);
    let size = ImageSize::new(
        u32::try_from(columns).unwrap_or(u32::MAX),
        u32::try_from(rows).unwrap_or(u32::MAX),
    )?;
    let mut values = image(size)?;
    for (row, held) in values.rows_mut().zip(pixels.chunks(columns)) {
        for (value, &pixel) in row
            .as_chunks_mut::<{ Image::CHANNELS }>()
            .0
            .iter_mut()
            .zip(held)
        {
            *value = pixel;
        }
    }
    let mut encoded = display(size)?;
    encoded.encode(&values)?;
    Ok(encoded
        .bytes()
        .as_chunks::<{ DisplayImage::CHANNELS }>()
        .0
        .iter()
        .take(pixels.len())
        .copied()
        .collect())
}

/// Where `held` first differs from the display corpus file `expected`, byte for byte.
///
/// The pixel, from 0, whose line differs, or the line past the pixels; `None` when they are
/// the same. Equality decides: a file with other line endings, or without its last newline,
/// is another file; the lines, each with its ending, only name where they differ.
#[must_use]
pub(crate) fn first_difference(held: &str, expected: &str) -> Option<String> {
    if held == expected {
        return None;
    }
    let mut held_lines = held.split_inclusive('\n');
    let differing = expected
        .split_inclusive('\n')
        .enumerate()
        .find(|&(_, line)| held_lines.next() != Some(line));
    Some(match differing {
        // The two lines before the pixels: the opening brace and the list's key.
        Some((at, line)) => match at.checked_sub(2) {
            Some(pixel) if line.starts_with("    [") => format!("pixel {pixel}"),
            _ => format!("line {}", at.saturating_add(1)),
        },
        // Every line of `expected` held, and more.
        None => format!(
            "line {}",
            expected.split_inclusive('\n').count().saturating_add(1)
        ),
    })
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
