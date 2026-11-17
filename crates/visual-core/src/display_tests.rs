//! Tests of `display.rs`: every threshold proven in exact rational arithmetic (no float
//! function, whose rounding the thresholds exist to avoid), the codes at the edges of their
//! domain, and the display image's allocation and encoding.

extern crate alloc;

use alloc::collections::TryReserveError;
use alloc::vec;
use alloc::vec::Vec;

use super::{ALPHA8_THRESHOLDS, DisplayImage, SRGB8_THRESHOLDS, alpha8, pixel8, srgb8};
use crate::{Image, ImageSize, VisualError};

/// Counts heap operations in the tests, to prove encoding never allocates.
#[global_allocator]
static ALLOCATOR: visual_testkit::CountingAllocator = visual_testkit::CountingAllocator;

/// A natural number of any size: its 64-bit limbs, least significant first.
#[derive(Debug, Clone)]
struct Natural(Vec<u64>);

impl Natural {
    /// The number one.
    fn one() -> Self {
        Self(vec![1])
    }

    /// It times `factor`.
    fn times(&self, factor: u64) -> Self {
        let mut carry = 0_u64;
        let mut limbs: Vec<u64> = self
            .0
            .iter()
            .map(|&limb| {
                // limb × factor + carry < 2^128: no overflow.
                let product = u128::from(limb)
                    .checked_mul(u128::from(factor))
                    .and_then(|product| product.checked_add(u128::from(carry)))
                    .expect("below 2^128");
                let [low, high] = [product, product >> 64_u32]
                    .map(|half| u64::try_from(half & u128::from(u64::MAX)).expect("a limb"));
                carry = high;
                low
            })
            .collect();
        limbs.push(carry);
        Self(limbs)
    }

    /// It times `factor` raised to `power`.
    fn times_power(&self, factor: u64, power: u32) -> Self {
        (0..power).fold(self.clone(), |product, _| product.times(factor))
    }

    /// Its limbs from the most significant, leading zeros dropped.
    fn significant(&self) -> Vec<u64> {
        self.0
            .iter()
            .rev()
            .copied()
            .skip_while(|&limb| limb == 0)
            .collect()
    }

    /// Whether it is at least `other`.
    fn at_least(&self, other: &Self) -> bool {
        let (mine, theirs) = (self.significant(), other.significant());
        (mine.len(), mine) >= (theirs.len(), theirs)
    }
}

/// A positive normal float below 1 as `(mantissa, shift)`, with the float equal to
/// `mantissa / 2^shift` exactly.
fn rational(value: f32) -> (u64, u32) {
    let bits = value.to_bits();
    let exponent = (bits >> 23_u32) & 0xFF;
    assert!(
        exponent > 0 && exponent < 127 && bits >> 31_u32 == 0,
        "{value:e}: positive, normal, below 1"
    );
    // value = (2^23 + fraction) × 2^(exponent − 150), the exponent below 127.
    let shift = 150_u32.checked_sub(exponent).expect("exponent below 150");
    ((1 << 23_u32) | u64::from(bits & 0x7F_FFFF), shift)
}

/// `left × right`, checked.
fn product(left: u128, right: u128) -> u128 {
    left.checked_mul(right).expect("no overflow")
}

/// Whether the sRGB encoding of `value` (IEC 61966-2-1) is at least `(2 code + 1) / 510`,
/// the middle between `code` and the code after it, in exact rational arithmetic.
fn encodes_to_at_least(value: f32, code: u64) -> bool {
    let (mantissa, shift) = rational(value);
    let odd = code
        .checked_mul(2)
        .and_then(|twice| twice.checked_add(1))
        .expect("small");
    let power = 1_u128.checked_shl(shift).expect("a shift below 128");
    // value ≤ 0.003 130 8, i.e. mantissa × 10⁷ ≤ 31 308 × 2^shift: the linear segment,
    // 12.92 value ≥ odd / 510, i.e. 1292 × 510 × mantissa ≥ 100 × odd × 2^shift.
    if product(u128::from(mantissa), 10_000_000) <= product(31_308, power) {
        return product(u128::from(mantissa), 1292 * 510)
            >= product(product(u128::from(odd), 100), power);
    }
    // 1.055 value^(1/2.4) − 0.055 ≥ odd / 510, i.e. value⁵ ≥ (N / D)¹² with
    // N = 200 odd + 5610 and D = 510 × 211: mantissa⁵ × D¹² ≥ N¹² × 2^(5 shift).
    let numerator = odd
        .checked_mul(200)
        .and_then(|scaled| scaled.checked_add(5610))
        .expect("small");
    let left = Natural::one()
        .times_power(mantissa, 5)
        .times_power(510 * 211, 12);
    let right = Natural::one()
        .times_power(numerator, 12)
        .times_power(2, shift.checked_mul(5).expect("small"));
    left.at_least(&right)
}

/// Whether `value` is at least `(2 code + 1) / 510`, exactly.
fn at_least_half_code(value: f32, code: u64) -> bool {
    let (mantissa, shift) = rational(value);
    let odd = code
        .checked_mul(2)
        .and_then(|twice| twice.checked_add(1))
        .expect("small");
    product(u128::from(mantissa), 510)
        >= product(
            u128::from(odd),
            1_u128.checked_shl(shift).expect("a shift below 128"),
        )
}

/// The float just below the positive normal float `value`.
fn below(value: f32) -> f32 {
    f32::from_bits(value.to_bits().checked_sub(1).expect("positive"))
}

#[test]
fn holds_the_smallest_float_reaching_each_srgb_code() {
    for (code, &threshold) in (0_u64..).zip(SRGB8_THRESHOLDS.iter()) {
        assert!(
            encodes_to_at_least(threshold, code),
            "after code {code}: {threshold:e} reaches the next"
        );
        assert!(
            !encodes_to_at_least(below(threshold), code),
            "after code {code}: the float below {threshold:e} does not"
        );
    }
}

#[test]
fn holds_the_smallest_float_reaching_each_alpha_code() {
    for (code, &threshold) in (0_u64..).zip(ALPHA8_THRESHOLDS.iter()) {
        assert!(
            at_least_half_code(threshold, code),
            "after alpha {code}: {threshold:e} reaches the next"
        );
        assert!(
            !at_least_half_code(below(threshold), code),
            "after alpha {code}: the float below {threshold:e} does not"
        );
    }
}

#[test]
fn encodes_known_values() {
    // Known answers of the sRGB encoding (IEC 61966-2-1), rounded to 8 bits: the middle
    // grey, 18% grey, the end of the linear segment, and the extremes.
    for (linear, code) in [
        (0.0_f32, 0_u8),
        (0.5, 188),
        (0.18, 118),
        (0.214_041_14, 128),
        (0.003_130_8, 10),
        (1.0, 255),
    ] {
        assert_eq!(srgb8(linear), code, "{linear}");
    }
    for (value, code) in [
        (0.0_f32, 0_u8),
        (0.5, 128),
        (1.0, 255),
        (0.25, 64),
        (0.75, 191),
    ] {
        assert_eq!(alpha8(value), code, "alpha {value}");
    }
}

#[test]
fn encodes_each_threshold_to_its_code_and_the_float_below_to_the_one_before() {
    for (code, (&srgb, &alpha)) in
        (1..=255_u8).zip(SRGB8_THRESHOLDS.iter().zip(ALPHA8_THRESHOLDS.iter()))
    {
        assert_eq!(
            (srgb8(srgb), srgb8(below(srgb))),
            (code, code - 1),
            "sRGB code {code}"
        );
        assert_eq!(
            (alpha8(alpha), alpha8(below(alpha))),
            (code, code - 1),
            "alpha {code}"
        );
    }
}

#[test]
fn clamps_every_value_outside_the_codes() {
    for value in [
        -0.0_f32,
        -1.0,
        f32::MIN,
        f32::from_bits(1),
        f32::NAN,
        f32::NEG_INFINITY,
    ] {
        assert_eq!((srgb8(value), alpha8(value)), (0, 0), "{value:e}: 0");
    }
    for value in [1.0_f32, 1.000_001, 2.0, f32::MAX, f32::INFINITY] {
        assert_eq!((srgb8(value), alpha8(value)), (255, 255), "{value:e}: 255");
    }
}

#[test]
fn encodes_a_pixel_with_straight_alpha() {
    assert_eq!(
        pixel8(1.0, 0.5, 0.0, 1.0),
        [255, 188, 0, 255],
        "opaque: the colors as they are"
    );
    assert_eq!(
        pixel8(0.25, 0.5, 0.0, 0.5),
        [188, 255, 0, 128],
        "half: each color divided by alpha"
    );
    assert_eq!(
        pixel8(0.3, 0.3, 0.3, 0.0),
        [0; 4],
        "alpha 0: transparent black"
    );
    assert_eq!(
        pixel8(0.3, 0.3, 0.3, -1.0),
        [0; 4],
        "alpha below 0: transparent black"
    );
    assert_eq!(
        pixel8(0.001, 0.001, 0.001, 0.001),
        [0; 4],
        "alpha encoding to 0: transparent black, whatever its colors"
    );
    assert_eq!(
        pixel8(0.002, 0.0, 0.002, 0.002),
        [255, 0, 255, 1],
        "the faintest alpha keeps its colors"
    );
}

/// A reservation that fails as the allocator does when the machine has no room left.
fn refused(bytes: &mut Vec<u8>, _: usize) -> Result<(), TryReserveError> {
    bytes.try_reserve_exact(usize::MAX)
}

/// A reservation that succeeds only for the bytes of a 1920 x 1080 image, exactly.
fn exact(bytes: &mut Vec<u8>, length: usize) -> Result<(), TryReserveError> {
    bytes.try_reserve_exact(if length == 1920 * 1080 * 4 {
        length
    } else {
        usize::MAX
    })
}

#[test]
fn refuses_a_display_image_the_memory_cannot_hold_naming_its_size() {
    let size = ImageSize::new(1920, 1080).expect("a valid size");
    assert_eq!(
        DisplayImage::allocate(size, refused).err(),
        Some(VisualError::OutOfMemory {
            width: 1920,
            height: 1080,
            bytes: 1920 * 1080 * 4
        })
    );
    assert_eq!(
        usize::try_from(DisplayImage::PIXEL_BYTES),
        Ok(DisplayImage::CHANNELS),
        "a byte a channel"
    );
    let display = DisplayImage::allocate(size, exact).expect("room for it");
    assert_eq!(
        (display.size(), display.bytes().len()),
        (size, 1920 * 1080 * 4),
        "the bytes of the size, exactly"
    );
    assert!(
        display.bytes().iter().all(|&byte| byte == 0),
        "transparent black"
    );
    let largest = ImageSize::new(16_384, 2048).expect("the largest size");
    assert_eq!(
        DisplayImage::try_new(largest).map(|largest_display| largest_display.bytes().len()),
        Ok(16_384 * 2048 * 4),
        "the largest, 128 MiB"
    );
}

#[test]
fn encodes_every_pixel_of_an_image_in_order_without_allocating() {
    let size = ImageSize::new(3, 2).expect("a valid size");
    let mut image = Image::try_new(size).expect("room for it");
    let pixels: [[f32; 4]; 6] = [
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        [0.5, 0.18, 0.003_130_8, 1.0],
        [0.25, 0.5, 0.0, 0.5],
        [0.3, 0.3, 0.3, 0.0],
        [1.0, 0.0, 0.0, 1.0],
    ];
    for (value, given) in image.rows_mut().flatten().zip(pixels.as_flattened()) {
        *value = *given;
    }
    let mut display = DisplayImage::try_new(size).expect("room for it");
    let (encoded, operations) =
        visual_testkit::heap_operations_during(|| display.encode(&image)).expect("counting");
    assert_eq!(
        (encoded, operations),
        (Ok(()), 0),
        "encoded, nothing allocated"
    );
    assert_eq!(
        display.bytes(),
        [
            0, 0, 0, 255, //
            255, 255, 255, 255, //
            188, 118, 10, 255, //
            188, 255, 0, 128, //
            0, 0, 0, 0, //
            255, 0, 0, 255,
        ],
        "row by row from the top, each pixel by pixel8"
    );
}

#[test]
fn refuses_an_image_of_another_size_and_stays_as_it_was() {
    let mut display =
        DisplayImage::try_new(ImageSize::new(2, 2).expect("a valid size")).expect("room");
    let mut white = Image::try_new(ImageSize::new(2, 2).expect("a valid size")).expect("room");
    white.rows_mut().flatten().for_each(|value| *value = 1.0);
    assert_eq!(display.encode(&white), Ok(()));
    let other = Image::try_new(ImageSize::new(2, 3).expect("a valid size")).expect("room");
    assert_eq!(
        display.encode(&other),
        Err(VisualError::DisplaySizeMismatch {
            width: 2,
            height: 3
        })
    );
    assert!(
        display.bytes().iter().all(|&byte| byte == 255),
        "the white kept"
    );
    let wider = Image::try_new(ImageSize::new(3, 2).expect("a valid size")).expect("room");
    assert_eq!(
        display.encode(&wider),
        Err(VisualError::DisplaySizeMismatch {
            width: 3,
            height: 2
        }),
        "the width checked too"
    );
}
