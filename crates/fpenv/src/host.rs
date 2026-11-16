//! Support for tests of either library: forcing the floating-point mode a host may set on
//! the calling thread, and observing which mode is in force.
//!
//! A test shows what the default mode protects by running code under a host's mode on
//! purpose ([`as_host`]): rendering under a flushing or rounding host, with and without
//! [`with_default_mode`](crate::with_default_mode). Each forced mode is first shown in force
//! by an observation ([`subnormals_kept`], [`rounding`]), as the testing policy asks of an
//! emulation, so that code the compiler moved out of the forced mode fails loudly, never
//! passes.
//!
//! Review only: code run under a mode other than the default is outside Rust's floating-point
//! rule (RFC 3514; the audit of the control-register file says why this crate breaks it), and
//! under a mode that unmasks exceptions (a trapping mode of [`every_host_mode`], told by
//! [`traps`]) a float operation raising one traps. Tests run only what they observe there,
//! never production code.

#![forbid(unsafe_code)]

use core::hint::black_box;

use crate::word::{self, CONTROL, ROUND_DOWN, ROUND_TOWARD_ZERO, ROUND_UP};
#[cfg(target_arch = "aarch64")]
use crate::word::{AH, AHP, DN, EBF, FIZ, FZ, FZ16, NEP, TRAP_ENABLES};
#[cfg(target_arch = "x86_64")]
use crate::word::{DAZ, FTZ, MASKS};

pub use crate::word::{DEFAULT, Word};

/// The mode fields of the control register (`MXCSR` on `x86_64`, `FPCR` on aarch64), as a
/// [`Word`]: the bits a mode sets, the others being flags or reserved.
pub const MODE_FIELDS: Word = CONTROL;

/// The fields of the control register, by name, with the values they take.
///
/// The values are written into the field's bits: every rounding mode, flushes, traps and
/// the other mode fields. The first is the rounding, its values those of [`ROUNDINGS`], in
/// its order.
#[cfg(target_arch = "x86_64")]
pub const FIELDS: [(&str, &[Word]); 4] = [
    ("rounding", &[0, ROUND_DOWN, ROUND_UP, ROUND_TOWARD_ZERO]),
    ("flush-to-zero", &[0, FTZ]),
    ("denormals-are-zero", &[0, DAZ]),
    // Every exception masked (the default), then every one unmasked (traps).
    ("exception masks", &[MASKS, 0]),
];

/// The fields of the control register, by name, with the values they take.
///
/// The values are written into the field's bits: every rounding mode, flushes, traps and
/// the other mode fields. The first is the rounding, its values those of [`ROUNDINGS`], in
/// its order.
#[cfg(target_arch = "aarch64")]
pub const FIELDS: [(&str, &[Word]); 10] = [
    ("rounding", &[0, ROUND_UP, ROUND_DOWN, ROUND_TOWARD_ZERO]),
    ("flush-to-zero", &[0, FZ]),
    ("flush inputs to zero (FIZ)", &[0, FIZ]),
    ("half-precision flush (FZ16)", &[0, FZ16]),
    ("alternate handling (AH)", &[0, AH]),
    ("default NaN", &[0, DN]),
    ("alternative half precision", &[0, AHP]),
    ("extended bfloat16 (EBF)", &[0, EBF]),
    ("NEP", &[0, NEP]),
    // Every trap enable on (unsupported traps read back as zero).
    ("trap enables", &[0, TRAP_ENABLES]),
];

/// Each value of the first field of [`FIELDS`], the rounding, with the rounding it sets,
/// in its order.
#[cfg(target_arch = "x86_64")]
pub const ROUNDINGS: [(Word, Rounding); 4] = [
    (0, Rounding::Nearest),
    (ROUND_DOWN, Rounding::Down),
    (ROUND_UP, Rounding::Up),
    (ROUND_TOWARD_ZERO, Rounding::TowardZero),
];

/// Each value of the first field of [`FIELDS`], the rounding, with the rounding it sets,
/// in its order.
#[cfg(target_arch = "aarch64")]
pub const ROUNDINGS: [(Word, Rounding); 4] = [
    (0, Rounding::Nearest),
    (ROUND_UP, Rounding::Up),
    (ROUND_DOWN, Rounding::Down),
    (ROUND_TOWARD_ZERO, Rounding::TowardZero),
];

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets: every
/// exception masked, FTZ and DAZ (disjoint fields, so their sum is their union).
#[cfg(target_arch = "x86_64")]
pub const FLUSHING: Word = MASKS.wrapping_add(FTZ).wrapping_add(DAZ);

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets: FZ, FZ16 and
/// FIZ (disjoint fields, so their sum is their union).
#[cfg(target_arch = "aarch64")]
pub const FLUSHING: Word = FZ.wrapping_add(FZ16).wrapping_add(FIZ);

/// The flush-to-zero bit, which every processor of the target supports.
#[cfg(target_arch = "x86_64")]
pub const FLUSH_TO_ZERO: Word = FTZ;

/// The flush-to-zero bit (FZ), which every processor of the target supports (FIZ is
/// optional: an Apple M2 reads it back as zero).
#[cfg(target_arch = "aarch64")]
pub const FLUSH_TO_ZERO: Word = FZ;

/// Rounding up (toward +∞), every exception masked (disjoint fields, so their sum is their
/// union).
#[cfg(target_arch = "x86_64")]
pub const ROUNDING_UP: Word = MASKS.wrapping_add(ROUND_UP);

/// Rounding up (toward +∞).
#[cfg(target_arch = "aarch64")]
pub const ROUNDING_UP: Word = ROUND_UP;

/// The field that decides whether an exception traps, and its value when none does: the
/// exception masks of MXCSR, every one set (masked).
#[cfg(target_arch = "x86_64")]
const TRAP_FIELD: (Word, Word) = (MASKS, MASKS);

/// The field that decides whether an exception traps, and its value when none does: the
/// trap enables of FPCR, every one clear.
#[cfg(target_arch = "aarch64")]
const TRAP_FIELD: (Word, Word) = (TRAP_ENABLES, 0);

/// Whether `mode` lets a floating-point exception trap.
///
/// It does when an exception is unmasked (`x86_64`) or a trap enabled (aarch64). Under such
/// a mode, float work raising that exception stops the thread, so a test observes only the
/// mode fields there, never computes. One body for both targets, which differ only in
/// the field read and its value without traps, so that the tests of either architecture
/// prove it.
#[must_use]
pub const fn traps(mode: Word) -> bool {
    let (field, none) = TRAP_FIELD;
    mode & field != none
}

/// The mode fields of the control register of the calling thread now.
#[must_use]
pub fn control() -> Word {
    word::read() & CONTROL
}

/// Runs `body` with the calling thread's mode fields set to those of `host`.
///
/// Gives what it returned with the mode fields read back right after setting them (a field
/// the processor does not support reads back as zero). Restores the mode found on entry
/// afterwards, before the caller asserts anything, and when `body` unwinds: the caller's
/// floating point runs in the mode it had.
///
/// `body` passes through `black_box` after the write and its result before the restore, as
/// in [`with_default_mode`](crate::with_default_mode), so that its float work reading its
/// captures or memory runs in the forced mode.
pub fn as_host<R>(host: Word, body: impl FnOnce() -> R) -> (Word, R) {
    let entry = control();
    word::set_control(host);
    let set = control();
    // Restores the entry mode when dropped: after `body`, and if it unwinds.
    let restore = crate::Restore { host: entry };
    let body = black_box(body);
    let result = black_box(body());
    drop(restore);
    (set, result)
}

/// The number of host modes [`every_host_mode`] gives: the product of the counts of values
/// of every field.
#[must_use]
pub fn host_modes() -> usize {
    FIELDS.iter().map(|(_, values)| values.len()).product()
}

/// The number of distinct modes [`every_host_mode`] sets on the calling processor.
///
/// Counted as the modes read back: the product, over the fields, of the number of values of
/// each that read back distinct (a field the processor lacks reads back as zero, and counts
/// once).
///
/// A test looping over [`every_host_mode`] compares the modes it set with it, the positive
/// control that the loop really set every mode the processor can hold.
#[must_use]
pub fn settable_host_modes() -> usize {
    let set = |value: Word| as_host(value, || ()).0;
    FIELDS
        .iter()
        .map(|(_, values)| {
            values
                .iter()
                .enumerate()
                .filter(|&(at, &value)| {
                    let read = set(value);
                    !values.iter().take(at).any(|&earlier| set(earlier) == read)
                })
                .count()
        })
        .product()
}

/// Every combination of the values of [`FIELDS`], as mode fields: the first field varies
/// slowest.
pub fn every_host_mode() -> impl Iterator<Item = Word> {
    (0..host_modes()).map(|mut index| {
        FIELDS.iter().rev().fold(0, |mode: Word, (_, values)| {
            let value = index
                .checked_rem(values.len())
                .and_then(|at| values.get(at))
                .copied()
                .unwrap_or_default();
            index = index.checked_div(values.len()).unwrap_or_default();
            // The fields hold disjoint bits, so their sum is their union.
            mode.wrapping_add(value)
        })
    })
}

/// Whether subnormal numbers survive in the calling thread's mode, in one format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubnormalsKept {
    /// A subnormal operand is used as it is (no denormals-are-zero).
    pub operands: bool,
    /// A subnormal result is kept (no flush-to-zero).
    pub results: bool,
}

/// Which subnormals survive in the calling thread's mode, in each format, by name.
///
/// The twin of `audio_core::SubnormalArithmetic`, which this crate cannot name (it depends
/// on nothing), so that a test compares the two field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubnormalArithmetic {
    /// In binary32.
    pub binary32: SubnormalsKept,
    /// In binary64.
    pub binary64: SubnormalsKept,
}

/// Which subnormals survive in the calling thread's mode, in each format: a flush field
/// covers both formats, so the two agree under every mode of the targets.
///
/// An operand survives when the smallest subnormal times 2^60 (2^24 in binary32) is not zero,
/// a normal product, so that only a flush of operands can change it (flush-to-zero zeroes
/// every tiny result, exact or not, so the operand probe's product must be normal); a result
/// survives when half the smallest normal number is not zero. Every value passes through
/// `black_box`, so the processor computes it in the mode in force, and is read by its bits:
/// a floating-point comparison with zero would itself read a subnormal as zero where the
/// mode flushes operands (x86's denormals-are-zero), and see a kept result as flushed.
#[must_use]
pub fn subnormals_kept() -> SubnormalArithmetic {
    SubnormalArithmetic {
        binary32: SubnormalsKept {
            operands: black_box(black_box(f32::from_bits(1)) * black_box(16_777_216.0_f32))
                .to_bits()
                != 0,
            results: black_box(black_box(f32::MIN_POSITIVE) / black_box(2.0_f32)).to_bits() != 0,
        },
        binary64: SubnormalsKept {
            operands: black_box(
                black_box(f64::from_bits(1)) * black_box(f64::from_bits(0x43b0_0000_0000_0000)),
            )
            .to_bits()
                != 0,
            results: black_box(black_box(f64::MIN_POSITIVE) / black_box(2.0_f64)).to_bits() != 0,
        },
    }
}

/// A direction of rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// To nearest, ties to even: the default.
    Nearest,
    /// Toward +∞.
    Up,
    /// Toward −∞.
    Down,
    /// Toward zero.
    TowardZero,
}

/// The direction of rounding in force on the calling thread.
///
/// `1 + 2^-60` rounds above 1 only up, `-1 - 2^-60` below -1 only down, and of the two
/// others `1 + 1.5 ulp`, a tie, gives `1 + 2 ulp` only to nearest (to even), `1 + ulp`
/// toward zero.
#[must_use]
pub fn rounding() -> Rounding {
    // 2^-60, written as its bits: the biased exponent 1023 - 60 = 0x3c3, no fraction.
    let tiny = f64::from_bits(0x3c30_0000_0000_0000);
    let ulp = f64::EPSILON;
    if black_box(black_box(1.0_f64) + black_box(tiny)) > 1.0 {
        Rounding::Up
    } else if black_box(black_box(-1.0_f64) - black_box(tiny)) < -1.0 {
        Rounding::Down
    } else if black_box(black_box(1.0_f64) + black_box(1.5_f64 * ulp)).to_bits()
        == 1.0_f64.to_bits().wrapping_add(2)
    {
        Rounding::Nearest
    } else {
        Rounding::TowardZero
    }
}
