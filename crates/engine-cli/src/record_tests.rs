//! Tests of the conformance record: what `check` and `bless` refuse of it.

use alloc::collections::BTreeMap;

use super::{
    Fingerprints, Record, References, Versions, bless_problems, blessed, current_fingerprints,
    record_problems, same_rules,
};

/// The versions of `boxes`, by name.
fn versions(boxes: &[(&str, u32)]) -> Versions {
    boxes
        .iter()
        .map(|(name, version)| ((*name).to_owned(), *version))
        .collect()
}

/// Fingerprints of `boxes`, each at a version with a print.
fn prints(boxes: &[(&str, u32, &str)]) -> Fingerprints {
    boxes
        .iter()
        .map(|(name, version, print)| ((*name).to_owned(), (*version, (*print).to_owned())))
        .collect()
}

/// A record of render hash 2, render rules 1 and battery 1, with `cases` and the
/// fingerprints `recorded` (box type, version, print).
fn record(cases: &[(&str, Versions)], recorded: &[(&str, u32, &str)]) -> Record {
    let mut fingerprints: BTreeMap<String, BTreeMap<u32, String>> = BTreeMap::new();
    for (name, version, print) in recorded {
        drop(
            fingerprints
                .entry((*name).to_owned())
                .or_default()
                .insert(*version, (*print).to_owned()),
        );
    }
    Record {
        cases: cases
            .iter()
            .map(|(name, boxes)| ((*name).to_owned(), boxes.clone()))
            .collect(),
        fingerprint_battery: 1,
        fingerprints,
        render_hash: "render-hash/2".to_owned(),
        render_rules: 1,
    }
}

/// References of `cases`, each a case with its hash in binary64.
fn references(cases: &[(&str, &str)]) -> References {
    cases
        .iter()
        .map(|(name, hash)| {
            (
                (*name).to_owned(),
                BTreeMap::from([("binary64".to_owned(), (*hash).to_owned())]),
            )
        })
        .collect()
}

#[test]
fn fingerprints_every_registered_box_type_at_its_version() {
    let now = engine_fpenv::with_default_mode(current_fingerprints);
    assert_eq!(
        now.keys().map(String::as_str).collect::<Vec<_>>(),
        ["dac~", "osc~"],
        "every box type of the registry"
    );
    for (name, (version, print)) in &now {
        assert_eq!(*version, 1, "{name}: at version 1");
        assert_eq!(print.len(), 64, "{name}: a SHA-256");
    }
    assert_ne!(now["dac~"].1, now["osc~"].1, "each its own");
}

#[test]
fn records_the_current_engine_and_keeps_the_fingerprints_of_its_battery() {
    let now = prints(&[("osc~", 2, "new")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 2)]))]);
    let fresh = blessed(cases.clone(), &now, None);
    assert_eq!(
        fresh,
        record(&[("a", versions(&[("osc~", 2)]))], &[("osc~", 2, "new")]),
        "the current versions, render hash, render rules and battery"
    );
    // Older versions of a registered box type are kept, a box type no longer registered
    // is dropped, the current version's print replaces a recorded one.
    let earlier = record(
        &[],
        &[("osc~", 1, "old"), ("osc~", 2, "stale"), ("gone~", 1, "x")],
    );
    assert_eq!(
        blessed(cases.clone(), &now, Some(&earlier)).fingerprints,
        record(&[], &[("osc~", 1, "old"), ("osc~", 2, "new")]).fingerprints,
        "history kept for the box types still registered"
    );
    let other_battery = Record {
        fingerprint_battery: 2,
        ..earlier
    };
    assert_eq!(
        blessed(cases, &now, Some(&other_battery)).fingerprints,
        record(&[], &[("osc~", 2, "new")]).fingerprints,
        "another battery's fingerprints are not kept"
    );
}

#[test]
fn compares_references_only_under_the_same_render_rules_and_hash() {
    let base = record(&[], &[]);
    assert!(same_rules(&base), "the current ones");
    let rules = Record {
        render_rules: 2,
        ..base.clone()
    };
    assert!(!same_rules(&rules), "other render rules");
    let hash = Record {
        render_hash: "render-hash/1".to_owned(),
        ..base
    };
    assert!(!same_rules(&hash), "another render hash");
}

#[test]
fn check_refuses_every_difference_from_the_current_engine() {
    let now = prints(&[("dac~", 1, "d"), ("osc~", 1, "o")]);
    let good = record(&[], &[("dac~", 1, "d"), ("osc~", 1, "o"), ("osc~", 0, "z")]);
    assert_eq!(
        record_problems(&good, &now),
        Vec::<String>::new(),
        "as recorded, older versions kept"
    );
    let changed = record(
        &[],
        &[("dac~", 1, "d"), ("osc~", 1, "other"), ("old~", 3, "x")],
    );
    assert_eq!(
        record_problems(&changed, &now),
        [
            "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version \
             needed)",
            "STALE    old~: fingerprint of no box type",
        ],
        "a changed fingerprint, a stale one"
    );
    let missing = record(&[], &[("dac~", 1, "d"), ("osc~", 2, "o")]);
    assert_eq!(
        record_problems(&missing, &now),
        ["MISSING  osc~: no fingerprint at algorithm version 1 (run bless)"],
        "none at the current version"
    );
    let other = Record {
        render_hash: "render-hash/1".to_owned(),
        render_rules: 0,
        fingerprint_battery: 0,
        ..good
    };
    assert_eq!(
        record_problems(&other, &now),
        [
            "HASH     the references were hashed with render-hash/1, the engine with \
             render-hash/2 (run bless)",
            "RULES    the references were rendered under render rules version 0, the engine's \
             is 1 (run bless)",
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 1 (run \
             bless)",
        ],
        "every version named, no fingerprint compared across batteries"
    );
}

#[test]
fn bless_refuses_bits_changed_without_a_new_version() {
    let now = prints(&[("osc~", 1, "o")]);
    let cases = BTreeMap::from([
        ("a".to_owned(), versions(&[("osc~", 1)])),
        ("b".to_owned(), versions(&[("osc~", 1)])),
    ]);
    let old = record(
        &[
            ("a", versions(&[("osc~", 1)])),
            ("b", versions(&[("osc~", 1)])),
        ],
        &[("osc~", 1, "o")],
    );
    let earlier = references(&[("a", "1"), ("b", "2")]);
    let same = references(&[("a", "1"), ("b", "2")]);
    assert_eq!(
        bless_problems(&old, &earlier, &now, &cases, &same, &[]),
        Vec::<String>::new(),
        "nothing changed"
    );
    let moved = references(&[("a", "1"), ("b", "9"), ("new", "5")]);
    assert_eq!(
        bless_problems(&old, &earlier, &now, &cases, &moved, &[]),
        [
            "CHANGED  b binary64: its reference changed under the same algorithm versions (a new \
          version, or --case-changed b)"
        ],
        "a changed reference refused, a new case accepted"
    );
    assert_eq!(
        bless_problems(&old, &earlier, &now, &cases, &moved, &["b".to_owned()]),
        Vec::<String>::new(),
        "the case named as changed"
    );
    assert_eq!(
        bless_problems(
            &old,
            &earlier,
            &now,
            &cases,
            &moved,
            &["a".to_owned(), "c".to_owned()]
        ),
        [
            "UNKNOWN  --case-changed c: no such case",
            "CHANGED  b binary64: its reference changed under the same algorithm versions (a new \
             version, or --case-changed b)",
        ],
        "only the case named, which must be a case"
    );
    // A new version of the box type the case uses, other render rules or another render
    // hash: the reference may change.
    let bumped = BTreeMap::from([
        ("a".to_owned(), versions(&[("osc~", 1)])),
        ("b".to_owned(), versions(&[("osc~", 2)])),
    ]);
    let now_bumped = prints(&[("osc~", 2, "p")]);
    assert_eq!(
        bless_problems(&old, &earlier, &now_bumped, &bumped, &moved, &[]),
        Vec::<String>::new(),
        "a new version"
    );
    for other in [
        Record {
            render_rules: 0,
            ..old.clone()
        },
        Record {
            render_hash: "render-hash/1".to_owned(),
            ..old
        },
    ] {
        assert_eq!(
            bless_problems(&other, &earlier, &now, &cases, &moved, &[]),
            Vec::<String>::new(),
            "{other:?}"
        );
    }
}

#[test]
fn bless_refuses_lower_versions_and_changed_fingerprints() {
    let old = record(
        &[("a", versions(&[("dac~", 1), ("osc~", 3)]))],
        &[("dac~", 1, "d"), ("osc~", 2, "o")],
    );
    let cases = BTreeMap::new();
    let none = References::new();
    let lower = prints(&[("dac~", 1, "d"), ("osc~", 2, "o")]);
    assert_eq!(
        bless_problems(&old, &none, &lower, &cases, &none, &[]),
        ["LOWER    osc~: at algorithm version 2, below version 3 already recorded"],
        "below the version a case was blessed with"
    );
    let fingerprinted = record(&[], &[("osc~", 4, "o"), ("osc~", 5, "p")]);
    assert_eq!(
        bless_problems(&fingerprinted, &none, &lower, &cases, &none, &[]),
        ["LOWER    osc~: at algorithm version 2, below version 5 already recorded"],
        "below the highest version fingerprinted"
    );
    let changed = prints(&[("dac~", 1, "other"), ("osc~", 3, "o")]);
    assert_eq!(
        bless_problems(&old, &none, &changed, &cases, &none, &[]),
        [
            "BITS     dac~ 1: its outputs changed at the same algorithm version (a new version \
          needed)"
        ],
        "a changed fingerprint at a recorded version, whatever --case-changed says"
    );
    let other_battery = Record {
        fingerprint_battery: 2,
        ..old.clone()
    };
    assert_eq!(
        bless_problems(&other_battery, &none, &changed, &cases, &none, &[]),
        Vec::<String>::new(),
        "fingerprints of another battery are not compared"
    );
    let later_rules = Record {
        render_rules: 2,
        ..old
    };
    assert_eq!(
        bless_problems(&later_rules, &none, &changed, &cases, &none, &[]),
        [
            "LOWER    render rules version 1: below version 2 already recorded",
            "BITS     dac~ 1: its outputs changed at the same algorithm version (a new version \
             needed)",
        ],
        "render rules below those recorded"
    );
}
