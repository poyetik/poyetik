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

use crate::word::{self, CONTROL};

pub use crate::word::{DEFAULT, Word};

/// The mode fields of the control register (`MXCSR` on `x86_64`, `FPCR` on aarch64), as a
/// [`Word`]: the bits a mode sets, the others being flags or reserved.
pub const MODE_FIELDS: Word = CONTROL;

/// The fields of the control register, by name, with the values they take.
///
/// The values are written into the field's bits: every rounding mode, flushes, traps and
/// the other mode fields. The first is the rounding, its values in the order of
/// [`ROUNDINGS`].
#[cfg(target_arch = "x86_64")]
pub const FIELDS: [(&str, &[Word]); 4] = [
    // Round to nearest, down, up, toward zero.
    ("rounding", &[0, 0x2000, 0x4000, 0x6000]),
    ("flush-to-zero", &[0, 0x8000]),
    ("denormals-are-zero", &[0, 0x0040]),
    // Every exception masked (the default), then every one unmasked (traps).
    ("exception masks", &[0x1f80, 0]),
];

/// The fields of the control register, by name, with the values they take.
///
/// The values are written into the field's bits: every rounding mode, flushes, traps and
/// the other mode fields. The first is the rounding, its values in the order of
/// [`ROUNDINGS`].
#[cfg(target_arch = "aarch64")]
pub const FIELDS: [(&str, &[Word]); 10] = [
    // Round to nearest, up, down, toward zero.
    ("rounding", &[0, 0x0040_0000, 0x0080_0000, 0x00c0_0000]),
    ("flush-to-zero", &[0, 0x0100_0000]),
    ("flush inputs to zero (FIZ)", &[0, 0x1]),
    ("half-precision flush (FZ16)", &[0, 0x0008_0000]),
    ("alternate handling (AH)", &[0, 0x2]),
    ("default NaN", &[0, 0x0200_0000]),
    ("alternative half precision", &[0, 0x0400_0000]),
    ("extended bfloat16 (EBF)", &[0, 0x2000]),
    ("NEP", &[0, 0x4]),
    // Every trap enable on (unsupported traps read back as zero).
    ("trap enables", &[0, 0x9f00]),
];

/// The rounding of each value of the first field of [`FIELDS`], in its order.
#[cfg(target_arch = "x86_64")]
pub const ROUNDINGS: [Rounding; 4] = [
    Rounding::Nearest,
    Rounding::Down,
    Rounding::Up,
    Rounding::TowardZero,
];

/// The rounding of each value of the first field of [`FIELDS`], in its order.
#[cfg(target_arch = "aarch64")]
pub const ROUNDINGS: [Rounding; 4] = [
    Rounding::Nearest,
    Rounding::Up,
    Rounding::Down,
    Rounding::TowardZero,
];

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets: every
/// exception masked (`0x1f80`), FTZ (`0x8000`) and DAZ (`0x0040`).
#[cfg(target_arch = "x86_64")]
pub const FLUSHING: Word = 0x9fc0;

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets: FZ
/// (`0x0100_0000`), FZ16 (`0x0008_0000`) and FIZ (`0x1`).
#[cfg(target_arch = "aarch64")]
pub const FLUSHING: Word = 0x0108_0001;

/// The flush-to-zero bit, which every processor of the target supports.
#[cfg(target_arch = "x86_64")]
pub const FLUSH_TO_ZERO: Word = 0x8000;

/// The flush-to-zero bit (FZ), which every processor of the target supports (FIZ is
/// optional: an Apple M2 reads it back as zero).
#[cfg(target_arch = "aarch64")]
pub const FLUSH_TO_ZERO: Word = 0x0100_0000;

/// Rounding up (toward +∞, `0x4000`), every exception masked (`0x1f80`).
#[cfg(target_arch = "x86_64")]
pub const ROUNDING_UP: Word = 0x5f80;

/// Rounding up (toward +∞).
#[cfg(target_arch = "aarch64")]
pub const ROUNDING_UP: Word = 0x0040_0000;

/// The field that decides whether an exception traps, and its value when none does: the
/// exception masks of MXCSR, every one set (masked).
#[cfg(target_arch = "x86_64")]
const TRAP_FIELD: (Word, Word) = (0x1f80, 0x1f80);

/// The field that decides whether an exception traps, and its value when none does: the
/// trap enables of FPCR, every one clear.
#[cfg(target_arch = "aarch64")]
const TRAP_FIELD: (Word, Word) = (0x9f00, 0);

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

/// Which subnormals survive in the calling thread's mode, `[binary64, binary32]`: a flush
/// field covers both formats, so the two agree under every mode of the targets.
///
/// An operand survives when the smallest subnormal times two is not zero; a result survives
/// when half the smallest normal number is not zero. Every value passes through `black_box`,
/// so the processor computes it in the mode in force.
#[must_use]
pub fn subnormals_kept() -> [SubnormalsKept; 2] {
    [
        SubnormalsKept {
            operands: black_box(black_box(f64::from_bits(1)) * black_box(2.0_f64)) != 0.0,
            results: black_box(black_box(f64::MIN_POSITIVE) / black_box(2.0_f64)) != 0.0,
        },
        SubnormalsKept {
            operands: black_box(black_box(f32::from_bits(1)) * black_box(2.0_f32)) != 0.0,
            results: black_box(black_box(f32::MIN_POSITIVE) / black_box(2.0_f32)) != 0.0,
        },
    ]
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
