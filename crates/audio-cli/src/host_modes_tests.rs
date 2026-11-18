//! Tests of the engine under the floating-point modes a host may set: every conformance
//! case renders its reference hashes under a flushing host with or without the default mode
//! (the engine flushes subnormals itself), and under every host mode inside the default
//! mode, which a host rounding up shows is needed; the limiter's live changes in a release
//! tail render alike under a flushing host; and the engine's own observation of the
//! arithmetic sees each forced mode as `fpenv` does.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use audio_core::{RenderFormat, SubnormalArithmetic};
use fpenv::host::{
    FLUSH_TO_ZERO, FLUSHING, ROUNDING_UP, as_host, every_host_mode, settable_host_modes,
    subnormals_kept, traps,
};
use fpenv::{DefaultMode, with_default_mode};

use crate::case::prepare_unproven;
use crate::prepare_case;

/// [`as_host`], the one call of it in this file: the forced mode is test support, which
/// Clippy refuses anywhere else (`clippy.toml`), so this expectation also proves the lint
/// fires.
#[expect(
    clippy::disallowed_methods,
    reason = "these tests render under host modes on purpose, through this helper only"
)]
fn forced<R>(host: fpenv::host::Word, body: impl FnOnce() -> R) -> (fpenv::host::Word, R) {
    as_host(host, body)
}

/// The conformance directory of the repository.
fn conformance() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

/// The reference hashes of a case, by the name of the render format.
type Hashes = BTreeMap<String, String>;

/// Every conformance case with its patch and reference hashes: `(name, case, patch,
/// hashes)`, one for each case file of the corpus.
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
    let files = std::fs::read_dir(dir.join("cases"))
        .expect("the case files")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("an entry")
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .count();
    assert_eq!(
        cases.len(),
        files,
        "the whole corpus: a reference per case file"
    );
    cases
}

/// The render hash of a case in `format`, as `audio-cli` renders it (its own case reader,
/// plan and output stage), in the mode of the caller: no default mode set.
fn render(case: &str, patch: &str, format: RenderFormat) -> String {
    prepare_unproven(case, patch, format, &audio_boxes::REGISTRY)
        .expect("a valid case")
        .render_unproven()
        .hash()
        .expect("whole frames")
        .to_string()
}

/// The render hash of a case in `format`, as every command of `audio-cli` renders it: in
/// the default mode `mode` proves.
fn render_in(case: &str, patch: &str, format: RenderFormat, mode: DefaultMode<'_>) -> String {
    prepare_case(case, patch, format, mode)
        .expect("a valid case")
        .render(mode)
        .hash()
        .expect("whole frames")
        .to_string()
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

#[test]
fn renders_every_conformance_case_to_its_reference_under_a_flushing_host_guarded_or_not() {
    // The engine flushes subnormals itself, at every flush point, delivery included
    // (decision 11), so a flushing host changes no bit of any case of the corpus, with or
    // without the default mode: the subnormal case whose binary64 samples round to binary32
    // subnormals too. That the default mode clears the flush fields is shown where it can
    // be seen, by fpenv's `keeps_subnormals_inside_the_default_mode_under_a_flushing_host`.
    // On aarch64, a flush can change a sample at the rounding to binary32, before delivery
    // flushes (fpenv's `flushes_before_rounding_to_binary32_on_aarch64_only`): no case of
    // the corpus has a sample in that range, so this holds of this corpus, not of every
    // render.
    for (name, case, patch, references) in corpus() {
        for format in RenderFormat::ALL {
            let (set, (unguarded, guarded)) = forced(FLUSHING, || {
                (
                    render(&case, &patch, format),
                    with_default_mode(|mode| render_in(&case, &patch, format, mode)),
                )
            });
            assert_eq!(
                set & FLUSH_TO_ZERO,
                FLUSH_TO_ZERO,
                "{name}: flush-to-zero set"
            );
            let expected = reference(&references, format);
            assert_eq!(
                unguarded, expected,
                "{name} {format}: under a flushing host"
            );
            assert_eq!(
                guarded, expected,
                "{name} {format}: in the default mode under a flushing host"
            );
        }
    }
    // In force: the same mode really flushes.
    let (_, kept) = forced(FLUSHING, || SubnormalArithmetic::observe().binary64);
    assert_eq!(
        (kept.operands, kept.results),
        (false, false),
        "subnormals flushed in that mode"
    );
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
        let (_, (unguarded, guarded)) = forced(ROUNDING_UP, || {
            (
                render(&case, &patch, format),
                with_default_mode(|mode| render_in(&case, &patch, format, mode)),
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
fn renders_the_subnormal_case_to_its_references_inside_the_default_mode_under_every_host_mode() {
    let (case, patch, references) = subnormal_case();
    let mut distinct = BTreeSet::new();
    for host in every_host_mode() {
        let (set, hashes) = forced(host, || {
            with_default_mode(|mode| {
                RenderFormat::ALL.map(|format| render_in(&case, &patch, format, mode))
            })
        });
        let _: bool = distinct.insert(set);
        for (format, hash) in RenderFormat::ALL.into_iter().zip(hashes) {
            assert_eq!(
                hash,
                reference(&references, format),
                "host {host:#x}: the reference hash of {format}"
            );
        }
    }
    // The positive control of the loop: it set every mode the processor can hold.
    assert_eq!(distinct.len(), settable_host_modes(), "distinct host modes");
}

#[test]
fn observes_the_arithmetic_as_fpenv_does_under_every_host_mode() {
    // The engine's observation (the probe a browser's engine reports) against fpenv's,
    // under every host mode that computes without trapping: each flush field seen alike, in
    // both formats.
    let pair = |kept: audio_core::SubnormalsKept| (kept.operands, kept.results);
    let mut flushed = BTreeSet::new();
    for host in every_host_mode().filter(|&mode| !traps(mode)) {
        let (_, (engine, own)) =
            forced(host, || (SubnormalArithmetic::observe(), subnormals_kept()));
        let kept = |kept: fpenv::host::SubnormalsKept| (kept.operands, kept.results);
        assert_eq!(
            [pair(engine.binary64), pair(engine.binary32)],
            [kept(own.binary64), kept(own.binary32)],
            "host {host:#x}: the same observation"
        );
        let _: bool = flushed.insert(pair(engine.binary64));
    }
    // The control: the loop observed every flush the fields can make, each alone, both and
    // none, as `(operands kept, results kept)`. On x86_64, FTZ flushes results only and DAZ
    // operands only.
    #[cfg(target_arch = "x86_64")]
    let expected = BTreeSet::from([(false, false), (false, true), (true, false), (true, true)]);
    // On aarch64, FZ flushes both while AH is clear, and FIZ operands only; with AH set, FZ
    // flushes results only. FIZ (bit 0) and AH (bit 1) are optional (`FEAT_AFP`): a
    // processor without them reads them back as zero, and only FZ flushes. Review only: no
    // Mac that runs this test has had `FEAT_AFP` (an Apple M2 has not), so its two pairs are
    // the Arm manual's, not yet observed.
    #[cfg(target_arch = "aarch64")]
    let expected = {
        let reads_back = |bit: fpenv::host::Word| forced(bit, || ()).0 == bit;
        let mut expected = BTreeSet::from([(false, false), (true, true)]);
        if reads_back(0x1) {
            let _: bool = expected.insert((false, true));
        }
        if reads_back(0x2) {
            let _: bool = expected.insert((true, false));
        }
        expected
    };
    assert_eq!(flushed, expected, "every flush the fields make, and none");
}

/// The stage [`release_tail`] renders through: a limited one of binary64, given as computed,
/// in blocks of 64 frames.
type Stage = audio_core::OutputStage<audio_core::Binary64, f64, 64>;

/// The limiter's settings of [`release_tail`]: a ceiling of -1 dBTP, a lookahead of
/// `lookahead` milliseconds, a release of 10 ms and no adaptivity.
fn tail_settings(lookahead: f64) -> audio_core::LimiterSettings {
    use audio_core::Binary64;
    audio_core::LimiterSettings::new(
        Binary64::new(-1.0_f64),
        Binary64::new(lookahead),
        Binary64::new(10.0_f64),
        Binary64::new(0.0_f64),
    )
    .expect("settings in range")
}

/// The limiter's output stage alone, protected or switchable (`mode`), at `rate`, its
/// lookahead `from` milliseconds of a 5 ms maximum ([`tail_settings`]), on one channel,
/// `first` applied to it before its first block: peaks of 2.0 for twice the maximum, then
/// 0.25, which needs nothing, for 707 releases (the release stages would fall by `e^-707`,
/// to about `2^-1020`, where steps and shares are subnormal; the engine keeps none below
/// `2^-100`), when `then` is applied to
/// it; then 0.25 for four maxima, peaks of 2.0 for two and 0.25 for two more, the latency
/// and a block. Every sample given, as bits, in the mode of the caller.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "frame counts of a few hundred thousand, far from overflow"
)]
fn release_tail(
    (rate, mode, from): (u32, audio_core::LimiterMode, f64),
    first: impl FnOnce(&mut Stage),
    then: impl FnOnce(&mut Stage),
) -> Vec<u64> {
    use audio_core::host::Raw;
    use audio_core::{Binary64, Detector, SampleRate};
    let maximum = usize::try_from(rate / 200).expect("small");
    let limiter = audio_core::TruePeakLimiter::new(
        SampleRate::new(rate).expect("a rate"),
        1,
        Binary64::new(5.0_f64),
        Detector::Standard,
        tail_settings(from),
        mode,
    )
    .expect("a limiter");
    let mut stage = Stage::limited(limiter);
    first(&mut stage);
    let burst = 2 * maximum;
    let tail = usize::try_from(rate / 100 * 707).expect("small");
    let second = burst + tail + 4 * maximum;
    let end = second + 4 * maximum + stage.latency() + 64;
    let mut then = Some(then);
    let mut given = Vec::new();
    for start in (0..end).step_by(64) {
        if start >= burst + tail
            && let Some(then) = then.take()
        {
            then(&mut stage);
        }
        let blocks = stage.apply(|blocks| {
            for (offset, sample) in blocks[0].iter_mut().enumerate() {
                let frame = start + offset;
                let loud = frame < burst || (second..second + burst).contains(&frame);
                *sample = Binary64::new(if loud { 2.0_f64 } else { 0.25_f64 });
            }
        });
        given.extend(blocks[0].iter().map(|sample| sample.bits()));
    }
    given
}

#[test]
fn renders_a_live_change_in_a_release_tail_alike_under_a_flushing_host() {
    // A live lookahead change (growing and shrinking) and a crossfade between raw and
    // protected (each way), each made at the end of a release tail of 707 releases, where a
    // control path keeping every value would compute subnormal ones: a host that flushes
    // subnormal numbers to zero gives every sample the same bits as one that keeps them,
    // since the engine's control path keeps no value below `2^-100` and so computes none
    // (decision 11). In the tail every
    // reduction gives a gain of exactly 1, the input passed on unchanged whether it is
    // applied or not; the burst after it is what shows a host that left a stage running,
    // or ended it otherwise, limited differently.
    // At the two lowest rates of the test matrix, an integer number of frames per
    // millisecond and not: the tail lasts 707 releases, whose frames grow with the rate, and
    // audio-core's law of the same values holds at every rate, and its differential compares
    // the control state itself (`computes_no_subnormal_value_through_a_release_to_zero`,
    // `differs_on_no_host_through_a_release_tail`).
    use audio_core::LimiterMode;
    for rate in [8_000_u32, 11_025] {
        for (from, to) in [(0.5_f64, 5.0_f64), (5.0_f64, 0.5_f64)] {
            let render = || {
                release_tail(
                    (rate, LimiterMode::Protected, from),
                    |_| (),
                    |stage| assert_eq!(stage.set_limiter(tail_settings(to)), Ok(()), "live"),
                )
            };
            let (_, flushed) = forced(FLUSHING, render);
            assert_eq!(
                flushed,
                render(),
                "{rate} Hz, from {from} to {to} ms: the same bits"
            );
        }
        for protected in [true, false] {
            let render = || {
                release_tail(
                    (rate, LimiterMode::Switchable, 1.5_f64),
                    |stage| assert_eq!(stage.set_protected(!protected), Ok(()), "first"),
                    |stage| assert_eq!(stage.set_protected(protected), Ok(()), "then"),
                )
            };
            let (_, flushed) = forced(FLUSHING, render);
            assert_eq!(
                flushed,
                render(),
                "{rate} Hz, toward protected {protected}: the same bits"
            );
        }
    }
}
