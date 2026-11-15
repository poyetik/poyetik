//! Tests of the math layer gate: which crates may compute in floating point, and how.

use crate::tests::{
    CHANGES_MODE, FAKE_JOBS, ONE_UNSAFE_OPERATION, audited, failed, fixture_metadata, found,
    lifting, parsed, reports, unsafe_gate,
};
use crate::{Action, Failure, Mode, UNSAFE_FILES, check_steps};

/// [`crate::math_layer_gate`] on `metadata` and `sources`, read by [`parsed`].
fn math_layer_gate(metadata: &str, sources: &[(String, String)]) -> Result<(), Failure> {
    crate::math_layer_gate(metadata, &parsed(sources))
}

/// The texts that hold `attribute` somewhere but not among the leading inner attributes of
/// the file: in a block comment, in a raw string, in an inline module, after an item, after
/// an outer doc comment.
fn misplaced(attribute: &str) -> [String; 5] {
    [
        format!("/* {attribute} */\nfn f() {{}}\n"),
        format!("const S: &str = r#\"\n{attribute}\n\"#;\n"),
        format!("mod inner {{\n    {attribute}\n}}\n"),
        format!("fn f() {{}}\n{attribute}\n"),
        format!("/// An item.\n{attribute}\nfn f() {{}}\n"),
    ]
}

#[test]
fn requires_each_forbid_as_a_leading_inner_attribute_of_its_file() {
    let forbid = format!("#![forbid({}_{})]", "unsafe", "code");
    let audited_sources: Vec<(String, String)> = UNSAFE_FILES
        .iter()
        .map(|path| ((*path).to_owned(), audited(path)))
        .collect();
    let root = "crates/audio-cli/src/main.rs";
    let unsafe_site = |text: &str| {
        let mut sources = audited_sources.clone();
        sources.push((root.to_owned(), text.to_owned()));
        unsafe_gate(&sources)
    };
    let assembly = crate::ASSEMBLY_FILES[0];
    let assembly_site = |text: &str| {
        let mut sources = audited_sources.clone();
        for (path, held) in &mut sources {
            if path == assembly {
                *held = format!(
                    "{}{text}fn g() {{\n    unsafe {{\n        ::core::arch::asm!(\"nop\");\n    }}\n}}\n",
                    lifting()
                );
            }
        }
        unsafe_gate(&sources)
    };
    let module = "crates/audio-core/src/plan.rs";
    let math_site = |path: &str, text: &str| {
        math_layer_gate(r#"{"packages":[]}"#, &[(path.to_owned(), text.to_owned())])
    };
    let (float, arithmetic) = (crate::BOX_FLOAT_FORBID, crate::ENGINE_ARITHMETIC_FORBID);
    assert_eq!(unsafe_site(&leading(&forbid)), Ok(()), "{forbid}");
    assert_eq!(
        assembly_site(&leading(ONE_UNSAFE_OPERATION)),
        Ok(()),
        "{ONE_UNSAFE_OPERATION}"
    );
    assert_eq!(
        math_site(crate::BOX_ROOT, &leading(float)),
        Ok(()),
        "{float}"
    );
    assert_eq!(
        math_site(module, &leading(arithmetic)),
        Ok(()),
        "{arithmetic}"
    );
    for text in misplaced(&forbid) {
        assert_eq!(
            unsafe_site(&text),
            failed(format!(
                "the lint against unsafe code: {root} does not hold `{forbid}`"
            )),
            "{text}"
        );
    }
    for text in misplaced(ONE_UNSAFE_OPERATION) {
        let found = assembly_site(&text);
        assert!(
            reports(
                &found,
                &format!(
                    "{assembly} does not hold `{ONE_UNSAFE_OPERATION}`: an unsafe block of it \
                     could hold more than its assembly call"
                )
            ),
            "{text}: {found:?}"
        );
    }
    for (path, attribute) in [
        (crate::BOX_ROOT, float),
        (crate::VISUAL_BOX_ROOT, float),
        (module, arithmetic),
    ] {
        for text in misplaced(attribute) {
            let found = math_site(path, &text);
            assert!(
                reports(&found, &format!("{path} does not hold `{attribute}`")),
                "{text}: {found:?}"
            );
        }
    }
}

/// The summary of the math layer gate's failures.
const MATH_LAYER: &str = "box arithmetic only through the math layer (audio-core, \
                          audio-testkit only may depend on libm; box code names no raw float, \
                          libm, host or delivery, and computes no float; no iterator reduction \
                          outside the math layer; code comes in by `mod` alone)";

#[test]
fn refuses_libm_outside_the_math_layer_and_the_measurements() {
    let gate = |dependencies: &str| {
        math_layer_gate(
            &format!(
                r#"{{"packages":[{{"name":"audio-core","dependencies":[{{"name":"libm","kind":null}}]}},{{"name":"audio-testkit","dependencies":[{{"name":"libm","kind":null}}]}},{{"name":"audio-boxes","dependencies":{dependencies}}}]}}"#
            ),
            &[],
        )
    };
    let refused = |name: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: {name} depends on libm: its arithmetic must go through the math \
             layer of audio-core"
        )))
    };
    assert_eq!(
        gate("[]"),
        Ok(()),
        "the math layer and the measurements may"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":"dev"}]"#),
        Ok(()),
        "a test's reference may"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":null}]"#),
        refused("audio-boxes"),
        "a box crate may not"
    );
    assert_eq!(
        gate(r#"[{"name":"libm","kind":"build","rename":"math"}]"#),
        refused("audio-boxes"),
        "not as a build dependency, not renamed"
    );
    assert_eq!(
        gate(r#"[{"name":"libmx","kind":null},{"name":"audio-core","kind":null,"path":"/r"}]"#),
        Ok(()),
        "another crate"
    );
    let two = math_layer_gate(
        r#"{"packages":[{"name":"a","dependencies":[{"name":"libm"}]},{"name":"b","dependencies":[{"name":"libm","kind":"normal"}]}]}"#,
        &[],
    );
    assert_eq!(
        two.map_err(|failure| (failure.summary, failure.details.len())),
        Err((format!("{MATH_LAYER}: 2 problems, listed above"), 2)),
        "every crate"
    );
    assert!(
        math_layer_gate("[", &[])
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "unreadable metadata"
    );
    assert_eq!(
        math_layer_gate("{}", &[]),
        failed("cargo metadata lists no packages"),
        "metadata without packages"
    );
    assert_eq!(
        math_layer_gate(r#"{"packages":[{"dependencies":[{"name":"libm"}]}]}"#, &[]),
        failed("cargo metadata lists a package without a name"),
        "a package without a name is refused, never read as \"\""
    );
    assert_eq!(
        crate::MATH_LAYER_SOURCES,
        [
            format!("{}*.rs", crate::BOX_CODE),
            format!("{}*.rs", crate::ENGINE_CODE),
            crate::VISUAL_BOX_ROOT.to_owned()
        ],
        "the pathspecs of the box and engine directories, and the visual boxes' root"
    );
    assert_eq!(
        crate::BOX_ROOTS,
        [
            "crates/audio-boxes/src/lib.rs",
            "crates/visual-boxes/src/lib.rs"
        ],
        "the roots of both libraries' box crates"
    );
}

#[test]
fn refuses_raw_floats_libm_and_host_in_box_code() {
    let gate = |text: &str| {
        math_layer_gate(
            r#"{"packages":[]}"#,
            &[("crates/audio-boxes/src/a.rs".to_owned(), text.to_owned())],
        )
    };
    let refused = |line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/audio-boxes/src/a.rs:{line}: box code names `{word}`: it \
             computes with the engine's sample types, through the math layer"
        )))
    };
    for word in crate::BOX_FORBIDDEN_NAMES {
        assert_eq!(
            gate(&format!("fn a() {{}}\nlet x: {word} = y;")),
            refused(2, word),
            "{word}"
        );
    }
    assert_eq!(
        gate("let x = 1.5_f64;"),
        refused(1, "1.5_f64"),
        "a suffixed literal"
    );
    assert_eq!(
        gate("let x = 2f32;"),
        refused(1, "2f32"),
        "a suffix without separator"
    );
    assert_eq!(
        gate("use audio_core::host::Raw;"),
        refused(1, "host"),
        "the host module"
    );
    assert_eq!(gate("x as f64"), refused(1, "f64"), "a cast");
    // Prose and other words are not code: comments, strings, longer names.
    for allowed in [
        "// an f64 phase\nlet a = 1;",
        "/* f32\n libm */ let a = 1;",
        "/// in f64\nfn a() {}",
        r#"let s = "f64 \" host";"#,
        "let f64x = hostname + libm_like + f32_; let n = 1.0e-3;",
        "let x = Binary64::new(440.0);",
    ] {
        assert_eq!(gate(allowed), Ok(()), "{allowed}");
    }
    assert_eq!(
        gate("/* a comment */ f64"),
        refused(1, "f64"),
        "code after a block comment"
    );
    assert_eq!(
        gate("/* x */\n// y\nlet s = \"a\nb\";\nf64"),
        refused(5, "f64"),
        "lines counted through comments and strings"
    );
    // A sample's float taken out through the host's delivery, as a box could write it.
    assert_eq!(
        gate("let p = Delivered::deliver(self.phase);")
            .map_err(|failure| (failure.summary, failure.details)),
        Err((
            format!("{MATH_LAYER}: 2 problems, listed above"),
            vec![
                "crates/audio-boxes/src/a.rs:1: box code names `Delivered`: it computes \
                 with the engine's sample types, through the math layer"
                    .to_owned(),
                "crates/audio-boxes/src/a.rs:1: box code names `deliver`: it computes with \
                 the engine's sample types, through the math layer"
                    .to_owned(),
            ]
        )),
        "the delivery of samples to a host"
    );
    assert_eq!(
        math_layer_gate(
            r#"{"packages":[]}"#,
            &[
                (
                    "crates/audio-boxes/src/osc.rs".to_owned(),
                    "#[cfg(test)]\n#[path = \"osc_tests.rs\"]\nmod tests;\n".to_owned()
                ),
                (
                    "crates/audio-boxes/src/osc_tests.rs".to_owned(),
                    "let x: f64 = libm::sin(1.0);".to_owned()
                )
            ],
        ),
        Ok(()),
        "test files are not box code"
    );
}

#[test]
fn reads_char_literals_and_raw_strings_as_prose() {
    let gate = |text: &str| {
        math_layer_gate(
            r#"{"packages":[]}"#,
            &[("crates/audio-boxes/src/a.rs".to_owned(), text.to_owned())],
        )
    };
    let refused = |line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/audio-boxes/src/a.rs:{line}: box code names `{word}`: it \
             computes with the engine's sample types, through the math layer"
        )))
    };
    // Literals that hold a quote: a char literal and raw strings are not strings to the end
    // of the file, and the code after them is read.
    for (text, line) in [
        ("let q = '\"'; let x: f64 = 2.0;", 1),
        ("let q = '\\''; let x: f64 = 2.0;", 1),
        ("let q = '\\\\'; let s = \"\"; f64", 1),
        ("let q = '\\u{22}'; f64", 1),
        ("fn a<'a>(x: &'a u8) {}\nf64", 2),
        ("let s = r\"\\\"; f64", 1),
        ("let s = r#\"a\"b\"#; let t = \"\";\nf64", 2),
        ("let s = br##\"a\"#b\n\"##; f64", 2),
        ("let s = cr#\"a\"#; f64", 1),
        ("let a = 1; /*/ f64 */ f64", 1),
        ("let a = 1; /**/ f64", 1),
        ("let q = '\\\"'; f64", 1),
        ("let a = ['\\x41','\"', f64];", 1),
        ("fn a<'f64>() {}", 1),
        ("let r#f64 = 1;", 1),
    ] {
        let found = gate(text);
        assert_eq!(found, refused(line, "f64"), "{text}");
    }
    for allowed in [
        "let q = '\"'; let s = \"f64\";",
        "let s = r#\"f64 \" libm\"#;",
        "let s = r\"f64\";",
        "let r#type = 1; let s = \"f64\";",
        "/* a/b f64 */ let a = 1;",
        "/* a* f64 */ let a = 1;",
    ] {
        assert_eq!(gate(allowed), Ok(()), "{allowed}");
    }
}

#[test]
fn refuses_sums_outside_the_math_layer() {
    let gate = |path: &str, text: &str| {
        math_layer_gate(r#"{"packages":[]}"#, &[(path.to_owned(), text.to_owned())])
    };
    let refused = |path: &str, line: usize, word: &str| {
        Err(Failure::from(format!(
            "{MATH_LAYER}: {path}:{line}: `{word}` is an iterator reduction outside the math \
             layer: an iterator sums samples only through ordered_sum, in the reduction order"
        )))
    };
    let (boxes, engine) = ("crates/audio-boxes/src/a.rs", crate::ENGINE_ROOT);
    for path in [boxes, engine] {
        for word in crate::REDUCTION_NAMES {
            assert_eq!(
                gate(path, &format!("let a = 1;\nlet t = x.iter().{word}(y);")),
                refused(path, 2, word),
                "{path}: {word}"
            );
        }
        assert_eq!(
            gate(
                path,
                "let t = ordered_sum(x); // a fold, a sum\nlet s = \"sum\";"
            ),
            Ok(()),
            "{path}: the helper, prose and strings"
        );
    }
    assert_eq!(
        gate(
            engine,
            "#[expect(clippy::arithmetic_side_effects, reason = \"x\")]"
        ),
        Err(Failure::from(format!(
            "{MATH_LAYER}: {engine}:1: engine code names `arithmetic_side_effects`: it does \
             no arithmetic on a sample type outside the math layer"
        ))),
        "arithmetic on a generic sample type in the engine"
    );
    assert_eq!(
        gate(
            boxes,
            "#[expect(clippy::arithmetic_side_effects, reason = \"x\")]"
        ),
        Ok(()),
        "a box computes on its samples"
    );
    assert_eq!(
        gate(engine, "let x: f64 = host::Raw::raw(y);"),
        Ok(()),
        "the engine core names floats and its host module"
    );
    let anything = "let t = x.iter().fold(0, f).sum(); arithmetic_side_effects; f64";
    assert_eq!(
        gate("crates/audio-core/src/math.rs", anything),
        Ok(()),
        "the math layer"
    );
    for (declaring, exempt, name) in [
        (
            "crates/audio-core/src/plan.rs",
            "crates/audio-core/src/plan_tests.rs",
            "plan",
        ),
        (
            "crates/audio-boxes/src/osc.rs",
            "crates/audio-boxes/src/osc_tests.rs",
            "osc",
        ),
    ] {
        let declaration = format!(
            "{}\n#[cfg(test)]\n#[path = \"{name}_tests.rs\"]\nmod tests;\n",
            crate::ENGINE_ARITHMETIC_FORBID
        );
        assert_eq!(
            math_layer_gate(
                r#"{"packages":[]}"#,
                &[
                    (declaring.to_owned(), declaration),
                    (exempt.to_owned(), anything.to_owned())
                ]
            ),
            Ok(()),
            "{exempt}: declared test code"
        );
    }
    assert_eq!(
        gate("crates/audio-patch/src/lib.rs", "let a = 1;"),
        Err(Failure::from(format!(
            "{MATH_LAYER}: crates/audio-patch/src/lib.rs: not a source the math layer gate \
             reads"
        ))),
        "a source of another crate"
    );
}

#[test]
fn requires_the_compiler_to_refuse_float_and_generic_arithmetic() {
    let none = r#"{"packages":[]}"#;
    let gate = |sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        math_layer_gate(none, &owned)
    };
    let (float, arithmetic) = (crate::BOX_FLOAT_FORBID, crate::ENGINE_ARITHMETIC_FORBID);
    assert_eq!(
        gate(&[
            (crate::BOX_ROOT, &format!("#![no_std]\n{float}\n")),
            ("crates/audio-boxes/src/osc.rs", "fn f() {}\n"),
            (
                "crates/audio-core/src/plan.rs",
                &format!("//! Plans.\n\n{arithmetic}\n")
            ),
            (crate::ENGINE_ROOT, "mod plan;\n"),
            (
                crate::MATH_LAYER_FILE,
                "#[expect(clippy::arithmetic_side_effects)]\n"
            ),
        ]),
        Ok(()),
        "each forbid where it belongs, the root and the math layer apart"
    );
    assert_eq!(
        gate(&[(
            crate::VISUAL_BOX_ROOT,
            &format!("#![no_std]\n{float}\nfn f() -> f32 {{ 1.0f32 + libm::sinf(0.5) }}\n")
        )]),
        Ok(()),
        "the visual box root: its forbid alone is read, the audio rules are not its own"
    );
    assert_eq!(
        gate(&[(
            crate::VISUAL_BOX_ROOT,
            "#![no_std]\n#![forbid(clippy::float_arithmetic)]\n"
        )]),
        failed(format!(
            "{MATH_LAYER}: {} does not hold `{float}`",
            crate::VISUAL_BOX_ROOT
        )),
        "the visual box root without its exact line"
    );
    assert_eq!(
        gate(&[(
            crate::BOX_ROOT,
            "#![no_std]\n#![forbid(clippy::float_arithmetic)]\n"
        )]),
        failed(format!(
            "{MATH_LAYER}: {} does not hold `{float}`",
            crate::BOX_ROOT
        )),
        "the box root without its exact line"
    );
    assert_eq!(
        gate(&[("crates/audio-core/src/render.rs", "fn f() {}\n")]),
        failed(format!(
            "{MATH_LAYER}: crates/audio-core/src/render.rs does not hold `{arithmetic}`"
        )),
        "an engine module without its line"
    );
    for word in crate::ENGINE_FORBIDDEN_NAMES {
        assert_eq!(
            gate(&[(
                crate::ENGINE_ROOT,
                &format!("#![expect(clippy::{word}, reason = \"r\")]\n")
            )]),
            failed(format!(
                "{MATH_LAYER}: {}:1: engine code names `{word}`: it does no arithmetic on a \
                 sample type outside the math layer",
                crate::ENGINE_ROOT
            )),
            "{word} in the root, which cannot forbid it"
        );
    }
}

#[test]
fn reads_every_source_of_box_and_engine_code() {
    let gate = |metadata: &str, sources: &[(&str, &str)]| {
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        math_layer_gate(metadata, &owned)
    };
    let none = r#"{"packages":[]}"#;
    let declared = "#[cfg(test)]\n#[path = \"a_tests.rs\"]\nmod tests;\n";
    let words = |path: &str, line: usize, word: &str| {
        format!(
            "{path}:{line}: names `{word}` outside a test declaration: code comes into these \
             crates by `mod` alone, from files the gate reads"
        )
    };
    let boxes = "crates/audio-boxes/src/a.rs";
    for (text, line, word) in [
        ("fn f() {}\ninclude!(\"../../x.rs\");\n", 2, "include"),
        ("include!(\"../../x.rs\");\n", 1, "include"),
    ] {
        assert_eq!(
            gate(none, &[(boxes, text)]),
            failed(format!("{MATH_LAYER}: {}", words(boxes, line, word))),
            "{text}"
        );
    }
    assert_eq!(
        gate(none, &[(crate::MATH_LAYER_FILE, "include!(\"x.rs\");\n")]),
        failed(format!(
            "{MATH_LAYER}: {}",
            words(crate::MATH_LAYER_FILE, 1, "include")
        )),
        "the math layer brings no code in either"
    );
    assert_eq!(
        gate(
            none,
            &[
                (boxes, declared),
                ("crates/audio-boxes/src/a_tests.rs", "f64")
            ]
        ),
        Ok(()),
        "a declared test file"
    );
    assert_eq!(
        gate(none, &[("crates/audio-boxes/src/a_tests.rs", "f64")]),
        Ok(()),
        "a test file not declared: the test code and loaded-files gates' to refuse"
    );
    for mode in [CHANGES_MODE, Mode::Full { jobs: FAKE_JOBS }] {
        let actions: Vec<Action> = check_steps(mode)
            .into_iter()
            .map(|step| step.action)
            .collect();
        for keeps_the_names_true in [Action::TestCodeGate, Action::LoadedFilesGate] {
            assert!(
                actions.contains(&Action::MathLayerGate) && actions.contains(&keeps_the_names_true),
                "{mode:?}: the math layer gate skips test files by name beside {keeps_the_names_true:?}"
            );
        }
    }
}

#[test]
fn reads_what_real_cargo_metadata_says_of_libm() {
    let (metadata, _) = fixture_metadata();
    assert_eq!(
        math_layer_gate(&metadata, &[]),
        found(
            MATH_LAYER,
            &[
                "audio-cli depends on libm: its arithmetic must go through the math layer of \
               audio-core"
                    .to_owned()
            ]
        ),
        "a dependency renamed; libm in the core and the measurements, and in the tests of the \
         boxes (a build script is the untested targets gate's)"
    );
}

/// `attribute` among other leading inner attributes, its tokens spaced otherwise.
fn leading(attribute: &str) -> String {
    format!(
        "//! The file.\n#![allow(dead_code, reason = \"r\")]\n{}\n/// An item.\nfn f() {{}}\n",
        attribute.replace('(', "( ").replace(')', " )")
    )
}
