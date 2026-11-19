//! The contract every visual box implements.

use crate::{Image, ImageIndex, ImageSize};

/// A point of an image of a given size, in subpixel units from its top left corner, always
/// inside it: built by [`ImageSize::point`] only.
///
/// [`crate::SUBPIXELS`] units per pixel, `x` to the right, `y` down. The pixel `(i, j)` covers
/// `[i × SUBPIXELS, (i + 1) × SUBPIXELS) × [j × SUBPIXELS, (j + 1) × SUBPIXELS)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubpixelPoint {
    /// The size of the image the point is in.
    pub(crate) size: ImageSize,
    /// From the left edge, below width × `SUBPIXELS`.
    pub(crate) x: u64,
    /// From the top edge, below height × `SUBPIXELS`.
    pub(crate) y: u64,
}

impl SubpixelPoint {
    /// The size of the image the point is in.
    #[must_use]
    pub const fn size(self) -> ImageSize {
        self.size
    }

    /// From the left edge, in subpixel units: below width × [`crate::SUBPIXELS`].
    #[must_use]
    pub const fn x(self) -> u64 {
        self.x
    }

    /// From the top edge, in subpixel units: below height × [`crate::SUBPIXELS`].
    #[must_use]
    pub const fn y(self) -> u64 {
        self.y
    }
}

/// What produced a point of a box's output: the answer of [`VisualBox::trace`], with
/// which a gesture on the output finds what it touched (picking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Trace {
    /// The box does not declare it: a gesture cannot be traced through this box, and a
    /// caller can report it (the editor's picking, #35 step 15).
    NotDeclared,
    /// Nothing the box draws: the background.
    Background,
    /// A shape the box draws, numbered by the box from 0.
    Shape(u32),
}

/// A visual box: renders one image at a time.
///
/// This first version covers boxes with no image input (generators); inputs come with
/// the design decisions of the visual engine (roadmap #35 step 2).
pub trait VisualBox: Send + core::fmt::Debug {
    /// Renders image `index` into `output`, overwriting every value.
    ///
    /// The result depends only on `index`, the box's parameters and the size of
    /// `output`: any image can be rendered first (random access), and rendering it twice
    /// gives the same bits. Must not allocate or free memory, lock or block.
    ///
    /// Every value written is finite, never subnormal and never -0 (0 is written instead,
    /// so that a target flushing subnormals to zero, as GPUs do, cannot change a bit), and
    /// every alpha lies within `[0, 1]`, the colors premultiplied by it
    /// (`spec/visual-design.md`, sections 3 and 4): `visual-testkit`'s law of finite values
    /// checks every box against it.
    fn render(&mut self, index: ImageIndex, output: &mut Image);

    /// What produced `point` of image `index`, for an output of the point's size: the
    /// optional mapping from output points back to what produced them. Exact, computed
    /// on the CPU only when a gesture asks, so it costs nothing otherwise, and needing no
    /// image: only its size, which the point carries. A point is always inside its image
    /// ([`ImageSize::point`]).
    ///
    /// A box that does not implement it answers [`Trace::NotDeclared`].
    fn trace(&self, _index: ImageIndex, _point: SubpixelPoint) -> Trace {
        Trace::NotDeclared
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
