//! Tests of the citation gate: every roadmap step a versioned file cites exists on the board.

use crate::tests::{BOARD, cite, step_found};

#[test]
fn accepts_the_steps_a_board_has() {
    for text in [
        "\u{23}12 step 3",
        "see \u{23}12 steps 1 and 2.",
        "\u{23}12, step 1",
        "(\u{23}12 (step 2 of it)",
        "\u{23}12 steps 1, 2, and 3",
        "\u{23}12 steps 1, 2 and 3",
        "\u{23}12 steps 1 to 3",
        "\u{23}12 steps 2 or 3; \u{23}3 step 1",
        "**Step.** \u{23}12 step 1 (with step 3). **Order:** 3, before step 2.",
        "  **Step.** Engine: \u{23}12 step 3; format: \u{23}3 step 1, the port at step 1.",
        "**Step.** \u{23}12 step 1; substep 9, x-step 9.",
        "card \u{23}1 (entry 1) step 9",
        "C\u{23}12 step 9, &\u{23}12 step 9, x\u{23}12 step 9",
        "step 9 of the plan, steps 9 and 10",
        "\u{23}12, ( step 9",
        "\u{23}99, \u{23}12 Steps 9, \u{23}12 stepping 9, \u{23}12 step",
        "",
    ] {
        assert_eq!(cite(text), Ok(()), "{text}");
    }
    assert_eq!(
        crate::citation_gate(&[(crate::ROADMAP_BOARD.to_owned(), BOARD.to_owned())]),
        Ok(()),
        "the board alone"
    );
}

#[test]
fn refuses_a_step_the_board_does_not_have() {
    let past = |step: u64| {
        format!(
            "x.md:1: cites \u{23}12 step {step}, but \u{23}12 has 3 steps: cite one of them, \
             or add it (see: pnpm --filter @poyetik/roadmap roadmap help)"
        )
    };
    for (text, problems) in [
        ("\u{23}12 step 4", vec![past(4)]),
        ("\u{23}12 step 0", vec![past(0)]),
        ("\u{23}12 steps 2 to 5", vec![past(4), past(5)]),
        ("\u{23}12 steps 1, 4 and 9", vec![past(4), past(9)]),
        ("\u{23}12 steps 1, and 4", vec![past(4)]),
        ("\u{23}12 steps 1 or 4", vec![past(4)]),
        (
            "\u{23}12 step 18446744073709551615",
            vec![past(18_446_744_073_709_551_615)],
        ),
        (
            "\u{23}7 step 1",
            vec!["x.md:1: cites \u{23}7 step 1, but the board has no card \u{23}7".to_owned()],
        ),
        ("**Step.** \u{23}12 step 1 (with step 4).", vec![past(4)]),
        ("**Step.** \u{23}12 step 1 (step 4).", vec![past(4)]),
        (
            "**Step.** \u{23}3 step 1, then \u{23}12: at step 4.",
            vec![past(4)],
        ),
        ("\u{23}12 step 3 \u{23}12 step 4 step 5", vec![past(4)]),
        ("\u{23}12  step 4", vec![past(4)]),
        ("\u{23}12,step 4", vec![past(4)]),
        ("\u{23}12step 4", vec![past(4)]),
    ] {
        let problems: Vec<String> = problems;
        assert_eq!(cite(text), step_found(&problems), "{text}");
    }
    assert_eq!(
        cite("\u{23}12 step 1\n\n\u{23}12 step 4\n"),
        step_found(&[past(4).replace("x.md:1:", "x.md:3:")]),
        "the line named"
    );
}

#[test]
fn refuses_a_citation_it_cannot_read() {
    let at = |problem: &str| format!("x.md:1: {problem}");
    for (text, problem) in [
        ("\u{23}12 step one", "a step without its number"),
        ("\u{23}12, steps x", "a step without its number"),
        (
            "\u{23}12 step 99999999999999999999",
            "a step without its number",
        ),
        (
            "\u{23}12 steps 3 to 1",
            "the range of steps 3 to 1 does not rise",
        ),
        (
            "\u{23}12 steps 2 to 2",
            "the range of steps 2 to 2 does not rise",
        ),
        (
            "\u{23}12 steps 1-3",
            "a range of steps from 1 written with a dash: write `1 to N`",
        ),
        (
            "\u{23}12 steps 1 and 2–3",
            "a range of steps from 2 written with a dash: write `2 to N`",
        ),
        (
            "**Step.** step 2, then \u{23}12 step 1.",
            "a step cited without its card: write `\u{23}N step 2`",
        ),
        (
            "**Step.** Engine block.",
            "a **Step.** line citing no roadmap step: write `\u{23}N step M`",
        ),
        (
            "**Step.** \u{23}12 steps 3 to 1.",
            "the range of steps 3 to 1 does not rise",
        ),
        (
            "see \u{23}12 item 6",
            "\u{23}12 item 6 cites no step: write `\u{23}12 step 6`",
        ),
        (
            "\u{23}12 items 6 and 7",
            "\u{23}12 items 6 cites no step: write `\u{23}12 step 6`",
        ),
        (
            "\u{23}12, task 3",
            "\u{23}12 task 3 cites no step: write `\u{23}12 step 3`",
        ),
        (
            "(\u{23}12 (tasks 2 to 3",
            "\u{23}12 tasks 2 cites no step: write `\u{23}12 step 2`",
        ),
    ] {
        assert_eq!(cite(text), step_found(&[at(problem)]), "{text}");
    }
    assert_eq!(cite("\u{23}12 steps 1-x"), Ok(()), "a dash, then no number");
    for text in [
        "\u{23}12 itemized 6",
        "\u{23}12 items x",
        "\u{23}12 item",
        "\u{23}12 tasks of it",
        "every item 6",
    ] {
        assert_eq!(cite(text), Ok(()), "not a citation: {text}");
    }
}

#[test]
fn reads_the_board_as_the_roadmap_writes_it() {
    let gate = |board: &str| {
        crate::citation_gate(&[(crate::ROADMAP_BOARD.to_owned(), board.to_owned())])
            .map_err(|failure| failure.summary)
    };
    assert_eq!(gate("{"), Err("cannot read the roadmap board apps/roadmap/board.json: EOF while parsing an object at line 1 column 1".to_owned()), "not JSON");
    for board in ["{}", r#"{"cards":{}}"#] {
        assert_eq!(
            gate(board),
            Err("apps/roadmap/board.json lists no cards (see: pnpm --filter @poyetik/roadmap roadmap validate)".to_owned()),
            "{board}"
        );
    }
    for board in [
        r#"{"cards":[{"tasks":[]}]}"#,
        r#"{"cards":[{"id":"12","tasks":[]}]}"#,
        r#"{"cards":[{"id":12}]}"#,
        r#"{"cards":[{"id":12,"tasks":{}}]}"#,
    ] {
        assert_eq!(
            gate(board),
            Err(
                "apps/roadmap/board.json holds a card without its number or its steps (see: \
                 pnpm --filter @poyetik/roadmap roadmap validate)"
                    .to_owned()
            ),
            "{board}"
        );
    }
    assert_eq!(
        crate::citation_gate(&[("x.md".to_owned(), String::new())])
            .map_err(|failure| failure.summary),
        Err("cannot read the roadmap board apps/roadmap/board.json".to_owned()),
        "no board"
    );
    assert_eq!(
        gate(r#"{"cards":[{"id":12,"tasks":[]}]}"#),
        Ok(()),
        "a card without steps"
    );
}

#[test]
fn refuses_a_card_named_as_roadmap_work_without_its_step() {
    let named = |number: u64| {
        format!(
            "x.md:1: cites \u{23}{number} without the step that ends it: write \
             `\u{23}{number} step M`"
        )
    };
    for (text, problems) in [
        ("until roadmap \u{23}12.", vec![named(12)]),
        ("(roadmap \u{23}3)", vec![named(3)]),
        ("roadmap ticket \u{23}3", vec![named(3)]),
        ("in ticket \u{23}12, later", vec![named(12)]),
        (
            "Roadmap \u{23}12 and Ticket \u{23}3",
            vec![named(12), named(3)],
        ),
        (
            "roadmap \u{23}3 item 1",
            vec!["x.md:1: \u{23}3 item 1 cites no step: write `\u{23}3 step 1`".to_owned()],
        ),
    ] {
        let problems: Vec<String> = problems;
        assert_eq!(cite(text), step_found(&problems), "{text}");
    }
    for text in [
        "roadmap \u{23}12 step 1",
        "roadmap \u{23}12, step 1",
        "ticket \u{23}3 (step 1",
        "roadmap \u{23}12 steps 1 and 2",
        "\u{23}12 is later work, \u{23}3 too",
        "the roadmap: \u{23}12",
        "roadmaps \u{23}12, aticket \u{23}3, roadmap\u{23}12",
        "\u{20ac}12345 \u{23}12",
        // Seven bytes back from the citation falls inside the second `€`: no word there.
        "a\u{20ac}b\u{20ac} \u{23}12",
    ] {
        assert_eq!(cite(text), Ok(()), "{text}");
    }
    let changelog = |text: &str| {
        crate::citation_gate(&[
            (crate::ROADMAP_BOARD.to_owned(), BOARD.to_owned()),
            ("CHANGELOG.md".to_owned(), text.to_owned()),
        ])
    };
    assert_eq!(
        changelog(
            "# Changelog\n\n## [Unreleased]\n\n- roadmap \u{23}3 step 1\n\n## [0.1.0] - \
             2026-11-08\n\n- roadmap \u{23}12\n- \u{23}12 step 9\n"
        ),
        step_found(&[
            "CHANGELOG.md:10: cites \u{23}12 step 9, but \u{23}12 has 3 steps: cite one of them, \
             or add it (see: pnpm --filter @poyetik/roadmap roadmap help)"
                .to_owned()
        ]),
        "a released section is as it was released, its steps still read"
    );
    assert_eq!(
        changelog("## [Unreleased]\n\n- roadmap \u{23}12\n\n## [0.1.0] - 2026-11-08\n"),
        step_found(&[named(12).replace("x.md:1:", "CHANGELOG.md:3:")]),
        "the unreleased section is read"
    );
    assert_eq!(
        crate::citation_gate(&[
            (crate::ROADMAP_BOARD.to_owned(), BOARD.to_owned()),
            (
                "a/CHANGELOG.md".to_owned(),
                "## [0.1.0] - 2026-11-08\n- roadmap \u{23}12\n".to_owned(),
            ),
        ]),
        step_found(&[named(12).replace("x.md:1:", "a/CHANGELOG.md:2:")]),
        "only the root changelog is released"
    );
}

/// The citation gate on a board whose card \u{23}12 has its first step done, its second not,
/// and one file, `x.md`, holding `text`.
fn cite_with_done(text: &str) -> Result<(), crate::Failure> {
    crate::citation_gate(&[
        (
            crate::ROADMAP_BOARD.to_owned(),
            r#"{"cards":[{"id":12,"tasks":[{"done":true},{"done":false},{}]}]}"#.to_owned(),
        ),
        ("x.md".to_owned(), text.to_owned()),
    ])
}

#[test]
fn refuses_a_done_step_cited_as_work_to_come() {
    let done = |word: &str| {
        crate::tests::failed(format!(
            "x.md:1: cites \u{23}12 step 1, done on the board, as work to come (`{word}`): cite \
             the step that remains, or say it is done"
        ))
    };
    for (text, word) in [
        ("The bound will be measured (\u{23}12 step 1).", "will"),
        ("Not yet measured: \u{23}12 step 1.", "not yet"),
        ("Exact Later, at \u{23}12 step 1, once wired.", "later"),
        ("Kept until \u{23}12 steps 1 and 2 land.", "until"),
        ("The rest is to come in \u{23}12 step 1.", "to come"),
    ] {
        assert_eq!(cite_with_done(text), done(word), "{text}");
    }
    for text in [
        "The bound will be measured (\u{23}12 step 2).",
        "Not yet: \u{23}12 step 3, whose state the board leaves unsaid.",
        "Measured at \u{23}12 step 1. The next bound will come later.",
        "Measured at \u{23}12 step 1; what is left will follow.",
        "Measured at \u{23}12 step 1, willingly, untiled, belated, in twill.",
    ] {
        assert_eq!(cite_with_done(text), Ok(()), "{text}");
    }
    assert_eq!(
        crate::citation_gate(&[
            (
                crate::ROADMAP_BOARD.to_owned(),
                r#"{"cards":[{"id":12,"tasks":[{"done":true}]}]}"#.to_owned(),
            ),
            (
                crate::CHANGELOG.to_owned(),
                "# Changelog\n\n## [Unreleased]\n\n## [1.0.0] - 2026-01-01\n\n- It will come at \u{23}12 step 1.\n"
                    .to_owned(),
            ),
        ]),
        Ok(()),
        "a released section, dated, says what was to come then"
    );
}

/// The citation gate on the board of [`cite_with_done`], with one file at `path` holding
/// `text`.
fn cite_in(path: &str, text: &str) -> Result<(), crate::Failure> {
    crate::citation_gate(&[
        (
            crate::ROADMAP_BOARD.to_owned(),
            r#"{"cards":[{"id":12,"tasks":[{"done":true},{"done":false},{}]}]}"#.to_owned(),
        ),
        (path.to_owned(), text.to_owned()),
    ])
}

#[test]
fn reads_a_sentence_wrapped_over_the_lines_of_its_paragraph_or_comment() {
    // Finding C10-4 of the third deep review of visual block 2 (#34): the gate read one line
    // at a time, so a sentence wrapped between its future word and its citation escaped it.
    let done = |at: &str, word: &str| {
        crate::tests::failed(format!(
            "{at}: cites \u{23}12 step 1, done on the board, as work to come (`{word}`): cite \
             the step that remains, or say it is done"
        ))
    };
    for (path, text, at, word) in [
        (
            "x.rs",
            "/// The bound is not\n/// yet measured (\u{23}12 step 1).\n",
            "x.rs:2",
            "not yet",
        ),
        (
            "x.rs",
            "    //! Measured at \u{23}12 step 1, but not\n    //! yet bounded.\n",
            "x.rs:1",
            "not yet",
        ),
        (
            "x.rs",
            "// Kept\n// until \u{23}12 step 1.\n",
            "x.rs:2",
            "until",
        ),
        (
            "x.ts",
            "/**\n * Kept until the\n * bound lands (\u{23}12 step 1). */\n",
            "x.ts:3",
            "until",
        ),
        (
            "x.ts",
            "/** It will be\n * measured at \u{23}12 step 1.\n */\n",
            "x.ts:2",
            "will",
        ),
        (
            "x.md",
            "It will be\nmeasured at \u{23}12 step 1.\n",
            "x.md:2",
            "will",
        ),
        (
            "x.md",
            "- It is to\n  come at \u{23}12 step 1.\n",
            "x.md:2",
            "to come",
        ),
        (
            "x.toml",
            "# Kept until\n# \u{23}12 step 1 lands.\n",
            "x.toml:2",
            "until",
        ),
    ] {
        assert_eq!(cite_in(path, text), done(at, word), "{text}");
    }
    for (path, text) in [
        // A blank line, or a blank comment line, ends a paragraph or a block.
        (
            "x.rs",
            "/// It will come.\n///\n/// Measured at \u{23}12 step 1.\n",
        ),
        ("x.md", "It will come\n\nat \u{23}12 step 1.\n"),
        ("x.ts", "/** It will come */\n/** at \u{23}12 step 1. */\n"),
        // A list item, a heading or a table row starts its own paragraph.
        ("x.md", "- It will come\n- at \u{23}12 step 1.\n"),
        ("x.md", "* It will come\n* at \u{23}12 step 1.\n"),
        ("x.toml", "# It will come\n#\n# at \u{23}12 step 1.\n"),
        ("x.md", "It will come\n# At \u{23}12 step 1\n"),
        ("x.md", "| It will come |\n| at \u{23}12 step 1 |\n"),
        ("x.rs", "/// It will come:\n/// - at \u{23}12 step 1.\n"),
        // Comments of different kinds, and lines of code, are never joined.
        ("x.rs", "/// It will come\n// at \u{23}12 step 1.\n"),
        ("x.rs", "let until = 1;\nlet at = \"\u{23}12 step 1\";\n"),
        ("x.rs", "// It will come\nlet at = 1; // \u{23}12 step 1.\n"),
        (
            "x.json",
            "{\"a\": \"until\",\n\"b\": \"\u{23}12 step 1\"}\n",
        ),
        ("x.toml", "# It will come\nkey = 1 # \u{23}12 step 1\n"),
        // A line starting with `*` continues only an open block comment, never a line comment.
        ("x.rs", "// A note\n* until \u{23}12 step 1.\n"),
    ] {
        assert_eq!(cite_in(path, text), Ok(()), "{path}: {text}");
    }
}

#[test]
fn reads_a_citation_wrapped_over_the_lines_of_its_paragraph_or_comment() {
    // Finding F7-2 of the fourth deep review of visual block 2 (#34): the gate read a
    // card, its step and a decision one line at a time, so a citation wrapped between its
    // card and its step, or a specification's name and its decision, escaped it. Each is
    // named at the line where it starts.
    let decision = |at: &str| {
        crate::tests::failed(format!(
            "{at}: cites spec/visual-design.md by decision: its parts are sections, write \
             `section N`"
        ))
    };
    for (path, text, at) in [
        (
            "x.rs",
            "/// As `spec/visual-design.md`,\n/// decision 1, says.\n",
            "x.rs:2",
        ),
        (
            "x.md",
            "As `spec/visual-design.md`,\ndecision 1, says.\n",
            "x.md:2",
        ),
        (
            "x.toml",
            "# As visual-design.md\n# decision 2.\n",
            "x.toml:2",
        ),
    ] {
        assert_eq!(cite_in(path, text), decision(at), "{text}");
    }
    for (path, text, problem) in [
        (
            "x.rs",
            "/// Measured at \u{23}12\n/// step 9.\n",
            "x.rs:1: cites \u{23}12 step 9, but \u{23}12 has 3 steps: cite one of them, or add \
             it (see: pnpm --filter @poyetik/roadmap roadmap help)",
        ),
        (
            "x.md",
            "Intro.\nMeasured at \u{23}7\nstep 1.\n",
            "x.md:2: cites \u{23}7 step 1, but the board has no card \u{23}7",
        ),
        (
            "x.ts",
            "/**\n * Kept at \u{23}12\n * item 2.\n */\n",
            "x.ts:2: \u{23}12 item 2 cites no step: write `\u{23}12 step 2`",
        ),
        (
            "x.rs",
            "/// A note.\n\nlet a = \"\u{23}12 step 9\"; // code\n",
            "x.rs:3: cites \u{23}12 step 9, but \u{23}12 has 3 steps: cite one of them, or add \
             it (see: pnpm --filter @poyetik/roadmap roadmap help)",
        ),
    ] {
        assert_eq!(cite_in(path, text), crate::tests::failed(problem), "{text}");
    }
    for (path, text) in [
        // A wrapped citation the board has is read whole, its card named as work included.
        ("x.rs", "/// Measured at \u{23}12\n/// step 3.\n"),
        ("x.md", "As the roadmap\n\u{23}12 step 2 says.\n"),
        ("x.md", "As roadmap \u{23}12\nstep 2 says.\n"),
        // A blank line ends a paragraph: no citation is joined over it.
        ("x.md", "Measured at \u{23}12\n\nstep 9.\n"),
        ("x.md", "As `spec/visual-design.md`,\n\ndecision 1, says.\n"),
        // Lines of code are read alone.
        ("x.rs", "let a = \"\u{23}12\";\nlet b = \"step 9\";\n"),
    ] {
        assert_eq!(cite_in(path, text), Ok(()), "{path}: {text}");
    }
}

#[test]
fn refuses_a_sectioned_specification_cited_by_decision() {
    // Finding C7-3 of the third deep review of visual block 2 (#34): the visual design is
    // written in sections, and six sites still cited it by decision, the audio design's word.
    // The fixtures write VD and ED for the names of the two designs, so that the gate,
    // reading this file, does not take them for citations.
    let named = |text: &str| {
        text.replace("VD", "visual-design.md")
            .replace("ED", "engine-design.md")
    };
    let decision = |at: &str| {
        crate::tests::failed(format!(
            "{at}: cites spec/visual-design.md by decision: its parts are sections, write \
             `section N`"
        ))
    };
    for (path, text) in [
        ("x.md", "As `spec/VD`, decision 1, says."),
        ("x.md", "As `spec/VD` decision 1 says."),
        ("x.md", "As [`VD`](VD), decision 2, says."),
        ("x.rs", "// (spec/VD, decision 9)"),
        ("spec/visual-design.md", "Linear light (decision 3)."),
        (
            "spec/visual-design.md",
            "As `ED`, decision 33, and decision 3 say.",
        ),
    ] {
        let text = &named(text);
        assert_eq!(
            cite_in(path, text),
            decision(&format!("{path}:1")),
            "{text}"
        );
    }
    for (path, text) in [
        ("x.md", "As `spec/VD`, section 1, says."),
        ("x.md", "As `spec/VD`, decision makers say."),
        ("x.md", "As `spec/ED`, decision 1, says."),
        ("x.md", "As `VD`. The decision 3 of the audio design."),
        ("spec/visual-design.md", "As `ED`, decision 33, says."),
        (
            "spec/VD",
            "The (`spec/ED` decision 32) and (`ED`, decision 29).",
        ),
        (
            "spec/visual-design.md",
            "The design decisions of the library.",
        ),
    ] {
        let text = &named(text);
        assert_eq!(cite_in(path, text), Ok(()), "{path}: {text}");
    }
}

#[test]
fn reads_the_unreleased_section_of_the_changelog_up_to_the_first_release() {
    assert_eq!(
        crate::citation_gate(&[
            (
                crate::ROADMAP_BOARD.to_owned(),
                r#"{"cards":[{"id":12,"tasks":[{"done":true}]}]}"#.to_owned(),
            ),
            (
                crate::CHANGELOG.to_owned(),
                "# Changelog\n\n## [Unreleased]\n\n- It will come at \u{23}12 step 1.\n\n## [1.0.0] - 2026-01-01\n\n- It will come at \u{23}12 step 1.\n"
                    .to_owned(),
            ),
        ]),
        crate::tests::failed(format!(
            "{}:5: cites \u{23}12 step 1, done on the board, as work to come (`will`): cite \
             the step that remains, or say it is done",
            crate::CHANGELOG
        )),
        "the unreleased entry is read, the released one is not"
    );
}

#[test]
fn cites_no_done_step_of_a_card_the_board_does_not_hold() {
    assert_eq!(
        super::done_as_to_come(
            "x.md",
            &[(0, "The bound will be measured at \u{23}13 step 1.")],
            &alloc::collections::BTreeMap::from([(12, vec![true])]),
        ),
        Vec::<String>::new(),
        "no card 13: no step of it is done"
    );
}
