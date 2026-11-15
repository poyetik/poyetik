//! Tests of `contract.rs`.

use super::{Trace, VisualBox};
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
        blank.trace(ImageIndex(3), image.size().point(1, 2).expect("inside")),
        Trace::NotDeclared,
        "default"
    );
}

#[test]
fn a_point_is_always_inside_its_image() {
    let size = ImageSize::new(3, 2).expect("a valid size");
    let unit = u64::from(crate::SUBPIXELS);
    for (x, y) in [(0, 0), (3 * unit - 1, 2 * unit - 1), (unit, 1)] {
        let point = size.point(x, y).expect("inside");
        assert_eq!(
            (point.x(), point.y(), point.size()),
            (x, y, size),
            "({x}, {y})"
        );
    }
    for (x, y) in [
        (3 * unit, 0),
        (0, 2 * unit),
        (u64::MAX, u64::MAX),
        (3 * unit, 2 * unit),
    ] {
        assert_eq!(size.point(x, y), None, "({x}, {y}): outside");
    }
    let widest = ImageSize::new(ImageSize::MAX_SIDE, 1).expect("a valid size");
    assert!(
        widest
            .point(u64::from(ImageSize::MAX_SIDE) * unit - 1, unit - 1)
            .is_some(),
        "the last subpixel of the widest row"
    );
}
