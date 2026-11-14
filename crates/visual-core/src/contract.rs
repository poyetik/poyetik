//! The contract every visual box implements.

use crate::{Image, ImageIndex};

/// A point of an image, in subpixel units from its top left corner.
///
/// [`crate::SUBPIXELS`] units per pixel, `x` to the right, `y` down. The pixel `(i, j)` covers
/// `[i × SUBPIXELS, (i + 1) × SUBPIXELS) × [j × SUBPIXELS, (j + 1) × SUBPIXELS)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubpixelPoint {
    /// From the left edge.
    pub x: u64,
    /// From the top edge.
    pub y: u64,
}

/// What produced a point of a box's output: the answer of [`VisualBox::trace`], with
/// which a gesture on the output finds what it touched (picking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trace {
    /// The box does not declare it: a gesture cannot be traced through this box, and
    /// the editor reports it.
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
    fn render(&mut self, index: ImageIndex, output: &mut Image);

    /// What produced `point` of image `index`, for an output of `output`'s size: the
    /// optional mapping from output points back to what produced them. Exact, computed
    /// on the CPU only when a gesture asks, so it costs nothing otherwise.
    ///
    /// A box that does not implement it answers [`Trace::NotDeclared`].
    fn trace(&self, _index: ImageIndex, _output: &Image, _point: SubpixelPoint) -> Trace {
        Trace::NotDeclared
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
