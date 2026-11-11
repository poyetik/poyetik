//! What the conformance references were blessed under, `algorithms.json`
//! (`spec/engine-design.md`, decision 29): the render hash version, the render rules
//! version, the fingerprint of every box type at each algorithm version blessed, and the
//! algorithm version of every box type each case uses. From it, what `check` and `bless`
//! refuse.

use crate::status::Status;
use alloc::collections::BTreeMap;

use engine_battery::{FINGERPRINT_BATTERY, fingerprint};
use engine_core::RENDER_RULES_VERSION;
use engine_hash::SCHEME;
use engine_objects::REGISTRY;
use serde::{Deserialize, Serialize};

/// The reference hashes of a conformance directory: for each case, the hash of each render
/// format, by the format's name.
pub(crate) type References = BTreeMap<String, BTreeMap<String, String>>;

/// The algorithm version of each box type a case uses, by the box type's name.
pub(crate) type Versions = BTreeMap<String, u32>;

/// The fingerprint of every registered box type at its current algorithm version: the
/// version and the fingerprint, by the box type's name.
pub(crate) type Fingerprints = BTreeMap<String, (u32, String)>;

/// The record of a conformance directory, `algorithms.json`, as `bless` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    /// The algorithm version of every box type each case uses, by case.
    pub(crate) cases: BTreeMap<String, Versions>,
    /// The version of the battery the fingerprints were computed with
    /// ([`FINGERPRINT_BATTERY`]).
    pub(crate) fingerprint_battery: u32,
    /// The fingerprint of each box type at every algorithm version blessed, by box type
    /// then version: the older versions kept, so that a version number is never given
    /// other bits.
    pub(crate) fingerprints: BTreeMap<String, BTreeMap<u32, String>>,
    /// The render hash version the references were computed with ([`SCHEME`]).
    pub(crate) render_hash: String,
    /// The render rules version the references were rendered under
    /// ([`RENDER_RULES_VERSION`]).
    pub(crate) render_rules: u32,
}

/// The fingerprint of every registered box type at its current algorithm version.
pub(crate) fn current_fingerprints() -> Fingerprints {
    REGISTRY
        .iter()
        .map(|spec| {
            (
                spec.name.to_owned(),
                (spec.algorithm_version.get(), fingerprint(spec)),
            )
        })
        .collect()
}

/// The record `bless` writes: the versions of the current engine, `cases`, and the
/// fingerprints of `earlier` (the record it replaces) computed with the same battery,
/// for the box types still registered, with `now`'s added.
pub(crate) fn blessed(
    cases: BTreeMap<String, Versions>,
    now: &Fingerprints,
    earlier: Option<&Record>,
) -> Record {
    let mut fingerprints: BTreeMap<String, BTreeMap<u32, String>> = earlier
        .filter(|record| record.fingerprint_battery == FINGERPRINT_BATTERY)
        .map(|record| record.fingerprints.clone())
        .unwrap_or_default();
    fingerprints.retain(|name, _| now.contains_key(name));
    for (name, (version, print)) in now {
        drop(
            fingerprints
                .entry(name.clone())
                .or_default()
                .insert(*version, print.clone()),
        );
    }
    Record {
        cases,
        fingerprint_battery: FINGERPRINT_BATTERY,
        fingerprints,
        render_hash: SCHEME.to_owned(),
        render_rules: RENDER_RULES_VERSION,
    }
}

/// Whether the references of `record` were rendered and hashed under the current render
/// rules and render hash: only then can a reference that changed under the same box
/// versions be said to have changed without a new version.
pub(crate) fn same_rules(record: &Record) -> bool {
    record.render_hash == SCHEME && record.render_rules == RENDER_RULES_VERSION
}

/// The problems `check` reports of `record` against the current engine, whose
/// fingerprints are `now`: another render hash, render rules or battery version, a
/// fingerprint missing or changed at a recorded version, or one of no box type.
pub(crate) fn record_problems(record: &Record, now: &Fingerprints) -> Vec<String> {
    let mut problems = Vec::new();
    if record.render_hash != SCHEME {
        problems.push(format!(
            "{status} the references were hashed with {}, the engine with {SCHEME} (run bless)",
            record.render_hash,
            status = Status::Hash
        ));
    }
    if record.render_rules != RENDER_RULES_VERSION {
        problems.push(format!(
            "{status} the references were rendered under render rules version {}, the \
             engine's is {RENDER_RULES_VERSION} (run bless)",
            record.render_rules,
            status = Status::Rules
        ));
    }
    if record.fingerprint_battery == FINGERPRINT_BATTERY {
        for (name, (version, print)) in now {
            match record
                .fingerprints
                .get(name)
                .and_then(|versions| versions.get(version))
            {
                Some(recorded) if recorded == print => {}
                Some(_) => problems.push(changed_bits(name, *version)),
                None => problems.push(format!(
                    "{status} {name}: no fingerprint at algorithm version {version} (run bless)",
                    status = Status::Missing
                )),
            }
        }
    } else {
        problems.push(format!(
            "{status} the fingerprints were computed with battery {}, the engine's is \
             {FINGERPRINT_BATTERY} (run bless)",
            record.fingerprint_battery,
            status = Status::Battery
        ));
    }
    problems.extend(
        record
            .fingerprints
            .keys()
            .filter(|name| !now.contains_key(*name))
            .map(|name| {
                format!(
                    "{status} {name}: fingerprint of no box type",
                    status = Status::Stale
                )
            }),
    );
    problems
}

/// The line of a box type whose outputs changed at algorithm version `version`.
fn changed_bits(name: &str, version: u32) -> String {
    format!(
        "{status} {name} {version}: its outputs changed at the same algorithm version (a new \
         version needed)",
        status = Status::Bits
    )
}

/// What `bless` refuses to write in place of `old` and `earlier` (the record and the
/// references read from the directory), the current engine's fingerprints being `now`,
/// its cases' versions `cases` and their references `references`: a box type, or the
/// render rules, at a lower version than recorded; a fingerprint changed at a recorded
/// version under the same battery; a reference of a case that changed while its box
/// types' versions, the render rules and the render hash all stayed as recorded, unless
/// the case is named in `changed` (`--case-changed`), whose every name must be a case.
pub(crate) fn bless_problems(
    old: &Record,
    earlier: &References,
    now: &Fingerprints,
    cases: &BTreeMap<String, Versions>,
    references: &References,
    changed: &[String],
) -> Vec<String> {
    let mut problems: Vec<String> = changed
        .iter()
        .filter(|name| !references.contains_key(*name))
        .map(|name| {
            format!(
                "{status} --case-changed {name}: no such case",
                status = Status::Unknown
            )
        })
        .collect();
    if RENDER_RULES_VERSION < old.render_rules {
        problems.push(format!(
            "{status} render rules version {RENDER_RULES_VERSION}: below version {} already \
             recorded",
            old.render_rules,
            status = Status::Lower
        ));
    }
    for (name, (version, print)) in now {
        let recorded = old.fingerprints.get(name);
        let highest = recorded
            .and_then(|versions| versions.keys().next_back().copied())
            .into_iter()
            .chain(
                old.cases
                    .values()
                    .filter_map(|boxes| boxes.get(name).copied()),
            )
            .max();
        if let Some(highest) = highest.filter(|highest| highest > version) {
            problems.push(format!(
                "{status} {name}: at algorithm version {version}, below version {highest} \
                 already recorded",
                status = Status::Lower
            ));
        }
        if old.fingerprint_battery == FINGERPRINT_BATTERY
            && recorded
                .and_then(|versions| versions.get(version))
                .is_some_and(|before| before != print)
        {
            problems.push(changed_bits(name, *version));
        }
    }
    if same_rules(old) {
        for (case, hashes) in references {
            if old.cases.get(case) != cases.get(case) || changed.contains(case) {
                continue;
            }
            for (format, hash) in hashes {
                if earlier
                    .get(case)
                    .and_then(|before| before.get(format))
                    .is_some_and(|before| before != hash)
                {
                    problems.push(format!(
                        "{status} {case} {format}: its reference changed under the same \
                         algorithm versions (a new version, or --case-changed {case})",
                        status = Status::Changed
                    ));
                }
            }
        }
    }
    problems
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
