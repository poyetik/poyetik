//! Tests of the default mode: forced host modes, each first proven in force, every
//! conformance case rendered under them, the host's mode restored, nesting, early exits,
//! unwinding and threads.

#![forbid(unsafe_code)]

extern crate alloc;
extern crate std;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box;

use audio_core::{RenderFormat, Renderer, SubnormalArithmetic, SubnormalsKept};
use audio_hash::Samples;

use super::with_default_mode;
use super::word::{CONTROL, DEFAULT, Word, read, set_control};

/// The fields of the control register, by name, with the values they take (written into
/// their bits): every rounding mode, flushes, traps and the other mode fields.
#[cfg(target_arch = "x86_64")]
const FIELDS: [(&str, &[Word]); 4] = [
    // Round to nearest, down, up, toward zero.
    ("rounding", &[0, 0x2000, 0x4000, 0x6000]),
    ("flush-to-zero", &[0, 0x8000]),
    ("denormals-are-zero", &[0, 0x0040]),
    // Every exception masked (the default), then every one unmasked (traps).
    ("exception masks", &[0x1f80, 0]),
];

/// The fields of the control register, by name, with the values they take (written into
/// their bits): every rounding mode, flushes, traps and the other mode fields.
#[cfg(target_arch = "aarch64")]
const FIELDS: [(&str, &[Word]); 10] = [
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

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets.
#[cfg(target_arch = "x86_64")]
const FLUSHING: Word = 0x1f80 | 0x8000 | 0x0040;

/// Flush-to-zero and denormals-are-zero at once, the mode a plugin host sets: FZ, FIZ and
/// FZ16.
#[cfg(target_arch = "aarch64")]
const FLUSHING: Word = 0x0100_0000 | 0x1 | 0x0008_0000;

/// The flush-to-zero bit, which every processor of the target supports.
#[cfg(target_arch = "x86_64")]
const FLUSH_TO_ZERO: Word = 0x8000;

/// The flush-to-zero bit (FZ), which every processor of the target supports (FIZ is
/// optional: an Apple M2 reads it back as zero).
#[cfg(target_arch = "aarch64")]
const FLUSH_TO_ZERO: Word = 0x0100_0000;

/// Rounding up (toward +∞), every exception masked.
#[cfg(target_arch = "x86_64")]
const ROUNDING_UP: Word = 0x1f80 | 0x4000;

/// Rounding up (toward +∞).
#[cfg(target_arch = "aarch64")]
const ROUNDING_UP: Word = 0x0040_0000;

/// The mode fields of the control register now.
fn control() -> Word {
    read() & CONTROL
}

/// Runs `body` with the host's mode fields set to `host`, and gives what it returned with
/// the mode fields read back right after setting them (a field the processor does not
/// support reads back as zero). Restores the mode found on entry afterwards, before any
/// assertion: the caller's floating point runs in the mode it had.
///
/// Rust code under a forced mode is outside Rust's floating-point rule (RFC 3514; see the
/// audit of the control-register file): `body` passes through `black_box` after the write
/// and its result before the restore, as in `with_default_mode`, so that its float work
/// reading its captures or memory runs in the forced mode.
fn as_host<R>(host: Word, body: impl FnOnce() -> R) -> (Word, R) {
    let entry = control();
    set_control(host);
    let set = control();
    let body = black_box(body);
    let result = black_box(body());
    set_control(entry);
    (set, result)
}

/// Each flush field alone, with what [`subnormals_kept`] observes under it, and whether a
/// processor of the target may lack it (it then reads back as zero). On `x86_64`, FTZ
/// flushes results only and DAZ operands only; on aarch64 (AH clear), FZ flushes both and
/// FIZ operands only.
#[cfg(target_arch = "x86_64")]
const FLUSH_FIELDS: [(&str, Word, (bool, bool), bool); 3] = [
    ("flush-to-zero alone", 0x8000, (true, false), false),
    ("denormals-are-zero alone", 0x0040, (false, true), false),
    ("both", 0x8000 | 0x0040, (false, false), false),
];

/// Each flush field alone, with what [`subnormals_kept`] observes under it, and whether a
/// processor of the target may lack it (it then reads back as zero). On `x86_64`, FTZ
/// flushes results only and DAZ operands only; on aarch64 (AH clear), FZ flushes both and
/// FIZ operands only (FIZ is optional: an Apple M2 reads it back as zero).
#[cfg(target_arch = "aarch64")]
const FLUSH_FIELDS: [(&str, Word, (bool, bool), bool); 2] = [
    ("FZ alone", 0x0100_0000, (false, false), false),
    ("FIZ alone", 0x1, (false, true), true),
];

/// Whether subnormals survive as operands and as results, `(operands kept, results kept)`,
/// as `audio-core`'s observation of the arithmetic sees them (the probe a browser's engine
/// reports, proven here under every mode forced). A flush field covers both formats, so the
/// binary32 and binary64 observations must agree.
fn subnormals_kept() -> (bool, bool) {
    let observed = SubnormalArithmetic::observe();
    let pair = |kept: SubnormalsKept| (kept.operands, kept.results);
    assert_eq!(
        pair(observed.binary64),
        pair(observed.binary32),
        "binary64 observed like binary32"
    );
    pair(observed.binary32)
}

/// The direction of rounding observed: `1 + 1.5 ulp` gives `1 + 2 ulp` to nearest (a tie,
/// to even) and up, `1 + ulp` toward zero and down; `1 + 2^-60` gives `1 + ulp` only up;
/// `-1 - 2^-60` gives `-1 - ulp` only down.
fn rounding() -> &'static str {
    let ulp = f64::EPSILON;
    let tie = black_box(black_box(1.0_f64) + black_box(1.5_f64 * ulp)).to_bits();
    let below = black_box(black_box(-1.0_f64) - black_box(libm_free_tiny())).to_bits();
    let above = black_box(black_box(1.0_f64) + black_box(libm_free_tiny())).to_bits();
    let one = 1.0_f64.to_bits();
    let minus_one = (-1.0_f64).to_bits();
    match (
        tie == one.wrapping_add(2),
        below == minus_one.wrapping_add(1),
        above == one.wrapping_add(1),
    ) {
        (true, false, false) => "nearest",
        (true, false, true) => "up",
        (false, true, false) => "down",
        (false, false, false) => "toward zero",
        _ => "unknown",
    }
}

/// 2^-60, written as its bits.
const fn libm_free_tiny() -> f64 {
    f64::from_bits((1_023 - 60) << 52)
}

/// The conformance directory of the repository.
fn conformance() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

/// The reference hashes of a case, by the name of the render format.
type Hashes = BTreeMap<String, String>;

/// Every conformance case with its patch and reference hashes: `(name, case, patch,
/// hashes)`.
fn corpus() -> Vec<(String, String, String, Hashes)> {
    let dir = conformance();
    let references: BTreeMap<String, Hashes> = serde_json::from_str(
        &std::fs::read_to_string(dir.join("references.json")).expect("the references"),
    )
    .expect("a map of hashes by format");
    let cases: Vec<_> = references
        .into_iter()
        .map(|(name, hash)| {
            let read = |folder: &str| {
                std::fs::read_to_string(dir.join(folder).join(format!("{name}.json")))
                    .expect("a case file")
            };
            (name.clone(), read("cases"), read("patches"), hash)
        })
        .collect();
    assert!(cases.len() >= 13, "the whole corpus: {}", cases.len());
    cases
}

/// The output stage a case names (`output`: a mode, a lookahead and a detector), for
/// `channels` channels at `rate`: raw when it names none, as `audio-cli` renders it.
fn stage<S: audio_core::Sample>(
    case: &serde_json::Value,
    rate: audio_core::SampleRate,
    channels: usize,
) -> audio_core::OutputStage<S> {
    let output = &case["output"];
    if output.is_null() || output["mode"] == "raw" {
        return audio_core::OutputStage::new();
    }
    let lookahead = <audio_core::Binary64 as audio_core::host::Raw>::from_raw(
        output["lookahead"].as_f64().expect("a lookahead"),
    );
    let detector = match output["detector"].as_str().expect("a detector") {
        "standard" => audio_core::Detector::Standard,
        "precise" => audio_core::Detector::Precise,
        _ => audio_core::Detector::Mastering,
    };
    let settings = audio_core::LimiterSettings::DEFAULT
        .with_lookahead(lookahead)
        .expect("a lookahead in range");
    audio_core::OutputStage::limited(
        audio_core::TruePeakLimiter::new(rate, channels, lookahead, detector, settings, true)
            .expect("a limiter"),
    )
}

/// The render hash of a case in `format`, as `audio-cli` renders it: its patch parsed, its
/// plan built and rendered through its output stage, its samples hashed. Parsing and every
/// float operation happen in the mode of the caller.
fn render(case: &str, patch: &str, format: RenderFormat) -> String {
    let case: serde_json::Value = serde_json::from_str(case).expect("a case");
    let named = case.clone();
    let rate = audio_core::SampleRate::new(
        u32::try_from(case["sample_rate"].as_u64().expect("a rate")).expect("u32"),
    )
    .expect("a valid rate");
    let frames = usize::try_from(case["frames"].as_u64().expect("frames")).expect("usize");
    let patch = audio_patch::parse_patch(patch).expect("a valid patch");
    let registry = &audio_boxes::REGISTRY;
    let wide = || {
        audio_core::Plan::<_, audio_boxes::BuiltIn<audio_core::Binary64>>::build(
            &patch, registry, rate,
        )
        .expect("a plan")
    };
    let hash = |channels, samples| {
        audio_hash::render_hash(rate, channels, samples, audio_core::MODE)
            .expect("whole frames")
            .to_string()
    };
    match format {
        RenderFormat::Binary64 => {
            let plan = wide();
            let channels = plan.channels();
            let stage = stage(&named, rate, channels);
            let samples: Vec<f64> = Renderer::with_stage(plan, stage)
                .render(frames)
                .expect("a render");
            hash(channels, Samples::Binary64(&samples))
        }
        RenderFormat::Binary64AsBinary32 => {
            let plan = wide();
            let channels = plan.channels();
            let stage = stage(&named, rate, channels);
            let samples: Vec<f32> = Renderer::with_stage(plan, stage)
                .render(frames)
                .expect("a render");
            hash(channels, Samples::Binary64AsBinary32(&samples))
        }
        RenderFormat::Binary32 => {
            let plan = audio_core::Plan::<_, audio_boxes::BuiltIn<audio_core::Binary32>>::build(
                &patch, registry, rate,
            )
            .expect("a plan");
            let channels = plan.channels();
            let stage = stage(&named, rate, channels);
            let samples: Vec<f32> = Renderer::with_stage(plan, stage)
                .render(frames)
                .expect("a render");
            hash(channels, Samples::Binary32(&samples))
        }
    }
}

/// The reference hash of `format` among `hashes`.
fn reference(hashes: &Hashes, format: RenderFormat) -> &str {
    hashes
        .get(format.name())
        .expect("a reference of every format")
}

/// The case of the corpus whose sine starts subnormal, short enough to render many times.
fn subnormal_case() -> (String, String, Hashes) {
    corpus()
        .into_iter()
        .find(|(name, ..)| name == "sine-1.1754944e-38-at-8k")
        .map(|(_, case, patch, hash)| (case, patch, hash))
        .expect("the subnormal case")
}

/// Every combination of the values of [`FIELDS`], as mode fields.
fn every_host_mode() -> Vec<Word> {
    FIELDS.iter().fold(vec![0], |modes, (_, values)| {
        modes
            .iter()
            .flat_map(|mode| values.iter().map(move |value| mode | value))
            .collect()
    })
}

#[test]
fn the_default_is_a_mode_of_the_control_fields() {
    assert_eq!(DEFAULT & !CONTROL, 0, "the default sets mode fields only");
    for (name, values) in FIELDS {
        for value in values {
            assert_eq!(value & !CONTROL, 0, "{name}: {value:#x} is a mode field");
        }
    }
}

#[test]
fn proves_each_forced_mode_in_force_before_relying_on_it() {
    // Positive controls: the default keeps subnormals and rounds to nearest; a forced
    // flushing mode really flushes, a forced rounding mode really rounds up.
    let (_, default) = as_host(DEFAULT, || (subnormals_kept(), rounding()));
    assert_eq!(default, ((true, true), "nearest"), "the default mode");
    let (_, flushing) = as_host(FLUSHING, subnormals_kept);
    assert_eq!(
        flushing,
        (false, false),
        "flush-to-zero and denormals-are-zero in force"
    );
    let (_, up) = as_host(ROUNDING_UP, rounding);
    assert_eq!(up, "up", "rounding up in force");
    // Each flush field in force on its own, flushing exactly what it defines.
    for (name, field, expected, optional) in FLUSH_FIELDS {
        let (set, kept) = as_host(DEFAULT | field, subnormals_kept);
        if set & field == field {
            assert_eq!(kept, expected, "{name} in force");
        } else {
            assert!(
                optional,
                "{name} supported by every processor of the target"
            );
            assert_eq!(set & field, 0, "{name} unsupported: none of its bits");
            assert_eq!(kept, (true, true), "{name} unsupported: nothing flushed");
        }
    }
    // Every rounding mode of the fields is observed as itself.
    let (_, modes) = FIELDS[0];
    let observed: Vec<&str> = modes
        .iter()
        .map(|&mode| as_host(DEFAULT & !modes[3] | mode, rounding).1)
        .collect();
    #[cfg(target_arch = "x86_64")]
    let expected = ["nearest", "down", "up", "toward zero"];
    #[cfg(target_arch = "aarch64")]
    let expected = ["nearest", "up", "down", "toward zero"];
    assert_eq!(observed, expected, "each rounding mode");
}

#[test]
fn runs_in_the_default_mode_and_restores_the_host_under_every_host_mode() {
    let (case, patch, references) = subnormal_case();
    let modes = every_host_mode();
    assert_eq!(
        modes.len(),
        FIELDS
            .iter()
            .map(|(_, values)| values.len())
            .product::<usize>(),
        "every combination"
    );
    let mut distinct = BTreeSet::new();
    for host in modes {
        let (set, (inside, hashes, after)) = as_host(host, || {
            let (inside, hashes) = with_default_mode(|_| {
                let hashes = RenderFormat::ALL.map(|format| render(&case, &patch, format));
                (control(), hashes)
            });
            (inside, hashes, control())
        });
        let _: bool = distinct.insert(set);
        assert_eq!(inside, DEFAULT, "host {host:#x}: the default inside");
        for (format, hash) in RenderFormat::ALL.into_iter().zip(hashes) {
            assert_eq!(
                hash,
                reference(&references, format),
                "host {host:#x}: the reference hash of {format}"
            );
        }
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
                .map(|&value| as_host(value, || ()).0)
                .collect::<BTreeSet<_>>()
                .len()
        })
        .product();
    #[cfg(target_arch = "x86_64")]
    assert_eq!(settable, 32, "every field of MXCSR is supported");
    assert_eq!(distinct.len(), settable, "distinct host modes");
}

#[test]
fn renders_every_conformance_case_to_its_reference_under_a_flushing_host() {
    for (name, case, patch, references) in corpus() {
        for format in RenderFormat::ALL {
            let (set, hash) = as_host(FLUSHING, || {
                with_default_mode(|_| render(&case, &patch, format))
            });
            assert_eq!(
                set & FLUSH_TO_ZERO,
                FLUSH_TO_ZERO,
                "{name}: flush-to-zero set"
            );
            assert_eq!(
                hash,
                reference(&references, format),
                "{name} {format}: under a flushing host"
            );
        }
    }
    // In force: the same mode really flushes.
    let (_, kept) = as_host(FLUSHING, subnormals_kept);
    assert_eq!(kept, (false, false), "subnormals flushed in that mode");
}

#[test]
fn needs_the_default_mode_to_give_the_references() {
    // The control of the conformance comparison: without the default mode, a host rounding
    // up changes the render of a sine; inside it, the reference comes back.
    let (_, case, patch, references) = corpus()
        .into_iter()
        .find(|(name, ..)| name == "sine-440")
        .expect("sine-440");
    for format in RenderFormat::ALL {
        let (_, (unguarded, guarded)) = as_host(ROUNDING_UP, || {
            (
                render(&case, &patch, format),
                with_default_mode(|_| render(&case, &patch, format)),
            )
        });
        let expected = reference(&references, format);
        assert_ne!(
            unguarded, expected,
            "{format}: rounding up changes the render"
        );
        assert_eq!(
            guarded, expected,
            "{format}: the default mode gives it back"
        );
    }
}

#[test]
fn keeps_subnormals_inside_the_default_mode_under_a_flushing_host() {
    // The control of the flushing comparison: a flushing host flushes subnormal operands
    // and results; inside the default mode, both are kept again.
    let (set, (unguarded, guarded)) = as_host(FLUSHING, || {
        (subnormals_kept(), with_default_mode(|_| subnormals_kept()))
    });
    assert_eq!(set & FLUSH_TO_ZERO, FLUSH_TO_ZERO, "flush-to-zero set");
    assert_eq!(unguarded, (false, false), "flushed by the host");
    assert_eq!(guarded, (true, true), "kept inside the default mode");
}

#[test]
fn gives_the_subnormal_case_its_reference_under_a_flushing_host_even_unguarded() {
    // Delivery flushes every sample subnormal in the format given (decision 11), so the
    // case whose binary64 samples round to binary32 subnormals renders its reference given
    // as binary32 under a flushing host, with or without the default mode: the hardware's
    // flush has nothing left to change at delivery.
    let (case, patch, references) = subnormal_case();
    let format = RenderFormat::Binary64AsBinary32;
    let (set, (unguarded, guarded)) = as_host(FLUSHING, || {
        (
            render(&case, &patch, format),
            with_default_mode(|_| render(&case, &patch, format)),
        )
    });
    assert_eq!(set & FLUSH_TO_ZERO, FLUSH_TO_ZERO, "flush-to-zero set");
    let expected = reference(&references, format);
    assert_eq!(
        unguarded, expected,
        "a flushing host changes no delivered bit"
    );
    assert_eq!(guarded, expected, "nor does the default mode");
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
    let (_, inside) = as_host(ROUNDING_UP, || with_default_mode(|_| read() & !CONTROL));
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
    let (set, (outer, inner, between)) = as_host(ROUNDING_UP, || {
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
    let (_, (inside, after)) = as_host(DEFAULT, || (with_default_mode(|_| control()), control()));
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
        let (set, (inside, after)) = as_host(host, || {
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
fn restores_the_host_on_an_early_exit_and_on_unwinding() {
    let (set, (early, after_early)) = as_host(FLUSHING, || {
        let early: Result<(), String> = with_default_mode(|_| Err("refused".to_owned()));
        (early, control())
    });
    assert_eq!(early, Err("refused".to_owned()), "the error passes through");
    assert_eq!(after_early, set, "restored after an error");
    let (unwinding_set, (unwound, after_unwinding)) = as_host(FLUSHING, || {
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
    let (set, (mine, (other_set, other_seen))) = as_host(ROUNDING_UP, || {
        std::thread::scope(|scope| {
            let other = scope.spawn(|| {
                as_host(FLUSHING, || {
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
    let (all, ()) = as_host(CONTROL, || ());
    for host in every_host_mode() {
        let (set, (again, from_all)) = as_host(host, || {
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
