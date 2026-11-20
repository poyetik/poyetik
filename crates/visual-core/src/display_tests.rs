//! Tests of `display.rs`: every threshold proven in exact rational arithmetic (no float
//! function, whose rounding the thresholds exist to avoid), the codes at the edges of their
//! domain, and the display image's allocation and encoding.

extern crate alloc;

use alloc::collections::TryReserveError;
use alloc::format;
use alloc::string::ToString as _;
use alloc::vec;
use alloc::vec::Vec;

use visual_testkit::proptest::prelude::{Just, Strategy as _, prop_oneof};
use visual_testkit::proptest::test_runner::TestCaseError;
use visual_testkit::runner::runner;

use super::{
    ALPHA8_THRESHOLDS, DISPLAY_THRESHOLD_BITS, DISPLAY_WGSL, DisplayImage, SRGB8_THRESHOLDS,
    alpha8, encode_pixels, pixel8, srgb8, straight8,
};
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

    /// It times `2^bits`: whole limbs prepended, then the rest shifted with its carries.
    fn shifted(&self, bits: u32) -> Self {
        let (whole, part) = (bits / 64, bits % 64);
        let mut limbs = vec![0_u64; usize::try_from(whole).expect("a count")];
        let mut carry = 0_u64;
        for &limb in &self.0 {
            limbs.push(limb.checked_shl(part).unwrap_or(0) | carry);
            // The bits pushed out of the limb, none when nothing is shifted.
            carry = 64_u32
                .checked_sub(part)
                .and_then(|back| limb.checked_shr(back))
                .unwrap_or(0);
        }
        limbs.push(carry);
        Self(limbs)
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
        .shifted(shift.checked_mul(5).expect("small"));
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

/// A positive finite float as `(significand, exponent)`, the float equal to
/// `significand × 2^exponent` exactly (subnormals included).
fn significand_exponent(value: f32) -> (u64, i32) {
    let bits = value.to_bits();
    assert!(
        bits >> 31_u32 == 0 && value.is_finite(),
        "{value:e}: positive, finite"
    );
    let fraction = u64::from(bits & 0x7F_FFFF);
    let field = i32::try_from(bits >> 23_u32).expect("8 bits");
    if field == 0 {
        (fraction, -149)
    } else {
        (
            fraction | 1 << 23_u32,
            field.checked_sub(150).expect("small"),
        )
    }
}

/// Whether `threshold × alpha ≤ color` exactly, in integers: positive finite `threshold` and
/// `alpha`, any finite `color`.
fn reaches(threshold: f32, alpha: f32, color: f32) -> bool {
    if color <= 0.0 {
        return false;
    }
    let ((threshold, at), (alpha, aa), (color, ac)) = (
        significand_exponent(threshold),
        significand_exponent(alpha),
        significand_exponent(color),
    );
    let left = Natural::one().times(threshold).times(alpha);
    let right = Natural::one().times(color);
    let shift = at
        .checked_add(aa)
        .and_then(|product| ac.checked_sub(product))
        .expect("small");
    u32::try_from(shift).map_or_else(
        |_| right.at_least(&left.shifted(shift.unsigned_abs())),
        |up| right.shifted(up).at_least(&left),
    )
}

/// Whether `left × 2^left_exponent ≥ right × 2^right_exponent`, exactly.
fn scaled_at_least(
    left: &Natural,
    left_exponent: i32,
    right: &Natural,
    right_exponent: i32,
) -> bool {
    let shift = left_exponent.checked_sub(right_exponent).expect("small");
    u32::try_from(shift).map_or_else(
        |_| left.at_least(&right.shifted(shift.unsigned_abs())),
        |up| left.shifted(up).at_least(right),
    )
}

/// The 8-bit sRGB code of the float `value`, at or above 0, by the words of the
/// specification, independently of the thresholds: its encoding by the transfer function of
/// IEC 61966-2-1, in exact rational arithmetic, rounded to the nearest code, a tie to the
/// higher (255 at or above 1).
fn iec_code(value: f32) -> u8 {
    if value >= 1.0 {
        return u8::MAX;
    }
    // Below 2^-13 (0 included) the code is 0: 12.92 × 2^-13 is below 1/510, the middle
    // between 0 and 1, and the encoding only grows.
    let smallest = f32::from_bits(0x3900_0000);
    assert!(!encodes_to_at_least(smallest, 0), "2^-13 encodes to 0");
    if value < smallest {
        return 0;
    }
    codes_reached(|code| encodes_to_at_least(value, code))
}

/// How many of the codes' middles `reached_after` holds for, from the first: `reached_after(code)` says
/// whether the middle after `code` is reached, true below some code and false from it (a
/// value's encoding only grows), so that a binary search finds the first it misses.
fn codes_reached(reached_after: impl Fn(u64) -> bool) -> u8 {
    if !reached_after(0) {
        return 0;
    }
    if reached_after(254) {
        return u8::MAX;
    }
    // The middle after `reached` is reached, the one after `missed` is not.
    let (mut reached, mut missed) = (0_u64, 254_u64);
    while missed.checked_sub(reached).expect("ordered") > 1 {
        let middle = reached
            .checked_add(missed.checked_sub(reached).expect("ordered") / 2)
            .expect("small");
        if reached_after(middle) {
            reached = middle;
        } else {
            missed = middle;
        }
    }
    u8::try_from(missed).expect("at most 254")
}

/// The largest 32-bit float at or below the real quotient `color / alpha`, for a positive
/// finite `alpha`: found by a binary search over the bits of the positive floats, which
/// order as their values, each candidate `f` compared exactly (`f × alpha ≤ color`); 0 when
/// no positive float is at or below it, infinity when `color` is.
fn float_below_quotient(color: f32, alpha: f32) -> f32 {
    if color.is_nan() || color <= 0.0 {
        return 0.0;
    }
    if color.is_infinite() {
        return f32::INFINITY;
    }
    let (mut low, mut high) = (1_u32, f32::MAX.to_bits());
    if !reaches(f32::from_bits(low), alpha, color) {
        return 0.0;
    }
    if reaches(f32::from_bits(high), alpha, color) {
        return f32::MAX;
    }
    // `low` is at or below the quotient, `high` above it.
    while high.checked_sub(low).expect("ordered") > 1 {
        let middle = low
            .checked_add(high.checked_sub(low).expect("ordered") / 2)
            .expect("small");
        if reaches(f32::from_bits(middle), alpha, color) {
            low = middle;
        } else {
            high = middle;
        }
    }
    f32::from_bits(low)
}

/// The straight value's code by the specification's words (`spec/visual-design.md`,
/// section 9, "Display encoding"): the largest 32-bit float at or below the real quotient `color / alpha`,
/// then its sRGB code. Independent of `straight8`: no threshold is counted against a
/// product; the float is found first, then encoded.
fn straight_by_the_float_below(color: f32, alpha: f32) -> u8 {
    iec_code(float_below_quotient(color, alpha))
}

/// Whether the real quotient `color / alpha` (positive finite both) encodes by IEC
/// 61966-2-1 to at least `(2 code + 1) / 510`, in exact rational arithmetic: the quotient
/// never rounded to a float.
fn quotient_encodes_to_at_least(color: f32, alpha: f32, code: u64) -> bool {
    let ((color, color_exponent), (alpha, alpha_exponent)) =
        (significand_exponent(color), significand_exponent(alpha));
    let odd = code
        .checked_mul(2)
        .and_then(|twice| twice.checked_add(1))
        .expect("small");
    // q ≤ 0.003 130 8, i.e. color × 10⁷ ≤ 31 308 × alpha: the linear segment,
    // 12.92 q ≥ odd / 510, i.e. 1292 × 510 × color ≥ 100 × odd × alpha.
    if scaled_at_least(
        &Natural::one().times(31_308).times(alpha),
        alpha_exponent,
        &Natural::one().times(10_000_000).times(color),
        color_exponent,
    ) {
        return scaled_at_least(
            &Natural::one().times(1292 * 510).times(color),
            color_exponent,
            &Natural::one().times(odd).times(100).times(alpha),
            alpha_exponent,
        );
    }
    // q⁵ ≥ (N / D)¹², i.e. color⁵ × D¹² ≥ N¹² × alpha⁵, as in `encodes_to_at_least`.
    let numerator = odd
        .checked_mul(200)
        .and_then(|scaled| scaled.checked_add(5610))
        .expect("small");
    scaled_at_least(
        &Natural::one()
            .times_power(color, 5)
            .times_power(510 * 211, 12),
        color_exponent.checked_mul(5).expect("small"),
        &Natural::one()
            .times_power(numerator, 12)
            .times_power(alpha, 5),
        alpha_exponent.checked_mul(5).expect("small"),
    )
}

/// The code of the real quotient `color / alpha` itself, never rounded to a float, encoded
/// by IEC 61966-2-1 and rounded to the nearest code (positive finite `alpha`): the wording
/// the specification held before 2026-11-20, which the encoding does not follow.
fn straight_by_the_real_quotient(color: f32, alpha: f32) -> u8 {
    if color.is_nan() || color <= 0.0 {
        return 0;
    }
    if color.is_infinite() || reaches(1.0, alpha, color) {
        return u8::MAX;
    }
    codes_reached(|code| quotient_encodes_to_at_least(color, alpha, code))
}

/// The code of `color / alpha` as the encoding counts it: how many thresholds `t` have
/// `t × alpha` at most `color`, exactly.
fn straight_code(color: f32, alpha: f32) -> u8 {
    u8::try_from(
        SRGB8_THRESHOLDS
            .iter()
            .filter(|&&threshold| reaches(threshold, alpha, color))
            .count(),
    )
    .expect("at most 255")
}

#[test]
fn holds_the_smallest_float_reaching_each_srgb_code() {
    for (code, &threshold) in (0_u64..).zip(SRGB8_THRESHOLDS.iter()) {
        assert!(
            encodes_to_at_least(threshold, code),
            "after code {code}: {threshold:e} reaches the next"
        );
        assert!(
            !encodes_to_at_least(threshold.next_down(), code),
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
            !at_least_half_code(threshold.next_down(), code),
            "after alpha {code}: the float below {threshold:e} does not"
        );
    }
}

#[test]
fn encodes_known_values() {
    // Known answers of the sRGB encoding (IEC 61966-2-1), rounded to 8 bits: the middle
    // gray, 18% gray, the end of the linear segment, and the extremes.
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
            (srgb8(srgb), srgb8(srgb.next_down())),
            (code, code - 1),
            "sRGB code {code}"
        );
        assert_eq!(
            (alpha8(alpha), alpha8(alpha.next_down())),
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
fn encodes_as_many_whole_pixels_as_values_and_bytes_hold_without_allocating() {
    let values = [
        1.0_f32, 0.5, 0.0, 1.0, // opaque
        0.25, 0.5, 0.0, 0.5, // half
        0.3, 0.3, 0.3, 0.0, // transparent
        0.7, // a part of a pixel
    ];
    let mut bytes = [7_u8; 4 * 4 + 2];
    let ((), operations) =
        visual_testkit::heap_operations_during(|| encode_pixels(&values, &mut bytes))
            .expect("counting");
    assert_eq!(operations, 0, "nothing allocated");
    assert_eq!(
        bytes,
        [
            255, 188, 0, 255, 188, 255, 0, 128, 0, 0, 0, 0, 7, 7, 7, 7, 7, 7
        ],
        "three whole pixels by pixel8, the bytes past them as they were"
    );
    let mut fewer = [7_u8; 4 + 3];
    encode_pixels(&values, &mut fewer);
    assert_eq!(
        fewer,
        [255, 188, 0, 255, 7, 7, 7],
        "as many as the bytes hold"
    );
}

#[test]
fn hands_its_bytes_to_another_encoder() {
    let size = ImageSize::new(3, 2).expect("a valid size");
    let mut display = DisplayImage::try_new(size).expect("room for it");
    assert_eq!(display.bytes_mut().len(), 3 * 2 * 4, "every byte");
    display.bytes_mut().fill(7);
    assert!(
        display.bytes().iter().all(|&byte| byte == 7),
        "written in place"
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

#[test]
fn encodes_a_color_over_alpha_exactly_at_every_threshold() {
    // For each threshold and alphas from the faintest to above 1, the colors nearest the
    // product and their neighbors: the code is the count of thresholds each reaches.
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3EAA_AAAB),
        0.5,
        0.75,
        1.0_f32.next_down(),
        1.0,
        1.5,
    ];
    for &threshold in &SRGB8_THRESHOLDS {
        for alpha in alphas {
            // The product, rounded to a float, then the floats around it.
            let nearest = threshold * alpha;
            for color in [
                nearest.next_down().next_down(),
                nearest.next_down(),
                nearest,
                nearest.next_up(),
                nearest.next_up().next_up(),
            ] {
                assert_eq!(
                    straight8(color, alpha),
                    straight_code(color, alpha),
                    "{color:e} over {alpha:e}, at threshold {threshold:e}"
                );
            }
        }
    }
}

#[test]
fn encodes_any_color_over_any_alpha_exactly() {
    let floats = |top: u32| {
        prop_oneof![
            (1_u32..=top).prop_map(f32::from_bits),
            Just(f32::from_bits(1)),
            Just(1.0_f32),
            Just(f32::from_bits(top)),
        ]
    };
    // Colors from the smallest subnormal to 2, alphas from it to 2: at and above 1 included.
    let verdict = runner(4096).run(
        &(floats(0x4000_0000), floats(0x4000_0000)),
        |(color, alpha)| {
            let (ours, exact_code) = (straight8(color, alpha), straight_code(color, alpha));
            if ours == exact_code {
                Ok(())
            } else {
                Err(TestCaseError::fail(format!(
                    "{color:e} over {alpha:e}: {ours}, exactly {exact_code}"
                )))
            }
        },
    );
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

/// The colors and alphas the straight value's oracles are compared on: for each threshold
/// and alphas from the faintest to above 1, the floats nearest the product and their
/// neighbors, two on each side.
fn threshold_grid() -> Vec<(f32, f32)> {
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3EAA_AAAB),
        0.5,
        0.75,
        1.0_f32.next_down(),
        1.0,
        1.5,
    ];
    SRGB8_THRESHOLDS
        .iter()
        .flat_map(|&threshold| {
            alphas.map(|alpha| {
                let nearest = threshold * alpha;
                [
                    nearest.next_down().next_down(),
                    nearest.next_down(),
                    nearest,
                    nearest.next_up(),
                    nearest.next_up().next_up(),
                ]
                .map(|color| (color, alpha))
            })
        })
        .flatten()
        .collect()
}

#[test]
fn encodes_a_color_over_alpha_as_the_code_of_the_float_below_the_quotient() {
    // The specification's words (section 9: the largest 32-bit float at or below the real
    // quotient, then its sRGB code), followed by an oracle that finds that float first and
    // encodes it by IEC 61966-2-1 in rationals, never counting thresholds against products.
    for (color, alpha) in threshold_grid() {
        assert_eq!(
            straight8(color, alpha),
            straight_by_the_float_below(color, alpha),
            "{color:e} over {alpha:e}"
        );
    }
    let floats = |top: u32| {
        prop_oneof![
            (1_u32..=top).prop_map(f32::from_bits),
            Just(f32::from_bits(1)),
            Just(1.0_f32),
            Just(f32::from_bits(top)),
        ]
    };
    let verdict = runner(1024).run(
        &(floats(0x4000_0000), floats(0x4000_0000)),
        |(color, alpha)| {
            let (ours, by_words) = (
                straight8(color, alpha),
                straight_by_the_float_below(color, alpha),
            );
            if ours == by_words {
                Ok(())
            } else {
                Err(TestCaseError::fail(format!(
                    "{color:e} over {alpha:e}: {ours}, by the specification {by_words}"
                )))
            }
        },
    );
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

#[test]
fn encodes_by_the_specification_at_the_edges_of_color_and_alpha() {
    // Colors of no code, the smallest and largest floats and infinity, over alphas from the
    // faintest to the largest: quotients below every float, above every float, and at 1.
    for alpha in [ALPHA8_THRESHOLDS[0], 0.5, 1.0, 2.0, f32::MAX] {
        for color in [
            0.0_f32,
            -0.0,
            -1.0,
            f32::NAN,
            f32::from_bits(1),
            0.25,
            1.0,
            2.0,
            f32::MAX,
            f32::INFINITY,
        ] {
            let (ours, below, real) = (
                straight8(color, alpha),
                straight_by_the_float_below(color, alpha),
                straight_by_the_real_quotient(color, alpha),
            );
            assert_eq!(ours, below, "{color:e} over {alpha:e}");
            assert!(
                below <= real && real - below <= 1,
                "{color:e} over {alpha:e}: {below}, the real quotient {real}"
            );
        }
    }
}

#[test]
fn encodes_the_float_below_the_quotient_not_the_quotient_itself() {
    // Pinned: the real quotient, never rounded to a float, can encode one code higher, when
    // it falls between a threshold and the float below it. The display corpus holds this
    // pixel (green over the first alpha threshold): code 0, where the real quotient's is 1.
    let (green, alpha_threshold) = (f32::from_bits(0x349F_C277), ALPHA8_THRESHOLDS[0]);
    assert_eq!(
        [
            straight8(green, alpha_threshold),
            straight_by_the_float_below(green, alpha_threshold),
            straight_by_the_real_quotient(green, alpha_threshold),
        ],
        [0, 0, 1],
        "the encoding, the float below, the real quotient"
    );
    // Elsewhere at every threshold the two differ by that one code at most, and only up.
    let differ = threshold_grid()
        .into_iter()
        .filter(|&(color, alpha)| {
            let (below, real) = (
                straight_by_the_float_below(color, alpha),
                straight_by_the_real_quotient(color, alpha),
            );
            assert!(
                below <= real && real - below <= 1,
                "{color:e} over {alpha:e}: {below}, the real quotient {real}"
            );
            below != real
        })
        .count();
    assert!(differ > 0, "the two wordings differ at some threshold");
}

#[test]
fn differs_from_a_rounded_division_where_it_must() {
    // Why the quotient is never a rounded division: a color just below a threshold times
    // alpha can round, divided, up to the threshold, one code above the float below the
    // real quotient.
    // Found by searching alphas from 1/8 up: 1/8 and the float above it, the first threshold.
    let alpha = f32::from_bits(0x3E00_0001);
    let witness = SRGB8_THRESHOLDS.iter().find_map(|&threshold| {
        let nearest = threshold * alpha;
        [nearest.next_down(), nearest, nearest.next_up()]
            .into_iter()
            .find(|&color| srgb8(color / alpha) != straight8(color, alpha))
    });
    let color = witness.expect("a color where a rounded division misses the counted code");
    assert_eq!(
        straight8(color, alpha),
        straight_code(color, alpha),
        "the counted code"
    );
}

#[test]
fn encodes_colors_outside_the_codes_over_any_alpha() {
    for alpha in [ALPHA8_THRESHOLDS[0], 0.5, 1.0, 2.0] {
        for color in [
            -0.0_f32,
            0.0,
            -1.0,
            f32::NEG_INFINITY,
            f32::NAN,
            f32::from_bits(1),
        ] {
            assert_eq!(straight8(color, alpha), 0, "{color:e} over {alpha:e}: 0");
        }
        for color in [alpha, 2.0 * alpha, f32::MAX, f32::INFINITY] {
            assert_eq!(
                straight8(color, alpha),
                255,
                "{color:e} over {alpha:e}: 255"
            );
        }
    }
    // An infinite alpha takes every finite color to 0, the infinite one to 255.
    assert_eq!(
        [
            straight8(f32::MAX, f32::INFINITY),
            straight8(f32::INFINITY, f32::INFINITY)
        ],
        [0, 255]
    );
}

#[test]
fn gives_the_gpu_its_thresholds_as_bits_and_its_shader() {
    let expected: Vec<u32> = SRGB8_THRESHOLDS
        .iter()
        .chain(&ALPHA8_THRESHOLDS)
        .map(|threshold| threshold.to_bits())
        .collect();
    assert_eq!(
        DISPLAY_THRESHOLD_BITS.to_vec(),
        expected,
        "sRGB, then alpha"
    );
    assert!(
        DISPLAY_WGSL.contains("var<storage, read> thresholds: array<u32, 510>;"),
        "the shader reads them all"
    );
}

/// How many of `thresholds` are at or below `value`: the code, counted naively.
fn counted(thresholds: &[f32; 255], value: f32) -> u8 {
    u8::try_from(
        thresholds
            .iter()
            .filter(|&&threshold| threshold <= value)
            .count(),
    )
    .expect("at most 255")
}

#[test]
fn names_the_smallest_float_of_each_bucket_in_order() {
    // 0x680 buckets of 2^16 floats, from 2^-13 to 1: each floor is the smallest float of
    // the next table entry, the one other crates' corpora reach every bucket through.
    let floors: Vec<u32> = super::display_bucket_floors().map(f32::to_bits).collect();
    assert_eq!(floors.len(), super::BUCKETS, "one a bucket");
    assert_eq!(
        (floors.first(), floors.last()),
        (Some(&(1.0_f32 / 8192.0).to_bits()), Some(&0x3F7F_0000_u32)),
        "from 2^-13 to the bucket below 1"
    );
    for (bucket, bits) in floors.iter().enumerate() {
        assert_eq!(
            (bits >> super::BUCKET_SHIFT) - (super::LOW_BITS >> super::BUCKET_SHIFT),
            u32::try_from(bucket).expect("a few thousand"),
            "{bits:#x}: the floor of bucket {bucket}"
        );
        assert_eq!(
            bits & 0xFFFF,
            0,
            "{bits:#x}: the smallest float of its bucket"
        );
    }
}

#[test]
fn starts_each_bucket_at_the_thresholds_below_it_and_holds_one_at_most_inside() {
    for (thresholds, starts) in [
        (&SRGB8_THRESHOLDS, &super::SRGB8_STARTS),
        (&ALPHA8_THRESHOLDS, &super::ALPHA8_STARTS),
    ] {
        for (bucket, &start) in starts.iter().enumerate() {
            let high = (super::LOW_BITS >> super::BUCKET_SHIFT)
                .checked_add(u32::try_from(bucket).expect("a few thousand"))
                .expect("small");
            let low = high << super::BUCKET_SHIFT;
            let next = high.checked_add(1).expect("small") << super::BUCKET_SHIFT;
            let below = thresholds
                .iter()
                .filter(|threshold| threshold.to_bits() < low)
                .count();
            let inside = thresholds
                .iter()
                .filter(|threshold| (low..next).contains(&threshold.to_bits()))
                .count();
            assert_eq!(
                usize::from(start),
                below,
                "bucket {bucket}: the thresholds below"
            );
            assert!(inside <= 1, "bucket {bucket}: {inside} thresholds inside");
        }
        // Why one at most: consecutive thresholds are further apart than a bucket's floats
        // (measured 2026-11-19: 100,925 floats at least for sRGB, 65,793 for alpha, against
        // 65,536), so that no bucket holds two.
        let closest = thresholds
            .windows(2)
            .map(|pair| pair[1].to_bits() - pair[0].to_bits())
            .min()
            .expect("255 thresholds");
        assert!(
            closest > 1 << super::BUCKET_SHIFT,
            "{closest} floats apart at the closest"
        );
    }
    // Built at compile time; built again here, the same tables.
    assert_eq!(
        super::starts(&SRGB8_THRESHOLDS),
        super::SRGB8_STARTS,
        "sRGB"
    );
    assert_eq!(
        super::starts(&ALPHA8_THRESHOLDS),
        super::ALPHA8_STARTS,
        "alpha"
    );
    assert_eq!(
        (super::LOW_BITS, super::HIGH_BITS, super::BUCKETS),
        (0x3900_0000, 1.0_f32.to_bits(), 0x680),
        "from 2^-13, below every threshold, to 1, above every one"
    );
    assert!(
        SRGB8_THRESHOLDS[0].to_bits() >= super::LOW_BITS
            && ALPHA8_THRESHOLDS[0].to_bits() >= super::LOW_BITS
            && SRGB8_THRESHOLDS[254].to_bits() < super::HIGH_BITS
            && ALPHA8_THRESHOLDS[254].to_bits() < super::HIGH_BITS,
        "every threshold between"
    );
}

#[test]
fn encodes_any_bits_as_the_naive_count_does() {
    // Every float of a bucket's edges and of random bits, of either sign, not a number
    // included: the fast code is the naive count.
    let edges = (0..=super::BUCKETS).flat_map(|bucket| {
        let low = (super::LOW_BITS >> super::BUCKET_SHIFT)
            .checked_add(u32::try_from(bucket).expect("a few thousand"))
            .expect("small")
            << super::BUCKET_SHIFT;
        [low.saturating_sub(1), low, low.saturating_add(1)]
    });
    let specials = [
        0,
        1,
        super::INFINITY_BITS,
        0x7FC0_0000,
        0x8000_0000,
        0xFF80_0000,
        u32::MAX,
    ];
    let thresholds = SRGB8_THRESHOLDS
        .iter()
        .chain(&ALPHA8_THRESHOLDS)
        .flat_map(|threshold| {
            let bits = threshold.to_bits();
            [bits.saturating_sub(1), bits, bits.saturating_add(1)]
        });
    for bits in edges.chain(specials).chain(thresholds) {
        let value = f32::from_bits(bits);
        assert_eq!(
            (srgb8(value), alpha8(value)),
            (
                counted(&SRGB8_THRESHOLDS, value),
                counted(&ALPHA8_THRESHOLDS, value)
            ),
            "{bits:#010x}"
        );
    }
    let verdict = runner(4096).run(&(0_u32..=u32::MAX), |bits| {
        let value = f32::from_bits(bits);
        let fast = (srgb8(value), alpha8(value));
        let naive = (
            counted(&SRGB8_THRESHOLDS, value),
            counted(&ALPHA8_THRESHOLDS, value),
        );
        if fast == naive {
            Ok(())
        } else {
            Err(TestCaseError::fail(format!(
                "{bits:#010x}: {fast:?}, naively {naive:?}"
            )))
        }
    });
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

#[test]
fn starts_two_comparisons_at_most_from_each_code() {
    // The buckets make the codes fast: from a value's bucket, one code at most remains to
    // add, at every edge of every bucket and at every threshold. Derived, not measured: a
    // bucket holds one threshold at most (the gap checked by
    // `starts_each_bucket_at_the_thresholds_below_it_and_holds_one_at_most_inside`), so two
    // comparisons decide it, the threshold inside and the one after it.
    for (thresholds, starts) in [
        (&SRGB8_THRESHOLDS, &super::SRGB8_STARTS),
        (&ALPHA8_THRESHOLDS, &super::ALPHA8_STARTS),
    ] {
        let edges = (0..super::BUCKETS).flat_map(|bucket| {
            let low = (super::LOW_BITS >> super::BUCKET_SHIFT)
                .checked_add(u32::try_from(bucket).expect("a few thousand"))
                .expect("small")
                << super::BUCKET_SHIFT;
            [low, low | 0xFFFF]
        });
        let around = thresholds.iter().map(|threshold| threshold.to_bits());
        for bits in edges.chain(around) {
            let code = usize::from(counted(thresholds, f32::from_bits(bits)));
            let start = super::bucket_start(starts, bits);
            assert!(
                start <= code && code <= start + 1,
                "{bits:#010x}: code {code}, its bucket starts at {start}"
            );
        }
    }
}

#[test]
fn starts_each_color_over_alpha_at_its_code_or_the_one_above() {
    // The start is never below the exact code, and one above at most: the rounded quotient
    // is at or above the largest float below the real one, and an infinite color over an
    // infinite alpha (a quotient that is not a number) starts at 255, its code. So the code
    // is found moving down only, by one or two exact products: over the grid of every
    // threshold, and over the special values as colors and alphas (an alpha whose code is
    // above 0, the only ones `straight8` is given).
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3E00_0001),
        0.5,
        0.75,
        1.0_f32.next_down(),
        1.5,
    ];
    let mut cases = Vec::new();
    for &threshold in &SRGB8_THRESHOLDS {
        for alpha in alphas {
            let nearest = threshold * alpha;
            for color in [nearest.next_down(), nearest, nearest.next_up()] {
                cases.push((color, alpha));
            }
        }
    }
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
    for color in specials {
        for alpha in specials.into_iter().chain(alphas) {
            if alpha8(alpha) > 0 {
                cases.push((color, alpha));
            }
        }
    }
    for (color, alpha) in cases {
        let (start, exact_code) = (super::straight_start(color, alpha), straight8(color, alpha));
        assert!(
            exact_code <= start && start.abs_diff(exact_code) <= 1,
            "{color:e} over {alpha:e}: starts at {start}, exactly {exact_code}"
        );
    }
    assert_eq!(
        (
            super::straight_start(f32::INFINITY, f32::INFINITY),
            straight8(f32::INFINITY, f32::INFINITY)
        ),
        (255, 255),
        "an infinite color over an infinite alpha: its code at once"
    );
}

#[test]
fn shows_a_pixel_whose_alpha_encodes_to_0_as_transparent_black_whatever_its_colors() {
    // Every alpha whose code is 0: zeros of both signs, negatives, not a number, subnormals
    // and the largest value below the first alpha threshold; each with colors of both signs,
    // zero, one, above one and not a number. The colors over such an alpha have no code
    // (`straight8` is defined for an alpha that encodes above 0 only): the pixel is
    // transparent black.
    let below_first = ALPHA8_THRESHOLDS[0].next_down();
    for alpha in [
        0.0_f32,
        -0.0,
        -0.5,
        -1.0,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(1),
        f32::from_bits(0x007f_ffff),
        below_first,
    ] {
        assert_eq!(alpha8(alpha), 0, "{alpha:e} encodes to 0");
        for color in [0.0_f32, -0.0, 0.25, -0.5, 1.0, 2.0, f32::NAN] {
            assert_eq!(
                pixel8(color, color, color, alpha),
                [0; 4],
                "{color:e} over {alpha:e}"
            );
        }
    }
    assert_ne!(
        alpha8(ALPHA8_THRESHOLDS[0]),
        0,
        "the first threshold encodes above 0"
    );
}
