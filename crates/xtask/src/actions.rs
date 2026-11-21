//! Running the steps: the inputs each action reads and how its verdict is said.

use alloc::collections::BTreeSet;
use core::num::NonZeroUsize;
use core::time::Duration;
use std::io;
use std::io::Write;

use crate::{
    changes::{licenses, main_base, rust_mutants_of_changed_lines, rust_mutants_of_changes},
    citations::{citation_gate, citation_sources},
    cli::{bless, conformance},
    commits::SCOPES_SINCE,
    conflicts::{GIT_ATTRIBUTES, binary_suffixes, conflict_marker_gate},
    corpus::{CORPUS_EXEMPT, corpus_mutants},
    counts::list_counts_gate,
    coverage::{
        TreeState, coverage_gate, coverage_locality, measure, tree_gate, tree_state,
        ts_coverage_gate,
    },
    dependencies::{
        BUILD_TIME_REVIEWED, DEPENDENCIES_REVIEWED, build_time_gate, dependency_gate, library_gate,
        merged_metadata, metadata_command, pin_gate, resolved_metadata_command, shared_tests_gate,
        shared_versions_gate,
    },
    float::float_methods,
    hints::runnable_hints_gate,
    history::{
        BREAKING_SINCE, CHANGELOG_SINCE, breaking_bang_gate, breaking_log_command,
        history_commands, history_gate, history_pins, pins_command, pins_gate, pre_push_gate,
    },
    licenses::{LICENSE_FILES, LICENSE_STATEMENTS, part_license_gate},
    loaded_files::{loaded_files_of_the_workspace, workflows},
    math_layer::{MATH_LAYER_SOURCES, math_layer_gate, named_sources_gate},
    parts::{Part, full_parts, part_steps, parts_gate, preparation_steps},
    preflight::{
        CARGO_CONFIG_FILE, GPU_MANIFEST, GpuDriver, RUST_TOOLCHAIN_FILE, TOOLS, cargo_config_gate,
        gpu_manifest_gate, mutant_left_gate, mutant_search_command, preflight_gate,
    },
    proof_record::{record_mutation_proof, tree_cleanliness},
    release::{
        CARGO_TOML, CHANGELOG, breaking_names, breaking_spellings_gate, duplicate_entries_gate,
        release_gate, released_sections, unreleased_shape_gate,
    },
    retired::{RETIRED_PATTERNS, retired_patterns_gate},
    reviews::{
        BYPASSES_REVIEWED, Disable, LinkedModule, STRYKER_CONFIG, Scope, bypass_gate,
        bypass_sources, disables, equivalent_gate, is_test, linked_conformance_gate,
        membership_gate, mutants_list_command, package_check_gate, package_order_gate,
        register_names_gate, scope_gate, shared_constants_gate, stryker_config_gate,
        stryker_scripts_gate, test_exports_gate, ts_mutation_gate, unviable_gate,
        unviable_reviews_gate,
    },
    runner::{CommandFailure, Exit, Failed, Runner, hermetic_git_gate},
    rust_source::RustSource,
    steps::{
        Action, Cmd, DOC_CODES_FIXTURE, DOC_TEST_PASS, Dir, Failure, MUTANTS_MISSED_EXIT, Step,
        UNCHECKED_CODE, all_of, check_steps, doc_codes_fixture_command, doc_test_pass_command,
        doc_tests_command, fused_command, fused_gate, gpu_mutants_command,
        gpu_mutants_list_command, listed_above, verdict,
    },
    supply_chain::{
        PNPM_SETTINGS_FILES, SUPPLY_CHAIN_FILES, cpu_reference_command, cpu_reference_gate,
        supply_chain_gate,
    },
    test_code::{
        compile_fail_codes_gate, review_names_gate, review_reasons_gate, spec_names_gate,
        test_code_gate, test_data_gate, untested_targets_gate, unviable_names_gate,
        unviable_reasons_gate,
    },
    test_order::{baseline_times, nextest_configs_gate, order_suggestion, slow_group},
    unsafety::unsafe_gate,
    vocabulary::{VOCABULARY, vocabulary_gate, vocabulary_sources},
    workflow::NODE_VERSION_FILE,
    workspace::{
        CargoWorkspace, EQUIVALENT_REVIEWED, Extent, GPU_MUTANTS_REPORT, GPU_WORKSPACE, Mode,
        OnFailure, PACKAGES, Package, RUST_CRATES, UNVIABLE_REPORT, UNVIABLE_REVIEWED,
        code_sources, library_lints_gate, no_std_portability_gate, package_jsons,
        source_kinds_gate, stryker_sources, ts_sources,
    },
};

/// Reads a file through `runner`, naming it when it cannot be read.
pub(crate) fn read_file(runner: &mut dyn Runner, path: &str) -> Result<String, Failure> {
    runner
        .read(path)
        .map_err(|error| Failure::from(format!("cannot read {path}: {error}")))
}

/// Captures what a git command run at the root writes, saying what could not be done
/// (`doing`: "read the work tree") when it fails.
pub(crate) fn git_output(
    runner: &mut dyn Runner,
    args: &[&str],
    doing: &str,
) -> Result<String, Failure> {
    runner
        .capture(&Cmd::new("git", args, Dir::Root))
        .map_err(|failure| Failure::from(format!("cannot {doing}: {failure}")))
}

/// Reads the commit checked out and the state of the working tree, then checks a push.
pub(crate) fn pre_push(runner: &mut dyn Runner, pushed: &[String]) -> Result<(), Failure> {
    let head = git_output(runner, &["rev-parse", "HEAD"], "read the work tree")?;
    let view = tree_cleanliness(runner)?;
    // Git gives the hook the object pushed: for an annotated tag, the tag object, which is
    // never the commit checked out. The gate compares the commit each object names.
    let commits = pushed
        .iter()
        .map(|object| {
            if is_deletion(object) {
                return Ok(object.clone());
            }
            git_output(
                runner,
                &["rev-parse", "--verify", &format!("{object}^{{commit}}")],
                &format!("resolve the pushed {object}"),
            )
            .map(|commit| commit.trim().to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    pre_push_gate(&head, &view, &commits)
}

/// Whether git pushes `object` to delete a ref: the hook then receives an object name of
/// all zeros, 40 of them (SHA-1) or 64 (SHA-256).
pub(crate) fn is_deletion(object: &str) -> bool {
    [40, 64].contains(&object.len()) && object.bytes().all(|byte| byte == b'0')
}

/// Reads the history of `main` and checks the gate's pins in it ([`pins_gate`]), then
/// reads the history of `HEAD` and checks it.
pub(crate) fn history(runner: &mut dyn Runner) -> Result<(), Failure> {
    let (base, against) = main_base(runner, "the commits the history gate is pinned to")?;
    let ancestors = runner.capture(&pins_command(&base)).map_err(|failure| {
        Failure::from(format!("cannot read the history of {against}: {failure}"))
    })?;
    // A precondition: the history read below starts at the pins.
    pins_gate(&history_pins(), &ancestors, (&base, against))?;
    let [messages, files, scoped] = history_commands(CHANGELOG_SINCE, SCOPES_SINCE).map(|cmd| {
        runner
            .capture(&cmd)
            .map_err(|failure| Failure::from(format!("cannot read the history: {failure}")))
    });
    let breaking = runner
        .capture(&breaking_log_command(BREAKING_SINCE))
        .map_err(|failure| Failure::from(format!("cannot read the history: {failure}")))?;
    all_of([
        history_gate(&messages?, &files?, &scoped?),
        breaking_bang_gate(&breaking),
    ])
}

/// Asks every tool of [`TOOLS`] for its version, then checks them, a Vulkan driver too when
/// `driver` requires one.
pub(crate) fn preflight(runner: &mut dyn Runner, driver: GpuDriver) -> Result<(), Failure> {
    let answers: Vec<Result<String, CommandFailure>> = TOOLS
        .iter()
        .map(|tool| runner.capture(&Cmd::new(tool.program, tool.args, Dir::Root)))
        .collect();
    let node_version = read_file(runner, NODE_VERSION_FILE)?;
    let toolchain = read_file(runner, RUST_TOOLCHAIN_FILE)?;
    preflight_gate(
        std::env::consts::OS,
        &answers,
        &node_version,
        &toolchain,
        &runner.environment(),
        driver,
    )
}

/// The files tracked by git matching `patterns`, described as `what` if they cannot be
/// listed.
pub(crate) fn list_tracked(
    runner: &mut dyn Runner,
    patterns: &[&str],
    what: &str,
) -> Result<Vec<String>, Failure> {
    runner
        .tracked(patterns)
        .map_err(|failure| Failure::from(format!("cannot list {what}: {failure}")))
}

/// Reads each of `paths`, with its path.
pub(crate) fn read_all(
    runner: &mut dyn Runner,
    paths: Vec<String>,
) -> Result<Vec<(String, String)>, Failure> {
    paths
        .into_iter()
        .map(|path| read_file(runner, &path).map(|text| (path, text)))
        .collect()
}

/// Every tracked text file: every tracked file but those [`GIT_ATTRIBUTES`] declares
/// binary.
pub(crate) fn tracked_text_files(runner: &mut dyn Runner) -> Result<Vec<String>, Failure> {
    let binary = binary_suffixes(&read_file(runner, GIT_ATTRIBUTES)?)?;
    Ok(list_tracked(runner, &["*"], "the tracked files")?
        .into_iter()
        .filter(|path| !binary.iter().any(|suffix| path.ends_with(suffix.as_str())))
        .collect())
}

/// Reads the files `paths` as text ([`read_all`]) for a gate over text, a file that cannot
/// be read named with what helps: a binary file is declared so in [`GIT_ATTRIBUTES`], which
/// keeps it from every gate over text.
fn read_tracked_texts(
    runner: &mut dyn Runner,
    paths: Vec<String>,
) -> Result<Vec<(String, String)>, Failure> {
    read_all(runner, paths).map_err(|failure| {
        Failure::from(format!(
            "{} (a binary file is declared `*.<extension> binary` in {GIT_ATTRIBUTES})",
            failure.summary
        ))
    })
}

/// Reads every tracked text file ([`tracked_text_files`], [`read_tracked_texts`]), then
/// checks that none holds a conflict marker.
pub(crate) fn conflict_markers(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = tracked_text_files(runner)?;
    conflict_marker_gate(&read_tracked_texts(runner, paths)?)
}

/// Lists every tracked text file ([`tracked_text_files`]), then checks that each is of a
/// known kind ([`source_kinds_gate`]).
fn source_kinds(runner: &mut dyn Runner) -> Result<(), Failure> {
    source_kinds_gate(&tracked_text_files(runner)?)
}

/// Reads the tracked text files ([`tracked_text_files`], [`read_tracked_texts`]) and the
/// list of retired patterns, then checks them ([`retired_patterns_gate`], which skips its
/// own list and tests).
fn retired_patterns(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = tracked_text_files(runner)?;
    let sources = read_tracked_texts(runner, paths)?;
    retired_patterns_gate(&sources, &read_file(runner, RETIRED_PATTERNS)?)
}

/// Reads the tracked code files and READMEs ([`read_tracked_texts`]), then checks their
/// hints ([`runnable_hints_gate`]).
fn runnable_hints(runner: &mut dyn Runner) -> Result<(), Failure> {
    let mut paths = list_tracked(runner, &code_sources(), "the code files")?;
    paths.extend(list_tracked(runner, &[README_FILES], "the READMEs")?);
    runnable_hints_gate(&read_tracked_texts(runner, paths)?)
}

/// Reads the tracked code files ([`code_sources`]), then checks that each that starts git
/// starts it hermetically ([`hermetic_git_gate`]).
fn hermetic_git(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = list_tracked(runner, &code_sources(), "the code files")?;
    hermetic_git_gate(&read_all(runner, paths)?)
}

/// Reads the tracked files, the words of the tracked code and the TypeScript registers of
/// reviewed exceptions, then checks that each reason names what exists
/// ([`register_names_gate`]).
fn register_names(runner: &mut dyn Runner) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &["*"], "the tracked files")?
        .into_iter()
        .collect();
    let paths = list_tracked(runner, &code_sources(), "the code files")?;
    let words = read_all(runner, paths)?
        .iter()
        .flat_map(|(_, text)| {
            text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let bypasses = read_file(runner, BYPASSES_REVIEWED)?;
    let equivalent = read_file(runner, EQUIVALENT_REVIEWED)?;
    register_names_gate(
        &[
            (BYPASSES_REVIEWED, &bypasses),
            (EQUIVALENT_REVIEWED, &equivalent),
        ],
        &tracked,
        &words,
    )
}

/// Reads the tracked sources Stryker mutates ([`stryker_sources`]) and the reviews of their
/// equivalent mutants, then checks every disable comment is reviewed ([`equivalent_gate`]).
fn equivalents(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = list_tracked(runner, &stryker_sources(), "the TypeScript files")?;
    let sources = read_all(runner, paths)?;
    equivalent_gate(&sources, &read_file(runner, EQUIVALENT_REVIEWED)?)
}

/// The READMEs, as a git pathspec: the root's and every part's.
const README_FILES: &str = "*README.md";

/// Lists the manifests tracked by git, then checks them.
pub(crate) fn membership(runner: &mut dyn Runner) -> Result<(), Failure> {
    let packages = list_tracked(runner, &["*package.json"], "the *package.json files")?;
    let ignores = list_tracked(runner, &["*.gitignore"], "the *.gitignore files")?;
    let manifests = list_tracked(runner, &["*Cargo.toml"], "the *Cargo.toml files")?;
    membership_gate(&packages, &read_all(runner, manifests)?, &ignores)
}

/// Lists the parts (the packages and the crates) and the READMEs tracked by git, then checks
/// that every part is described.
pub(crate) fn readmes(runner: &mut dyn Runner) -> Result<(), Failure> {
    let readmes = list_tracked(runner, &["*README.md"], "the *README.md files")?;
    let readmes = read_all(runner, readmes)?;
    let parts: Vec<String> = PACKAGES
        .iter()
        .map(|package| package.dir.to_owned())
        .chain(RUST_CRATES.iter().map(|krate| krate.dir.to_owned()))
        .collect();
    let specs = list_tracked(runner, &[SPEC_FILES], "the specification files")?;
    let index = readmes
        .iter()
        .find(|(path, _)| path == SPEC_INDEX)
        .map_or("", |(_, text)| text.as_str());
    let sources = list_tracked(runner, &ts_sources(), "the TypeScript sources")?;
    let packages = PACKAGES.map(|package| package.dir);
    all_of([
        readme_gate(&parts, &readmes),
        spec_index_gate(&specs, index),
        module_table_gate(&sources, &readmes, &packages),
    ])
}

/// The folders of a TypeScript package whose modules its README describes one by one.
pub(crate) const MODULE_FOLDERS: [&str; 2] = ["src/", "ts/"];

/// Checks that the README of each package of `packages` (their directories) names every
/// module of its sources: each of `sources` (the tracked TypeScript files) under one of its
/// [`MODULE_FOLDERS`], test code ([`is_test`]) and declaration files (`.d.ts`) apart, named
/// in backquotes by its path in the package or by a directory holding it (`src/cli/`), so
/// that a module added after the README's table is not left out of it. `readmes` are the
/// tracked READMEs with their text; a package without one is the readme gate's to report.
///
/// # Errors
///
/// Returns every module its package's README does not name.
pub(crate) fn module_table_gate(
    sources: &[String],
    readmes: &[(String, String)],
    packages: &[&str],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for dir in packages {
        let readme = format!("{dir}/README.md");
        let text = readmes
            .iter()
            .find(|(path, _)| *path == readme)
            .map_or("", |(_, text)| text.as_str());
        let named: Vec<&str> = text.split('`').skip(1).step_by(2).collect();
        let modules = sources
            .iter()
            .filter_map(|path| path.strip_prefix(dir)?.strip_prefix('/'))
            .filter(|module| {
                MODULE_FOLDERS
                    .iter()
                    .any(|folder| module.starts_with(folder))
            })
            .filter(|module| !module.ends_with(".d.ts") && !is_test(module));
        for module in modules {
            let described = named
                .iter()
                .any(|name| *name == module || (name.ends_with('/') && module.starts_with(name)));
            if !described {
                problems.push(format!(
                    "{readme} does not name {module} (`{module}`, or its directory)"
                ));
            }
        }
    }
    verdict("modules not described", problems)
}

/// The tracked files of the specifications, as git takes the pattern: every file below
/// `spec/`.
const SPEC_FILES: &str = "spec/*";

/// The index of the specifications, which [`spec_index_gate`] holds to them.
const SPEC_INDEX: &str = "spec/README.md";

/// Checks that the index of the specifications (`index`, the text of [`SPEC_INDEX`]) links
/// every one of them (`files`, the tracked files below `spec/`), each as `](<name>)`, and no
/// other file of that folder: a specification no reader finds from the index, or a link to
/// one that is gone, is refused.
///
/// # Errors
///
/// Returns every file not linked and every link to a file of the folder that is not
/// tracked.
pub(crate) fn spec_index_gate(files: &[String], index: &str) -> Result<(), Failure> {
    let named: BTreeSet<&str> = files
        .iter()
        .filter_map(|path| path.strip_prefix("spec/"))
        .filter(|name| *name != "README.md")
        .collect();
    let linked: BTreeSet<&str> = index
        .split("](")
        .skip(1)
        .filter_map(|rest| rest.split_once(')'))
        .map(|(target, _)| target.split_once('#').map_or(target, |(file, _)| file))
        .filter(|target| !target.is_empty() && !target.contains('/') && !target.contains(':'))
        .collect();
    let problems =
        named
            .difference(&linked)
            .map(|name| format!("{SPEC_INDEX} does not link {name} (`[…]({name})`)"))
            .chain(linked.difference(&named).map(|name| {
                format!("{SPEC_INDEX} links {name}, which is not a tracked file of spec/")
            }))
            .collect();
    verdict("specifications not indexed", problems)
}

/// Checks that every part is described where a reader looks for it.
///
/// `parts` are the directories of the TypeScript packages and of the crates (`apps/roadmap`,
/// `crates/audio-core`); `readmes` the tracked `README.md` files with their text. Each
/// TypeScript package has its own README; each part is named, in backquotes, in the README
/// of the folder that holds it (the parts table of `crates/README.md`, `apps/README.md` …),
/// and each such folder is named in the root README. Crates are described by their crate
/// documentation, which rustdoc requires.
///
/// # Errors
///
/// Returns every missing README and every part a README does not name.
pub fn readme_gate(parts: &[String], readmes: &[(String, String)]) -> Result<(), Failure> {
    let text = |path: &str| {
        readmes
            .iter()
            .find(|(readme, _)| readme == path)
            .map(|(_, content)| content.as_str())
    };
    let mut problems = Vec::new();
    for package in PACKAGES {
        if text(&format!("{}/README.md", package.dir)).is_none() {
            problems.push(format!("{} has no README.md", package.dir));
        }
    }
    let mut folders = BTreeSet::new();
    for part in parts {
        let Some((folder, name)) = part.rsplit_once('/') else {
            problems.push(format!(
                "{part} is not inside a folder of parts (apps/, crates/ …)"
            ));
            continue;
        };
        let first = folders.insert(folder);
        match text(&format!("{folder}/README.md")) {
            // Said once per folder, not once per part it holds.
            None if first => problems.push(format!("{folder} has no README.md")),
            Some(content) if !content.contains(&format!("`{name}")) => {
                problems.push(format!("{folder}/README.md does not name {name}"));
            }
            None | Some(_) => {}
        }
    }
    let root = text("README.md").unwrap_or_default();
    problems.extend(
        folders
            .into_iter()
            .filter(|folder| !root.contains(&format!("`{folder}/`")))
            .map(|folder| format!("README.md does not name {folder}/")),
    );
    verdict("parts not described", problems)
}

/// Reads the Stryker configuration of every package and every package manifest, then checks
/// that no configuration hides a mutant and that no script runs Stryker otherwise.
pub(crate) fn stryker_configs(runner: &mut dyn Runner) -> Result<(), Failure> {
    let mut results = Vec::new();
    for package in PACKAGES {
        let path = format!("{}/{STRYKER_CONFIG}", package.dir);
        results.push(stryker_config_gate(package.dir, &read_file(runner, &path)?));
    }
    for path in package_jsons() {
        results.push(stryker_scripts_gate(&path, &read_file(runner, &path)?));
    }
    all_of(results)
}

/// Reads the version of the workspace, of every package and of the changelog, then checks
/// that they agree, that every line of `[Unreleased]` has a shape its gates read
/// ([`unreleased_shape_gate`]), that it holds one entry per change
/// ([`duplicate_entries_gate`]) and writes every breaking one as the gates read it
/// ([`breaking_spellings_gate`]), that the released sections are as on main, and that each
/// breaking entry cites something the last version has ([`breaking_names`]).
fn versions(runner: &mut dyn Runner) -> Result<(), Failure> {
    let cargo = read_file(runner, CARGO_TOML)?;
    let packages = read_all(runner, package_jsons())?;
    let changelog = read_file(runner, CHANGELOG)?;
    all_of([
        release_gate(&cargo, &packages, &changelog),
        unreleased_shape_gate(&changelog),
        duplicate_entries_gate(&changelog),
        breaking_spellings_gate(&changelog),
        released_sections(runner, &changelog),
        breaking_names(runner, &changelog),
    ])
}

/// Reads the manifest of every package, then checks that its `check` script runs exactly the
/// scripts its checks run, the order of the packages and the builds they need
/// ([`package_order_gate`]), that none exports test code ([`test_exports_gate`]), that
/// each module a package ships is held to the conformance runner of every library it links
/// ([`linked_conformance_gate`]), and that no constant is copied from one package into another
/// ([`shared_constants_gate`]).
pub(crate) fn package_checks(runner: &mut dyn Runner) -> Result<(), Failure> {
    let mut manifests = Vec::new();
    for package in PACKAGES {
        manifests.push((
            package,
            read_file(runner, &format!("{}/package.json", package.dir))?,
        ));
    }
    let mut results: Vec<Result<(), Failure>> = manifests
        .iter()
        .map(|(package, manifest)| {
            package_check_gate(
                &format!("{}/package.json", package.dir),
                manifest,
                package.checks,
            )
        })
        .collect();
    let read: Vec<(Package, &str)> = manifests
        .iter()
        .map(|(package, manifest)| (*package, manifest.as_str()))
        .collect();
    results.push(package_order_gate(&read));
    let paths: Vec<String> = manifests
        .iter()
        .map(|(package, _)| format!("{}/package.json", package.dir))
        .collect();
    let named: Vec<(&str, &str)> = paths
        .iter()
        .zip(&manifests)
        .map(|(path, (_, manifest))| (path.as_str(), manifest.as_str()))
        .collect();
    results.push(test_exports_gate(&named));
    results.push(linked_modules(runner, &manifests));
    results.push(shared_constants(runner));
    all_of(results)
}

/// The packages that restate the facts they check, by design, never importing them: the
/// independent oracle, whose worth is to share no code with what it checks (its hash's
/// arithmetic mode, for one, is the engine's restated). The shared constants gate reads none
/// of their sources.
const INDEPENDENT_PACKAGES: [&str; 1] = ["tooling/oracle"];

/// Reads the TypeScript sources of every package, test code apart ([`is_test`]), the
/// independent ones apart ([`INDEPENDENT_PACKAGES`]), then checks that no constant is
/// exported alike by several of them ([`shared_constants_gate`]).
fn shared_constants(runner: &mut dyn Runner) -> Result<(), Failure> {
    let tracked = list_tracked(
        runner,
        &ts_sources(),
        "the TypeScript sources of the packages",
    )?;
    let mut sources = Vec::new();
    for package in PACKAGES
        .iter()
        .filter(|package| !INDEPENDENT_PACKAGES.contains(&package.dir))
    {
        let prefix = format!("{}/", package.dir);
        let paths: Vec<String> = tracked
            .iter()
            .filter(|path| path.strip_prefix(&prefix).is_some_and(|rel| !is_test(rel)))
            .cloned()
            .collect();
        for (path, text) in read_all(runner, paths)? {
            sources.push((package.dir, path, text));
        }
    }
    let read: Vec<(&str, &str, &str)> = sources
        .iter()
        .map(|(dir, path, text)| (*dir, path.as_str(), text.as_str()))
        .collect();
    shared_constants_gate(&read)
}

/// Reads the crate and the tests of every package of `manifests` that has a crate, then
/// checks that each module it ships is held to the conformance runner of every library it
/// links ([`linked_conformance_gate`]).
fn linked_modules(runner: &mut dyn Runner, manifests: &[(Package, String)]) -> Result<(), Failure> {
    let patterns: Vec<&str> = core::iter::once("*Cargo.toml")
        .chain(ts_sources())
        .collect();
    let tracked = list_tracked(
        runner,
        &patterns,
        "the crates and TypeScript files of the packages",
    )?;
    let mut modules = Vec::new();
    for (package, manifest) in manifests {
        let cargo_path = format!("{}/Cargo.toml", package.dir);
        if !tracked.contains(&cargo_path) {
            continue;
        }
        let test_dir = format!("{}/test/", package.dir);
        let cargo = read_file(runner, &cargo_path)?;
        // Its TypeScript files under test/ (the list holds no other file there).
        let test_files = read_all(
            runner,
            tracked
                .iter()
                .filter(|path| path.starts_with(&test_dir))
                .cloned()
                .collect(),
        )?;
        modules.push((package.dir, manifest.as_str(), cargo, test_files));
    }
    let read: Vec<LinkedModule<'_>> = modules
        .iter()
        .map(|(dir, manifest, cargo, tests)| LinkedModule {
            dir,
            manifest,
            cargo: cargo.as_str(),
            tests: tests.clone(),
        })
        .collect();
    linked_conformance_gate(&read)
}

/// Reads what `package` measures and mutates, then checks that every source of it is in
/// both, or reviewed.
pub(crate) fn scope(runner: &mut dyn Runner, package: Package) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &ts_sources(), "the TypeScript sources")?;
    let [coverage, mutation] = package.reports();
    let scope = Scope {
        dir: package.dir,
        stryker: read_file(runner, &format!("{}/{STRYKER_CONFIG}", package.dir))?,
        summary: read_file(runner, &format!("{coverage}/coverage-summary.json"))?,
        mutation: read_file(runner, &format!("{mutation}/mutation.json"))?,
    };
    scope_gate(&tracked, &[scope], &read_file(runner, BYPASSES_REVIEWED)?)
}

/// The mutants disabled by a well-formed comment in the tracked TypeScript sources, which
/// [`ts_mutation_gate`] accepts as ignored.
///
/// # Errors
///
/// Returns why the sources cannot be listed or read.
pub(crate) fn disabled_mutants(runner: &mut dyn Runner) -> Result<Vec<Disable>, Failure> {
    let paths = list_tracked(runner, &stryker_sources(), "the TypeScript files")?;
    let (found, _) = disables(&read_all(runner, paths)?);
    Ok(found.into_iter().map(|(disable, _)| disable).collect())
}

/// The unviable mutants cargo-mutants reported ([`UNVIABLE_REPORT`]) in a run of `extent`.
///
/// cargo-mutants writes no report when the changes hold no mutant, and its report directory
/// was emptied just before it ran: after a partial run, a missing report means none found.
/// A complete run always writes it.
///
/// # Errors
///
/// Returns why the report cannot be read.
pub(crate) fn unviable_report(runner: &mut dyn Runner, extent: Extent) -> Result<String, Failure> {
    match (runner.read(UNVIABLE_REPORT), extent) {
        (Err(error), Extent::Partial) if error.kind() == io::ErrorKind::NotFound => {
            Ok(String::new())
        }
        (read, _) => {
            read.map_err(|error| Failure::from(format!("cannot read {UNVIABLE_REPORT}: {error}")))
        }
    }
}

/// Says what the last mutation run of `workspace` suggests of its slow group
/// ([`order_suggestion`]), from the test times of its unmutated baseline: nothing when the
/// group is as measured, one line when the baseline log or the configuration cannot be
/// read, or when the log gives no test time.
/// Never fails the run: the order changes how long a run takes, never what it proves.
///
/// # Errors
///
/// Returns why it cannot write to `out`.
pub(crate) fn suggest_test_order(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    workspace: CargoWorkspace,
) -> Result<(), Failure> {
    let config = workspace.nextest_config();
    let log = format!("{}/log/baseline.log", workspace.mutants_report());
    let lines = match runner.read(&log) {
        Err(error) => vec![format!(
            "  test order: no suggestion, cannot read {log}: {error}"
        )],
        Ok(baseline) => match runner.read(config) {
            Err(error) => vec![format!(
                "  test order: no suggestion, cannot read {config}: {error}"
            )],
            Ok(text) => slow_group(config, &text).map_or_else(
                |_| {
                    vec![format!(
                        "  test order: no suggestion, {config} holds more than its slow group \
                         (the nextest configuration gate says where)"
                    )]
                },
                |group| {
                    let times = baseline_times(&baseline);
                    if times.is_empty() {
                        // Never said as a group as measured: a change of nextest's output, or
                        // of how it is written, would otherwise silence every suggestion.
                        vec![format!(
                            "  test order: no suggestion, no test time read from {log}, in a \
                             form of nextest's output this reader does not know"
                        )]
                    } else {
                        order_suggestion(config, &group, &times)
                    }
                },
            ),
        },
    };
    lines.iter().try_for_each(|line| say(out, line))
}

/// Runs the cargo-mutants command `cmd` of `workspace`, whose report it writes to that
/// workspace's [`CargoWorkspace::mutants_report`] (the directory of its `missed.txt`), says
/// what its baseline suggests of the order of the tests ([`suggest_test_order`]), then, when
/// it missed mutants and nothing worse
/// ([`MUTANTS_MISSED_EXIT`]), its doc-test pass ([`doc_test_pass_command`]): those mutants
/// tested again by `cargo test`, doc tests included, which nextest does not run. Says which
/// mutants it tests again; they are caught when that pass succeeds and its report lists
/// every one of them caught.
///
/// # Errors
///
/// Returns why `cmd` failed but by missing mutants (a timeout, a failing baseline, a
/// mutant it could not build or test), that failure too when its report names no mutant
/// missed, why that report cannot be read, or why the doc-test pass failed (a mutant still
/// missed, or timed out).
pub(crate) fn mutants_with_doc_tests(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    cmd: &Cmd,
    workspace: CargoWorkspace,
) -> Result<(), Failure> {
    let report = workspace.mutants_report();
    let ran = runner.run(cmd);
    suggest_test_order(runner, out, workspace)?;
    let Err(first) = ran else {
        return Ok(());
    };
    if !matches!(
        first.failed,
        Failed::Ended {
            exit: Exit::Code(MUTANTS_MISSED_EXIT),
            ..
        }
    ) {
        return Err(Failure::from(first.to_string()));
    }
    let listed = read_file(runner, &format!("{report}/missed.txt"))?;
    let missed: Vec<&str> = listed.lines().filter(|line| !line.is_empty()).collect();
    if missed.is_empty() {
        return Err(Failure::from(first.to_string()));
    }
    let count = missed.len();
    let mutants = if count == 1 { "mutant" } else { "mutants" };
    say(
        out,
        &format!(
            "  {count} {mutants} no test caught under nextest, tested again by cargo test, doc \
             tests included:"
        ),
    )?;
    for name in &missed {
        say(out, &format!("    {name}"))?;
    }
    match runner.run(&doc_test_pass_command(cmd, report, &missed)) {
        Ok(()) => {}
        // Missed mutants of its own: cargo-mutants (27.1.0) adds every deletion of a struct
        // literal's field whatever its filters, and the doc tests alone catch few of them.
        // The first run judged those; only the mutants given are judged here, below.
        Err(failure)
            if matches!(
                failure.failed,
                Failed::Ended {
                    exit: Exit::Code(MUTANTS_MISSED_EXIT),
                    ..
                }
            ) => {}
        Err(failure) => return Err(Failure::from(failure.to_string())),
    }
    // Its exit alone proves nothing: cargo-mutants also succeeds when its filters select no
    // mutant, and fails for mutants it was not given. Every mutant it was given must be in
    // its report of those caught.
    let caught = read_file(
        runner,
        &format!("{report}/{DOC_TEST_PASS}/mutants.out/caught.txt"),
    )?;
    let not_caught: Vec<&str> = missed
        .iter()
        .copied()
        .filter(|name| !caught.lines().any(|line| line == *name))
        .collect();
    if !not_caught.is_empty() {
        return Err(Failure::from(format!(
            "the doc-test pass did not catch every mutant it tested again: {}",
            not_caught.join(", ")
        )));
    }
    say(
        out,
        &format!("  caught with the doc tests: {count} {mutants}"),
    )
}

/// Runs every mutant of the GPU workspace ([`gpu_mutants_command`]), then adds the unviable
/// mutants it reported to [`UNVIABLE_REPORT`], the root's run of `extent` (created when a
/// partial run wrote none: [`unviable_report`]), each path from the root
/// ([`GPU_WORKSPACE`] before it), so that the one review gate reads both.
///
/// # Errors
///
/// Returns why the run failed (a mutant missed, even by the doc tests
/// ([`mutants_with_doc_tests`]), or timed out among them), or why a report cannot be read
/// (the root's missing after a complete run among them) or written.
pub(crate) fn gpu_mutants(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    extent: Extent,
) -> Result<(), Failure> {
    mutants_with_doc_tests(runner, out, &gpu_mutants_command(), CargoWorkspace::Gpu)?;
    let report = format!("{GPU_MUTANTS_REPORT}/unviable.txt");
    let gpu = read_file(runner, &report)?;
    let root = unviable_report(runner, extent)?;
    let added = from_the_root(&gpu);
    runner
        .write(UNVIABLE_REPORT, &format!("{root}{added}"))
        .map_err(|error| Failure::from(format!("cannot write {UNVIABLE_REPORT}: {error}")))
}

/// The mutants of `listed`, one a line as cargo-mutants of the GPU workspace writes them
/// (each path from that workspace's root), each path from the repository's root
/// ([`GPU_WORKSPACE`] before it), one a line; blank lines dropped.
pub(crate) fn from_the_root(listed: &str) -> String {
    listed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .flat_map(|line| [GPU_WORKSPACE, "/", line, "\n"])
        .collect()
}

/// Searches the Rust files of the GPU workspace for a mutant a stopped run left in one
/// ([`mutant_search_command`]), and refuses any ([`mutant_left_gate`]).
///
/// # Errors
///
/// Returns why the search failed (grep ending with another code than 1, its "none"), or the
/// gate's refusal.
fn mutant_left(runner: &mut dyn Runner) -> Result<(), Failure> {
    let found = match runner.capture(&mutant_search_command()) {
        Ok(found) => found,
        Err(failure)
            if matches!(
                failure.failed,
                Failed::Ended {
                    exit: Exit::Code(1_i32),
                    ..
                }
            ) =>
        {
            String::new()
        }
        Err(failure) => {
            return Err(Failure::from(format!(
                "cannot search the GPU workspace for a mutant left in place: {failure}"
            )));
        }
    };
    mutant_left_gate(&found)
}

/// Reads the tracked Rust files, the reviews of unviable mutants (now and at the merge base
/// with `main`, the last version's), the corpus's exemptions ([`CORPUS_EXEMPT`]) and the
/// specifications, then checks that every function they name exists
/// ([`unviable_names_gate`], [`review_names_gate`], [`spec_names_gate`]) and that each
/// review added since the last version, and each exemption, names its test
/// ([`unviable_reasons_gate`], [`review_reasons_gate`]).
///
/// # Errors
///
/// Returns why a file cannot be listed or read (the reviews at the merge base among them),
/// or every name a gate refuses.
fn named_functions(runner: &mut dyn Runner) -> Result<(), Failure> {
    let sources = rust_sources(runner)?;
    let reviews = read_file(runner, UNVIABLE_REVIEWED)?;
    let (base, _) = main_base(runner, "the unviable reviews of the last version")?;
    let released = git_output(
        runner,
        &["show", &format!("{base}:{UNVIABLE_REVIEWED}")],
        &format!("read {UNVIABLE_REVIEWED} at the merge base {base}"),
    )?;
    let specs = list_tracked(runner, &["spec/*.md"], "the specifications")?;
    let specs = read_all(runner, specs)?;
    // The corpus's exemptions are in the form of the unviable reviews, and every one of
    // them came with the rule that a reason names its test: none is read as released.
    let exempt = read_file(runner, CORPUS_EXEMPT)?;
    all_of([
        unviable_names_gate(&reviews, &sources),
        unviable_reasons_gate(&reviews, &released, &sources),
        review_names_gate(CORPUS_EXEMPT, &exempt, &sources),
        review_reasons_gate(CORPUS_EXEMPT, &exempt, "", &sources),
        spec_names_gate(&specs, &sources),
    ])
}

/// Reads the tree of the CPU reference in each workspace, and checks them
/// ([`cpu_reference_gate`]).
fn cpu_reference(runner: &mut dyn Runner) -> Result<(), Failure> {
    let mut read = |workspace| {
        runner
            .capture(&cpu_reference_command(workspace))
            .map_err(|failure| {
                Failure::from(format!("cannot read the CPU reference's tree: {failure}"))
            })
    };
    let root = read(CargoWorkspace::Root)?;
    cpu_reference_gate(&root, &read(CargoWorkspace::Gpu)?)
}

/// The tracked Rust files, each read into its tokens ([`RustSource::parse`]).
///
/// # Errors
///
/// Returns why they cannot be listed or read, or the first that does not lex.
pub(crate) fn rust_sources(runner: &mut dyn Runner) -> Result<Vec<RustSource>, Failure> {
    let paths = list_tracked(runner, &["*.rs"], "the Rust files")?;
    parse_all(read_all(runner, paths)?)
}

/// Each of `sources` (path and text) read into its tokens ([`RustSource::parse`]).
///
/// # Errors
///
/// Returns the first that does not lex.
pub(crate) fn parse_all(sources: Vec<(String, String)>) -> Result<Vec<RustSource>, Failure> {
    sources
        .into_iter()
        .map(|(path, text)| RustSource::parse(&path, &text))
        .collect()
}

/// Lists every mutant of the workspace and checks the reviews against them
/// ([`unviable_reviews_gate`]).
pub(crate) fn unviable_reviews(runner: &mut dyn Runner) -> Result<(), Failure> {
    let root = runner
        .capture(&mutants_list_command())
        .map_err(|failure| Failure::from(format!("cannot list the Rust mutants: {failure}")))?;
    let gpu = runner
        .capture(&gpu_mutants_list_command())
        .map_err(|failure| {
            Failure::from(format!(
                "cannot list the Rust mutants of the GPU workspace: {failure}"
            ))
        })?;
    // The GPU workspace's mutants, each path from the root, as their reviews name them.
    let mut listed: String = root.lines().flat_map(|line| [line, "\n"]).collect();
    listed.push_str(&from_the_root(&gpu));
    unviable_reviews_gate(&listed, &read_file(runner, UNVIABLE_REVIEWED)?)
}

/// Reads the unviable mutants of every shard (`reports`) and checks them together, as after
/// a complete run.
pub(crate) fn shards_unviable(runner: &mut dyn Runner, reports: &[String]) -> Result<(), Failure> {
    let mut report = String::new();
    for path in reports {
        report.push_str(&read_file(runner, path)?);
        report.push('\n');
    }
    unviable_gate(
        &report,
        Extent::Complete,
        &read_file(runner, UNVIABLE_REVIEWED)?,
    )
}

/// Checks `parts` against the full check ([`parts_gate`]); the number of cargo-mutants
/// jobs changes neither.
pub(crate) fn parts_of_the_full_check(parts: Vec<Part>) -> Result<(), Failure> {
    let jobs = NonZeroUsize::MIN;
    let parts: Vec<(String, Vec<Step>)> = parts
        .into_iter()
        .map(|part| (part.name(), part_steps(part, jobs)))
        .collect();
    parts_gate(
        &check_steps(Mode::Full { jobs }),
        &parts,
        &preparation_steps(),
    )
}

/// The crates of every workspace and their direct dependencies ([`metadata_command`] of
/// each, [`merged_metadata`]).
pub(crate) fn crates_metadata(runner: &mut dyn Runner) -> Result<String, Failure> {
    let mut capture = |workspace| {
        runner
            .capture(&metadata_command(workspace))
            .map_err(|failure| Failure::from(format!("cannot list the crates: {failure}")))
    };
    let root = capture(CargoWorkspace::Root)?;
    let others = CargoWorkspace::ALL
        .into_iter()
        .filter(|&workspace| workspace != CargoWorkspace::Root)
        .map(capture)
        .collect::<Result<Vec<_>, _>>()?;
    merged_metadata(&root, &others)
}

/// Reads what the dependency gate checks, and runs it ([`dependency_gate`]).
pub(crate) fn dependencies(runner: &mut dyn Runner) -> Result<(), Failure> {
    let metadata = crates_metadata(runner)?;
    let packages = read_all(runner, package_jsons())?;
    dependency_gate(
        &metadata,
        &packages,
        &read_file(runner, DEPENDENCIES_REVIEWED)?,
    )
}

/// Reads every workspace's resolved metadata and the register, and runs the build-time
/// code gate ([`build_time_gate`]) and the gate tying the versions both lock files share
/// ([`shared_versions_gate`]).
pub(crate) fn build_time(runner: &mut dyn Runner) -> Result<(), Failure> {
    let [root, gpu] = CargoWorkspace::ALL.map(|workspace| {
        runner
            .capture(&resolved_metadata_command(workspace))
            .map_err(|failure| {
                Failure::from(format!("cannot list the crates the builds use: {failure}"))
            })
    });
    let (root, gpu) = (root?, gpu?);
    all_of([
        build_time_gate(
            &[root.clone(), gpu.clone()],
            &read_file(runner, BUILD_TIME_REVIEWED)?,
        ),
        shared_versions_gate(&root, &gpu),
    ])
}

/// Reads what the pin gate checks, and runs it ([`pin_gate`]).
pub(crate) fn pins(runner: &mut dyn Runner) -> Result<(), Failure> {
    let metadata = crates_metadata(runner)?;
    pin_gate(&metadata, &read_file(runner, DEPENDENCIES_REVIEWED)?)
}

/// Reads what the part license gate checks, and runs it ([`part_license_gate`]).
pub(crate) fn part_licenses(runner: &mut dyn Runner) -> Result<(), Failure> {
    let metadata = crates_metadata(runner)?;
    let packages = read_all(runner, package_jsons())?;
    let files = LICENSE_FILES
        .iter()
        .map(|(file, _)| *file)
        .chain(LICENSE_STATEMENTS)
        .map(str::to_owned)
        .collect();
    part_license_gate(&metadata, &packages, &read_all(runner, files)?)
}

/// Reads the tracked files a sentence can be written in ([`citation_sources`]), and runs
/// the citation gate ([`citation_gate`]) on them.
pub(crate) fn citations(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = list_tracked(runner, &citation_sources(), "the text files")?;
    citation_gate(&read_all(runner, paths)?)
}

/// Reads the tracked files of [`vocabulary_sources`] and the vocabulary, and runs the
/// vocabulary gate ([`vocabulary_gate`]) on them.
fn vocabulary(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = list_tracked(runner, &vocabulary_sources(), "the prose files")?;
    let sources = read_all(runner, paths)?;
    vocabulary_gate(&sources, &read_file(runner, VOCABULARY)?)
}

/// Reads the tracked files a sentence can be written in ([`citation_sources`]), and runs
/// the list counts gate ([`list_counts_gate`]) on them.
fn list_counts(runner: &mut dyn Runner) -> Result<(), Failure> {
    let paths = list_tracked(runner, &citation_sources(), "the text files")?;
    list_counts_gate(&read_all(runner, paths)?)
}

/// Runs the fused test build ([`fused_command`]) and checks its option was in force.
pub(crate) fn fused_build(runner: &mut dyn Runner) -> Result<(), Failure> {
    let output = runner
        .capture(&fused_command())
        .map_err(|failure| Failure::from(format!("the fused test build failed: {failure}")))?;
    fused_gate(&output)
}

/// Runs the doc tests of [`DOC_CODES_FIXTURE`], which must fail for the error code their
/// `compile_fail` names ([`UNCHECKED_CODE`]), then those of the workspace, the codes checked
/// alike ([`doc_tests_command`]).
///
/// When the fixture does not fail as it must, the failure carries what rustdoc wrote, its
/// output then its error output, one line each: the cause (`error[E…]`, the codes it
/// found) is in its output, cargo's error line only says that a doc test failed.
pub(crate) fn doc_tests(runner: &mut dyn Runner) -> Result<(), Failure> {
    let fixture = runner.run_for_stdout(&doc_codes_fixture_command());
    let said = |text: &str| text.contains(UNCHECKED_CODE);
    let summary = match fixture.ended {
        Err(_) if said(&fixture.stdout) || said(&fixture.stderr) => None,
        Err(failure) => Some(format!(
            "the doc tests of {DOC_CODES_FIXTURE} failed for another reason than its error \
             code: {failure}"
        )),
        Ok(()) => Some(format!(
            "the doc tests of {DOC_CODES_FIXTURE} passed: rustdoc did not check the error code \
             of a compile_fail doc test"
        )),
    };
    if let Some(summary) = summary {
        return Err(Failure {
            summary,
            details: fixture
                .stdout
                .lines()
                .chain(fixture.stderr.lines())
                .map(str::to_owned)
                .collect(),
            usage: false,
        });
    }
    CargoWorkspace::ALL.into_iter().try_for_each(|workspace| {
        runner
            .run(&doc_tests_command(workspace))
            .map_err(|failure| Failure::from(failure.to_string()))
    })
}

/// Reads what the math layer gate checks, and runs it ([`math_layer_gate`]), after checking
/// that every file it names is among them ([`named_sources_gate`]).
pub(crate) fn math_layer(runner: &mut dyn Runner) -> Result<(), Failure> {
    let metadata = crates_metadata(runner)?;
    let paths = list_tracked(runner, &MATH_LAYER_SOURCES, "the box and engine sources")?;
    let sources = parse_all(read_all(runner, paths)?)?;
    // A precondition: the math layer gate reads every file it names among these.
    named_sources_gate(&sources)?;
    math_layer_gate(&metadata, &sources)
}

/// Reads the coverage and mutation reports of `package`, then checks both: 100% coverage,
/// and every mutant killed after a complete run.
fn typescript_reports(runner: &mut dyn Runner, package: Package) -> Result<(), Failure> {
    let disabled = disabled_mutants(runner)?;
    let [coverage, mutation] = package.reports();
    let summary = read_file(runner, &format!("{coverage}/coverage-summary.json"))?;
    let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
    all_of([
        ts_coverage_gate(package.dir, &summary).map_err(Failure::from),
        ts_mutation_gate(package.dir, &report, &disabled, Extent::Complete),
    ])
}

/// Reads what the supply-chain gate checks, and runs it ([`supply_chain_gate`]).
pub(crate) fn supply_chain(runner: &mut dyn Runner) -> Result<(), Failure> {
    let [deny, dependabot, pnpm, package, gpu_deny] =
        SUPPLY_CHAIN_FILES.map(|path| read_file(runner, path));
    supply_chain_gate(
        &[deny?, dependabot?, pnpm?, package?, gpu_deny?],
        &list_tracked(
            runner,
            &PNPM_SETTINGS_FILES,
            "the npm configuration and pnpm hooks files",
        )?,
    )
}

/// Reads the Cargo configuration, the manifests the GPU workspace shares settings with and
/// the nextest configurations, and runs their gates ([`cargo_config_gate`],
/// [`gpu_manifest_gate`], [`nextest_configs_gate`]).
pub(crate) fn cargo_configs(runner: &mut dyn Runner) -> Result<(), Failure> {
    cargo_config_gate(&read_file(runner, CARGO_CONFIG_FILE)?)?;
    let manifests = read_file(runner, CARGO_TOML)
        .and_then(|root| gpu_manifest_gate(&root, &read_file(runner, GPU_MANIFEST)?));
    let nextest = nextest_configs(runner);
    all_of([manifests, nextest])
}

/// Lists the tracked files named like a nextest configuration, reads each workspace's, and
/// runs [`nextest_configs_gate`] on them.
fn nextest_configs(runner: &mut dyn Runner) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &["*nextest.toml"], "the nextest configurations")?;
    let configs = read_all(
        runner,
        CargoWorkspace::ALL
            .iter()
            .map(|workspace| workspace.nextest_config().to_owned())
            .collect(),
    )?;
    nextest_configs_gate(&configs, &tracked)
}

/// Runs one action of a step, saying on `out` what it found when that matters.
pub(crate) fn run_action(
    action: &Action,
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    tree: &mut Option<TreeState>,
) -> Result<(), Failure> {
    match action {
        Action::TreeRecorded => {
            *tree = Some(tree_state(runner)?);
            Ok(())
        }
        Action::TreeUnchanged => tree_gate(tree.as_ref(), &tree_state(runner)?),
        Action::Run(cmd) | Action::ToolGate(cmd) => runner
            .run(cmd)
            .map_err(|failure| Failure::from(failure.to_string())),
        Action::Coverage(coverage) => coverage_gate(&measure(runner, *coverage)?, *coverage),
        Action::CoverageLocality => coverage_locality(runner),
        Action::VersionGate => versions(runner),
        Action::FreshDirs(paths) => paths.iter().try_for_each(|path| fresh(runner, path)),
        Action::MembershipGate => membership(runner),
        Action::ReadmeGate => readmes(runner),
        Action::CitationGate => citations(runner),
        Action::PinGate => pins(runner),
        Action::LibraryGate => crates_metadata(runner)
            .and_then(|metadata| all_of([library_gate(&metadata), shared_tests_gate(&metadata)])),
        Action::PartLicenseGate => part_licenses(runner),
        Action::ConflictMarkerGate => conflict_markers(runner),
        Action::SourceKindsGate => source_kinds(runner),
        Action::ListCountsGate => list_counts(runner),
        Action::VocabularyGate => vocabulary(runner),
        Action::RetiredPatternsGate => retired_patterns(runner),
        Action::RunnableHintsGate => runnable_hints(runner),
        Action::HermeticGitGate => hermetic_git(runner),
        Action::RegisterNamesGate => register_names(runner),
        Action::Preflight(driver) => preflight(runner, *driver),
        Action::MutantLeftGate => mutant_left(runner),
        Action::HistoryGate => history(runner),
        Action::WorkflowGate => workflows(runner),
        Action::UnsafeGate => rust_sources(runner).and_then(|sources| unsafe_gate(&sources)),
        Action::LibraryLintsGate => rust_sources(runner).and_then(|sources| {
            all_of([
                library_lints_gate(&sources),
                no_std_portability_gate(&sources),
            ])
        }),
        Action::TestCodeGate => rust_sources(runner).and_then(|sources| test_code_gate(&sources)),
        Action::TestDataGate => rust_sources(runner).and_then(|sources| test_data_gate(&sources)),
        Action::UntestedTargetsGate => {
            let metadata = crates_metadata(runner)?;
            let sources = rust_sources(runner)?;
            all_of([
                untested_targets_gate(&metadata, &sources),
                compile_fail_codes_gate(&sources),
            ])
        }
        Action::LicenseGate => licenses(runner),
        Action::BuildTimeGate => build_time(runner),
        Action::CargoConfigGate => cargo_configs(runner),
        Action::FloatMethodGate => float_methods(runner),
        Action::SupplyChainGate => supply_chain(runner),
        Action::CpuReferenceGate => cpu_reference(runner),
        Action::FusedBuild => fused_build(runner),
        Action::DocTests => doc_tests(runner),
        Action::LoadedFilesGate => loaded_files_of_the_workspace(runner),
        Action::UnviableNamesGate => named_functions(runner),
        Action::MathLayerGate => math_layer(runner),
        Action::DependencyGate => dependencies(runner),
        Action::TypeScriptReportsGate(package) => typescript_reports(runner, *package),
        Action::TypeScriptMutationGate { package, extent } => {
            let disabled = disabled_mutants(runner)?;
            let [_, mutation] = package.reports();
            let report = read_file(runner, &format!("{mutation}/mutation.json"))?;
            ts_mutation_gate(package.dir, &report, &disabled, *extent)
        }
        Action::TypeScriptScopeGate(package) => scope(runner, *package),
        Action::StrykerConfigGate => stryker_configs(runner),
        Action::PackageCheckGate => package_checks(runner),
        Action::BypassGate => {
            let paths = list_tracked(runner, &bypass_sources(), "the source files")?;
            bypass_gate(
                &read_all(runner, paths)?,
                &read_file(runner, BYPASSES_REVIEWED)?,
            )
        }
        Action::EquivalentGate => equivalents(runner),
        Action::RustMutants(cmd) => mutants_with_doc_tests(runner, out, cmd, CargoWorkspace::Root),
        Action::GpuMutants(extent) => gpu_mutants(runner, out, *extent),
        Action::CorpusMutants(mode) => corpus_mutants(runner, out, *mode),
        Action::RustMutantsOfChanges { jobs } => rust_mutants_of_changes(runner, out, *jobs),
        Action::RustMutantsOfChangedLines { jobs, since } => {
            rust_mutants_of_changed_lines(runner, out, *jobs, since)
        }
        Action::MutationProofRecorded => record_mutation_proof(runner, out, tree.as_ref()),
        Action::Conformance => conformance(runner, out),
        Action::Bless(options) => bless(runner, out, options),
        Action::UnviableGate(extent) => {
            let report = unviable_report(runner, *extent)?;
            unviable_gate(&report, *extent, &read_file(runner, UNVIABLE_REVIEWED)?)
        }
        Action::UnviableReviewsCurrent => unviable_reviews(runner),
        Action::ShardsUnviableGate(reports) => shards_unviable(runner, reports),
        Action::PartsGate => parts_of_the_full_check(full_parts()),
    }
}

/// Empties a directory through `runner`, naming it when it cannot.
pub(crate) fn fresh(runner: &mut dyn Runner, path: &str) -> Result<(), Failure> {
    runner
        .fresh_dir(path)
        .map_err(|error| Failure::from(format!("cannot empty {path}: {error}")))
}

/// Writes one line to stdout, naming stdout when it cannot.
pub(crate) fn say(out: &mut dyn Write, line: &str) -> Result<(), Failure> {
    writeln!(out, "{line}")
        .map_err(|error| Failure::from(format!("cannot write to stdout: {error}")))
}

/// Runs `steps` in order and stops at the first failure: [`run_steps_then`] with
/// [`OnFailure::Stop`].
///
/// # Errors
///
/// As [`run_steps_then`].
pub fn run_steps(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    run_steps_then(steps, runner, out, OnFailure::Stop)
}

/// A duration as the progress of the checks shows it: tenths of a second below a minute
/// (`12.3 s`, cut, never rounded up to the next unit), then whole seconds (`4 min 05 s`,
/// `1 h 02 min 03 s`).
#[must_use]
pub fn duration_text(duration: Duration) -> String {
    let tenths = duration.as_millis() / 100;
    let seconds = duration.as_secs();
    if tenths < 600 {
        format!("{}.{} s", tenths / 10, tenths % 10)
    } else if seconds < 3_600 {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    } else {
        format!(
            "{} h {:02} min {:02} s",
            seconds / 3_600,
            seconds % 3_600 / 60,
            seconds % 60
        )
    }
}

/// Runs `steps` in order; at a failure, does what `on_failure` says.
///
/// Each step is announced (`▶ [n/total] name`), then, once it ends, its elapsed time is
/// said (`done in …` or `failed after …`, [`duration_text`] of [`Runner::clock`]), and the
/// run ends with its total time: `✔ all N checks passed in …`, `✘ stopped after …, n of N
/// checks run` ([`OnFailure::Stop`]) or `✘ k of N checks failed in …`
/// ([`OnFailure::RunTheRest`]).
///
/// # Errors
///
/// With [`OnFailure::Stop`], the failure of the first step that failed, its summary naming
/// the step. With [`OnFailure::RunTheRest`], once every step has run, a summary counting
/// the failed steps, and in the details, for each failed step in order, its own details
/// then its summary naming the step. In both modes, why the progress could not be written.
pub fn run_steps_then(
    steps: &[Step],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    on_failure: OnFailure,
) -> Result<(), Failure> {
    let total_steps = steps.len();
    let mut tree = None;
    let mut failures = Vec::new();
    let started = runner.clock();
    let mut step_started = started;
    for (index, step) in steps.iter().enumerate() {
        let number = index.saturating_add(1);
        say(out, &format!("▶ [{number}/{total_steps}] {}", step.name))?;
        let outcome = run_action(&step.action, runner, out, &mut tree);
        let ended = runner.clock();
        let took = duration_text(ended.saturating_sub(step_started));
        step_started = ended;
        let Err(failure) = outcome else {
            say(out, &format!("  done in {took}"))?;
            continue;
        };
        say(out, &format!("  failed after {took}"))?;
        let failure = Failure {
            summary: format!("{}: {}", step.name, failure.summary),
            ..failure
        };
        if on_failure == OnFailure::Stop {
            let total = duration_text(step_started.saturating_sub(started));
            say(
                out,
                &format!("✘ stopped after {total}, {number} of {total_steps} checks run"),
            )?;
            return Err(failure);
        }
        failures.push(failure);
    }
    let total = duration_text(step_started.saturating_sub(started));
    if failures.is_empty() {
        return say(
            out,
            &format!("✔ all {total_steps} checks passed in {total}"),
        );
    }
    say(
        out,
        &format!(
            "✘ {} of {total_steps} checks failed in {total}",
            failures.len()
        ),
    )?;
    Err(Failure {
        summary: format!(
            "{} of {total_steps} checks failed, listed above",
            failures.len()
        ),
        details: listed_above(failures),
        usage: false,
    })
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
