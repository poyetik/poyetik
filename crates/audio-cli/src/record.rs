//! What the conformance references were blessed under, `algorithms.json`
//! (`spec/engine-design.md`, decision 29): the render hash version, the render rules
//! version, the fingerprint of every box type at each algorithm version blessed, and the
//! algorithm version of every box type each case uses. From it, what `check` and `bless`
//! refuse.

use crate::hint::{Hint, said};
use crate::status::Status;
use alloc::collections::BTreeMap;
use std::path::Path;

use audio_battery::{FINGERPRINT_BATTERY, fingerprint};
use audio_boxes::{BuiltIn, REGISTRY};
use audio_core::{Binary32, Binary64, RENDER_RULES_VERSION};
use audio_hash::SCHEME;
use fpenv::DefaultMode;
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
    // The engine's instance type creates every registered box type: each is measured (a box
    // type it could not create would have no fingerprint, which the record refuses).
    REGISTRY
        .iter()
        .filter_map(|spec| {
            fingerprint::<BuiltIn<Binary64>, BuiltIn<Binary32>>(spec).map(|measured| {
                (
                    spec.name().to_owned(),
                    (spec.algorithm_version().get(), measured),
                )
            })
        })
        .collect()
}

/// The record `bless` writes: the versions of the current engine, `cases`, and the
/// fingerprints of `earlier` (the record it replaces) computed with the same battery, those
/// of `base` (the spent record, `--spent`) at the versions it spent replacing them, for
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

/// The box types of `now` whose current version `record` holds no fingerprint of, with
/// that version: the one test of a new version both `check` and a bless changing the
/// battery make, so that both choose the same hint.
fn unrecorded<'now>(
    record: &'now Record,
    now: &'now Fingerprints,
) -> impl Iterator<Item = (&'now str, u32)> {
    now.iter()
        .filter(|(name, (version, _))| {
            !record
                .fingerprints
                .get(*name)
                .is_some_and(|versions| versions.contains_key(version))
        })
        .map(|(name, (version, _))| (name.as_str(), *version))
}

/// What `check` found besides the record: whether a case had a problem, and the cases
/// whose reference changed under the same algorithm versions, which the last bless of a
/// battery change blessed alone must name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Others<'cases> {
    /// Whether check found a problem of a case.
    pub(crate) failed: bool,
    /// The cases whose reference changed under the same algorithm versions.
    pub(crate) changed: &'cases [String],
}

/// The problems `check` reports of `record` against the current engine, whose
/// fingerprints are `now`: another render hash, render rules or battery version, a
/// fingerprint missing or changed at a recorded version, or one of no box type. A battery
/// change with a problem of a case (`others`), other render rules, another render hash or
/// a version the record does not hold ([`unrecorded`], each said on a line without a hint)
/// is blessed alone first, as a bless changing the battery would refuse
/// ([`battery_apart`]). While the battery differs, no line but the battery's gives a hint
/// ([`said`]): every other bless is refused then.
pub(crate) fn record_problems(
    record: &Record,
    now: &Fingerprints,
    others: Others<'_>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let hints = record.fingerprint_battery == FINGERPRINT_BATTERY;
    if record.render_hash != SCHEME {
        problems.push(format!(
            "{status} the references were hashed with {}, the engine with {SCHEME}{hint}",
            record.render_hash,
            status = Status::Hash,
            hint = said(Hint::Hash, hints)
        ));
    }
    if record.render_rules != RENDER_RULES_VERSION {
        problems.push(format!(
            "{status} the references were rendered under render rules version {}, the \
             engine's is {RENDER_RULES_VERSION}{hint}",
            record.render_rules,
            status = Status::Rules,
            hint = said(Hint::Rules, hints)
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
        let said = problems.len();
        problems.extend(unrecorded(record, now).map(|(name, version)| {
            format!(
                "{status} {name}: version {version} is not recorded",
                status = Status::Version
            )
        }));
        let alone = others.failed || !same_rules(record) || problems.len() > said;
        problems.push(format!(
            "{status} the fingerprints were computed with battery {}, the engine's is \
             {FINGERPRINT_BATTERY} ({hint})",
            record.fingerprint_battery,
            status = Status::Battery,
            hint = if alone {
                Hint::BatteryAlone {
                    changed: others.changed,
                }
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
                    "{status} {name}: fingerprint of no box type{hint}",
                    status = Status::Stale,
                    hint = said(Hint::Stale, hints)
                )
            }),
    );
    problems
}

/// The problems `check` reports of `record` against `base`, the spent record: a box
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
                        "{status} {name} {version}: other bits than the spent record holds at \
                         this version{hint}",
                        status = Status::Bits,
                        hint = said(
                            Hint::SpentBits { name },
                            record.fingerprint_battery == FINGERPRINT_BATTERY
                        )
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
                "{status} the spent record: its fingerprints were computed with battery {}, \
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
    /// Every version of the record `bless` replaces: `audio-cli bless` alone, which knows
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
        problems.extend(changed_references(before, (case, hashes)));
    }
    problems
}

/// The line of each render format of `case` whose reference, among `hashes`, differs from
/// the one `before` holds, under the same algorithm versions, with what to do.
fn changed_references(
    before: &Before<'_>,
    (case, hashes): (&str, &BTreeMap<String, String>),
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
                "{status} {case} {format}: its reference changed under the same algorithm \
                 versions ({hint})",
                status = Status::Changed,
                hint = Hint::ChangedReference { case }
            )
        })
        .collect()
}

/// What a bless changing the battery refuses besides what every bless refuses, one line
/// each: render rules or a render hash other than the replaced record's, a version it does
/// not record ([`unrecorded`]), a reference changed (at any version) or missing from the
/// replaced references, or a new case; and the cases whose reference changed under the
/// same algorithm versions, which the bless after it must name. Under new render rules or a
/// new render hash every reference may change, so neither may move with the battery; and a
/// battery change replaces every fingerprint, so the references are the only guard left in
/// that bless, which therefore writes none it does not compare (`spec/engine-design.md`,
/// decision 29). The procedure that blesses the battery change alone first is said once,
/// by [`bless_problems`].
fn battery_apart(before: &Before<'_>, now: &Blessing<'_>) -> (Vec<String>, Vec<String>) {
    let old = before.record;
    let mut problems = Vec::new();
    let mut named = Vec::new();
    if old.render_rules != RENDER_RULES_VERSION {
        problems.push(format!(
            "{status} --battery-changed: the render rules changed too, from version {} to \
             {RENDER_RULES_VERSION}",
            old.render_rules,
            status = Status::Rules
        ));
    }
    if old.render_hash != SCHEME {
        problems.push(format!(
            "{status} --battery-changed: the render hash changed too, from {} to {SCHEME}",
            old.render_hash,
            status = Status::Hash
        ));
    }
    problems.extend(unrecorded(old, now.fingerprints).map(|(name, version)| {
        format!(
            "{status} {name}: version {version} is not recorded",
            status = Status::Version
        )
    }));
    if same_rules(old) {
        for (case, hashes) in now.references {
            if !old.cases.contains_key(case) {
                problems.push(format!(
                    "{status} {case}: a case the record bless replaces does not hold",
                    status = Status::Changed
                ));
                continue;
            }
            let earlier = before.references.get(case);
            for (format, hash) in hashes {
                let line = match earlier.and_then(|earlier| earlier.get(format)) {
                    Some(earlier) if earlier == hash => continue,
                    Some(_) => {
                        if old.cases.get(case) == now.cases.get(case) && !named.contains(case) {
                            named.push(case.clone());
                        }
                        "its reference changed"
                    }
                    None => "no reference in the references bless replaces",
                };
                problems.push(format!(
                    "{status} {case} {format}: {line}",
                    status = Status::Changed
                ));
            }
        }
    }
    (problems, named)
}

/// What `bless` refuses to write in place of `before`, as `told`:
/// - a name of `--case-changed` that is no case;
/// - a box type, or the render rules, at a lower version than recorded (in the replaced
///   record or the spent record);
/// - a fingerprint changed at a spent version ([`Spent`]);
/// - a reference of a case that changed while its box types' versions, the render rules
///   and the render hash all stayed as recorded, unless the case is named in `--case-changed`;
/// - a reference of a case the replaced record holds no versions of, unless the case is
///   named;
/// - another battery without `--battery-changed`, and with it, an unchanged battery or
///   what [`battery_apart`] refuses: a battery change is a bless of its own. With another
///   battery, the lines of what [`battery_apart`] refuses are given, without a hint, with
///   the option or without, in place of the references' lines above, whose hints would be
///   refused too. The battery's line says how to bless it alone first when the option is
///   refused, or would be: once, after those lines, its last bless naming the cases of
///   `--case-changed` that are cases and those whose reference changed under the same
///   versions.
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
    if old.fingerprint_battery == FINGERPRINT_BATTERY {
        if told.battery_changed {
            problems.push(format!(
                "{status} --battery-changed: the battery is unchanged ({FINGERPRINT_BATTERY}) \
                 ({hint})",
                status = Status::Battery,
                hint = Hint::WithoutBatteryChanged
            ));
        }
    } else {
        let (apart, also_named) = battery_apart(before, now);
        let alone = !apart.is_empty();
        let mut named: Vec<String> = told
            .changed
            .iter()
            .filter(|case| now.references.contains_key(*case))
            .cloned()
            .collect();
        named.extend(
            also_named
                .into_iter()
                .filter(|case| !told.changed.contains(case)),
        );
        problems.extend(apart);
        if alone || !told.battery_changed {
            problems.push(format!(
                "{status} the fingerprints were computed with battery {}, the engine's is \
                 {FINGERPRINT_BATTERY}, and a battery change blesses no reference and no \
                 version ({hint})",
                old.fingerprint_battery,
                status = Status::Battery,
                hint = if alone {
                    Hint::BatteryAlone { changed: &named }
                } else {
                    Hint::Battery
                }
            ));
        }
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
    if same_rules(old) && old.fingerprint_battery == FINGERPRINT_BATTERY && !told.battery_changed {
        problems.extend(reference_problems(before, now, told.changed));
    }
    problems
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
