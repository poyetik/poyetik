//! Tests of the WGSL gate: the valid shaders validated together, each invalid fixture
//! alone and required refused, every way it can fail said.

use crate::steps::Failure;
use crate::tests::passing;
use crate::{COPIES, INVALID_SHADERS, copies_gate, wgsl_command, wgsl_gate};

/// The command validating the invalid fixture alone.
fn refusal() -> String {
    wgsl_command(&INVALID_SHADERS.map(|(shader, _)| shader)).line()
}

/// The refusal of the gate saying `problems`.
fn refused(problems: &[String]) -> Result<(), Failure> {
    crate::steps::verdict(
        "every WGSL shader valid, the invalid fixtures refused",
        problems.to_vec(),
    )
}

#[test]
fn validates_the_tracked_shaders_with_wgsl_test_at_its_locked_versions() {
    assert_eq!(
        wgsl_command(&["a.wgsl", "b.wgsl"]).line(),
        "cargo run --manifest-path tooling/wgsl/Cargo.toml --package wgsl-test --locked --quiet \
         -- validate a.wgsl b.wgsl",
        "the WGSL tools workspace's own build"
    );
    assert_eq!(
        INVALID_SHADERS,
        [("tooling/wgsl/wgsl-test/tests/fixtures/invalid.wgsl", "4:12")],
        "the fixture wgsl-test's tests require refused, where it is"
    );
    let mut fake = passing();
    assert_eq!(wgsl_gate(&mut fake), Ok(()), "{:?}", fake.ran);
    let validated = wgsl_command(&["crates/visual-core/src/display.wgsl"]).line();
    assert!(
        fake.ran.contains(&validated) && fake.ran.contains(&refusal()),
        "the valid shaders together, the fixture alone: {:?}",
        fake.ran
    );
}

#[test]
fn says_the_valid_shaders_refused() {
    let mut fake = passing();
    let validated = wgsl_command(&["crates/visual-core/src/display.wgsl"]).line();
    fake.fail = Some(validated.clone());
    assert_eq!(
        wgsl_gate(&mut fake),
        refused(&[format!("`{validated}` exited with code 3")]),
        "wgsl-test said each problem itself"
    );
}

#[test]
fn validates_nothing_together_when_only_fixtures_are_tracked() {
    let mut fake = passing();
    drop(fake.files.remove("crates/visual-core/src/display.wgsl"));
    assert_eq!(wgsl_gate(&mut fake), Ok(()));
    assert!(
        !fake
            .ran
            .iter()
            .any(|line| line.contains("validate") && *line != refusal()),
        "no run without a valid shader to give it: {:?}",
        fake.ran
    );
}

#[test]
fn requires_each_invalid_fixture_tracked_and_refused() {
    let [(invalid, _)] = INVALID_SHADERS;
    let mut untracked = passing();
    drop(untracked.files.remove(invalid));
    assert_eq!(
        wgsl_gate(&mut untracked),
        refused(&[format!(
            "{invalid}: listed among the invalid shaders (INVALID_SHADERS) but not tracked"
        )]),
        "a stale entry"
    );
    let mut accepted = passing();
    drop(accepted.files.remove(&format!("rejects {}", refusal())));
    assert_eq!(
        wgsl_gate(&mut accepted),
        refused(&[format!(
            "{invalid}: listed among the invalid shaders (INVALID_SHADERS), but valid"
        )]),
        "a fixture turned valid"
    );
    let mut silent = passing();
    drop(silent.files.remove(&format!("stdout {}", refusal())));
    let mut other_name = passing();
    drop(other_name.files.insert(
        format!("stdout {}", refusal()),
        "other.wgsl:1:1: invalid: x\n".to_owned(),
    ));
    let mut two_lines = passing();
    drop(two_lines.files.insert(
        format!("stdout {}", refusal()),
        format!("{invalid}:4:12: invalid: x\n{invalid}:4:12: invalid: y\n"),
    ));
    let answer = |line: String| {
        let mut fake = passing();
        drop(fake.files.insert(format!("stdout {}", refusal()), line));
        fake
    };
    let parsed = answer(format!("{invalid}:4:12: parse error: x\n"));
    let elsewhere = answer(format!("{invalid}:4:11: invalid: x\n"));
    let unread = answer(format!("{invalid}: cannot read: x\n"));
    let mut usage = passing();
    drop(usage.files.remove(&format!("rejects {}", refusal())));
    usage.refuse = Some(refusal());
    for (mut fake, code, why) in [
        (silent, 1_i32, "refused without its problem line"),
        (other_name, 1_i32, "a line naming another file"),
        (two_lines, 1_i32, "two lines for one file"),
        (parsed, 1_i32, "refused by the parser, not the validator"),
        (elsewhere, 1_i32, "refused at another place"),
        (unread, 1_i32, "not read"),
        (usage, 2_i32, "a usage error"),
    ] {
        assert_eq!(
            wgsl_gate(&mut fake),
            refused(&[format!(
                "{invalid}: an invalid shader not refused as wgsl-test refuses one: `{}` \
                 exited with code {code}",
                refusal()
            )]),
            "{why}"
        );
    }
}

#[test]
fn says_every_problem_at_once_and_why_the_shaders_cannot_be_listed() {
    let mut both = passing();
    let validated = wgsl_command(&["crates/visual-core/src/display.wgsl"]).line();
    both.fail = Some(validated.clone());
    drop(both.files.remove(&format!("rejects {}", refusal())));
    let [(invalid, _)] = INVALID_SHADERS;
    assert_eq!(
        wgsl_gate(&mut both),
        refused(&[
            format!("`{validated}` exited with code 3"),
            format!("{invalid}: listed among the invalid shaders (INVALID_SHADERS), but valid"),
        ]),
        "the valid ones' failure and the fixture's"
    );
    let mut unlisted = passing();
    unlisted.broken = Some("list *.wgsl".to_owned());
    assert!(
        wgsl_gate(&mut unlisted).is_err_and(|failure| failure.summary.contains("the WGSL shaders")),
        "git's failure, naming what it listed"
    );
}

#[test]
fn requires_each_copy_identical_to_its_original() {
    assert_eq!(
        COPIES,
        [(
            "tooling/wgsl/wgsl-test/tests/data/safe-message-vectors.json",
            "spec/safe-message-vectors.json"
        )],
        "the shared vectors of the safe message rule"
    );
    let same = ("{}\n".to_owned(), "{}\n".to_owned());
    assert_eq!(copies_gate(&[same]), Ok(()), "identical");
    let [(copy, original)] = COPIES;
    for (texts, why) in [
        (("{}".to_owned(), "{}\n".to_owned()), "a byte missing"),
        (("{}\r\n".to_owned(), "{}\n".to_owned()), "another line end"),
    ] {
        assert_eq!(
            copies_gate(&[texts]),
            crate::steps::verdict(
                "every copy the WGSL tools hold identical to the project's file",
                vec![format!("{copy} differs from {original}, which it copies")],
            ),
            "{why}"
        );
    }
    let mut fake = passing();
    drop(
        fake.files
            .insert(copy.to_owned(), "{\"x\": 1}\n".to_owned()),
    );
    let ran = |checked: &mut crate::tests::Fake| {
        crate::actions::run_action(
            &crate::Action::WgslCopiesGate,
            checked,
            &mut Vec::new(),
            &mut None,
        )
    };
    assert_eq!(
        ran(&mut passing()),
        Ok(()),
        "the copy and its original, read"
    );
    assert!(ran(&mut fake).is_err(), "a copy changed, refused");
    for missing in [copy, original] {
        let mut unread = passing();
        drop(unread.files.remove(missing));
        assert!(
            ran(&mut unread).is_err_and(|failure| failure.summary.contains(missing)),
            "{missing} unread, named"
        );
    }
}
