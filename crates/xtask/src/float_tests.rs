//! Tests of the float gate: the floating-point methods allowed, banned and audited.

use crate::tests::{
    FAKE_NIGHTLY, FAKE_SYSROOT, RELEASE, TOOLCHAIN, float_fixture, float_source, found, passing,
};
use crate::{Action, Step, run_steps};

#[test]
fn accepts_every_float_method_banned_or_reviewed() {
    let (sources, clippy) = float_fixture();
    assert_eq!(
        crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
        Ok(()),
        "consistent"
    );
    let methods = crate::parse_float_methods(
        "f64.rs",
        &float_source(
            "f64",
            &[
                ("abs", true, "Exact."),
                ("gamma", false, "Unspecified precision"),
                ("log2", true, "Unspecified precision"),
            ],
        ),
    )
    .expect("parsed");
    assert_eq!(
        methods,
        vec![
            crate::FloatMethod {
                float: "f64".to_owned(),
                name: "abs".to_owned(),
                since: Some((1, 0, 0)),
                documented_varying: false
            },
            crate::FloatMethod {
                float: "f64".to_owned(),
                name: "gamma".to_owned(),
                since: None,
                documented_varying: true
            },
            crate::FloatMethod {
                float: "f64".to_owned(),
                name: "log2".to_owned(),
                since: Some((1, 0, 0)),
                documented_varying: true
            },
        ],
        "every method, with its stability and documentation"
    );
    let unsafe_method = "impl f32 {\n    #[stable(feature = \"a\", since = \"1.0.0\")]\n    pub unsafe fn to_int_unchecked<Int>(self) -> Int {}\n    /// It is non-deterministic.\n    #[stable(feature = \"a\", since = \"1.0.0\")]\n    pub fn sin(self) -> f32 {}\n}\n";
    let parsed = crate::parse_float_methods("f32.rs", unsafe_method).expect("parsed");
    assert_eq!(
        parsed
            .iter()
            .map(|method| (method.name.as_str(), method.documented_varying))
            .collect::<Vec<_>>(),
        [("to_int_unchecked", false), ("sin", true)],
        "unsafe and const functions, and the documentation of each method alone"
    );
}

/// A change of a source's text.
type SourceEdit = Box<dyn Fn(String) -> String>;

/// The fixture with one change of the `core` source of `f64`: `edit` maps its text.
fn float_sources_with(edit: impl Fn(String) -> String) -> Vec<(String, String)> {
    float_fixture()
        .0
        .into_iter()
        .map(|(path, text)| {
            if path.ends_with("core/src/num/f64.rs") {
                (path, edit(text))
            } else {
                (path, text)
            }
        })
        .collect()
}

#[test]
fn refuses_a_float_method_neither_banned_nor_reviewed() {
    let (_, clippy) = float_fixture();
    let sources =
        float_sources_with(|text| text.replace("pub const fn abs(", "pub const fn lerp("));
    assert_eq!(
        crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`f64::lerp` is neither banned in clippy.toml nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)".to_owned(),
            ]
        ),
        "a new method"
    );
    // A method only `f64` lacks keeps its review current through `f32`.
    let unstable = float_sources_with(|text| {
        text.replace("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[unstable(feature = \"x\", issue = \"1\")]\n    #[inline]\n    pub const fn abs(")
    });
    assert_eq!(
        crate::float_gate(&unstable, FAKE_NIGHTLY, RELEASE, &clippy),
        Ok(()),
        "an unstable method needs no classification"
    );
}

#[test]
fn refuses_a_float_method_both_banned_and_reviewed_or_documented_as_varying() {
    let (sources, clippy) = float_fixture();
    let both = clippy.replace("]\n", "  { path = \"f64::abs\", reason = \"r\" },\n]\n");
    assert_eq!(
        crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &both),
        found(
            "the float methods of the pinned toolchain",
            &["`f64::abs` is both banned in clippy.toml and reviewed as deterministic".to_owned()]
        ),
        "both"
    );
    for warning in [
        "# Unspecified precision",
        "may return either, non-deterministically",
    ] {
        let varying = float_sources_with(|text| {
            text.replace("    /// Exact.\n    ///\n    /// ```\n    /// let x = 1.0;\n    /// ```\n    #[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn sqrt(", &format!("    /// {warning}\n    #[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn sqrt("))
        });
        assert_eq!(
            crate::float_gate(&varying, FAKE_NIGHTLY, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &["`f64::sqrt` is reviewed as deterministic, but its documentation says its result may differ: ban it in clippy.toml".to_owned()]
            ),
            "{warning}"
        );
    }
}

#[test]
fn refuses_stale_float_bans_and_reviews() {
    let (sources, clippy) = float_fixture();
    let stale = clippy.replace("]\n", "  { path = \"f64::lerp\", reason = \"r\" },\n]\n");
    assert_eq!(
        crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &stale),
        found(
            "the float methods of the pinned toolchain",
            &["clippy.toml bans `f64::lerp`, which the pinned toolchain does not have".to_owned()]
        ),
        "a stale ban"
    );
    // An unstable method can be banned: it exists.
    let unstable = clippy.replace("]\n", "  { path = \"f64::gamma\", reason = \"r\" },\n]\n");
    assert_eq!(
        crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &unstable),
        Ok(()),
        "an unstable ban"
    );
    let gone: Vec<(String, String)> = sources
        .into_iter()
        .map(|(path, text)| {
            (
                path,
                text.replace("pub const fn trunc(", "pub const fn trunk("),
            )
        })
        .collect();
    let with_trunk = clippy.replace("]\n", "  { path = \"f32::trunk\", reason = \"r\" },\n  { path = \"f64::trunk\", reason = \"r\" },\n]\n");
    assert_eq!(
        crate::float_gate(&gone, FAKE_NIGHTLY, RELEASE, &with_trunk),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`trunc` is reviewed as deterministic, but no stable float method has that name"
                    .to_owned()
            ]
        ),
        "a stale review"
    );
}

#[test]
fn reads_release_numbers() {
    assert_eq!(
        crate::parse_release("1.99.1"),
        Some((1, 99, 1)),
        "a release"
    );
    assert_eq!(
        crate::parse_release("1.101.0"),
        Some((1, 101, 0)),
        "numbers, not text"
    );
    for text in [
        "1.99", "1.99.1.0", "1.99.x", "1..1", "", "1.99.+1", "1.99. 1",
    ] {
        assert_eq!(crate::parse_release(text), None, "{text:?}");
    }
}

#[test]
fn audits_the_methods_of_the_pinned_release_only() {
    let (_, clippy) = float_fixture();
    // Stabilized after the release, or by the nightly only: not callable with the release.
    for since in ["1.100.0", "CURRENT_RUSTC_VERSION"] {
        let later = float_sources_with(|text| {
            text.replace(
                "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
                &format!("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(self) -> f64 {{\n        self\n    }}\n\n    #[stable(feature = \"lerp\", since = \"{since}\")]\n    pub const fn lerp("),
            )
        });
        assert_eq!(
            crate::float_gate(&later, FAKE_NIGHTLY, RELEASE, &clippy),
            Ok(()),
            "{since}"
        );
    }
    // Stabilized by the release itself: audited.
    let now = float_sources_with(|text| {
        text.replace(
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(self) -> f64 {\n        self\n    }\n\n    #[stable(feature = \"lerp\", since = \"1.99.1\")]\n    pub const fn lerp(",
        )
    });
    assert_eq!(
        crate::float_gate(&now, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &["`f64::lerp` is neither banned in clippy.toml nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)".to_owned()]
        ),
        "stabilized in the release"
    );
    // A reviewed method the release does not have yet is a stale review.
    let (sources, _) = float_fixture();
    let unreleased: Vec<(String, String)> = sources
        .into_iter()
        .map(|(path, text)| {
            (
                path,
                text.replace(
                    "since = \"1.0.0\")]\n    #[inline]\n    pub const fn trunc(",
                    "since = \"1.100.0\")]\n    #[inline]\n    pub const fn trunc(",
                ),
            )
        })
        .collect();
    assert_eq!(
        crate::float_gate(&unreleased, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[
                "`trunc` is reviewed as deterministic, but no stable float method has that name"
                    .to_owned()
            ]
        ),
        "not in the release yet"
    );
}

#[test]
fn refuses_sources_older_than_the_release() {
    let (sources, clippy) = float_fixture();
    for version in [
        "rustc 1.98.0-nightly (a 2026-08-01)\n",
        "rustc 1.99.0 (a 2026-09-01)\n",
        "rustc\n",
        "rustc nightly (a)\n",
    ] {
        assert_eq!(
            crate::float_gate(&sources, version, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "the sources come from `{}`, not from a release at least 1.99.1: they may lack its methods",
                    version.trim()
                )]
            ),
            "{version}"
        );
    }
    for version in [
        "rustc 1.99.1 (58cf8f9c3 2026-10-13)\n",
        "rustc 1.99.1-nightly (a 2026-10-01)\n",
    ] {
        assert_eq!(
            crate::float_gate(&sources, version, RELEASE, &clippy),
            Ok(()),
            "{version}"
        );
    }
}

#[test]
fn refuses_float_sources_it_cannot_read() {
    let (_, clippy) = float_fixture();
    let edits: [(SourceEdit, &str); 8] = [
        (
            Box::new(|text: String| {
                text.replacen(
                    "since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(",
                    "since = \"1.0\")]\n    #[inline]\n    pub const fn abs(",
                    1,
                )
            }),
            "`#[stable(feature = \"rust1\", since = \"1.0\")]` gives no release the method was stabilized in",
        ),
        (
            Box::new(|text: String| {
                text.replacen("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[stable(feature = \"rust1\")]\n    #[inline]\n    pub const fn abs(", 1)
            }),
            "`#[stable(feature = \"rust1\")]` gives no release the method was stabilized in",
        ),
        (
            Box::new(|text: String| text.replace("impl f64 {", "impl F64 {")),
            "0 `impl f32 {` or `impl f64 {` blocks, expected one",
        ),
        (
            Box::new(|text: String| format!("{text}impl f64 {{\n}}\n")),
            "2 `impl f32 {` or `impl f64 {` blocks, expected one",
        ),
        (
            Box::new(|text: String| text.replace("\n}\n\nfn outside", "\n\nfn outside")),
            "the `impl f64 {` block is never closed",
        ),
        (
            Box::new(|text: String| {
                text.replace("    pub const fn abs(", "        pub const fn abs(")
            }),
            "`pub const fn abs(self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
        (
            Box::new(|text: String| text.replace("    pub const fn abs(", "    pub const fn (")),
            "`pub const fn (self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
        (
            Box::new(|text: String| {
                text.replacen("#[stable(feature = \"rust1\", since = \"1.0.0\")]\n    #[inline]\n    pub const fn abs(", "#[inline]\n    pub const fn abs(", 1)
            }),
            "`pub const fn abs(self) -> f64 {` is not a method of the expected shape, with its stability",
        ),
    ];
    for (edit, problem) in edits {
        let sources = float_sources_with(edit);
        let path = format!("{FAKE_SYSROOT}/{}", crate::FLOAT_SOURCES[1]);
        assert_eq!(
            crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy)
                .map_err(|failure| failure.summary),
            Err(format!(
                "the float methods of the pinned toolchain: {path}: {problem}"
            )),
            "{problem}"
        );
    }
    assert_eq!(
        crate::parse_float_methods("f64.rs", "impl f64 {\n    pub const RADIX: u32 = 2;\n}\n"),
        Err("f64.rs: no method in the `impl f64 {` block".to_owned()),
        "an empty block"
    );
}

#[test]
fn refuses_a_macro_among_the_float_methods_unless_reviewed() {
    let (_, clippy) = float_fixture();
    let path = format!("{FAKE_SYSROOT}/{}", crate::FLOAT_SOURCES[1]);
    // A macro may define methods: among the items, only a reviewed one is accepted (the
    // fixture calls `float_impl!` there, and a macro inside each method's body).
    for call in ["cfg_select! {", "methods!(", "float_impl2!{", "!x {"] {
        let sources =
            float_sources_with(|text| text.replace("    float_impl! {", &format!("    {call}")));
        assert_eq!(
            crate::float_gate(&sources, FAKE_NIGHTLY, RELEASE, &clippy),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "{path}: `{call}` calls a macro among the items of the `impl f64 {{` block that is not reviewed as defining only constants (FLOAT_ITEM_MACROS in xtask)"
                )]
            ),
            "{call}"
        );
    }
}

#[test]
fn refuses_a_reviewed_float_macro_that_may_define_methods() {
    let (sources, clippy) = float_fixture();
    let (name, source) = crate::FLOAT_ITEM_MACROS[0];
    let macro_path = format!("{FAKE_SYSROOT}/{source}");
    let edited = |from: &str, to: &str| -> Vec<(String, String)> {
        sources
            .iter()
            .map(|(path, text)| {
                if *path == macro_path {
                    (path.clone(), text.replace(from, to))
                } else {
                    (path.clone(), text.clone())
                }
            })
            .collect()
    };
    let radix = "    pub const RADIX: u32 = 2;\n";
    for (line, problem) in [
        ("pub fn lerp(self) -> Self {}", "may define a method"),
        ("const fn helper() {}", "may define a method"),
        ("methods! {}", "may define a method"),
    ] {
        assert_eq!(
            crate::float_gate(
                &edited(radix, &format!("{radix}    {line}\n")),
                FAKE_NIGHTLY,
                RELEASE,
                &clippy
            ),
            found(
                "the float methods of the pinned toolchain",
                &[format!(
                    "{macro_path}: `{line}` {problem} in `{name}!`, which is reviewed as defining only constants"
                )]
            ),
            "{line}"
        );
    }
    assert_eq!(
        crate::float_gate(
            &edited("macro float_impl(", "macro float_impl2("),
            FAKE_NIGHTLY,
            RELEASE,
            &clippy
        ),
        found(
            "the float methods of the pinned toolchain",
            &[format!(
                "{macro_path}: no `macro {name}(` definition, reviewed in FLOAT_ITEM_MACROS"
            )]
        ),
        "another macro"
    );
    let unaudited: Vec<(String, String)> = sources
        .iter()
        .filter(|(path, _)| *path != macro_path)
        .cloned()
        .collect();
    assert_eq!(
        crate::float_gate(&unaudited, FAKE_NIGHTLY, RELEASE, &clippy),
        found(
            "the float methods of the pinned toolchain",
            &[format!(
                "`{name}!` is reviewed as defining only constants, but its source {source} is not audited"
            )]
        ),
        "an unaudited macro"
    );
}

#[test]
fn reads_the_float_sources_of_the_toolchain_s_sysroot() {
    let mut fake = passing();
    let step = Step {
        name: "floats".to_owned(),
        action: Action::FloatMethodGate,
    };
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new()),
        Ok(()),
        "read and checked"
    );
    let read: Vec<&String> = fake
        .ran
        .iter()
        .filter(|line| line.starts_with("read ") || line.starts_with("capture rustc"))
        .collect();
    assert_eq!(
        read,
        [
            "read rust-toolchain.toml",
            "capture rustc +nightly-2026-11-01 --version",
            "capture rustc +nightly-2026-11-01 --print sysroot",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/f32.rs",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/f64.rs",
            "read /toolchain/lib/rustlib/src/rust/library/std/src/num/f32.rs",
            "read /toolchain/lib/rustlib/src/rust/library/std/src/num/f64.rs",
            "read /toolchain/lib/rustlib/src/rust/library/core/src/num/float_macros.rs",
            "read clippy.toml",
        ],
        "the release, the nightly's version and sysroot, then every source, then the bans"
    );
    let unpinned = fake.files.insert(
        crate::RUST_TOOLCHAIN_FILE.to_owned(),
        "[toolchain]\nchannel = \"stable\"\n".to_owned(),
    );
    assert!(unpinned.is_some(), "replaced");
    assert_eq!(
        run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("floats: rust-toolchain.toml must pin one release: channel = \"X.Y.Z\"".to_owned()),
        "no release"
    );
    let pinned = fake
        .files
        .insert(crate::RUST_TOOLCHAIN_FILE.to_owned(), TOOLCHAIN.to_owned());
    assert!(pinned.is_some(), "replaced");
    fake.broken = Some("rustc +nightly-2026-11-01 --version".to_owned());
    let unversioned = run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
        .expect_err("no version");
    assert_eq!(
        unversioned.summary,
        "floats: cannot find the toolchain's sources: cannot run `rustc +nightly-2026-11-01 --version`: not found",
        "no version"
    );
    fake.broken = Some("rustc +nightly-2026-11-01 --print sysroot".to_owned());
    let failure = run_steps(core::slice::from_ref(&step), &mut fake, &mut Vec::new())
        .expect_err("no sysroot");
    assert!(
        failure.summary
            == "floats: cannot find the toolchain's sources: cannot run `rustc +nightly-2026-11-01 --print sysroot`: not found",
        "{}",
        failure.summary
    );
}
