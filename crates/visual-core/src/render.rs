//! The exact CPU reference renderer: a sequence of images, one after the other.

use crate::{Image, ImageIndex, VisualBox};

/// Renders `count` images of `visual` from image `first`, each into `image`, and hands
/// each one to `each` with its index before rendering the next.
///
/// The reference every other renderer is measured against: exact, deterministic, the same
/// bits on every target. It allocates nothing: `image` holds every value.
pub fn render_sequence(
    visual: &mut dyn VisualBox,
    first: ImageIndex,
    count: u64,
    image: &mut Image,
    mut each: impl FnMut(ImageIndex, &Image),
) {
    for offset in 0..count {
        let index = ImageIndex(first.0.saturating_add(offset));
        visual.render(index, image);
        each(index, image);
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec::Vec;

    use super::render_sequence;
    use crate::{Image, ImageIndex, ImageSize, VisualBox};

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
                ImageIndex(5),
                count,
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
    fn stops_at_the_last_index() {
        let mut image = Image::new(ImageSize::new(1, 1).expect("a valid size"));
        let mut seen = Vec::new();
        render_sequence(
            &mut Counter,
            ImageIndex(u64::MAX),
            2,
            &mut image,
            |index, _| {
                seen.push(index.0);
            },
        );
        assert_eq!(seen, [u64::MAX, u64::MAX], "saturates instead of wrapping");
    }
}
