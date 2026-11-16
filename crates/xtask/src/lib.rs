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
mod coverage;
mod dependencies;
mod float;
mod history;
mod licenses;
mod loaded_files;
mod math_layer;
mod parts;
mod preflight;
mod proof_record;
mod release;
mod reviews;
mod runner;
mod rust_source;
mod steps;
mod supply_chain;
mod test_code;
mod unsafety;
mod vocabulary;
mod workflow;
mod workspace;

pub use actions::{duration_text, readme_gate, run_steps, run_steps_then};
pub use changes::{LOCK_FILE, MUTATE_EVERYTHING_WHEN, WORKSPACE_DEPENDENCIES, new_file_diff};
pub use citations::{CITATION_SOURCES, ROADMAP_BOARD, citation_gate};
pub use cli::{
    BestEffort, CONFORMANCE_RECORD, SPENT_OPTION, SPENT_RECORD, Target, USAGE,
    bless_benchmarks_steps, bless_command, bless_step, cli, conformance_command, file_at,
    mutants_steps, mutation_target,
};
pub use commits::{
    CI_MARKER, CI_MARKER_EXEMPT, CI_PARTS_MARKER, COMMIT_HEADER_MAX, COMMIT_SCOPE_EXTRAS,
    COMMIT_TYPES, SCOPES_SINCE, ci_marker, ci_outputs, ci_parts, ci_parts_marker, commit_msg_gate,
    commit_scopes, matrix_parts, named_parts,
};
pub use conflicts::{GIT_ATTRIBUTES, MARKER_SIZE, binary_suffixes, conflict_marker_gate};
pub use coverage::{TreeState, coverage_gate, tree_gate, ts_coverage_gate};
pub use dependencies::{
    BIT_DEFINING, CARGO_CONFIG, DEPENDENCIES_REVIEWED, NPM_FIELDS, dependency_gate, library_gate,
    metadata_command, pin_gate, shared_tests_gate,
};
pub use float::{
    CLIPPY_TOML, DETERMINISTIC_FLOAT_METHODS, FLOAT_ITEM_MACROS, FLOAT_SOURCES, FloatMethod,
    Release, audit_float_macro, float_gate, parse_float_methods, parse_release,
};
pub use history::{
    BREAKING_ENTRY, BREAKING_SINCE, CHANGELOG_SINCE, breaking_bang_gate, breaking_log_command,
    changes_behavior, history_commands, history_gate, history_pins, pins_command, pins_gate,
    pre_push_gate,
};
pub use licenses::{
    APPS, APPS_LICENSE, LICENSE_CONTROLS, LICENSE_FILES, OPEN_LICENSE, SHIPPED_LICENSES,
    TOOLING_EXCEPTIONS, TOOLING_EXTRA_LICENSES, license_gate, license_of, part_license_gate,
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
    CARGO_CONFIG_FILE, Environment, Install, NODE_FLOOR, Need, PINNED_TOOLS,
    REFUSED_VARIABLE_PREFIXES, REFUSED_VARIABLES, RUST_TOOLCHAIN_FILE, TOOLS, Tool,
    cargo_config_candidates, cargo_config_gate, node_release, preflight_gate, refused_variable,
    toolchain_channel,
};
pub use proof_record::{
    LINKER_DRIVER, MEASURED_BASE, PROOF_RECORD_FORMAT, PROOF_RECORD_RULES, PROOF_RECORD_TAG,
    PROOF_RECORDS, ProofRecord, TreeView, filtered, hidden, unclean,
};
pub use release::{
    CARGO_TOML, CHANGELOG, SAME_NAMES, SAME_OPENING, Version, breaking_names_gate,
    duplicate_entries_gate, release_gate, released_sections_gate,
};
pub use reviews::{
    BYPASS_SOURCES, BYPASSES_REVIEWED, Disable, POLICY_FIXTURES, SCOPE_REVIEWS, STRYKER_CONFIG,
    STRYKER_KEYS, STRYKER_SCRIPT, Scope, TS_SOURCES, bypass_gate, check_script, equivalent_gate,
    membership_gate, mutants_list_command, package_check_gate, scope_gate, stryker_config_gate,
    stryker_scripts_gate, ts_mutation_gate, unviable_gate, unviable_reviews_gate,
};
pub use runner::{
    CommandFailure, Exit, Failed, HERMETIC_GIT, LOCAL_GIT_VARIABLES, PATHSPEC_GIT_VARIABLES, Ran,
    Runner, SystemRunner, error_reason,
};
pub use rust_source::RustSource;
pub use steps::{
    Action, Cmd, Dir, FUSED_CONFIG, FUSED_PROOF, FUSED_TARGET, Failure, INCLUDE_IGNORED,
    MAX_MESSAGE_CHARS, MUTANT_BUILD_CONFIG, MUTANT_MEMORY_LIMIT, MUTANT_TEST_RUNNER,
    MUTANT_TIMEOUT_FLOOR, MUTANT_TIMEOUT_MARGIN, Step, VISUAL_CONFORMANCE, changes_mutants_command,
    check_steps, fused_command, fused_gate, miri_command, mutant_timeout_args, safe_message,
    stryker_command, unsafe_crates, workspace_mutants_command,
};
pub use supply_chain::{
    DENY_CONFIG, DENY_TOML, DEPENDABOT, DEPENDABOT_CONFIG, MINIMUM_RELEASE_AGE, PNPM_WORKSPACE,
    PNPM_WORKSPACE_KEYS, ROOT_PACKAGE_JSON, SUPPLY_CHAIN_FILES, supply_chain_gate,
};
pub use test_code::{
    DOC_TESTED_KINDS, NOT_TEST_ATTRIBUTES, TARGET_KINDS, test_code_gate, test_data_gate,
    untested_targets_gate, unviable_names_gate,
};
pub use unsafety::{
    ASSEMBLY_CALL, ASSEMBLY_FILES, ASSEMBLY_REFUSED_WORDS, DOC_TESTS_FORBID_UNSAFE_CODE,
    FORBID_UNSAFE_CODE, ONE_UNSAFE_OPERATION, REFUSED_ASSEMBLY, TEST_DECLARATION_FORM,
    TestDeclaration, UNSAFE_CODE_KEPT, UNSAFE_FILES, test_declarations, unsafe_gate,
};
pub use vocabulary::{VOCABULARY, VOCABULARY_SOURCES, vocabulary_gate};
pub use workflow::{
    AUDIO_SINK, JOB_TIMEOUTS, MIRI_TIMEOUT, NIGHTLY_COMPONENTS, NODE_VERSION_FILE,
    PLAYWRIGHT_OUTPUTS, RUNNER, WASM_MODULES, WORKFLOW, WORKFLOW_CONDITIONS, WORKFLOW_PERMISSIONS,
    WORKFLOW_TRIGGERS, failure_uploads, workflow_concurrency, workflow_gate, workflow_text,
};
pub use workspace::{
    CHANGES_DIFF, CONFORMANCE_EXPORT, COVERAGE_DIR, COVERAGE_LOCALITY, Coverage, CrateRole,
    ENGINE_PACKAGE, EQUIVALENT_REVIEWED, Extent, FIXTURES, LIBRARIES, Library, MAIN_BASES,
    MUTANTS_OUT, MUTATION_BASES, Measured, Mode, NATIVE_PACKAGES, NATIVE_TARGETS,
    NIGHTLY_TOOLCHAIN, OnFailure, PACKAGES, PORTABILITY_PACKAGES, PORTABILITY_TARGETS, Package,
    RUST_CRATES, RustCrate, STRYKER_INCREMENTAL, UNVIABLE_REPORT, UNVIABLE_REVIEWED, WASM_PACKAGES,
    WASM_TARGET, build_outputs, crates_in, mutation_jobs, package_jsons, typescript_reports,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
