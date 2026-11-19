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
    alpha8, pixel8, srgb8, straight8,
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

/// The code of `color / alpha`, exactly: how many thresholds `color` reaches over `alpha`.
fn straight_code(color: f32, alpha: f32) -> u8 {
    u8::try_from(
        SRGB8_THRESHOLDS
            .iter()
            .filter(|&&threshold| reaches(threshold, alpha, color))
            .count(),
    )
    .expect("at most 255")
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

/// The float just above the finite float `value` of positive sign.
fn above(value: f32) -> f32 {
    f32::from_bits(value.to_bits().checked_add(1).expect("finite"))
}

#[test]
fn encodes_a_color_over_alpha_exactly_at_every_threshold() {
    // For each threshold and alphas from the faintest to above 1, the colors nearest the
    // exact product and their neighbours: the code is the exact quotient's.
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3EAA_AAAB),
        0.5,
        0.75,
        below(1.0),
        1.0,
        1.5,
    ];
    for &threshold in &SRGB8_THRESHOLDS {
        for alpha in alphas {
            // The product, rounded to a float, then the floats around it.
            let nearest = threshold * alpha;
            for color in [
                below(below(nearest)),
                below(nearest),
                nearest,
                above(nearest),
                above(above(nearest)),
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

#[test]
fn differs_from_a_rounded_division_where_it_must() {
    // Why the quotient is never rounded: a color just below a threshold times alpha can
    // round, divided, up to the threshold, one code above the exact quotient's.
    // Found by searching alphas from 1/8 up: 1/8 and the float above it, the first threshold.
    let alpha = f32::from_bits(0x3E00_0001);
    let witness = SRGB8_THRESHOLDS.iter().find_map(|&threshold| {
        let nearest = threshold * alpha;
        [below(nearest), nearest, above(nearest)]
            .into_iter()
            .find(|&color| srgb8(color / alpha) != straight8(color, alpha))
    });
    let color = witness.expect("a color where a rounded division misses the exact code");
    assert_eq!(
        straight8(color, alpha),
        straight_code(color, alpha),
        "the exact code"
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
fn starts_each_bucket_at_the_thresholds_below_it_and_holds_two_at_most_inside() {
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
            assert!(inside <= 2, "bucket {bucket}: {inside} thresholds inside");
        }
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
    // The buckets make the codes fast: from a value's bucket, two thresholds at most remain
    // to compare, at every edge of every bucket and at every threshold.
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
                start <= code && code <= start + 2,
                "{bits:#010x}: code {code}, its bucket starts at {start}"
            );
        }
    }
}

#[test]
fn starts_each_color_over_alpha_a_code_at_most_from_its_own() {
    // The rounded quotient starts one code at most from the exact one: one or two exact
    // products decide it.
    let alphas = [
        ALPHA8_THRESHOLDS[0],
        f32::from_bits(0x3E00_0001),
        0.5,
        0.75,
        below(1.0),
        1.5,
    ];
    for &threshold in &SRGB8_THRESHOLDS {
        for alpha in alphas {
            let nearest = threshold * alpha;
            for color in [below(nearest), nearest, above(nearest)] {
                let (start, exact_code) =
                    (super::straight_start(color, alpha), straight8(color, alpha));
                assert!(
                    start.abs_diff(exact_code) <= 1,
                    "{color:e} over {alpha:e}: starts at {start}, exactly {exact_code}"
                );
            }
        }
    }
}

#[test]
fn shows_a_pixel_whose_alpha_encodes_to_0_as_transparent_black_whatever_its_colors() {
    // Every alpha whose code is 0: zeros of both signs, negatives, not a number, subnormals
    // and the largest value below the first alpha threshold; each with colors of both signs,
    // zero, one, above one and not a number. The colors over such an alpha have no code
    // (`straight8` is defined for an alpha that encodes above 0 only): the pixel is
    // transparent black.
    let below_first = f32::from_bits(ALPHA8_THRESHOLDS[0].to_bits() - 1);
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
    assert_ne!(alpha8(ALPHA8_THRESHOLDS[0]), 0, "the first threshold encodes above 0");
}
