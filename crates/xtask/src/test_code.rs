//! The gates that keep test code, test targets and test data where the rules that tell them apart look.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;

use crate::{
    changes::{in_tests_dir, rust_test_code, unit_test_file},
    math_layer::{BOX_FLOAT_FORBID, ENGINE_ARITHMETIC_FORBID, cargo_packages, workspace_root},
    release::CARGO_TOML,
    rust_source::{RustSource, Token, TokenKind},
    steps::{Failure, step_verdict},
    unsafety::{
        DOC_TESTS_FORBID_UNSAFE_CODE, TEST_DECLARATION_FORM, TestDeclaration, test_declarations,
    },
    workspace::{RUST_CRATES, RustCrate, UNVIABLE_REVIEWED},
};

/// Checks that every function a review of an unviable mutant names exists, and that its
/// reason names tests.
///
/// A review cites the test that kills its mutant's viable variant by its name, a fact the
/// test keeps: every word of `reviews` ([`UNVIABLE_REVIEWED`], its comments apart) written
/// like a function's name of three parts or more (lowercase letters and digits, joined by
/// `_`) is the name of a `fn` in the tokens of `sources` (the tracked Rust files); after the
/// line's ` -- ` (its reason), of a `fn` of test code (a crate's `tests/` or `*_tests.rs`
/// files, the generic test bodies a macro instantiates included). So a test renamed or
/// removed leaves no review citing it, and a reason cannot cite a function that tests
/// nothing. Names of two parts are not checked.
///
/// # Errors
///
/// Returns every such word that names no function, or in a reason no function of test
/// code, with its line.
pub fn unviable_names_gate(reviews: &str, sources: &[RustSource]) -> Result<(), Failure> {
    let mut functions = BTreeSet::new();
    let mut tests = BTreeSet::new();
    for source in sources {
        let test_code =
            in_crate(&source.path).is_some_and(|(_, relative)| rust_test_code(relative));
        // The token after `fn` is the function's name, or the `(` of a function pointer
        // type, which no review's word can be.
        let named = source.tokens.iter().zip(source.tokens.iter().skip(1));
        for (_, name) in named.filter(|(keyword, _)| keyword.is_ident("fn")) {
            let _new = functions.insert(name.text.as_str());
            if test_code {
                let _test = tests.insert(name.text.as_str());
            }
        }
    }
    let names = |text: &'_ str| -> Vec<String> {
        text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .filter(|word| {
                word.starts_with(|first: char| first.is_ascii_lowercase())
                    && !word.chars().any(|character| character.is_ascii_uppercase())
                    && word.split('_').filter(|part| !part.is_empty()).count() >= 3
            })
            .map(str::to_owned)
            .collect()
    };
    let mut problems = Vec::new();
    for (index, line) in reviews.lines().enumerate() {
        if line.starts_with('#') {
            continue;
        }
        let at = index.saturating_add(1);
        let (mutant, reason) = line.split_once(" -- ").unwrap_or((line, ""));
        for name in names(mutant) {
            if !functions.contains(name.as_str()) {
                problems.push(format!(
                    "{UNVIABLE_REVIEWED}:{at}: names `{name}`, which no Rust source defines as a \
                     function: a review names the test that kills its viable variant as it is now"
                ));
            }
        }
        for name in names(reason) {
            if !functions.contains(name.as_str()) {
                problems.push(format!(
                    "{UNVIABLE_REVIEWED}:{at}: names `{name}`, which no Rust source defines as a \
                     function: a review names the test that kills its viable variant as it is now"
                ));
            } else if !tests.contains(name.as_str()) {
                problems.push(format!(
                    "{UNVIABLE_REVIEWED}:{at}: names `{name}` as its reason, which no test code \
                     defines as a function: a review's reason names the test that kills its \
                     viable variant"
                ));
            }
        }
    }
    step_verdict(problems)
}

/// The attributes outside test code that may name `test`: the lint forbids that apply
/// outside tests, and the forbid of unsafe code in doc tests.
///
/// Each only among a file's leading inner attributes (`RustSource::leading_attribute`),
/// besides a test declaration ([`test_declarations`]).
pub const NOT_TEST_ATTRIBUTES: [&str; 3] = [
    ENGINE_ARITHMETIC_FORBID,
    BOX_FLOAT_FORBID,
    DOC_TESTS_FORBID_UNSAFE_CODE,
];

/// The crate of [`RUST_CRATES`] holding `path`, and the path relative to its directory.
pub(crate) fn in_crate(path: &str) -> Option<(&'static RustCrate, &str)> {
    RUST_CRATES
        .iter()
        .find_map(|krate| krate.holds(path).map(|relative| (krate, relative)))
}

/// Checks that Rust test code lives only under a crate's `tests/` or in its `*_tests.rs`
/// files, each declared once in the one form ([`test_declarations`]).
///
/// So every rule that tells test code by its path (the mutation widening, the math layer
/// gate) sees all of it.
///
/// `sources` are the Rust files, read into their tokens ([`RustSource`]). In a crate of
/// [`RUST_CRATES`], outside `tests/` and the `*_tests.rs` files, the identifier `test`
/// (`#[cfg(test)]`, `#[test]`, `cfg_attr(test, …)`, `cfg!(test)`, `mod test`…) appears only
/// in a test declaration or in a leading inner attribute of [`NOT_TEST_ATTRIBUTES`], and
/// `include` (`include!`, which brings in code no gate reads by its path) nowhere. A
/// `*_tests.rs` file brings in no other file (a module without a body, `include!`), which
/// no rule would tell as test code: its modules are written inline. What a `path` attribute
/// or a macro of another file loads is no word of a file: [`loaded_files_gate`](crate::loaded_files::loaded_files_gate) sees it,
/// from what rustc read for every configuration the check builds, and refuses a file
/// outside its crate (whose changes would not widen this crate), one that is not `.rs`
/// (which no gate reads), one named as test code that a build without tests reads, and one
/// only tests read not named as test code; the configurations its doc lists are refused at
/// their source, and the rest is stated there. **Review only**: what a doc test loads
/// (`include!`, `#[path]`, `include_str!` in a code block of a doc comment), which is prose
/// to every gate and which no build the loaded-files gate reads compiles; the compiler
/// refuses unsafe code there ([`DOC_TESTS_FORBID_UNSAFE_CODE`]).
///
/// # Errors
///
/// Returns every other `test` or `include` with its file and line, every file a
/// `*_tests.rs` file would load, and every `*_tests.rs` file declared other than once.
pub fn test_code_gate(sources: &[RustSource]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut declared: BTreeMap<String, usize> = BTreeMap::new();
    let crate_sources = sources.iter().filter(|source| {
        in_crate(&source.path).is_some_and(|(_, relative)| !in_tests_dir(relative))
    });
    for source in crate_sources.clone() {
        let path = &source.path;
        if unit_test_file(path) {
            problems.extend(loaded_by_tests(source));
            continue;
        }
        let declarations = test_declarations(source);
        let exempt: Vec<core::ops::Range<usize>> = declarations
            .iter()
            .map(|declaration| declaration.tokens.clone())
            .chain(
                NOT_TEST_ATTRIBUTES
                    .into_iter()
                    .filter_map(|attribute| source.leading_attribute(attribute)),
            )
            .collect();
        for TestDeclaration { file, .. } in declarations {
            let times = declared.entry(file).or_default();
            *times = times.saturating_add(1);
        }
        for (at, token) in source.tokens.iter().enumerate() {
            let line = token.line;
            if token.is_ident("test") && !exempt.iter().any(|range| range.contains(&at)) {
                problems.push(format!(
                    "{path}:{line}: names `test` outside test code: tests live in tests/ or in a \
                     *_tests.rs file declared {TEST_DECLARATION_FORM}"
                ));
            }
            if token.is_ident("include") {
                problems.push(format!(
                    "{path}:{line}: names `include`: code is brought in by `mod` only, so that \
                     its path says what it is"
                ));
            }
        }
    }
    for path in crate_sources
        .map(|source| &source.path)
        .filter(|path| unit_test_file(path))
    {
        match declared.get(path).copied().unwrap_or_default() {
            1 => {}
            0 => problems.push(format!(
                "{path}: a *_tests.rs file no file beside it declares as test code"
            )),
            times => problems.push(format!("{path}: declared as test code {times} times")),
        }
    }
    step_verdict(problems)
}

/// The problems of `source`, a `*_tests.rs` file, that would load another file: a module
/// declared without a body (the tokens `mod <identifier> ;`, the name possibly a macro
/// variable, `mod $name;`, which rustc reads from a file beside it), or the identifier
/// `include` (`include_str` and `include_bytes` are other identifiers, read as test data by
/// [`test_data_gate`]). A file a `path` attribute loads is the loaded-files gate's
/// ([`loaded_files_gate`](crate::loaded_files::loaded_files_gate)).
pub(crate) fn loaded_by_tests(source: &RustSource) -> Vec<String> {
    let path = source.path.as_str();
    let mut found: Vec<(usize, &str)> = Vec::new();
    for (at, keyword) in source.tokens.iter().enumerate() {
        // `mod`, its name (an identifier, or a macro variable `$name`), then `;`.
        let after = source
            .tokens
            .get(at.saturating_add(1)..)
            .unwrap_or_default();
        let after = match after {
            [dollar, rest @ ..] if dollar.is_punct('$') => rest,
            _ => after,
        };
        if keyword.is_ident("mod") && matches!(after, [_, end, ..] if end.is_punct(';')) {
            found.push((
                keyword.line,
                "declares a module without a body, loaded from another file",
            ));
        }
    }
    for token in &source.tokens {
        if token.is_ident("include") {
            found.push((token.line, "names `include`, which loads another file"));
        }
    }
    found.sort_by_key(|(line, _)| *line);
    found
        .into_iter()
        .map(|(line, what)| {
            format!(
                "{path}:{line}: {what}, which no rule tells as test code: a *_tests.rs file \
                 writes its modules inline"
            )
        })
        .collect()
}

/// The kinds of target whose documentation rustdoc tests.
pub const DOC_TESTED_KINDS: [&str; 2] = ["lib", "rlib"];

/// The kinds of target the workspace may hold, as cargo metadata names them.
///
/// Libraries (a C library for the WebAssembly module included), binaries and tests. Every
/// other kind is refused ([`untested_targets_gate`]): a procedural macro (`proc-macro`) or a build
/// script (`custom-build`) generates or runs code at build time that no gate reads (the
/// keyword `unsafe` in a procedural macro's expansion, which rustc does not lint, or a file
/// a build script reads), and the checks prove no other kind.
pub const TARGET_KINDS: [&str; 5] = ["lib", "rlib", "cdylib", "bin", "test"];

/// Checks that no target of the workspace leaves its tests unrun.
///
/// `metadata` is the output of [`metadata_command`](crate::dependencies::metadata_command), `sources` the Rust files (their paths
/// relative to the root, read into their tokens). Every way a manifest offers to keep
/// `cargo test` from running a target's tests is refused, but one:
/// - a target may turn its unit tests off (`test = false` in its manifest, which spares the
///   compiler and the linker a test harness at every build of a mutant; cargo's default for
///   a build script) only if its root is all its code: it declares no module (no `mod` in
///   its tokens). The test code gate allows test code outside `tests/` only in
///   `*_tests.rs` files a module declaration brings in, so such a root holds no test that
///   would never run. A test target never may: its root's own tests are its tests;
/// - no target needs features (`required-features`), which `cargo test` skips it without;
/// - every file cargo discovers as a test target (`tests/<name>.rs`, `tests/<name>/main.rs`
///   of a package) is the root of a test target, so that `autotests = false` or a target
///   moved elsewhere leaves none of them unbuilt;
/// - a library keeps its doc tests ([`DOC_TESTED_KINDS`]);
/// - every target is of a kind of [`TARGET_KINDS`], so that the workspace holds no
///   procedural macro and no build script.
///
/// The `harness` key, which cargo metadata does not report and which can replace a
/// target's tests by its own `main`, is refused by [`bypass_gate`](crate::reviews::bypass_gate) in every manifest.
///
/// # Errors
///
/// Returns every target or file that breaks one of these, every package or target the
/// metadata does not describe as expected, or why the metadata cannot be read.
pub fn untested_targets_gate(metadata: &str, sources: &[RustSource]) -> Result<(), Failure> {
    let root = workspace_root(metadata)?;
    let below_root = |path: &str| -> Option<String> {
        path.strip_prefix(root.as_str())
            .and_then(|path| path.strip_prefix('/'))
            .map(str::to_owned)
    };
    let mut problems = Vec::new();
    for (name, package) in cargo_packages(metadata)? {
        let targets = package
            .get("targets")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| Failure::from(format!("cargo metadata lists no targets of {name}")))?;
        let Some(dir) = package
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .and_then(below_root)
            .and_then(|manifest| manifest.strip_suffix(CARGO_TOML).map(str::to_owned))
            .filter(|dir| dir.is_empty() || dir.ends_with('/'))
        else {
            problems.push(format!(
                "{name}: a package cargo metadata does not describe as expected (a \
                 `manifest_path` below the workspace root)"
            ));
            continue;
        };
        let mut test_roots = Vec::new();
        for target in targets {
            let field = |key: &str| target.get(key).and_then(serde_json::Value::as_bool);
            let kinds: Vec<&str> = target
                .get("kind")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .collect();
            let source = target
                .get("src_path")
                .and_then(serde_json::Value::as_str)
                .and_then(below_root);
            let (Some(tested), Some(doc_tested), Some(source)) =
                (field("test"), field("doctest"), source)
            else {
                problems.push(format!(
                    "{name}: a target cargo metadata does not describe as expected (`test`, \
                     `doctest`, a source below the workspace root)"
                ));
                continue;
            };
            if let Some(kind) = kinds.iter().find(|kind| !TARGET_KINDS.contains(kind)) {
                problems.push(format!(
                    "{source}: a `{kind}` target, a kind outside `TARGET_KINDS`: the checks \
                     prove only those (what a procedural macro or a build script generates or \
                     runs at build time no gate reads)"
                ));
                continue;
            }
            let test_kind = kinds.contains(&"test");
            if test_kind && tested {
                test_roots.push(source.clone());
            }
            if target
                .get("required-features")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|features| !features.is_empty())
            {
                problems.push(format!(
                    "{source}: its target needs features (`required-features`), without \
                     which `cargo test` skips it: its tests would never run"
                ));
            }
            if !doc_tested && kinds.iter().any(|kind| DOC_TESTED_KINDS.contains(kind)) {
                problems.push(format!(
                    "{source}: its doc tests are turned off (`doctest = false`): they would \
                     never run"
                ));
            }
            if tested {
                continue;
            }
            if test_kind {
                problems.push(format!(
                    "{source}: a test target that turns its tests off (`test = false`): they \
                     would never run"
                ));
                continue;
            }
            untested_root(&source, sources, &mut problems);
        }
        undiscovered_tests(&dir, sources, &test_roots, &mut problems);
    }
    step_verdict(problems)
}

/// The problems of the files of `sources` cargo discovers as test targets of the package
/// in `dir` (`""` or a path ending in `/`: `tests/<name>.rs`, `tests/<name>/main.rs`) that
/// are not among `test_roots`, the roots of its tested test targets
/// ([`untested_targets_gate`]).
pub(crate) fn undiscovered_tests(
    dir: &str,
    sources: &[RustSource],
    test_roots: &[String],
    problems: &mut Vec<String>,
) {
    for RustSource { path, .. } in sources {
        let discovered = path
            .strip_prefix(dir)
            .and_then(|rest| rest.strip_prefix("tests/"))
            .is_some_and(|rest| match rest.split_once('/') {
                None => rest
                    .rsplit_once('.')
                    .is_some_and(|(_, extension)| extension == "rs"),
                Some((_, file)) => file == "main.rs",
            });
        if discovered && !test_roots.contains(path) {
            problems.push(format!(
                "{path}: a test file cargo discovers, but no tested test target's root \
                 (`autotests = false`, a target moved or turned off): its tests would never \
                 run"
            ));
        }
    }
}

/// The problems of `source`, the root of a target that turns its tests off: it must be
/// among `sources` and declare no module ([`untested_targets_gate`]).
pub(crate) fn untested_root(source: &str, sources: &[RustSource], problems: &mut Vec<String>) {
    match sources.iter().find(|held| held.path == source) {
        None => problems.push(format!(
            "{source}: the root of a target without tests, not among the Rust files read"
        )),
        Some(held) => problems.extend(
            held.tokens
                .iter()
                .filter(|token| token.is_ident("mod"))
                .map(|token| {
                    format!(
                        "{source}:{}: declares a module, but its target turns its tests off \
                         (`test = false`): only a root that is all its code may, so that no \
                         test hides in it",
                        token.line
                    )
                }),
        ),
    }
}

/// The repository path that `relative` names from the directory `dir` (both relative to
/// the root, `..` and `.` resolved), or `None` when it leaves the repository.
pub(crate) fn resolve(dir: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                let _parent = parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// Whether `path`, a file or directory of the repository, lies under `root`: a file or a
/// directory ending in `/`.
pub(crate) fn under_root(path: &str, root: &str) -> bool {
    root.strip_suffix('/')
        .map_or_else(|| path == root, |dir| path == dir || path.starts_with(root))
}

/// The identifiers [`test_data_gate`] follows: the forms that read a file or name a
/// directory of the repository.
pub(crate) const TEST_DATA_WORDS: [&str; 6] = [
    "include_str",
    "include_bytes",
    "env",
    "option_env",
    "var",
    "var_os",
];

/// The variables a crate may read with `env!` that name no file of the repository: those
/// Cargo sets for the crate itself, by their prefix.
pub(crate) const CRATE_VARIABLES: [&str; 2] = ["CARGO_BIN_EXE_", "CARGO_PKG_"];

/// What an identifier of [`TEST_DATA_WORDS`] reads, by the tokens that follow it
/// ([`test_data_read`]).
pub(crate) enum Read<'text> {
    /// A file relative to the file that reads it, by its literal if it has one.
    Literal(Option<&'text str>),
    /// A file relative to the crate's directory, by its literal if it has one.
    Joined(Option<&'text str>),
    /// No file of the repository.
    Other,
    /// A form the gate cannot follow, and why.
    Refused(String),
}

/// The text of `token` when it is a plain string literal: quoted, without a prefix and
/// without an escape, so that its text is the string itself.
pub(crate) fn plain_string(token: &Token) -> Option<&str> {
    // Only a literal's text starts and ends with `"`.
    let text = token.text.strip_prefix('"')?.strip_suffix('"')?;
    (!text.contains('\\')).then_some(text)
}

/// The string of `tokens` when they open with `( "<plain string>" )`: the one argument of a
/// macro call, a plain string literal ([`plain_string`]).
pub(crate) fn literal_argument(tokens: &[Token]) -> Option<&str> {
    match tokens {
        [open, literal, close, ..] if open.opens('(') && close.kind == TokenKind::Close => {
            plain_string(literal)
        }
        _ => None,
    }
}

/// What `tokens[at]`, an identifier of [`TEST_DATA_WORDS`], reads, from the tokens after it.
pub(crate) fn test_data_read(tokens: &[Token], at: usize) -> Read<'_> {
    let word = tokens.get(at).map_or("", |token| token.text.as_str());
    let after = |offset: usize| tokens.get(at.saturating_add(offset)..).unwrap_or_default();
    let called = after(1).first().is_some_and(|bang| bang.is_punct('!'));
    match word {
        "include_str" | "include_bytes" => {
            Read::Literal(literal_argument(after(2)).filter(|_| called))
        }
        "env" if !called => Read::Other,
        "env" => match literal_argument(after(2)) {
            Some("CARGO_MANIFEST_DIR") => manifest_join(after(5)),
            Some(name)
                if CRATE_VARIABLES
                    .iter()
                    .any(|prefix| name.starts_with(prefix)) =>
            {
                Read::Other
            }
            _ => Read::Refused(
                "`env!` of a variable that may name a directory of the repository".to_owned(),
            ),
        },
        "option_env" => {
            Read::Refused("`option_env!`, which may name a directory of the repository".to_owned())
        }
        _ => variable_read(word, after(1)),
    }
}

/// What `env!("CARGO_MANIFEST_DIR")` reads, `tokens` being those after it: in a path's
/// constructor (its `)`), then one `.join("<literal>")`, and no further `.join` or
/// `.parent` ([`test_data_gate`]).
pub(crate) fn manifest_join(tokens: &[Token]) -> Read<'_> {
    let joined = match tokens {
        [close, dot, join, rest @ ..]
            if close.kind == TokenKind::Close && dot.is_punct('.') && join.is_ident("join") =>
        {
            literal_argument(rest).map(|literal| (literal, rest.get(3..).unwrap_or_default()))
        }
        _ => None,
    };
    match joined {
        Some((_, [dot, next, ..]))
            if dot.is_punct('.') && (next.is_ident("join") || next.is_ident("parent")) =>
        {
            Read::Refused(
                "a path joined or left again after the one `.join(\"<literal>\")` the gate \
                 follows"
                    .to_owned(),
            )
        }
        _ => Read::Joined(joined.map(|(literal, _)| literal)),
    }
}

/// What `word`, `var`, `var_os` or another identifier of [`TEST_DATA_WORDS`] not called as
/// a macro, reads, `tokens` being those after it: a call whose first argument is a plain
/// string literal naming one of the variables Cargo sets for the crate's manifest
/// (`CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH` …) is refused. A turbofish, a raw string or
/// an escape naming that variable is outside the gate's scope: only a deliberate evasion
/// writes it (`CLAUDE.md`, "A gate guards against honest mistakes").
pub(crate) fn variable_read<'text>(word: &str, tokens: &'text [Token]) -> Read<'text> {
    // A string's text is quoted; any other first argument names its variable at run time.
    let argument = match tokens {
        [open, first, ..] if open.opens('(') => first.text.as_str(),
        _ => return Read::Other,
    };
    if argument
        .strip_prefix('"')
        .is_some_and(|name| name.starts_with("CARGO_MANIFEST_"))
    {
        Read::Refused(format!(
            "`{word}` of a variable naming the crate's manifest, a directory read at run time \
             the gate cannot follow"
        ))
    } else {
        Read::Other
    }
}

/// Checks that the tests of every crate read nothing of the repository outside the crate's
/// directory but its declared test data ([`RustCrate::test_data`]).
///
/// So a change to that data widens the default check's mutation testing to the crate.
///
/// In the tokens of every Rust file of a crate (`sources`), each `include_str!` and
/// `include_bytes!` takes a literal path, a plain string (no escape, not raw), relative to
/// its file, and each
/// `env!("CARGO_MANIFEST_DIR")` is the argument of a path's constructor followed by one
/// `.join("<literal>")`, relative to the crate, and by no other `.join` or `.parent` (the
/// forms that name a file of the repository); each must lie in the crate or under one of
/// its roots, and each root must be read so. Only test code (`tests/`, `*_tests.rs`) reads
/// outside its crate: a library's read would be an input of every crate whose tests build
/// it, which no crate declares. The other ways to name the crate's directory are refused:
/// `option_env!`, `env!` of any variable but `CARGO_MANIFEST_DIR` and those Cargo sets for
/// a crate (`CARGO_BIN_EXE_<name>`, `CARGO_PKG_<field>`), and `var` or `var_os` of a plain
/// string literal naming a variable of the crate's manifest (`CARGO_MANIFEST_DIR`,
/// `CARGO_MANIFEST_PATH`); a turbofish, a raw string or an escape naming it is outside the
/// gate's scope, written only on purpose. **Review only**: any path computed at run time,
/// which no text gate tells from any other value: a relative path given to a file API
/// (cargo runs the tests in the crate's directory), the current directory, the executable's
/// path,
/// `file!()`, a variable of the environment read by a name not written as a literal or
/// through `vars()`, and a path held in a variable, then joined again (xtask's fixtures,
/// below its `tests/fixtures/`). A binary's entry point (`src/main.rs` of a
/// crate with [`RustCrate::binary`]) is not read: it only wires the real process, finding
/// the repository it runs in, and its tests give it their own files.
///
/// # Errors
///
/// Returns every read outside the declared data or from code other than tests, every form
/// without its literal or that the gate cannot follow, and every root no test reads.
pub fn test_data_gate(sources: &[RustSource]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut read: BTreeSet<(&str, &str)> = BTreeSet::new();
    for source in sources {
        let path = &source.path;
        let Some((krate, relative)) = in_crate(path) else {
            continue;
        };
        if krate.binary && relative == "src/main.rs" {
            continue;
        }
        let file_dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        for (at, token) in source.tokens.iter().enumerate() {
            let (line, word) = (token.line, token.text.as_str());
            // Only an identifier's text is a bare word: a literal's is quoted or a number.
            if !TEST_DATA_WORDS.contains(&word) {
                continue;
            }
            let (from, literal) = match test_data_read(&source.tokens, at) {
                Read::Literal(literal) => (file_dir, literal),
                Read::Joined(literal) => (krate.dir, literal),
                Read::Other => continue,
                Read::Refused(why) => {
                    problems.push(format!("{path}:{line}: {why}"));
                    continue;
                }
            };
            let Some(literal) = literal else {
                problems.push(format!(
                    "{path}:{line}: `{word}!` without a literal path the gate can follow"
                ));
                continue;
            };
            let Some(target) = resolve(from, literal) else {
                problems.push(format!("{path}:{line}: `{literal}` leaves the repository"));
                continue;
            };
            if krate.holds(&target).is_some() {
                continue;
            }
            if !rust_test_code(relative) {
                problems.push(format!(
                    "{path}:{line}: code that is not a test reads `{target}`, outside its \
                     crate: only tests read test data"
                ));
                continue;
            }
            match krate
                .test_data
                .iter()
                .find(|root| under_root(&target, root))
            {
                Some(root) => {
                    let _new = read.insert((krate.name, root));
                }
                None => problems.push(format!(
                    "{path}:{line}: the tests of {} read `{target}`, which is not in its \
                     test_data",
                    krate.name
                )),
            }
        }
    }
    for krate in &RUST_CRATES {
        for root in krate.test_data {
            if !read.contains(&(krate.name, *root)) {
                problems.push(format!(
                    "{}: declares `{root}` as test data, which no test of it reads",
                    krate.name
                ));
            }
        }
    }
    step_verdict(problems)
}
