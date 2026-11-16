//! Tests of the order of the tests in mutant runs: the slow group's form, the baseline's
//! times, the suggestion and the nextest configuration gate.

use super::{
    FAST_TEST_MILLIS, OVERRIDE_HEADER, SLOW_PRIORITY_LINE, SLOW_TEST_MILLIS,
    SLOW_TEST_SUGGESTED_MILLIS, TestName, baseline_times, millis, nextest_configs_gate,
    order_suggestion, plain, seconds, slow_group,
};
use crate::steps::Failure;
use crate::workspace::{
    CargoWorkspace, GPU_MUTANTS_OUT, GPU_MUTANTS_REPORT, GPU_WORKSPACE, MUTANTS_OUT,
};

/// The test `test` of the binary `binary`.
fn name(binary: &str, test: &str) -> TestName {
    TestName {
        binary: binary.to_owned(),
        path: test.to_owned(),
    }
}

/// The entry of the slow group naming `test` of `binary`, as the configuration writes it.
fn entry(binary: &str, test: &str) -> String {
    format!(
        "{OVERRIDE_HEADER}\nfilter = 'binary_id(={binary}) & test(={test})'\n{SLOW_PRIORITY_LINE}\n"
    )
}

/// The failure of a gate that found these `problems`, more than one.
fn problems(problems: &[String]) -> Result<(), Failure> {
    Err(Failure {
        summary: format!("{} problems, listed above", problems.len()),
        details: problems.to_vec(),
        usage: false,
    })
}

#[test]
fn the_thresholds_keep_a_band_of_a_factor_of_two_each_way() {
    assert_eq!(SLOW_TEST_MILLIS, 500);
    assert_eq!(
        SLOW_TEST_SUGGESTED_MILLIS, 1_000,
        "twice the group's threshold"
    );
    assert_eq!(FAST_TEST_MILLIS, 250, "half the group's threshold");
    assert_eq!(OVERRIDE_HEADER, "[[profile.default.overrides]]");
    assert_eq!(SLOW_PRIORITY_LINE, "priority = -1");
}

#[test]
fn each_workspace_has_its_report_and_its_configuration_at_its_root() {
    assert_eq!(CargoWorkspace::Root.mutants_report(), MUTANTS_OUT);
    assert_eq!(
        CargoWorkspace::Gpu.mutants_report(),
        format!("{GPU_MUTANTS_OUT}/mutants.out")
    );
    assert_eq!(GPU_MUTANTS_REPORT, "mutants.out/gpu/mutants.out");
    assert_eq!(
        CargoWorkspace::Root.nextest_config(),
        ".config/nextest.toml"
    );
    assert_eq!(
        CargoWorkspace::Gpu.nextest_config(),
        format!("{GPU_WORKSPACE}/.config/nextest.toml")
    );
}

#[test]
fn reads_plain_names_only() {
    assert!(plain("audio-testkit::plan_laws"));
    assert!(plain("tests::a_1"));
    assert!(!plain(""), "empty");
    for refused in ["a b", "a)", "a'", "a|b", "a&b", "é"] {
        assert!(!plain(refused), "{refused}");
    }
}

#[test]
fn reads_the_slow_group_in_order_with_its_comments_and_blank_lines() {
    let config = format!(
        "# The slow group.\n\n{}\n# Another.\n{}",
        entry("audio-core", "limiter::tests::b"),
        entry("audio-core::binary", "tests::a")
    );
    assert_eq!(
        slow_group("n.toml", &config),
        Ok(vec![
            name("audio-core", "limiter::tests::b"),
            name("audio-core::binary", "tests::a")
        ])
    );
    assert_eq!(slow_group("n.toml", ""), Ok(Vec::new()), "an empty group");
    assert_eq!(
        slow_group(
            "n.toml",
            &format!("{}{}", entry("a", "t::b"), entry("b", "t::a"))
        ),
        Ok(vec![name("a", "t::b"), name("b", "t::a")]),
        "by binary first"
    );
}

#[test]
fn refuses_anything_but_the_slow_group() {
    assert_eq!(
        slow_group("n.toml", "[profile.default]\nretries = 2\n").map(|_| ()),
        problems(&[
            "n.toml:1: `[profile.default]`, where an entry starts with \
             `[[profile.default.overrides]]`: the file holds the slow group only"
                .to_owned(),
            "n.toml:2: `retries = 2`, where an entry starts with \
             `[[profile.default.overrides]]`: the file holds the slow group only"
                .to_owned(),
        ]),
        "another setting"
    );
    assert_eq!(
        slow_group(
            "n.toml",
            &format!("{OVERRIDE_HEADER}\nfilter = 'test(=a)'\n{SLOW_PRIORITY_LINE}\n")
        )
        .map_err(|failure| failure.summary),
        Err(
            "n.toml:2: `filter = 'test(=a)'`, where the entry names one test exactly: \
             `filter = 'binary_id(=<binary>) & test(=<test>)'`"
                .to_owned()
        ),
        "a filter that names more than one test"
    );
    assert_eq!(
        slow_group(
            "n.toml",
            &format!(
                "{OVERRIDE_HEADER}\nfilter = 'binary_id(=a) & test(=b)'\nslow-timeout = \"1s\"\n"
            )
        )
        .map_err(|failure| failure.summary),
        Err(
            "n.toml:3: `slow-timeout = \"1s\"`, where the entry ends with `priority = -1`"
                .to_owned()
        ),
        "another setting in an entry"
    );
    assert_eq!(
        slow_group(
            "n.toml",
            &format!("{OVERRIDE_HEADER}\nfilter = 'binary_id(=a) & test(=b)'\n")
        )
        .map_err(|failure| failure.summary),
        Err("n.toml:1: an entry without its filter and priority lines".to_owned()),
        "a priority missing"
    );
    assert_eq!(
        slow_group("n.toml", &format!("{OVERRIDE_HEADER}\n")).map_err(|failure| failure.summary),
        Err("n.toml:1: an entry without its filter and priority lines".to_owned()),
        "a filter missing"
    );
}

#[test]
fn refuses_an_entry_twice_or_out_of_order() {
    for (first, second, what) in [
        (("a", "t::b"), ("a", "t::b"), "twice"),
        (("a", "t::b"), ("a", "t::a"), "out of order by test"),
        (("b", "t::a"), ("a", "t::b"), "out of order by binary"),
    ] {
        assert_eq!(
            slow_group(
                "n.toml",
                &format!("{}{}", entry(first.0, first.1), entry(second.0, second.1))
            )
            .map_err(|failure| failure.summary),
            Err(format!(
                "n.toml:5: {} {} out of order: the entries are sorted by binary then test, \
                 each once",
                second.0, second.1
            )),
            "{what}"
        );
    }
}

#[test]
fn reads_a_filter_naming_one_plain_test_only() {
    for filter in [
        "filter = 'binary_id(=a)'",
        "filter = \"binary_id(=a) & test(=b)\"",
        "filter = 'binary_id(=a) & test(=b)",
        "filter = 'binary_id(=a) | test(=b)'",
        "filter = 'binary_id(=) & test(=b)'",
        "filter = 'binary_id(=a) & test(=b c)'",
        "filter = 'binary_id(=a) & test(=b) & test(=c)'",
    ] {
        assert!(
            slow_group(
                "n.toml",
                &format!("{OVERRIDE_HEADER}\n{filter}\n{SLOW_PRIORITY_LINE}\n")
            )
            .is_err(),
            "{filter}"
        );
    }
}

#[test]
fn reads_nextest_s_times_in_milliseconds() {
    assert_eq!(millis("0.018"), Some(18));
    assert_eq!(millis("90.745"), Some(90_745));
    assert_eq!(millis("18446744073709551.615"), Some(u64::MAX));
    for refused in [
        "1",
        ".018",
        "1.01",
        "1.0180",
        "1x.018",
        "+1.018",
        "1.-18",
        "1.+18",
        "18446744073709551.616",
        "18446744073709552.000",
        "99999999999999999999.000",
    ] {
        assert_eq!(millis(refused), None, "{refused}");
    }
}

#[test]
fn says_milliseconds_as_seconds() {
    assert_eq!(seconds(0), "0.000");
    assert_eq!(seconds(18), "0.018");
    assert_eq!(seconds(90_745), "90.745");
}

#[test]
fn reads_the_times_of_the_tests_that_passed_and_nothing_else() {
    let log = "\
    Starting 3 tests across 2 binaries
        PASS [   0.018s] (  1/3) audio-core math::tests::a
        SLOW [> 60.000s] (───────) audio-core limiter::tests::b
        PASS [  90.745s] (  2/3) audio-testkit::plan_laws tests::c
        FAIL [   0.010s] (  3/3) audio-core math::tests::d
        PASS [ 0.5s] (1/1) a t::short_decimals
        PASS [   1.000s] (1/1) a
        PASS [   1.000s] (1/1) a t::x extra
        PASS [   1.000s 1/1 a t::y
        PASS [   1.000s] (1/1)a t::z
PASS [   2.000s] (1/1) b t::unindented
";
    assert_eq!(
        baseline_times(log),
        [
            (name("audio-core", "math::tests::a"), 18),
            (name("audio-testkit::plan_laws", "tests::c"), 90_745),
            (name("b", "t::unindented"), 2_000),
        ]
    );
}

#[test]
fn suggests_moving_the_slow_tests_in_and_the_fast_ones_out() {
    let group = [
        name("a", "t::fast"),
        name("a", "t::gone"),
        name("a", "t::in_band"),
        name("a", "t::slow"),
        name("unmeasured", "t::x"),
    ];
    let times = [
        (name("a", "t::fast"), 249),
        (name("a", "t::in_band"), FAST_TEST_MILLIS),
        (name("a", "t::slow"), 5_000),
        (name("a", "t::new_slow"), SLOW_TEST_SUGGESTED_MILLIS),
        (name("a", "t::below"), 999),
        (name("b", "t::other"), 2_500),
    ];
    assert_eq!(
        order_suggestion("n.toml", &group, &times),
        [
            "  test order: 4 tests to move in the slow group of n.toml (moving them in saves up \
             to 3.500 s a mutant a faster test catches waits; a test slower than measured under \
             load may rather need speeding up):",
            "    run with the others: a t::fast (0.249 s)",
            "    run last: a t::new_slow (1.000 s)",
            "    run last: b t::other (2.500 s)",
            "    remove, no longer run: a t::gone",
        ]
    );
}

#[test]
fn suggests_nothing_when_the_group_is_as_measured_and_one_move_as_one() {
    let group = [name("a", "t::slow")];
    assert_eq!(
        order_suggestion("n.toml", &group, &[(name("a", "t::slow"), 600)]),
        Vec::<String>::new()
    );
    assert_eq!(
        order_suggestion("n.toml", &group, &[]),
        Vec::<String>::new(),
        "no baseline time: nothing judged"
    );
    assert_eq!(
        order_suggestion("n.toml", &[], &[(name("a", "t::slow"), 1_234)]),
        [
            "  test order: 1 test to move in the slow group of n.toml (moving them in saves up \
             to 1.234 s a mutant a faster test catches waits; a test slower than measured under \
             load may rather need speeding up):",
            "    run last: a t::slow (1.234 s)",
        ]
    );
}

#[test]
fn allows_each_workspace_s_nextest_configuration_holding_its_slow_group_only() {
    let root = CargoWorkspace::Root.nextest_config().to_owned();
    let gpu = CargoWorkspace::Gpu.nextest_config().to_owned();
    let configs = [
        (root.clone(), entry("a", "t::b")),
        (gpu.clone(), String::new()),
    ];
    let tracked = [root.clone(), gpu.clone()];
    assert_eq!(nextest_configs_gate(&configs, &tracked), Ok(()));
    let elsewhere = [
        root.clone(),
        gpu.clone(),
        "crates/audio-core/.config/nextest.toml".to_owned(),
        "nextest.toml".to_owned(),
    ];
    assert_eq!(
        nextest_configs_gate(&configs, &elsewhere),
        problems(&[
            "crates/audio-core/.config/nextest.toml: a nextest configuration outside a \
             workspace's own (`.config/nextest.toml` at its root), which no gate reads"
                .to_owned(),
            "nextest.toml: a nextest configuration outside a workspace's own \
             (`.config/nextest.toml` at its root), which no gate reads"
                .to_owned(),
        ]),
        "a nextest configuration anywhere else is refused"
    );
    assert_eq!(
        nextest_configs_gate(&configs, core::slice::from_ref(&root))
            .map_err(|failure| failure.summary),
        Err(format!(
            "{gpu}: missing, or not tracked by git: the workspace's slow group"
        )),
        "each workspace's is required"
    );
}

#[test]
fn refuses_a_nextest_configuration_holding_more_than_its_slow_group() {
    let root = CargoWorkspace::Root.nextest_config().to_owned();
    let gpu = CargoWorkspace::Gpu.nextest_config().to_owned();
    let tracked = [root.clone(), gpu.clone()];
    assert_eq!(
        nextest_configs_gate(
            &[
                (root.clone(), "retries = 2\n".to_owned()),
                (gpu.clone(), String::new())
            ],
            &tracked
        )
        .map_err(|failure| failure.summary),
        Err(format!(
            "{root}:1: `retries = 2`, where an entry starts with \
             `[[profile.default.overrides]]`: the file holds the slow group only"
        )),
        "one problem"
    );
    assert_eq!(
        nextest_configs_gate(
            &[
                (root.clone(), "retries = 2\n".to_owned()),
                (
                    gpu.clone(),
                    "[profile.default]\nfail-fast = false\n".to_owned()
                )
            ],
            &tracked
        ),
        problems(&[
            format!(
                "{root}:1: `retries = 2`, where an entry starts with \
                 `[[profile.default.overrides]]`: the file holds the slow group only"
            ),
            format!(
                "{gpu}:1: `[profile.default]`, where an entry starts with \
                 `[[profile.default.overrides]]`: the file holds the slow group only"
            ),
            format!(
                "{gpu}:2: `fail-fast = false`, where an entry starts with \
                 `[[profile.default.overrides]]`: the file holds the slow group only"
            ),
        ]),
        "every problem of every configuration"
    );
}
