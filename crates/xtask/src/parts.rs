//! The parts of the full check that CI runs as parallel jobs, and the gate that proves they cover it.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use core::num::NonZeroUsize;

use crate::{
    steps::{Action, Cmd, Failure, Step, roles, verdict, workspace_mutants_command},
    workspace::{Extent, Mode, PACKAGES, Package},
};

/// How many shards the Rust mutation testing of the full check is split into in CI, each a
/// part of its own ([`Part::Mutation`]).
pub const MUTATION_SHARDS: usize = 8;

/// One shard of the Rust mutants: cargo-mutants' `--shard <index>/<MUTATION_SHARDS>`.
///
/// Only [`Shard::all`] makes one, so that its index is always below [`MUTATION_SHARDS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard(usize);

impl Shard {
    /// Every shard, in order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..MUTATION_SHARDS).map(Self)
    }

    /// Its index, from 0, as cargo-mutants counts.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// What a step needs prepared before it, done again in every part that needs it: the
/// preparation is repeated, never the proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// The JavaScript dependencies installed, and the TypeScript reports and build outputs
    /// emptied.
    JavaScript,
    /// The engine package built ([`ENGINE_PACKAGE`](crate::workspace::ENGINE_PACKAGE)).
    EngineBuild,
    /// The conformance cases exported as WAV files ([`CONFORMANCE_EXPORT`](crate::workspace::CONFORMANCE_EXPORT)), for the oracle.
    ConformanceExport,
}

/// A part of the full check: what one CI job runs (`cargo xtask check --full --part
/// <name>`), in parallel with the others.
///
/// Together the parts run every step of the full check once ([`parts_gate`]); the
/// preparation steps ([`Setup`]) are repeated in the parts that need them, and every part
/// starts with the tools and the state of the working tree and ends checking it unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Formatting, lints, documentation, tests and portability builds.
    Rust,
    /// The Rust coverage of every crate.
    Coverage,
    /// One shard of the Rust mutants.
    Mutation(Shard),
    /// Miri on the unsafe code.
    Miri,
    /// Conformance, supply chain and the gates on the repository.
    Gates,
    /// The checks, mutation testing and gates of one TypeScript package.
    Package(Package),
    /// What needs every other part: the unviable mutants of every shard, reviewed and
    /// every review current. Run last, by a job that needs every other one.
    Final,
}

impl Part {
    /// Its name on the command line and in the workflow's matrix: no `/`, which an artifact
    /// name cannot hold.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Rust => "rust".to_owned(),
            Self::Coverage => "coverage".to_owned(),
            Self::Mutation(shard) => format!("mutation-{}", shard.index()),
            Self::Miri => "miri".to_owned(),
            Self::Gates => "gates".to_owned(),
            Self::Package(package) => package.dir.replace('/', "-"),
            Self::Final => "final".to_owned(),
        }
    }

    /// What it needs prepared.
    #[must_use]
    pub fn needs(self) -> Vec<Setup> {
        match self {
            Self::Gates => vec![Setup::JavaScript],
            Self::Package(package) => [&[Setup::JavaScript], package.needs].concat(),
            Self::Rust | Self::Coverage | Self::Mutation(_) | Self::Miri | Self::Final => {
                Vec::new()
            }
        }
    }
}

/// Every part of the full check, in order, [`Part::Final`] last.
#[must_use]
pub fn full_parts() -> Vec<Part> {
    let mut parts = vec![Part::Rust, Part::Coverage];
    parts.extend(Shard::all().map(Part::Mutation));
    parts.extend([Part::Miri, Part::Gates]);
    parts.extend(PACKAGES.map(Part::Package));
    parts.push(Part::Final);
    parts
}

/// Where the final part reads the unviable mutants of the shard `part` (named as by
/// [`Part::name`]).
///
/// The CI job of each shard uploads its [`UNVIABLE_REPORT`](crate::workspace::UNVIABLE_REPORT) as the artifact
/// `unviable-<part>`, which the final job downloads below [`SHARD_REPORTS`], one directory
/// per artifact.
#[must_use]
pub fn shard_unviable_report(part: &str) -> String {
    format!("{SHARD_REPORTS}/{UNVIABLE_ARTIFACT}{part}/unviable.txt")
}

/// Where the final part finds the reports of the shards, one directory per artifact.
pub const SHARD_REPORTS: &str = "mutants.out/shards";

/// The prefix of the name of the artifact of each shard's unviable mutants.
pub const UNVIABLE_ARTIFACT: &str = "unviable-";

/// Who runs a step of the full check when it is split into parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Every part: the tools, and the working tree recorded then unchanged.
    Frame,
    /// Every part that needs this preparation.
    Setup(Setup),
    /// That part only.
    Proof(Part),
    /// Every shard ([`Part::Mutation`]): the mutation report emptied.
    EveryShard,
    /// Split among the shards: each runs its own mutants.
    Sharded,
    /// The final part, on what every shard found: the unviable-mutant gate.
    AcrossShards,
}

/// The steps of `part` of the full check, in the order of the full check, with `jobs`
/// cargo-mutants jobs.
#[must_use]
pub fn part_steps(part: Part, jobs: NonZeroUsize) -> Vec<Step> {
    let needs = part.needs();
    roles(Mode::Full { jobs })
        .into_iter()
        .filter_map(|(role, step)| match (role, part) {
            (Role::Frame, _) | (Role::EveryShard, Part::Mutation(_)) => Some(step),
            (Role::Setup(setup), _) => needs.contains(&setup).then_some(step),
            (Role::Proof(owner), _) => (owner == part).then_some(step),
            (Role::Sharded, Part::Mutation(shard)) => Some(Step {
                name: format!("{} (shard {}/{MUTATION_SHARDS})", step.name, shard.index()),
                action: Action::RustMutants(workspace_mutants_command(jobs, Some(shard))),
            }),
            (Role::AcrossShards, Part::Final) => Some(Step {
                name: format!("{} (every shard)", step.name),
                action: Action::ShardsUnviableGate(
                    Shard::all()
                        .map(|shard| shard_unviable_report(&Part::Mutation(shard).name()))
                        .collect(),
                ),
            }),
            (Role::EveryShard | Role::Sharded | Role::AcrossShards, _) => None,
        })
        .collect()
}

/// The arguments that make a cargo-mutants command run one shard: `--shard <k>/<n>`, written
/// as cargo-mutants reads it (no sign, no leading zero), then round-robin sharding.
///
/// Returns the command without them, the shard's index and the count of shards, or `None` if `cmd`
/// holds no such arguments.
pub(crate) fn unsharded(cmd: &Cmd) -> Option<(Cmd, usize, usize)> {
    let at = cmd.args.iter().position(|arg| arg == "--shard")?;
    let [_, value, sharding, ..] = cmd.args.get(at..)? else {
        return None;
    };
    let (index, count) = value.split_once('/')?;
    let (index, count): (usize, usize) = (index.parse().ok()?, count.parse().ok()?);
    if *value != format!("{index}/{count}") || sharding != "--sharding=round-robin" {
        return None;
    }
    let mut args = cmd.args.clone();
    let _removed: Vec<String> = args.drain(at..at.checked_add(3)?).collect();
    Some((
        Cmd {
            args,
            ..cmd.clone()
        },
        index,
        count,
    ))
}

/// What the steps of the parts stand for in the full check ([`parts_gate`]).
#[derive(Debug, Default)]
pub(crate) struct Origins {
    /// For each step of the full check, the parts that run it whole.
    whole: Vec<Vec<String>>,
    /// For each step of the full check run in shards: each shard (its index and the count of
    /// shards), and its part.
    shards: BTreeMap<usize, Vec<(usize, usize, String)>>,
    /// The parts that run a shard, in order.
    shard_parts: Vec<String>,
    /// The reports read by each unviable-mutant gate over the shards.
    across: Vec<Vec<String>>,
}

/// Finds what `step`, a step of `part`, stands for in `full`: its index there, recorded in
/// `origins`, or a problem.
pub(crate) fn origin(
    full: &[Step],
    part: &str,
    step: &Step,
    origins: &mut Origins,
    problems: &mut Vec<String>,
) -> Option<usize> {
    if let Some(index) = full.iter().position(|whole| whole == step) {
        origins.whole.get_mut(index)?.push(part.to_owned());
        return Some(index);
    }
    if let Action::RustMutants(cmd) = &step.action
        && let Some((whole, shard, count)) = unsharded(cmd)
        && let Some(index) = full.iter().position(|of| {
            of.action == Action::RustMutants(whole.clone())
                && step.name == format!("{} (shard {shard}/{count})", of.name)
        })
    {
        if origins.shard_parts.iter().any(|name| name == part) {
            problems.push(format!(
                "{part}: runs a second shard, whose report would replace the first"
            ));
        }
        origins.shard_parts.push(part.to_owned());
        origins
            .shards
            .entry(index)
            .or_default()
            .push((shard, count, part.to_owned()));
        return Some(index);
    }
    if let Action::ShardsUnviableGate(reports) = &step.action
        && let Some(index) = full.iter().position(|of| {
            of.action == Action::UnviableGate(Extent::Complete)
                && step.name == format!("{} (every shard)", of.name)
        })
    {
        origins.across.push(reports.clone());
        return Some(index);
    }
    problems.push(format!(
        "{part}: `{}` is no step of the full check",
        step.name
    ));
    None
}

/// The problems of a step of the full check run in shards: `sharded` (each shard's index,
/// the count of shards, and its part), never whole, and every shard of the same count once.
pub(crate) fn shard_problems(
    name: &str,
    whole: &[String],
    sharded: &[(usize, usize, String)],
) -> Vec<String> {
    let mut problems = Vec::new();
    if !whole.is_empty() {
        problems.push(format!(
            "`{name}` runs both whole ({}) and in shards",
            whole.join(", ")
        ));
    }
    let counts: BTreeSet<usize> = sharded.iter().map(|(_, count, _)| *count).collect();
    let [count] = counts.into_iter().collect::<Vec<_>>()[..] else {
        problems.push(format!("`{name}` is split into shards of different counts"));
        return problems;
    };
    for (shard, _, part) in sharded.iter().filter(|(shard, _, _)| *shard >= count) {
        problems.push(format!(
            "{part}: shard {shard}/{count} of `{name}` is no shard"
        ));
    }
    for index in 0..count {
        let parts: Vec<&str> = sharded
            .iter()
            .filter(|(shard, _, _)| *shard == index)
            .map(|(_, _, part)| part.as_str())
            .collect();
        match parts.len() {
            0 => problems.push(format!("shard {index}/{count} of `{name}` runs in no part")),
            1 => {}
            _ => problems.push(format!(
                "shard {index}/{count} of `{name}` runs in {} parts: {}",
                parts.len(),
                parts.join(", ")
            )),
        }
    }
    problems
}

/// Checks that `parts` (each named, with its steps) run every step of `full`, the full check,
/// once.
///
/// Each step of a part stands for one step of `full`, and the steps of a part keep the order
/// of `full`. A step of a part is a step of `full`; or one shard of a cargo-mutants step of
/// `full` (its command with `--shard <k>/<n> --sharding=round-robin` added, named after it
/// with ` (shard <k>/<n>)`); or the unviable-mutant gate over the shards
/// ([`Action::ShardsUnviableGate`], named after the complete [`Action::UnviableGate`] of
/// `full` with ` (every shard)`), which stands for that gate. Every step of `full` runs in
/// exactly one part, except: the `preparation` steps, which run in one part at least; a step
/// run in shards, which runs whole in no part, every shard `k` from 0 to `n - 1` in exactly
/// one part, a part running one shard at most; and the unviable-mutant gate, run over the
/// shards in exactly one part, reading the report of every part that runs a shard
/// ([`shard_unviable_report`]), in order. Part names are unique.
///
/// So no step of the full check is lost or proven twice, and no part runs a step the full
/// check does not (cargo-mutants itself splits the mutants exactly among the shards).
///
/// # Errors
///
/// Returns every step run in no part or in several, every shard missing or repeated, every
/// step of a part that stands for none of `full` or runs out of its order, and every
/// repeated part name.
pub fn parts_gate(
    full: &[Step],
    parts: &[(String, Vec<Step>)],
    preparation: &[String],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut origins = Origins {
        whole: vec![Vec::new(); full.len()],
        ..Origins::default()
    };
    let mut names = BTreeSet::new();
    for (part, steps) in parts {
        if !names.insert(part) {
            problems.push(format!("two parts are named {part}"));
        }
        let mut last = None;
        for step in steps {
            let Some(index) = origin(full, part, step, &mut origins, &mut problems) else {
                continue;
            };
            if last.is_some_and(|last| last >= index) {
                problems.push(format!(
                    "{part}: `{}` runs out of the order of the full check",
                    step.name
                ));
            }
            last = Some(index);
        }
    }
    let reports: Vec<String> = origins
        .shard_parts
        .iter()
        .map(|part| shard_unviable_report(part))
        .collect();
    for (index, (step, whole)) in full.iter().zip(&origins.whole).enumerate() {
        let name = &step.name;
        if let Some(sharded) = origins.shards.get(&index) {
            problems.extend(shard_problems(name, whole, sharded));
            continue;
        }
        let across = if step.action == Action::UnviableGate(Extent::Complete) {
            origins.across.len()
        } else {
            0
        };
        let runs = whole.len().saturating_add(across);
        if runs == 0 {
            problems.push(format!("`{name}` runs in no part"));
        } else if runs > 1 && !preparation.contains(name) {
            problems.push(format!("`{name}` runs in {runs} parts"));
        }
    }
    problems.extend(
        origins
            .across
            .iter()
            .filter(|read| **read != reports)
            .map(|read| {
                format!(
                    "the unviable-mutant gate over the shards reads {}, not the reports of the \
                     shards: {}",
                    read.join(" "),
                    reports.join(" ")
                )
            }),
    );
    verdict(
        "the parts of the full check do not run each of its steps once",
        problems,
    )
}

/// The names of the steps of the full check that more than one part may run: those that
/// prepare ([`Setup`], and the mutation report emptied by every shard) or frame the others,
/// never a proof.
#[must_use]
pub fn preparation_steps() -> Vec<String> {
    roles(Mode::Full {
        jobs: NonZeroUsize::MIN,
    })
    .into_iter()
    .filter(|(role, _)| matches!(role, Role::Frame | Role::Setup(_) | Role::EveryShard))
    .map(|(_, step)| step.name)
    .collect()
}

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;
