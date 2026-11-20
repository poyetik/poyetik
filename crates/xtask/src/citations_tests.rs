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
             or add it (`pnpm roadmap step add 12 …`)"
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
    assert!(
        gate("{").is_err_and(|summary| summary
            .starts_with("cannot read the roadmap board apps/roadmap/board.json: ")),
        "not JSON"
    );
    for board in ["{}", r#"{"cards":{}}"#] {
        assert_eq!(
            gate(board),
            Err("apps/roadmap/board.json lists no cards (see: pnpm roadmap validate)".to_owned()),
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
                 pnpm roadmap validate)"
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
             or add it (`pnpm roadmap step add 12 …`)"
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
        "Measured at \u{23}12 step 1, willingly, untiled, belated.",
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
