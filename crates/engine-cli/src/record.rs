//! What the conformance references were blessed under, `algorithms.json`
//! (`spec/engine-design.md`, decision 29): the render hash version, the render rules
//! version, the fingerprint of every box type at each algorithm version blessed, and the
//! algorithm version of every box type each case uses. From it, what `check` and `bless`
//! refuse.

use crate::hint::Hint;
use crate::status::Status;
use alloc::collections::BTreeMap;
use std::path::Path;

use engine_battery::{FINGERPRINT_BATTERY, fingerprint};
use engine_core::{Binary32, Binary64, RENDER_RULES_VERSION};
use engine_fpenv::DefaultMode;
use engine_hash::SCHEME;
use engine_objects::{BuiltIn, REGISTRY};
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

/// The fingerprint of every registered box type at its current algorithm version, rendered
/// in the default floating-point mode (`mode`, the proof of it), as every recorded
/// fingerprint was: a host mode that flushed subnormals could give other bits.
pub(crate) fn current_fingerprints(_mode: DefaultMode<'_>) -> Fingerprints {
    REGISTRY
        .iter()
        .map(|spec| {
            (
                spec.name.to_owned(),
                (
                    spec.algorithm_version.get(),
                    fingerprint::<BuiltIn<Binary64>, BuiltIn<Binary32>>(spec),
                ),
            )
        })
        .collect()
}

/// The record `bless` writes: the versions of the current engine, `cases`, and the
/// fingerprints of `earlier` (the record it replaces) computed with the same battery, those
/// of `base` (the record on `main`, `--spent`) at the versions it spent replacing them, for
/// the box types still registered, with `now`'s added.
///
/// A version spent on `main` keeps `main`'s bits (`spec/engine-design.md`, decision 29), so
/// a replaced record that gave one other bits (a conflict resolved by hand) is mended by the
/// bless of the new version that change needs, which `check` then accepts against `main`.
pub(crate) fn blessed(
    cases: BTreeMap<String, Versions>,
    now: &Fingerprints,
    earlier: Option<&Record>,
    base: Option<&Record>,
) -> Record {
    let of_this_battery = |record: &&Record| record.fingerprint_battery == FINGERPRINT_BATTERY;
    let mut fingerprints: BTreeMap<String, BTreeMap<u32, String>> = earlier
        .filter(of_this_battery)
        .map(|record| record.fingerprints.clone())
        .unwrap_or_default();
    for (name, versions) in base
        .filter(of_this_battery)
        .iter()
        .flat_map(|base| &base.fingerprints)
    {
        fingerprints.entry(name.clone()).or_default().extend(
            versions
                .iter()
                .map(|(version, print)| (*version, print.clone())),
        );
    }
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
/// fingerprint missing or changed at a recorded version, or one of no box type. `others`
/// says whether check found a problem of a case: a battery change with it, or with other
/// render rules or another render hash, is blessed alone first.
pub(crate) fn record_problems(record: &Record, now: &Fingerprints, others: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if record.render_hash != SCHEME {
        problems.push(format!(
            "{status} the references were hashed with {}, the engine with {SCHEME} ({hint})",
            record.render_hash,
            status = Status::Hash,
            hint = Hint::Hash
        ));
    }
    if record.render_rules != RENDER_RULES_VERSION {
        problems.push(format!(
            "{status} the references were rendered under render rules version {}, the \
             engine's is {RENDER_RULES_VERSION} ({hint})",
            record.render_rules,
            status = Status::Rules,
            hint = Hint::Rules
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
                Some(_) => problems.push(changed_bits(name, *version, Hint::BitsChanged { name })),
                None => problems.push(format!(
                    "{status} {name}: no fingerprint at algorithm version {version} ({hint})",
                    status = Status::Missing,
                    hint = Hint::NoFingerprint
                )),
            }
        }
    } else {
        let others = others || !same_rules(record);
        problems.push(format!(
            "{status} the fingerprints were computed with battery {}, the engine's is \
             {FINGERPRINT_BATTERY} ({hint})",
            record.fingerprint_battery,
            status = Status::Battery,
            hint = if others {
                Hint::BatteryAlone { changed: &[] }
            } else {
                Hint::Battery
            }
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

/// The problems `check` reports of `record` against `base`, the record on `main`: a box
/// type whose fingerprint differs from the one `base` holds at the same version, a version
/// spent on `main` given other bits (`spec/engine-design.md`, decision 29), whatever way
/// `record` was written (a conflict resolved by hand included). None when `base` is none or
/// its fingerprints were computed with another battery: a battery change is blessed apart
/// (and [`spent_skipped`] says so).
pub(crate) fn spent_problems(record: &Record, base: Option<&Record>) -> Vec<String> {
    let Some(base) = base.filter(|base| base.fingerprint_battery == record.fingerprint_battery)
    else {
        return Vec::new();
    };
    base.fingerprints
        .iter()
        .flat_map(|(name, versions)| {
            versions
                .iter()
                .filter(|(version, print)| {
                    record
                        .fingerprints
                        .get(name)
                        .and_then(|recorded| recorded.get(*version))
                        .is_some_and(|recorded| recorded != *print)
                })
                .map(move |(version, _)| {
                    format!(
                        "{status} {name} {version}: other bits than the record on main holds at \
                         this spent version ({hint})",
                        status = Status::Bits,
                        hint = Hint::SpentBits { name }
                    )
                })
        })
        .collect()
}

/// The line `check` prints when it cannot compare `record` with `base`, the record on
/// `main`: their fingerprints were computed with other batteries, so no version can be
/// compared, until the battery change reaches `main` (`spec/engine-design.md`, decision
/// 29). It reports no problem: it says that this comparison did not run.
pub(crate) fn spent_skipped(record: &Record, base: Option<&Record>) -> Option<String> {
    base.filter(|base| base.fingerprint_battery != record.fingerprint_battery)
        .map(|base| {
            format!(
                "{status} the record on main: its fingerprints were computed with battery {}, \
                 these with battery {} (compared once the battery change reaches main)",
                base.fingerprint_battery,
                record.fingerprint_battery,
                status = Status::Skipped
            )
        })
}

/// The line of a box type whose outputs changed at algorithm version `version`, with what
/// to do, `hint`.
fn changed_bits(name: &str, version: u32, hint: Hint<'_>) -> String {
    format!(
        "{status} {name} {version}: its outputs changed at the same algorithm version ({hint})",
        status = Status::Bits
    )
}

/// Which algorithm versions are spent: a version whose fingerprint may never change
/// (`spec/engine-design.md`, decision 29: a number is spent when it reaches `main`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Spent<'record> {
    /// Every version of the record `bless` replaces: `engine-cli bless` alone, which knows
    /// no merge base.
    Replaced,
    /// The versions of the record at the merge base (`--spent`, which `cargo xtask bless`
    /// gives), none when it had no record: a version first blessed on the branch may be
    /// blessed again with other bits, until it reaches `main`.
    Base(Option<&'record Record>),
}

/// What `bless` compares the current engine with: the record and the references it
/// replaces, and which versions are spent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Before<'before> {
    /// The record it replaces.
    pub(crate) record: &'before Record,
    /// Where that record is, which a hint names.
    pub(crate) path: &'before Path,
    /// The references it replaces.
    pub(crate) references: &'before References,
    /// Which versions are spent.
    pub(crate) spent: Spent<'before>,
}

/// What `bless` would write: the current engine's fingerprints, the versions of its cases'
/// box types and their references.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Blessing<'now> {
    /// The fingerprint of every registered box type at its current version.
    pub(crate) fingerprints: &'now Fingerprints,
    /// The versions of the box types of each case.
    pub(crate) cases: &'now BTreeMap<String, Versions>,
    /// The reference hashes of each case.
    pub(crate) references: &'now References,
}

/// What `bless` was told: the cases whose references changed on purpose
/// (`--case-changed`), and whether the battery changed (`--battery-changed`), both
/// "Review only" in decision 29.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Told<'told> {
    /// The cases named by `--case-changed`.
    pub(crate) changed: &'told [String],
    /// Whether `--battery-changed` was given.
    pub(crate) battery_changed: bool,
}

/// The highest version of `name` recorded in `record`, by a fingerprint or by a case.
fn highest(record: &Record, name: &str) -> Option<u32> {
    record
        .fingerprints
        .get(name)
        .and_then(|versions| versions.keys().next_back().copied())
        .into_iter()
        .chain(
            record
                .cases
                .values()
                .filter_map(|boxes| boxes.get(name).copied()),
        )
        .max()
}

/// The fingerprint `name` must keep at `version`, if that version is spent: the spent
/// record's, when computed with the current battery; else, the version being spent, the
/// replaced record's, which the bless that changed the battery computed with this one
/// while it refused every new version.
fn kept<'record>(before: &Before<'record>, name: &str, version: u32) -> Option<&'record String> {
    let at = |record: &'record Record| {
        (record.fingerprint_battery == FINGERPRINT_BATTERY)
            .then(|| record.fingerprints.get(name)?.get(&version))
            .flatten()
    };
    match before.spent {
        Spent::Replaced => at(before.record),
        Spent::Base(None) => None,
        Spent::Base(Some(base)) => base
            .fingerprints
            .get(name)
            .is_some_and(|versions| versions.contains_key(&version))
            .then(|| at(base).or_else(|| at(before.record)))
            .flatten(),
    }
}

/// The names of `told.changed` that are no case of `references`: refused whatever bless
/// compares with.
pub(crate) fn unknown_cases(told: &Told<'_>, references: &References) -> Vec<String> {
    told.changed
        .iter()
        .filter(|name| !references.contains_key(*name))
        .map(|name| {
            format!(
                "{status} --case-changed {name}: no such case",
                status = Status::Unknown
            )
        })
        .collect()
}

/// The references `bless` refuses to replace (see [`bless_problems`]), under the render
/// rules and hash of the record it replaces, `changed` being the cases named.
fn reference_problems(before: &Before<'_>, now: &Blessing<'_>, changed: &[String]) -> Vec<String> {
    let old = before.record;
    let mut problems = Vec::new();
    for (case, hashes) in now.references {
        let named = changed.contains(case);
        let Some(recorded) = old.cases.get(case) else {
            if before.references.contains_key(case) && !named {
                problems.push(format!(
                    "{status} {case}: a reference but no versions recorded in the record \
                     bless replaces ({hint})",
                    status = Status::Missing,
                    hint = Hint::NoVersionsAtBless {
                        case,
                        record: before.path
                    }
                ));
            }
            continue;
        };
        if Some(recorded) != now.cases.get(case) || named {
            continue;
        }
        problems.extend(changed_references(
            before,
            (case, hashes),
            " under the same algorithm versions",
            Hint::ChangedReference { case },
        ));
    }
    problems
}

/// The line of each render format of `case` whose reference, among `hashes`, differs from
/// the one `before` holds, saying it changed, then `under` (the condition that makes it a
/// problem), then what to do, `hint`.
fn changed_references(
    before: &Before<'_>,
    (case, hashes): (&str, &BTreeMap<String, String>),
    under: &str,
    hint: Hint<'_>,
) -> Vec<String> {
    hashes
        .iter()
        .filter(|(format, hash)| {
            before
                .references
                .get(case)
                .and_then(|earlier| earlier.get(*format))
                .is_some_and(|earlier| earlier != *hash)
        })
        .map(|(format, _)| {
            format!(
                "{status} {case} {format}: its reference changed{under} ({hint})",
                status = Status::Changed
            )
        })
        .collect()
}

/// What a bless changing the battery refuses besides what every bless refuses: render rules
/// or a render hash other than the replaced record's, a version it does not record, a
/// reference changed (at any version), or a new case, each with the procedure that blesses the battery
/// change alone first, `changed` being the cases named. Under new render rules or a new
/// render hash every reference may change, so neither may move with the battery; and a
/// battery change replaces every fingerprint, so the references are the only guard left in
/// that bless (`spec/engine-design.md`, decision 29).
fn battery_apart(before: &Before<'_>, now: &Blessing<'_>, changed: &[String]) -> Vec<String> {
    let old = before.record;
    let hint = Hint::BatteryAlone { changed };
    let mut problems = Vec::new();
    if old.render_rules != RENDER_RULES_VERSION {
        problems.push(format!(
            "{status} --battery-changed: the render rules changed too, from version {} to \
             {RENDER_RULES_VERSION} ({hint})",
            old.render_rules,
            status = Status::Rules
        ));
    }
    if old.render_hash != SCHEME {
        problems.push(format!(
            "{status} --battery-changed: the render hash changed too, from {} to {SCHEME} \
             ({hint})",
            old.render_hash,
            status = Status::Hash
        ));
    }
    for (name, (version, _)) in now.fingerprints {
        if !old
            .fingerprints
            .get(name)
            .is_some_and(|versions| versions.contains_key(version))
        {
            problems.push(format!(
                "{status} {name}: version {version} is not recorded ({hint})",
                status = Status::Version
            ));
        }
    }
    if same_rules(old) {
        for (case, hashes) in now.references {
            if old.cases.contains_key(case) {
                problems.extend(changed_references(before, (case, hashes), "", hint));
            } else {
                problems.push(format!(
                    "{status} {case}: a case the record bless replaces does not hold ({hint})",
                    status = Status::Changed
                ));
            }
        }
    }
    problems
}

/// What `bless` refuses to write in place of `before`, as `told`:
/// - a name of `--case-changed` that is no case;
/// - a box type, or the render rules, at a lower version than recorded (in the replaced
///   record or the one at the spent base);
/// - a fingerprint changed at a spent version ([`Spent`]);
/// - a reference of a case that changed while its box types' versions, the render rules
///   and the render hash all stayed as recorded, unless the case is named in `--case-changed`;
/// - a reference of a case the replaced record holds no versions of, unless the case is
///   named;
/// - another battery without `--battery-changed`, and with it, an unchanged battery or
///   what [`battery_apart`] refuses: a battery change is a bless of its own. Without the
///   option, the battery's line says how to bless it alone first when the option would be
///   refused too.
pub(crate) fn bless_problems(
    before: &Before<'_>,
    now: &Blessing<'_>,
    told: &Told<'_>,
) -> Vec<String> {
    let old = before.record;
    let mut problems = unknown_cases(told, now.references);
    let rules_floor = match before.spent {
        Spent::Base(Some(base)) => old.render_rules.max(base.render_rules),
        Spent::Replaced | Spent::Base(None) => old.render_rules,
    };
    if RENDER_RULES_VERSION < rules_floor {
        problems.push(format!(
            "{status} render rules version {RENDER_RULES_VERSION}: below version {rules_floor} \
             already recorded",
            status = Status::Lower
        ));
    }
    let apart = battery_apart(before, now, told.changed);
    let battery_changed = old.fingerprint_battery != FINGERPRINT_BATTERY;
    match (battery_changed, told.battery_changed) {
        (true, false) => problems.push(format!(
            "{status} the fingerprints were computed with battery {}, the engine's is \
             {FINGERPRINT_BATTERY}, and a battery change blesses no reference and no version \
             ({hint})",
            old.fingerprint_battery,
            status = Status::Battery,
            hint = if apart.is_empty() {
                Hint::Battery
            } else {
                Hint::BatteryAlone {
                    changed: told.changed,
                }
            }
        )),
        (false, true) => problems.push(format!(
            "{status} --battery-changed: the battery is unchanged ({FINGERPRINT_BATTERY})",
            status = Status::Battery
        )),
        (true, true) | (false, false) => {}
    }
    if told.battery_changed {
        problems.extend(apart);
    }
    for (name, (version, print)) in now.fingerprints {
        let spent = match before.spent {
            Spent::Base(Some(base)) => highest(base, name),
            Spent::Replaced | Spent::Base(None) => None,
        };
        if let Some(higher) = highest(old, name)
            .into_iter()
            .chain(spent)
            .max()
            .filter(|higher| higher > version)
        {
            problems.push(format!(
                "{status} {name}: at algorithm version {version}, below version {higher} \
                 already recorded",
                status = Status::Lower
            ));
        }
        if kept(before, name, *version).is_some_and(|before| before != print) {
            problems.push(changed_bits(name, *version, Hint::BitsKept { name }));
        }
    }
    if same_rules(old) && !told.battery_changed {
        problems.extend(reference_problems(before, now, told.changed));
    }
    problems
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
