//! Project automation for Poyetik.
//!
//! `cargo xtask check` runs every quality check of the repository, in order, and stops
//! at the first failure, mutation testing only the changes ([`Mode::Changes`]); the
//! pre-push hook runs it. `cargo xtask check --full` runs every mutant ([`Mode::Full`]);
//! CI runs it. A green default check proves less than a green full check: see
//! [`Mode`]. The measured choices behind the check's speed are recorded, dated, in the
//! crate's `README.md`.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod actions;
mod changes;
mod citations;
mod cli;
mod commits;
mod conflicts;
mod corpus;
mod coverage;
mod dependencies;
mod float;
mod hints;
mod history;
mod licenses;
mod loaded_files;
mod math_layer;
mod parts;
mod preflight;
mod proof_record;
mod release;
mod retired;
mod reviews;
mod runner;
mod rust_source;
mod steps;
mod supply_chain;
mod test_code;
mod test_order;
mod unsafety;
mod vocabulary;
mod workflow;
mod workspace;

pub use actions::{duration_text, readme_gate, run_steps, run_steps_then};
pub use changes::{LOCK_FILE, MUTATE_EVERYTHING_WHEN, WORKSPACE_DEPENDENCIES, new_file_diff};
pub use citations::{CITATION_SOURCES, ROADMAP_BOARD, citation_gate};
pub use cli::{
    BestEffort, CONFORMANCE_RECORD, Revision, SPENT_OPTION, SPENT_RECORD, Target, USAGE,
    bless_benchmarks_steps, bless_command, bless_step, cli, conformance_command, file_at,
    mutants_steps, mutation_target,
};
pub use commits::{
    CI_MARKER, CI_MARKER_EXEMPT, CI_PARTS_MARKER, COMMIT_HEADER_MAX, COMMIT_SCOPE_EXTRAS,
    COMMIT_TYPES, SCOPES_SINCE, ci_marker, ci_outputs, ci_parts, ci_parts_marker, commit_msg_gate,
    commit_scopes, matrix_parts, named_parts,
};
pub use conflicts::{GIT_ATTRIBUTES, MARKER_SIZE, binary_suffixes, conflict_marker_gate};
pub use corpus::{
    CORPUS_EXEMPT, CORPUS_INPUTS, CORPUS_MISSED, CORPUS_MUTANTS_OUT, CORPUS_MUTATED, CORPUS_TEST,
    CORPUS_TEST_TIMEOUT, corpus_baseline_command, corpus_gate, corpus_mutants_command,
    corpus_reached, corpus_steps,
};
pub use coverage::{TreeState, coverage_gate, tree_gate, ts_coverage_gate};
pub use dependencies::{
    BIT_DEFINING, BUILD_TIME_REVIEWED, CARGO_CONFIG, DEPENDENCIES_REVIEWED, NPM_FIELDS,
    build_time_gate, dependency_gate, library_gate, merged_metadata, metadata_command, pin_gate,
    resolved_metadata_command, shared_tests_gate, shared_versions_gate,
};
pub use float::{
    CLIPPY_TOML, DETERMINISTIC_FLOAT_METHODS, FLOAT_ITEM_MACROS, FLOAT_SOURCES, FloatMethod,
    Release, audit_float_macro, float_gate, parse_float_methods, parse_release,
};
pub use hints::{HINT_OPENING, RUNNABLE_PREFIXES, runnable_hints_gate};
pub use history::{
    BREAKING_ENTRY, BREAKING_SINCE, CHANGELOG_SINCE, breaking_bang_gate, breaking_log_command,
    changes_behavior, history_commands, history_gate, history_pins, pins_command, pins_gate,
    pre_push_gate,
};
pub use licenses::{
    APPS, APPS_LICENSE, EXPORTS_NOTICES, LICENSE_CONTROLS, LICENSE_FILES, LICENSE_STATEMENTS,
    OPEN_LICENSE, SHIPPED_LICENSES, TOOLING_EXCEPTIONS, TOOLING_EXTRA_LICENSES, license_gate,
    license_of, part_license_gate,
};
pub use loaded_files::{Loaded, LoadedCommand, loaded_files_commands, loaded_files_gate};
pub use math_layer::{
    BOX_CODE, BOX_FLOAT_FORBID, BOX_FORBIDDEN_NAMES, BOX_ROOT, ENGINE_ARITHMETIC_FORBID,
    ENGINE_CODE, ENGINE_FORBIDDEN_NAMES, ENGINE_ROOT, MATH_LAYER_FILE, MATH_LAYER_NAMED,
    MATH_LAYER_SOURCES, REDUCTION_NAMES, VISUAL_BOX_ROOT, math_layer_gate, named_sources_gate,
};
pub use parts::{
    MUTATION_SHARDS, Part, SHARD_REPORTS, Setup, Shard, UNVIABLE_ARTIFACT, full_parts, part_steps,
    parts_gate, preparation_steps, shard_unviable_report,
};
pub use preflight::{
    CARGO_CONFIG_FILE, Environment, GPU_MANIFEST, GpuDriver, Install, NODE_FLOOR, Need,
    PINNED_TOOLS, REFUSED_RUNTIME_VARIABLES, REFUSED_VARIABLE_PREFIXES, REFUSED_VARIABLES,
    RUST_TOOLCHAIN_FILE, SHARED_MANIFEST_TABLES, TOOLS, Tool, cargo_config_candidates,
    cargo_config_gate, gpu_manifest_gate, node_release, preflight_gate, refused_variable,
    toolchain_channel,
};
pub use proof_record::{
    LINKER_DRIVER, MEASURED_BASE, PROOF_RECORD_FORMAT, PROOF_RECORD_RULES, PROOF_RECORD_TAG,
    PROOF_RECORDS, ProofRecord, TreeView, filtered, hidden, unclean,
};
pub use release::{
    CARGO_TOML, CHANGE_KINDS, CHANGELOG, SAME_NAMES, SAME_OPENING, SAME_RUN, Version,
    breaking_names_gate, breaking_spellings_gate, duplicate_entries_gate, release_gate,
    released_sections_gate, removed_names_gate, unreleased_shape_gate,
};
pub use retired::{RETIRED_EXEMPT, RETIRED_PATTERNS, RETIRED_TESTS, retired_patterns_gate};
pub use reviews::{
    BYPASS_SOURCES, BYPASSES_REVIEWED, CONFORMANCE_RUNNERS, Disable, LinkedModule, POLICY_FIXTURES,
    SCOPE_REVIEWS, STRYKER_CONFIG, STRYKER_KEYS, STRYKER_SCRIPT, Scope, TS_SOURCES, bypass_gate,
    check_script, equivalent_gate, linked_conformance_gate, membership_gate, mutants_list_command,
    package_check_gate, package_order_gate, scope_gate, shared_constants_gate, stryker_config_gate,
    stryker_scripts_gate, test_exports_gate, ts_mutation_gate, unviable_gate,
    unviable_reviews_gate,
};
pub use runner::{
    CommandFailure, Exit, Failed, HERMETIC_GIT, LOCAL_GIT_VARIABLES, NEUTRAL_GIT_CONFIG,
    PATHSPEC_GIT_VARIABLES, Ran, Runner, SystemRunner, error_reason,
};
pub use rust_source::RustSource;
pub use steps::{
    Action, BOOTSTRAP_VARIABLE, Cmd, DOC_CODES_FIXTURE, DOC_TESTS_TARGET, Dir, FUSED_CONFIG,
    FUSED_PROOF, FUSED_TARGET, Failure, INCLUDE_IGNORED, MAX_MESSAGE_CHARS, MUTANT_BUILD_CONFIG,
    MUTANT_FAIL_FAST, MUTANT_MEMORY_LIMIT, MUTANT_NO_TESTS, MUTANT_TEST_RUNNER, MUTANT_TEST_TOOL,
    MUTANT_TIMEOUT_FLOOR, MUTANT_TIMEOUT_MARGIN, RUNNING_SUBCOMMANDS, Step, UNCHECKED_CODE,
    VISUAL_CONFORMANCE, bootstrapped, changes_list_command, changes_mutants_command, check_steps,
    doc_codes_fixture_command, doc_tests_command, fused_command, fused_gate, gates_steps,
    gpu_mutants_command, gpu_mutants_list_command, miri_command, mutant_timeout_args, preflighted,
    runs_gpu_command, safe_message, stryker_command, unsafe_crates, workspace_mutants_command,
};
pub use supply_chain::{
    AUDIT_CONFIG, CPU_REFERENCE, DENY_CONFIG, DENY_TOML, DEPENDABOT, DEPENDABOT_CONFIG,
    GPU_DENY_EXCEPTIONS, GPU_DENY_HEADER, GPU_DENY_TOML, MINIMUM_RELEASE_AGE, PNPM_SETTINGS_FILES,
    PNPM_WORKSPACE, PNPM_WORKSPACE_KEYS, ROOT_PACKAGE_JSON, SUPPLY_CHAIN_FILES,
    cpu_reference_command, cpu_reference_gate, gpu_deny_config, supply_chain_gate,
};
pub use test_code::{
    DOC_TESTED_KINDS, NOT_TEST_ATTRIBUTES, TARGET_KINDS, compile_fail_codes_gate,
    review_names_gate, review_reasons_gate, spec_names_gate, test_code_gate, test_data_gate,
    untested_targets_gate, unviable_names_gate, unviable_reasons_gate,
};
pub use test_order::{
    FAST_TEST_MILLIS, OVERRIDE_HEADER, SLOW_PRIORITY_LINE, SLOW_TEST_MILLIS,
    SLOW_TEST_SUGGESTED_MILLIS, TestName, baseline_times, nextest_configs_gate, order_suggestion,
    slow_group,
};
pub use unsafety::{
    ASSEMBLY_CALL, ASSEMBLY_FILES, ASSEMBLY_REFUSED_WORDS, DOC_TESTS_FORBID_UNSAFE_CODE,
    FORBID_UNSAFE_CODE, ONE_UNSAFE_OPERATION, REFUSED_ASSEMBLY, TEST_DECLARATION_FORM,
    TestDeclaration, UNSAFE_CODE_KEPT, UNSAFE_FILES, test_declarations, unsafe_gate,
};
pub use vocabulary::{VOCABULARY, VOCABULARY_SOURCES, vocabulary_gate};
pub use workflow::{
    AUDIO_SINK, JOB_TIMEOUTS, MIRI_TIMEOUT, NIGHTLY_COMPONENTS, NODE_VERSION_FILE,
    PLAYWRIGHT_OUTPUTS, RUNNER, SOFTWARE_GPU, WASM_MODULES, WORKFLOW, WORKFLOW_CONDITIONS,
    WORKFLOW_PERMISSIONS, WORKFLOW_TRIGGERS, failure_uploads, workflow_concurrency, workflow_gate,
    workflow_text,
};
pub use workspace::{
    CHANGES_DIFF, CODE_SOURCES, CONFORMANCE_EXPORT, COVERAGE_DIR, COVERAGE_LOCALITY,
    CargoWorkspace, Coverage, CrateRole, EQUIVALENT_REVIEWED, Extent, FIXTURES, GPU_MUTANTS_OUT,
    GPU_MUTANTS_REPORT, GPU_WORKSPACE, LIBRARIES, LIBRARY_LINTS, Library, MAIN_BASES, MUTANTS_OUT,
    MUTATION_BASES, Measured, Mode, NATIVE_PACKAGES, NATIVE_TARGETS, NIGHTLY_TOOLCHAIN,
    NO_STD_NOT_PORTABLE, OTHER_SOURCES, OnFailure, PACKAGES, PORTABILITY_PACKAGES,
    PORTABILITY_TARGETS, Package, RUST_CRATES, RustCrate, STRYKER_INCREMENTAL, UNVIABLE_REPORT,
    UNVIABLE_REVIEWED, WASM_PACKAGES, WASM_TARGET, build_outputs, crates_in, library_lints_gate,
    mutation_jobs, needed_builds, no_std_portability_gate, package_jsons, source_kinds_gate,
    typescript_reports,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
