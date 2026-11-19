//! Tests of the GPU renderer: the CPU reference's bits at every point of the visual test
//! matrix and at random squares, in bands of any height, and every way it can fail.

use core::cell::Cell;
use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll};

use visual_boxes::Square;
use visual_core::{
    ALPHA8_THRESHOLDS, DisplayImage, Image, ImageIndex, ImageRate, ImageSize, SRGB8_THRESHOLDS,
    Seconds, VisualBox,
};
use visual_testkit::generate::{rate_terms, seconds_terms, side, small_size, u64_edges, wide_size};
use visual_testkit::matrix::{image_indices, image_rates, image_sizes};
use visual_testkit::proptest::prelude::Rng as _;
use visual_testkit::proptest::prelude::prop_oneof;
use visual_testkit::proptest::test_runner::{RngAlgorithm, TestCaseError, TestRng};
use visual_testkit::runner::runner;

use super::{GpuError, GpuSquare, PIXEL_BYTES, RenderCost, block_on, display_band, render_cost};

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
        (
            GpuError::Size(17, 9),
            "the display image is not of the size of the image it encodes, 17x9",
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

/// The checks run on a GPU that can time its own work: the timed paths, their coverage and
/// their mutants are reached only there. Lavapipe (CI) and Metal have the clock; a GPU
/// without it fails here, naming what it lacks, rather than as missing coverage or as
/// mutants that survive for want of a clock.
#[test]
fn runs_the_checks_on_a_gpu_with_a_clock() {
    assert!(
        gpu().timer.is_some(),
        "the checks need a GPU with timestamp queries (wgpu::Features::TIMESTAMP_QUERY)"
    );
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

/// The float just below the finite float `value` of positive sign.
fn below(value: f32) -> f32 {
    f32::from_bits(value.to_bits().saturating_sub(1))
}

/// The float just above the finite float `value` of positive sign.
fn above(value: f32) -> f32 {
    f32::from_bits(value.to_bits().saturating_add(1))
}

/// An image holding `pixels` row by row, padded with transparent black: as many rows of 64
/// pixels as they need.
fn image_of(pixels: &[[f32; 4]]) -> Image {
    let rows = u32::try_from(pixels.len().div_ceil(64)).expect("a few rows");
    let mut image =
        Image::try_new(ImageSize::new(64, rows.max(1)).expect("a valid size")).expect("room");
    for (slot, value) in image.rows_mut().flatten().zip(
        pixels
            .iter()
            .flatten()
            .copied()
            .chain(core::iter::repeat(0.0)),
    ) {
        *slot = value;
    }
    image
}

/// The first pixel whose display bytes differ between `gpu`'s encoding of `image` and the
/// CPU's, if any: its index, its values and both pixels.
fn first_encoding_difference(gpu: &GpuSquare, image: &Image) -> Option<String> {
    let mut ours = DisplayImage::try_new(image.size()).expect("room");
    ours.bytes_mut().fill(0xA5);
    gpu.encode(image, &mut ours).expect("encoded");
    let mut theirs = DisplayImage::try_new(image.size()).expect("room");
    theirs.encode(image).expect("the same size");
    ours.bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(theirs.bytes().as_chunks::<4>().0)
        .zip(image.values().as_chunks::<4>().0)
        .position(|((gpu_pixel, cpu_pixel), _)| gpu_pixel != cpu_pixel)
        .map(|at| {
            let values = image.values().as_chunks::<4>().0.get(at).copied();
            let gpu_pixel = ours.bytes().as_chunks::<4>().0.get(at).copied();
            let cpu_pixel = theirs.bytes().as_chunks::<4>().0.get(at).copied();
            format!("pixel {at} {values:?}: {gpu_pixel:?} on the GPU, {cpu_pixel:?} on the CPU")
        })
}

#[test]
fn encodes_for_display_as_the_cpu_at_every_threshold() {
    // Every sRGB threshold over alphas from the faintest to above 1, the colors nearest the
    // exact product and their neighbours; every alpha threshold and its neighbours.
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3E00_0001),
        f32::from_bits(0x3EAA_AAAB),
        0.5,
        0.75,
        below(1.0),
        1.0,
        1.5,
    ];
    let mut pixels = Vec::new();
    for &threshold in &SRGB8_THRESHOLDS {
        for alpha in alphas {
            let nearest = threshold * alpha;
            for color in [below(nearest), nearest, above(nearest)] {
                pixels.push([color, below(color), above(color), alpha]);
            }
        }
    }
    for &threshold in &ALPHA8_THRESHOLDS {
        for alpha in [below(threshold), threshold, above(threshold)] {
            pixels.push([0.5 * alpha, alpha, 0.25 * alpha, alpha]);
        }
    }
    assert_eq!(first_encoding_difference(&gpu(), &image_of(&pixels)), None);
}

#[test]
fn encodes_for_display_as_the_cpu_whatever_the_values() {
    // Not a number, infinities, zeros of both signs, subnormals, negatives and values above
    // 1, in every place of a pixel.
    let specials = [
        f32::NAN,
        -f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0,
        -0.0,
        f32::from_bits(1),
        f32::from_bits(0x007F_FFFF),
        f32::MIN_POSITIVE,
        -0.5,
        0.5,
        1.0,
        2.0,
        f32::MAX,
    ];
    let mut pixels = Vec::new();
    for &color in &specials {
        for &alpha in &specials {
            pixels.push([color, alpha, color, alpha]);
            pixels.push([alpha, color, 0.5, alpha]);
        }
    }
    assert_eq!(first_encoding_difference(&gpu(), &image_of(&pixels)), None);
    // Random bits, half of them among the values an image holds (0 to 2), half any at all:
    // 2^16 pixels of a fixed sequence.
    let mut rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let random: Vec<[f32; 4]> = (0..1 << 16_u32)
        .map(|at: u32| {
            [0_u8; 4].map(|_| {
                let bits = rng.next_u32();
                f32::from_bits(if at.is_multiple_of(2) {
                    bits % 0x4000_0001
                } else {
                    bits
                })
            })
        })
        .collect();
    assert_eq!(first_encoding_difference(&gpu(), &image_of(&random)), None);
}

#[test]
fn encodes_the_square_it_renders_as_the_cpu_across_the_matrix() {
    let gpu = gpu();
    for size in image_sizes() {
        let visual = square(
            ImageRate::new(60, 1).expect("a valid rate"),
            39_322,
            (1001, 1000),
        );
        for index in image_indices() {
            let mut image = Image::try_new(size).expect("room");
            gpu.render(&visual, index, &mut image).expect("rendered");
            assert_eq!(
                first_encoding_difference(&gpu, &image),
                None,
                "{size:?} image {index:?}"
            );
        }
    }
}

#[test]
fn encodes_in_bands_of_any_size() {
    let pixels: Vec<[f32; 4]> = (0..300_u16)
        .map(|at| {
            let value = f32::from(at) / 300.0;
            [value * 0.5, value * 0.25, value, value]
        })
        .collect();
    let image = image_of(&pixels);
    for band_pixels in [1, 5, 64, 299, 300, 10_000] {
        let mut banded = gpu();
        banded.band_bytes = band_pixels * PIXEL_BYTES;
        assert_eq!(
            first_encoding_difference(&banded, &image),
            None,
            "bands of {band_pixels} pixels"
        );
    }
}

#[test]
fn says_when_the_device_is_lost_while_encoding() {
    let gpu = gpu();
    gpu.device.destroy();
    let image = Image::try_new(ImageSize::new(4, 2).expect("a valid size")).expect("room for it");
    let mut display = DisplayImage::try_new(image.size()).expect("room");
    let lost = gpu.encode(&image, &mut display);
    assert!(matches!(lost, Err(GpuError::Lost(_))), "{lost:?}");
}

#[test]
fn refuses_a_display_image_of_another_size() {
    let image = Image::try_new(ImageSize::new(4, 2).expect("a valid size")).expect("room");
    let mut display =
        DisplayImage::try_new(ImageSize::new(2, 4).expect("a valid size")).expect("room");
    assert_eq!(
        gpu().encode(&image, &mut display),
        Err(GpuError::Size(4, 2))
    );
    assert!(display.bytes().iter().all(|&byte| byte == 0), "untouched");
}

#[test]
fn encodes_in_bands_of_as_many_pixels_as_fit() {
    // 16 bytes of values a pixel: as many as fit, at least one, at most the image's.
    assert_eq!(display_band(1000, 16 * 300), 300, "300 fit");
    assert_eq!(display_band(1000, 16 * 300 + 15), 300, "whole pixels only");
    assert_eq!(display_band(1000, 15), 1, "one at least");
    assert_eq!(display_band(1000, 16 << 20), 1000, "the image's at most");
    assert_eq!(display_band(1, 0), 1, "one, the image's");
}

#[test]
fn validates_the_library_s_shaders() {
    // naga, the WGSL front end wgpu compiles with, parses and validates each one without a
    // GPU, with WebGPU's core capabilities: a shader error is its message, with the line,
    // before any pipeline is created. Every shader xtask's loaded-files gate accepts
    // (`VALIDATED_SHADERS`) is one of these.
    for (name, source) in [
        (
            "crates/visual-boxes/src/square.wgsl",
            visual_boxes::SQUARE_WGSL,
        ),
        (
            "crates/visual-core/src/display.wgsl",
            visual_core::DISPLAY_WGSL,
        ),
    ] {
        let module = wgpu::naga::front::wgsl::parse_str(source)
            .map_err(|error| error.emit_to_string_with_path(source, name));
        let validated = module.and_then(|parsed| {
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::default(),
            )
            .validate(&parsed)
            .map(drop)
            .map_err(|error| error.emit_to_string_with_path(source, name))
        });
        assert_eq!(validated, Ok(()), "{name}");
    }
}
