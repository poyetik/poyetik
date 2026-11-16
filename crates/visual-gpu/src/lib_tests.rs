//! Tests of the GPU renderer: the CPU reference's bits at every point of the visual test
//! matrix and at random squares, in bands of any height, and every way it can fail.

use core::cell::Cell;
use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll};

use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRate, ImageSize, Seconds, VisualBox};
use visual_testkit::generate::{rate_terms, seconds_terms, side, small_size, u64_edges, wide_size};
use visual_testkit::matrix::{image_indices, image_rates, image_sizes};
use visual_testkit::proptest::prelude::prop_oneof;
use visual_testkit::proptest::test_runner::TestCaseError;
use visual_testkit::runner::runner;

use super::{GpuError, GpuSquare, PIXEL_BYTES, block_on};

/// The GPU the tests render on: the first of any API.
fn gpu() -> GpuSquare {
    GpuSquare::new(wgpu::Backends::all()).expect("a GPU (a software one in CI)")
}

/// The square of these terms.
fn square(rate: ImageRate, side: u32, cycle: (u32, u32)) -> Square {
    Square::new(
        rate,
        side,
        Seconds::new(cycle.0, cycle.1).expect("a valid cycle"),
    )
    .expect("a valid side")
}

/// The first value whose bits differ between `gpu` and the CPU reference `cpu`, if any.
fn first_difference(gpu: &Image, cpu: &Image) -> Option<(usize, f32, f32)> {
    gpu.values()
        .iter()
        .zip(cpu.values())
        .position(|(ours, theirs)| ours.to_bits() != theirs.to_bits())
        .map(|at| (at, gpu.values()[at], cpu.values()[at]))
}

/// Renders image `index` of `visual` at `size` on `gpu` and on the CPU, and says where they
/// differ.
fn compare(
    gpu: &GpuSquare,
    visual: &Square,
    index: ImageIndex,
    size: ImageSize,
) -> Result<(), String> {
    let mut ours = Image::try_new(size)?;
    // Not the value any pixel renders: a band left unwritten shows.
    ours.rows_mut().flatten().for_each(|value| *value = -1.0);
    gpu.render(visual, index, &mut ours)
        .map_err(|error| error.to_string())?;
    let mut theirs = Image::try_new(size)?;
    visual.clone().render(index, &mut theirs);
    match first_difference(&ours, &theirs) {
        None => Ok(()),
        Some((at, gpu_value, cpu_value)) => Err(format!(
            "{visual:?} image {} at {}x{}: value {at} is {gpu_value:e} on the GPU, \
             {cpu_value:e} on the CPU",
            index.0,
            size.width(),
            size.height()
        )),
    }
}

#[test]
fn gives_the_cpu_references_bits_across_the_matrix() {
    // Every rate, size and index of the visual test matrix, for sides from none to the
    // full height and cycles from a hundredth of a second to a long odd one.
    let gpu = gpu();
    let mut images = 0_u32;
    for rate in image_rates() {
        for side in [0, 1, 39_322, Square::FULL_SIDE - 1, Square::FULL_SIDE] {
            for cycle in [(1, 100), (1001, 1000), (7919, 3)] {
                let visual = square(rate, side, cycle);
                for size in image_sizes() {
                    for index in image_indices() {
                        assert_eq!(compare(&gpu, &visual, index, size), Ok(()));
                        images += 1;
                    }
                }
            }
        }
    }
    assert_eq!(images, 7 * 5 * 3 * 6 * 6, "every image of the matrix");
}

#[test]
fn gives_the_cpu_references_bits_for_random_squares() {
    let gpu = gpu();
    let cases = (
        rate_terms(),
        side(),
        seconds_terms(),
        prop_oneof![small_size(), wide_size()],
        u64_edges(),
    );
    let verdict = runner(256).run(&cases, |(rate, side, cycle, (width, height), index)| {
        let rate = ImageRate::new(rate.0, rate.1).map_err(|_| TestCaseError::fail("a rate"))?;
        let size = ImageSize::new(width, height).map_err(|_| TestCaseError::fail("a size"))?;
        compare(&gpu, &square(rate, side, cycle), ImageIndex(index), size)
            .map_err(TestCaseError::fail)
    });
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

#[test]
fn renders_the_same_pixels_in_bands_of_any_height() {
    // 17 x 9 pixels, rendered in bands of 1, 2, 4 and 8 rows (the last band shorter), in
    // one band of exactly the image, and in one band larger than it.
    let size = ImageSize::new(17, 9).expect("a valid size");
    let visual = square(
        ImageRate::new(60, 1).expect("a valid rate"),
        39_322,
        (1001, 1000),
    );
    let row = 17 * PIXEL_BYTES;
    for rows in [1, 2, 4, 8, 9, 1000] {
        let mut banded = gpu();
        banded.band_bytes = row * rows;
        for index in image_indices() {
            assert_eq!(
                compare(&banded, &visual, index, size),
                Ok(()),
                "bands of {rows} rows"
            );
        }
    }
    // A band smaller than a row still renders one row at a time.
    let mut thin = gpu();
    thin.band_bytes = row - 1;
    assert_eq!(compare(&thin, &visual, ImageIndex(59), size), Ok(()));
}

#[test]
fn names_its_gpu() {
    let gpu = gpu();
    assert!(
        gpu.adapter().ends_with(')') && gpu.adapter().contains(" ("),
        "{}",
        gpu.adapter()
    );
}

#[test]
fn says_when_no_gpu_answers() {
    assert!(matches!(
        GpuSquare::new(wgpu::Backends::empty()),
        Err(GpuError::NoAdapter(_))
    ));
}

#[test]
fn says_when_the_gpu_refuses_a_device() {
    let refused = GpuSquare::open(wgpu::Backends::all(), |limits| wgpu::Limits {
        max_bind_groups: limits.max_bind_groups.saturating_add(1),
        ..limits
    });
    assert!(matches!(refused, Err(GpuError::NoDevice(_))), "{refused:?}");
}

#[test]
fn says_when_the_device_is_lost() {
    let gpu = gpu();
    gpu.device.destroy();
    let mut image =
        Image::try_new(ImageSize::new(4, 2).expect("a valid size")).expect("room for it");
    let visual = square(ImageRate::new(1, 1).expect("a valid rate"), 1, (1, 1));
    let lost = gpu.render(&visual, ImageIndex(0), &mut image);
    assert!(matches!(lost, Err(GpuError::Lost(_))), "{lost:?}");
}

#[test]
fn shows_each_error_in_one_line() {
    for (error, line) in [
        (
            GpuError::NoAdapter("none".to_owned()),
            "no GPU adapter: none",
        ),
        (
            GpuError::NoDevice("refused".to_owned()),
            "no GPU device: refused",
        ),
        (
            GpuError::Lost("destroyed".to_owned()),
            "the GPU did not give the image back: destroyed",
        ),
    ] {
        assert_eq!(error.to_string(), line);
    }
}

/// A future pending at its first poll and ready at the next.
struct PendingOnce<'cell>(&'cell Cell<u32>);

impl Future for PendingOnce<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        self.0.set(self.0.get().saturating_add(1));
        if self.0.get() == 1 {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

#[test]
fn waits_for_a_pending_future() {
    let polls = Cell::new(0);
    block_on(pin!(PendingOnce(&polls)));
    assert_eq!(polls.get(), 2, "polled again until ready, once only");
}
