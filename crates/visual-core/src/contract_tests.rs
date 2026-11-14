//! Tests of `contract.rs`.

use super::{SubpixelPoint, Trace, VisualBox};
use crate::{Image, ImageIndex, ImageSize};

/// A box that keeps the default trace.
#[derive(Debug)]
struct Blank;

impl VisualBox for Blank {
    fn render(&mut self, _index: ImageIndex, output: &mut Image) {
        for row in output.rows_mut() {
            row.fill(0.0);
        }
    }
}

#[test]
fn a_box_traces_nothing_unless_it_declares_it() {
    let mut blank = Blank;
    let mut image = Image::new(ImageSize::new(2, 2).expect("a valid size"));
    blank.render(ImageIndex(0), &mut image);
    assert_eq!(
        blank.trace(ImageIndex(3), &image, SubpixelPoint { x: 1, y: 2 }),
        Trace::NotDeclared,
        "default"
    );
}
