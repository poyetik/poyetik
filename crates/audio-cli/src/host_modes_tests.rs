//! Tests of the engine under the floating-point modes a host may set: every conformance
//! case renders its reference hashes under a flushing host and under every host mode inside
//! the default mode, the default mode is what gives them back, and the engine's own
//! observation of the arithmetic sees each forced mode as `fpenv` does.

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
    prepare_unproven(case, patch, format)
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
fn renders_every_conformance_case_to_its_reference_under_a_flushing_host() {
    for (name, case, patch, references) in corpus() {
        for format in RenderFormat::ALL {
            let (set, hash) = as_host(FLUSHING, || {
                with_default_mode(|mode| render_in(&case, &patch, format, mode))
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
    let (_, kept) = as_host(FLUSHING, || SubnormalArithmetic::observe().binary64);
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
        let (_, (unguarded, guarded)) = as_host(ROUNDING_UP, || {
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
            with_default_mode(|mode| render_in(&case, &patch, format, mode)),
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
fn renders_the_subnormal_case_to_its_references_inside_the_default_mode_under_every_host_mode() {
    let (case, patch, references) = subnormal_case();
    let mut distinct = BTreeSet::new();
    for host in every_host_mode() {
        let (set, hashes) = as_host(host, || {
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
            as_host(host, || (SubnormalArithmetic::observe(), subnormals_kept()));
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
        let reads_back = |bit: fpenv::host::Word| as_host(bit, || ()).0 == bit;
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
