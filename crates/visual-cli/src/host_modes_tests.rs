//! Tests of the CPU reference under the floating-point modes a host may set
//! (`spec/visual-design.md`, the decision on subnormals and -0): every visual conformance
//! case gives its reference hash under every host mode that lets float work run, a
//! flushing host's and every rounding direction included, with nothing set by the
//! renderer. An image holds no subnormal and no -0 and is computed from integers by exact
//! operations, so no mode can move a bit. The display encoding gives the display corpus's
//! bytes under every one of those modes too: its comparisons and exact products leave a mode
//! nothing to round or flush that changes a code.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use fpenv::host::{
    FLUSH_TO_ZERO, FLUSHING, ROUNDING_UP, as_host, every_host_mode, host_modes, traps,
};

use crate::display::{display_text, first_difference};
use crate::{display_corpus, hash_case};

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

/// Every visual conformance case, its text and its reference hash: `(name, text, hash)`.
fn corpus() -> Vec<(String, String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/visual");
    let references: BTreeMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(dir.join("references.json")).expect("the references"),
    )
    .expect("a map of hashes by case");
    references
        .into_iter()
        .map(|(name, hash)| {
            let text = std::fs::read_to_string(dir.join("cases").join(format!("{name}.json")))
                .expect("a case file");
            (name, text, hash)
        })
        .collect()
}

/// The pixels a case renders: its width × height × images.
fn pixels(text: &str) -> u64 {
    let case: serde_json::Value = serde_json::from_str(text).expect("a case");
    ["width", "height", "images"]
        .iter()
        .map(|field| case[field].as_u64().expect("a count"))
        .product()
}

/// The largest work, in pixels, of a case rendered under every host mode: the cases below
/// it (an odd size, a column, a square without side, the smallest image), about 10 000
/// pixels in all, render under each of the hundreds of modes in seconds (measured: 1 << 16
/// takes a minute, 2026-11-16).
const SMALL_CASE: u64 = 1 << 13;

#[test]
fn renders_every_visual_case_to_its_reference_under_a_flushing_host() {
    // A plugin host's mode, flush-to-zero and denormals-are-zero, over the whole corpus,
    // the largest image included.
    let cases = corpus();
    assert!(cases.len() >= 11, "the whole corpus: {}", cases.len());
    for (name, text, reference) in &cases {
        let (set, hash) = forced(FLUSHING, || hash_case(text).map(|hash| hash.to_string()));
        assert_eq!(
            set & FLUSH_TO_ZERO,
            FLUSH_TO_ZERO,
            "{name}: flush-to-zero set"
        );
        assert_eq!(hash.as_deref(), Ok(reference.as_str()), "{name}");
    }
}

#[test]
fn renders_the_small_visual_cases_to_their_references_under_every_host_mode() {
    let cases: Vec<_> = corpus()
        .into_iter()
        .filter(|(_, text, _)| pixels(text) < SMALL_CASE)
        .collect();
    assert!(cases.len() >= 4, "the small cases: {}", cases.len());
    let modes: Vec<_> = every_host_mode().filter(|&mode| !traps(mode)).collect();
    assert!(
        modes.len() > 1 && modes.len() < host_modes(),
        "the modes that let float work run, some of them only: {} of {}",
        modes.len(),
        host_modes()
    );
    let mut set_modes = BTreeSet::new();
    for mode in modes {
        for (name, text, reference) in &cases {
            let (set, hash) = forced(mode, || hash_case(text).map(|hash| hash.to_string()));
            let _: bool = set_modes.insert(set);
            assert_eq!(
                hash.as_deref(),
                Ok(reference.as_str()),
                "{name} under the host mode {mode:#x} (set: {set:#x})"
            );
        }
    }
    assert!(
        set_modes
            .iter()
            .any(|&set| set & FLUSH_TO_ZERO == FLUSH_TO_ZERO)
            && set_modes
                .iter()
                .any(|&set| set & ROUNDING_UP == ROUNDING_UP),
        "a flushing host and a rounding direction other than the default among the modes set"
    );
}

#[test]
fn encodes_the_display_corpus_to_its_bytes_under_every_host_mode() {
    // The corpus is built in the default mode (its colors are products of floats, which a
    // rounding direction would move), then encoded under each mode.
    let pixels = display_corpus();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/visual");
    assert_eq!(
        first_difference(
            &std::fs::read_to_string(dir.join("display.json")).expect("the display corpus"),
            &display_text(&pixels).expect("encoded")
        ),
        None,
        "the repository's display corpus, in the default mode"
    );
    let encode = || -> Vec<[u8; 4]> {
        pixels
            .iter()
            .map(|&[red, green, blue, alpha]| visual_core::pixel8(red, green, blue, alpha))
            .collect()
    };
    let expected = encode();
    let mut set_modes = BTreeSet::new();
    for mode in every_host_mode().filter(|&mode| !traps(mode)) {
        let (set, bytes) = forced(mode, encode);
        let _: bool = set_modes.insert(set);
        let differing = bytes
            .iter()
            .zip(&expected)
            .position(|(got, want)| got != want);
        assert_eq!(
            differing, None,
            "the first pixel encoded otherwise under the host mode {mode:#x} (set: {set:#x})"
        );
    }
    assert!(
        set_modes
            .iter()
            .any(|&set| set & FLUSH_TO_ZERO == FLUSH_TO_ZERO)
            && set_modes
                .iter()
                .any(|&set| set & ROUNDING_UP == ROUNDING_UP),
        "a flushing host and a rounding direction other than the default among the modes set"
    );
}
