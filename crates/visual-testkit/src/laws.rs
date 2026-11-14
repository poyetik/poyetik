//! The laws every visual box must respect.

use visual_core::{Image, ImageIndex, ImageRate, ImageSize, SubpixelPoint, Trace, VisualBox};

use crate::heap_operations_during;
use crate::matrix::{image_indices, image_rates, image_sizes};

/// Creates a fresh instance of the box under test for images at a rate.
pub type Factory<'make> = dyn Fn(ImageRate) -> Box<dyn VisualBox> + 'make;

/// Where a law was checked: named in every failure.
fn place(rate: ImageRate, size: ImageSize, index: ImageIndex) -> String {
    format!(
        "rate {}/{}, size {}x{}, image {}",
        rate.numerator(),
        rate.denominator(),
        size.width(),
        size.height(),
        index.0
    )
}

/// The bit patterns of an image's values, so that `-0.0` and NaN payloads compare exactly.
fn bits(image: &Image) -> Vec<u32> {
    image.values().iter().map(|value| value.to_bits()).collect()
}

/// Renders image `index` of `visual` into a fresh image of `size`, counting heap
/// operations.
fn render(
    visual: &mut dyn VisualBox,
    size: ImageSize,
    index: ImageIndex,
) -> Result<(Image, usize), String> {
    let mut image = Image::new(size);
    let ((), operations) = heap_operations_during(|| visual.render(index, &mut image))?;
    Ok((image, operations))
}

/// Points to trace in an image of `size`: its corners, its center, and one outside it.
fn points(size: ImageSize) -> [SubpixelPoint; 4] {
    let unit = u64::from(visual_core::SUBPIXELS);
    let right = u64::from(size.width()).saturating_mul(unit);
    let bottom = u64::from(size.height()).saturating_mul(unit);
    [
        SubpixelPoint { x: 0, y: 0 },
        SubpixelPoint {
            x: right / 2,
            y: bottom / 2,
        },
        SubpixelPoint {
            x: right.saturating_sub(1),
            y: bottom.saturating_sub(1),
        },
        SubpixelPoint {
            x: right,
            y: bottom,
        },
    ]
}

/// Checks the value laws of one rendered image: finite values, alpha within `[0, 1]`.
fn check_values(image: &Image, at: &str) -> Result<(), String> {
    for pixel in image.values().as_chunks::<{ Image::CHANNELS }>().0 {
        let &[red, green, blue, alpha] = pixel;
        if !(red.is_finite() && green.is_finite() && blue.is_finite() && alpha.is_finite()) {
            return Err(format!("finite values: a value is NaN or infinite at {at}"));
        }
        if !(0.0..=1.0).contains(&alpha) {
            return Err(format!(
                "finite values: alpha {alpha} outside [0, 1] at {at}"
            ));
        }
    }
    Ok(())
}

/// Checks every law on the box `make` creates, at every image rate, size and index of
/// the shared visual matrix.
///
/// # Errors
///
/// Returns the first law broken, naming it and where (rate, size, image index). Also
/// fails when [`crate::CountingAllocator`] is not installed, since the no-heap law could
/// not be checked.
pub fn check_laws(make: &Factory<'_>) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    for rate in image_rates() {
        for size in image_sizes() {
            check_at(make, rate, size)?;
        }
    }
    Ok(())
}

/// Checks every law at one rate and size, over every image index of the matrix.
fn check_at(make: &Factory<'_>, rate: ImageRate, size: ImageSize) -> Result<(), String> {
    let mut first = make(rate);
    let mut second = make(rate);
    // Renders the same images in the same order as `first`, each into an image filled with
    // NaN first: only what rendering leaves unwritten can differ.
    let mut third = make(rate);
    let mut references = Vec::new();
    for index in image_indices() {
        let at = place(rate, size, index);
        let (image, operations) = render(first.as_mut(), size, index)?;
        if operations != 0 {
            return Err(format!(
                "no heap operation: {operations} while rendering at {at}"
            ));
        }
        check_values(&image, &at)?;
        let (again, _) = render(second.as_mut(), size, index)?;
        if bits(&again) != bits(&image) {
            return Err(format!("determinism: two fresh instances differ at {at}"));
        }
        let mut dirty = Image::new(size);
        for row in dirty.rows_mut() {
            row.fill(f32::NAN);
        }
        third.render(index, &mut dirty);
        if bits(&dirty) != bits(&image) {
            return Err(format!(
                "overwrite: a value kept from before rendering at {at}"
            ));
        }
        references.push((index, image));
    }
    let mut shuffled = make(rate);
    for (index, reference) in references.iter().rev() {
        let at = place(rate, size, *index);
        let (image, _) = render(shuffled.as_mut(), size, *index)?;
        if bits(&image) != bits(reference) {
            return Err(format!(
                "random access: rendering other images first changes the bits at {at}"
            ));
        }
        for point in points(size) {
            let fresh = make(rate);
            let traced: Trace = fresh.trace(*index, reference, point);
            if fresh.trace(*index, reference, point) != traced
                || shuffled.trace(*index, reference, point) != traced
                || first.trace(*index, reference, point) != traced
            {
                return Err(format!(
                    "pure trace: ({}, {}) traces differently at {at}",
                    point.x, point.y
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
