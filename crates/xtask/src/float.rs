//! The float gate: the floating-point methods allowed, banned and audited.

use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;

use crate::{
    actions::{read_all, read_file},
    preflight::{RUST_TOOLCHAIN_FILE, toolchain_channel},
    runner::Runner,
    steps::{Cmd, Dir, Failure, verdict},
    workspace::NIGHTLY_TOOLCHAIN,
};

/// The Clippy configuration, whose `disallowed-methods` ban the float methods whose result
/// may differ between targets.
pub const CLIPPY_TOML: &str = "clippy.toml";

/// The sources of the inherent methods of `f32` and `f64` in the standard library.
///
/// Relative to the sysroot of the pinned nightly ([`NIGHTLY_TOOLCHAIN`]), whose `rust-src`
/// component Miri already needs: the pinned release has no sources of its own, since
/// installing them would make the compiler write their local paths into the engine's
/// module, which would no longer be reproducible. The nightly is newer than the release,
/// and each method's `#[stable(since = …)]` says whether the release has it. `core` holds
/// the methods that need no operating system, `std` those that call the platform's math
/// library.
pub const FLOAT_SOURCES: [&str; 4] = [
    "lib/rustlib/src/rust/library/core/src/num/f32.rs",
    "lib/rustlib/src/rust/library/core/src/num/f64.rs",
    "lib/rustlib/src/rust/library/std/src/num/f32.rs",
    "lib/rustlib/src/rust/library/std/src/num/f64.rs",
];

/// The macros a float's `impl` block may call among its methods, each with the source of
/// the nightly that defines it (relative to the sysroot, as [`FLOAT_SOURCES`]).
///
/// A macro called there could define methods the audit would not see, so
/// [`parse_float_methods`] refuses any other, and [`float_gate`] audits each source here:
/// it must define the macro and declare no function nor call a macro, so that the macro
/// defines only constants. `float_impl!`, shared by every float type of `core`, defines
/// their constants.
pub const FLOAT_ITEM_MACROS: [(&str, &str); 1] = [(
    "float_impl",
    "lib/rustlib/src/rust/library/core/src/num/float_macros.rs",
)];

/// The stable float methods reviewed as giving the same result on every target.
///
/// For `f32` and `f64` alike, with the reason. Every other stable method of the pinned toolchain
/// must be banned in [`CLIPPY_TOML`] ([`float_gate`]).
pub const DETERMINISTIC_FLOAT_METHODS: [(&str, &str); 32] = [
    ("abs", "clears the sign bit"),
    ("ceil", "exact: an integer value, rounded toward +infinity"),
    ("classify", "reads the bits"),
    (
        "copysign",
        "copies a sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (#12 step 9, the output stage)",
    ),
    (
        "div_euclid",
        "a correctly rounded division, then exact rounding to an integer value",
    ),
    ("floor", "exact: an integer value, rounded toward -infinity"),
    ("fract", "exact: the value less its integer part"),
    ("from_be_bytes", "a bit copy in a fixed byte order"),
    ("from_bits", "a bit copy"),
    ("from_le_bytes", "a bit copy in a fixed byte order"),
    ("is_finite", "reads the bits"),
    ("is_infinite", "reads the bits"),
    ("is_nan", "reads the bits"),
    ("is_normal", "reads the bits"),
    (
        "is_sign_negative",
        "reads the sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (#12 step 9, the output stage)",
    ),
    (
        "is_sign_positive",
        "reads the sign bit; a NaN's bits, which arithmetic may set differently per target, are outside this audit (#12 step 9, the output stage)",
    ),
    ("is_subnormal", "reads the bits"),
    (
        "midpoint",
        "correctly rounded additions and exact halvings, in a fixed order",
    ),
    (
        "next_down",
        "the next value toward -infinity, from the bits (IEEE 754 nextDown)",
    ),
    (
        "next_up",
        "the next value toward +infinity, from the bits (IEEE 754 nextUp)",
    ),
    ("recip", "a correctly rounded division"),
    (
        "rem_euclid",
        "an exact remainder, then a correctly rounded addition",
    ),
    ("round", "exact: an integer value, ties away from zero"),
    ("round_ties_even", "exact: an integer value, ties to even"),
    ("signum", "a constant of the sign of the value"),
    ("sqrt", "correctly rounded by IEEE 754 on every target"),
    ("to_be_bytes", "a bit copy in a fixed byte order"),
    (
        "to_bits",
        "a bit copy; a NaN's bits, which arithmetic may set differently per target, are outside this audit (#12 step 9, the output stage)",
    ),
    (
        "to_int_unchecked",
        "an unsafe function: every crate forbids unsafe code, so none can call it",
    ),
    ("to_le_bytes", "a bit copy in a fixed byte order"),
    ("total_cmp", "orders the bits"),
    ("trunc", "exact: an integer value, rounded toward zero"),
];

/// A release number, `X.Y.Z`, ordered as releases are.
pub type Release = (u32, u32, u32);

/// Reads a release number written `X.Y.Z`, nothing else.
#[must_use]
pub fn parse_release(text: &str) -> Option<Release> {
    let mut parts = text.split('.').map(|part| {
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse::<u32>().ok())
            .flatten()
    });
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => {
            Some((major, minor, patch))
        }
        _ => None,
    }
}

/// One inherent method of `f32` or `f64` found in the standard library's sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatMethod {
    /// `f32` or `f64`.
    pub float: String,
    /// The method's name.
    pub name: String,
    /// The release that stabilized it; `None` when it is unstable, or stabilized by the
    /// nightly only (`since = "CURRENT_RUSTC_VERSION"`).
    pub since: Option<Release>,
    /// Whether its documentation says its result may differ ("non-deterministic",
    /// "Unspecified precision").
    pub documented_varying: bool,
}

/// Reads the inherent methods of `f32` or `f64` from one source of the standard library.
///
/// The single `impl f32 {` or `impl f64 {` block at the start of a line, to its closing `}`
/// at the start of a line; each method is a line `    pub [const] [unsafe] fn <name>`,
/// after its documentation and its `#[stable(…, since = "X.Y.Z")]` (or
/// `"CURRENT_RUSTC_VERSION"`) or `#[unstable(` attribute. A line indented by fewer than
/// eight spaces is among the block's items, not in a method's body: a macro called there
/// could define methods, so it must be one of [`FLOAT_ITEM_MACROS`], whose sources
/// [`float_gate`] audits.
///
/// # Errors
///
/// Returns what is not in that shape: no such block or more than one, a block never closed,
/// a line declaring a function that is not read as a method, a method without a stability
/// attribute, a `since` of another shape, a macro called among the items that is not
/// reviewed, or no method at all. Nothing is skipped.
pub fn parse_float_methods(path: &str, source: &str) -> Result<Vec<FloatMethod>, String> {
    let lines: Vec<&str> = source.lines().collect();
    let starts: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match *line {
            "impl f32 {" => Some((index, "f32")),
            "impl f64 {" => Some((index, "f64")),
            _ => None,
        })
        .collect();
    let [(start, float)] = starts.as_slice() else {
        return Err(format!(
            "{path}: {} `impl f32 {{` or `impl f64 {{` blocks, expected one",
            starts.len()
        ));
    };
    let body = lines.get(start.saturating_add(1)..).unwrap_or_default();
    let Some(end) = body.iter().position(|line| *line == "}") else {
        return Err(format!(
            "{path}: the `impl {float} {{` block is never closed"
        ));
    };
    let mut methods = Vec::new();
    let mut doc = String::new();
    let mut stability: Option<Option<Release>> = None;
    for line in body.iter().take(end) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[stable(") {
            let since = trimmed
                .split_once("since = \"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(since, _)| since);
            stability = match since {
                Some("CURRENT_RUSTC_VERSION") => Some(None),
                Some(release) if parse_release(release).is_some() => Some(parse_release(release)),
                _ => {
                    return Err(format!(
                        "{path}: `{}` gives no release the method was stabilized in",
                        trimmed.trim_end()
                    ));
                }
            };
        } else if trimmed.starts_with("#[unstable(") {
            stability = Some(None);
        } else if trimmed.starts_with("///") {
            doc.push_str(trimmed);
            doc.push('\n');
        }
        let item = !line.starts_with("        ");
        let unreviewed = macro_call(trimmed).is_some_and(|name| {
            item && !FLOAT_ITEM_MACROS
                .iter()
                .any(|(reviewed, _)| *reviewed == name)
        });
        if unreviewed {
            return Err(format!(
                "{path}: `{}` calls a macro among the items of the `impl {float} {{` block that is not reviewed as defining only constants (FLOAT_ITEM_MACROS in xtask)",
                trimmed.trim_end()
            ));
        }
        let declares = trimmed.starts_with("pub ") && trimmed.contains("fn ");
        if !declares {
            continue;
        }
        let name = line
            .strip_prefix("    pub ")
            .map(|rest| {
                rest.trim_start_matches("const ")
                    .trim_start_matches("unsafe ")
            })
            .and_then(|rest| rest.strip_prefix("fn "))
            .map(|rest| {
                rest.chars()
                    .take_while(|letter| {
                        letter.is_ascii_lowercase() || letter.is_ascii_digit() || *letter == '_'
                    })
                    .collect::<String>()
            })
            .filter(|name| !name.is_empty());
        let (Some(name), Some(since)) = (name, stability) else {
            return Err(format!(
                "{path}: `{}` is not a method of the expected shape, with its stability",
                line.trim()
            ));
        };
        let documented_varying =
            doc.contains("non-deterministic") || doc.contains("Unspecified precision");
        methods.push(FloatMethod {
            float: (*float).to_owned(),
            name,
            since,
            documented_varying,
        });
        doc.clear();
        stability = None;
    }
    if methods.is_empty() {
        return Err(format!("{path}: no method in the `impl {float} {{` block"));
    }
    Ok(methods)
}

/// The name of the macro a line calls, when it starts with one: `name!` (an empty name
/// when it starts with `!`, which is no Rust item either).
pub(crate) fn macro_call(trimmed: &str) -> Option<&str> {
    let end = trimmed.find(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_'))?;
    let (name, rest) = trimmed.split_at(end);
    rest.starts_with('!').then_some(name)
}

/// Checks that a macro of [`FLOAT_ITEM_MACROS`] defines only constants.
///
/// Its source must define it (a line holding `macro <name>(`), and no line may declare a
/// function (hold `fn ` outside a comment or an attribute, private ones included) or call a
/// macro, which could declare one.
///
/// # Errors
///
/// Returns the missing definition, or the first line that may define a method.
pub fn audit_float_macro(path: &str, source: &str, name: &str) -> Result<(), String> {
    let definition = format!("macro {name}(");
    if !source.lines().any(|line| line.contains(&definition)) {
        return Err(format!(
            "{path}: no `{definition}` definition, reviewed in FLOAT_ITEM_MACROS"
        ));
    }
    for line in source.lines().map(str::trim) {
        let declares = line.contains("fn ") && !line.starts_with("//") && !line.starts_with("#[");
        if declares || macro_call(line).is_some() {
            return Err(format!(
                "{path}: `{line}` may define a method in `{name}!`, which is reviewed as defining only constants"
            ));
        }
    }
    Ok(())
}

/// The float methods `clippy.toml` bans: every `{ path = "f32::…" }` or `{ path = "f64::…" }`
/// of its `disallowed-methods`, as `f32::name`.
pub(crate) fn banned_float_methods(clippy: &str) -> BTreeSet<String> {
    clippy
        .lines()
        .filter_map(|line| line.trim().strip_prefix("{ path = \""))
        .filter_map(|rest| rest.split('"').next())
        .filter(|path| path.starts_with("f32::") || path.starts_with("f64::"))
        .map(str::to_owned)
        .collect()
}

/// Checks that every stable float method of the pinned release is banned or reviewed.
///
/// A method must give the same result on every target or be banned: each is banned in `clippy` (the
/// text of [`CLIPPY_TOML`]) or reviewed in [`DETERMINISTIC_FLOAT_METHODS`], never both, and never
/// reviewed when its own documentation says its result may differ; every ban and every review names
/// a method that exists. So a new toolchain cannot bring a method no one classified.
///
/// `sources` holds the path and text of each file of [`FLOAT_SOURCES`] and of
/// [`FLOAT_ITEM_MACROS`] (each audited by [`audit_float_macro`]), read from a toolchain whose
/// `rustc --version` is `sources_version`; `release` is the pinned release: its stable methods are
/// those stabilized in it or before.
///
/// # Errors
///
/// Returns sources not newer than the release, every source not in the expected shape
/// ([`parse_float_methods`], [`audit_float_macro`]) or missing, and every method or entry
/// breaking a rule above.
pub fn float_gate(
    sources: &[(String, String)],
    sources_version: &str,
    release: Release,
    clippy: &str,
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let newer = sources_version
        .split_whitespace()
        .nth(1)
        .and_then(|version| version.split('-').next())
        .and_then(parse_release)
        .is_some_and(|version| version >= release);
    if !newer {
        problems.push(format!(
            "the sources come from `{}`, not from a release at least {}.{}.{}: they may lack its methods",
            sources_version.trim(),
            release.0,
            release.1,
            release.2
        ));
    }
    let mut methods = Vec::new();
    for (path, text) in sources {
        let audited = match FLOAT_ITEM_MACROS
            .iter()
            .find(|(_, source)| path.ends_with(source))
        {
            Some((name, _)) => audit_float_macro(path, text, name).map(|()| Vec::new()),
            None => parse_float_methods(path, text),
        };
        match audited {
            Ok(found) => methods.extend(found),
            Err(problem) => problems.push(problem),
        }
    }
    problems.extend(
        FLOAT_ITEM_MACROS
            .iter()
            .filter(|(_, source)| !sources.iter().any(|(path, _)| path.ends_with(source)))
            .map(|(name, source)| {
                format!("`{name}!` is reviewed as defining only constants, but its source {source} is not audited")
            }),
    );
    let banned = banned_float_methods(clippy);
    let reviewed: BTreeMap<&str, &str> = DETERMINISTIC_FLOAT_METHODS.into_iter().collect();
    let in_release = |method: &FloatMethod| method.since.is_some_and(|since| since <= release);
    for method in methods.iter().filter(|method| in_release(method)) {
        let path = format!("{}::{}", method.float, method.name);
        let is_banned = banned.contains(&path);
        let is_reviewed = reviewed.contains_key(method.name.as_str());
        if is_banned && is_reviewed {
            problems.push(format!(
                "`{path}` is both banned in {CLIPPY_TOML} and reviewed as deterministic"
            ));
        } else if !is_banned && !is_reviewed {
            problems.push(format!(
                "`{path}` is neither banned in {CLIPPY_TOML} nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)"
            ));
        } else if is_reviewed && method.documented_varying {
            problems.push(format!(
                "`{path}` is reviewed as deterministic, but its documentation says its result may differ: ban it in {CLIPPY_TOML}"
            ));
        }
    }
    let found: BTreeSet<String> = methods
        .iter()
        .map(|method| format!("{}::{}", method.float, method.name))
        .collect();
    problems.extend(
        banned
            .iter()
            .filter(|path| !found.contains(*path))
            .map(|path| {
                format!("{CLIPPY_TOML} bans `{path}`, which the pinned toolchain does not have")
            }),
    );
    problems.extend(
        reviewed
            .keys()
            .filter(|name| {
                !methods
                    .iter()
                    .any(|method| in_release(method) && method.name == **name)
            })
            .map(|name| format!("`{name}` is reviewed as deterministic, but no stable float method has that name")),
    );
    verdict("the float methods of the pinned toolchain", problems)
}

/// Reads the pinned release, the pinned nightly's float sources and `clippy.toml`, then
/// checks them.
pub(crate) fn float_methods(runner: &mut dyn Runner) -> Result<(), Failure> {
    let toolchain = read_file(runner, RUST_TOOLCHAIN_FILE)?;
    let release = toolchain_channel(&toolchain)
        .and_then(parse_release)
        .ok_or_else(|| {
            Failure::from(format!(
                "{RUST_TOOLCHAIN_FILE} must pin one release: channel = \"X.Y.Z\""
            ))
        })?;
    let mut capture = |args: &[&str]| {
        runner
            .capture(&Cmd::new("rustc", args, Dir::Root))
            .map_err(|failure| {
                Failure::from(format!("cannot find the toolchain's sources: {failure}"))
            })
    };
    let version = capture(&[NIGHTLY_TOOLCHAIN, "--version"])?;
    let sysroot = capture(&[NIGHTLY_TOOLCHAIN, "--print", "sysroot"])?;
    let paths = FLOAT_SOURCES
        .iter()
        .chain(FLOAT_ITEM_MACROS.iter().map(|(_, source)| source))
        .map(|source| format!("{}/{source}", sysroot.trim_end()))
        .collect();
    let sources = read_all(runner, paths)?;
    float_gate(
        &sources,
        &version,
        release,
        &read_file(runner, CLIPPY_TOML)?,
    )
}
