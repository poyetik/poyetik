//! What the repository's visual conformance corpus reaches of the square's arithmetic.
//!
//! The corpus is the only test of the renderers that only replay it (the WebAssembly
//! modules, the host modes): a value of the arithmetic no case reaches is a mistake no
//! hash can see there. Each case's images are placed by [`Square::placement`], the
//! placement every renderer draws, and the coverages its pixels take are computed here,
//! independently of the renderer.

#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use visual_boxes::Placement;
    use visual_cli::{PreparedCase, parse_case};
    use visual_core::{ImageSize, SUBPIXELS};

    /// The largest coverage of a pixel, a whole one: [`SUBPIXELS`] squared, 2²⁴ subpixel
    /// units of area.
    const FULL_PIXEL: u32 = SUBPIXELS * SUBPIXELS;

    /// The lowest coverage a 32-bit float holds with no bit to spare: from 2²³ on, an odd
    /// coverage is its last bit.
    const FLOAT_ODD_FROM: u32 = 1 << (f32::MANTISSA_DIGITS - 1);

    const _: () = assert!(
        FULL_PIXEL == FLOAT_ODD_FROM << 1,
        "a whole pixel, 2²⁴, is the lowest coverage past the odd ones a float holds"
    );

    /// The sub-pixel bits of an edge: [`SUBPIXELS`] is 2¹², a power of two.
    const SUBPIXEL_BITS: u32 = {
        assert!(SUBPIXELS.is_power_of_two(), "whole bits of a pixel");
        SUBPIXELS.trailing_zeros()
    };

    /// The repository's visual conformance cases.
    fn cases() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/visual/cases")
    }

    /// Every case of the corpus, read from its file as the renderer reads it, with the
    /// file's path.
    fn corpus() -> Vec<(String, PreparedCase)> {
        let mut read = Vec::new();
        for entry in std::fs::read_dir(cases()).expect("the cases") {
            let path = entry.expect("an entry").path();
            // Only case files: a file manager's `.DS_Store`, which git ignores, is none.
            if path.extension() != Some(std::ffi::OsStr::new("json")) {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a case");
            let case = parse_case(&text).expect("a valid case");
            read.push((path.display().to_string(), case));
        }
        read
    }

    /// The lengths, in subpixel units, of the overlaps of `[start, end)` with the cells of
    /// a side of `cells` pixels: every distinct one some pixel of that side takes.
    fn overlaps(start: u32, end: u32, cells: u32) -> Vec<u32> {
        let unit = SUBPIXELS;
        let first = start / unit;
        let last = end.saturating_sub(1) / unit;
        // The cells of the edges, one inside them, and one on either side.
        let near = [
            first.saturating_sub(1),
            first,
            first.saturating_add(1),
            last.saturating_sub(1),
            last,
            last.saturating_add(1),
        ];
        let mut lengths: Vec<u32> = near
            .into_iter()
            .filter(|&cell| cell < cells)
            .map(|cell| {
                // At most 16 384 × 4096 = 2²⁶: never saturates.
                let cell_start = cell.saturating_mul(unit);
                let low = start.max(cell_start);
                let high = end.min(cell_start.saturating_add(unit));
                high.saturating_sub(low)
            })
            .collect();
        // Any other cell is outside the square, or wholly inside it, between its edges'
        // cells; both values are among those of the cells above whenever a cell takes them.
        lengths.sort_unstable();
        lengths.dedup();
        lengths
    }

    /// What the corpus reaches.
    #[derive(Default)]
    struct Reach {
        /// Every coverage a pixel takes, in subpixel units of area.
        coverages: alloc::collections::BTreeSet<u32>,
        /// Whether some square's side is an odd number of subpixel units.
        odd_length: bool,
        /// For each edge (left, right, top, bottom), the bits of the sub-pixel position
        /// seen set.
        set: [u32; 4],
        /// As `set`, the bits seen clear.
        clear: [u32; 4],
    }

    impl Reach {
        /// Adds one image's placement, of an image of `size`.
        fn add(&mut self, placed: Placement, size: ImageSize) {
            let edges = [placed.left(), placed.right(), placed.top(), placed.bottom()];
            let mask = (1 << SUBPIXEL_BITS) - 1;
            for (at, edge) in edges.into_iter().enumerate() {
                self.set[at] |= edge & mask;
                self.clear[at] |= !edge & mask;
            }
            if placed.right().saturating_sub(placed.left()) % 2 == 1 {
                self.odd_length = true;
            }
            let rows = overlaps(placed.top(), placed.bottom(), size.height());
            for horizontal in overlaps(placed.left(), placed.right(), size.width()) {
                for vertical in &rows {
                    let _new = self.coverages.insert(horizontal.saturating_mul(*vertical));
                }
            }
        }
    }

    /// Everything the corpus reaches, over every image of every case.
    fn reach() -> Reach {
        let mut reach = Reach::default();
        let corpus = corpus();
        assert!(!corpus.is_empty(), "the corpus has cases");
        for (name, case) in corpus {
            assert!(case.range().count() > 0, "{name} renders images");
            for index in case.range().indices() {
                reach.add(case.square().placement(index, case.size()), case.size());
            }
        }
        reach
    }

    #[test]
    fn measures_the_overlaps_of_a_side_independently() {
        // A square from 1.5 to 3.25 pixels on a side of 5: half of pixel 1, all of pixel 2,
        // a quarter of pixel 3, nothing of pixels 0 and 4.
        let unit = SUBPIXELS;
        assert_eq!(
            overlaps(unit + unit / 2, 3 * unit + unit / 4, 5),
            [0, unit / 4, unit / 2, unit]
        );
        // One wider than its side of 2, clipped: both pixels whole.
        assert_eq!(overlaps(0, 3 * unit, 2), [unit]);
        // Within one pixel: its length, and nothing beside it.
        assert_eq!(overlaps(unit + 3, unit + 8, 3), [0, 5]);
    }

    #[test]
    fn reaches_every_bit_of_the_coverage_and_of_each_edge() {
        let reach = reach();
        let odd = |coverage: &&u32| *coverage % 2 == 1;
        assert!(
            reach
                .coverages
                .iter()
                .filter(odd)
                .any(|&coverage| coverage < FLOAT_ODD_FROM),
            "an odd coverage below 2^23"
        );
        assert!(
            reach
                .coverages
                .iter()
                .filter(odd)
                .any(|&coverage| coverage >= FLOAT_ODD_FROM),
            "an odd coverage of 2^23 or more"
        );
        assert!(
            reach
                .coverages
                .iter()
                .all(|&coverage| coverage <= FULL_PIXEL),
            "no coverage above a whole pixel"
        );
        let unset: Vec<u32> = (0..=FULL_PIXEL.trailing_zeros())
            .filter(|bit| {
                reach
                    .coverages
                    .iter()
                    .all(|coverage| coverage >> bit & 1 == 0)
            })
            .collect();
        assert!(unset.is_empty(), "coverage bits never set: {unset:?}");
        assert!(
            reach.odd_length,
            "a side of an odd number of subpixel units"
        );
        let mask = (1 << SUBPIXEL_BITS) - 1;
        for (at, edge) in ["left", "right", "top", "bottom"].into_iter().enumerate() {
            assert_eq!(
                reach.set[at], mask,
                "every sub-pixel bit of the {edge} edge set"
            );
            assert_eq!(
                reach.clear[at], mask,
                "every sub-pixel bit of the {edge} edge clear"
            );
        }
    }
}
