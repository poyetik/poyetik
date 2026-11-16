//! Tests of the default mode and of the host modes tests force: each forced mode first
//! proven in force, the default inside under every host mode, the host's mode restored,
//! nesting, early exits, unwinding and threads; and where a flush changes the rounding to
//! binary32, per architecture.

#![forbid(unsafe_code)]

extern crate alloc;
extern crate std;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::black_box;

use super::host::{
    DEFAULT, FIELDS, FLUSH_TO_ZERO, FLUSHING, MODE_FIELDS, ROUNDING_UP, ROUNDINGS, Rounding,
    SubnormalArithmetic, SubnormalsKept, Word, as_host, control, every_host_mode, host_modes,
    rounding, settable_host_modes, subnormals_kept, traps,
};
use super::with_default_mode;
use super::word::{CONTROL, read, set_control};

/// [`as_host`], the one call of it in this file: the forced mode is test support, which
/// Clippy refuses anywhere else (`clippy.toml`), so this expectation also proves the lint
/// fires.
#[expect(
    clippy::disallowed_methods,
    reason = "the tests of fpenv force host modes on purpose, through this helper only"
)]
fn forced<R>(host: Word, body: impl FnOnce() -> R) -> (Word, R) {
    as_host(host, body)
}

/// A word of flush fields, with what [`subnormals_kept`] observes under it, `(operands
/// kept, results kept)`; the bits of it a processor of the target may lack (they then read
/// back as zero); and what it observes without them.
type FlushField = (&'static str, Word, (bool, bool), Word, (bool, bool));

/// Each flush field alone ([`FlushField`]). On `x86_64`, FTZ flushes results only and DAZ
/// operands only, every processor having both.
#[cfg(target_arch = "x86_64")]
const FLUSH_FIELDS: [FlushField; 3] = [
    (
        "flush-to-zero alone",
        0x8000,
        (true, false),
        0,
        (true, false),
    ),
    (
        "denormals-are-zero alone",
        0x0040,
        (false, true),
        0,
        (false, true),
    ),
    ("both", 0x8000 | 0x0040, (false, false), 0, (false, false)),
];

/// Each flush field alone ([`FlushField`]). On aarch64, with AH clear, FZ flushes both and
/// FIZ operands only (FIZ is optional: an Apple M2 reads it back as zero, and nothing is
/// flushed); with AH set (alternate handling, `FEAT_AFP`, optional too: an M2 reads it back
/// as zero, FZ then acting alone), FZ flushes results only, as the Arm architecture defines
/// it, observed only on a processor that has it.
#[cfg(target_arch = "aarch64")]
const FLUSH_FIELDS: [FlushField; 3] = [
    ("FZ alone", 0x0100_0000, (false, false), 0, (false, false)),
    ("FIZ alone", 0x1, (false, true), 0x1, (true, true)),
    (
        "FZ with AH",
        0x0100_0000 | 0x2,
        (true, false),
        0x2,
        (false, false),
    ),
];

/// What [`subnormals_kept`] observes, as `(operands kept, results kept)`: binary64 and
/// binary32 alike, which a flush field covers both.
fn kept() -> (bool, bool) {
    let SubnormalArithmetic { binary32, binary64 } = subnormals_kept();
    assert_eq!(binary64, binary32, "binary64 observed like binary32");
    let SubnormalsKept { operands, results } = binary64;
    (operands, results)
}

#[test]
fn the_default_is_a_mode_of_the_control_fields() {
    assert_eq!(MODE_FIELDS, CONTROL, "the mode fields are those written");
    assert_eq!(DEFAULT & !CONTROL, 0, "the default sets mode fields only");
    for (name, values) in FIELDS {
        for value in values {
            assert_eq!(value & !CONTROL, 0, "{name}: {value:#x} is a mode field");
        }
    }
    for forced in [FLUSHING, FLUSH_TO_ZERO, ROUNDING_UP] {
        assert_eq!(forced & !CONTROL, 0, "{forced:#x} is a mode");
    }
}

#[test]
fn writes_each_forced_mode_as_the_fields_it_names() {
    // The bits of each field as the architecture manuals give them, written apart.
    let values: Vec<&[Word]> = FIELDS.iter().map(|&(_, values)| values).collect();
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(FLUSHING, 0x1f80 | 0x8000 | 0x0040, "masks, FTZ and DAZ");
        assert_eq!(FLUSH_TO_ZERO, 0x8000, "FTZ");
        assert_eq!(ROUNDING_UP, 0x1f80 | 0x4000, "masks and rounding up");
        assert_eq!(
            (CONTROL, DEFAULT),
            (0xffc0, 0x1f80),
            "the mode fields, the masks"
        );
        let expected: [&[Word]; 4] = [
            &[0, 0x2000, 0x4000, 0x6000],
            &[0, 0x8000],
            &[0, 0x0040],
            &[0x1f80, 0],
        ];
        assert_eq!(values, expected, "the values of each field");
    }
    #[cfg(target_arch = "aarch64")]
    {
        assert_eq!(
            FLUSHING,
            0x0100_0000 | 0x0008_0000 | 0x1,
            "FZ, FZ16 and FIZ"
        );
        assert_eq!(FLUSH_TO_ZERO, 0x0100_0000, "FZ");
        assert_eq!(ROUNDING_UP, 0x0040_0000, "rounding up");
        assert_eq!(
            (CONTROL, DEFAULT),
            (0x07c8_bf07, 0),
            "the mode fields, none set"
        );
        let expected: [&[Word]; 10] = [
            &[0, 0x0040_0000, 0x0080_0000, 0x00c0_0000],
            &[0, 0x0100_0000],
            &[0, 0x1],
            &[0, 0x0008_0000],
            &[0, 0x2],
            &[0, 0x0200_0000],
            &[0, 0x0400_0000],
            &[0, 0x2000],
            &[0, 0x4],
            &[0, 0x9f00],
        ];
        assert_eq!(values, expected, "the values of each field");
    }
}

#[test]
fn proves_each_forced_mode_in_force_before_relying_on_it() {
    // Positive controls: the default keeps subnormals and rounds to nearest; a forced
    // flushing mode really flushes, a forced rounding mode really rounds up.
    let (_, default) = forced(DEFAULT, || (kept(), rounding()));
    assert_eq!(
        default,
        ((true, true), Rounding::Nearest),
        "the default mode"
    );
    let (_, flushing) = forced(FLUSHING, kept);
    assert_eq!(
        flushing,
        (false, false),
        "flush-to-zero and denormals-are-zero in force"
    );
    let (_, up) = forced(ROUNDING_UP, rounding);
    assert_eq!(up, Rounding::Up, "rounding up in force");
    // Each flush field in force on its own, flushing exactly what it defines.
    for (name, field, expected, optional, without) in FLUSH_FIELDS {
        let (set, observed) = forced(DEFAULT | field, kept);
        if set & field == field {
            assert_eq!(observed, expected, "{name} in force");
        } else {
            assert_eq!(
                field & !set,
                optional,
                "{name}: only its optional bits may be lacking"
            );
            assert_eq!(observed, without, "{name} without its optional bits");
        }
    }
    // Every rounding mode of the fields is observed as itself, in the order written apart.
    let (_, modes) = FIELDS[0];
    let observed: Vec<Rounding> = modes
        .iter()
        .map(|&mode| forced(DEFAULT & !modes[3] | mode, rounding).1)
        .collect();
    #[cfg(target_arch = "x86_64")]
    let expected = [
        Rounding::Nearest,
        Rounding::Down,
        Rounding::Up,
        Rounding::TowardZero,
    ];
    #[cfg(target_arch = "aarch64")]
    let expected = [
        Rounding::Nearest,
        Rounding::Up,
        Rounding::Down,
        Rounding::TowardZero,
    ];
    assert_eq!(observed, expected, "each rounding mode");
    assert_eq!(
        ROUNDINGS.map(|(_, rounding)| rounding),
        expected,
        "the roundings of the field, in its order"
    );
    assert_eq!(
        ROUNDINGS.map(|(value, _)| value),
        modes,
        "the values of the field, in its order"
    );
}

#[test]
fn gives_every_combination_of_the_fields_once() {
    let modes: Vec<Word> = every_host_mode().collect();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        host_modes(),
        4 * 2 * 2 * 2,
        "the product of the field counts"
    );
    #[cfg(target_arch = "aarch64")]
    assert_eq!(
        host_modes(),
        4 * 2 * 2 * 2 * 2 * 2 * 2 * 2 * 2 * 2,
        "the product of the field counts"
    );
    assert_eq!(modes.len(), host_modes(), "one mode per combination");
    assert_eq!(
        modes.iter().collect::<BTreeSet<_>>().len(),
        modes.len(),
        "each combination once"
    );
    // The first mode takes the first value of every field, the last the last; the first
    // field varies slowest, the last fastest.
    let firsts = FIELDS.iter().fold(0, |mode, (_, values)| mode | values[0]);
    let lasts = FIELDS
        .iter()
        .fold(0, |mode, (_, values)| mode | values[values.len() - 1]);
    let (_, last_field) = FIELDS[FIELDS.len() - 1];
    let (_, rounding_values) = FIELDS[0];
    assert_eq!(modes.first(), Some(&firsts), "every first value");
    assert_eq!(modes.last(), Some(&lasts), "every last value");
    assert_eq!(
        modes.get(1),
        Some(&(firsts & !last_field[0] | last_field[1])),
        "the last field varies fastest"
    );
    assert_eq!(
        modes.get(host_modes() / rounding_values.len()),
        Some(&(firsts & !rounding_values[0] | rounding_values[1])),
        "the first field varies slowest"
    );
}

#[test]
fn tells_the_modes_that_trap() {
    for safe in [DEFAULT, FLUSHING, FLUSH_TO_ZERO | DEFAULT, ROUNDING_UP] {
        assert!(!traps(safe), "{safe:#x} traps nothing");
    }
    // The last field unmasks or enables every exception: half the host modes trap.
    let (_, exceptions) = FIELDS[FIELDS.len() - 1];
    assert!(
        !traps(DEFAULT & !exceptions[0] | exceptions[0]),
        "the field's default"
    );
    assert!(
        traps(DEFAULT & !exceptions[0] | exceptions[1]),
        "every exception able to trap"
    );
    #[cfg(target_arch = "x86_64")]
    assert!(traps(DEFAULT & !0x0080), "one exception unmasked");
    #[cfg(target_arch = "aarch64")]
    assert!(traps(0x0100), "one trap enabled");
    assert_eq!(
        every_host_mode().filter(|&mode| traps(mode)).count(),
        host_modes() / 2,
        "half the host modes"
    );
}

#[test]
fn runs_in_the_default_mode_and_restores_the_host_under_every_host_mode() {
    let mut distinct = BTreeSet::new();
    for host in every_host_mode() {
        let (set, (inside, sum, after)) = forced(host, || {
            let (inside, sum) = with_default_mode(|_| {
                // A sum whose rounding and subnormal result tell the mode apart.
                let sum = black_box(black_box(f64::MIN_POSITIVE) / black_box(3.0_f64));
                (control(), sum.to_bits())
            });
            (inside, sum, control())
        });
        let _: bool = distinct.insert(set);
        assert_eq!(inside, DEFAULT, "host {host:#x}: the default inside");
        assert_eq!(
            sum,
            (f64::MIN_POSITIVE / 3.0).to_bits(),
            "host {host:#x}: computed in the default mode"
        );
        assert_eq!(after, set, "host {host:#x}: its mode restored exactly");
    }
    // The positive control of the loop: the modes it set really differed, every field
    // combining with the others as each reads back alone (a field the processor lacks reads
    // back as zero, and counts once).
    let settable: usize = FIELDS
        .iter()
        .map(|(_, values)| {
            values
                .iter()
                .map(|&value| forced(value, || ()).0)
                .collect::<BTreeSet<_>>()
                .len()
        })
        .product();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(settable, 32, "every field of MXCSR is supported");
    assert_eq!(distinct.len(), settable, "distinct host modes");
    assert_eq!(settable_host_modes(), settable, "the count the tests share");
}

#[test]
fn keeps_subnormals_inside_the_default_mode_under_a_flushing_host() {
    // The control of the flushing comparison: a flushing host flushes subnormal operands
    // and results; inside the default mode, both are kept again.
    let (set, (unguarded, guarded)) = forced(FLUSHING, || (kept(), with_default_mode(|_| kept())));
    assert_eq!(set & FLUSH_TO_ZERO, FLUSH_TO_ZERO, "flush-to-zero set");
    assert_eq!(unguarded, (false, false), "flushed by the host");
    assert_eq!(guarded, (true, true), "kept inside the default mode");
}

/// `value` rounded to the nearest binary32 by the processor, in the mode in force: the
/// conversion delivery makes of a binary64 render given as binary32.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the hardware's f64 to f32 conversion is what this observes; no From exists"
)]
fn to_binary32(value: f64) -> u32 {
    black_box(black_box(value) as f32).to_bits()
}

#[test]
fn flushes_before_rounding_to_binary32_on_aarch64_only() {
    // 2^-126 - 2^-156, which IEEE rounds to nearest up to 2^-126, the smallest normal
    // binary32 (0x0080_0000). Under flush-to-zero, x86_64 decides tininess after rounding
    // and keeps it; aarch64 (AH clear) decides it before rounding and gives +0: the one
    // value range, [2^-126 - 2^-150, 2^-126), where a host's flush changes a sample at
    // delivery, after which the engine's own flush cannot undo it.
    // With AH set (`FEAT_AFP`), aarch64 decides tininess after rounding, as x86_64 does: a
    // row observed only where a processor has AH, which an Apple M2 reads back as zero.
    let below = f64::from_bits(0x380f_ffff_f800_0000);
    let (_, ieee) = forced(DEFAULT, || to_binary32(below));
    assert_eq!(ieee, 0x0080_0000, "rounded up to 2^-126 by IEEE");
    // Each word, the bits of it a processor may lack, the binary32 given, and that given
    // without them.
    #[cfg(target_arch = "x86_64")]
    let words: [(&str, Word, Word, u32, u32); 1] = [(
        "flush-to-zero: kept, tiny only after rounding",
        FLUSH_TO_ZERO,
        0,
        0x0080_0000,
        0x0080_0000,
    )];
    #[cfg(target_arch = "aarch64")]
    let words: [(&str, Word, Word, u32, u32); 2] = [
        ("FZ: flushed, tiny before rounding", FLUSH_TO_ZERO, 0, 0, 0),
        (
            "FZ with AH: kept, tiny after rounding",
            FLUSH_TO_ZERO | 0x2,
            0x2,
            0x0080_0000,
            0,
        ),
    ];
    for (name, word, optional, expected, without) in words {
        let (set, given) = forced(DEFAULT | word, || to_binary32(below));
        assert_eq!(
            set & FLUSH_TO_ZERO,
            FLUSH_TO_ZERO,
            "{name}: flush-to-zero set"
        );
        if set & word == word {
            assert_eq!(given, expected, "{name}");
        } else {
            assert_eq!(
                word & !set,
                optional,
                "{name}: only its optional bits lacking"
            );
            assert_eq!(given, without, "{name}, without its optional bits");
        }
    }
}

#[test]
fn rounds_a_sum_up_outside_the_default_mode_only() {
    // The control of the rounding comparison: a host rounding up changes a sum; inside the
    // default mode, it rounds to nearest again.
    let sum = || black_box(black_box(1.0_f64) + black_box(f64::EPSILON / 4.0_f64)).to_bits();
    let (set, (unguarded, guarded)) = forced(ROUNDING_UP, || (sum(), with_default_mode(|_| sum())));
    assert_eq!(set & ROUNDING_UP, ROUNDING_UP, "rounding up set");
    assert_eq!(unguarded, 1.0_f64.to_bits() + 1, "rounded up by the host");
    assert_eq!(
        guarded,
        1.0_f64.to_bits(),
        "to nearest inside the default mode"
    );
}

#[test]
fn keeps_the_bits_outside_the_mode_fields() {
    // Every bit outside the mode fields is kept by a write of the mode fields: x86_64 has its
    // sticky exception flags there, the inexact one (bit 5) raised here by a division before
    // the guard and kept through it; aarch64 has only reserved bits there, all zero.
    let before = black_box(black_box(1.0_f64) / black_box(3.0_f64));
    let outside = read() & !CONTROL;
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        outside & 0x20,
        0x20,
        "the inexact flag raised: {outside:#x}"
    );
    #[cfg(target_arch = "aarch64")]
    assert_eq!(outside, 0, "only reserved bits, zero");
    let (_, inside) = forced(ROUNDING_UP, || with_default_mode(|_| read() & !CONTROL));
    assert_ne!(before.to_bits(), 0, "an inexact division");
    assert_eq!(
        inside & outside,
        outside,
        "nothing outside the mode fields is cleared"
    );
}

#[test]
fn skips_the_write_when_the_host_is_already_in_the_default_mode() {
    // Nested calls: the inner one finds the default and leaves it on entry, and writes the
    // default back on exit; only the outer one restores the host.
    let (set, (outer, inner, between)) = forced(ROUNDING_UP, || {
        with_default_mode(|_| {
            let outer = control();
            let inner = with_default_mode(|_| control());
            (outer, inner, control())
        })
    });
    assert_eq!(
        (outer, inner, between),
        (DEFAULT, DEFAULT, DEFAULT),
        "nested"
    );
    assert_eq!(set & ROUNDING_UP, ROUNDING_UP, "rounding up set");
    let (_, (inside, after)) = forced(DEFAULT, || (with_default_mode(|_| control()), control()));
    assert_eq!(
        (inside, after),
        (DEFAULT, DEFAULT),
        "a default host stays so"
    );
}

#[test]
fn restores_a_mode_changed_inside_its_body_whatever_the_host() {
    // Foreign code inside the body changes the mode (flush-to-zero here): the host's mode
    // comes back after, also when the host was already in the default mode, the one case
    // where the entry write is skipped.
    for host in [DEFAULT, ROUNDING_UP] {
        let (set, (inside, after)) = forced(host, || {
            let inside = with_default_mode(|_| {
                let inside = control();
                set_control(FLUSH_TO_ZERO);
                inside
            });
            (inside, control())
        });
        assert_eq!(inside, DEFAULT, "host {host:#x}: the default on entry");
        assert_eq!(after, set, "host {host:#x}: its mode restored exactly");
    }
}

#[test]
fn restores_the_entry_mode_after_forcing_a_host() {
    // as_host itself gives the caller its mode back: forced inside another forced mode, the
    // outer one comes back, and the default after both.
    let (outer, (inner, between)) =
        forced(ROUNDING_UP, || (forced(FLUSHING, control).1, control()));
    assert_eq!(inner & FLUSH_TO_ZERO, FLUSH_TO_ZERO, "flushing inside");
    assert_eq!(between, outer, "the outer host back");
    assert_eq!(control(), DEFAULT, "the default after both");
}

#[test]
fn restores_the_mode_found_on_entry_when_a_forced_body_unwinds() {
    // A test failing under a forced mode unwinds out of as_host: the thread gets the mode
    // it had back, never runs on in the forced one.
    let (outer, (unwound, after)) = forced(ROUNDING_UP, || {
        let unwound = std::panic::catch_unwind(|| {
            forced(FLUSHING, || std::panic::resume_unwind(Box::new(control())))
        });
        (unwound, control())
    });
    let inside = unwound
        .expect_err("it unwound")
        .downcast::<Word>()
        .expect("the mode inside");
    assert_eq!(
        *inside & FLUSH_TO_ZERO,
        FLUSH_TO_ZERO,
        "flushing while it ran"
    );
    assert_eq!(
        after, outer,
        "the mode found on entry, back after unwinding"
    );
    assert_eq!(control(), DEFAULT, "and the default after both");
}

#[test]
fn restores_the_host_on_an_early_exit_and_on_unwinding() {
    let (set, (early, after_early)) = forced(FLUSHING, || {
        let early: Result<(), String> = with_default_mode(|_| Err("refused".to_owned()));
        (early, control())
    });
    assert_eq!(early, Err("refused".to_owned()), "the error passes through");
    assert_eq!(after_early, set, "restored after an error");
    let (unwinding_set, (unwound, after_unwinding)) = forced(FLUSHING, || {
        let unwound = std::panic::catch_unwind(|| {
            with_default_mode(|_| std::panic::resume_unwind(Box::new(control())))
        });
        (unwound, control())
    });
    let inside = unwound
        .expect_err("it unwound")
        .downcast::<Word>()
        .expect("the mode inside");
    assert_eq!(*inside, DEFAULT, "the default while unwinding started");
    assert_eq!(after_unwinding, unwinding_set, "restored after unwinding");
}

#[test]
fn changes_the_mode_of_its_own_thread_only() {
    // This thread runs in the default mode while another holds a flushing mode: the other
    // reads its own mode while this one is inside, between two barriers, and neither changes
    // the other's.
    let inside = std::sync::Barrier::new(2);
    let read_there = std::sync::Barrier::new(2);
    let (set, (mine, (other_set, other_seen))) = forced(ROUNDING_UP, || {
        std::thread::scope(|scope| {
            let other = scope.spawn(|| {
                forced(FLUSHING, || {
                    let _: std::sync::BarrierWaitResult = inside.wait();
                    let seen = control();
                    let _: std::sync::BarrierWaitResult = read_there.wait();
                    seen
                })
            });
            let mine = with_default_mode(|_| {
                let _: std::sync::BarrierWaitResult = inside.wait();
                let _: std::sync::BarrierWaitResult = read_there.wait();
                control()
            });
            (mine, other.join().expect("the other thread"))
        })
    });
    assert_eq!(mine, DEFAULT, "this thread in the default mode");
    assert_eq!(
        other_set & FLUSH_TO_ZERO,
        FLUSH_TO_ZERO,
        "the other thread flushing"
    );
    assert_eq!(
        other_seen, other_set,
        "and still so while this thread was inside"
    );
    assert_eq!(set & ROUNDING_UP, ROUNDING_UP, "this thread's host mode");
}

#[test]
fn sets_the_mode_fields_exactly_whatever_they_were() {
    // Every host mode written over itself, and over the mode with every field set: the
    // mode fields read back are exactly those written, whatever bits they shared.
    let (all, ()) = forced(CONTROL, || ());
    for host in every_host_mode() {
        let (set, (again, from_all)) = forced(host, || {
            set_control(host);
            let again = control();
            set_control(CONTROL);
            set_control(host);
            (again, control())
        });
        assert_eq!(again, set, "{host:#x} over itself");
        assert_eq!(from_all, set, "{host:#x} over {all:#x}");
    }
    assert_ne!(
        all & FLUSH_TO_ZERO,
        0,
        "every field set, flush-to-zero included"
    );
}
