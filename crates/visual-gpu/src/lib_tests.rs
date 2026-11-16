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

use super::{GpuError, GpuSquare, PIXEL_BYTES, RenderCost, block_on, render_cost};

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
                    // Every size of the matrix fits one band on every WebGPU device (a row
                    // or column of the longest side, 256 KiB): one submission an image.
                    assert_eq!(gpu.cost(size).bands, 1, "{size:?} in one band");
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
    let refused = GpuSquare::open(
        wgpu::Backends::all(),
        |limits| wgpu::Limits {
            max_bind_groups: limits.max_bind_groups.saturating_add(1),
            ..limits
        },
        true,
    );
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

/// The bytes of pixels a band holds on every WebGPU device: the smallest storage binding
/// size WebGPU guarantees, 128 MiB.
const WEBGPU_BAND: u64 = 128 << 20;

#[test]
fn costs_the_same_transfers_on_every_gpu_of_the_same_limits() {
    // The benchmarks' ladder, then the largest image, at the bands every WebGPU device
    // allows: the counts the benchmarks hold on every machine.
    let cost = |width, height| {
        render_cost(
            ImageSize::new(width, height).expect("a valid size"),
            WEBGPU_BAND,
        )
    };
    let expected = |bands, workgroups, read_back| RenderCost {
        bands,
        workgroups,
        uploaded: 32 * u64::from(bands),
        read_back,
    };
    assert_eq!(cost(64, 36), expected(1, 8 * 5, 64 * 36 * 16));
    assert_eq!(cost(320, 180), expected(1, 40 * 23, 320 * 180 * 16));
    assert_eq!(cost(640, 360), expected(1, 80 * 45, 640 * 360 * 16));
    assert_eq!(cost(1280, 720), expected(1, 160 * 90, 1280 * 720 * 16));
    // 512 rows of 256 KiB a band: four bands of 64 workgroups' rows.
    assert_eq!(
        cost(16_384, 2048),
        expected(4, 2048 * 64 * 4, 16_384 * 2048 * 16)
    );
    // Bands of 2 rows of a 17 x 9 image: four of 2 rows, then one of 1, each one row of
    // three workgroups.
    let small = render_cost(ImageSize::new(17, 9).expect("a valid size"), 17 * 16 * 2);
    assert_eq!(small, expected(5, 3 * 5, 17 * 9 * 16));
    // A band smaller than a row still holds one.
    assert_eq!(
        render_cost(ImageSize::new(17, 9).expect("a valid size"), 1),
        expected(9, 3 * 9, 17 * 9 * 16)
    );
    // The image's own GPU: the bands its buffers allow.
    let gpu = gpu();
    let size = ImageSize::new(1280, 720).expect("a valid size");
    assert_eq!(gpu.cost(size), render_cost(size, gpu.band_bytes));
    assert_eq!(PIXEL_BYTES, 16, "four 32-bit floats a pixel");
}

#[test]
fn times_its_work_by_the_gpus_clock_when_it_has_one() {
    let size = ImageSize::new(320, 180).expect("a valid size");
    let visual = square(
        ImageRate::new(60, 1).expect("a valid rate"),
        39_322,
        (1001, 1000),
    );
    let mut image = Image::try_new(size).expect("room for the image");
    let timed = gpu();
    let took = timed
        .timed_render(&visual, ImageIndex(7), &mut image)
        .expect("rendered");
    assert_eq!(
        took.is_some(),
        timed.timer.is_some(),
        "a time exactly when the GPU has a clock"
    );
    assert!(
        took.is_none_or(|nanoseconds| nanoseconds > 0.0_f64 && nanoseconds < 1e9_f64),
        "some work, well under a second: {took:?}"
    );
    let mut reference = Image::try_new(size).expect("room for the image");
    visual.clone().render(ImageIndex(7), &mut reference);
    assert_eq!(
        first_difference(&image, &reference),
        None,
        "timed, the same image"
    );
    let untimed = GpuSquare::open(wgpu::Backends::all(), |limits| limits, false)
        .expect("a GPU (a software one in CI)");
    assert_eq!(
        untimed.timed_render(&visual, ImageIndex(7), &mut image),
        Ok(None),
        "no time without its clock"
    );
    assert_eq!(first_difference(&image, &reference), None, "the same image");
    assert_eq!(
        super::nanoseconds(3, 2.5).to_bits(),
        7.5_f64.to_bits(),
        "ticks times the period"
    );
    assert_eq!(
        super::exact_f64((1 << 53) - 1).to_bits(),
        0x433F_FFFF_FFFF_FFFF_u64,
        "2^53 - 1, exact below 2^53"
    );
    assert_eq!(
        super::exact_f64(u64::MAX).to_bits(),
        0x43F0_0000_0000_0000_u64,
        "the nearest beyond: 2^64"
    );
}
