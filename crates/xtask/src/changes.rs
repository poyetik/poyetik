//! What changed since the mutation base, and how far a change widens the mutants to test.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use core::num::NonZeroUsize;
use std::io::Write;

use crate::{
    actions::{
        crates_metadata, git_output, gpu_mutants, list_tracked, mutants_with_doc_tests, read_file,
        say,
    },
    cli::{Revision, file_at},
    licenses::{license_commands, license_gate, shipped_exceptions_gate},
    math_layer::{cargo_packages, dependencies_of, workspace_dependency, workspace_root},
    preflight::{CARGO_CONFIG_FILE, RUST_TOOLCHAIN_FILE},
    proof_record::{
        MEASURED_BASE, ProofRecord, filtered_files, hidden, mutation_rule, unshown_files,
        usable_proof,
    },
    release::CARGO_TOML,
    runner::Runner,
    rust_source::RustSource,
    steps::{
        Cmd, Dir, Failure, NativePart, all_of, changes_list_command, changes_mutants_command,
        safe_message, tested_mutants_command,
    },
    supply_chain::PNPM_SETTINGS_FILES,
    supply_chain::PNPM_WORKSPACE,
    test_code::{in_crate, under_root},
    unsafety::test_declarations,
    workspace::{
        CHANGES_DIFF, CargoWorkspace, CrateRole, Extent, GPU_WORKSPACE, MAIN_BASES, MUTATION_BASES,
        RUST_CRATES, RustCrate, code_sources,
    },
};

/// Lists the licenses of what ships and of everything, then checks them, and the
/// exceptions of `pnpm audit` against what ships ([`shipped_exceptions_gate`]).
pub(crate) fn licenses(runner: &mut dyn Runner) -> Result<(), Failure> {
    let [shipped, all] = license_commands().map(|cmd| {
        runner
            .capture(&cmd)
            .map_err(|failure| Failure::from(format!("cannot list licenses: {failure}")))
    });
    let shipped = shipped?;
    let workspace = read_file(runner, PNPM_WORKSPACE)?;
    all_of([
        license_gate(&shipped, &all?),
        shipped_exceptions_gate(&shipped, &workspace),
    ])
}

/// The merge base of `HEAD` with the first of `bases` git resolves, and that base's name.
///
/// # Errors
///
/// Returns `none`, the summary of no base found, with why each could not be resolved.
pub(crate) fn merge_base(
    runner: &mut dyn Runner,
    bases: &[&'static str],
    none: &str,
) -> Result<(String, &'static str), Failure> {
    let mut tried = Vec::new();
    for against in bases {
        match runner.capture(&Cmd::new(
            "git",
            &["merge-base", "HEAD", against],
            Dir::Root,
        )) {
            Ok(base) => return Ok((base.trim().to_owned(), against)),
            Err(failure) => tried.push(failure.to_string()),
        }
    }
    Err(Failure {
        summary: none.to_owned(),
        details: tried,
        usage: false,
    })
}

/// The merge base of `HEAD` with the first of [`MUTATION_BASES`] git resolves, and that
/// base's name.
///
/// # Errors
///
/// Returns that none could be resolved, with why each could not.
pub(crate) fn mutation_base(runner: &mut dyn Runner) -> Result<(String, &'static str), Failure> {
    merge_base(
        runner,
        &MUTATION_BASES,
        &format!(
            "no base to measure the changes against: HEAD shares no history with any of {} \
             (see: cargo xtask check --full)",
            MUTATION_BASES.join(", ")
        ),
    )
}

/// The merge base of `HEAD` with the first of [`MAIN_BASES`] git resolves, and that base's
/// name: where what reached `main` is read, `reading` (said when there is none).
///
/// # Errors
///
/// Returns that none could be resolved, with why each could not.
pub(crate) fn main_base(
    runner: &mut dyn Runner,
    reading: &str,
) -> Result<(String, &'static str), Failure> {
    merge_base(
        runner,
        &MAIN_BASES,
        &format!(
            "no merge base with {} to read {reading} from (fetch or create one of them)",
            MAIN_BASES.join(" or ")
        ),
    )
}

/// The merge base where the algorithm versions spent are read ([`main_base`]).
///
/// # Errors
///
/// Returns that none could be resolved, with why each could not.
pub(crate) fn spent_base(runner: &mut dyn Runner) -> Result<(String, &'static str), Failure> {
    main_base(runner, "the algorithm versions spent")
}

/// The diff git would show for `path`, a new file holding `text`, added whole; nothing for
/// an empty file, which holds no mutant.
#[must_use]
pub fn new_file_diff(path: &str, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        text.lines().count()
    );
    for line in text.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if !text.ends_with('\n') {
        diff.push_str("\\ No newline at end of file\n");
    }
    diff
}

/// Whether `rel`, a path relative to its crate, is Rust test code: under `tests/` (its
/// fixtures included, [`in_tests_dir`]), or a unit test file beside its source
/// ([`unit_test_file`]).
pub(crate) fn rust_test_code(rel: &str) -> bool {
    in_tests_dir(rel) || unit_test_file(rel)
}

/// Whether `path` names a Rust file: it ends with `.rs`, exactly as git's pathspec `*.rs`
/// matches it (`*` matching any prefix, `/` and the empty string included: `dir/.rs`), in
/// that case only (`X.RS` is none, as for git).
pub(crate) fn rust_file(path: &str) -> bool {
    path.as_bytes().ends_with(b".rs")
}

/// Whether `path` names a unit test file beside its source (`*_tests.rs`): a path relative
/// to its crate or to the repository, the suffix being the same in both.
pub(crate) fn unit_test_file(path: &str) -> bool {
    path.ends_with("_tests.rs")
}

/// Whether `rel`, a path relative to its crate (never to the repository), lies under the
/// crate's `tests/` directory.
pub(crate) fn in_tests_dir(rel: &str) -> bool {
    rel.starts_with("tests/")
}

/// What every Rust test depends on, at the root: the default check mutates everything when
/// one changes.
///
/// The pinned toolchain, and the repository's Cargo configuration ([`CARGO_CONFIG_FILE`]),
/// whose `[env]` or `[build]` table would reach every cargo command: a change to either can
/// let any mutant survive (the legacy `.cargo/config` beside it is refused by the preflight
/// whenever it exists, [`cargo_config_candidates`](crate::preflight::cargo_config_candidates)). The workspace's manifest
/// ([`CARGO_TOML`]) widens to everything too, but for a change of its shared dependencies or
/// its lints alone (`manifest_widened`).
pub const MUTATE_EVERYTHING_WHEN: [&str; 2] = [RUST_TOOLCHAIN_FILE, CARGO_CONFIG_FILE];

/// The header of the table of the workspace's shared dependencies, which its crates
/// inherit (`<name>.workspace = true`), as the workspace's manifest writes it.
pub const WORKSPACE_DEPENDENCIES: &str = "[workspace.dependencies]";

/// The resolved dependencies of the workspace: a change mutates every crate whose build
/// reaches a changed entry (`lock_widened`).
pub const LOCK_FILE: &str = "Cargo.lock";

/// The names of the files that configure a Cargo build, wherever they are.
pub(crate) const CARGO_FILES: [&str; 3] = [CARGO_TOML, LOCK_FILE, RUST_TOOLCHAIN_FILE];

/// What a change to one file widens the default check's mutation testing to, beyond the
/// lines it changes ([`widening`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Widening {
    /// Every crate: a file of [`MUTATE_EVERYTHING_WHEN`], or a Cargo file ([`CARGO_FILES`])
    /// of no crate of [`RUST_CRATES`].
    Everything,
    /// The crates whose build reaches a changed entry of [`LOCK_FILE`].
    Lock,
    /// The crates whose build includes a crate that declares a changed entry of the
    /// workspace's shared dependencies ([`WORKSPACE_DEPENDENCIES`] in [`CARGO_TOML`]), none
    /// for its lint tables, or every crate when anything else of it changed
    /// ([`manifest_widened`]).
    Workspace,
    /// The crate whose manifest it is, and every crate whose build includes it.
    Manifest(&'static str),
    /// Every crate whose tests use this crate of shared test code ([`CrateRole::SharedTests`]).
    SharedTests(&'static str),
}

/// What a change to `path` widens the mutation testing to, if anything: [`Widening`].
///
/// A Cargo file below a crate's directory but its manifest (a fixture of its tests) is a
/// file of that crate, widened as its test code or as nothing.
pub(crate) fn widening(path: &str) -> Option<Widening> {
    if MUTATE_EVERYTHING_WHEN.contains(&path) {
        return Some(Widening::Everything);
    }
    if path == LOCK_FILE {
        return Some(Widening::Lock);
    }
    if path == CARGO_TOML {
        return Some(Widening::Workspace);
    }
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    match in_crate(path) {
        Some((krate, CARGO_TOML)) => Some(Widening::Manifest(krate.name)),
        Some((krate, _)) => krate
            .roles
            .contains(&CrateRole::SharedTests)
            .then_some(Widening::SharedTests(krate.name)),
        None => CARGO_FILES.contains(&name).then_some(Widening::Everything),
    }
}

/// One package of a lock file, by its name, version and source (empty for a crate of the
/// workspace): its whole entry as written, and the packages its `dependencies` name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LockEntry {
    /// The lines of its entry, after its name, version and source.
    lines: Vec<String>,
    /// Its dependencies as the lock names them: a name, then a version and a source only
    /// when the name alone is ambiguous.
    dependencies: Vec<String>,
}

/// The key of a lock entry: name, version, source.
pub(crate) type LockKey = (String, String, String);

/// The value of a line `<key> = "<value>"`, as Cargo writes it.
pub(crate) fn lock_value<'line>(line: &'line str, key: &str) -> Option<&'line str> {
    line.strip_prefix(key)?
        .strip_prefix(" = \"")?
        .strip_suffix('"')
}

/// The lines of a lock file, each with its number from 1.
pub(crate) type LockLines<'text> =
    core::iter::Peekable<core::iter::Zip<core::str::Lines<'text>, core::ops::RangeFrom<usize>>>;

/// The value of the next line of `lines`, `<key> = "<value>"`.
///
/// # Errors
///
/// Returns the line that is not, or that the file ends there.
pub(crate) fn lock_field<'text>(
    lines: &mut LockLines<'text>,
    key: &str,
) -> Result<&'text str, String> {
    let (line, number) = lines
        .next()
        .ok_or_else(|| format!("it ends where `{key} = \"…\"` is expected"))?;
    lock_value(line, key).ok_or_else(|| format!("line {number} is not `{key} = \"…\"`"))
}

/// The entries of `lock`, a lock file as Cargo writes it (`version = 4` and its header,
/// then `[[package]]` tables of `name`, `version`, an optional `source` and `checksum`, and
/// optional `dependencies`, one quoted name per line, indented by one space).
///
/// # Errors
///
/// Returns where it is written otherwise: the line, or the end of the file.
pub(crate) fn lock_entries(lock: &str) -> Result<BTreeMap<LockKey, LockEntry>, String> {
    let mut entries = BTreeMap::new();
    let mut lines: LockLines<'_> = lock.lines().zip(1..).peekable();
    while lines
        .next_if(|(header, _)| header.starts_with('#') || header.is_empty())
        .is_some()
    {}
    if lines.next().map(|(line, _)| line) != Some("version = 4") {
        return Err("it does not start with `version = 4` after its comments".to_owned());
    }
    while let Some((table, at)) = lines.next() {
        if table.is_empty() {
            continue;
        }
        if table != "[[package]]" {
            return Err(format!("line {at} is not `[[package]]`"));
        }
        let name = lock_field(&mut lines, "name")?;
        let version = lock_field(&mut lines, "version")?;
        let source = if lines
            .peek()
            .is_some_and(|(field, _)| field.starts_with("source = "))
        {
            lock_field(&mut lines, "source")?
        } else {
            ""
        };
        let mut entry = LockEntry {
            lines: Vec::new(),
            dependencies: Vec::new(),
        };
        if lines
            .peek()
            .is_some_and(|(field, _)| field.starts_with("checksum = "))
        {
            entry
                .lines
                .push(lock_field(&mut lines, "checksum")?.to_owned());
        }
        if lines
            .next_if(|(field, _)| *field == "dependencies = [")
            .is_some()
        {
            loop {
                let (item, number) = lines
                    .next()
                    .ok_or_else(|| "it ends in a list of dependencies".to_owned())?;
                if item == "]" {
                    break;
                }
                let dependency = item
                    .strip_prefix(" \"")
                    .and_then(|rest| rest.strip_suffix("\","))
                    .ok_or_else(|| format!("line {number} is not ` \"<dependency>\",`"))?;
                entry.dependencies.push(dependency.to_owned());
                entry.lines.push(item.to_owned());
            }
        }
        if let Some((_, number)) = lines.peek().filter(|(next, _)| !next.is_empty()) {
            return Err(format!(
                "line {number} is not the blank line ending an entry"
            ));
        }
        let key = (name.to_owned(), version.to_owned(), source.to_owned());
        if entries.insert(key, entry).is_some() {
            return Err(format!("the entry of line {at} repeats an earlier one"));
        }
    }
    Ok(entries)
}

/// The one entry of `entries` that `dependency` (`name`, `name version` or `name version
/// (source)`) names.
///
/// # Errors
///
/// Returns that it names none or several, or is written otherwise.
pub(crate) fn lock_dependency<'entries>(
    entries: &'entries BTreeMap<LockKey, LockEntry>,
    dependency: &str,
) -> Result<(&'entries LockKey, &'entries LockEntry), String> {
    let unresolved = || {
        format!(
            "a dependency, `{}`, names no one entry",
            dependency.escape_debug()
        )
    };
    let (name, rest) = dependency.split_once(' ').unwrap_or((dependency, ""));
    let (version, source) = rest.split_once(' ').unwrap_or((rest, ""));
    let source = match source {
        "" => None,
        quoted => Some(
            quoted
                .strip_prefix('(')
                .and_then(|inner| inner.strip_suffix(')'))
                .ok_or_else(unresolved)?,
        ),
    };
    let mut named = entries.iter().filter(|((entry, at, from), _)| {
        entry == name
            && (version.is_empty() || at == version)
            && source.is_none_or(|source| from == source)
    });
    let found = named.next().ok_or_else(unresolved)?;
    named
        .next()
        .is_none()
        .then_some(found)
        .ok_or_else(unresolved)
}

/// The crates of [`RUST_CRATES`] whose build reaches an entry of `lock` (a lock file) that
/// differs from `before` (the same file at an earlier commit), themselves included: an
/// entry added, removed or changed (its version, source, checksum or dependencies), found by
/// following the dependencies `lock` records.
///
/// A crate that reaches no changed entry builds with the same packages as before: an entry
/// that reached a removed one changed its dependencies, so it is reached first.
///
/// # Errors
///
/// Returns why the crates cannot be found: either file is not as Cargo writes it, a crate
/// of the workspace has not exactly one entry, or a dependency names no one entry.
pub(crate) fn lock_widened(before: &str, lock: &str) -> Result<BTreeSet<&'static str>, String> {
    let old = lock_entries(before).map_err(|reason| format!("its earlier version: {reason}"))?;
    let new = lock_entries(lock)?;
    let changed: BTreeSet<&LockKey> = old
        .keys()
        .chain(new.keys())
        .filter(|key| old.get(*key) != new.get(*key))
        .collect();
    let mut widened = BTreeSet::new();
    for krate in RUST_CRATES {
        let mut pending: Vec<(&LockKey, &LockEntry)> = new
            .iter()
            .filter(|((name, _, source), _)| name == krate.name && source.is_empty())
            .collect();
        if pending.len() != 1 {
            return Err(format!(
                "it does not hold exactly one entry of {}",
                krate.name
            ));
        }
        let mut seen: BTreeSet<&LockKey> = BTreeSet::new();
        while let Some((key, entry)) = pending.pop() {
            if !seen.insert(key) {
                continue;
            }
            if changed.contains(key) {
                let _new = widened.insert(krate.name);
            }
            for dependency in &entry.dependencies {
                pending.push(lock_dependency(&new, dependency)?);
            }
        }
    }
    Ok(widened)
}

/// The file at `path` in the commit `since` ([`file_at`]), then in the work tree.
///
/// # Errors
///
/// Returns why either cannot be read: missing at `since`, git's error, or the system's.
pub(crate) fn file_changes(
    runner: &mut dyn Runner,
    since: &str,
    path: &str,
) -> Result<(String, String), String> {
    let before = file_at(runner, since, path)
        .map_err(|failure| failure.summary)?
        .ok_or_else(|| format!("missing at {since}"))?;
    let now = runner
        .read(path)
        .map_err(|error| format!("cannot read it: {error}"))?;
    Ok((before, now))
}

/// The crates whose build reaches an entry of [`LOCK_FILE`] changed since `since`
/// ([`lock_widened`]).
///
/// # Errors
///
/// Returns why every crate may build differently: the file cannot be read there or here,
/// or the crates cannot be found from it.
pub(crate) fn lock_changes(
    runner: &mut dyn Runner,
    since: &str,
) -> Result<BTreeSet<&'static str>, String> {
    let (before, lock) = file_changes(runner, since, LOCK_FILE)?;
    lock_widened(&before, &lock)
}

/// Whether `value`, what follows `=` on the line of an entry, holds one whole value on that
/// line, before an optional comment: its basic strings (`"…"`) closed, every inline table
/// and array it opens closed in order. A backslash (an escape) or an apostrophe outside a
/// basic string (a literal string, where a bracket or a quote is text) is refused, never
/// read.
pub(crate) fn one_line_value(value: &str) -> bool {
    let mut open: Vec<char> = Vec::new();
    let mut quoted = false;
    for character in value.chars() {
        match (quoted, character) {
            (false, '#') => break,
            (_, '\\') | (false, '\'') => return false,
            (_, '"') => quoted = !quoted,
            (false, '{') => open.push('}'),
            (false, '[') => open.push(']'),
            (false, '}' | ']') if open.pop() != Some(character) => return false,
            _ => {}
        }
    }
    !quoted && open.is_empty()
}

/// Whether `key` is a bare key of TOML: ASCII letters, digits, `-` and `_`, at least one.
pub(crate) fn bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
}

/// The name of the entry of shared dependencies `line` writes (`<name> = <value>`, leading
/// blanks apart), if it is one whole entry on one line: a bare key ([`one_line_value`]).
pub(crate) fn dependency_entry(line: &str) -> Option<&str> {
    let (name, value) = line.trim_start_matches([' ', '\t']).split_once('=')?;
    let name = name.trim_end_matches([' ', '\t']);
    (bare_key(name) && one_line_value(value)).then_some(name)
}

/// The header of the workspace's lint table, which its crates inherit (`[lints]`,
/// `workspace = true`), as the workspace's manifest may write it.
pub(crate) const WORKSPACE_LINTS: &str = "[workspace.lints]";

/// Whether `line` is exactly the header of a table of the workspace's lints:
/// [`WORKSPACE_LINTS`] or `[workspace.lints.<tool>]`, a bare key ([`bare_key`]). A header
/// written otherwise (spaces, quotes, a comment after it, a carriage return) is none, so
/// its table is compared whole.
pub(crate) fn lints_header(line: &str) -> bool {
    line == WORKSPACE_LINTS
        || line
            .strip_prefix("[workspace.lints.")
            .and_then(|tool| tool.strip_suffix(']'))
            .is_some_and(bare_key)
}

/// The workspace's manifest `manifest` but the lines of its shared dependencies and of its
/// lint tables, and each of those entries by its name ([`dependency_entry`]), its whole
/// line.
///
/// Read only as far as it can be read soundly, so that a manifest it accepts holds exactly
/// those entries in that table: no multi-line string (in which a line could look like a
/// table), exactly one line [`WORKSPACE_DEPENDENCIES`] (the table, which every other line
/// beginning with `[` ends), each line of the table blank, a comment or one entry, no name
/// twice. An entry the rest of the manifest writes elsewhere in the table (a key of
/// `[workspace]`, a table of its own) is in the rest, compared whole.
///
/// The lines of a lint table ([`lints_header`]), its header included, are left out up to
/// the next line beginning with `[`: a key there can only be a lint of that table (TOML has
/// no way out of a table but a header), and a line of a value beginning with `[` ends what
/// is left out early, the rest then compared. A change of lint levels changes no mutant's
/// outcome: every build cargo-mutants makes caps lints ([`MUTANTS_ARGS`](crate::steps::MUTANTS_ARGS)), and the
/// measurements behind it are in `crates/xtask/README.md`.
///
/// # Errors
///
/// Returns what it does not read: a multi-line string, the table not exactly once, or the
/// line of the table that is not an entry, or repeats one.
pub(crate) fn shared_dependencies(
    manifest: &str,
) -> Result<(String, BTreeMap<&str, &str>), String> {
    if manifest.contains("\"\"\"") || manifest.contains("'''") {
        return Err("it holds a multi-line string".to_owned());
    }
    let lines: Vec<&str> = manifest.split('\n').collect();
    let mut headers = lines
        .iter()
        .zip(1_usize..)
        .filter(|(line, _)| **line == WORKSPACE_DEPENDENCIES);
    let (Some((_, start)), None) = (headers.next(), headers.next()) else {
        return Err(format!(
            "it does not hold exactly one line `{WORKSPACE_DEPENDENCIES}`"
        ));
    };
    let table: Vec<(&str, usize)> = lines
        .iter()
        .copied()
        .zip(1_usize..)
        .skip(start)
        .take_while(|(line, _)| !line.trim_start().starts_with('['))
        .collect();
    let mut entries = BTreeMap::new();
    for (line, number) in &table {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let name = dependency_entry(line).ok_or_else(|| {
            format!("line {number} is not one entry `<name> = <value>` on one line")
        })?;
        if entries.insert(name, *line).is_some() {
            return Err(format!("line {number} repeats an entry"));
        }
    }
    let mut in_lints = false;
    let rest: Vec<&str> = lines
        .iter()
        .take(start)
        .chain(lines.iter().skip(start.saturating_add(table.len())))
        .copied()
        .filter(|line| {
            if line.trim_start().starts_with('[') {
                in_lints = lints_header(line);
            }
            !in_lints
        })
        .collect();
    Ok((rest.join("\n"), entries))
}

/// The names of the entries of the workspace's shared dependencies that differ between
/// `manifest` (the workspace's manifest) and `before` (the same file at an earlier
/// commit): added, removed or changed, comment lines and lint tables apart
/// ([`shared_dependencies`]); an entry's trailing comment is part of its line, so changing
/// it widens that entry.
///
/// An entry reaches only the crates that declare it (`<name>.workspace = true`); what it
/// resolves to is in the lock file, widened on its own ([`lock_widened`]).
///
/// # Errors
///
/// Returns why every crate may build differently: either file is not read, or anything
/// else of the manifest but its lints changed (its profiles, patches, members, package
/// fields), which no entry names.
pub(crate) fn manifest_widened(before: &str, manifest: &str) -> Result<BTreeSet<String>, String> {
    let (old_rest, old) =
        shared_dependencies(before).map_err(|reason| format!("its earlier version: {reason}"))?;
    let (rest, new) = shared_dependencies(manifest)?;
    if old_rest != rest {
        return Err(format!(
            "it changed outside the entries of `{WORKSPACE_DEPENDENCIES}`"
        ));
    }
    Ok(old
        .keys()
        .chain(new.keys())
        .filter(|name| old.get(*name) != new.get(*name))
        .map(|name| (*name).to_owned())
        .collect())
}

/// The crates whose build includes a crate that declares an entry of the shared
/// dependencies changed since `since` ([`manifest_widened`], `builds`).
///
/// # Errors
///
/// Returns why every crate may build differently: the manifest cannot be read there or
/// here, or what changed is not entries alone.
pub(crate) fn manifest_changes(
    runner: &mut dyn Runner,
    since: &str,
    builds: &Builds,
) -> Result<BTreeSet<&'static str>, String> {
    let (before, manifest) = file_changes(runner, since, CARGO_TOML)?;
    Ok(manifest_widened(&before, &manifest)?
        .iter()
        .filter_map(|name| builds.declaring.get(name))
        .flatten()
        .flat_map(|krate| crates_building(&builds.closures, krate))
        .collect())
}

/// The crates of [`RUST_CRATES`] whose closure ([`crate_closures`]) holds `name`.
pub(crate) fn crates_building(closures: &Closures, name: &str) -> BTreeSet<&'static str> {
    closures
        .iter()
        .filter(|(_, closure)| closure.iter().any(|built| built.name == name))
        .map(|(krate, _)| *krate)
        .collect()
}

/// The crates of the workspace each crate of [`RUST_CRATES`] compiles for its tests, by
/// its name, in the order of [`RUST_CRATES`] ([`crate_closures`]).
pub(crate) type Closures = BTreeMap<&'static str, Vec<&'static RustCrate>>;

/// What the crates of [`RUST_CRATES`] build, from `cargo metadata` ([`crate_builds`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Builds {
    /// What each compiles for its tests.
    pub(crate) closures: Closures,
    /// The crates declaring a dependency, by the name their manifest gives it (its key:
    /// cargo's `rename`, else its `name`), the one an entry of the shared dependencies has.
    pub(crate) declaring: BTreeMap<String, BTreeSet<&'static str>>,
}

/// What the changes since a commit widen the default check's mutation testing to, beyond
/// their lines ([`reach`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Every crate, by the first path that widens to it, and why it could not be narrowed
    /// to some crates, if it could have been ([`LOCK_FILE`], [`CARGO_TOML`]).
    Everything {
        /// The path.
        path: String,
        /// Why its change could not be narrowed.
        unnarrowed: Option<String>,
    },
    /// Each crate widened to every mutant, with the first path that widens it.
    Crates(BTreeMap<&'static str, String>),
}

/// What the default check says of a change it could not narrow to some crates (`why`), if
/// any: ` and cannot be narrowed (<why>)`.
pub(crate) fn unnarrowed_text(why: Option<&str>) -> String {
    why.map_or_else(String::new, |why| {
        format!(" and cannot be narrowed ({why})")
    })
}

/// Whether one of `changed` widens through the builds of crates (a manifest, the shared
/// test crate), which needs the [`Builds`].
pub(crate) fn needs_closures<'path>(mut changed: impl Iterator<Item = &'path str>) -> bool {
    changed.any(|path| {
        matches!(
            widening(path),
            Some(Widening::Manifest(_) | Widening::SharedTests(_) | Widening::Workspace)
        )
    })
}

/// What the paths of `changed`, changed since `since`, widen the default check's mutation
/// testing to ([`widening`], [`Reach`]): the one decision both the run and a proof record
/// take, `builds` holding what each crate builds when [`needs_closures`].
///
/// A lock file that cannot be compared entry by entry, or a workspace manifest whose change
/// is not of its shared dependencies alone, widens to every crate.
pub(crate) fn reach<'path>(
    runner: &mut dyn Runner,
    since: &str,
    changed: impl IntoIterator<Item = &'path str>,
    builds: &Builds,
) -> Reach {
    let mut widened: BTreeMap<&'static str, String> = BTreeMap::new();
    for path in changed {
        // The crates it reaches, or why it could not be narrowed, if it could have been.
        let narrowed = match widening(path) {
            None => continue,
            Some(Widening::Everything) => Err(None),
            Some(Widening::Lock) => lock_changes(runner, since).map_err(Some),
            Some(Widening::Workspace) => manifest_changes(runner, since, builds).map_err(Some),
            Some(Widening::Manifest(name) | Widening::SharedTests(name)) => {
                Ok(crates_building(&builds.closures, name))
            }
        };
        let crates = match narrowed {
            Ok(crates) => crates,
            Err(unnarrowed) => {
                return Reach::Everything {
                    path: path.to_owned(),
                    unnarrowed,
                };
            }
        };
        for krate in crates {
            let _first = widened.entry(krate).or_insert_with(|| path.to_owned());
        }
    }
    Reach::Crates(widened)
}

/// The paths of the tracked files changed since `since`, committed or not, as git names
/// them (`--name-only -z`: never quoted, whatever they hold).
///
/// # Errors
///
/// Returns why git cannot list them.
pub(crate) fn changed_since(runner: &mut dyn Runner, since: &str) -> Result<Vec<String>, Failure> {
    let listed = git_output(
        runner,
        &[
            "diff",
            "--no-color",
            "--name-only",
            "-z",
            "--no-renames",
            since,
        ],
        "list the changes",
    )?;
    Ok(nul_separated(&listed))
}

/// The paths of a NUL-separated list of git (`-z`), in its order.
pub(crate) fn nul_separated(listed: &str) -> Vec<String> {
    listed
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// How git lists the untracked files the repository does not ignore, before the pathspecs.
///
/// Only its own `.gitignore` files are read (`--exclude-per-directory`), never
/// `--exclude-standard`, which adds the user's ignore file (`core.excludesFile`, or
/// `git/ignore` below `XDG_CONFIG_HOME`) and `info/exclude`: cargo, vitest and Astro read a
/// source whatever the user ignores. Not seen: an untracked `.gitignore` that ignores itself
/// and the files beside it.
pub(crate) const UNTRACKED_LISTING: [&str; 4] = [
    "ls-files",
    "-z",
    "--others",
    "--exclude-per-directory=.gitignore",
];

/// The new Rust files the repository does not ignore ([`UNTRACKED_LISTING`]).
///
/// # Errors
///
/// Returns why git cannot list them.
pub(crate) fn new_rust_files(runner: &mut dyn Runner) -> Result<Vec<String>, Failure> {
    let listed = git_output(
        runner,
        &[&UNTRACKED_LISTING[..], &["--", "*.rs"]].concat(),
        "list the new Rust files",
    )?;
    Ok(nul_separated(&listed))
}

/// The untracked files the checks read whatever ignores them, but those the repository's
/// own `.gitignore` files name ([`UNTRACKED_LISTING`]): the code ([`code_sources`]), every
/// crate's test data ([`RustCrate::test_data`]) and npm's and pnpm's settings
/// ([`PNPM_SETTINGS_FILES`]), which cargo, vitest, Astro, npm, pnpm and the tests find by
/// themselves.
///
/// # Errors
///
/// Returns why git cannot list them.
pub(crate) fn untracked_read(runner: &mut dyn Runner) -> Result<Vec<String>, Failure> {
    let mut args = [&UNTRACKED_LISTING[..], &["--"], &code_sources()].concat();
    for root in RUST_CRATES.iter().flat_map(|krate| krate.test_data) {
        if !args.contains(root) {
            args.push(root);
        }
    }
    args.extend(PNPM_SETTINGS_FILES);
    let listed = git_output(
        runner,
        &args,
        "list the untracked code, test data and settings",
    )?;
    Ok(nul_separated(&listed))
}

/// The untracked files a change is read from: those neither the repository nor the user
/// ignores (`--exclude-standard`), and those the checks read whatever ignores them
/// ([`untracked_read`]): a backup of an editor's the user ignores (`x.rs.orig`) is then no
/// change, and test data the user ignores is one.
///
/// # Errors
///
/// Returns why git cannot list them.
pub(crate) fn untracked_changes(runner: &mut dyn Runner) -> Result<Vec<String>, Failure> {
    let mut paths = nul_separated(&git_output(
        runner,
        &["ls-files", "-z", "--others", "--exclude-standard"],
        "list the untracked files",
    )?);
    paths.extend(untracked_read(runner)?);
    Ok(paths)
}

/// The diff of the tracked Rust files (`*.rs`, the only files cargo-mutants mutates) since
/// `since`, committed or not, but those below the directories of `left_out` and the files
/// `whole`, which git leaves out (`:(exclude)` pathspecs, literal for the files, whose names
/// git would otherwise read as patterns): so that the diff holds no section of a crate or a
/// file tested whole or of a crate proven, rather than a second one, and none of a file that
/// is not Rust, whose bytes may not be text. Every file is shown as text (`--text`),
/// whatever its `diff` or `binary` attribute says, which no gate reads ([`incomplete_diff`]), and every option of
/// git's that changes which lines it writes or how is given, whatever the user's
/// configuration or environment says (the context, the algorithm, the prefixes, an empty
/// context line; [`DIFF_GIT_VARIABLES`](crate::runner::DIFF_GIT_VARIABLES)).
///
/// # Errors
///
/// Returns why git cannot show it.
pub(crate) fn diff_since(
    runner: &mut dyn Runner,
    since: &str,
    left_out: &[&RustCrate],
    whole: &[&str],
) -> Result<String, Failure> {
    let excluded: Vec<String> = left_out
        .iter()
        .map(|krate| format!(":(exclude){}/", krate.dir))
        .chain(whole.iter().map(|path| format!(":(exclude,literal){path}")))
        .collect();
    let mut args = vec![
        "-c",
        "diff.suppressBlankEmpty=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--text",
        "--no-renames",
        "--unified=3",
        "--inter-hunk-context=0",
        "--diff-algorithm=myers",
        "--indent-heuristic",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        since,
        "--",
        "*.rs",
    ];
    args.extend(excluded.iter().map(String::as_str));
    git_output(runner, &args, "read the changes")
}

/// Why `diff`, git's diff of the Rust files changed outside the crates tested whole or
/// proven, is not exactly one text section for each of the `expected` paths, if it is not.
///
/// Each changed file has exactly one section (`diff --git`), a mode change or an empty file
/// included, so the paths of the sections are compared with `expected`, as sets: a path
/// missing means git left a change out (a configuration or a variable no gate sees, such as
/// the pathspec variables [`command`](crate::runner::command) removes), a path more that it showed what was not
/// asked. A path is read from its header by git's rule without renames (`diff --git a/`,
/// the path, ` b/`, the same path again); a header of another shape is refused, and so is a
/// quoted one (`core.quotePath` quotes a path holding a byte out of printable ASCII, which
/// no Rust module's path holds, `#[path]` being refused). A path shown twice is refused: a
/// file deleted then created is a type change (a file turned into a symbolic link, or the
/// reverse), which cargo-mutants would read as one change. No section is binary (`Binary
/// files … differ`, `GIT binary patch`), which `--text` asks git never to write, whatever
/// the attributes say. A line of a file starts with `+`, `-`, a space or `\`, or is empty
/// (an empty context line under `diff.suppressBlankEmpty`), so these lines can only be
/// git's.
pub(crate) fn incomplete_diff(diff: &str, expected: &BTreeSet<&str>) -> Option<String> {
    let mut shown: Vec<&str> = Vec::new();
    let (mut header, mut deleted) = ("", false);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            header = line;
            let Some(path) = section_path(rest) else {
                return Some(format!(
                    "git wrote `{line}`, a header whose path cannot be read (quoted, or not \
                     `a/<path> b/<path>`)"
                ));
            };
            if shown.contains(&path) {
                return Some(if deleted && shown.last() == Some(&path) {
                    format!(
                        "git's diff shows {path} as two sections, a file deleted then one \
                         created: its type changed (a file turned into a symbolic link, or the \
                         reverse), which cargo-mutants would read as one change"
                    )
                } else {
                    format!("git's diff shows {path} in two sections")
                });
            }
            shown.push(path);
            deleted = false;
        } else if line.starts_with("deleted file mode ") {
            deleted = true;
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            return Some(format!(
                "git wrote `{header}` as binary: cargo-mutants would see none of its changed \
                 lines"
            ));
        }
    }
    let shown: BTreeSet<&str> = shown.into_iter().collect();
    if let Some(missing) = expected.difference(&shown).next() {
        return Some(format!(
            "git's diff holds no section for {missing}, a changed Rust file: it left the change \
             out, whose mutants would go untested"
        ));
    }
    shown.difference(expected).next().map(|more| {
        format!(
            "git's diff holds a section for {more}, which is no changed Rust file outside the \
             crates left out"
        )
    })
}

/// The path of a section header of git's diff without renames, after `diff --git `: the
/// path `P` of `a/P b/P`, both halves equal, or `None` (a quoted path, another shape).
pub(crate) fn section_path(header: &str) -> Option<&str> {
    let pair = header.strip_prefix("a/")?;
    let half = pair.len().checked_sub(3)? / 2;
    let (path, rest) = (pair.get(..half)?, pair.get(half..)?);
    (rest.strip_prefix(" b/")? == path).then_some(path)
}

/// The crates of the workspace each crate of [`RUST_CRATES`] depends on, at any depth and
/// of any kind (normal, build, development), itself included, from `metadata` (the output
/// of [`metadata_command`](crate::dependencies::metadata_command)): what its tests compile.
///
/// # Errors
///
/// Returns why the metadata cannot be read.
pub(crate) fn crate_closures(metadata: &str) -> Result<Closures, Failure> {
    let packages = cargo_packages(metadata)?;
    let root = workspace_root(metadata)?;
    let mut direct: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    for (name, package) in packages {
        let Some(krate) = RUST_CRATES.iter().find(|krate| krate.name == name) else {
            continue;
        };
        let built = direct.entry(krate.name).or_default();
        for dependency in dependencies_of(&package) {
            if let Some(dependency) = workspace_dependency(dependency, &root)? {
                let _new = built.insert(dependency.name);
            }
        }
    }
    Ok(RUST_CRATES
        .iter()
        .map(|krate| {
            let mut closure = BTreeSet::from([krate.name]);
            let mut pending = vec![krate.name];
            while let Some(name) = pending.pop() {
                for dependency in direct.get(name).into_iter().flatten() {
                    if closure.insert(dependency) {
                        pending.push(dependency);
                    }
                }
            }
            let crates = RUST_CRATES
                .iter()
                .filter(|built| closure.contains(built.name))
                .collect();
            (krate.name, crates)
        })
        .collect())
}

/// What the crates of [`RUST_CRATES`] build ([`Builds`]), from `metadata` (the output of
/// [`metadata_command`](crate::dependencies::metadata_command)): their closures ([`crate_closures`]) and the name each dependency
/// has in their manifests, every kind and target included.
///
/// # Errors
///
/// Returns why the metadata cannot be read, or that a dependency has no name.
pub(crate) fn crate_builds(metadata: &str) -> Result<Builds, Failure> {
    let mut declaring: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    for (name, package) in cargo_packages(metadata)? {
        let Some(krate) = RUST_CRATES.iter().find(|krate| krate.name == name) else {
            continue;
        };
        for dependency in dependencies_of(&package) {
            let key = ["rename", "name"]
                .iter()
                .find_map(|field| dependency.get(field).and_then(serde_json::Value::as_str))
                .ok_or_else(|| {
                    Failure::from("cargo metadata lists a dependency without a name".to_owned())
                })?;
            let _new = declaring
                .entry(key.to_owned())
                .or_default()
                .insert(krate.name);
        }
    }
    Ok(Builds {
        closures: crate_closures(metadata)?,
        declaring,
    })
}

/// The crates the proof record `record` still proves: those whose inputs are unchanged
/// since its commit ([`ProofRecord`]), none when what every test depends on or the mutation
/// rules changed since (said on `out`). Its changes widen as the run's do ([`reach`]); a
/// change of any crate a crate builds (`builds`) is a change of its inputs.
///
/// # Errors
///
/// Returns why the changes since the record, the untracked files or the lock files cannot
/// be read, or stdout cannot be written.
pub(crate) fn proven_crates(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    record: &ProofRecord,
    builds: &Builds,
) -> Result<Vec<RustCrate>, Failure> {
    let tracked = changed_since(runner, &record.commit)?;
    let untracked = untracked_changes(runner)?;
    let changed: BTreeSet<&str> = tracked
        .iter()
        .chain(&untracked)
        .map(String::as_str)
        .collect();
    let voiding = match changed.iter().find(|path| mutation_rule(path)) {
        Some(path) => format!("{path} changed since, the mutation rules"),
        None => match reach(runner, &record.commit, changed.iter().copied(), builds) {
            Reach::Everything { path, unnarrowed } => format!(
                "{path} changed since{}, what every test depends on",
                unnarrowed_text(unnarrowed.as_deref())
            ),
            Reach::Crates(reached) => {
                return Ok(RUST_CRATES
                    .into_iter()
                    .filter(|krate| {
                        !reached.contains_key(krate.name)
                            && !changed
                                .iter()
                                .any(|path| inputs_changed(krate, path, &builds.closures))
                    })
                    .collect());
            }
        },
    };
    say(
        out,
        &safe_message(&format!(
            "  ignored the mutation proof record of {}: {voiding}",
            record.commit
        )),
    )?;
    Ok(Vec::new())
}

/// Whether `path`, changed, is an input of the tests of `krate`: a file of a crate it
/// builds (`closures`, itself included) or of its declared test data.
pub(crate) fn inputs_changed(krate: &RustCrate, path: &str, closures: &Closures) -> bool {
    closures
        .get(krate.name)
        .into_iter()
        .flatten()
        .any(|built| built.holds(path).is_some())
        || krate.test_data.iter().any(|root| under_root(path, root))
}

/// How much of a crate the changes put in scope ([`in_scope`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MutatedPart {
    /// The lines of it that changed.
    Lines,
    /// Every mutant of it, with why (`None` for [`Reach::Everything`], said once).
    Whole(Option<String>),
    /// Every mutant of each source declaring one of these changed unit test files
    /// ([`declaring_sources`]), and the lines of it that changed; every mutant of it when a
    /// source declares none of them.
    UnitTests(Vec<String>),
}

/// Each crate the changes `changed` put in scope, and how much of it ([`MutatedPart`]): its own
/// lines changed; the sources whose unit tests changed; the whole crate when its integration
/// tests or its test data changed, or `reach` widens it.
///
/// A unit test file widens to the source declaring it only, though its tests may kill
/// mutants of other sources too: a weakened test can then leave a mutant of another source
/// alive that the default check does not run. The full check mutates everything, and every
/// change reaches `main` through it.
pub(crate) fn in_scope(changed: &[String], reach: &Reach) -> Vec<(RustCrate, MutatedPart)> {
    RUST_CRATES
        .into_iter()
        .filter_map(|krate| {
            let every = format!("every mutant of {}", krate.name);
            let code = changed
                .iter()
                .find(|path| krate.holds(path).is_some_and(in_tests_dir))
                .map(|_| format!("the integration tests of {} changed: {every}", krate.name));
            let data = changed
                .iter()
                .find(|path| krate.test_data.iter().any(|root| under_root(path, root)))
                .map(|_| format!("the test data of {} changed: {every}", krate.name));
            let whole = match reach {
                Reach::Everything { .. } => Some(MutatedPart::Whole(None)),
                Reach::Crates(crates) => code
                    .or(data)
                    .or_else(|| {
                        crates
                            .get(krate.name)
                            .map(|path| format!("{path} changed: {every}"))
                    })
                    .map(|said| MutatedPart::Whole(Some(said))),
            };
            // A change below `tests/` is tested whole above: what is left of test code is
            // unit test files.
            let unit_tests: Vec<String> = changed
                .iter()
                .filter(|path| krate.holds(path).is_some_and(unit_test_file))
                .cloned()
                .collect();
            let touched = changed.iter().any(|path| krate.holds(path).is_some());
            whole
                .or_else(|| (!unit_tests.is_empty()).then_some(MutatedPart::UnitTests(unit_tests)))
                .or_else(|| touched.then_some(MutatedPart::Lines))
                .map(|scope| (krate, scope))
        })
        .collect()
}

/// The sources of `krate` among `sources` that declare the unit test files `tests`
/// ([`test_declarations`]: the one declaration form, which the test code gate requires of
/// every unit test file, once), each with why, said once per source; or, when a test file
/// is declared by none (deleted, or its declaration removed), why the whole crate is tested
/// instead.
///
/// # Errors
///
/// Returns why a source of `krate` cannot be read or lexed.
pub(crate) fn declaring_sources(
    runner: &mut dyn Runner,
    krate: &RustCrate,
    tests: &[String],
    sources: &[String],
) -> Result<Result<Vec<(String, String)>, String>, Failure> {
    let mut declarer = BTreeMap::new();
    for path in sources
        .iter()
        .filter(|path| krate.holds(path).is_some_and(|rel| !rust_test_code(rel)))
    {
        let source = RustSource::parse(path, &read_file(runner, path)?)?;
        for declaration in test_declarations(&source) {
            let _first = declarer.insert(declaration.file, path.clone());
        }
    }
    let mut found: Vec<(String, String)> = Vec::new();
    for file in tests {
        let Some(source) = declarer.get(file) else {
            return Ok(Err(format!(
                "{file} changed, which no source of {name} declares: every mutant of {name}",
                name = krate.name
            )));
        };
        if !found.iter().any(|(known, _)| known == source) {
            found.push((
                source.clone(),
                format!("{file} changed: every mutant of {source}, which declares it"),
            ));
        }
    }
    Ok(Ok(found))
}

/// The diff cargo-mutants mutates the lines of: the changes since `base` (`changed`, the
/// tracked files changed since it; `new`, the new Rust files, added whole) but those of the
/// crates `tested` whole or `proven` and of the sources declaring the unit test files of
/// `unit_tests` ([`declaring_sources`]; a crate with a test file no source declares is
/// tested whole), which git leaves out, so that each path has at most one section; then
/// each of those sources and every source of each crate tested whole (its test code apart),
/// added whole, after saying why on `out`.
///
/// # Errors
///
/// Returns why git cannot show the changes, or shows them other than as one text section
/// per changed Rust file outside the crates and sources left out ([`incomplete_diff`]), a
/// file cannot be read or lexed, or stdout cannot be written.
pub(crate) fn mutated_diff(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    base: &str,
    (changed, new): (&[String], &[String]),
    mut tested: Vec<(RustCrate, Option<String>)>,
    unit_tests: Vec<(RustCrate, Vec<String>)>,
    proven: &[RustCrate],
) -> Result<String, Failure> {
    let mut sources = Vec::new();
    if !tested.is_empty() || !unit_tests.is_empty() {
        sources = list_tracked(runner, &["*.rs"], "the Rust files")?;
        sources.extend(new.iter().cloned());
        sources.sort();
        sources.dedup();
    }
    let mut declaring: Vec<(String, String)> = Vec::new();
    for (krate, tests) in unit_tests {
        match declaring_sources(runner, &krate, &tests, &sources)? {
            Ok(found) => declaring.extend(found),
            Err(said) => tested.push((krate, Some(said))),
        }
    }
    // Git leaves out every crate tested whole or proven, and every source tested whole, so
    // that each path has at most one section: the changes of the others, then each source
    // tested whole.
    let left_out: Vec<&RustCrate> = tested
        .iter()
        .map(|(krate, _)| krate)
        .chain(proven)
        .collect();
    let whole: Vec<&str> = declaring
        .iter()
        .map(|(source, _)| source.as_str())
        .collect();
    let outside = |path: &str| {
        !left_out.iter().any(|krate| krate.holds(path).is_some()) && !whole.contains(&path)
    };
    let mut diff = diff_since(runner, base, &left_out, &whole)?;
    let expected: BTreeSet<&str> = changed
        .iter()
        .filter(|path| outside(path) && rust_file(path))
        .map(String::as_str)
        .collect();
    if let Some(reason) = incomplete_diff(&diff, &expected) {
        return Err(Failure::from(reason));
    }
    for path in new.iter().filter(|path| outside(path)) {
        diff.push_str(&new_file_diff(path, &read_file(runner, path)?));
    }
    for (source, said) in &declaring {
        say(out, &safe_message(&format!("  {said}")))?;
        diff.push_str(&new_file_diff(source, &read_file(runner, source)?));
    }
    for (krate, said) in tested {
        if let Some(said) = said {
            say(out, &safe_message(&format!("  {said}")))?;
        }
        for path in &sources {
            if krate.holds(path).is_some_and(|rel| !rust_test_code(rel)) {
                let text = read_file(runner, path)?;
                diff.push_str(&new_file_diff(path, &text));
            }
        }
    }
    Ok(diff)
}

/// Checks that git shows every change of the tracked files: none hidden from its status
/// ([`hidden`]) nor passed through a filter ([`filtered`](crate::proof_record::filtered)), and none whose bytes git
/// does not show ([`unshown`](crate::proof_record::unshown)), which would be in no diff git gives, and so go
/// unmutated.
///
/// # Errors
///
/// Returns the first such file, or why git cannot list them.
pub(crate) fn shown_whole(runner: &mut dyn Runner) -> Result<(), Failure> {
    let listed = git_output(runner, &["ls-files", "-v", "-z"], "read the work tree")?;
    if let Some(reason) = hidden(&listed) {
        return Err(Failure::from(reason));
    }
    if let Some(reason) = filtered_files(runner)? {
        return Err(Failure::from(reason));
    }
    unshown_files(runner)?.map_or(Ok(()), |reason| Err(Failure::from(reason)))
}

/// Runs cargo-mutants on what changed since the merge base ([`Action::RustMutantsOfChanges`](crate::steps::Action::RustMutantsOfChanges)):
/// the changes of the tracked files, committed or not, and every new Rust file the repository
/// does not ignore; nothing when nothing changed.
///
/// A change to the unit tests of a source adds that source whole ([`declaring_sources`]); a
/// change to the integration tests of a crate, or to the test data it declares
/// ([`RustCrate::test_data`]), can weaken the tests of any of its code, so every source of
/// that crate is then added whole, and so is every crate a change of a shared input
/// reaches ([`reach`]); a change to what every test depends on
/// ([`MUTATE_EVERYTHING_WHEN`]) mutates the whole workspace. A valid proof record of the
/// base ([`usable_proof`]) then leaves out the crates it still proves ([`proven_crates`]),
/// each the check would otherwise have tested said: every other crate is mutated exactly as
/// without it.
pub(crate) fn rust_mutants_of_changes(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    jobs: NonZeroUsize,
) -> Result<(), Failure> {
    shown_whole(runner)?;
    let (base, against) = mutation_base(runner)?;
    runner
        .write(MEASURED_BASE, &format!("{base}\n"))
        .map_err(|error| Failure::from(format!("cannot write {MEASURED_BASE}: {error}")))?;
    let proof = usable_proof(runner, out, &base)?;
    let new = new_rust_files(runner)?;
    let tracked = changed_since(runner, &base)?;
    let changed: Vec<String> = tracked.iter().chain(&new).cloned().collect();
    let measured = format!("{base} (merge base with {against})");
    if changed.is_empty() {
        return say(
            out,
            &format!("  no change since {measured}: no mutant to test"),
        );
    }
    say(out, &format!("  the changes since {measured}"))?;
    // What every crate builds, read once, for the run and the record alike.
    let builds = if proof.is_some() || needs_closures(changed.iter().map(String::as_str)) {
        crate_builds(&crates_metadata(runner)?)?
    } else {
        Builds::default()
    };
    let proven = match &proof {
        Some(record) => proven_crates(runner, out, record, &builds)?,
        None => Vec::new(),
    };
    let reach = reach(runner, &base, changed.iter().map(String::as_str), &builds);
    let scoped = in_scope(&changed, &reach);
    // Each proven crate the check would otherwise have tested is named.
    let skipped: Vec<&RustCrate> = scoped
        .iter()
        .map(|(krate, _)| krate)
        .filter(|krate| proven.contains(krate))
        .collect();
    if let Reach::Everything { path, unnarrowed } = &reach {
        let but = if skipped.is_empty() {
            ""
        } else {
            " but the crates proven below"
        };
        let why = unnarrowed_text(unnarrowed.as_deref());
        say(
            out,
            &safe_message(&format!("  {path} changed{why}: every Rust mutant{but}")),
        )?;
    }
    if let Some(record) = &proof {
        for krate in skipped {
            say(
                out,
                &format!(
                    "  {}: proven by an earlier check at {}, unchanged since",
                    krate.name, record.commit
                ),
            )?;
        }
    }
    // The GPU workspace is tested whole when the changes reach its crate: cargo-mutants of
    // the root's workspace neither sees it nor takes its files.
    let gpu = scoped
        .iter()
        .any(|(krate, _)| krate.workspace == CargoWorkspace::Gpu && !proven.contains(krate));
    let root_scoped: Vec<(RustCrate, MutatedPart)> = scoped
        .into_iter()
        .filter(|(krate, _)| krate.workspace == CargoWorkspace::Root)
        .collect();
    let root_changes = RootChanges {
        base: &base,
        files: (&tracked, &new),
        everything: matches!(reach, Reach::Everything { .. }),
        proven: &proven,
    };
    root_mutants(runner, out, jobs, &root_changes, root_scoped)?;
    if gpu {
        say(
            out,
            &format!("  {GPU_WORKSPACE} reached: every mutant of the GPU workspace"),
        )?;
        gpu_mutants(runner, out, Extent::Partial)?;
    }
    Ok(())
}

/// What [`root_mutants`] mutates: the changes since `base` (the tracked and the new files),
/// whether they reach every crate, and the crates proven.
struct RootChanges<'changes> {
    /// The merge base.
    base: &'changes str,
    /// The tracked files changed and the new Rust files.
    files: (&'changes [String], &'changes [String]),
    /// Whether the changes reach every crate.
    everything: bool,
    /// The crates an earlier check proved.
    proven: &'changes [RustCrate],
}

/// The mutants of the root's workspace the changes reach ([`rust_mutants_of_changes`]):
/// every one when they reach every crate and none is proven, else those of `scoped` (the
/// crates of the root's workspace in scope), through the diff written to [`CHANGES_DIFF`];
/// nothing when that diff is empty.
///
/// # Errors
///
/// Returns why the diff cannot be made or written, or a surviving mutant.
fn root_mutants(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    jobs: NonZeroUsize,
    changes: &RootChanges<'_>,
    scoped: Vec<(RustCrate, MutatedPart)>,
) -> Result<(), Failure> {
    let root = CargoWorkspace::Root;
    if changes.everything && changes.proven.is_empty() {
        let every = tested_mutants_command(&["--workspace"], jobs);
        return mutants_with_doc_tests(runner, out, &every, root);
    }
    let mut tested: Vec<(RustCrate, Option<String>)> = Vec::new();
    let mut unit_tests: Vec<(RustCrate, Vec<String>)> = Vec::new();
    for (krate, scope) in scoped
        .into_iter()
        .filter(|(krate, _)| !changes.proven.contains(krate))
    {
        match scope {
            MutatedPart::Lines => {}
            MutatedPart::Whole(said) => tested.push((krate, said)),
            MutatedPart::UnitTests(tests) => unit_tests.push((krate, tests)),
        }
    }
    // Git also leaves out the crates of the other workspaces, which the root's cargo-mutants
    // does not see, as it leaves out the crates proven.
    let left_out: Vec<RustCrate> = changes
        .proven
        .iter()
        .copied()
        .chain(other_workspaces_crates())
        .collect();
    let diff = mutated_diff(
        runner,
        out,
        changes.base,
        changes.files,
        tested,
        unit_tests,
        &left_out,
    )?;
    if diff.is_empty() {
        let outside = if changes.proven.is_empty() {
            ""
        } else {
            " outside the crates proven"
        };
        return say(
            out,
            &format!("  no Rust file changed{outside}: no mutant to test"),
        );
    }
    runner
        .write(CHANGES_DIFF, &diff)
        .map_err(|error| Failure::from(format!("cannot write {CHANGES_DIFF}: {error}")))?;
    mutants_with_doc_tests(runner, out, &changes_mutants_command(jobs), root)
}

/// Runs cargo-mutants on the Rust lines changed since the merge base of `HEAD` with `since`
/// only ([`Action::RustMutantsOfChangedLines`](crate::steps::Action::RustMutantsOfChangedLines)):
/// the changes of the tracked files, committed or not, and every new Rust file git does not
/// ignore, the root's lines never widened, and with no proof record; nothing when no Rust
/// line changed. The root's cargo-mutants does not see the GPU workspace: a change there is
/// widened to every mutant of that workspace, run after the root's lines (its lines never
/// mutated alone), and adds its
/// unviable mutants to the root's report for the review gate ([`gpu_mutants`]).
/// Before the run, it says the merge base (its short hash and subject) and how many mutants
/// cargo-mutants will test ([`changes_list_command`]).
///
/// For development, between two default checks, where the default check may widen to a
/// whole source or crate (measured in `crates/xtask/README.md`). The revision is named,
/// never the upstream's default of [`MUTATION_BASES`]: on a branch not pushed, or one
/// branched from another branch not merged, that default mutates the work of others.
///
/// # Errors
///
/// Returns why the changes cannot be read, described, listed or mutated ([`shown_whole`],
/// no merge base, [`mutated_diff`]), why the diff cannot be written, or a surviving mutant.
pub(crate) fn rust_mutants_of_changed_lines(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    jobs: NonZeroUsize,
    since: &Revision,
) -> Result<(), Failure> {
    shown_whole(runner)?;
    let base = git_output(
        runner,
        &["merge-base", "HEAD", &since.commit],
        &format!("find the merge base of HEAD with {}", since.name),
    )?
    .trim()
    .to_owned();
    let described = git_output(
        runner,
        &["show", "-s", "--format=%h %s", &base],
        "describe the merge base",
    )?;
    let measured = format!(
        "{} (merge base of HEAD with {})",
        described.trim(),
        since.name
    );
    let new = new_rust_files(runner)?;
    let tracked = changed_since(runner, &base)?;
    // The root's cargo-mutants does not see the other workspaces' crates: git leaves them
    // out of its diff, and a change of the GPU workspace mutates that workspace whole.
    let others = other_workspaces_crates();
    let gpu = others
        .iter()
        .filter(|krate| krate.workspace == CargoWorkspace::Gpu)
        .any(|krate| {
            tracked
                .iter()
                .chain(&new)
                .any(|path| krate.holds(path).is_some())
        });
    let diff = mutated_diff(
        runner,
        out,
        &base,
        (&tracked, &new),
        Vec::new(),
        Vec::new(),
        &others,
    )?;
    if diff.is_empty() {
        if !gpu {
            return say(
                out,
                &format!("  no Rust line changed since {measured}: no mutant to test"),
            );
        }
        say(
            out,
            &format!("  no Rust line of the root's workspace changed since {measured}"),
        )?;
    } else {
        runner
            .write(CHANGES_DIFF, &diff)
            .map_err(|error| Failure::from(format!("cannot write {CHANGES_DIFF}: {error}")))?;
        let listed = runner
            .capture(&changes_list_command())
            .map_err(|failure| Failure::from(format!("cannot list the mutants: {failure}")))?;
        let count = listed.lines().count();
        let mutants = if count == 1 { "mutant" } else { "mutants" };
        say(
            out,
            &format!(
                "  the Rust lines changed since {measured}, not widened: {count} {mutants} to \
                 run"
            ),
        )?;
        mutants_with_doc_tests(
            runner,
            out,
            &changes_mutants_command(jobs),
            CargoWorkspace::Root,
        )?;
    }
    if gpu {
        say(
            out,
            &format!(
                "  {GPU_WORKSPACE} changed: every mutant of the GPU workspace, its lines not \
                 mutated alone"
            ),
        )?;
        gpu_mutants(runner, out, Extent::Partial)?;
    }
    Ok(())
}

/// The crates of every workspace but the root's: cargo-mutants of the root's workspace
/// neither sees them nor takes their files, so git leaves them out of its diff.
fn other_workspaces_crates() -> Vec<RustCrate> {
    RUST_CRATES
        .into_iter()
        .filter(|krate| krate.workspace != CargoWorkspace::Root)
        .collect()
}

/// The first path of `changed` that selects `part` of `cargo xtask check --native`, if any:
/// one that `reach` widens to one of its crates ([`NativePart::crates`]), every crate for
/// [`Reach::Everything`], or an input of one of them ([`inputs_changed`]: a file of a crate
/// it builds, `closures`, or of its declared test data).
pub(crate) fn native_reached(
    part: NativePart,
    changed: &[String],
    reach: &Reach,
    closures: &Closures,
) -> Option<String> {
    let widened = match reach {
        Reach::Everything { path, .. } => return Some(path.clone()),
        Reach::Crates(widened) => widened,
    };
    changed
        .iter()
        .find(|path| {
            RUST_CRATES
                .iter()
                .filter(|krate| part.crates().contains(&krate.name))
                .any(|krate| {
                    widened.get(krate.name) == Some(*path) || inputs_changed(krate, path, closures)
                })
        })
        .cloned()
}

/// The parts of `cargo xtask check --native` the changes since the merge base with `main`
/// ([`main_base`]) reach ([`native_reached`]), the untracked files included, each part said
/// on `out` with the path that selects it, or as not run.
///
/// # Errors
///
/// Returns why the base, the changes, the untracked files or the crates cannot be read, or
/// stdout cannot be written.
pub(crate) fn native_parts(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
) -> Result<Vec<NativePart>, Failure> {
    let (base, against) = main_base(runner, "the changes of the native check")?;
    let mut changed = changed_since(runner, &base)?;
    changed.extend(untracked_changes(runner)?);
    let builds = crate_builds(&crates_metadata(runner)?)?;
    let reach = reach(runner, &base, changed.iter().map(String::as_str), &builds);
    say(
        out,
        &format!("  the changes since {base} (merge base with {against})"),
    )?;
    let mut parts = Vec::new();
    for part in NativePart::ALL {
        let name = part.name();
        match native_reached(part, &changed, &reach, &builds.closures) {
            Some(path) => {
                say(
                    out,
                    &safe_message(&format!("  {path} changed: the {name} part runs")),
                )?;
                parts.push(part);
            }
            None => say(
                out,
                &format!("  no change reaches the {name} part: not run"),
            )?,
        }
    }
    Ok(parts)
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
