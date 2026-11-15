//! Tests of `render.rs`.

extern crate alloc;

use alloc::vec::Vec;

use super::render_sequence;
use crate::{Image, ImageIndex, ImageRange, ImageSize, VisualBox};

/// Writes its image index in every value, to show which image was rendered.
#[derive(Debug)]
struct Counter;

impl VisualBox for Counter {
    fn render(&mut self, index: ImageIndex, output: &mut Image) {
        let value = f32::from(u16::try_from(index.0).unwrap_or(u16::MAX));
        for row in output.rows_mut() {
            row.fill(value);
        }
    }
}

#[test]
fn renders_each_image_in_order_from_the_first() {
    let mut image = Image::new(ImageSize::new(2, 1).expect("a valid size"));
    let three: Vec<(u64, Option<u32>)> = [5_u16, 6, 7]
        .iter()
        .map(|&index| (u64::from(index), Some(f32::from(index).to_bits())))
        .collect();
    for (count, expected) in [(3, three), (0, Vec::new())] {
        let mut seen = Vec::new();
        render_sequence(
            &mut Counter,
            ImageRange::new(ImageIndex(5), count).expect("a valid range"),
            &mut image,
            |index, rendered| {
                seen.push((
                    index.0,
                    rendered.values().first().map(|value| value.to_bits()),
                ));
            },
        );
        assert_eq!(
            seen, expected,
            "{count} images from 5, each rendered before it is handed"
        );
    }
}

#[test]
fn renders_the_last_index_once() {
    let mut image = Image::new(ImageSize::new(1, 1).expect("a valid size"));
    let mut seen = Vec::new();
    render_sequence(
        &mut Counter,
        ImageRange::new(ImageIndex(u64::MAX - 1), 2).expect("a valid range"),
        &mut image,
        |index, _| {
            seen.push(index.0);
        },
    );
    assert_eq!(
        seen,
        [u64::MAX - 1, u64::MAX],
        "each index once, the last included"
    );
}
