//! Tests of the parts of the full check that CI runs as parallel jobs, and the gate that proves
//! they cover it.

use crate::tests::{
    FAKE_JOBS, Fake, PART_NAMES, added_package_steps, added_packages, command_lines, failed,
    parts_refused, visual_steps,
};
use crate::{
    Action, Cmd, Dir, Extent, Failure, Mode, Step, UNVIABLE_REVIEWED, check_steps, run_steps,
};

/// The steps of the part named `name` of the full check, with the fake's jobs.
fn part_named(name: &str) -> Vec<Step> {
    let part = crate::full_parts()
        .into_iter()
        .find(|part| part.name() == name)
        .expect("a part of that name");
    crate::part_steps(part, FAKE_JOBS)
}

/// Every part of the full check, named, with its steps.
fn every_part() -> Vec<(String, Vec<Step>)> {
    PART_NAMES
        .iter()
        .map(|name| ((*name).to_owned(), part_named(name)))
        .collect()
}

/// The steps of the full check, with the fake's jobs.
fn full_check() -> Vec<Step> {
    check_steps(Mode::Full { jobs: FAKE_JOBS })
}

#[test]
fn splits_the_full_check_into_named_parts() {
    assert_eq!(crate::MUTATION_SHARDS, 8, "eight mutation shards");
    assert_eq!(
        crate::full_parts()
            .into_iter()
            .map(crate::Part::name)
            .collect::<Vec<_>>(),
        PART_NAMES,
        "the parts, in order, the final one last"
    );
    assert_eq!(
        crate::Shard::all()
            .map(crate::Shard::index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6, 7],
        "every shard, as cargo-mutants counts them"
    );
    for name in PART_NAMES {
        let steps = part_named(name);
        let names: Vec<&str> = steps.iter().map(|step| step.name.as_str()).collect();
        assert_eq!(
            names.get(..3),
            Some(
                &[
                    "Tools and platform",
                    "No mutant left in place in the GPU workspace",
                    "Working tree recorded"
                ][..]
            ),
            "{name}: the tools, the sources and the tree first"
        );
        assert_eq!(
            names.last(),
            Some(&"Working tree unchanged by the checks"),
            "{name}: the tree checked last"
        );
    }
}

#[test]
fn runs_each_part_of_the_full_check_as_its_own_job() {
    let lines = |name: &str| command_lines(&part_named(name));
    let frame = |inner: &[&str]| {
        let mut all = vec![
            "preflight, no GPU driver".to_owned(),
            "mutant left gate".to_owned(),
            "tree recorded".to_owned(),
        ];
        all.extend(inner.iter().map(|line| (*line).to_owned()));
        all.push("tree unchanged".to_owned());
        all
    };
    assert_eq!(
        lines("mutation-3"),
        frame(&[
            "fresh mutants.out",
            "Root: cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --shard 3/8 --sharding=round-robin --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
        ]),
        "one shard of the mutants, round-robin"
    );
    assert_eq!(
        part_named("mutation-3")
            .get(4)
            .map(|step| step.name.as_str()),
        Some("Rust mutation testing (shard 3/8)"),
        "named after its shard"
    );
    assert_eq!(
        lines("final"),
        frame(&[
            "unviable gate over mutants.out/shards/unviable-mutation-0/unviable.txt mutants.out/shards/unviable-mutation-1/unviable.txt mutants.out/shards/unviable-mutation-2/unviable.txt mutants.out/shards/unviable-mutation-3/unviable.txt mutants.out/shards/unviable-mutation-4/unviable.txt mutants.out/shards/unviable-mutation-5/unviable.txt mutants.out/shards/unviable-mutation-6/unviable.txt mutants.out/shards/unviable-mutation-7/unviable.txt",
        ]),
        "the unviable mutants of every shard"
    );
    assert_eq!(
        lines("miri"),
        frame(&[&format!(
            "Root: cargo +nightly-2026-11-01 {} test --package visual-wasm --package alloc-counter --package audio-wasm --package web-page --locked -- --include-ignored",
            "miri"
        )]),
        "Miri alone"
    );
    assert_eq!(
        lines("tooling-oracle").get(3..6).map(<[String]>::to_vec),
        Some(
            [
                "fresh target/conformance",
                "Root: cargo run --package audio-cli --locked --quiet -- export conformance target/conformance",
                "Root: pnpm install --frozen-lockfile",
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "the oracle on the exported conformance files"
    );
    assert_eq!(
        lines("tooling-config").len(),
        4 + 6 + 3,
        "the frame, the JavaScript preparation (and no other) and the package's six steps: its \
         three scripts, its Stryker run and its two gates"
    );
    let gates = lines("gates");
    assert!(
        gates.contains(&"parts gate".to_owned())
            && gates.contains(&"workflow gate".to_owned())
            && gates.contains(&"Root: pnpm install --frozen-lockfile".to_owned())
            && !gates.iter().any(|line| line.contains("export conformance")),
        "the gates, with the JavaScript dependencies their license gate reads"
    );
    assert_eq!(
        crate::preparation_steps(),
        [
            "Tools and platform",
            "No mutant left in place in the GPU workspace",
            "Working tree recorded",
            "Fresh Rust mutation report",
            "Fresh conformance export directory",
            "Conformance export (native WAV files)",
            "JavaScript dependencies",
            "Fresh TypeScript report directories",
            "Fresh TypeScript build outputs",
            "Build of bindings/audio-wasm, for the packages that use it",
            "Build of bindings/visual-wasm, for the packages that use it",
            "Build of bindings/web-page, for the packages that use it",
            "Working tree unchanged by the checks",
        ]
        .map(str::to_owned)
        .to_vec(),
        "only preparation runs in several parts, never a proof"
    );
}

#[test]
fn requires_a_gpu_driver_only_of_the_parts_that_run_gpu_code() {
    let mut requiring = Vec::new();
    for name in PART_NAMES {
        let steps = part_named(name);
        let runs = steps.iter().any(|step| step.action.runs_gpu_code());
        let preflights: Vec<&Action> = steps
            .iter()
            .map(|step| &step.action)
            .filter(|action| matches!(action, Action::Preflight(_)))
            .collect();
        let driver = if runs {
            crate::GpuDriver::Required
        } else {
            crate::GpuDriver::Unneeded
        };
        assert_eq!(
            preflights,
            [&Action::Preflight(driver)],
            "{name}: one preflight, requiring a driver exactly when a step runs GPU code"
        );
        if runs {
            requiring.push(name);
        }
    }
    assert_eq!(
        requiring,
        ["rust", "coverage", "mutation-0"],
        "the GPU workspace's tests, benchmark and doc tests, its coverage, and its mutants \
         (the first shard's)"
    );
    assert_eq!(
        crate::parts_gate(&full_check(), &every_part(), &crate::preparation_steps()),
        Ok(()),
        "a part's preflight stands for the full check's, whatever driver it requires"
    );
}

#[test]
fn builds_what_the_playground_reads_for_its_part() {
    let frame = |inner: &[&str]| {
        let mut all = vec![
            "preflight, no GPU driver".to_owned(),
            "mutant left gate".to_owned(),
            "tree recorded".to_owned(),
        ];
        all.extend(inner.iter().map(|line| (*line).to_owned()));
        all.push("tree unchanged".to_owned());
        all
    };
    assert_eq!(
        command_lines(&part_named("apps-playground")),
        frame(&[
            "Root: pnpm install --frozen-lockfile",
            "fresh tooling/config/coverage tooling/config/reports/mutation tooling/oracle/coverage tooling/oracle/reports/mutation tooling/wasm-build/coverage tooling/wasm-build/reports/mutation bindings/audio-wasm/coverage bindings/audio-wasm/reports/mutation bindings/visual-wasm/coverage bindings/visual-wasm/reports/mutation bindings/web-page/coverage bindings/web-page/reports/mutation apps/playground/coverage apps/playground/reports/mutation apps/roadmap/coverage apps/roadmap/reports/mutation",
            "fresh bindings/audio-wasm/dist bindings/visual-wasm/dist bindings/web-page/dist",
            "Package(\"bindings/audio-wasm\"): pnpm run build",
            "Package(\"bindings/visual-wasm\"): pnpm run build",
            "Package(\"bindings/web-page\"): pnpm run build",
            "Package(\"apps/playground\"): pnpm run typecheck",
            "Package(\"apps/playground\"): pnpm run lint",
            "Package(\"apps/playground\"): pnpm run test",
            "Package(\"apps/playground\"): pnpm run build",
            "Package(\"apps/playground\"): pnpm run test:e2e",
            "Package(\"apps/playground\"): pnpm exec stryker run stryker.config.json --incremental --incrementalFile reports/stryker-incremental.json --force",
            "typescript reports gate of apps/playground",
            "typescript scope gate of apps/playground",
        ]),
        "the playground on the builds of the engine, the visual library and the page \
         module, the visual library's first, without their checks"
    );
}

#[test]
fn builds_the_visual_library_for_the_page_module_s_part() {
    let lines = |name: &str| command_lines(&part_named(name));
    assert_eq!(
        lines("bindings-web-page").get(6..8).map(<[String]>::to_vec),
        Some(
            [
                "Package(\"bindings/visual-wasm\"): pnpm run build",
                "Package(\"bindings/web-page\"): pnpm run build",
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "the page module on the visual library's build, without its checks, then its own"
    );
}

#[test]
fn the_parts_run_every_step_of_the_full_check_once() {
    let preparation = crate::preparation_steps();
    assert_eq!(
        crate::parts_gate(&full_check(), &every_part(), &preparation),
        Ok(()),
        "the real parts"
    );
    let mut steps_run = 0;
    for (_, steps) in every_part() {
        steps_run += steps.len();
    }
    assert_eq!(
        steps_run,
        (18 + added_packages()) * 4
            + (122 + visual_steps() + added_package_steps() - 4 - 7 - 2)
            + 8 * 2
            + 1
            + 2
            + (6 + added_packages()) * 3
            + crate::PACKAGES
                .iter()
                .flat_map(|package| package.needs)
                .filter(|setup| matches!(setup, crate::Setup::Build(_)))
                .count()
            + 8
            + 2
            + 1
            + 2,
        "each part's frame (four steps); every other step of the 122 (more with the visual \
         library's) once, but the seven \
         preparations (and each build prepared for the packages beyond the engine's, a step \
         and a preparation both), the mutation run and its unviable gate; in each of the eight \
         shards, the report emptied and the shard's run; the gate over the shards; the \
         conformance export (two steps) for the oracle; the JavaScript preparation (three \
         steps) for the gates and each package; each build a package uses, in its part; the \
         GPU workspace's eight steps (formatting, lints, documentation, tests, supply chain, \
         CPU reference, time benchmark, mutation testing); the build-time code gate; the \
         visual corpus's mutation report emptied and its run, in the first shard"
    );
}

#[test]
fn refuses_parts_that_miss_or_repeat_a_step() {
    let [first, second, third] = abc();
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        Ok(()),
        "each step once"
    );
    assert_eq!(
        gate(
            &[("x", vec![first.clone()]), ("y", vec![third.clone()])],
            &[]
        ),
        parts_refused(&["`b` runs in no part"]),
        "a step missing"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![second.clone(), third.clone()])
            ],
            &[]
        ),
        parts_refused(&["`b` runs in 2 parts"]),
        "a proof run twice"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), second.clone()]),
                ("y", vec![second.clone(), third.clone()])
            ],
            &["b"]
        ),
        Ok(()),
        "a preparation, run where it is needed"
    );
    assert_eq!(
        gate(
            &[("x", vec![first.clone()]), ("y", vec![third.clone()])],
            &["b"]
        ),
        parts_refused(&["`b` runs in no part"]),
        "a preparation runs somewhere"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![second.clone(), first.clone()]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        parts_refused(&["x: `a` runs out of the order of the full check"]),
        "out of order"
    );
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), first, second]),
                ("y", vec![third])
            ],
            &["a"]
        ),
        parts_refused(&["x: `a` runs out of the order of the full check"]),
        "twice in one part"
    );
}

#[test]
fn refuses_parts_with_steps_or_names_of_their_own() {
    let [first, second, third] = abc();
    let renamed = Step {
        name: "B".to_owned(),
        ..second.clone()
    };
    let changed = plain("b", "b", &["--more"]);
    assert_eq!(
        gate(
            &[
                ("x", vec![first.clone(), renamed, changed]),
                ("y", vec![third.clone()])
            ],
            &[]
        ),
        parts_refused(&[
            "x: `B` is no step of the full check",
            "x: `b` is no step of the full check",
            "`b` runs in no part",
        ]),
        "a step of a part is a step of the full check, name and action"
    );
    assert_eq!(
        gate(&[("x", vec![first, second]), ("x", vec![third])], &[]),
        parts_refused(&["two parts are named x"]),
        "unique part names"
    );
}

/// A shard `value` of the mutation step `m`, named `name`, as a step of a part.
fn shard(name: &str, value: &str, sharding: &str) -> Step {
    mutants(
        name,
        &[
            "mutants",
            "--workspace",
            "--shard",
            value,
            sharding,
            "--",
            "x",
        ],
    )
}

/// Round-robin sharding, as cargo-mutants reads it.
const ROUND_ROBIN: &str = "--sharding=round-robin";

#[test]
fn refuses_shards_that_do_not_split_the_mutants_exactly() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/2)", "1/2", rr),
            &["s0", "s1"]
        ),
        Ok(()),
        "two shards of two, their reports read by the final part"
    );
    assert_eq!(
        crate::shard_unviable_report("s0"),
        "mutants.out/shards/unviable-s0/unviable.txt",
        "where the final job downloads the artifact of a shard"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 0/2)", "0/2", rr),
            &["s0", "s1"]
        ),
        parts_refused(&[
            "shard 0/2 of `m` runs in 2 parts: s0, s1",
            "shard 1/2 of `m` runs in no part",
        ]),
        "a shard twice, another never"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 2/2)", "2/2", rr),
            &["s0", "s1"]
        ),
        parts_refused(&[
            "s1: shard 2/2 of `m` is no shard",
            "shard 1/2 of `m` runs in no part",
        ]),
        "an index out of range"
    );
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/3)", "1/3", rr),
            &["s0", "s1"]
        ),
        parts_refused(&["`m` is split into shards of different counts"]),
        "two counts"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("w", vec![full[0].clone()]),
            ("f", vec![across(&["s0"])]),
        ]),
        parts_refused(&["`m` runs both whole (w) and in shards"]),
        "whole and in shards"
    );
    assert_eq!(
        gate(vec![
            (
                "s",
                vec![
                    shard("m (shard 0/2)", "0/2", rr),
                    shard("m (shard 1/2)", "1/2", rr)
                ]
            ),
            ("f", vec![across(&["s", "s"])]),
        ]),
        parts_refused(&[
            "s: runs a second shard, whose report would replace the first",
            "s: `m (shard 1/2)` runs out of the order of the full check",
        ]),
        "one shard per part, since each writes the same report"
    );
}

#[test]
fn refuses_a_final_part_that_reads_not_every_shard() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    assert_eq!(
        two(
            shard("m (shard 0/2)", "0/2", rr),
            shard("m (shard 1/2)", "1/2", rr),
            &["s0"]
        ),
        parts_refused(&[
            "the unviable-mutant gate over the shards reads mutants.out/shards/unviable-s0/unviable.txt, \
             not the reports of the shards: mutants.out/shards/unviable-s0/unviable.txt \
             mutants.out/shards/unviable-s1/unviable.txt"
        ]),
        "a shard's unviable mutants unread"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![across(&["s0"])]),
            ("g", vec![across(&["s0"])]),
        ]),
        parts_refused(&["`u` runs in 2 parts"]),
        "the gate over the shards once"
    );
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![full[1].clone()]),
        ]),
        Ok(()),
        "the gate run whole stands for itself (the full check, in one part)"
    );
    assert_eq!(
        gate(vec![("s0", vec![shard("m (shard 0/1)", "0/1", rr)])]),
        parts_refused(&["`u` runs in no part"]),
        "the unviable mutants of the shards unread"
    );
}

#[test]
fn refuses_shards_written_otherwise_than_cargo_mutants_reads_them() {
    let rr = ROUND_ROBIN;
    let gate = shards_gate;
    let full = mutation_check();
    for (value, sharding) in [
        ("+0/1", rr),
        ("00/1", rr),
        ("0/01", rr),
        ("0-1", rr),
        ("x/1", rr),
        ("0/x", rr),
        ("0/1", "--sharding=slice"),
    ] {
        let named = format!("m (shard {value})");
        assert_eq!(
            gate(vec![
                ("s0", vec![shard(&named, value, sharding)]),
                ("f", vec![full[1].clone()]),
            ]),
            parts_refused(&[
                &format!("s0: `{named}` is no step of the full check"),
                "`m` runs in no part",
            ]),
            "{value} {sharding}: only the canonical shard of cargo-mutants, round-robin"
        );
    }
    let cut = mutants(
        "m (shard 0/1)",
        &["mutants", "--workspace", "--shard", "0/1"],
    );
    let ended = mutants("m (shard 0/1)", &["mutants", "--shard"]);
    for step in [cut, ended] {
        assert_eq!(
            gate(vec![("s0", vec![step]), ("f", vec![full[1].clone()])])
                .map_err(|failure| failure.summary),
            Err("the parts of the full check do not run each of its steps once: 2 problems, listed above".to_owned()),
            "a command cut short is no shard"
        );
    }
    let misnamed = shard("m (shard 1/1)", "0/1", rr);
    assert_eq!(
        gate(vec![("s0", vec![misnamed]), ("f", vec![full[1].clone()])]),
        parts_refused(&[
            "s0: `m (shard 1/1)` is no step of the full check",
            "`m` runs in no part",
        ]),
        "named after the shard it runs"
    );
    let misnamed_gate = Step {
        name: "u".to_owned(),
        ..across(&["s0"])
    };
    let partial = Step {
        name: "u (every shard)".to_owned(),
        action: Action::UnviableGate(Extent::Partial),
    };
    assert_eq!(
        gate(vec![
            ("s0", vec![shard("m (shard 0/1)", "0/1", rr)]),
            ("f", vec![misnamed_gate, partial]),
        ]),
        parts_refused(&[
            "f: `u` is no step of the full check",
            "f: `u (every shard)` is no step of the full check",
            "`u` runs in no part",
        ]),
        "the gate over the shards is named after the complete gate, and stands for no other"
    );
}

#[test]
fn reviews_the_unviable_mutants_of_every_shard_together() {
    let reports: Vec<String> = ["s0", "s1"]
        .iter()
        .map(|part| crate::shard_unviable_report(part))
        .collect();
    let step = [Step {
        name: "Shards".to_owned(),
        action: Action::ShardsUnviableGate(reports.clone()),
    }];
    let with = |first: &str, second: &str, reviewed: &str| {
        let mut fake = Fake::default();
        for (path, text) in reports.iter().zip([first, second]) {
            let _new = fake.files.insert(path.clone(), text.to_owned());
        }
        let _new = fake
            .files
            .insert(UNVIABLE_REVIEWED.to_owned(), reviewed.to_owned());
        run_steps(&step, &mut fake, &mut Vec::new())
    };
    let one = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()";
    let other = "crates/a/src/c.rs:4:1: replace g -> U with Default::default()\n";
    let reviews = "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
                   crates/a/src/c.rs: replace g -> U with Default::default() -- U has none\n";
    assert_eq!(
        with(one, other, reviews),
        Ok(()),
        "each shard's own, reviewed"
    );
    assert_eq!(
        with(one, "", reviews),
        failed(
            "Shards: unviable mutants must be reviewed in mutants-unviable.txt: stale review: \
             crates/a/src/c.rs: replace g -> U with Default::default()"
        ),
        "a review no shard needs is stale, as after a complete run"
    );
    assert_eq!(
        with(one, one, reviews),
        failed(
            "Shards: unviable mutants must be reviewed in mutants-unviable.txt: 2 problems, \
             listed above\n\
             not reviewed: crates/a/src/b.rs: replace f -> T with Default::default() \
             (2 found, 1 reviewed)\n\
             stale review: crates/a/src/c.rs: replace g -> U with Default::default()"
        ),
        "the shards counted together, a report without a final newline kept apart"
    );
    let mut lacking = Fake::default();
    let _new = lacking.files.insert(reports[0].clone(), one.to_owned());
    assert_eq!(
        run_steps(&step, &mut lacking, &mut Vec::new()),
        failed(format!("Shards: cannot read {}: missing", reports[1])),
        "a shard that wrote no report"
    );
}

/// A step that runs `program`.
fn plain(name: &str, program: &'static str, args: &[&str]) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::Run(Cmd::new(program, args, Dir::Root)),
    }
}

/// A step that runs cargo-mutants with `args`, then its doc-test pass.
fn mutants(name: &str, args: &[&str]) -> Step {
    Step {
        name: name.to_owned(),
        action: Action::RustMutants(Cmd::new("cargo", args, Dir::Root)),
    }
}

/// The steps `a`, `b` and `c`: a full check of three steps.
fn abc() -> [Step; 3] {
    [
        plain("a", "a", &[]),
        plain("b", "b", &[]),
        plain("c", "c", &[]),
    ]
}

/// The parts gate on `parts` of the full check [`abc`], with `preparation`.
fn gate(parts: &[(&str, Vec<Step>)], preparation: &[&str]) -> Result<(), Failure> {
    let parts: Vec<(String, Vec<Step>)> = parts
        .iter()
        .map(|(name, steps)| ((*name).to_owned(), steps.clone()))
        .collect();
    let preparation: Vec<String> = preparation.iter().map(|name| (*name).to_owned()).collect();
    crate::parts_gate(&abc(), &parts, &preparation)
}

/// The unviable-mutant gate over the reports of the parts `parts`.
fn across(parts: &[&str]) -> Step {
    Step {
        name: "u (every shard)".to_owned(),
        action: Action::ShardsUnviableGate(
            parts
                .iter()
                .map(|part| crate::shard_unviable_report(part))
                .collect(),
        ),
    }
}

/// A full check of a mutation step `m` and its unviable-mutant gate `u`.
fn mutation_check() -> [Step; 2] {
    [
        mutants("m", &["mutants", "--workspace", "--", "x"]),
        Step {
            name: "u".to_owned(),
            action: Action::UnviableGate(Extent::Complete),
        },
    ]
}

/// The parts gate on `parts` of [`mutation_check`], without preparation.
fn shards_gate(parts: Vec<(&str, Vec<Step>)>) -> Result<(), Failure> {
    let parts: Vec<(String, Vec<Step>)> = parts
        .into_iter()
        .map(|(name, steps)| (name.to_owned(), steps))
        .collect();
    crate::parts_gate(&mutation_check(), &parts, &[])
}

/// The parts gate on two shard parts `s0` and `s1` running `first` and `second`, and a
/// final part reading the reports of `reports`.
fn two(first: Step, second: Step, reports: &[&str]) -> Result<(), Failure> {
    shards_gate(vec![
        ("s0", vec![first]),
        ("s1", vec![second]),
        ("f", vec![across(reports)]),
    ])
}
