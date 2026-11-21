//! The gates over reviewed exceptions: mutation reports, Stryker, proof bypasses, scopes and unviable mutants.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;

use crate::{
    dependencies::{NPM_FIELDS, WORKSPACE_SCOPE},
    parts::Setup,
    release::CARGO_TOML,
    steps::{Cmd, Dir, Failure, MUTANTS_ARGS, step_verdict, verdict},
    workspace::{
        EQUIVALENT_REVIEWED, Extent, FIXTURES, PACKAGES, Package, RUST_CRATES, UNVIABLE_REVIEWED,
        package_jsons,
    },
};

/// A mutant disabled in a TypeScript source by a well-formed, reviewed comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disable {
    /// The source, relative to the root.
    pub path: String,
    /// The line the comment disables, numbered from 1.
    pub line: usize,
    /// The mutator it disables.
    pub mutator: String,
}

/// Checks the mutation report of a TypeScript package (Stryker `json`).
///
/// At least one mutant after a [`Extent::Complete`] run (a partial one may find none);
/// every mutant `Killed`, or `Timeout` (a mutant that makes a test
/// hang is detected, as Stryker counts it), or `Ignored` exactly where a disable comment
/// of `disabled` says so (same file, line and mutator; [`equivalent_gate`] requires every
/// such comment to be reviewed). Any other status (`Survived`, `NoCoverage`,
/// `RuntimeError`, `CompileError`, one a future Stryker adds) fails.
///
/// # Errors
///
/// Returns every mutant that was not killed, named `file:line Mutator status`, or why the
/// report cannot be read.
pub fn ts_mutation_gate(
    package: &str,
    report: &str,
    disabled: &[Disable],
    extent: Extent,
) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| Failure::from(format!("{package}: invalid mutation report: {error}")))?;
    let mut mutants = 0_usize;
    let mut problems = Vec::new();
    // The ranges of its line (start and end, as the report writes them) where each reviewed
    // disable comment let mutants be ignored: it reviews one operation, whose mutants (a
    // comparison's two) all replace the same range; an operation nested in another starts
    // where it does, so a start alone cannot tell them apart.
    let mut ignored: BTreeMap<(String, usize, String), BTreeSet<(String, String)>> =
        BTreeMap::new();
    let files = json
        .get("files")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten();
    for (file, entry) in files {
        let path = format!("{package}/{file}");
        let listed = entry
            .get("mutants")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten();
        for mutant in listed {
            mutants = mutants.saturating_add(1);
            let text = |key: &str| {
                mutant
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            };
            let line = mutant
                .pointer("/location/start/line")
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
                .unwrap_or_default();
            let (mutator, status) = (text("mutatorName"), text("status"));
            let reviewed = disabled.iter().any(|disable| {
                disable.path == path && disable.line == line && disable.mutator == mutator
            });
            let detected = matches!(status, "Killed" | "Timeout");
            let accepted = detected || (status == "Ignored" && reviewed);
            if !accepted {
                problems.push(format!("{path}:{line} {mutator} {status}"));
            } else if !detected {
                let place = |end: &str| {
                    mutant
                        .pointer(&format!("/location/{end}"))
                        .map(ToString::to_string)
                        .unwrap_or_default()
                };
                let _: bool = ignored
                    .entry((path.clone(), line, mutator.to_owned()))
                    .or_default()
                    .insert((place("start"), place("end")));
            }
        }
    }
    for ((path, line, mutator), columns) in ignored {
        if columns.len() > 1 {
            problems.push(format!(
                "{path}:{line} {mutator}: mutants of {} operations ignored by one disable comment, which reviews one: put the equivalent operation on a line of its own",
                columns.len()
            ));
        }
    }
    if mutants == 0 && extent == Extent::Complete {
        problems.push("no mutant in the mutation report".to_owned());
    }
    verdict(&format!("{package}: mutants not killed"), problems)
}

/// The Stryker configuration of every package, relative to it: the one file the gates read
/// ([`stryker_config_gate`], [`scope_gate`]), so the one every Stryker run names.
pub const STRYKER_CONFIG: &str = "stryker.config.json";

/// The only command a package script may run Stryker with.
pub const STRYKER_SCRIPT: &str = "stryker run stryker.config.json";

/// Checks that no script of the package manifest at `path` runs Stryker but as
/// [`STRYKER_SCRIPT`], so that none reads another configuration or narrows the run.
///
/// # Errors
///
/// Returns every script that mentions Stryker otherwise and every script that is not a
/// command, or why the manifest cannot be read.
pub fn stryker_scripts_gate(path: &str, manifest: &str) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(manifest)
        .map_err(|error| Failure::from(format!("{path}: invalid manifest: {error}")))?;
    let mut problems = Vec::new();
    let scripts = json
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten();
    for (name, script) in scripts {
        match script.as_str() {
            Some(command) if command.contains("stryker") && command != STRYKER_SCRIPT => {
                problems.push(format!("`{name}`: {command}"));
            }
            Some(_) => {}
            None => problems.push(format!("`{name}` is not a command")),
        }
    }
    verdict(
        &format!("{path} runs Stryker otherwise than `{STRYKER_SCRIPT}`"),
        problems,
    )
}

/// The `check` script of a package manifest that runs `checks` in order, as its checks do.
#[must_use]
pub fn check_script(checks: &[&str]) -> String {
    checks
        .iter()
        .map(|script| format!("pnpm {script}"))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Checks that the `check` script of the package manifest at `path` runs exactly `checks`, in
/// order ([`check_script`]).
///
/// The checks run and time each script as a step of its own, so that `pnpm run check` in the
/// package and `cargo xtask check` run the same scripts; no `precheck` or `postcheck` script
/// either, which `pnpm run check` would run around it and the checks would not.
///
/// # Errors
///
/// Returns every problem: the script found and the one expected, or that the manifest has
/// none, and each `precheck` or `postcheck` script, each saying that the scripts and
/// `Package::checks` in xtask change together; or why the manifest cannot be read.
pub fn package_check_gate(path: &str, manifest: &str, checks: &[&str]) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(manifest)
        .map_err(|error| Failure::from(format!("{path}: invalid manifest: {error}")))?;
    let expected = check_script(checks);
    let scripts = json.get("scripts");
    let mut problems = Vec::new();
    match scripts
        .and_then(|scripts| scripts.get("check"))
        .and_then(serde_json::Value::as_str)
    {
        Some(found) if found == expected => {}
        Some(found) => problems.push(format!("its `check` script runs `{found}`")),
        None => problems.push("it has no `check` script".to_owned()),
    }
    for around in ["precheck", "postcheck"] {
        if scripts.and_then(|scripts| scripts.get(around)).is_some() {
            problems.push(format!(
                "it has a `{around}` script, which the checks would not run"
            ));
        }
    }
    verdict(
        &format!(
            "{path}: the checks run and time `{expected}` (`Package::checks` in xtask's \
             workspace.rs, which changes with the manifest)"
        ),
        problems,
    )
}

/// The directory of a package's build outputs, the only one any package exports.
pub(crate) const BUILD_OUTPUT: &str = "dist";

/// Whether `exports`, a manifest's exports with their conditions, names a file below the
/// package's directory `dir`.
fn exports_under(exports: &serde_json::Value, dir: &str) -> bool {
    match exports {
        serde_json::Value::String(target) => target
            .strip_prefix("./")
            .and_then(|path| path.split_once('/'))
            .is_some_and(|(top, _)| top == dir),
        serde_json::Value::Object(conditions) => {
            conditions.values().any(|target| exports_under(target, dir))
        }
        serde_json::Value::Array(targets) => {
            targets.iter().any(|target| exports_under(target, dir))
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
    }
}

/// Every file `target`, an export's value with its conditions and fallbacks, names.
fn export_targets(target: &serde_json::Value) -> Vec<&str> {
    match target {
        serde_json::Value::String(file) => vec![file.as_str()],
        serde_json::Value::Object(conditions) => {
            conditions.values().flat_map(export_targets).collect()
        }
        serde_json::Value::Array(fallbacks) => fallbacks.iter().flat_map(export_targets).collect(),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Vec::new()
        }
    }
}

/// Checks that no package exports test code: `manifests` holds the path and the text of
/// each `package.json`.
///
/// The scope gate leaves a package's test code out of its sources (`is_test`: under
/// `test/`, or a test or specification file), so that neither coverage nor mutation testing
/// measures it; an export into it would hand code no proof sees to another package. Each
/// export is read as Node.js reads it: an object whose keys all start with `.` maps subpaths
/// to their targets, anything else is the target of `.`; a target's conditions and
/// fallbacks are read too.
///
/// # Errors
///
/// Returns every export into test code, naming its manifest, its subpath and its file, or why
/// a manifest cannot be read.
pub fn test_exports_gate(manifests: &[(&str, &str)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, manifest) in manifests {
        let json: serde_json::Value = serde_json::from_str(manifest)
            .map_err(|error| Failure::from(format!("{path}: invalid manifest: {error}")))?;
        let Some(exports) = json.get("exports") else {
            continue;
        };
        let subpaths: Vec<(&str, &serde_json::Value)> = match exports.as_object() {
            Some(map) if map.keys().all(|key| key.starts_with('.')) => map
                .iter()
                .map(|(key, target)| (key.as_str(), target))
                .collect(),
            Some(_) | None => vec![(".", exports)],
        };
        let mut found: Vec<String> = subpaths
            .into_iter()
            .flat_map(|(subpath, target)| {
                export_targets(target)
                    .into_iter()
                    .filter(|file| is_test(file.trim_start_matches("./")))
                    .map(move |file| format!("{path}: {subpath} exports {file}"))
            })
            .collect();
        found.sort();
        problems.extend(found);
    }
    verdict(
        "package exports into test code, which neither coverage nor mutation testing measures \
         (the scope gate skips it): move each under the package's measured sources \
         (src/testing/, ts/testing/)",
        problems,
    )
}

/// The libraries with a conformance runner, each with the mark of that runner in a test.
///
/// A shipped module may link a library's WebAssembly interface; a test runs the library's
/// runner when it names it: its export, `@poyetik/visual-wasm/testing/conformance`, or its
/// source in the library's own package.
pub const CONFORMANCE_RUNNERS: [(&str, &str); 1] = [("visual-wasm", "testing/conformance")];

/// What [`linked_conformance_gate`] reads of one TypeScript package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedModule<'text> {
    /// Its directory, as in [`Package::dir`](crate::workspace::Package::dir).
    pub dir: &'static str,
    /// The text of its `package.json`.
    pub manifest: &'text str,
    /// The text of the `Cargo.toml` of its crate: a package without one links no library
    /// and is never given.
    pub cargo: &'text str,
    /// Its tracked TypeScript files, with their text.
    pub tests: Vec<(String, String)>,
}

/// Whether the Cargo manifest `cargo` names `library` among its `[dependencies]` (inline,
/// or as a `[dependencies.<library>]` table), or is `library` itself: what it links.
fn links(cargo: &str, library: &str) -> bool {
    let mut table = "";
    for line in cargo.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line;
            if line == format!("[dependencies.{library}]") {
                return true;
            }
            continue;
        }
        let key = line
            .split_once('=')
            .map(|(key, _)| key.trim().trim_matches('"'));
        if (table == "[dependencies]" && key == Some(library))
            || (table == "[package]"
                && key == Some("name")
                && line.ends_with(&format!("\"{library}\"")))
        {
            return true;
        }
    }
    false
}

/// Checks that every module a package ships is held to the corpus of every library it links.
///
/// A package exporting a `.wasm` file, whose crate links a library of
/// [`CONFORMANCE_RUNNERS`] (or is it), has a test under its `test/` that runs that library's
/// conformance runner. So a module built apart, from its own crate with its own dependencies
/// and memory, is never proven only through its sibling's tests.
///
/// # Errors
///
/// Returns every shipped module, with the library it links, that no test holds to that
/// library's runner, or why a manifest cannot be read.
pub fn linked_conformance_gate(packages: &[LinkedModule<'_>]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for package in packages {
        let dir = package.dir;
        let json: serde_json::Value = serde_json::from_str(package.manifest).map_err(|error| {
            Failure::from(format!("{dir}/package.json: invalid manifest: {error}"))
        })?;
        let shipped: Vec<&str> = json
            .get("exports")
            .map(export_targets)
            .unwrap_or_default()
            .into_iter()
            .filter(|file| {
                std::path::Path::new(file)
                    .extension()
                    .is_some_and(|extension| extension == "wasm")
            })
            .collect();
        let cargo = package.cargo;
        let tests = format!("{dir}/test/");
        for (library, mark) in CONFORMANCE_RUNNERS {
            let held = package
                .tests
                .iter()
                .any(|(path, text)| path.starts_with(&tests) && text.contains(mark));
            if links(cargo, library) && !held {
                problems.extend(shipped.iter().map(|module| {
                    format!(
                        "{dir} ships {module}, which links {library}, but no test of it under \
                         test/ runs its conformance runner ({mark})"
                    )
                }));
            }
        }
    }
    verdict(
        "modules held to no conformance runner of a library they link: every module the \
         product ships is held to the corpus of every library it links",
        problems,
    )
}

/// The constant `line` exports, when it is one a package could copy from another: its name
/// (upper case, digits and `_`) and its value, a literal written on that one line (a
/// number, a quoted string, `true` or `false`), the type annotation, if any, left out.
fn exported_constant(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("export const ")?;
    let (left, value) = rest.split_once(" = ")?;
    let name = left.split(':').next().unwrap_or_default().trim();
    let value = value.trim_end().strip_suffix(';')?;
    let named = !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    let number = value
        .strip_prefix('-')
        .unwrap_or(value)
        .starts_with(|character: char| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.+-".contains(character));
    let quoted = ['"', '\'', '`'].iter().any(|quote| {
        value
            .strip_prefix(*quote)
            .and_then(|inner| inner.strip_suffix(*quote))
            .is_some_and(|inner| !inner.contains(*quote))
    });
    (named && (number || quoted || value == "true" || value == "false")).then_some((name, value))
}

/// The value a literal of [`exported_constant`] stands for, so that two spellings of one
/// number are one fact: a number without its separators (`_`), then, when it parses, as
/// the shortest decimal of its binary64 value (`48_000`, `48000`, `48000.0` and `4.8e4`
/// alike), or as the decimal of an integer written in hexadecimal, octal or binary; any
/// other literal as written.
pub(crate) fn literal_value(value: &str) -> String {
    let bare: String = value
        .chars()
        .filter(|&character| character != '_')
        .collect();
    let (sign, digits) = bare
        .strip_prefix('-')
        .map_or(("", bare.as_str()), |digits| ("-", digits));
    let radix = [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ]
    .iter()
    .find_map(|(prefix, radix)| Some((digits.strip_prefix(prefix)?, *radix)));
    if let Some((integer, radix)) = radix {
        return u128::from_str_radix(integer, radix)
            .map_or_else(|_| value.to_owned(), |number| format!("{sign}{number}"));
    }
    if !digits.starts_with(|character: char| character.is_ascii_digit()) {
        return value.to_owned();
    }
    bare.parse::<f64>()
        .map_or_else(|_| value.to_owned(), |number| format!("{number}"))
}

/// Checks that no constant is exported with one name and one value by several packages:
/// `sources` holds the directory of each package's source file, its path and its text.
///
/// One fact copied into two packages drifts apart unseen when one copy changes (deep review 3
/// found the render quantum typed twice, the audio bindings' and the playground's): each
/// fact is exported by one package, which the others import or re-export. Only a constant
/// whose value is a literal on its line is read (a number, a quoted string, `true` or
/// `false`, on one line), two spellings of one number one value (`literal_value`); a
/// re-export (`export { X } from`) is no copy.
///
/// # Errors
///
/// Returns every constant exported alike by several packages, with every file exporting it.
pub fn shared_constants_gate(sources: &[(&str, &str, &str)]) -> Result<(), Failure> {
    /// The spellings of one constant's value, and the files of each package exporting it.
    type Copies<'a> = (BTreeSet<&'a str>, BTreeMap<&'a str, BTreeSet<&'a str>>);
    let mut exported: BTreeMap<(&str, String), Copies<'_>> = BTreeMap::new();
    for (package, path, text) in sources {
        for line in text.lines() {
            if let Some((name, value)) = exported_constant(line) {
                let (spellings, packages) =
                    exported.entry((name, literal_value(value))).or_default();
                let _spelled = spellings.insert(value);
                let _new = packages.entry(package).or_default().insert(path);
            }
        }
    }
    let problems = exported
        .into_iter()
        .filter(|(_, (_, packages))| packages.len() > 1)
        .map(|((name, _), (spellings, packages))| {
            let files: Vec<&str> = packages.into_values().flatten().collect();
            let values: Vec<&str> = spellings.into_iter().collect();
            format!("{name} = {}: {}", values.join(" or "), files.join(", "))
        })
        .collect();
    verdict(
        "constants exported alike by several packages, one fact copied: export each from one \
         package and import or re-export it in the others",
        problems,
    )
}

/// Checks the order of the packages and the builds they need, from their manifests:
/// `manifests` holds each package with the text of its `package.json`, in the order their
/// checks run ([`PACKAGES`]).
///
/// Every workspace package a package depends on (`@poyetik/*`, in any section of
/// dependencies) is a package of the checks whose checks run before its own, so
/// that its build outputs exist when they are read (they are emptied before the checks);
/// and the builds a package needs ([`Setup::Build`] in its `needs`) are exactly those of the
/// packages it depends on, directly or not, that have build outputs, in that order, so that
/// a part of the full check running it alone prepares every one of them, each after the
/// builds it reads. A package has build outputs (`Package::outputs`) exactly when its
/// exports name a file of its `dist/`.
///
/// # Errors
///
/// Returns every problem, each naming the manifest, or why a manifest cannot be read.
pub fn package_order_gate(manifests: &[(Package, &str)]) -> Result<(), Failure> {
    let mut names: Vec<String> = Vec::new();
    let mut uses: Vec<Vec<String>> = Vec::new();
    let mut problems = Vec::new();
    for (package, manifest) in manifests {
        let path = format!("{}/package.json", package.dir);
        let json: serde_json::Value = serde_json::from_str(manifest)
            .map_err(|error| Failure::from(format!("{path}: invalid manifest: {error}")))?;
        let exported = json
            .get("exports")
            .is_some_and(|exports| exports_under(exports, BUILD_OUTPUT));
        let declared = package.outputs.contains(&BUILD_OUTPUT);
        if exported && !declared {
            problems.push(format!(
                "{path}: exports its build outputs ({BUILD_OUTPUT}/), which `outputs` \
                 (`PACKAGES` in xtask's workspace.rs) does not list, so that they are never \
                 emptied before the checks nor built for the packages that read them"
            ));
        } else if declared && !exported {
            problems.push(format!(
                "{path}: `outputs` (`PACKAGES` in xtask's workspace.rs) lists {BUILD_OUTPUT}/, \
                 which it does not export"
            ));
        }
        let name = json
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Failure::from(format!("{path}: no `name`")))?;
        names.push(name.to_owned());
        uses.push(
            NPM_FIELDS
                .iter()
                .filter_map(|section| json.get(section).and_then(serde_json::Value::as_object))
                .flat_map(serde_json::Map::keys)
                .filter(|dependency| dependency.starts_with(WORKSPACE_SCOPE))
                .cloned()
                .collect(),
        );
    }
    for (at, ((package, _), used)) in manifests.iter().zip(&uses).enumerate() {
        let path = format!("{}/package.json", package.dir);
        for dependency in used {
            match names.iter().position(|name| name == dependency) {
                None => problems.push(format!(
                    "{path}: depends on {dependency}, which is no package of the checks"
                )),
                Some(before) if before >= at => problems.push(format!(
                    "{path}: depends on {dependency}, whose checks do not run before its own \
                     (`PACKAGES` in xtask's workspace.rs)"
                )),
                Some(_) => {}
            }
        }
        // Every package it reads, directly or not: each a package listed before it, so that
        // the search ends.
        let mut reached = BTreeSet::new();
        let mut pending: Vec<&String> = used.iter().collect();
        while let Some(dependency) = pending.pop() {
            if let Some(index) = names.iter().position(|name| name == dependency)
                && index < at
                && reached.insert(index)
            {
                pending.extend(uses.get(index).into_iter().flatten());
            }
        }
        let needed: Vec<&str> = manifests
            .iter()
            .enumerate()
            .filter(|(index, (built, _))| reached.contains(index) && !built.outputs.is_empty())
            .map(|(_, (built, _))| built.dir)
            .collect();
        let declared: Vec<&str> = package
            .needs
            .iter()
            .filter_map(|setup| match setup {
                Setup::Build(dir) => Some(*dir),
                Setup::JavaScript | Setup::ConformanceExport => None,
            })
            .collect();
        if needed != declared {
            problems.push(format!(
                "{path}: reads the builds of {needed:?}, in that order, but its `needs` \
                 (`PACKAGES` in xtask's workspace.rs) build {declared:?}"
            ));
        }
    }
    step_verdict(problems)
}

/// The keys a Stryker configuration may use: none of them hides a mutant. Any other key
/// (`ignoreStatic`, `mutator.excludedMutations`, `ignorers` …) could, and is refused.
pub const STRYKER_KEYS: [&str; 11] = [
    "$schema",
    "coverageAnalysis",
    "disableTypeChecks",
    "ignorePatterns",
    "mutate",
    "plugins",
    "reporters",
    "tempDirName",
    "testRunner",
    "thresholds",
    "vitest",
];

/// Checks the Stryker configuration of a TypeScript package.
///
/// Only [`STRYKER_KEYS`], no negated (`!`) glob in `mutate`, which would exclude files from
/// mutation, and the `json` reporter, whose report [`ts_mutation_gate`] reads.
///
/// # Errors
///
/// Returns every key, glob or missing reporter at fault, or why the file cannot be read.
pub fn stryker_config_gate(package: &str, config: &str) -> Result<(), Failure> {
    let json: serde_json::Value = serde_json::from_str(config).map_err(|error| {
        Failure::from(format!("{package}: invalid Stryker configuration: {error}"))
    })?;
    let keys = json.as_object().ok_or_else(|| {
        Failure::from(format!(
            "{package}: the Stryker configuration is not an object"
        ))
    })?;
    let strings = |key: &str| -> Vec<&str> {
        keys.get(key)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect()
    };
    let mut problems: Vec<String> = keys
        .keys()
        .filter(|key| !STRYKER_KEYS.contains(&key.as_str()))
        .map(|key| format!("`{key}` is not allowed: it could hide mutants"))
        .collect();
    problems.extend(
        strings("mutate")
            .into_iter()
            .filter(|glob| glob.starts_with('!'))
            .map(|glob| format!("`{glob}` in mutate excludes files from mutation")),
    );
    if !strings("reporters").contains(&"json") {
        problems
            .push("the `json` reporter is missing: the mutation report is not written".to_owned());
    }
    verdict(
        &format!("{package}/stryker.config.json hides mutants"),
        problems,
    )
}

/// What may remove code from a proof without a test failing: a coverage exclusion, a
/// skipped mutant, an ignored or focused test. Built at run time, so that this file does
/// not hold them; each with the extensions of the files where it acts.
///
/// A `cfg` that a coverage build sets (`coverage`, and the one of the nightly build) is an
/// unexpected `cfg` in every other build, which the workspace lints refuse; so is any lifting of
/// that lint, and any declaration of an expected `cfg` (in a manifest or a build script), are
/// refused here. So is the `cfg` of debug assertions, under which code differs between the
/// release build of the engine package's WebAssembly and every build the checks lint, cover,
/// mutate or read the files of ([`rust_hints`] and [`manifest_hints`] add the `cfg` of doc
/// tests and documentation, and a crate's features).
pub(crate) fn bypass_tokens() -> Vec<(String, &'static [&'static str])> {
    let rust: &[&str] = &["rs"];
    let rust_or_manifest: &[&str] = &["rs", "toml"];
    let typescript: &[&str] = &BYPASS_TYPESCRIPT;
    vec![
        (format!("{}::{}", "mutants", "skip"), rust),
        (format!("{}(off)", "coverage"), rust),
        (format!("cfg({}", "coverage"), rust),
        (format!("{}_{}", "coverage", "nightly"), rust),
        (format!("{}_{}", "debug", "assertions"), rust),
        (format!("{}_{}", "unexpected", "cfgs"), rust_or_manifest),
        (format!("{}-{}", "check", "cfg"), rust_or_manifest),
        (format!(".{}(", "only"), typescript),
        (format!(".{}(", "skip"), typescript),
        (format!(".{}(", "todo"), typescript),
        (format!(".{}(", "fixme"), typescript),
        (format!("{}If(", "skip"), typescript),
        (format!("{}If(", "run"), typescript),
    ]
}

/// What ignores a Rust test or makes an attribute conditional, as rustc accepts it: an
/// `ignore` attribute, written with any whitespace or as a raw identifier, and any
/// conditional attribute (its condition may name the coverage build or Miri, where the
/// ignored tests run, and it may ignore a test there); and the `cfg` of doc tests and of
/// documentation as a condition's first, last or middle term (rustc accepts both silently,
/// and code under them is compiled only by rustdoc, whose builds the checks neither lint nor
/// cover). Built at run time, like [`bypass_tokens`]; [`bypass_gate`] seeks them in a whole
/// file, without case or whitespace.
pub(crate) fn rust_hints() -> Vec<String> {
    let mut hints = vec![
        format!("#[{}", "ignore"),
        format!("r#{}", "ignore"),
        format!("{}_{}", "cfg", "attr"),
    ];
    for name in [format!("{}{}", "doc", "test"), "doc".to_owned()] {
        hints.extend([
            format!("({name})"),
            format!("({name},"),
            format!(",{name})"),
            format!(",{name},"),
        ]);
    }
    hints
}

/// What replaces a Cargo target's tests by a program of its own, unseen by cargo metadata:
/// the `harness` key of a target, in any spelling of its whitespace or case; and a crate's
/// features table, since code under a feature off by default is built by no check. Built
/// at run time, like [`bypass_tokens`]; [`bypass_gate`] seeks them in a whole manifest,
/// without case or whitespace.
pub(crate) fn manifest_hints() -> Vec<String> {
    vec![format!("{}{}", "har", "ness"), format!("[{}]", "features")]
}

/// The `cfg` Miri sets: a test that names it can leave the Miri run ([`miri_command`](crate::steps::miri_command)), so
/// [`bypass_gate`] refuses it as a word anywhere in Rust code (a raw identifier included),
/// except in the exact string literal that names the tool.
pub(crate) const MIRI: &str = "miri";

/// Whether `line`, Rust code, holds the word [`MIRI`] outside the string literal of
/// exactly that word: neither preceded nor followed by a letter, digit or underscore.
pub(crate) fn names_miri(line: &str) -> bool {
    let code = line.replace(&format!("{MIRI:?}"), "");
    let word = |character: char| character.is_alphanumeric() || character == '_';
    code.match_indices(MIRI).any(|(at, _)| {
        let before = code.get(..at).and_then(|text| text.chars().next_back());
        let after = code
            .get(at.saturating_add(MIRI.len())..)
            .and_then(|text| text.chars().next());
        !before.into_iter().chain(after).any(word)
    })
}

/// The extensions of the TypeScript and JavaScript files whose bypasses [`bypass_gate`]
/// refuses: those of [`ts_sources`](crate::workspace::ts_sources).
pub(crate) const BYPASS_TYPESCRIPT: [&str; 9] = [
    "ts", "tsx", "mts", "cts", "astro", "js", "jsx", "mjs", "cjs",
];

/// What makes a TypeScript coverage tool exclude code: the ignore comments of its
/// providers, which accept any whitespace between their words and any case, and the option
/// excluding methods by name. Built at run time, like [`bypass_tokens`]; [`bypass_gate`]
/// seeks them in a whole file, without case or whitespace.
pub(crate) fn coverage_hints() -> [String; 5] {
    [
        format!("{} {}", "v8", "ignore"),
        format!("{} {}", "c8", "ignore"),
        format!("{} {}", "istanbul", "ignore"),
        format!("{}:{} {}", "node", "coverage", "ignore"),
        format!("{}{}", "ignore", "ClassMethods"),
    ]
}

/// The line (numbered from 0) of `text` where each occurrence of `hint` starts, both
/// compared in lowercase with every whitespace removed, so that no spacing, line break or
/// case hides it.
pub(crate) fn hint_lines(text: &str, hint: &str) -> Vec<usize> {
    let mut line = 0_usize;
    let mut compact = Vec::new();
    for character in text.chars() {
        if character == '\n' {
            line = line.saturating_add(1);
        }
        if !character.is_whitespace() {
            compact.extend(character.to_lowercase().map(|lower| (lower, line)));
        }
    }
    let needle: Vec<char> = hint
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    compact
        .windows(needle.len())
        .filter(|window| window.iter().map(|(character, _)| character).eq(&needle))
        .filter_map(|window| window.first().map(|(_, start)| *start))
        .collect()
}

/// The attributes of a doc test that keep it from running: rustdoc never compiles an
/// `ignore` test, even with [`INCLUDE_IGNORED`](crate::steps::INCLUDE_IGNORED), and compiles a `no_run` one only.
pub(crate) const SILENT_DOC_TESTS: [&str; 2] = ["ignore", "no_run"];

/// The attribute of [`SILENT_DOC_TESTS`] that `line`, a fence opening a code block of Rust
/// documentation (three backquotes or tildes), carries, if any.
pub(crate) fn silent_doc_test(line: &str) -> Option<&'static str> {
    let fences = ["`".repeat(3), "~".repeat(3)];
    let info = fences
        .iter()
        .find_map(|fence| line.split_once(fence.as_str()))
        .map(|(_, info)| info)?;
    SILENT_DOC_TESTS
        .into_iter()
        .find(|attribute| info.contains(attribute))
}

/// The fixtures of the shared TypeScript policy: each holds on purpose what a rule forbids,
/// to prove the rule rejects it, and is never run as a test.
pub const POLICY_FIXTURES: [&str; 2] = [
    "tooling/config/test/policy/",
    "tooling/config/test/compiler/",
];

/// The tracked files the bypass gate reads (git pathspecs): the configurations of
/// cargo-mutants and Cargo, and every code file
/// ([`code_sources`](crate::workspace::code_sources)).
#[must_use]
pub fn bypass_sources() -> Vec<&'static str> {
    ["*mutants.toml", "*Cargo.toml"]
        .into_iter()
        .chain(crate::workspace::code_sources())
        .collect()
}

/// Checks that the reasons of a register of reviewed exceptions name what exists.
///
/// `registers` are each register's path and text (lines `<path>: <what> -- <reason>`,
/// comments `#` and blank lines apart: [`BYPASSES_REVIEWED`] and
/// [`EQUIVALENT_REVIEWED`]), `tracked` every tracked
/// file, and `words` every word of the tracked code (its identifiers). In a reason, a path (a
/// word holding a `/` and ending with an extension) names a tracked file, from the root or
/// from a directory above the entry's own path (`test/runner.test.ts` beside its package's
/// sources); and a name in code quotes that is an identifier (`caseHashes`, `mul_then_add`)
/// is a word of the code, so that a reason citing a test file or a function renamed or
/// removed since is refused. A test a reason names by its title is not read (only a review
/// sees it).
///
/// # Errors
///
/// Returns every path and every name a reason cites that does not exist, at its line.
pub fn register_names_gate(
    registers: &[(&str, &str)],
    tracked: &BTreeSet<String>,
    words: &BTreeSet<String>,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (register, text) in registers {
        for (index, line) in text.lines().enumerate() {
            let Some((entry, reason)) = line.split_once(" -- ").filter(|_| !line.starts_with('#'))
            else {
                continue;
            };
            let entry_path = entry.split_once(": ").map_or(entry, |(path, _)| path);
            let at = format!("{register}:{}", index.saturating_add(1));
            for path in reason
                .split(|character: char| character.is_whitespace() || "`(),;".contains(character))
                .map(|word| word.trim_end_matches(['.', ':']))
                .filter(|word| {
                    word.contains('/')
                        && word.rsplit_once('.').is_some_and(|(_, extension)| {
                            extension
                                .chars()
                                .all(|character| character.is_ascii_alphanumeric())
                        })
                })
            {
                let found = core::iter::once(String::new())
                    .chain(
                        std::path::Path::new(entry_path)
                            .ancestors()
                            .skip(1)
                            .map(|dir| dir.display().to_string())
                            .filter(|dir| !dir.is_empty())
                            .map(|dir| format!("{dir}/")),
                    )
                    .any(|dir| tracked.contains(&format!("{dir}{path}")));
                if !found {
                    problems.push(format!(
                        "{at}: the reason cites {path}, which no tracked file is (from the root or \
                         a directory above {entry_path}): cite the file as it is now"
                    ));
                }
            }
            for name in reason.split('`').skip(1).step_by(2).filter(|name| {
                name.chars().any(char::is_alphabetic)
                    && name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_')
            }) {
                if !words.contains(name) {
                    problems.push(format!(
                        "{at}: the reason names `{name}`, which no tracked code holds: name what \
                         the code holds now"
                    ));
                }
            }
        }
    }
    step_verdict(problems)
}

/// Reviewed bypasses of coverage or mutation testing: every occurrence of a bypass token
/// ([`bypass_gate`]), each with the reason it is legitimate. Empty while there is none.
pub const BYPASSES_REVIEWED: &str = "proof-bypasses.txt";

/// Checks that nothing removes code from coverage or mutation testing unseen.
///
/// `sources` are the tracked source files (path and text): every line holding a bypass
/// token (a skipped mutant, an ignored test, a coverage exclusion, an
/// ignore comment of a coverage tool, a focused or skipped test, each in the languages
/// where it acts) outside the [`POLICY_FIXTURES`] is an occurrence, and so is any tracked
/// `mutants.toml` (a cargo-mutants configuration can exclude code); every occurrence must
/// be reviewed in `reviewed` ([`BYPASSES_REVIEWED`]: one `<file>: <token> at <line> --
/// <reason>` line per
/// occurrence), and every review must be current. The reviews of [`scope_gate`] are its
/// own, but one of a file in no package of [`PACKAGES`] is stale here, since no scope gate
/// reads it.
///
/// # Errors
///
/// Returns every unreviewed occurrence, stale review and review without a reason.
pub fn bypass_gate(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    let tokens = bypass_tokens();
    let mut keys = Vec::new();
    for (path, text) in sources {
        if POLICY_FIXTURES
            .iter()
            .any(|fixtures| path.starts_with(fixtures))
        {
            continue;
        }
        if path.rsplit('/').next() == Some("mutants.toml") {
            keys.push(format!("{path}: a cargo-mutants configuration"));
            continue;
        }
        let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines {
            for (token, extensions) in &tokens {
                if extensions.contains(&extension) && line.contains(token.as_str()) {
                    keys.push(format!("{path}: {token} at {}", line.trim()));
                }
            }
            if extension == "rs" && names_miri(line) {
                keys.push(format!("{path}: {MIRI} at {}", line.trim()));
            }
            if let Some(attribute) = silent_doc_test(line).filter(|_| extension == "rs") {
                keys.push(format!(
                    "{path}: {}{attribute} at {}",
                    "`".repeat(3),
                    line.trim()
                ));
            }
        }
        let hints = if BYPASS_TYPESCRIPT.contains(&extension) {
            coverage_hints().to_vec()
        } else if extension == "rs" {
            rust_hints()
        } else if extension == "toml" {
            manifest_hints()
        } else {
            Vec::new()
        };
        for hint in hints {
            for at in hint_lines(text, &hint) {
                let line = lines.get(at).map_or("", |line| line.trim());
                keys.push(format!("{path}: {hint} at {line}"));
            }
        }
    }
    let mut problems = Vec::new();
    compare_reviews(
        &count(keys),
        Extent::Complete,
        &reviews_of(reviewed, false),
        &mut problems,
    );
    let dirs = PACKAGES.map(|package| package.dir);
    problems.extend(
        reviews_of(reviewed, true)
            .lines()
            .map(review_key)
            .filter(|key| !key.starts_with('#') && !reviews_below(key, &dirs))
            .map(|key| format!("stale review: {key} (a file of no package)")),
    );
    verdict(
        &format!(
            "bypasses of coverage or mutation testing must be reviewed in {BYPASSES_REVIEWED}"
        ),
        problems,
    )
}

/// The suffixes of the reviews of [`scope_gate`] in [`BYPASSES_REVIEWED`].
///
/// A source the coverage of its package does not measure, one it measures but Stryker
/// does not mutate, and one with statements that the `mutate` globs select but the mutation
/// report lacks (Stryker found no mutant in it, or read another configuration). Every other
/// review of that file belongs to [`bypass_gate`].
pub const SCOPE_REVIEWS: [&str; 3] = [
    ": not measured",
    ": measured, not mutated",
    ": mutated, no mutant reported",
];

/// The key of a review line: what precedes its reason.
pub(crate) fn review_key(line: &str) -> &str {
    line.split_once(" -- ").map_or(line, |(key, _)| key).trim()
}

/// The lines of `reviewed` that are reviews of [`scope_gate`] (`scope`), or the others.
pub(crate) fn reviews_of(reviewed: &str, scope: bool) -> String {
    reviewed
        .lines()
        .filter(|line| {
            SCOPE_REVIEWS
                .iter()
                .any(|suffix| review_key(line).ends_with(suffix))
                == scope
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The reviews of [`scope_gate`] in `reviewed` that name a file below one of `dirs`.
pub(crate) fn scope_reviews_below(reviewed: &str, dirs: &[&str]) -> String {
    reviews_of(reviewed, true)
        .lines()
        .filter(|line| reviews_below(review_key(line), dirs))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether the review key `key` names a file below one of `dirs`.
pub(crate) fn reviews_below(key: &str, dirs: &[&str]) -> bool {
    dirs.iter().any(|dir| {
        key.strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// What [`scope_gate`] reads of one TypeScript package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Its directory, as in [`Package::dir`](crate::workspace::Package::dir).
    pub dir: &'static str,
    /// The text of its `stryker.config.json`.
    pub stryker: String,
    /// The text of its coverage summary (Vitest `json-summary`), whose keys are the
    /// absolute paths of the files measured.
    pub summary: String,
    /// The text of its mutation report (Stryker `json`), complete: its `files` are the
    /// sources Stryker read, relative to the package.
    pub mutation: String,
}

/// Whether `rel`, a path relative to its package, is test code: under `test/`, or a test or
/// end-to-end specification file. Tests are what measures, not what is measured.
pub(crate) fn is_test(rel: &str) -> bool {
    rel.starts_with("test/")
        || [".test.ts", ".test.tsx", ".spec.ts"]
            .iter()
            .any(|suffix| rel.ends_with(suffix))
}

/// Whether `name` (one path segment) matches `pattern`, where `*` matches any run of
/// characters.
pub(crate) fn segment_matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let middle: Vec<&str> = parts.collect();
    let Some((last, between)) = middle.split_last() else {
        return rest.is_empty();
    };
    for part in between {
        match rest.find(part) {
            Some(at) => {
                rest = rest
                    .get(at.saturating_add(part.len())..)
                    .unwrap_or_default();
            }
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Whether the path `segments` match the glob `pattern` (segments too), where `**` matches
/// any number of whole segments, none included.
pub(crate) fn glob_matches(pattern: &[&str], segments: &[&str]) -> bool {
    match (pattern.split_first(), segments.split_first()) {
        (None, None) => true,
        (Some((&"**", rest)), _) => {
            glob_matches(rest, segments)
                || segments
                    .split_first()
                    .is_some_and(|(_, tail)| glob_matches(pattern, tail))
        }
        (Some((glob, rest)), Some((name, tail))) => {
            segment_matches(glob, name) && glob_matches(rest, tail)
        }
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// The characters of a glob syntax [`glob_matches`] does not understand (braces, classes,
/// single characters, negations, extended globs): a `mutate` glob holding one is refused
/// rather than read wrongly.
pub(crate) const GLOB_UNSUPPORTED: [char; 9] = ['{', '}', '[', ']', '?', '!', '(', ')', '@'];

/// The Stryker configuration, the coverage summary and the mutation report of `scope`, or
/// the first that cannot be read.
pub(crate) fn parse_scope(scope: &Scope) -> Result<[serde_json::Value; 3], String> {
    let dir = scope.dir;
    let parse = |text: &str, what: &str| {
        serde_json::from_str(text).map_err(|error| format!("{dir}: invalid {what}: {error}"))
    };
    Ok([
        parse(&scope.stryker, "Stryker configuration")?,
        parse(&scope.summary, "coverage summary")?,
        parse(&scope.mutation, "mutation report")?,
    ])
}

/// Checks that every TypeScript source of the packages of `scopes` is both measured and
/// mutated, or reviewed.
///
/// `tracked` are the files tracked with an extension of [`ts_sources`](crate::workspace::ts_sources); `scopes` what each
/// package checked (one, or several, of [`PACKAGES`]) measures (its coverage summary) and mutates
/// (the `mutate` globs of its Stryker configuration, of which only `*` and `**` are understood).
/// The sources of a package are its tracked files but its tests (`test/`, `*.test.ts`,
/// `*.spec.ts`). Each source must be measured and mutated; one measured but not mutated is a
/// `<source>: measured, not mutated` occurrence, one neither measured nor mutated a
/// `<source>: not measured` occurrence, and one mutated, measured with statements (or with
/// no count of them) but absent from the package's mutation report (which a check makes
/// complete, the incremental runs included) a `<source>: mutated, no mutant reported`
/// occurrence: Stryker found no mutant in it, or ran with another configuration than
/// [`STRYKER_CONFIG`]. Each is to be reviewed in `reviewed`
/// ([`BYPASSES_REVIEWED`], whose other reviews are [`bypass_gate`]'s), every review of a
/// file of these packages current (a review of a file in no package is the bypass gate's
/// to refuse, so that each package can be checked alone). So the coverage `include` and the Stryker
/// `mutate` of a package select the same sources, but for reviewed differences, and no source
/// escapes both unseen.
///
/// # Errors
///
/// Returns every source mutated but not measured, every file measured that is not a source
/// of the package, every `mutate` glob that is not understood or matches no source, every
/// unreadable configuration, summary or mutation report, every mutation report without
/// `files`, and every unreviewed occurrence or stale review.
pub fn scope_gate(tracked: &[String], scopes: &[Scope], reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for scope in scopes {
        let dir = scope.dir;
        let prefix = format!("{dir}/");
        let sources: Vec<&str> = tracked
            .iter()
            .filter_map(|path| path.strip_prefix(&prefix))
            .filter(|rel| !is_test(rel))
            .collect();
        let [stryker, summary, mutation] = match parse_scope(scope) {
            Ok(parsed) => parsed,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        let globs: Vec<&str> = stryker
            .get("mutate")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect();
        let Some(reported) = mutation.get("files").and_then(serde_json::Value::as_object) else {
            problems.push(format!("{dir}: the mutation report lists no `files`"));
            continue;
        };
        // Each source measured, with whether it has statements: one without any (only
        // declarations) has nothing to mutate, and Stryker leaves it out of its report.
        let mut measured = BTreeMap::new();
        for (file, entry) in summary
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| *key != "total")
        {
            match sources
                .iter()
                .find(|rel| file.ends_with(&format!("/{prefix}{rel}")))
            {
                Some(rel) => {
                    let statements = entry.pointer("/statements/total");
                    let _new = measured.insert(*rel, statements != Some(&0_u64.into()));
                }
                None => problems.push(format!(
                    "{dir}: the coverage measures {file}, which is not a tracked source of the package"
                )),
            }
        }
        let mut used = BTreeSet::new();
        for glob in &globs {
            if glob.contains(GLOB_UNSUPPORTED) {
                problems.push(format!(
                    "{dir}/stryker.config.json: `{glob}` in mutate: only `*` and `**` are understood"
                ));
                let _new = used.insert(*glob);
            }
        }
        for rel in &sources {
            let segments: Vec<&str> = rel.split('/').collect();
            let matching: Vec<&str> = globs
                .iter()
                .copied()
                .filter(|glob| glob_matches(&glob.split('/').collect::<Vec<_>>(), &segments))
                .collect();
            used.extend(matching.iter().copied());
            match (measured.get(rel), !matching.is_empty()) {
                (Some(true), true) if !reported.contains_key(*rel) => {
                    keys.push(format!("{prefix}{rel}: mutated, no mutant reported"));
                }
                (Some(_), true) => {}
                (None, true) => problems.push(format!(
                    "{prefix}{rel} is mutated but not measured: the coverage `include` misses it"
                )),
                (Some(_), false) => keys.push(format!("{prefix}{rel}: measured, not mutated")),
                (None, false) => keys.push(format!("{prefix}{rel}: not measured")),
            }
        }
        problems.extend(
            globs
                .iter()
                .filter(|glob| !used.contains(*glob))
                .map(|glob| {
                    format!("{dir}/stryker.config.json: `{glob}` in mutate matches no source")
                }),
        );
    }
    let dirs: Vec<&str> = scopes.iter().map(|scope| scope.dir).collect();
    compare_reviews(
        &count(keys),
        Extent::Complete,
        &scope_reviews_below(reviewed, &dirs),
        &mut problems,
    );
    verdict(
        &format!(
            "TypeScript sources outside coverage or mutation testing must be reviewed in {BYPASSES_REVIEWED}"
        ),
        problems,
    )
}

/// The disable comments of `sources` (path and text of each TypeScript file), and a problem
/// for every malformed one.
///
/// A disable comment must be `// Stryker disable next-line <Mutator>: <reason>`, one
/// mutator, never `all`, with a reason, above a line that is not empty; any other use of
/// `Stryker disable` is refused.
pub(crate) fn disables(sources: &[(String, String)]) -> (Vec<(Disable, String)>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for (path, text) in sources {
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("Stryker disable") {
                continue;
            }
            let comment = line.trim();
            let rule = comment
                .strip_prefix(STRYKER_DISABLE)
                .and_then(|rest| rest.split_once(':'))
                .filter(|(mutator, reason)| {
                    !mutator.contains([' ', ',']) && *mutator != "all" && !reason.trim().is_empty()
                });
            let next = lines
                .get(index.saturating_add(1))
                .map_or("", |following| following.trim());
            match rule {
                Some((mutator, _reason)) if !next.is_empty() => found.push((
                    Disable {
                        path: path.clone(),
                        line: index.saturating_add(2),
                        mutator: mutator.to_owned(),
                    },
                    next.to_owned(),
                )),
                _ => problems.push(format!(
                    "{path}:{}: write `{STRYKER_DISABLE}<Mutator>: <reason>` above the line",
                    index.saturating_add(1)
                )),
            }
        }
    }
    (found, problems)
}

/// Identifies a mutant independently of line and column numbers, so that unrelated
/// edits do not invalidate a review: `crates/a/src/b.rs: replace f -> T with x`.
pub(crate) fn mutant_key(line: &str) -> Option<String> {
    let (file, rest) = line.split_once(".rs:")?;
    let mut parts = rest.splitn(3, ':');
    let (Some(_line), Some(_column), Some(description)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some(format!("{file}.rs: {}", description.trim()))
}

/// Counts each key of `keys`.
pub(crate) fn count<I: IntoIterator<Item = String>>(keys: I) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in keys {
        let entry: &mut usize = counts.entry(key).or_default();
        *entry = entry.saturating_add(1);
    }
    counts
}

/// Compares what a gate `found`, each key with how many times, with the keys of a review
/// file, adding to `problems` every line without a reason, every key found more often than
/// reviewed and, when `found` is [`Extent::Complete`], every review that is stale (after a
/// partial run, a key reviewed more often than found may be found elsewhere).
///
/// `reviewed` holds one `<key> -- <reason>` line per occurrence (a key found twice is
/// reviewed twice, each with its reason); blank lines and lines starting with `#` are
/// ignored.
pub(crate) fn compare_reviews(
    found: &BTreeMap<String, usize>,
    extent: Extent,
    reviewed: &str,
    problems: &mut Vec<String>,
) {
    let known = reviewed_keys(reviewed, problems);
    for (key, &seen) in found {
        let listed = known.get(key).copied().unwrap_or_default();
        if seen > listed {
            problems.push(format!("not reviewed: {}", counted(key, seen, listed)));
        }
    }
    if extent == Extent::Complete {
        stale_reviews(found, &known, problems);
    }
}

/// The keys of a review file, each with how many times it is reviewed, adding to `problems`
/// every line without a reason (see [`compare_reviews`] for the format).
pub(crate) fn reviewed_keys(reviewed: &str, problems: &mut Vec<String>) -> BTreeMap<String, usize> {
    let mut keys = Vec::new();
    for line in reviewed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        match line.split_once(" -- ") {
            // The line is trimmed, so text always follows the separator: the reason is never empty.
            Some((key, _reason)) => keys.push(key.trim().to_owned()),
            None => problems.push(format!("review without a reason: {line}")),
        }
    }
    count(keys)
}

/// A key with its counts, when both are nonzero: `<key> (<seen> found, <listed> reviewed)`.
pub(crate) fn counted(key: &str, seen: usize, listed: usize) -> String {
    match (seen, listed) {
        (_, 0) | (0, _) => key.to_owned(),
        _ => format!("{key} ({seen} found, {listed} reviewed)"),
    }
}

/// Adds to `problems` every review of `known` (each key with how many times it is
/// reviewed) that is stale: reviewed more often than `found` holds it.
pub(crate) fn stale_reviews(
    found: &BTreeMap<String, usize>,
    known: &BTreeMap<String, usize>,
    problems: &mut Vec<String>,
) {
    for (key, &listed) in known {
        let seen = found.get(key).copied().unwrap_or_default();
        if listed > seen {
            problems.push(format!("stale review: {}", counted(key, seen, listed)));
        }
    }
}

/// Checks that the unviable mutants of the last run are reviewed and, after a
/// [`Extent::Complete`] run, exactly the reviewed ones.
///
/// A mutant is known by its file and description, not its line, so that
/// unrelated edits do not invalidate a review; two mutants with the same key (the same
/// operator twice in one function) need two reviews. `reviewed` holds one
/// `<file>.rs: <mutant> -- <reason>` line per mutant; blank lines and lines starting with
/// `#` are ignored.
///
/// # Errors
///
/// Returns a message listing unreviewed mutants, stale reviews (after a complete run) and
/// malformed lines.
pub fn unviable_gate(report: &str, extent: Extent, reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for line in report.lines().filter(|line| !line.trim().is_empty()) {
        match mutant_key(line) {
            Some(key) => keys.push(key),
            None => problems.push(format!("unrecognized mutant: {line}")),
        }
    }
    let found = count(keys);
    compare_reviews(&found, extent, reviewed, &mut problems);
    verdict(
        &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
        problems,
    )
}

/// The command listing every mutant of the workspace without building or testing any
/// (`cargo mutants --list`), with the file selection of every run (the arguments of every
/// cargo-mutants command).
#[must_use]
pub fn mutants_list_command() -> Cmd {
    Cmd::new(
        "cargo",
        &[&MUTANTS_ARGS[..], &["--list", "--workspace"]].concat(),
        Dir::Root,
    )
}

/// Checks that every review of [`UNVIABLE_REVIEWED`] names a mutant cargo-mutants can still
/// generate.
///
/// `listed` is every mutant of the workspace ([`mutants_list_command`]), so a
/// review of a mutant it no longer holds, or reviewed more often than it holds it, is stale.
/// The default check, which mutates only the changes, refuses stale reviews this way, as
/// the full check does after its complete run.
///
/// # Errors
///
/// Returns every stale review, every review without a reason and every line of `listed`
/// that is not a mutant.
pub fn unviable_reviews_gate(listed: &str, reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for line in listed.lines().filter(|line| !line.trim().is_empty()) {
        match mutant_key(line) {
            Some(key) => keys.push(key),
            None => problems.push(format!("unrecognized mutant: {line}")),
        }
    }
    let known = reviewed_keys(reviewed, &mut problems);
    stale_reviews(&count(keys), &known, &mut problems);
    verdict(
        &format!("every review of {UNVIABLE_REVIEWED} must name a mutant that exists"),
        problems,
    )
}

/// The comment that disables one mutator on the next line, in TypeScript sources.
pub(crate) const STRYKER_DISABLE: &str = "// Stryker disable next-line ";

/// Checks that the mutants disabled in `sources` (path and text of each TypeScript file)
/// are exactly the reviewed ones.
///
/// A disable comment must be `// Stryker disable next-line <Mutator>: <reason>`, one
/// mutator, never `all`, with a reason; any other use of `Stryker disable` is refused.
/// `reviewed` holds one `<file>: <Mutator> at <next line> -- <reason>` line per comment;
/// blank lines and lines starting with `#` are ignored.
///
/// # Errors
///
/// Returns a message listing malformed comments, unreviewed and stale entries.
pub fn equivalent_gate(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    let (found, mut problems) = disables(sources);
    let keys = found
        .into_iter()
        .map(|(disable, next)| format!("{}: {} at {next}", disable.path, disable.mutator));
    compare_reviews(&count(keys), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!("disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}"),
        problems,
    )
}

/// Checks that the manifests tracked by git are exactly those xtask knows.
///
/// The tracked `package.json` files are exactly [`package_jsons`]: the root one and one per
/// package of [`PACKAGES`], whose checks and reports therefore run. Every crate manifest
/// (`Cargo.toml` below the root) takes the workspace version and repository, and inherits
/// the workspace lints. The only `.gitignore` is the
/// root one: build outputs go to `dist/` or `target/`, never hidden inside a source folder.
///
/// # Errors
///
/// Returns every manifest unknown to xtask or missing from the repository, and every crate
/// with a version of its own.
pub fn membership_gate(
    packages: &[String],
    crates: &[(String, String)],
    ignores: &[String],
) -> Result<(), Failure> {
    let tracked: BTreeSet<&str> = packages.iter().map(String::as_str).collect();
    let expected = package_jsons();
    let known: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    let mut problems: Vec<String> = tracked
        .difference(&known)
        .map(|path| {
            format!("{path} is not a package of PACKAGES: its checks and reports would not run")
        })
        .chain(
            known
                .difference(&tracked)
                .map(|path| format!("{path} belongs to PACKAGES but is not tracked")),
        )
        .collect();
    let manifests: BTreeSet<&str> = crates
        .iter()
        .map(|(path, _)| path.as_str())
        .filter(|path| *path != CARGO_TOML && !path.starts_with(FIXTURES))
        .collect();
    let members: Vec<String> = RUST_CRATES
        .iter()
        .map(|krate| format!("{}/Cargo.toml", krate.dir))
        .collect();
    let members: BTreeSet<&str> = members.iter().map(String::as_str).collect();
    problems.extend(manifests.difference(&members).map(|path| {
        format!("{path} is not a crate of RUST_CRATES: its coverage would not be measured")
    }));
    problems.extend(
        members
            .difference(&manifests)
            .map(|path| format!("{path} belongs to RUST_CRATES but is not tracked")),
    );
    for (key, what) in [("version", "version"), ("repository", "repository")] {
        let inherited = format!("{key}.workspace = true");
        problems.extend(
            crates
                .iter()
                .filter(|(path, _)| path != CARGO_TOML && !path.starts_with(FIXTURES))
                .filter(|(_, text)| !text.lines().any(|line| line.trim() == inherited))
                .map(|(path, _)| format!("{path} does not take the workspace {what}")),
        );
    }
    // Cargo refuses any `[lints.*]` table next to `workspace = true`: this pins the whole
    // policy, which a crate without it would silently lose.
    problems.extend(
        crates
            .iter()
            .filter(|(path, _)| path != CARGO_TOML && !path.starts_with(FIXTURES))
            .filter(|(_, text)| {
                let lines: Vec<&str> = text.lines().map(str::trim).collect();
                !lines
                    .windows(2)
                    .any(|pair| pair == ["[lints]", "workspace = true"])
            })
            .map(|(path, _)| {
                format!("{path} does not inherit the workspace lints (`[lints]` then `workspace = true`)")
            }),
    );
    problems.extend(
        ignores
            .iter()
            .filter(|path| path.as_str() != ".gitignore")
            .map(|path| {
                format!(
                    "{path}: build outputs go to dist/ or target/, not hidden in a source folder"
                )
            }),
    );
    verdict("the manifests differ from what xtask knows", problems)
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
