//! The gates that keep test code, test targets and test data where the rules that tell them apart look.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;

use crate::{
    changes::{in_tests_dir, rust_test_code, unit_test_file},
    logical_text::{Language, code_literals, line_at},
    math_layer::{BOX_FLOAT_FORBID, ENGINE_ARITHMETIC_FORBID, cargo_packages, workspace_root},
    release::CARGO_TOML,
    rust_source::{RustSource, Token, TokenKind, close_of},
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
    review_names_gate(UNVIABLE_REVIEWED, reviews, sources)
}

/// [`unviable_names_gate`] on `reviews`, the text of `file`, any file of reviewed mutants in
/// the form of [`UNVIABLE_REVIEWED`] (`<file>.rs: <mutant> -- <reason>`, `#` comments), each
/// problem naming `file`.
///
/// # Errors
///
/// Returns every word that names no function, or in a reason no function of test code.
pub fn review_names_gate(file: &str, reviews: &str, sources: &[RustSource]) -> Result<(), Failure> {
    let (functions, tests) = defined_functions(sources);
    let names = |text: &str| function_names(text, 3);
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
                    "{file}:{at}: names `{name}`, which no Rust source defines as a \
                     function: a review names the test that kills its viable variant as it is now"
                ));
            }
        }
        for name in names(reason) {
            if !functions.contains(name.as_str()) {
                problems.push(format!(
                    "{file}:{at}: names `{name}`, which no Rust source defines as a \
                     function: a review names the test that kills its viable variant as it is now"
                ));
            } else if !tests.contains(name.as_str()) {
                problems.push(format!(
                    "{file}:{at}: names `{name}` as its reason, which no test code \
                     defines as a function: a review's reason names the test that kills its \
                     viable variant"
                ));
            }
        }
    }
    step_verdict(problems)
}

/// The functions `sources` define, and those of test code among them (a crate's `tests/`
/// or `*_tests.rs` files, the generic test bodies a macro instantiates included).
fn defined_functions(sources: &[RustSource]) -> (BTreeSet<&str>, BTreeSet<&str>) {
    let mut functions = BTreeSet::new();
    let mut tests = BTreeSet::new();
    for source in sources {
        let test_code = is_test_code(source);
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
    (functions, tests)
}

/// Whether `source` is test code: a crate's `tests/` or `*_tests.rs` files.
pub(crate) fn is_test_code(source: &RustSource) -> bool {
    is_test_path(&source.path)
}

/// Whether `path` is a file of test code of a crate of [`RUST_CRATES`].
pub(crate) fn is_test_path(path: &str) -> bool {
    in_crate(path).is_some_and(|(_, relative)| rust_test_code(relative))
}

/// Whether `tokens` hold a `#[test]` attribute at `at`: `#`, `[` and `test`, which only
/// that attribute's closing `]` follows in code that compiles.
fn test_attribute(tokens: &[Token], at: usize) -> bool {
    matches!(
        tokens.get(at..).unwrap_or_default(),
        [hash, open, word, ..] if hash.is_punct('#') && open.opens('[') && word.is_ident("test")
    )
}

/// The index past the group `tokens` opens at `open` (an [`TokenKind::Open`] token), its
/// closing delimiter included ([`close_of`]), or the end.
fn past_group(tokens: &[Token], open: usize) -> usize {
    close_of(tokens, open).saturating_add(1).min(tokens.len())
}

/// The tests of `sources`: in test code, every `fn` under a `#[test]` attribute (other
/// attributes may stand between them), and the generic bodies a test instantiates, each a
/// `fn` of test code named either with a turbofish (`name::<…>`) in the body of such a
/// test, or among the arguments of a macro whose definition (`macro_rules!`, in test code)
/// holds `#[test]`, as one that writes a test for each precision of a generic body.
///
/// A helper of test code, which no test instantiates, is no test.
fn defined_tests(sources: &[RustSource]) -> BTreeSet<&str> {
    let test_code: Vec<&RustSource> = sources
        .iter()
        .filter(|source| is_test_code(source))
        .collect();
    let mut tests = BTreeSet::new();
    let mut generic = BTreeSet::new();
    let mut macros = BTreeSet::new();
    for source in &test_code {
        let tokens = &source.tokens;
        let mut attributed = false;
        for at in 0..tokens.len() {
            match tokens.get(at..).unwrap_or_default() {
                _ if test_attribute(tokens, at) => attributed = true,
                [keyword, name, ..] if keyword.is_ident("fn") => {
                    if attributed {
                        let _test = tests.insert(name.text.as_str());
                        let body = (at..tokens.len())
                            .find(|&open| tokens.get(open).is_some_and(|token| token.opens('{')))
                            .unwrap_or(tokens.len());
                        generic.extend((body..past_group(tokens, body)).filter_map(|inner| {
                            match tokens.get(inner..).unwrap_or_default() {
                                [called, first, second, generics, ..]
                                    if first.is_punct(':')
                                        && second.is_punct(':')
                                        && generics.is_punct('<') =>
                                {
                                    Some(called.text.as_str())
                                }
                                _ => None,
                            }
                        }));
                    }
                    attributed = false;
                }
                // `macro_rules`, `!`, its name and its body, in code that compiles.
                [keyword, _, name, ..] if keyword.is_ident("macro_rules") => {
                    let start = at.saturating_add(3);
                    if (start..past_group(tokens, start)).any(|inner| test_attribute(tokens, inner))
                    {
                        let _macro = macros.insert(name.text.as_str());
                    }
                }
                _ => {}
            }
        }
    }
    for source in &test_code {
        let tokens = &source.tokens;
        for at in 0..tokens.len() {
            // A test macro's name and `!`, then its arguments: a group in code that compiles.
            if let [name, bang, ..] = tokens.get(at..).unwrap_or_default()
                && bang.is_punct('!')
                && macros.contains(name.text.as_str())
            {
                let start = at.saturating_add(2);
                generic.extend(
                    tokens
                        .get(start..past_group(tokens, start))
                        .unwrap_or(&[])
                        .iter()
                        .map(|token| token.text.as_str()),
                );
            }
        }
    }
    let (_, of_test_code) = defined_functions(sources);
    tests.extend(generic.intersection(&of_test_code));
    tests
}

/// The words of `text` written like a function's name of `parts` parts or more: lowercase
/// letters and digits, joined by `_`.
fn function_names(text: &str, parts: usize) -> Vec<String> {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|word| {
            word.starts_with(|first: char| first.is_ascii_lowercase())
                && !word.chars().any(|character| character.is_ascii_uppercase())
                && word.split('_').filter(|part| !part.is_empty()).count() >= parts
        })
        .map(str::to_owned)
        .collect()
}

/// Checks that the reason of every review of an unviable mutant added since the last
/// version names a test.
///
/// A review's reason ([`UNVIABLE_REVIEWED`], after the line's ` -- `) names the test that
/// kills its mutant's viable variant, so that a reason cannot rest on an argument no test
/// holds: at least one of its words written like a function's name of three parts or more
/// is a test of `sources` (`defined_tests`: a `#[test]` function, or a generic body a test
/// instantiates; never a helper of test code), [`unviable_names_gate`] holding every such
/// word to an existing function. Only the reviews `released` (the file at the merge base with
/// `main`, the last version's) does not hold, line for line, are read: those older,
/// reasoned before the rule, are reworded at #24 step 166.
///
/// # Errors
///
/// Returns every such review whose reason names no test, with its line.
pub fn unviable_reasons_gate(
    reviews: &str,
    released: &str,
    sources: &[RustSource],
) -> Result<(), Failure> {
    review_reasons_gate(UNVIABLE_REVIEWED, reviews, released, sources)
}

/// [`unviable_reasons_gate`] on `reviews`, the text of `file`, any file of reviewed mutants
/// in the form of [`UNVIABLE_REVIEWED`], each problem naming `file`.
///
/// # Errors
///
/// Returns every review not in `released` whose reason names no test, with its line.
pub fn review_reasons_gate(
    file: &str,
    reviews: &str,
    released: &str,
    sources: &[RustSource],
) -> Result<(), Failure> {
    let tests = defined_tests(sources);
    let released: BTreeSet<&str> = released.lines().collect();
    let problems = reviews
        .lines()
        .zip(1_usize..)
        .filter(|(line, _)| {
            !line.starts_with('#') && !line.trim().is_empty() && !released.contains(line)
        })
        .filter(|(line, _)| {
            let reason = line.split_once(" -- ").map_or("", |(_, reason)| reason);
            !function_names(reason, 3)
                .iter()
                .any(|name| tests.contains(name.as_str()))
        })
        .map(|(_, at)| {
            format!(
                "{file}:{at}: its reason names no test: a review added since the \
                 last version names the test that kills its mutant's viable variant"
            )
        })
        .collect();
    step_verdict(problems)
}

/// Checks that every test a specification names exists.
///
/// In `specs` (the tracked `spec/*.md` files, path and text), a code span that is one name
/// written like a function's of four parts or more (`holds_the_ceiling_exactly`) names a
/// `fn` of `sources`, and one ending `_*` (`holds_the_ceiling_with_no_slack_*`), the
/// prefix of one: so a test renamed or removed leaves no specification citing it. Names
/// of three parts or fewer (fields, keys, lints) and spans holding anything else (paths,
/// `::`, prose) are not read.
///
/// # Errors
///
/// Returns every such name no function holds, with its file and line.
pub fn spec_names_gate(specs: &[(String, String)], sources: &[RustSource]) -> Result<(), Failure> {
    let (functions, _) = defined_functions(sources);
    let mut problems = Vec::new();
    for (path, text) in specs {
        for (line, at) in text.lines().zip(1_usize..) {
            for span in line.split('`').skip(1).step_by(2) {
                let (name, family) = span
                    .strip_suffix('*')
                    .map_or((span, false), |prefix| (prefix, true));
                if function_names(name, 4).first().map(String::as_str) != Some(name) {
                    continue;
                }
                let held = functions.iter().any(|function| {
                    if family {
                        function.starts_with(name)
                    } else {
                        *function == name
                    }
                });
                if !held {
                    problems.push(format!(
                        "{path}:{at}: names `{span}`, which no Rust source defines as a \
                         function: a specification names a test as it is now, or says what it \
                         measured without the name"
                    ));
                }
            }
        }
    }
    step_verdict(problems)
}

/// The index of the specifications, whose table of the files only a tool writes names the
/// test of each reader ([`reader_tests_gate`]).
pub const SPEC_INDEX: &str = "spec/README.md";

/// The heading of [`SPEC_INDEX`] that opens the table of the files only a tool writes.
pub const TOOL_FILES_HEADING: &str = "## Files only a tool writes";

/// The writers outside the project whose files [`TOOL_FILES_HEADING`] does not hold: a test
/// that reads a file only as one of them writes it is not named there.
pub const FOREIGN_WRITERS: [&str; 1] = ["cargo"];

/// Whether `name`, a test's title in words, says that a file is read only as its tool writes
/// it: "only as", then a writer not in [`FOREIGN_WRITERS`], then "writes it" or "writes
/// them".
fn says_only_as_written(name: &str) -> bool {
    name.split_once("only as ").is_some_and(|(_, writer)| {
        let foreign = writer
            .split(' ')
            .next()
            .is_some_and(|first| FOREIGN_WRITERS.contains(&first));
        !foreign && (writer.contains(" writes it") || writer.contains(" writes them"))
    })
}

/// Checks that the index of the specifications names the test of every reader of a file
/// only a tool writes.
///
/// A reader's test says so in its name: "only as … writes it" or "writes them", in words in
/// the title of a TypeScript test, joined by `_` in the name of a Rust one. Each such test
/// of `sources` (a `#[test]` function or a generic body one instantiates) is named in code
/// quotes, and each such title of `scripts` (the TypeScript test files, path and text) whole
/// and in double quotes, by a row of the table under [`TOOL_FILES_HEADING`] of `index`
/// ([`SPEC_INDEX`]'s text); a file another project's tool writes ([`FOREIGN_WRITERS`]) has no
/// row. And every text in double quotes of that table is a string of `scripts`, so that a
/// title renamed or removed is seen there as a Rust name is ([`spec_names_gate`]). A reader
/// whose test says otherwise is not seen (the whole register: #24 step 169).
///
/// # Errors
///
/// Returns every such test the table does not name, with its file and, for a title, its
/// line, and every title the table names that no TypeScript test holds, with its line.
pub fn reader_tests_gate(
    index: &str,
    sources: &[RustSource],
    scripts: &[(String, String)],
) -> Result<(), Failure> {
    let rows: Vec<(usize, &str)> = (1_usize..)
        .zip(index.lines())
        .skip_while(|(_, line)| *line != TOOL_FILES_HEADING)
        .skip(1)
        .take_while(|(_, line)| !line.starts_with("## "))
        .filter(|(_, line)| line.starts_with('|'))
        .collect();
    let between = |mark: char| -> Vec<(usize, &str)> {
        rows.iter()
            .flat_map(|(at, row)| row.split(mark).skip(1).step_by(2).map(|text| (*at, text)))
            .collect()
    };
    let names = between('`');
    let titles = between('"');
    let mut problems: Vec<String> = defined_tests(sources)
        .into_iter()
        .filter(|name| says_only_as_written(&name.replace('_', " ")))
        .filter(|name| !names.iter().any(|(_, named)| named == name))
        .map(|name| {
            format!(
                "{SPEC_INDEX}: `{TOOL_FILES_HEADING}` does not name `{name}`, the test of a \
                 reader of a file only a tool writes: name it in that file's row"
            )
        })
        .collect();
    let literals: Vec<(String, &str, usize)> = scripts
        .iter()
        .flat_map(|(path, text)| {
            code_literals(text, Language::Script)
                .into_iter()
                .map(move |(words, spans)| (words, path.as_str(), line_at(&spans, 0)))
        })
        .collect();
    problems.extend(
        literals
            .iter()
            .filter(|(words, ..)| says_only_as_written(words))
            .filter(|(words, ..)| !titles.iter().any(|(_, title)| title == words))
            .map(|(words, path, line)| {
                format!(
                    "{path}:{line}: `{TOOL_FILES_HEADING}` of {SPEC_INDEX} does not name \
                     \"{words}\", the test of a reader of a file only a tool writes: name it in \
                     that file's row"
                )
            }),
    );
    problems.extend(
        titles
            .iter()
            .filter(|(_, title)| !literals.iter().any(|(words, ..)| words == title))
            .map(|(at, title)| {
                format!(
                    "{SPEC_INDEX}:{at}: names \"{title}\", which no TypeScript test holds: name \
                     the test as it is now"
                )
            }),
    );
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

/// Whether `source`, of the crate in `dir`, reads [`SAFE_VECTORS`] in a form
/// [`test_data_gate`] follows ([`test_data_read`]).
fn reads_safe_vectors(source: &RustSource, dir: &str) -> bool {
    let file_dir = source
        .path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    source.tokens.iter().enumerate().any(|(at, token)| {
        let read = if TEST_DATA_WORDS.contains(&token.text.as_str()) {
            test_data_read(&source.tokens, at)
        } else {
            Read::Other
        };
        let target = match read {
            Read::Literal(Some(literal)) => resolve(file_dir, literal),
            Read::Joined(Some(literal)) => resolve(dir, literal),
            Read::Literal(None) | Read::Joined(None) | Read::Other | Read::Refused(_) => None,
        };
        target.as_deref() == Some(SAFE_VECTORS)
    })
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

/// The variables a crate may read with `option_env!`, each naming no file of the repository:
/// the engine block a build names (`audio_core::NAMED_ENGINE_BLOCK`, a decimal number its
/// reader refuses unless it is one of the engine blocks offered).
pub(crate) const BUILD_VARIABLES: [&str; 1] = ["POYETIK_ENGINE_BLOCK"];

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
        "option_env" => match literal_argument(after(2)).filter(|_| called) {
            Some(name) if BUILD_VARIABLES.contains(&name) => Read::Other,
            _ => Read::Refused(
                "`option_env!`, which may name a directory of the repository".to_owned(),
            ),
        },
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
/// its roots, and each root must be read so by its tests, a test's read in the crate under a
/// root counting as a read of that root (a crate may declare its own files, as the
/// loaded-files gate asks of a file that is not Rust a build of its tests loads), never a
/// read by the crate's other code. Only test code (`tests/`, `*_tests.rs`) reads
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
    crates_test_data_gate(sources, &RUST_CRATES)
}

/// [`test_data_gate`] over the crates `crates`, so that its tests can declare test data no
/// crate of the workspace declares.
pub(crate) fn crates_test_data_gate(
    sources: &[RustSource],
    crates: &[RustCrate],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut read: BTreeSet<(&str, &str)> = BTreeSet::new();
    for source in sources {
        let path = &source.path;
        let Some((krate, relative)) = crates
            .iter()
            .find_map(|krate| krate.holds(path).map(|relative| (krate, relative)))
        else {
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
            let root = krate
                .test_data
                .iter()
                .find(|root| under_root(&target, root));
            if krate.holds(&target).is_some() {
                if let Some(root) = root.filter(|_| rust_test_code(relative)) {
                    let _new = read.insert((krate.name, root));
                }
                continue;
            }
            if !rust_test_code(relative) {
                problems.push(format!(
                    "{path}:{line}: code that is not a test reads `{target}`, outside its \
                     crate: only tests read test data"
                ));
                continue;
            }
            match root {
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
    for krate in crates {
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

/// The shared known-answer vectors of how every tool escapes and bounds untrusted text.
pub const SAFE_VECTORS: &str = "spec/safe-message-vectors.json";

/// The binary crates whose tests read no [`SAFE_VECTORS`], each by its name with why.
///
/// A tool that prints no untrusted text, or escapes it through a crate whose tests read them.
pub const ESCAPED_ELSEWHERE: [(&str, &str); 1] = [(
    "audio-cli",
    "escapes every line through `audio_patch::safe_message`, whose own tests read the vectors",
)];

/// Requires the test code of every binary crate of `crates`, a command-line tool, to read
/// [`SAFE_VECTORS`], but those of `elsewhere` ([`RUST_CRATES`], [`ESCAPED_ELSEWHERE`]).
///
/// So a tool that prints untrusted text cannot escape it by a rule of its own. A test that
/// reads them declares them as test data ([`test_data_gate`]).
///
/// # Errors
///
/// Returns every binary crate that does neither, and every entry of `elsewhere` that is no
/// binary crate, or whose tests read the vectors.
pub fn safe_vectors_gate(
    sources: &[RustSource],
    crates: &[RustCrate],
    elsewhere: &[(&str, &str)],
) -> Result<(), Failure> {
    let reads = |krate: &RustCrate| {
        sources
            .iter()
            .filter(|source| krate.holds(&source.path).is_some_and(rust_test_code))
            .any(|source| reads_safe_vectors(source, krate.dir))
    };
    let mut problems: Vec<String> = crates
        .iter()
        .filter(|krate| krate.binary && !reads(krate))
        .filter(|krate| !elsewhere.iter().any(|&(name, _)| name == krate.name))
        .map(|krate| {
            format!(
                "{}: a tool whose tests read no {SAFE_VECTORS}: escape and bound the untrusted \
                 text it prints by the shared rule and test it against the vectors, or list it \
                 in ESCAPED_ELSEWHERE with why (crates/xtask/src/test_code.rs)",
                krate.name
            )
        })
        .collect();
    problems.extend(
        elsewhere
            .iter()
            .filter(|&&(name, _)| {
                !crates
                    .iter()
                    .any(|krate| krate.name == name && krate.binary && !reads(krate))
            })
            .map(|(name, _)| {
                format!(
                    "{name}: listed in ESCAPED_ELSEWHERE, but no binary crate whose \
                     tests read no {SAFE_VECTORS}: remove it (crates/xtask/src/test_code.rs)"
                )
            }),
    );
    step_verdict(problems)
}

/// The lines of the documentation `token` holds, at `index` of `tokens`, each with the line
/// it is on: a doc comment's text without its opening (`///`, `//!`, `/**`, `/*!`) and a
/// block's closing `*/`, each line of a block without its leading `*`; or the string of a
/// `#[doc = "…"]` or `#![doc = "…"]` attribute starting there, split at its line breaks
/// (written or escaped). `None` for any other token, and for a `doc` attribute whose value
/// is not a string literal (a macro's output, which no source of the repository writes: a
/// doc test it brings is review only, as what a doc test loads).
fn doc_lines(tokens: &[Token], index: usize) -> Option<Vec<(usize, String)>> {
    let token = tokens.get(index)?;
    if matches!(token.kind, TokenKind::Doc { .. }) {
        let text = token.text.get(3..).unwrap_or_default();
        let block = token.text.starts_with("/*");
        let text = if block {
            text.strip_suffix("*/").unwrap_or(text)
        } else {
            text
        };
        return Some(
            (token.line..)
                .zip(text.split('\n'))
                .map(|(line, written)| {
                    let trimmed = written.trim_start();
                    let shown = if block {
                        trimmed.strip_prefix('*').unwrap_or(trimmed)
                    } else {
                        written
                    };
                    (line, shown.to_owned())
                })
                .collect(),
        );
    }
    let bang = usize::from(tokens.get(index.saturating_add(1))?.is_punct('!'));
    let rest = tokens.get(index.saturating_add(1).saturating_add(bang)..)?;
    match rest {
        [open, name, equals, literal, ..]
            if token.is_punct('#')
                && open.opens('[')
                && name.is_ident("doc")
                && equals.is_punct('=')
                && literal.kind == TokenKind::Literal =>
        {
            let raw = literal.text.trim_start_matches('r').trim_matches('#');
            let value = raw.strip_prefix('"')?.strip_suffix('"')?;
            Some(
                value
                    .split('\n')
                    .flat_map(|line| line.split("\\n"))
                    .map(|line| (literal.line, line.to_owned()))
                    .collect(),
            )
        }
        _ => None,
    }
}

/// The fence that opens or closes a code block of Markdown on `line` (three backticks or
/// tildes or more, after spaces), with what follows it.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let text = line.trim_start();
    let mark = text
        .chars()
        .next()
        .filter(|mark| matches!(mark, '`' | '~'))?;
    let length = text
        .chars()
        .take_while(|character| *character == mark)
        .count();
    (length >= 3).then(|| (mark, length, text.get(length..).unwrap_or_default()))
}

/// Whether `word` is an error code of rustc, `E` and four digits.
fn error_code(word: &str) -> bool {
    word.strip_prefix('E').is_some_and(|digits| {
        digits.len() == 4 && digits.chars().all(|digit| digit.is_ascii_digit())
    })
}

/// Checks that every `compile_fail` doc test of `sources` (the tracked Rust files) names the
/// error code it fails with.
///
/// Rustdoc checks only the codes a `compile_fail` block lists ([`doc_tests_command`](crate::steps::doc_tests_command)):
/// one listing none passes whatever it fails with, a name renamed or a path moved, and
/// proves nothing. Read in the documentation of every source, as rustdoc reads it: each run
/// of doc comments, and each `doc` attribute of a string, as Markdown, where a fence of
/// three backticks or tildes or more opens a code block until the same fence closes it;
/// its attributes are the words of what follows the opening fence, split at commas and
/// spaces. A block whose attributes hold `compile_fail` must hold an `E` and four digits.
/// A doc test brought in from another file (`#[doc = include_str!(…)]`, which no source
/// writes) is review only, as what a doc test loads.
///
/// # Errors
///
/// Returns every `compile_fail` block that names no error code, with its file and line.
pub fn compile_fail_codes_gate(sources: &[RustSource]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for source in sources {
        let mut open: Option<(char, usize)> = None;
        for (index, token) in source.tokens.iter().enumerate() {
            let Some(lines) = doc_lines(&source.tokens, index) else {
                continue;
            };
            // A doc comment right after another continues its documentation; anything else
            // starts anew, as an item's documentation ends with it.
            let documents = |at: &Token| matches!(at.kind, TokenKind::Doc { .. });
            let continued = documents(token)
                && index
                    .checked_sub(1)
                    .and_then(|before| source.tokens.get(before))
                    .is_some_and(documents);
            if !continued {
                open = None;
            }
            for (line, text) in lines {
                match (open, fence(&text)) {
                    (None, Some((mark, length, info))) => {
                        open = Some((mark, length));
                        let words: Vec<&str> = info
                            .split(|character: char| character == ',' || character.is_whitespace())
                            .filter(|word| !word.is_empty())
                            .collect();
                        if words.contains(&"compile_fail")
                            && !words.iter().any(|word| error_code(word))
                        {
                            problems.push(format!(
                                "{}:{line}: a compile_fail doc test that names no error code \
                                 (`compile_fail,E…`), so rustdoc accepts it whatever it fails \
                                 with: name the code it fails with",
                                source.path
                            ));
                        }
                    }
                    (Some((mark, length)), Some((closing, count, rest)))
                        if closing == mark && count >= length && rest.trim().is_empty() =>
                    {
                        open = None;
                    }
                    _ => {}
                }
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "test_code_tests.rs"]
mod tests;
