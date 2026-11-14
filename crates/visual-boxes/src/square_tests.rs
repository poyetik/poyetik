extern crate alloc;

use alloc::vec::Vec;

use visual_core::{
    Image, ImageIndex, ImageRate, ImageSize, SUBPIXELS, Seconds, SubpixelPoint, Trace, VisualBox,
};
use visual_testkit::heap_operations_during;

use super::Square;
use crate::BoxError;

/// A square of `side` 65 536ths of the height, at `rate`, sweeping once per `seconds`.
fn square(rate: ImageRate, side: u32, seconds: (u32, u32)) -> Square {
    let cycle = Seconds::new(seconds.0, seconds.1).expect("a valid cycle");
    Square::new(rate, side, cycle).expect("a valid square")
}

/// One image per second.
fn per_second() -> ImageRate {
    ImageRate::new(1, 1).expect("a valid rate")
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
fn reaches_the_right_edge_at_the_end_of_the_cycle() {
    // 4 images per cycle on a width of 4 pixels with a 1-pixel square (side = height):
    // travel 3 pixels, so image 3 puts the left edge at 9/4 pixels.
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
    let mut forward = square(ImageRate::PER_SECOND_60, 17_000, (5, 3));
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
    // 10^12 images at 60 per second (about 528 years): no drift, still periodic.
    let mut visual = square(ImageRate::PER_SECOND_60, 30_000, (1, 1));
    let start = rendered(&mut visual, 0, 9, 4);
    let far = rendered(&mut visual, 60 * 1_000_000_000_000, 9, 4);
    assert_eq!(
        bits(far.values()),
        bits(start.values()),
        "whole cycles later"
    );
    let last = rendered(&mut visual, u64::MAX, 9, 4);
    assert!(
        last.values().iter().all(|value| value.is_finite()),
        "the last index renders"
    );
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
fn never_allocates_while_rendering() {
    let mut visual = square(ImageRate::PER_SECOND_60, 30_000, (2, 1));
    let mut image = Image::new(ImageSize::new(64, 36).expect("a valid size"));
    for index in [0, 1, 59, 1_000_000] {
        let ((), count) = heap_operations_during(|| visual.render(ImageIndex(index), &mut image))
            .expect("the counting allocator is installed");
        assert_eq!(count, 0, "image {index}");
    }
}

#[test]
fn traces_a_point_to_the_square_or_the_background() {
    // Image 0 of the 4 x 2 case: the square spans [0, 4096) x [2048, 6144).
    let visual = square(per_second(), Square::FULL_SIDE / 2, (4, 1));
    let image = Image::new(ImageSize::new(4, 2).expect("a valid size"));
    for (x, y, trace) in [
        (0, 2048, Trace::Shape(0)),
        (4095, 6143, Trace::Shape(0)),
        (4096, 2048, Trace::Background),
        (0, 6144, Trace::Background),
        (0, 2047, Trace::Background),
        (10_000, 4000, Trace::Background),
        (u64::MAX, u64::MAX, Trace::Background),
    ] {
        assert_eq!(
            visual.trace(ImageIndex(0), &image, SubpixelPoint { x, y }),
            trace,
            "({x}, {y})"
        );
    }
    // Image 1: the square moved 3/4 pixel to the right.
    let moved = SubpixelPoint { x: 3071, y: 4000 };
    assert_eq!(
        visual.trace(ImageIndex(1), &image, moved),
        Trace::Background,
        "left of it"
    );
    let inside = SubpixelPoint { x: 3072, y: 4000 };
    assert_eq!(
        visual.trace(ImageIndex(1), &image, inside),
        Trace::Shape(0),
        "its edge"
    );
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
        let center = SubpixelPoint {
            x: column * u64::from(SUBPIXELS) + half,
            y: row * u64::from(SUBPIXELS) + half,
        };
        let expected = if red.to_bits() == 1.0_f32.to_bits() {
            Trace::Shape(0)
        } else {
            Trace::Background
        };
        assert_eq!(
            visual.trace(ImageIndex(0), &image, center),
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
    assert!(
        Square::new(per_second(), Square::FULL_SIDE, cycle).is_ok(),
        "as tall as the image"
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
