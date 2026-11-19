//! Tests of the conformance record: what `check` and `bless` refuse of it.

use alloc::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{
    Before, Blessing, Fingerprints, Others, Record, References, Spent, Told, Versions,
    bless_problems, blessed, current_fingerprints, record_problems, same_rules, spent_problems,
    spent_skipped,
};

/// The line of bless for battery 0 with another change: the procedure, once.
const BATTERY_ALONE: &str = "BATTERY  the fingerprints were computed with battery 0, the engine's \
                             is 2, and a battery change blesses no reference and no version \
                             (bless the battery change alone first: set the other changes aside, \
                             then run `cargo xtask bless --battery-changed`, then commit, then \
                             bring the other changes back, then run `cargo xtask bless`)";

/// No problem of a case found by `check`.
const NO_OTHERS: Others<'static> = Others {
    failed: false,
    changed: &[],
    live_changed: false,
};

/// What `bless` refuses replacing `old` and `earlier`, every version of `old` spent, as
/// `cases` and `references` now are with the fingerprints `now`, the cases `changed` named.
fn problems(
    old: &Record,
    earlier: &References,
    now: &Fingerprints,
    cases: &BTreeMap<String, Versions>,
    references: &References,
    changed: &[String],
) -> Vec<String> {
    bless_problems(
        &Before {
            record: old,
            path: Path::new("conf/algorithms.json"),
            references: earlier,
            live: None,
            spent: Spent::Replaced,
        },
        &Blessing {
            fingerprints: now,
            cases,
            references,
            live: None,
        },
        &Told {
            changed,
            live_changed: false,
            battery_changed: false,
        },
    )
}

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

/// A record of render hash 2, render rules 1 and battery 2, with `cases` and the
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
        fingerprint_battery: 2,
        fingerprints,
        render_hash: "render-hash/2".to_owned(),
        render_rules: 3,
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
    let now = fpenv::with_default_mode(current_fingerprints);
    assert_eq!(
        now.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        audio_boxes::REGISTRY
            .iter()
            .map(|spec| spec.name())
            .collect::<BTreeSet<_>>(),
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
    let fresh = blessed(cases.clone(), &now, None, None);
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
        blessed(cases.clone(), &now, Some(&earlier), None).fingerprints,
        record(&[], &[("osc~", 1, "old"), ("osc~", 2, "new")]).fingerprints,
        "history kept for the box types still registered"
    );
    let other_battery = Record {
        fingerprint_battery: 3,
        ..earlier
    };
    assert_eq!(
        blessed(cases.clone(), &now, Some(&other_battery), None).fingerprints,
        record(&[], &[("osc~", 2, "new")]).fingerprints,
        "another battery's fingerprints are not kept"
    );
    // A version spent on main keeps main's print: a record that gave it other bits (a
    // conflict resolved by hand) is mended by the bless of the new version, which check
    // then accepts against main.
    let branch = record(&[], &[("osc~", 1, "other")]);
    let main = record(&[], &[("osc~", 1, "spent"), ("gone~", 1, "x")]);
    let mended = blessed(cases.clone(), &now, Some(&branch), Some(&main));
    assert_eq!(
        mended.fingerprints,
        record(&[], &[("osc~", 1, "spent"), ("osc~", 2, "new")]).fingerprints,
        "main's prints at the versions it spent, for the box types still registered"
    );
    assert_eq!(
        spent_problems(&mended, Some(&main)),
        Vec::<String>::new(),
        "then nothing to refuse against main"
    );
    let main_other_battery = Record {
        fingerprint_battery: 1,
        ..main
    };
    assert_eq!(
        blessed(cases, &now, Some(&branch), Some(&main_other_battery)).fingerprints,
        record(&[], &[("osc~", 1, "other"), ("osc~", 2, "new")]).fingerprints,
        "a spent record of another battery gives no print"
    );
}

#[test]
fn compares_references_only_under_the_same_render_rules_and_hash() {
    let base = record(&[], &[]);
    assert!(same_rules(&base), "the current ones");
    let rules = Record {
        render_rules: 4,
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
        record_problems(&good, &now, NO_OTHERS),
        Vec::<String>::new(),
        "as recorded, older versions kept"
    );
    let changed = record(
        &[],
        &[("dac~", 1, "d"), ("osc~", 1, "other"), ("old~", 3, "x")],
    );
    assert_eq!(
        record_problems(&changed, &now, NO_OTHERS),
        [
            "BITS     osc~ 1: its outputs changed at the same algorithm version (a new version of \
             osc~, then run `cargo xtask bless`)",
            "STALE    old~: fingerprint of no box type (run `cargo xtask bless`)",
        ],
        "a changed fingerprint, a stale one"
    );
    let missing = record(&[], &[("dac~", 1, "d"), ("osc~", 2, "o")]);
    assert_eq!(
        record_problems(&missing, &now, NO_OTHERS),
        ["MISSING  osc~: no fingerprint at algorithm version 1 (run `cargo xtask bless`)"],
        "none at the current version"
    );
    let other = Record {
        render_hash: "render-hash/1".to_owned(),
        render_rules: 0,
        fingerprint_battery: 0,
        ..good.clone()
    };
    assert_eq!(
        record_problems(&other, &now, NO_OTHERS),
        [
            "HASH     the references were hashed with render-hash/1, the engine with \
             render-hash/2",
            "RULES    the references were rendered under render rules version 0, the engine's \
             is 3",
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2 (bless \
             the battery change alone first: set the other changes aside, then run `cargo \
             xtask bless --battery-changed`, then commit, then bring the other changes back, \
             then run `cargo xtask bless`)",
        ],
        "every version named without a hint while the battery differs, no fingerprint compared \
         across batteries, the battery change blessed alone"
    );
    let battery = Record {
        fingerprint_battery: 0,
        ..good
    };
    let alone = "BATTERY  the fingerprints were computed with battery 0, the engine's is 2 (run \
                 `cargo xtask bless --battery-changed`)";
    assert_eq!(
        record_problems(&battery, &now, NO_OTHERS),
        [alone],
        "the battery alone changed"
    );
    assert_eq!(
        record_problems(
            &battery,
            &now,
            Others {
                failed: true,
                changed: &[],
                live_changed: false,
            }
        ),
        [
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2 (bless \
             the battery change alone first: set the other changes aside, then run `cargo \
             xtask bless --battery-changed`, then commit, then bring the other changes back, \
             then run `cargo xtask bless`)"
        ],
        "with another problem check found"
    );
    for (hash, rules) in [("render-hash/1", 3), ("render-hash/2", 0)] {
        let with = Record {
            render_hash: hash.to_owned(),
            render_rules: rules,
            ..battery.clone()
        };
        assert!(
            record_problems(&with, &now, NO_OTHERS)
                .last()
                .is_some_and(|line| line.contains("bless the battery change alone first")),
            "with the render hash {hash} or render rules {rules}"
        );
    }
}

#[test]
fn check_blesses_a_battery_change_alone_as_bless_would() {
    let now = prints(&[("dac~", 1, "d"), ("osc~", 1, "o")]);
    let battery = Record {
        fingerprint_battery: 0,
        ..record(&[], &[("dac~", 1, "d"), ("osc~", 1, "o")])
    };
    let named = ["a".to_owned()];
    assert_eq!(
        record_problems(
            &battery,
            &now,
            Others {
                failed: true,
                changed: &named,
                live_changed: false,
            }
        ),
        [
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2 (bless \
             the battery change alone first: set the other changes aside, then run `cargo \
             xtask bless --battery-changed`, then commit, then bring the other changes back, \
             then run `cargo xtask bless --case-changed a`)"
        ],
        "with a reference changed under the same versions, which the last bless names"
    );
    let alone = "BATTERY  the fingerprints were computed with battery 0, the engine's is 2 (bless \
                 the battery change alone first: set the other changes aside, then run `cargo \
                 xtask bless --battery-changed`, then commit, then bring the other changes \
                 back, then run `cargo xtask bless`)";
    for (state, recorded, said) in [
        (
            "a box type at a version the record does not hold",
            [("dac~", 1, "d"), ("osc~", 0, "o")],
            vec!["VERSION  osc~: version 1 is not recorded", alone],
        ),
        (
            "a box type the record does not hold",
            [("osc~", 1, "o"), ("old~", 1, "x")],
            vec![
                "VERSION  dac~: version 1 is not recorded",
                alone,
                "STALE    old~: fingerprint of no box type",
            ],
        ),
    ] {
        let unrecorded = Record {
            fingerprint_battery: 0,
            ..record(&[], &recorded)
        };
        assert_eq!(
            record_problems(&unrecorded, &now, NO_OTHERS),
            said,
            "{state}, no problem of a case: the version said without a hint, then the battery \
             change blessed alone, as bless --battery-changed would refuse it"
        );
    }
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
        problems(&old, &earlier, &now, &cases, &same, &[]),
        Vec::<String>::new(),
        "nothing changed"
    );
    let moved = references(&[("a", "1"), ("b", "9"), ("new", "5")]);
    assert_eq!(
        problems(&old, &earlier, &now, &cases, &moved, &[]),
        [
            "CHANGED  b binary64: its reference changed under the same algorithm versions (a new \
             version, then run `cargo xtask bless`, or `cargo xtask bless --case-changed b`)"
        ],
        "a changed reference refused, a new case accepted"
    );
    assert_eq!(
        problems(&old, &earlier, &now, &cases, &moved, &["b".to_owned()]),
        Vec::<String>::new(),
        "the case named as changed"
    );
    assert_eq!(
        problems(
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
             version, then run `cargo xtask bless`, or `cargo xtask bless --case-changed b`)",
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
        problems(&old, &earlier, &now_bumped, &bumped, &moved, &[]),
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
            problems(&other, &earlier, &now, &cases, &moved, &[]),
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
        problems(&old, &none, &lower, &cases, &none, &[]),
        ["LOWER    osc~: at algorithm version 2, below version 3 already recorded"],
        "below the version a case was blessed with"
    );
    let fingerprinted = record(&[], &[("osc~", 4, "o"), ("osc~", 5, "p")]);
    assert_eq!(
        problems(&fingerprinted, &none, &lower, &cases, &none, &[]),
        ["LOWER    osc~: at algorithm version 2, below version 5 already recorded"],
        "below the highest version fingerprinted"
    );
    let changed = prints(&[("dac~", 1, "other"), ("osc~", 3, "o")]);
    assert_eq!(
        problems(&old, &none, &changed, &cases, &none, &[]),
        [
            "BITS     dac~ 1: its outputs changed at the same algorithm version (a new version \
             of dac~, then run `cargo xtask bless`)"
        ],
        "a changed fingerprint at a recorded version, whatever --case-changed says"
    );
    let other_battery = Record {
        fingerprint_battery: 3,
        ..old.clone()
    };
    assert_eq!(
        problems(&other_battery, &none, &changed, &cases, &none, &[]),
        [
            "VERSION  osc~: version 3 is not recorded",
            "BATTERY  the fingerprints were computed with battery 3, the engine's is 2, and a \
             battery change blesses no reference and no version (bless the battery change alone first: set the other changes aside, then run `cargo xtask \
             bless --battery-changed`, then commit, then bring the other changes back, then run \
             `cargo xtask bless`)"
        ],
        "fingerprints of another battery are not compared, and the battery change is refused, \
         to be blessed alone: osc~ 3 is a version the record does not hold"
    );
    let later_rules = Record {
        render_rules: 4,
        ..old
    };
    assert_eq!(
        problems(&later_rules, &none, &changed, &cases, &none, &[]),
        [
            "LOWER    render rules version 3: below version 4 already recorded",
            "BITS     dac~ 1: its outputs changed at the same algorithm version (a new version \
             of dac~, then run `cargo xtask bless`)",
        ],
        "render rules below those recorded"
    );
}

/// What `bless` refuses replacing `old` and its references `earlier` with `now`, `cases`
/// and `references`, the spent versions being `spent`, as `told`.
fn refused(
    old: &Record,
    earlier: &References,
    spent: Spent<'_>,
    (now, cases, references): (&Fingerprints, &BTreeMap<String, Versions>, &References),
    told: Told<'_>,
) -> Vec<String> {
    bless_problems(
        &Before {
            record: old,
            path: Path::new("conf/algorithms.json"),
            references: earlier,
            live: None,
            spent,
        },
        &Blessing {
            fingerprints: now,
            cases,
            references,
            live: None,
        },
        &told,
    )
}

/// Nothing named, the battery unchanged.
const PLAIN: Told<'static> = Told {
    changed: &[],
    live_changed: false,
    battery_changed: false,
};

#[test]
fn blesses_again_a_version_the_merge_base_did_not_spend() {
    // osc~ 2 was blessed on the branch at print A; the merge base spent only version 1.
    let base = record(&[], &[("osc~", 1, "one")]);
    let old = record(&[], &[("osc~", 1, "one"), ("osc~", 2, "A")]);
    let none = References::new();
    let cases = BTreeMap::new();
    let changed = prints(&[("osc~", 2, "B")]);
    let now = (&changed, &cases, &none);
    assert_eq!(
        refused(&old, &none, Spent::Base(Some(&base)), now, PLAIN),
        Vec::<String>::new(),
        "a version first blessed on the branch takes other bits"
    );
    assert_eq!(
        refused(&old, &none, Spent::Base(None), now, PLAIN),
        Vec::<String>::new(),
        "no record at the merge base: nothing spent"
    );
    let bits = [
        "BITS     osc~ 2: its outputs changed at the same algorithm version (a new version of \
         osc~, then run `cargo xtask bless`)",
    ];
    assert_eq!(
        refused(&old, &none, Spent::Replaced, now, PLAIN),
        bits,
        "without the merge base, every recorded version is spent"
    );
    let spent = record(&[], &[("osc~", 2, "A")]);
    assert_eq!(
        refused(&old, &none, Spent::Base(Some(&spent)), now, PLAIN),
        bits,
        "a version the merge base spent keeps its bits"
    );
    let higher = record(&[("a", versions(&[("osc~", 3)]))], &[]);
    assert_eq!(
        refused(&old, &none, Spent::Base(Some(&higher)), now, PLAIN),
        ["LOWER    osc~: at algorithm version 2, below version 3 already recorded"],
        "below a version the merge base recorded"
    );
    // The merge base's fingerprints of another battery: the spent version keeps the print
    // the replaced record holds under this one.
    let older_battery = Record {
        fingerprint_battery: 0,
        ..spent
    };
    assert_eq!(
        refused(&old, &none, Spent::Base(Some(&older_battery)), now, PLAIN),
        bits,
        "the print the battery change recorded"
    );
    let unspent = record(&[], &[("osc~", 1, "one")]);
    assert_eq!(
        refused(
            &unspent,
            &none,
            Spent::Base(Some(&older_battery)),
            now,
            PLAIN
        ),
        Vec::<String>::new(),
        "no print of the spent version under this battery"
    );
}

#[test]
fn blesses_a_battery_change_alone() {
    let now = prints(&[("osc~", 1, "new")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let earlier = references(&[("a", "1")]);
    let current = record(&[("a", versions(&[("osc~", 1)]))], &[("osc~", 1, "new")]);
    let old = Record {
        fingerprint_battery: 0,
        ..current.clone()
    };
    let battery = Told {
        changed: &[],
        live_changed: false,
        battery_changed: true,
    };
    let same = (&now, &cases, &earlier);
    assert_eq!(
        refused(&old, &earlier, Spent::Replaced, same, PLAIN),
        [
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2, and a \
             battery change blesses no reference and no version (run `cargo xtask bless \
             --battery-changed`)"
        ],
        "a battery change, not said"
    );
    assert_eq!(
        refused(&old, &earlier, Spent::Replaced, same, battery),
        Vec::<String>::new(),
        "said, and nothing else changed"
    );
    assert_eq!(
        refused(&current, &earlier, Spent::Replaced, same, battery),
        [
            "BATTERY  --battery-changed: the battery is unchanged (2) (run `cargo xtask bless` \
          without it)"
        ],
        "said of an unchanged battery"
    );
    let moved = references(&[("a", "2"), ("new", "3")]);
    let named = ["a".to_owned()];
    assert_eq!(
        refused(
            &old,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &moved),
            Told {
                changed: &named,
                live_changed: false,
                battery_changed: true,
            }
        ),
        [
            "CHANGED  a binary64: its reference changed",
            "CHANGED  new: a case the record bless replaces does not hold",
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2, and a \
             battery change blesses no reference and no version (bless the battery change alone \
             first: set the other changes aside, then run `cargo xtask bless --battery-changed`, \
             then commit, then bring the other changes back, then run `cargo xtask bless \
             --case-changed a`)",
        ],
        "no reference change, even named, and no new case, each said, then the procedure \
         once: the last bless names the case again"
    );
    let bumped = prints(&[("osc~", 2, "new")]);
    let bumped_cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 2)]))]);
    assert_eq!(
        refused(
            &old,
            &earlier,
            Spent::Replaced,
            (&bumped, &bumped_cases, &earlier),
            battery
        ),
        ["VERSION  osc~: version 2 is not recorded", BATTERY_ALONE,],
        "no new version"
    );
    // A case blessed at another version than the one the record fingerprints: its reference
    // changes are refused too, whatever its versions.
    let older_case = Record {
        cases: BTreeMap::from([("a".to_owned(), versions(&[("osc~", 0)]))]),
        ..old
    };
    assert_eq!(
        refused(
            &older_case,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &references(&[("a", "2")])),
            battery
        ),
        ["CHANGED  a binary64: its reference changed", BATTERY_ALONE,],
        "a battery change blesses no reference change, at any version; a case at other \
         versions than recorded is not named, since the plain bless takes its change"
    );
}

#[test]
fn says_once_how_to_bless_a_battery_change_alone() {
    let now = prints(&[("osc~", 1, "new")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let earlier = references(&[("a", "1")]);
    let old = Record {
        fingerprint_battery: 0,
        ..record(&[("a", versions(&[("osc~", 1)]))], &[("osc~", 1, "new")])
    };
    let battery = Told {
        changed: &[],
        live_changed: false,
        battery_changed: true,
    };
    let moved = references(&[("a", "2"), ("new", "3")]);
    assert_eq!(
        refused(
            &old,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &moved),
            PLAIN
        ),
        [
            "CHANGED  a binary64: its reference changed",
            "CHANGED  new: a case the record bless replaces does not hold",
            "BATTERY  the fingerprints were computed with battery 0, the engine's is 2, and a \
             battery change blesses no reference and no version (bless the battery change alone \
             first: set the other changes aside, then run `cargo xtask bless --battery-changed`, \
             then commit, then bring the other changes back, then run `cargo xtask bless \
             --case-changed a`)",
        ],
        "unnamed, without the option: each change said without a hint, then the procedure, \
         whose last bless names the case whose reference changed under the same versions"
    );
    let mut two = references(&[("a", "2"), ("b", "4")]);
    for hashes in two.values_mut() {
        drop(hashes.insert("binary32".to_owned(), "6".to_owned()));
    }
    let two_cases = BTreeMap::from([
        ("a".to_owned(), versions(&[("osc~", 1)])),
        ("b".to_owned(), versions(&[("osc~", 1)])),
    ]);
    let two_old = Record {
        cases: two_cases.clone(),
        ..old
    };
    let mut earlier_two = references(&[("a", "1"), ("b", "3")]);
    for hashes in earlier_two.values_mut() {
        let first = hashes.values().next().cloned().unwrap_or_default();
        drop(hashes.insert("binary32".to_owned(), first));
    }
    let lines = refused(
        &two_old,
        &earlier_two,
        Spent::Replaced,
        (&now, &two_cases, &two),
        battery,
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("bless the battery change alone first"))
            .count(),
        1,
        "the procedure said once among {lines:?}"
    );
    assert!(
        lines.last().is_some_and(|line| line
            .ends_with("then run `cargo xtask bless --case-changed a --case-changed b`)")),
        "each case named once, though two of its references changed: {lines:?}"
    );
}

#[test]
fn blesses_with_the_battery_no_reference_the_replaced_ones_lack() {
    let now = prints(&[("osc~", 1, "new")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let earlier = references(&[("a", "1")]);
    let old = Record {
        fingerprint_battery: 0,
        ..record(&[("a", versions(&[("osc~", 1)]))], &[("osc~", 1, "new")])
    };
    let battery = Told {
        changed: &[],
        live_changed: false,
        battery_changed: true,
    };
    let lost = references(&[("a", "1")]);
    let mut kept = lost.clone();
    for hashes in kept.values_mut() {
        drop(hashes.insert("binary32".to_owned(), "5".to_owned()));
    }
    assert_eq!(
        refused(&old, &lost, Spent::Replaced, (&now, &cases, &kept), battery),
        [
            "CHANGED  a binary32: no reference in the references bless replaces",
            BATTERY_ALONE,
        ],
        "no reference the replaced references lack: none to compare it with (the plain bless \
         after fills it, unnamed)"
    );
    assert_eq!(
        refused(
            &old,
            &References::new(),
            Spent::Replaced,
            (&now, &cases, &earlier),
            battery
        )
        .first()
        .map(String::as_str),
        Some("CHANGED  a binary64: no reference in the references bless replaces"),
        "nor a case the replaced references lack"
    );
}

#[test]
fn blesses_a_battery_change_apart_from_the_render_rules_and_hash() {
    let now = prints(&[("osc~", 1, "new")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let earlier = references(&[("a", "1")]);
    let moved = references(&[("a", "2"), ("new", "3")]);
    let battery = Told {
        changed: &[],
        live_changed: false,
        battery_changed: true,
    };
    let old_rules = Record {
        fingerprint_battery: 0,
        render_rules: 0,
        ..record(&[("a", versions(&[("osc~", 1)]))], &[("osc~", 1, "new")])
    };
    assert_eq!(
        refused(
            &old_rules,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &moved),
            battery
        ),
        [
            "RULES    --battery-changed: the render rules changed too, from version 0 to 3",
            BATTERY_ALONE,
        ],
        "the render rules moved with the battery: refused, whatever the references did"
    );
    let old_hash = Record {
        render_rules: 3,
        render_hash: "render-hash/1".to_owned(),
        ..old_rules
    };
    assert_eq!(
        refused(
            &old_hash,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &earlier),
            battery
        ),
        [
            "HASH     --battery-changed: the render hash changed too, from render-hash/1 to \
             render-hash/2",
            BATTERY_ALONE,
        ],
        "the render hash moved with the battery"
    );
    let both = Record {
        render_rules: 0,
        ..old_hash
    };
    assert_eq!(
        refused(
            &both,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &earlier),
            battery
        )
        .len(),
        3,
        "each named, then the procedure"
    );
    assert_eq!(
        refused(
            &both,
            &earlier,
            Spent::Replaced,
            (&now, &cases, &moved),
            PLAIN
        ),
        [
            "RULES    --battery-changed: the render rules changed too, from version 0 to 3",
            "HASH     --battery-changed: the render hash changed too, from render-hash/1 to \
             render-hash/2",
            BATTERY_ALONE,
        ],
        "without the flag, each change said, then the procedure: new rules re-bless every \
         reference, so no reference is said"
    );
}

#[test]
fn refuses_render_rules_below_either_record() {
    let now = prints(&[("osc~", 1, "o")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let earlier = references(&[("a", "1")]);
    let old = record(&[("a", versions(&[("osc~", 1)]))], &[("osc~", 1, "o")]);
    let ahead = Record {
        render_rules: 4,
        ..old.clone()
    };
    assert_eq!(
        refused(
            &old,
            &earlier,
            Spent::Base(Some(&ahead)),
            (&now, &cases, &earlier),
            PLAIN
        ),
        ["LOWER    render rules version 3: below version 4 already recorded"],
        "below the rules on main, though not below the replaced record's"
    );
    assert_eq!(
        refused(
            &old,
            &earlier,
            Spent::Base(Some(&old)),
            (&now, &cases, &earlier),
            PLAIN
        ),
        Vec::<String>::new(),
        "at the rules on main"
    );
}

#[test]
fn refuses_a_reference_whose_case_the_record_does_not_hold() {
    let now = prints(&[("osc~", 1, "o")]);
    let cases = BTreeMap::from([("a".to_owned(), versions(&[("osc~", 1)]))]);
    let old = record(&[], &[("osc~", 1, "o")]);
    let earlier = references(&[("a", "1")]);
    let moved = references(&[("a", "2")]);
    assert_eq!(
        problems(&old, &earlier, &now, &cases, &moved, &[]),
        [
            "MISSING  a: a reference but no versions recorded in the record bless replaces \
             (`cargo xtask bless --case-changed a`, or restore conf/algorithms.json from git)"
        ],
        "a case left out of the record"
    );
    assert_eq!(
        problems(&old, &earlier, &now, &cases, &moved, &["a".to_owned()]),
        Vec::<String>::new(),
        "named as changed"
    );
    assert_eq!(
        problems(&old, &References::new(), &now, &cases, &moved, &[]),
        Vec::<String>::new(),
        "a new case"
    );
}

#[test]
fn refuses_other_bits_than_main_at_a_version_both_records_hold() {
    let main = record(
        &[],
        &[("osc~", 1, "one"), ("osc~", 2, "two"), ("dac~", 1, "d")],
    );
    let kept = record(
        &[],
        &[("osc~", 1, "one"), ("osc~", 2, "two"), ("osc~", 3, "x")],
    );
    assert_eq!(
        spent_problems(&kept, Some(&main)),
        Vec::<String>::new(),
        "every version of main kept or dropped, a version of the branch added"
    );
    let resolved = record(
        &[],
        &[("osc~", 1, "one"), ("osc~", 2, "other"), ("dac~", 1, "e")],
    );
    assert_eq!(
        spent_problems(&resolved, Some(&main)),
        [
            "BITS     dac~ 1: other bits than the spent record holds at this version (a new \
             version of dac~, then run `cargo xtask bless`)",
            "BITS     osc~ 2: other bits than the spent record holds at this version (a new \
             version of osc~, then run `cargo xtask bless`)",
        ],
        "each version given other bits, by box type then version"
    );
    let both_older = |record: &Record| Record {
        fingerprint_battery: 1,
        ..record.clone()
    };
    assert_eq!(
        spent_problems(&both_older(&resolved), Some(&both_older(&main))),
        [
            "BITS     dac~ 1: other bits than the spent record holds at this version",
            "BITS     osc~ 2: other bits than the spent record holds at this version",
        ],
        "both at a battery the engine no longer has: said without a hint, which the battery \
         change would refuse"
    );
    let other_battery = Record {
        fingerprint_battery: 1,
        ..main
    };
    assert_eq!(
        spent_problems(&resolved, Some(&other_battery)),
        Vec::<String>::new(),
        "another battery: a battery change, blessed apart"
    );
    assert_eq!(
        spent_problems(&resolved, None),
        Vec::<String>::new(),
        "no spent record"
    );
}

#[test]
fn says_when_the_record_on_main_cannot_be_compared_with() {
    let here = record(&[], &[("osc~", 1, "one")]);
    let older = Record {
        fingerprint_battery: 1,
        ..here.clone()
    };
    assert_eq!(
        spent_skipped(&here, Some(&older)),
        Some(
            "skipped  the spent record: its fingerprints were computed with battery 1, these \
             with battery 2 (compared once the battery change reaches main)"
                .to_owned()
        ),
        "another battery on main"
    );
    assert_eq!(spent_skipped(&here, Some(&here)), None, "the same battery");
    assert_eq!(spent_skipped(&here, None), None, "no spent record");
}
