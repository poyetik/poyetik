//! The exact CPU reference renderer: a sequence of images, one after the other.

use crate::{Image, ImageIndex, ImageRange, VisualBox};

/// Renders the images of `range`, in order, each into `image`, and hands each one to
/// `each` with its index before rendering the next.
///
/// The reference every other renderer is measured against: exact, deterministic, the same
/// bits on every target. It allocates nothing: `image` holds every value.
pub fn render_sequence(
    visual: &mut dyn VisualBox,
    range: ImageRange,
    image: &mut Image,
    mut each: impl FnMut(ImageIndex, &Image),
) {
    for index in range.indices() {
        visual.render(index, image);
        each(index, image);
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
