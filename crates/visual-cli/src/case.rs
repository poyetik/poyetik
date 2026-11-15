//! Visual conformance cases: what to render, read from JSON, and their render to a hash.

use serde::Deserialize;
use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRange, ImageRate, ImageSize, Seconds};
use visual_hash::{ImageHash, hash_sequence};

/// The largest case file read, in bytes.
pub const MAX_CASE_BYTES: usize = 64 * 1024;

/// The most images a case renders: enough for every motion, bounded so that a check stays
/// quick.
pub const MAX_IMAGES: u64 = 10_000;

/// The square a case renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquareCase {
    /// The side, in 65 536ths of the height.
    pub side: u32,
    /// One sweep lasts `cycle[0] / cycle[1]` seconds.
    pub cycle: [u32; 2],
}

/// A visual conformance case: a square rendered at a rate and a size, from an image index,
/// for a number of images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// Images per second, `rate[0] / rate[1]`.
    pub rate: [u32; 2],
    /// Width of the images, in pixels.
    pub width: u32,
    /// Height of the images, in pixels.
    pub height: u32,
    /// The index of the first image rendered.
    pub first: u64,
    /// How many images are rendered, from 1 to [`MAX_IMAGES`].
    pub images: u64,
    /// The square.
    pub square: SquareCase,
}

/// The values of a case, checked.
#[derive(Debug, Clone, Copy)]
struct Checked {
    /// The image rate.
    rate: ImageRate,
    /// The images rendered.
    range: ImageRange,
    /// The image size.
    size: ImageSize,
    /// The square.
    square: Square,
}

/// Reads a case from the text of its file.
///
/// # Errors
///
/// Returns why the text is not a valid case: not JSON, an unknown or missing field, a
/// value of the wrong type, or a value outside its range.
pub fn parse_case(text: &str) -> Result<Case, String> {
    if text.len() > MAX_CASE_BYTES {
        return Err(format!(
            "a case file holds at most {MAX_CASE_BYTES} bytes, this one {}",
            text.len()
        ));
    }
    let case: Case = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let _checked = check(&case)?;
    Ok(case)
}

/// Checks the values of `case`.
fn check(case: &Case) -> Result<Checked, String> {
    if !(1..=MAX_IMAGES).contains(&case.images) {
        return Err(format!(
            "images must be from 1 to {MAX_IMAGES}, not {}",
            case.images
        ));
    }
    let range =
        ImageRange::new(ImageIndex(case.first), case.images).map_err(|error| error.to_string())?;
    let [numerator, denominator] = case.rate;
    let rate = ImageRate::new(numerator, denominator).map_err(|error| error.to_string())?;
    let size = ImageSize::new(case.width, case.height).map_err(|error| error.to_string())?;
    let [seconds, divisor] = case.square.cycle;
    let cycle = Seconds::new(seconds, divisor).map_err(|error| error.to_string())?;
    let square = Square::new(rate, case.square.side, cycle).map_err(|error| error.to_string())?;
    Ok(Checked {
        rate,
        range,
        size,
        square,
    })
}

/// Renders `case` with the exact CPU reference renderer and returns the image hash of its
/// images.
///
/// # Errors
///
/// Returns why the case is invalid (see [`parse_case`]).
pub fn render_case(case: &Case) -> Result<ImageHash, String> {
    let mut checked = check(case)?;
    let mut image = Image::new(checked.size);
    Ok(hash_sequence(
        &mut checked.square,
        checked.rate,
        checked.range,
        &mut image,
    ))
}

/// Reads a case from the text of its file and renders it to its image hash: what the
/// commands do with a case file.
///
/// # Errors
///
/// Returns why the text is not a valid case (see [`parse_case`]).
pub fn hash_case(text: &str) -> Result<ImageHash, String> {
    render_case(&parse_case(text)?)
}

#[cfg(test)]
#[path = "case_tests.rs"]
mod tests;
