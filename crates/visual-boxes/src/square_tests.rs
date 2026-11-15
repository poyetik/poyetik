extern crate alloc;

use alloc::format;
use alloc::string::ToString as _;
use alloc::vec::Vec;

use visual_core::{Image, ImageIndex, ImageRate, ImageSize, SUBPIXELS, Seconds, Trace, VisualBox};
use visual_testkit::generate::{
    rate_terms, seconds_terms, side, small_size, tall_size, u64_edges, wide_size,
};
use visual_testkit::proptest::prelude::{Strategy, prop_oneof};
use visual_testkit::proptest::test_runner::TestCaseError;
use visual_testkit::runner::runner;

use super::Square;
use crate::BoxError;

/// A square of `side` 65 536ths of the height, at `rate`, sweeping once per `seconds`.
fn square(rate: ImageRate, side: u32, seconds: (u32, u32)) -> Square {
    let cycle = Seconds::new(seconds.0, seconds.1).expect("a valid cycle");
    Square::new(rate, side, cycle).expect("a valid square")
}

/// 60 images per second.
fn sixty() -> ImageRate {
    ImageRate::new(60, 1).expect("a valid rate")
}

/// One image per second.
fn per_second() -> ImageRate {
    ImageRate::new(1, 1).expect("a valid rate")
}

#[test]
fn is_the_full_side_the_generators_reach() {
    assert_eq!(
        visual_testkit::generate::FULL_SIDE,
        Square::FULL_SIDE,
        "the test kit's copy, which it cannot name"
    );
}

#[test]
fn keeps_the_rate_it_is_made_for() {
    let rate = ImageRate::new(24_000, 1001).expect("a valid rate");
    assert_eq!(square(rate, 1, (3, 2)).rate(), rate, "the rate given");
    assert_eq!(
        square(per_second(), 1, (3, 2)).rate(),
        per_second(),
        "another one"
    );
}

/// Renders image `index` of `visual` at `width` × `height`.
fn rendered(visual: &mut Square, index: u64, width: u32, height: u32) -> Image {
    let mut image = Image::new(ImageSize::new(width, height).expect("a valid size"));
    visual.render(ImageIndex(index), &mut image);
    image
}

/// The red value of every pixel, row by row.
fn reds(image: &Image) -> Vec<f32> {
    image
        .values()
        .as_chunks::<{ Image::CHANNELS }>()
        .0
        .iter()
        .map(|&[red, _, _, _]| red)
        .collect()
}

/// The bits of `values`, to compare floats exactly.
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

/// The covered area of an image, in pixels: the sum of its red values. Exact in `f64`:
/// every value is a multiple of 2^-24 and the sum stays below 2^28, so it needs at most
/// 52 bits.
fn covered_area(image: &Image) -> f64 {
    reds(image).iter().map(|&red| f64::from(red)).sum()
}

/// The area of a square of `side` 65 536ths of `height` in a `width`-pixel-wide image, in
/// pixels, clipped to the width: exact in `f64` (a product of two integers below 2^27,
/// divided by a power of two).
fn square_area(width: u32, height: u32, side: u32) -> f64 {
    let unit = u64::from(SUBPIXELS);
    let tall = u64::from(height).checked_mul(unit).expect("below 2^27");
    let edge = tall.checked_mul(u64::from(side)).expect("below 2^43") >> 16_u32;
    let visible = edge.min(u64::from(width).checked_mul(unit).expect("below 2^27"));
    let edge = u32::try_from(edge).expect("below 2^27");
    let visible = u32::try_from(visible).expect("below 2^27");
    f64::from(edge) * f64::from(visible) / 16_777_216.0_f64
}

#[test]
fn renders_the_hand_computed_images() {
    // A 4 x 2 image, a side of half the height (one pixel), four images per sweep: the
    // square covers half of each row vertically, and moves 3/4 pixel per image.
    let mut visual = square(per_second(), Square::FULL_SIDE / 2, (4, 1));
    let expected: [(u64, [f32; 8]); 3] = [
        (0, [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0]),
        (1, [0.125, 0.375, 0.0, 0.0, 0.125, 0.375, 0.0, 0.0]),
        (4, [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0]),
    ];
    for (index, red) in expected {
        let image = rendered(&mut visual, index, 4, 2);
        assert_eq!(bits(&reds(&image)), bits(&red), "image {index}");
    }
}

#[test]
fn is_white_on_opaque_black() {
    let mut visual = square(per_second(), 20_000, (3, 1));
    for index in 0..6 {
        let image = rendered(&mut visual, index, 7, 5);
        for pixel in image.values().as_chunks::<{ Image::CHANNELS }>().0 {
            let &[red, green, blue, alpha] = pixel;
            assert_eq!(red.to_bits(), green.to_bits(), "gray");
            assert_eq!(red.to_bits(), blue.to_bits(), "gray");
            assert_eq!(alpha.to_bits(), 1.0_f32.to_bits(), "opaque");
            assert!((0.0..=1.0).contains(&red), "{red} within 0..1");
        }
    }
}

#[test]
fn covers_exactly_its_area_wherever_it_is() {
    // The exact law of antialiasing: the values sum to the square's area, whatever the
    // position (here at sizes that put the edges at every fraction of a pixel).
    for (width, height, side) in [(16, 9, 30_000), (5, 7, 65_536), (33, 3, 1), (1, 1, 12_345)] {
        let mut visual = square(per_second(), side, (7, 1));
        for index in 0..7 {
            let image = rendered(&mut visual, index, width, height);
            assert_eq!(
                covered_area(&image).to_bits(),
                square_area(width, height, side).to_bits(),
                "{width}x{height}, side {side}, image {index}"
            );
        }
    }
}

#[test]
fn starts_each_cycle_at_the_left_edge_and_repeats() {
    let rate = ImageRate::new(24_000, 1001).expect("23.976 images per second");
    // A cycle of exactly 1001/1000 s: 24 images per sweep at 24000/1001 images per second.
    let mut visual = square(rate, 10_000, (1001, 1000));
    let first = rendered(&mut visual, 0, 32, 18);
    let red = reds(&first);
    assert!(
        red.iter().step_by(32).any(|&value| value > 0.0),
        "touches the left column"
    );
    for cycles in [1_u64, 2, 1000] {
        let again = rendered(&mut visual, 24 * cycles, 32, 18);
        assert_eq!(
            bits(again.values()),
            bits(first.values()),
            "after {cycles} cycles"
        );
    }
    let moved = rendered(&mut visual, 12, 32, 18);
    assert_ne!(
        bits(moved.values()),
        bits(first.values()),
        "moves within a cycle"
    );
}

#[test]
fn stops_short_of_the_right_edge_at_the_end_of_the_cycle() {
    // 4 images per cycle on a width of 4 pixels with a 1-pixel square (side = height):
    // travel 3 pixels, so image 3 puts the left edge at 9/4 pixels, a quarter of the
    // travel short of the right edge, which only the next cycle's start would reach.
    let mut visual = square(per_second(), Square::FULL_SIDE, (4, 1));
    let image = rendered(&mut visual, 3, 4, 1);
    assert_eq!(
        bits(&reds(&image)),
        bits(&[0.0, 0.0, 0.75, 0.25]),
        "left edge at 2.25 pixels"
    );
}

#[test]
fn gives_the_same_bits_in_any_order() {
    // Random access: image n depends only on n.
    let mut forward = square(sixty(), 17_000, (5, 3));
    let mut backward = forward;
    let ahead: Vec<Vec<u32>> = (0..40)
        .map(|index| bits(rendered(&mut forward, index, 11, 6).values()))
        .collect();
    for index in (0..40).rev() {
        let image = rendered(&mut backward, index, 11, 6);
        assert_eq!(
            Some(&bits(image.values())),
            ahead.get(usize::try_from(index).expect("small")),
            "image {index}"
        );
    }
}

#[test]
fn stays_exact_far_in_time() {
    // 10^12 cycles of one second at 60 images per second (6 × 10^13 images, about 31 700
    // years): no drift, still periodic.
    let mut visual = square(sixty(), 30_000, (1, 1));
    let start = rendered(&mut visual, 0, 9, 4);
    let far = rendered(&mut visual, 60 * 1_000_000_000_000, 9, 4);
    assert_eq!(
        bits(far.values()),
        bits(start.values()),
        "whole cycles later"
    );
    // Image 2^64 - 1 is where image 15 is: at 60 per second with a cycle of one second,
    // (2^64 - 1) mod 60 = 15; at 24000/1001 with a cycle of 1001/1000 s, 24 images a
    // cycle and (2^64 - 1) × 1001 × 1000 mod (24000 × 1001) = 15 × 1001 × 1000, a product
    // past 2^64, so that 64-bit arithmetic, saturating or wrapping, gives another image.
    for (rate, cycle) in [
        (sixty(), (1, 1)),
        (
            ImageRate::new(24_000, 1001).expect("a valid rate"),
            (1001, 1000),
        ),
    ] {
        let mut placed = square(rate, 30_000, cycle);
        let last = rendered(&mut placed, u64::MAX, 9, 4);
        let fifteenth = rendered(&mut placed, 15, 9, 4);
        let fourteenth = rendered(&mut placed, 14, 9, 4);
        assert_eq!(
            bits(last.values()),
            bits(fifteenth.values()),
            "{cycle:?}: the last index is image 15"
        );
        assert_ne!(
            bits(last.values()),
            bits(fourteenth.values()),
            "{cycle:?}: and not image 14"
        );
    }
}

#[test]
fn a_zero_side_draws_nothing_and_a_full_side_the_whole_height() {
    let mut empty = square(per_second(), 0, (2, 1));
    let image = rendered(&mut empty, 1, 5, 3);
    assert!(reds(&image).iter().all(|&red| red.to_bits() == 0), "black");
    let mut full = square(per_second(), Square::FULL_SIDE, (1, 1));
    let covered = rendered(&mut full, 0, 3, 3);
    assert!(
        reds(&covered)
            .iter()
            .all(|&red| red.to_bits() == 1.0_f32.to_bits()),
        "a square image covered by a square as tall as it"
    );
}

#[test]
fn never_moves_when_wider_than_the_image() {
    // A side of the full height on a narrow image: no travel, clipped at the right.
    let mut visual = square(per_second(), Square::FULL_SIDE, (3, 1));
    let first = rendered(&mut visual, 0, 2, 4);
    for index in 1..3 {
        let image = rendered(&mut visual, index, 2, 4);
        assert_eq!(bits(image.values()), bits(first.values()), "image {index}");
    }
    assert!(
        reds(&first)
            .iter()
            .all(|&red| red.to_bits() == 1.0_f32.to_bits()),
        "covers the whole image"
    );
}

#[test]
fn renders_at_the_edges_of_the_size_range() {
    let mut visual = square(per_second(), 40_000, (3, 2));
    for (width, height) in [(1, 1), (ImageSize::MAX_SIDE, 1), (1, ImageSize::MAX_SIDE)] {
        let image = rendered(&mut visual, 2, width, height);
        assert_eq!(
            covered_area(&image).to_bits(),
            square_area(width, height, 40_000).to_bits(),
            "{width}x{height}"
        );
    }
}

#[test]
fn traces_a_point_to_the_square_or_the_background() {
    // Image 0 of the 4 x 2 case: the square spans [0, 4096) x [2048, 6144).
    let visual = square(per_second(), Square::FULL_SIDE / 2, (4, 1));
    let size = ImageSize::new(4, 2).expect("a valid size");
    let at = |x, y| size.point(x, y).expect("inside the image");
    for (x, y, trace) in [
        (0, 2048, Trace::Shape(0)),
        (4095, 6143, Trace::Shape(0)),
        (4096, 2048, Trace::Background),
        (0, 6144, Trace::Background),
        (0, 2047, Trace::Background),
        (10_000, 4000, Trace::Background),
        (16_383, 8191, Trace::Background),
    ] {
        assert_eq!(visual.trace(ImageIndex(0), at(x, y)), trace, "({x}, {y})");
    }
    // Image 1: the square moved 3/4 pixel to the right.
    assert_eq!(
        visual.trace(ImageIndex(1), at(3071, 4000)),
        Trace::Background,
        "left of it"
    );
    assert_eq!(
        visual.trace(ImageIndex(1), at(3072, 4000)),
        Trace::Shape(0),
        "its edge"
    );
    // A square wider than the image is clipped: every point of the image is on it.
    let wide = square(per_second(), Square::FULL_SIDE, (4, 1));
    let narrow = ImageSize::new(1, 4).expect("a valid size");
    for (x, y) in [(0, 0), (4095, 16_383)] {
        assert_eq!(
            wide.trace(ImageIndex(3), narrow.point(x, y).expect("inside")),
            Trace::Shape(0),
            "({x}, {y}) of a clipped square"
        );
    }
}

#[test]
fn traces_what_it_draws() {
    // A point is on the square exactly where its pixel's value would be 1 for a pixel-
    // aligned square: checked at every pixel center of an aligned case.
    let mut visual = square(per_second(), Square::FULL_SIDE / 2, (4, 1));
    let image = rendered(&mut visual, 0, 4, 4);
    let half = u64::from(SUBPIXELS) / 2;
    for (pixel, red) in reds(&image).iter().enumerate() {
        let column = u64::try_from(pixel % 4).expect("small");
        let row = u64::try_from(pixel / 4).expect("small");
        let center = image
            .size()
            .point(
                column * u64::from(SUBPIXELS) + half,
                row * u64::from(SUBPIXELS) + half,
            )
            .expect("inside");
        let expected = if red.to_bits() == 1.0_f32.to_bits() {
            Trace::Shape(0)
        } else {
            Trace::Background
        };
        assert_eq!(
            visual.trace(ImageIndex(0), center),
            expected,
            "pixel {pixel}"
        );
    }
}

#[test]
fn rejects_a_side_taller_than_the_image() {
    let cycle = Seconds::new(1, 1).expect("a valid cycle");
    assert_eq!(
        Square::new(per_second(), Square::FULL_SIDE + 1, cycle),
        Err(BoxError::InvalidSide(65_537)),
        "too tall"
    );
    assert_eq!(
        Square::new(per_second(), u32::MAX, cycle),
        Err(BoxError::InvalidSide(u32::MAX)),
        "largest"
    );
    assert_eq!(
        Square::new(per_second(), Square::FULL_SIDE, cycle).map(|tall| tall.side),
        Ok(Square::FULL_SIDE),
        "as tall as the image"
    );
}

/// The coverage of pixel `(column, row)` by the square, in subpixel units of area,
/// computed independently of the square's code, from `spec/visual-design.md` (sections 4
/// and 6) in 128-bit integers: the reference each pixel is compared with.
fn reference_coverage(
    rate: (u32, u32),
    side: u32,
    cycle: (u32, u32),
    index: u64,
    dimensions: (u32, u32),
    pixel: (u32, u32),
) -> u128 {
    // Every operand is below 2^64 and every product below 2^128: checked, never wrong.
    let times = |left: u128, right: u128| left.checked_mul(right).expect("below 2^128");
    let over = |left: u128, right: u128| left.checked_div(right).expect("a nonzero divisor");
    let plus = |left: u128, right: u128| left.checked_add(right).expect("below 2^128");
    let unit = 4096_u128;
    let width = times(u128::from(dimensions.0), unit);
    let height = times(u128::from(dimensions.1), unit);
    let edge = over(times(height, u128::from(side)), 65_536);
    let travel = width.saturating_sub(edge);
    let period = times(u128::from(rate.0), u128::from(cycle.0));
    let elapsed = times(
        times(u128::from(index), u128::from(rate.1)),
        u128::from(cycle.1),
    );
    let within = elapsed.checked_rem(period).expect("a nonzero period");
    let left = over(times(within, travel), period);
    let top = over(height.saturating_sub(edge), 2);
    let overlap = |start: u128, end: u128, from: u128| {
        end.min(plus(from, unit)).saturating_sub(start.max(from))
    };
    times(
        overlap(left, plus(left, edge), times(u128::from(pixel.0), unit)),
        overlap(top, plus(top, edge), times(u128::from(pixel.1), unit)),
    )
}

#[test]
fn renders_every_pixel_as_its_exact_reference() {
    // Every pixel of every image of six whole cycles (35 images: 7/2 images a second, a
    // cycle of 5/3 s, 35/6 images, after which the positions repeat), and far in time, at small sizes and sides from none to the full
    // height, odd margins included: each value is exactly its coverage / 2^24.
    let rate = (7, 2);
    let cycle = (5, 3);
    let scale = f64::from(1_u32 << 24_u32).recip();
    for width in 1..=5_u32 {
        for height in 1..=4_u32 {
            for side in [
                0,
                1,
                4095,
                21_845,
                32_768,
                40_000,
                65_535,
                Square::FULL_SIDE,
            ] {
                let mut visual = square(
                    ImageRate::new(rate.0, rate.1).expect("a valid rate"),
                    side,
                    cycle,
                );
                for index in (0..36).chain([(1 << 53_u32) + 1, u64::MAX]) {
                    let image = rendered(&mut visual, index, width, height);
                    for (at, &red) in reds(&image).iter().enumerate() {
                        let at = u32::try_from(at).expect("small");
                        let coverage = reference_coverage(
                            rate,
                            side,
                            cycle,
                            index,
                            (width, height),
                            (at % width, at / width),
                        );
                        let expected =
                            f64::from(u32::try_from(coverage).expect("at most 2^24")) * scale;
                        assert_eq!(
                            f64::from(red).to_bits(),
                            expected.to_bits(),
                            "{width}x{height}, side {side}, image {index}, pixel {at}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn renders_every_pixel_as_its_exact_reference_at_the_extreme_terms() {
    // The regime where the position within a cycle times the travel passes 2^64: rate and
    // cycle terms of 32 bits (those of `square-last-images-at-extreme-terms`), indices up
    // to the largest, and the widest row, whose travel is near 2^26 subpixel units; the
    // smaller terms of the matrix's largest rate too. Every pixel equals its reference.
    let scale = f64::from(1_u32 << 24_u32).recip();
    for (rate, cycle) in [
        ((4_294_967_291, 4_294_968), (4_294_967_295, 4_294_967_291)),
        ((30_000, 1001), (4_294_967_295, 1)),
        ((1, 1), (4_294_967_295, 1)),
        // The largest elapsed product: both denominators near 2^32 and the last index, so
        // `index × q × b` comes within 2^97 of 2^128 and must not saturate.
        ((4_294_967_295, 4_294_967_294), (1, 4_294_967_295)),
    ] {
        for (width, height) in [(ImageSize::MAX_SIDE, 1), (64, 36), (5, 3)] {
            for side in [1, 30_000, Square::FULL_SIDE] {
                let mut visual = square(
                    ImageRate::new(rate.0, rate.1).expect("a valid rate"),
                    side,
                    cycle,
                );
                for index in [0, 1, 1 << 40_u32, u64::MAX - 47, u64::MAX - 1, u64::MAX] {
                    let image = rendered(&mut visual, index, width, height);
                    for (at, &red) in reds(&image).iter().enumerate() {
                        let at = u32::try_from(at).expect("small");
                        let coverage = reference_coverage(
                            rate,
                            side,
                            cycle,
                            index,
                            (width, height),
                            (at % width, at / width),
                        );
                        let expected =
                            f64::from(u32::try_from(coverage).expect("at most 2^24")) * scale;
                        assert_eq!(
                            f64::from(red).to_bits(),
                            expected.to_bits(),
                            "{rate:?} {cycle:?} {width}x{height}, side {side}, image {index}, pixel {at}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn moves_by_the_same_distance_from_image_to_image() {
    // The left edge, read from the pixels of a row the square fully covers: it advances by
    // ⌊travel × q b / (p a)⌋ subpixel units or one more, every image of a cycle, and
    // jumps back at its end. 64 × 36 pixels, a side of 30 000 / 65 536 of the height, at
    // 24000/1001 images a second, one sweep per 1001/1000 s: 24 images a cycle.
    let mut visual = square(
        ImageRate::new(24_000, 1001).expect("a valid rate"),
        30_000,
        (1001, 1000),
    );
    let unit = u64::from(SUBPIXELS);
    let edge = 36 * unit * 30_000 / 65_536;
    let travel = 64 * unit - edge;
    let step = travel * 1001 * 1000 / (24_000 * 1001);
    let left_edge = |image: &Image| -> u64 {
        let row: Vec<f32> = reds(image).into_iter().skip(64 * 18).take(64).collect();
        let column = row
            .iter()
            .position(|&red| red.to_bits() != 0)
            .expect("the square");
        let red = f64::from(*row.get(column).expect("a column"));
        // A fully covered row: the first column's value is its covered width / 4096.
        let covered = (1..=unit)
            .find(|&width| {
                (f64::from(u32::try_from(width).expect("small")) / 4096.0).to_bits()
                    == red.to_bits()
            })
            .expect("a whole number of subpixel units");
        u64::try_from(column + 1).expect("small") * unit - covered
    };
    let lefts: Vec<u64> = (0..=24)
        .map(|index| left_edge(&rendered(&mut visual, index, 64, 36)))
        .collect();
    assert_eq!(lefts.first(), Some(&0), "a cycle starts at the left edge");
    for (index, (&before, &after)) in lefts.iter().zip(lefts.iter().skip(1)).enumerate().take(23) {
        assert!(
            after - before == step || after - before == step + 1,
            "image {index} to the next: {} units, not {step} or one more",
            after - before
        );
    }
    assert_eq!(
        lefts.get(24),
        lefts.first(),
        "image 24 starts the next cycle"
    );
}

#[test]
fn respects_every_visual_box_law() {
    // Sides from none to the full height, cycles shorter and longer than an image, and
    // not whole numbers of images, at every rate, size and index of the shared matrix.
    for side in [0, 1, 30_000, Square::FULL_SIDE] {
        for cycle in [(1, 1), (1001, 1000), (1, 1000), (3600, 1)] {
            let laws = visual_testkit::check_laws(&|rate| -> alloc::boxed::Box<dyn VisualBox> {
                alloc::boxed::Box::new(square(rate, side, cycle))
            });
            assert_eq!(laws, Ok(()), "side {side}, cycle {cycle:?}");
        }
    }
}

/// A random square and image: the terms of its rate, its side, the terms of its cycle, an
/// image index and the image's width and height.
type Drawn = ((u32, u32), u32, (u32, u32), u64, (u32, u32));

/// Checks, on one random square and image, every property the square promises: each pixel
/// equals its exact reference, the values sum to the square's area, the image repeats one
/// whole period of images later when that index exists, and the trace agrees with the
/// pixels (a fully covered pixel's center is on the square, an empty one's is not).
fn holds_its_properties(
    (rate, side, cycle, index, (width, height)): Drawn,
) -> Result<(), TestCaseError> {
    let at = format!("{rate:?} side {side} cycle {cycle:?} image {index} {width}x{height}");
    let fail = |what: &str| TestCaseError::fail(format!("{at}: {what}"));
    let image_rate = ImageRate::new(rate.0, rate.1).map_err(|_| fail("an invalid rate drawn"))?;
    let mut visual = square(image_rate, side, cycle);
    let image = rendered(&mut visual, index, width, height);
    let scale = f64::from(1_u32 << 24_u32).recip();
    let unit = u64::from(SUBPIXELS);
    let half = unit / 2;
    for (column, row, red) in (0..height)
        .flat_map(|row| (0..width).map(move |column| (column, row)))
        .zip(reds(&image))
        .map(|((column, row), red)| (column, row, red))
    {
        let coverage = reference_coverage(rate, side, cycle, index, (width, height), (column, row));
        let expected =
            f64::from(u32::try_from(coverage).map_err(|_| fail("a coverage above 2^24"))?) * scale;
        if f64::from(red).to_bits() != expected.to_bits() {
            return Err(fail(&format!(
                "pixel ({column}, {row}) is {red}, not {expected}"
            )));
        }
        let center = u64::from(column)
            .checked_mul(unit)
            .and_then(|left| left.checked_add(half))
            .zip(
                u64::from(row)
                    .checked_mul(unit)
                    .and_then(|top| top.checked_add(half)),
            )
            .and_then(|(x, y)| image.size().point(x, y))
            .ok_or_else(|| fail("a pixel center outside the image"))?;
        let traced = visual.trace(ImageIndex(index), center);
        if (red.to_bits() == 1.0_f32.to_bits() && traced != Trace::Shape(0))
            || (red.to_bits() == 0.0_f32.to_bits() && traced != Trace::Background)
        {
            return Err(fail(&format!(
                "pixel ({column}, {row}) is {red}, traced {traced:?}"
            )));
        }
    }
    if covered_area(&image).to_bits() != square_area(width, height, side).to_bits() {
        return Err(fail("not the square's area"));
    }
    // One whole period of images: rate × cycle seconds, in lowest terms, below 2^64 × 2^64.
    let images = u128::from(rate.0).saturating_mul(u128::from(cycle.0));
    let seconds = u128::from(rate.1).saturating_mul(u128::from(cycle.1));
    let (mut common, mut rest) = (images, seconds);
    while let Some(next) = common.checked_rem(rest) {
        (common, rest) = (rest, next);
    }
    let period = images.checked_div(common).unwrap_or(images);
    if let Some(later) = u64::try_from(period)
        .ok()
        .and_then(|period| index.checked_add(period))
    {
        let again = rendered(&mut visual, later, width, height);
        if bits(&reds(&again)) != bits(&reds(&image)) {
            return Err(fail(&format!("image {later}, a period later, differs")));
        }
    }
    Ok(())
}

#[test]
fn holds_its_properties_at_random_terms_sizes_and_indices() {
    // Small images, every pixel against its reference; then wide and tall ones, where the
    // travel and the side are longest. Rates, cycles, sides and indices across their whole
    // domains, their edges often.
    let draw = |sizes| (rate_terms(), side(), seconds_terms(), u64_edges(), sizes);
    let small = runner(512).run(&draw(small_size().boxed()), holds_its_properties);
    assert_eq!(
        small.map_err(|error| error.to_string()),
        Ok(()),
        "small images"
    );
    let long = prop_oneof![wide_size(), tall_size()].boxed();
    let long = runner(64).run(&draw(long), holds_its_properties);
    assert_eq!(
        long.map_err(|error| error.to_string()),
        Ok(()),
        "wide and tall images"
    );
}
