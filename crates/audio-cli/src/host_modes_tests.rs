//! Tests of the engine under the floating-point modes a host may set: every conformance
//! case renders its reference hashes under a flushing host and under every host mode inside
//! the default mode, the default mode is what gives them back, and the engine's own
//! observation of the arithmetic sees each forced mode as `fpenv` does.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use audio_core::{RenderFormat, Renderer, SubnormalArithmetic};
use audio_hash::Samples;
use fpenv::host::{
    FLUSH_TO_ZERO, FLUSHING, ROUNDING_UP, as_host, every_host_mode, host_modes, subnormals_kept,
    traps,
};
use fpenv::with_default_mode;

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
            &patch,
            registry,
            rate,
            audio_core::ExecutionMode::Offline,
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
                &patch,
                registry,
                rate,
                audio_core::ExecutionMode::Offline,
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
fn renders_the_subnormal_case_to_its_references_inside_the_default_mode_under_every_host_mode() {
    let (case, patch, references) = subnormal_case();
    let mut distinct = BTreeSet::new();
    for host in every_host_mode() {
        let (set, hashes) = as_host(host, || {
            with_default_mode(|_| RenderFormat::ALL.map(|format| render(&case, &patch, format)))
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
    // The positive control of the loop: the host modes really differed.
    assert!(
        distinct.len() > 1 && distinct.len() <= host_modes(),
        "distinct host modes: {}",
        distinct.len()
    );
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
    // The control: the modes flushed operands, results and both, and kept them under others.
    assert!(
        flushed.contains(&(false, false)),
        "both flushed under a flushing mode"
    );
    assert!(
        flushed.contains(&(true, true)),
        "both kept under the default"
    );
}
