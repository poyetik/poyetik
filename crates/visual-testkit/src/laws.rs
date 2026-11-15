//! The laws every visual box must respect.

use visual_core::{Image, ImageIndex, ImageRate, ImageSize, SubpixelPoint, Trace, VisualBox};

use proptest::collection::vec;
use proptest::prelude::{Strategy, prop_oneof};
use proptest::test_runner::{TestCaseError, TestError};

use crate::generate::{rate_terms, small_size, u64_edges};
use crate::heap_operations_during;
use crate::matrix::{image_indices, image_rates, image_sizes};
use crate::runner::runner;

/// The random cases of [`check_laws`] beyond the matrix: each a rate, a size and up to
/// eight indices drawn at random, their edges often.
const RANDOM_CASES: u32 = 64;

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

/// Whether two images hold the same bits, value for value: floats compared exactly (`-0`
/// and `0` differ, a NaN equals itself), with no allocation.
fn same_bits(left: &Image, right: &Image) -> bool {
    left.values()
        .iter()
        .map(|value| value.to_bits())
        .eq(right.values().iter().map(|value| value.to_bits()))
}

/// Renders image `index` of `visual` into `image`, refusing any heap operation: the only
/// way the laws render, so that every pass is held to the no-heap law. `how` says how the
/// pass renders, after "while rendering" (empty for the first pass).
fn render_into(
    visual: &mut dyn VisualBox,
    image: &mut Image,
    index: ImageIndex,
    at: &str,
    how: &str,
) -> Result<(), String> {
    let ((), operations) = heap_operations_during(|| visual.render(index, image))?;
    if operations == 0 {
        Ok(())
    } else {
        Err(format!(
            "no heap operation: {operations} while rendering{how} at {at}"
        ))
    }
}

/// Renders image `index` of `visual` into a fresh image of `size` ([`render_into`]).
fn render(
    visual: &mut dyn VisualBox,
    size: ImageSize,
    index: ImageIndex,
    at: &str,
    how: &str,
) -> Result<Image, String> {
    let mut image = Image::new(size);
    render_into(visual, &mut image, index, at, how)?;
    Ok(image)
}

/// An image of `size` whose every value is NaN: what rendering leaves unwritten shows.
fn dirty(size: ImageSize) -> Image {
    let mut image = Image::new(size);
    for row in image.rows_mut() {
        row.fill(f32::NAN);
    }
    image
}

/// `indices` from both ends in turn: the first, the last, the second, the one before the
/// last, and so on, so that every step jumps far forward or back.
fn from_both_ends<T: Copy>(indices: &[T]) -> Vec<T> {
    let mut ends = indices.iter().copied();
    let mut order = Vec::with_capacity(indices.len());
    while let Some(low) = ends.next() {
        order.push(low);
        order.extend(ends.next_back());
    }
    order
}

/// Points to trace in an image of `size`: its first subpixel (the top left corner), its
/// center and its last subpixel (the bottom right corner), every one inside it, as every
/// point is.
fn points(size: ImageSize) -> impl Iterator<Item = SubpixelPoint> {
    let unit = u64::from(visual_core::SUBPIXELS);
    let right = u64::from(size.width()).saturating_mul(unit);
    let bottom = u64::from(size.height()).saturating_mul(unit);
    [
        (0, 0),
        (right / 2, bottom / 2),
        (right.saturating_sub(1), bottom.saturating_sub(1)),
    ]
    .into_iter()
    .filter_map(move |(x, y)| size.point(x, y))
}

/// The bits of -0 as a 32-bit float: its sign bit alone.
const NEGATIVE_ZERO: u32 = 1 << 31_u32;

/// Checks the law of finite values on one rendered image: every value finite, none
/// subnormal and none -0, and alpha within `[0, 1]` too.
fn check_values(image: &Image, at: &str) -> Result<(), String> {
    for pixel in image.values().as_chunks::<{ Image::CHANNELS }>().0 {
        let &[red, green, blue, alpha] = pixel;
        if !(red.is_finite() && green.is_finite() && blue.is_finite() && alpha.is_finite()) {
            return Err(format!("finite values: a value is NaN or infinite at {at}"));
        }
        // No subnormal value and no -0 (`spec/visual-design.md`, section 3): a target that
        // flushes subnormals to zero, or signs zero differently, then cannot change an
        // image's bits, nor its hash.
        if pixel.iter().any(|value| value.is_subnormal()) {
            return Err(format!("finite values: a subnormal value at {at}"));
        }
        if pixel.iter().any(|value| value.to_bits() == NEGATIVE_ZERO) {
            return Err(format!("finite values: a negative zero at {at}"));
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
/// the shared visual matrix, then at 64 random ones (`RANDOM_CASES`).
///
/// The random ones come from a fixed seed: a rate (its terms up to 32 bits), a size (each
/// side up to 64 pixels) and from two to eight indices, in the order drawn, so that a box
/// right only at the values of the matrix is caught.
///
/// # Errors
///
/// Returns the first law broken, naming it and where (rate, size, image index); off the
/// matrix, at the simplest random case found to break it, after "Test failed: ", with the
/// images drawn in their order. Also fails when
/// [`crate::CountingAllocator`] is not installed, since the no-heap law could not be
/// checked.
pub fn check_laws(make: &Factory<'_>) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    let indices = image_indices();
    let sizes = image_sizes();
    for rate in image_rates() {
        for &size in &sizes {
            check_at(make, rate, size, &indices)?;
        }
        check_sizes(make, rate, &sizes, &indices)?;
    }
    let cases = (
        rate_terms().prop_filter_map("a valid rate", |(numerator, denominator)| {
            ImageRate::new(numerator, denominator).ok()
        }),
        prop_oneof![small_size(), (1_u32..=64, 1_u32..=64)]
            .prop_filter_map("a valid size", |(width, height)| {
                ImageSize::new(width, height).ok()
            }),
        vec(u64_edges().prop_map(ImageIndex), 2..=8),
    );
    runner(RANDOM_CASES)
        .run(&cases, |(rate, size, drawn)| {
            check_at(make, rate, size, &drawn).map_err(|law| {
                // The order the images were drawn in, which the random-access and no-heap
                // laws depend on, said on the same line as the law.
                let order: Vec<String> = drawn.iter().map(|index| index.0.to_string()).collect();
                TestCaseError::fail(format!("{law}, images drawn [{}]", order.join(", ")))
            })
        })
        .map_err(|error: TestError<_>| {
            // Its first line says the law, where, and the images drawn in their order; the
            // shrunk case follows in Debug form over many lines, saying nothing more.
            error
                .to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned()
        })
}

/// Checks that one instance renders the same bits, and traces the same points of every size,
/// as a fresh one when the size of its output changes from one image to the next: every size
/// of `sizes` in turn, at each index.
fn check_sizes(
    make: &Factory<'_>,
    rate: ImageRate,
    sizes: &[ImageSize],
    indices: &[ImageIndex],
) -> Result<(), String> {
    let mut switching = make(rate);
    for &index in indices {
        for &size in sizes {
            let at = place(rate, size, index);
            let image = render(
                switching.as_mut(),
                size,
                index,
                &at,
                ", after another size,",
            )?;
            let fresh = make(rate);
            let mut reference_box = make(rate);
            let reference = render(reference_box.as_mut(), size, index, &at, "")?;
            if !same_bits(&image, &reference) {
                return Err(format!(
                    "size change: rendering at another size first changes the bits at {at}"
                ));
            }
            // Every size's points, not only this one's: a box answering from the geometry
            // of the image it rendered last is caught too.
            for &traced in sizes {
                for point in points(traced) {
                    if switching.trace(index, point) != fresh.trace(index, point) {
                        return Err(format!(
                            "size change: ({}, {}) of size {}x{} traces differently after size {}x{} at {at}",
                            point.x(),
                            point.y(),
                            traced.width(),
                            traced.height(),
                            size.width(),
                            size.height()
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Checks every law at one rate and size, over `indices`, in their order.
fn check_at(
    make: &Factory<'_>,
    rate: ImageRate,
    size: ImageSize,
    indices: &[ImageIndex],
) -> Result<(), String> {
    let mut first = make(rate);
    let mut second = make(rate);
    // Renders the same images in the same order as `first`, each into an image filled with
    // NaN first: only what rendering leaves unwritten can differ.
    let mut third = make(rate);
    let mut references = Vec::new();
    for &index in indices {
        let at = place(rate, size, index);
        let image = render(first.as_mut(), size, index, &at, "")?;
        check_values(&image, &at)?;
        let again = render(second.as_mut(), size, index, &at, ", a second instance,")?;
        if !same_bits(&again, &image) {
            return Err(format!("determinism: two fresh instances differ at {at}"));
        }
        let mut over = dirty(size);
        render_into(third.as_mut(), &mut over, index, &at, " over other values")?;
        if !same_bits(&over, &image) {
            return Err(format!(
                "overwrite: a value kept from before rendering at {at}"
            ));
        }
        references.push((index, image));
    }
    let mut shuffled = make(rate);
    for (index, reference) in references.iter().rev() {
        let at = place(rate, size, *index);
        let image = render(
            shuffled.as_mut(),
            size,
            *index,
            &at,
            ", other images first,",
        )?;
        if !same_bits(&image, reference) {
            return Err(format!(
                "random access: rendering other images first changes the bits at {at}"
            ));
        }
        for point in points(size) {
            let fresh = make(rate);
            let traced: Trace = fresh.trace(*index, point);
            if fresh.trace(*index, point) != traced
                || shuffled.trace(*index, point) != traced
                || first.trace(*index, point) != traced
            {
                return Err(format!(
                    "pure trace: ({}, {}) traces differently at {at}",
                    point.x(),
                    point.y()
                ));
            }
        }
    }
    // Every image again, jumping between both ends of the matrix, each twice in a row,
    // the second time over other values: random access in any direction, and a box that
    // skips an image it rendered last.
    let mut jumping = make(rate);
    for (index, reference) in from_both_ends(&references.iter().collect::<Vec<_>>()) {
        let at = place(rate, size, *index);
        let once = render(
            jumping.as_mut(),
            size,
            *index,
            &at,
            ", jumping between images,",
        )?;
        let mut twice = dirty(size);
        render_into(
            jumping.as_mut(),
            &mut twice,
            *index,
            &at,
            ", the same image again,",
        )?;
        if !same_bits(&once, reference) || !same_bits(&twice, reference) {
            return Err(format!(
                "random access: rendering the images in another order, each twice, changes the bits at {at}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
