//! The math layer gate: which crates may compute in floating point, and how.

use crate::{
    changes::unit_test_file,
    rust_source::{RustSource, TokenKind},
    steps::{Failure, step_verdict, verdict},
    workspace::{CrateRole, RUST_CRATES, RustCrate, crates_in},
};

/// The directory of box code, as a literal: [`BOX_CODE`], and the start of its pathspec.
macro_rules! box_code {
    () => {
        "crates/audio-boxes/src/"
    };
}

/// The directory of the engine core's code, as a literal: [`ENGINE_CODE`], and the start of
/// its pathspec.
macro_rules! engine_code {
    () => {
        "crates/audio-core/src/"
    };
}

/// The sources the math layer gate reads, as git pathspecs, their `*_tests.rs` files apart.
///
/// Box code ([`BOX_CODE`]) and the engine core's ([`ENGINE_CODE`]), built from the same
/// literals, and the root of the visual boxes ([`VISUAL_BOX_ROOT`]), read for its forbid
/// alone.
pub const MATH_LAYER_SOURCES: [&str; 3] = [
    concat!(box_code!(), "*.rs"),
    concat!(engine_code!(), "*.rs"),
    VISUAL_BOX_ROOT,
];

/// The directory of box code: the sources of the box crate, `audio-boxes`.
pub const BOX_CODE: &str = box_code!();

/// The directory of the engine core's code, `audio-core`.
pub const ENGINE_CODE: &str = engine_code!();

/// The math layer itself (`spec/engine-design.md`, decisions 10 and 28): the one file of
/// the engine core where samples are summed and computed on, which the gate does not read.
pub const MATH_LAYER_FILE: &str = "crates/audio-core/src/math.rs";

/// The names of the iterator reductions written only in the math layer
/// (`spec/engine-design.md`, decision 28).
///
/// Neither box code nor the engine core may write them outside it: the iterator methods
/// that accumulate, and the traits of `sum` and `product`, so that no iterator sums samples
/// but `ordered_sum`, in the reduction order. (The sample types implement neither trait, so
/// `sum` and `product` of samples do not compile anyway.) A loop or a chain of `+` is not a
/// name: those are reviewed.
pub const REDUCTION_NAMES: [&str; 10] = [
    "sum",
    "product",
    "fold",
    "rfold",
    "try_fold",
    "try_rfold",
    "reduce",
    "scan",
    "Sum",
    "Product",
];

/// The names the engine core may not write outside the math layer, but in its leading
/// inner attribute [`ENGINE_ARITHMETIC_FORBID`].
///
/// Clippy's `arithmetic_side_effects`, without whose exception Clippy refuses arithmetic on
/// a generic sample type there (`Binary64` and `Binary32` are allowed by `clippy.toml`),
/// its former name, and the group and lint that would lift it unnamed. The compiler
/// refuses any lifting in a module forbidding it (E0453), whatever name it uses; these
/// names are refused for the crate root, which cannot forbid it.
pub const ENGINE_FORBIDDEN_NAMES: [&str; 4] = [
    "arithmetic_side_effects",
    "integer_arithmetic",
    "restriction",
    "blanket_clippy_restriction_lints",
];

/// The leading inner attribute of every module of the engine core but its root and the
/// math layer.
///
/// Outside tests, the compiler then refuses any lifting of `arithmetic_side_effects` there,
/// by any name (E0453).
pub const ENGINE_ARITHMETIC_FORBID: &str =
    "#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]";

/// The leading inner attribute of the root of each box crate ([`BOX_ROOT`], [`VISUAL_BOX_ROOT`]): outside tests,
/// box code computes no float.
///
/// Clippy's `float_arithmetic`, which the compiler then refuses to lift by any name
/// (E0453). The box tests compute their references in raw floats.
pub const BOX_FLOAT_FORBID: &str = "#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]";

/// The root of the box crate, which holds [`BOX_FLOAT_FORBID`].
pub const BOX_ROOT: &str = "crates/audio-boxes/src/lib.rs";

/// The root of the visual boxes, which holds [`BOX_FLOAT_FORBID`].
///
/// A visual box computes its coverage in integers, exact on every target (`spec/visual-design.md`), and turns it into
/// values through the visual core alone. The math layer's other rules are the audio
/// engine's, so the gate reads nothing else of it.
pub const VISUAL_BOX_ROOT: &str = "crates/visual-boxes/src/lib.rs";

/// The files the gate names for a rule of their own.
///
/// The roots of both libraries' box crates, the engine core's root and the math layer. Each
/// must be among the sources it reads ([`named_sources_gate`]): one moved or renamed would
/// leave its rule holding for nothing.
pub const MATH_LAYER_NAMED: [&str; 4] = [BOX_ROOT, VISUAL_BOX_ROOT, ENGINE_ROOT, MATH_LAYER_FILE];

/// The root of the engine core, which names none of [`ENGINE_FORBIDDEN_NAMES`] instead of
/// forbidding the lint, so that the math layer may lift it.
pub const ENGINE_ROOT: &str = "crates/audio-core/src/lib.rs";

/// The names box code may not write (`spec/engine-design.md`, decision 10).
///
/// The raw float types, whose operators and methods bypass the math layer, `libm`, and
/// `host`, the module of the engine that reads and writes the floats of samples, with the
/// name of its delivery of samples to a host and of that delivery's function (a box naming
/// them could take a sample's float out with the type inferred, `f64` never written).
pub const BOX_FORBIDDEN_NAMES: [&str; 6] = ["f32", "f64", "libm", "host", "Delivered", "deliver"];

/// The packages of `metadata` (the output of [`metadata_command`](crate::dependencies::metadata_command)), each with its name.
///
/// # Errors
///
/// Returns why the metadata cannot be read, and refuses a package without a name instead
/// of reading it as `""`.
pub(crate) fn cargo_packages(metadata: &str) -> Result<Vec<(String, serde_json::Value)>, Failure> {
    let json: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
    json.get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::from("cargo metadata lists no packages".to_owned()))?
        .iter()
        .map(|package| {
            package
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(|name| (name.to_owned(), package.clone()))
                .ok_or_else(|| {
                    Failure::from("cargo metadata lists a package without a name".to_owned())
                })
        })
        .collect()
}

/// The workspace root `metadata` (the output of [`metadata_command`](crate::dependencies::metadata_command)) names, from which it
/// writes every path.
///
/// # Errors
///
/// Returns why the metadata cannot be read, or that it names no root.
pub(crate) fn workspace_root(metadata: &str) -> Result<String, Failure> {
    let json: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| Failure::from(format!("invalid cargo metadata: {error}")))?;
    json.get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Failure::from("cargo metadata names no workspace root".to_owned()))
}

/// The crate of [`RUST_CRATES`] that `dependency`, a dependency of a package of `cargo
/// metadata`, is: one with a `path`, named as the crate and at the crate's directory below
/// `root`, the workspace root. The one rule of the gates and the mutation proof for "a
/// crate of the workspace": a path dependency named like a crate but placed elsewhere is
/// none.
///
/// # Errors
///
/// Returns that the dependency has no name, which cargo always writes.
pub(crate) fn workspace_dependency(
    dependency: &serde_json::Value,
    root: &str,
) -> Result<Option<&'static RustCrate>, Failure> {
    let name = dependency
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            Failure::from("cargo metadata lists a dependency without a name".to_owned())
        })?;
    let path = dependency.get("path").and_then(serde_json::Value::as_str);
    Ok(RUST_CRATES
        .iter()
        .find(|krate| krate.name == name && path == Some(format!("{root}/{}", krate.dir).as_str())))
}

/// The dependencies `package`, a package of `cargo metadata`, lists.
pub(crate) fn dependencies_of(
    package: &serde_json::Value,
) -> impl Iterator<Item = &serde_json::Value> {
    package
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
}

/// What the math layer gate finds in the crates (`metadata`, the output of
/// [`metadata_command`](crate::dependencies::metadata_command)): a dependency on `libm` outside [`CrateRole::Libm`] (no crate has a
/// build script: [`TARGET_KINDS`](crate::test_code::TARGET_KINDS)).
///
/// # Errors
///
/// Returns why the metadata cannot be read.
pub(crate) fn math_layer_crates(metadata: &str) -> Result<Vec<String>, Failure> {
    let mut problems = Vec::new();
    for (name, krate) in cargo_packages(metadata)? {
        let name = name.as_str();
        let reaches_libm = krate
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|dependency| {
                let text = |key: &str| dependency.get(key).and_then(serde_json::Value::as_str);
                text("name") == Some("libm") && text("kind") != Some("dev")
            });
        if reaches_libm && !crates_in(CrateRole::Libm).contains(&name) {
            problems.push(format!(
                "{name} depends on libm: its arithmetic must go through the math layer of \
                 audio-core"
            ));
        }
    }
    Ok(problems)
}

/// Checks that every file of [`MATH_LAYER_NAMED`] is among `sources`, the files
/// [`math_layer_gate`] reads: a rule the gate keeps for a file it no longer finds would hold
/// for nothing, and the gate pass.
///
/// # Errors
///
/// Returns every named file not among `sources`.
pub fn named_sources_gate(sources: &[RustSource]) -> Result<(), Failure> {
    let problems = MATH_LAYER_NAMED
        .iter()
        .filter(|named| !sources.iter().any(|source| source.path == **named))
        .map(|named| {
            format!(
                "{named}: the math layer gate names it but does not read it: moved or removed, \
                 its rule would hold for nothing; name the file where it is"
            )
        })
        .collect();
    step_verdict(problems)
}

/// Checks that box arithmetic can only go through the math layer, and that no code but the
/// math layer's reduces through an iterator.
///
/// No crate but those of [`CrateRole::Libm`] depends on `libm`, as a normal or a build
/// dependency, under any name it is renamed to (no crate has a build script:
/// [`TARGET_KINDS`](crate::test_code::TARGET_KINDS)); box code names none of [`BOX_FORBIDDEN_NAMES`], nor a float literal
/// suffixed `f32` or `f64`, and its root holds [`BOX_FLOAT_FORBID`]; box code and the
/// engine core outside [`MATH_LAYER_FILE`] name none of [`REDUCTION_NAMES`]; the engine core
/// outside it names none of [`ENGINE_FORBIDDEN_NAMES`] but in its leading inner attribute
/// [`ENGINE_ARITHMETIC_FORBID`], which every module of it but [`ENGINE_ROOT`] holds; and no
/// source names `include` (`include!`).
///
/// `metadata` is the output of [`metadata_command`](crate::dependencies::metadata_command); `sources` the files of
/// [`MATH_LAYER_SOURCES`] (read into their tokens), of which the `*_tests.rs` files are test
/// code, not read (the test code gate, [`test_code_gate`](crate::test_code::test_code_gate), proves each declared once beside
/// its source, and the loaded-files gate, [`loaded_files_gate`](crate::loaded_files::loaded_files_gate), that no build of the code
/// reads one, both steps of every check), and [`MATH_LAYER_FILE`] is the math layer, read
/// only for `include`; [`VISUAL_BOX_ROOT`] is read for its forbid alone; any other file is
/// refused. So code reaches these crates only
/// through modules of their `src/`, from files the gate reads (the loaded-files gate refuses
/// a build of the code reading any other file); a box crate,
/// `no_std` and without `libm`, computes with the engine's sample types alone, whose
/// operators and math functions are the math layer's; and no iterator sums samples but the
/// math layer's `ordered_sum`. A loop or a chain of `+` that sums is reviewed.
///
/// # Errors
///
/// Returns every other crate that depends on `libm`, every forbidden
/// name with its file and line, every missing forbid, every file it does not read, or why
/// the metadata cannot be read.
pub fn math_layer_gate(metadata: &str, sources: &[RustSource]) -> Result<(), Failure> {
    let mut problems = math_layer_crates(metadata)?;
    for source in sources {
        let path = &source.path;
        if unit_test_file(path) {
            continue;
        }
        if path == VISUAL_BOX_ROOT {
            if source.leading_attribute(BOX_FLOAT_FORBID).is_none() {
                problems.push(format!("{path} does not hold `{BOX_FLOAT_FORBID}`"));
            }
            continue;
        }
        let is_box = path.starts_with(BOX_CODE);
        if !is_box && !path.starts_with(ENGINE_CODE) {
            problems.push(format!("{path}: not a source the math layer gate reads"));
            continue;
        }
        let layer = path == MATH_LAYER_FILE;
        let required = if path == BOX_ROOT {
            Some(BOX_FLOAT_FORBID)
        } else if is_box || layer || path == ENGINE_ROOT {
            None
        } else {
            Some(ENGINE_ARITHMETIC_FORBID)
        };
        let forbid = required.and_then(|forbid| source.leading_attribute(forbid));
        if let Some(missing) = required.filter(|_| forbid.is_none()) {
            problems.push(format!("{path} does not hold `{missing}`"));
        }
        for (at, token) in source.tokens.iter().enumerate() {
            let (line, word) = (token.line, token.text.as_str());
            if token.is_ident("include") {
                problems.push(format!(
                    "{path}:{line}: names `{word}` outside a test declaration: code comes \
                     into these crates by `mod` alone, from files the gate reads"
                ));
            }
            if layer {
                continue;
            }
            let named = |names: &[&str]| token.kind == TokenKind::Ident && names.contains(&word);
            let suffixed = token.kind == TokenKind::Literal
                && word.starts_with(|first: char| first.is_ascii_digit())
                && (word.ends_with("f32") || word.ends_with("f64"));
            if is_box && (suffixed || named(&BOX_FORBIDDEN_NAMES)) {
                problems.push(format!(
                    "{path}:{line}: box code names `{word}`: it computes with the engine's \
                     sample types, through the math layer"
                ));
            }
            if named(&REDUCTION_NAMES) {
                problems.push(format!(
                    "{path}:{line}: names `{word}`, the name of an iterator reduction, which \
                     only the math layer writes: an iterator sums samples only through \
                     ordered_sum, in the reduction order"
                ));
            }
            let in_forbid = forbid.as_ref().is_some_and(|range| range.contains(&at));
            if !is_box && named(&ENGINE_FORBIDDEN_NAMES) && !in_forbid {
                problems.push(format!(
                    "{path}:{line}: engine code names `{word}`: it does no arithmetic on a \
                     sample type outside the math layer"
                ));
            }
        }
    }
    verdict(
        &format!(
            "box arithmetic only through the math layer ({} only may depend on libm; box code \
             names no raw float, libm, host or delivery, and computes no float; no iterator \
             reduction outside the math layer; code comes in by `mod` alone)",
            crates_in(CrateRole::Libm).join(", ")
        ),
        problems,
    )
}

#[cfg(test)]
#[path = "math_layer_tests.rs"]
mod tests;
