//! Tests of the GPU's time benchmark: the command on a fake outside world, the baseline's
//! text, and the measurement and files of the real one.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use std::io;

use visual_boxes::Square;
use visual_core::{ImageRate, Seconds};

use super::{
    BASELINE, BLESS_CONDITION, BOUND_PERCENT, BenchError, BenchIo, CHECKED, CONCURRENT, Failure,
    Gpu, Held, IDLE, IMAGE, LOADED, POWER_CONDITION, REPEATS, Runs, SAFETY, SHOWN, SQUARE_CYCLE,
    SQUARE_RATE, SQUARE_SIDE, System, TRIES, Times, Timing, VERSION, Version, bound_percent,
    format_baseline, measure, measured_square, median_of, one_line, parse_baseline, run, usage,
    whole_nanoseconds, within,
};
use crate::GpuSquare;

/// A fake outside world: the GPU opened, the system, what measuring gives, the baseline
/// held, and what was written.
#[derive(Debug, Default)]
struct Fake {
    /// The GPU opened, its name and driver, or a failure's words; `None` for the measured
    /// times' GPU (`GPU A (metal)` without times), driver `-`.
    gpu: Option<Result<(String, String), String>>,
    /// Whether the GPU opened has no clock its programs can read.
    clockless: bool,
    /// The system's version, or a failure's words; `None` for [`SYSTEM`].
    system: Option<Result<String, String>>,
    /// How many times the GPU was measured.
    measures: usize,
    /// What measuring gives: a time or a failure's words; `None` for a failure (nothing to
    /// measure).
    measured: Option<Result<Times, String>>,
    /// The baseline's text, `None` when there is no file; `Err` when it cannot be read.
    held: Option<Result<String, io::ErrorKind>>,
    /// Whether writing fails.
    read_only: bool,
    /// What was written.
    written: Vec<String>,
}

impl super::sealed::Sealed for Fake {}

impl BenchIo for Fake {
    fn open(&mut self) -> Result<Gpu, Failure> {
        let named = match &self.measured {
            Some(Ok(times)) => times.adapter.clone(),
            _ => "GPU A (metal)".to_owned(),
        };
        match self
            .gpu
            .clone()
            .unwrap_or_else(|| Ok((named, "-".to_owned())))
        {
            Ok((adapter, driver)) => Ok(Gpu {
                adapter,
                driver,
                clock: !self.clockless,
            }),
            Err(words) => Err(words.into()),
        }
    }

    fn system(&mut self) -> io::Result<String> {
        self.system
            .clone()
            .unwrap_or_else(|| Ok(SYSTEM.to_owned()))
            .map_err(io::Error::other)
    }

    fn measure(&mut self) -> Result<Times, Failure> {
        self.measures = self.measures.saturating_add(1);
        match self
            .measured
            .clone()
            .unwrap_or_else(|| Err("nothing to measure".to_owned()))
        {
            Ok(times) => Ok(times),
            Err(words) => Err(words.into()),
        }
    }

    fn read_baseline(&mut self) -> io::Result<Option<String>> {
        match self.held.clone() {
            None => Ok(None),
            Some(Ok(text)) => Ok(Some(text)),
            Some(Err(kind)) => Err(io::Error::new(kind, "refused")),
        }
    }

    fn write_baseline(&mut self, text: &str) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::other("read-only"));
        }
        self.written.push(text.to_owned());
        Ok(())
    }
}

/// A median of `nanoseconds` measured in `tries` renders.
const fn timing(nanoseconds: u64, tries: usize) -> Timing {
    Timing::Measured { nanoseconds, tries }
}

/// Times of the GPU `adapter`, `checked` nanoseconds at the checked size.
fn times(adapter: &str, checked: u64) -> Times {
    Times {
        adapter: adapter.to_owned(),
        checked: timing(checked, 15),
        shown: [timing(7_000, 15), timing(12_000, 16), timing(30_000, 17)],
        display: timing(40_000, 18),
    }
}

/// The system the fake runs on, unless told otherwise.
const SYSTEM: &str = "macOS 26.5.2";

/// What the baseline holds of a GPU blessed at `time` on [`SYSTEM`], with no driver named.
fn entry(time: u64) -> Held {
    Held::new(time, SYSTEM.to_owned(), "-".to_owned())
}

/// The baseline `bless` writes of `times`, each a GPU's name and its time, on [`SYSTEM`].
fn held(times: &[(&str, u64)]) -> String {
    format_baseline(
        &times
            .iter()
            .map(|&(name, time)| (name.to_owned(), entry(time)))
            .collect(),
    )
}

/// Runs `args` on `fake`, and returns what it said and its outcome.
fn command(fake: &mut Fake, args: &[&str]) -> (String, Result<(), BenchError>) {
    let mut out = Vec::new();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let outcome = run(&args, fake, &mut out);
    (String::from_utf8(out).expect("UTF-8"), outcome)
}

/// A failure of the work, `line`.
fn failure(line: &str) -> Result<(), BenchError> {
    Err(BenchError {
        code: 1,
        line: line.to_owned(),
    })
}

#[test]
fn refuses_no_command_another_and_an_argument_and_prints_its_usage() {
    for (args, line) in [
        (
            &[][..],
            "no command given (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)",
        ),
        (
            &["run"][..],
            "unknown command \"run\" (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)",
        ),
        (
            &["check", "now"][..],
            "\"check\" takes no argument (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)",
        ),
        // A command holding a line break or a control character stays on one line, quoted.
        (
            &["che\nck\u{1b}", "now"][..],
            "\"che\\nck\\u{1b}\" takes no argument (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)",
        ),
    ] {
        let mut fake = Fake::default();
        assert_eq!(
            command(&mut fake, args),
            (
                String::new(),
                Err(BenchError {
                    code: 2,
                    line: line.to_owned()
                })
            ),
            "{args:?}"
        );
    }
    assert_eq!(
        command(&mut Fake::default(), &["help"]),
        (format!("{}\n", usage()), Ok(()))
    );
    assert_eq!(
        usage(),
        "usage: visual-gpu <check|bless|help>
  check  measure this GPU's time and compare it with spec/gpu-time-baseline.txt, within ±10% (3/2 times the largest deviation of 62 idle runs from their median, measured 2026-11-20; spec/benchmarks.md, \"Visual library\"), if it holds
         this GPU, on the system and driver it records (run on AC power, outside Low Power Mode)
  bless  measure this GPU's time and write it to spec/gpu-time-baseline.txt, with the system and driver: only
         on AC power, outside Low Power Mode, on the machine whose GPU it holds, with its runs measured again on a change (spec/benchmarks.md, \"Visual library\")
  help   print this help",
        "the usage states the bound the check applies, the runs it comes from, and the \
         condition of a bless"
    );
    // Finding H12-3 of the fifth deep review of visual block 2 (#34): each condition written
    // once, never retyped.
    assert!(
        usage().contains(&format!("(run {POWER_CONDITION})"))
            && usage().contains(&format!("only\n         {BLESS_CONDITION}, ")),
        "the usage states each condition from its constant"
    );
    assert!(
        BLESS_CONDITION.starts_with(POWER_CONDITION),
        "{BLESS_CONDITION}"
    );
}

/// `runs`' deviation of `time` from their median, in hundredths of a percent rounded to the
/// nearest (half up), as the prose writes it, computed apart from [`bound_percent`]:
/// `⌊(20 000 × |2t − m| + m) / 2m⌋`, `m` the sum of the two middle medians.
fn hundredths(runs: Runs, time: u64) -> u64 {
    let twice_median = runs.middle.0.saturating_add(runs.middle.1);
    time.saturating_mul(2)
        .abs_diff(twice_median)
        .saturating_mul(20_000)
        .saturating_add(twice_median)
        .div_euclid(twice_median.saturating_mul(2))
}

/// Hundredths of a percent as the prose writes them: `6.02`.
fn percent(hundredths: u64) -> String {
    format!(
        "{}.{:02}",
        hundredths.div_euclid(100),
        hundredths.rem_euclid(100)
    )
}

/// `runs`' median as the prose writes it, its thousands grouped by a space: `93 770.5`.
fn median_text(runs: Runs) -> String {
    let twice = runs.middle.0.saturating_add(runs.middle.1);
    let whole = twice.div_euclid(2);
    let half = if twice.rem_euclid(2) == 1 { ".5" } else { "" };
    format!(
        "{} {:03}{half}",
        whole.div_euclid(1000),
        whole.rem_euclid(1000)
    )
}

/// The figures of the runs, each as every text stating it writes it, from the constants
/// alone: the idle runs, the bound's derivation, the loaded runs and their spread.
fn figures() -> [String; 4] {
    let largest = hundredths(IDLE, IDLE.lowest).max(hundredths(IDLE, IDLE.highest));
    [
        format!(
            "{} {} runs (two sessions, {} ns: from {}% below to {}% above)",
            IDLE.count,
            IDLE.condition,
            median_text(IDLE),
            percent(hundredths(IDLE, IDLE.lowest)),
            percent(hundredths(IDLE, IDLE.highest))
        ),
        format!(
            "1.5 × {}% = {}%, rounded up to {BOUND_PERCENT}%",
            percent(largest),
            // 1.5 times, to the nearest hundredth: (3 × largest + 1) / 2.
            percent(
                largest
                    .saturating_mul(SAFETY.0)
                    .saturating_add(1)
                    .div_euclid(SAFETY.1)
            )
        ),
        format!("{} runs {}", LOADED.count, LOADED.condition),
        format!(
            "from {}% below to {}% above the idle median",
            percent(hundredths(IDLE, LOADED.lowest)),
            percent(hundredths(IDLE, LOADED.highest))
        ),
    ]
}

/// The texts stating the runs' figures, beside the constants: the spec and this module's
/// own documentation. The changelog states them too, held by review rather than read here,
/// since a test reading it would make every commit's entry this crate's test data.
const STATING: [(&str, &str); 2] = [
    (
        "spec/benchmarks.md",
        include_str!("../../../spec/benchmarks.md"),
    ),
    ("crates/visual-gpu/src/bench.rs", include_str!("bench.rs")),
];

#[test]
fn states_the_runs_figures_as_the_constants_give_them() {
    let [idle, bound, loaded, spread] = figures();
    assert_eq!(
        [idle.as_str(), &bound, &loaded, &spread],
        [
            "62 idle runs (two sessions, 93 770.5 ns: from 6.02% below to 1.00% above)",
            "1.5 × 6.02% = 9.03%, rounded up to 10%",
            "61 runs under a full check's load",
            "from 2.51% below to 0.82% above the idle median",
        ]
    );
    // Each text states every figure: a figure changed in the constants fails here until
    // every text says it (the figures retired are retired-patterns.txt's).
    for (file, text) in STATING {
        for figure in [&idle, &bound, &loaded, &spread] {
            assert!(text.contains(figure.as_str()), "{file}: {figure}");
        }
    }
    // The prose's rounding, half up, at its edges.
    // (Finding H16-3 of the fifth deep review of visual block 2 (#34): the median of 10 000
    // gave no half, so that any rounding passed.)
    let runs = Runs {
        middle: (20_000, 20_000),
        ..IDLE
    };
    assert_eq!(
        [20_001, 20_002, 20_003, 20_005, 19_999, 20_000].map(|time| hundredths(runs, time)),
        [1, 1, 2, 3, 1, 0],
        "half a hundredth, a whole one, one and a half, two and a half, half below, none"
    );
    assert_eq!(median_text(runs), "20 000");
    assert_eq!(percent(7), "0.07");
}

#[test]
fn derives_its_bound_from_the_idle_runs_measured() {
    // The runs recorded (2026-11-20, an Apple M2 Pro on AC power, outside Low Power Mode):
    // 62 idle, from 6.02% below their median to 1.00% above, in hundredths.
    assert_eq!(
        (IDLE.date, IDLE.condition, IDLE.count),
        ("2026-11-20", "idle", 62)
    );
    assert_eq!(
        (
            hundredths(IDLE, IDLE.lowest),
            hundredths(IDLE, IDLE.highest)
        ),
        (602, 100)
    );
    // The bound: 1.5 times the largest deviation, the smallest whole percent above it:
    // 3 × d / (2 × m) lies in (9%, 10%].
    let twice_median = IDLE.middle.0 + IDLE.middle.1;
    let largest = (twice_median - 2 * IDLE.lowest).max(2 * IDLE.highest - twice_median);
    assert_eq!(SAFETY, (3, 2), "1.5");
    assert!(
        300 * largest <= 10 * 2 * twice_median && 300 * largest > 9 * 2 * twice_median,
        "{largest}"
    );
    assert_eq!((bound_percent(IDLE), BOUND_PERCENT), (10, 10));
    // Each side of the largest deviation decides it, rounded up, and runs measuring 0 ns
    // give a bound of 0%, never a division by zero.
    let runs = |lowest: u64, highest: u64| Runs {
        lowest,
        highest,
        middle: (1000, 1000),
        ..IDLE
    };
    for ((lowest, highest), percent) in [
        ((1000, 1000), 0),
        ((999, 1000), 1),
        ((1000, 1001), 1),
        ((960, 1001), 6),
        ((999, 1040), 6),
        ((934, 1000), 10),
        ((933, 1000), 11),
    ] {
        assert_eq!(
            bound_percent(runs(lowest, highest)),
            percent,
            "{lowest}..{highest}"
        );
    }
    assert_eq!(
        bound_percent(Runs {
            middle: (0, 0),
            ..runs(0, 0)
        }),
        0
    );
    // Each side by the bound: 5% below and above 1000 ns.
    for (checked, inside) in [
        (950, true),
        (949, false),
        (1050, true),
        (1051, false),
        (1000, true),
    ] {
        assert_eq!(within(checked, 1000, 5), inside, "{checked} ns");
    }
}

#[test]
fn load_does_not_move_the_time_but_a_second_heavy_run_does() {
    // The runs under a full check's load (its Rust tests, its GPU steps, its mutation
    // testing) stay within the idle runs' own deviations from the idle median: a bless needs
    // no idle machine, only AC power.
    assert_eq!(
        (LOADED.date, LOADED.condition, LOADED.count),
        ("2026-11-20", "under a full check's load", 61)
    );
    assert_eq!(
        (
            hundredths(IDLE, LOADED.lowest),
            hundredths(IDLE, LOADED.highest)
        ),
        (251, 82)
    );
    let worst = hundredths(IDLE, IDLE.lowest).max(hundredths(IDLE, IDLE.highest));
    assert!(hundredths(IDLE, LOADED.lowest) <= worst && hundredths(IDLE, LOADED.highest) <= worst);
    // The one run beside a second heavy check, 17.0% above, is beyond the bound: the bound
    // holds with no other heavy work running, which the failure line asks for.
    assert_eq!(hundredths(IDLE, CONCURRENT), 1700);
    assert!(!within(CONCURRENT, IDLE.middle.1, BOUND_PERCENT));
    assert_eq!(
        BLESS_CONDITION,
        "on AC power, outside Low Power Mode, on the machine whose GPU it holds"
    );
}

#[test]
fn checks_a_gpu_that_has_a_baseline_within_its_bound_and_renders_nothing_on_any_other() {
    let shown = "GPU A (metal): render 64x36: 7000 ns (15 of 15 measured) (shown)\nGPU A (metal): render 320x180: 12000 ns (15 of 16 measured) (shown)\nGPU A (metal): render 640x360: 30000 ns (15 of 17 measured) (shown)\nGPU A (metal): display 1280x720: 40000 ns (15 of 18 measured) (shown)\n";
    let baseline = held(&[("GPU A (metal)", 100_000), ("GPU B (vulkan)", 5)]);
    // At each bound and one nanosecond past it, on each side, from the bound itself.
    let base = 100_000_u64;
    let (fastest, slowest) = (
        base - base * BOUND_PERCENT / 100,
        base + base * BOUND_PERCENT / 100,
    );
    for (checked, outcome) in [
        (slowest, Ok(())),
        (fastest, Ok(())),
        (base, Ok(())),
        (slowest + 1, failure(&beyond(slowest + 1))),
        (fastest - 1, failure(&beyond(fastest - 1))),
    ] {
        let mut fake = Fake {
            measured: Some(Ok(times("GPU A (metal)", checked))),
            held: Some(Ok(baseline.clone())),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut fake, &["check"]),
            (
                format!(
                    "{shown}GPU A (metal): render 1280x720: {checked} ns (15 of 15 measured)\n"
                ),
                outcome
            ),
            "{checked}"
        );
        assert!(fake.written.is_empty(), "check writes nothing");
    }
    assert_eq!((fastest, slowest), (90_000, 110_000), "today's bound, ±10%");
    // A GPU the baseline does not hold (CI's software GPU, another machine): nothing
    // rendered, one line, a pass.
    let mut unknown = Fake {
        gpu: Some(Ok(("GPU C (vulkan)".to_owned(), "llvmpipe".to_owned()))),
        held: Some(Ok(baseline)),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut unknown, &["check"]),
        (
            "GPU C (vulkan): no baseline (the baseline holds GPU A (metal), GPU B (vulkan)): its time is not checked, nothing rendered\n".to_owned(),
            Ok(())
        ),
        "the GPUs it holds named, so that a GPU renamed by its driver is seen"
    );
    assert_eq!(unknown.measures, 0, "nothing measured");
    let mut missing = Fake {
        gpu: Some(Err("not to be opened".to_owned())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut missing, &["check"]).1,
        failure(&format!(
            "{BASELINE} is missing: no time can be checked{MISSING}"
        )),
        "no file: the check fails before opening the GPU, never passes as shown"
    );
    // The remedy followed: the file restored by its hint, the check passes on the maintainer's
    // Mac, its system and driver.
    let mut restored = Fake {
        measured: Some(Ok(times("Apple M2 Pro (metal)", 93_292))),
        held: Some(Ok(COMMITTED.to_owned())),
        ..Fake::default()
    };
    assert_eq!(command(&mut restored, &["check"]).1, Ok(()), "restored");
    // A file holding no GPU's time, the header `bless` writes alone, is refused, never read
    // as a baseline that checks nothing: the gate cannot be turned off by emptying it.
    for (text, why) in [
        (held(&[]), "no GPU's time"),
        (String::new(), "no GPU's time"),
    ] {
        let mut empty = Fake {
            measured: Some(Ok(times("GPU C (vulkan)", 1))),
            held: Some(Ok(text.clone())),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut empty, &["check"]),
            (
                String::new(),
                failure(&format!("{BASELINE}: {why}{RESTORE}"))
            ),
            "{text:?}"
        );
    }
}

/// The line of a time `checked` beyond the bound of a baseline of 100 000 ns, on the system
/// and driver it was blessed on.
fn beyond(checked: u64) -> String {
    format!(
        "GPU A (metal): {checked} ns, baseline 100000 ns: beyond ±10% (3/2 times the largest deviation of 62 idle runs from their median, measured 2026-11-20; spec/benchmarks.md, \"Visual library\") (run it again with no other heavy work running (see: cargo xtask check); still beyond, it is a regression of the work, or a change of the machine the baseline does not record: measure the runs again (spec/benchmarks.md, \"Visual library\"), then bless with the reason (see: cargo xtask bless-benchmarks))"
    )
}

#[test]
fn fails_when_the_system_or_the_driver_changed_since_the_bless() {
    // The bound was measured on the system and driver the baseline records: on another,
    // the check fails before rendering, naming the change and the procedure measuring the
    // bound again, whatever the time.
    let change = |what: &str, old: &str, new: &str| {
        failure(&format!(
            "GPU A (metal): the {what} changed, {old:?} → {new:?}: the baseline and its bound were measured on {old:?}, so this time is not checked against them (measure the runs again (spec/benchmarks.md, \"Visual library\"), then bless (see: cargo xtask bless-benchmarks))"
        ))
    };
    for (system, driver, outcome) in [
        (
            "macOS 26.6",
            "-",
            change("system", "macOS 26.5.2", "macOS 26.6"),
        ),
        (SYSTEM, "AGX 2.0", change("driver", "-", "AGX 2.0")),
        (
            "macOS 27.0",
            "AGX 2.0",
            change("system", "macOS 26.5.2", "macOS 27.0"),
        ),
        // Finding H13-1 of the fifth deep review of visual block 2 (#34): an escape read as
        // a space, so that both sides read the same; both are quoted and escaped, spaces kept.
        (SYSTEM, "-\u{1b}", change("driver", "-", "-\u{1b}")),
        (
            SYSTEM,
            "A  B\u{202e}",
            change("driver", "-", "A  B\u{202e}"),
        ),
    ] {
        let mut fake = Fake {
            gpu: Some(Ok(("GPU A (metal)".to_owned(), driver.to_owned()))),
            system: Some(Ok(system.to_owned())),
            measured: Some(Ok(times("GPU A (metal)", 100_000))),
            held: Some(Ok(held(&[("GPU A (metal)", 100_000)]))),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut fake, &["check"]),
            (String::new(), outcome),
            "{system} {driver}"
        );
        assert_eq!(fake.measures, 0, "{system} {driver}: nothing measured");
    }
    // The same system and driver, the same time: a pass.
    let mut same = Fake {
        measured: Some(Ok(times("GPU A (metal)", 100_000))),
        held: Some(Ok(held(&[("GPU A (metal)", 100_000)]))),
        ..Fake::default()
    };
    assert_eq!(command(&mut same, &["check"]).1, Ok(()));
    assert_eq!(same.measures, 1);
}

/// Why a name, a system or a driver is refused: what a baseline holding it would not read back.
const UNREADABLE: &str = "empty, with a space around it, with a control, bidirectional formatting or line separator character, or, for a name, starting with #";

/// What a missing baseline is told to do: restore it, or bless it only where it belongs.
const MISSING: &str = ": restore it (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt), or bless it, only on AC power, outside Low Power Mode, on the machine whose GPU it holds (see: cargo xtask bless-benchmarks)";

/// What a baseline not as `bless` writes it is told to do, after its reason.
const RESTORE: &str = ": not as bless writes it: restore it (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt), or rewrite it only when its times are meant to change (see: cargo xtask bless-benchmarks)";

#[test]
fn blesses_this_gpus_time_system_and_driver_beside_the_others() {
    let mut fake = Fake {
        gpu: Some(Ok(("GPU A (metal)".to_owned(), "AGX 2.0".to_owned()))),
        system: Some(Ok("macOS 26.6".to_owned())),
        measured: Some(Ok(times("GPU A (metal)", 120))),
        held: Some(Ok(held(&[("GPU A (metal)", 100), ("GPU B (vulkan)", 5)]))),
        ..Fake::default()
    };
    let (said, outcome) = command(&mut fake, &["bless"]);
    assert_eq!(outcome, Ok(()));
    assert!(said.ends_with(&format!("wrote {BASELINE}\n")), "{said}");
    let blessed = Held {
        time: 120,
        system: "macOS 26.6".to_owned(),
        driver: "AGX 2.0".to_owned(),
    };
    assert_eq!(
        fake.written,
        [format_baseline(&BTreeMap::from([
            ("GPU A (metal)".to_owned(), blessed),
            ("GPU B (vulkan)".to_owned(), entry(5))
        ]))],
        "its line replaced, with the system and driver now, the others kept"
    );
    assert!(
        fake.written.first().is_some_and(|text| text.ends_with(
            "GPU A (metal)\t120\tmacOS 26.6\tAGX 2.0\nGPU B (vulkan)\t5\tmacOS 26.5.2\t-\n"
        )),
        "{:?}",
        fake.written
    );
    let mut first = Fake {
        measured: Some(Ok(times("GPU A (metal)", 120))),
        ..Fake::default()
    };
    assert_eq!(command(&mut first, &["bless"]).1, Ok(()));
    assert_eq!(
        parse_baseline(first.written.first().expect("written")),
        Ok(BTreeMap::from([("GPU A (metal)".to_owned(), entry(120))])),
        "the first baseline"
    );
}

/// What `bless` gives and writes for a GPU `name`, with `driver`, on `system`.
fn blessed(name: &str, system: &str, driver: &str) -> (Result<(), BenchError>, Vec<String>) {
    let mut fake = Fake {
        gpu: Some(Ok((name.to_owned(), driver.to_owned()))),
        system: Some(Ok(system.to_owned())),
        measured: Some(Ok(times(name, 120))),
        ..Fake::default()
    };
    let outcome = command(&mut fake, &["bless"]).1;
    (outcome, fake.written)
}

#[test]
fn blesses_only_a_baseline_that_reads_back_as_written() {
    // A GPU's name, a system or a driver is written as given: one holding a tab or a line
    // break, starting a line with `#`, empty or with a space around it would be read back
    // as another GPU, a malformed line or a comment, or refused. The bless refuses it,
    // naming the GPU, and writes nothing.
    for text in [
        "GPU\tA",
        "GPU\nA",
        "GPU\u{1b}[31mA",
        "GPU\u{85}A",
        "GPU\u{202e}A",
        "# GPU A",
        "GPU A\r\n",
        "",
        " GPU A",
        "GPU A ",
    ] {
        for (name, system, driver) in [
            (text, SYSTEM, "-"),
            ("GPU A (metal)", text, "-"),
            ("GPU A (metal)", SYSTEM, text),
        ] {
            // A leading `#` reads back as a comment only at the start of a line: the name's.
            if text == "# GPU A" && name != text {
                continue;
            }
            let (field, what) = if name == text {
                (name, "name")
            } else if system == text {
                (system, "system")
            } else {
                (driver, "driver")
            };
            let mut fake = Fake {
                gpu: Some(Ok((name.to_owned(), driver.to_owned()))),
                system: Some(Ok(system.to_owned())),
                measured: Some(Ok(times(name, 120))),
                ..Fake::default()
            };
            assert_eq!(
                command(&mut fake, &["bless"]),
                (
                    String::new(),
                    failure(&one_line(&format!(
                        "cannot bless {name:?}: {field:?} is not a GPU's {what}: {UNREADABLE}"
                    )))
                ),
                "{name:?} {system:?} {driver:?}"
            );
            // Findings H13-1 and J13-n1 of the fifth deep review of visual block 2 (#34): the
            // field named, and nothing measured before it is refused.
            assert_eq!(
                (fake.measures, fake.written.len()),
                (0, 0),
                "{field:?}: nothing measured, nothing written"
            );
        }
    }
    assert_eq!(
        blessed("GPU A (metal)", "# macOS", "# AGX").0,
        Ok(()),
        "a `#` past the line's start is text"
    );
    // Any name, system and driver: what bless writes reads back the same, or it writes
    // nothing.
    let text = || {
        visual_testkit::proptest::string::string_regex("[ -~\t\n\r#\u{80}-\u{10FFFF}]{0,12}")
            .expect("a pattern")
    };
    let verdict = visual_testkit::runner::runner(512).run(
        &(text(), text(), text()),
        |(name, system, driver)| {
            let (outcome, written) = blessed(&name, &system, &driver);
            let read_back = written.first().map(|baseline| parse_baseline(baseline));
            let expected = Ok(BTreeMap::from([(
                name.clone(),
                Held {
                    time: 120,
                    system: system.clone(),
                    driver: driver.clone(),
                },
            )]));
            if outcome.is_ok() == (read_back.as_ref() == Some(&expected))
                && (outcome.is_ok() || read_back.is_none())
            {
                Ok(())
            } else {
                Err(visual_testkit::proptest::test_runner::TestCaseError::fail(
                    format!("{name:?} {system:?} {driver:?}: {outcome:?}, read back {read_back:?}"),
                ))
            }
        },
    );
    assert_eq!(verdict.map_err(|error| error.to_string()), Ok(()));
}

#[test]
fn fails_on_a_held_gpu_without_a_clock_and_blesses_none() {
    // A GPU the baseline holds whose clock is gone (a wgpu or driver change dropping its
    // timestamp queries) fails the check before rendering: the gate is never silently
    // vacuous on the machine it exists for.
    let mut holding = Fake {
        clockless: true,
        measured: Some(Ok(times("GPU A (metal)", 1))),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut holding, &["check"]),
        (
            String::new(),
            failure(
                "GPU A (metal): this GPU has no clock its programs can read, though the baseline holds its time: no time to check (a wgpu or driver change that lost its timestamp queries is a defect to report)"
            )
        )
    );
    assert_eq!(holding.measures, 0, "nothing rendered");
    let mut blessing = Fake {
        clockless: true,
        measured: Some(Ok(times("GPU A (metal)", 1))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut blessing, &["bless"]),
        (
            String::new(),
            failure("GPU A (metal): this GPU has no clock its programs can read: no time to bless")
        )
    );
    assert_eq!(
        (blessing.measures, blessing.written.len()),
        (0, 0),
        "nothing rendered, nothing written"
    );
    // A GPU the baseline does not hold needs no clock: nothing rendered, a pass.
    let mut other = Fake {
        clockless: true,
        gpu: Some(Ok(("GPU C (vulkan)".to_owned(), "llvmpipe".to_owned()))),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut other, &["check"]),
        (
            "GPU C (vulkan): no baseline (the baseline holds GPU A (metal)): its time is not checked, nothing rendered\n".to_owned(),
            Ok(())
        )
    );
}

#[test]
fn fails_when_the_clock_measures_too_few_renders_and_shows_every_count() {
    // A clock that wrote timestamps of too few renders at the checked size: no time to check
    // or bless, never a pass by a time of 0; a shown size unmeasured says so.
    let mut unmeasured = times("GPU A (metal)", 1);
    unmeasured.checked = Timing::Unmeasured { measured: 14 };
    unmeasured.shown[0] = Timing::Unmeasured { measured: 0 };
    for args in ["check", "bless"] {
        let again = if args == "check" {
            "cargo xtask check"
        } else {
            "cargo xtask bless-benchmarks"
        };
        let line = format!(
            "GPU A (metal): its clock measured 14 of 45 renders at 1280x720, fewer than 15: no time to {args} (run it again with no other heavy work running (see: {again}); a clock failing so is a defect to report)"
        );
        let mut fake = Fake {
            measured: Some(Ok(unmeasured.clone())),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        let (said, outcome) = command(&mut fake, &[args]);
        assert_eq!(outcome, failure(&line), "{args}");
        assert!(
            said.starts_with(
                "GPU A (metal): render 64x36: unmeasured (0 of 45 measured) (shown)\n"
            ) && said.ends_with("GPU A (metal): render 1280x720: unmeasured (14 of 45 measured)\n"),
            "{args}: {said}"
        );
        assert!(fake.written.is_empty(), "{args}: nothing written");
    }
    assert_eq!(TRIES, 45, "three times the renders measured");
}

#[test]
fn names_what_it_cannot_measure_read_or_write() {
    let mut unmeasured = Fake {
        measured: Some(Err("no GPU adapter: none".to_owned())),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut unmeasured, &["check"]).1,
        failure("cannot measure the GPU: no GPU adapter: none")
    );
    for args in ["check", "bless"] {
        let mut unopened = Fake {
            gpu: Some(Err("no GPU adapter: none".to_owned())),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut unopened, &[args]).1,
            failure("cannot open the GPU: no GPU adapter: none"),
            "{args}"
        );
        let mut unknown_system = Fake {
            system: Some(Err("sw_vers exit status: 1".to_owned())),
            measured: Some(Ok(times("GPU A (metal)", 1))),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut unknown_system, &[args]).1,
            failure("cannot read the system's version: sw_vers exit status: 1"),
            "{args}"
        );
        assert!(unknown_system.written.is_empty(), "{args}: nothing written");
    }
    // Findings H13-4 and J1-n1 of the fifth deep review of visual block 2 (#34): every read
    // failure was told to restore the file, a permission or a directory in its place
    // included. A remedy per kind of error.
    for (kind, remedy) in [
        (
            io::ErrorKind::InvalidData,
            ": not as bless writes it: restore it (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt)",
        ),
        (
            io::ErrorKind::IsADirectory,
            ": a directory stands where the file should be: remove it, then restore the file (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt)",
        ),
        (
            io::ErrorKind::PermissionDenied,
            ": make it readable, then check again (see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- check)",
        ),
        (io::ErrorKind::Other, ""),
    ] {
        let mut unreadable = Fake {
            held: Some(Err(kind)),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut unreadable, &["check"]).1,
            failure(&format!(
                "cannot read spec/gpu-time-baseline.txt: refused{remedy}"
            )),
            "{kind:?}"
        );
    }
    let mut invalid = Fake {
        held: Some(Ok("GPU A 100\n".to_owned())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut invalid, &["bless"]).1,
        failure(&format!(
            "spec/gpu-time-baseline.txt: line 1: not a GPU's name, its time, its system and its driver, separated by tabs{RESTORE}"
        ))
    );
    let mut read_only = Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        read_only: true,
        ..Fake::default()
    };
    assert_eq!(
        command(&mut read_only, &["bless"]).1,
        failure("cannot write spec/gpu-time-baseline.txt: read-only")
    );
}

#[test]
fn reads_back_the_baseline_it_writes_and_refuses_any_other_text() {
    let times = BTreeMap::from([
        ("A (metal)".to_owned(), entry(1)),
        (
            "B (vulkan)".to_owned(),
            Held {
                time: 22,
                system: "linux 6.8.0".to_owned(),
                driver: "llvmpipe Mesa 24.0.5".to_owned(),
            },
        ),
    ]);
    let text = format_baseline(&times);
    assert_eq!(
        text,
        "# The GPU's time to render the square at 1280x720, in nanoseconds, by its own clock\n\
         # (spec/benchmarks.md, \"Visual library\"): one line per GPU, its name, its time, the system and the GPU's driver\n\
         # it was blessed on, separated by tabs; written by `visual-gpu bless`, only\n\
         # on AC power, outside Low Power Mode, on the machine whose GPU it holds.\n\
         A (metal)\t1\tmacOS 26.5.2\t-\n\
         B (vulkan)\t22\tlinux 6.8.0\tllvmpipe Mesa 24.0.5\n"
    );
    assert_eq!(parse_baseline(&text), Ok(times));
    // Each line's own fault, by its number in the file.
    let fields = "not a GPU's name, its time, its system and its driver, separated by tabs";
    for (bad, why) in [
        ("A 1 macOS -\n".to_owned(), format!("line 1: {fields}")),
        ("A\t1\n".to_owned(), format!("line 1: {fields}")),
        ("A\t1\tmacOS\n".to_owned(), format!("line 1: {fields}")),
        (
            "A\t1\tmacOS\t-\t-\n".to_owned(),
            format!("line 1: {fields}"),
        ),
        (
            "# c\nA\t1.5\tmacOS\t-\n".to_owned(),
            "line 2: \"1.5\" is not a whole number of nanoseconds above 0 a measured time rounds to".to_owned(),
        ),
        (
            "A\t-1\tmacOS\t-\n".to_owned(),
            "line 1: \"-1\" is not a whole number of nanoseconds above 0 a measured time rounds to".to_owned(),
        ),
        (
            "A\t1\tmacOS\t-\nA\t2\tmacOS\t-\n".to_owned(),
            "line 2: A written twice".to_owned(),
        ),
    ] {
        assert_eq!(parse_baseline(&bad), Err(why), "{bad:?}");
    }
}

#[test]
fn refuses_any_text_bless_would_not_write() {
    // Read only exactly as `bless` writes it: a file holding no GPU's time, a name, system
    // or driver `bless` refuses, or any text that reads as times but is not the one `bless`
    // writes of them, named by the first line that differs.
    let fields = "not a GPU's name, its time, its system and its driver, separated by tabs";
    let header = held(&[]);
    let one = held(&[("A (metal)", 94_583)]);
    let differs = |at: usize, found: &str, written: &str| {
        Err(format!("line {at}: {found:?}, not {written:?}"))
    };
    let first = format!("{}\n", header.lines().next().expect("a header"));
    let line = "A (metal)\t94583\tmacOS 26.5.2\t-\n";
    let spaced = |field: &str, what: &str| {
        Err(format!(
            "line 5: {field:?} is not a GPU's {what}: {UNREADABLE}"
        ))
    };
    for (bad, outcome) in [
        (String::new(), Err("no GPU's time".to_owned())),
        (header.clone(), Err("no GPU's time".to_owned())),
        (format!("{header}\t5\tmacOS\t-\n"), spaced("", "name")),
        (
            format!("{header}A (metal) \t5\tmacOS\t-\n"),
            spaced("A (metal) ", "name"),
        ),
        (
            format!("{header} A (metal)\t5\tmacOS\t-\n"),
            spaced(" A (metal)", "name"),
        ),
        (format!("{header}A\t5\t\t-\n"), spaced("", "system")),
        (
            format!("{header}A\t5\tmacOS \t-\n"),
            spaced("macOS ", "system"),
        ),
        (
            format!("{header}A\t5\t macOS\t-\n"),
            spaced(" macOS", "system"),
        ),
        (format!("{header}A\t5\tmacOS\t\n"), spaced("", "driver")),
        (format!("{header}A\t5\tmacOS\t- \n"), spaced("- ", "driver")),
        (format!("{header}A\t5\tmacOS\t -\n"), spaced(" -", "driver")),
        (line.to_owned(), differs(1, line, &first)),
        (
            format!("{header}# a comment\n{line}"),
            differs(5, "# a comment\n", line),
        ),
        (
            format!("{one}# a comment\n"),
            differs(6, "# a comment\n", ""),
        ),
        (
            format!("{header}B (vulkan)\t1\tmacOS\t-\nA (metal)\t2\tmacOS\t-\n"),
            differs(5, "B (vulkan)\t1\tmacOS\t-\n", "A (metal)\t2\tmacOS\t-\n"),
        ),
        (
            one.replace("94583", "+94583"),
            differs(5, &line.replace("94583", "+94583"), line),
        ),
        (
            one.replace("94583", "094583"),
            differs(5, &line.replace("94583", "094583"), line),
        ),
        (
            one.replace('\n', "\r\n"),
            differs(1, &first.replace('\n', "\r\n"), &first),
        ),
        (one.trim_end().to_owned(), differs(5, line.trim_end(), line)),
        (format!("{one}\n"), Err(format!("line 6: {fields}"))),
    ] {
        assert_eq!(parse_baseline(&bad), outcome, "{bad:?}");
    }
}

/// The baseline the repository holds, as `cargo xtask bless-benchmarks` wrote it.
const COMMITTED: &str = include_str!("../../../spec/gpu-time-baseline.txt");

#[test]
fn holds_a_baseline_as_bless_writes_it() {
    assert_eq!(
        parse_baseline(COMMITTED),
        Ok(BTreeMap::from([(
            "Apple M2 Pro (metal)".to_owned(),
            entry(93_292)
        )])),
        "the maintainer's Mac, the one GPU it holds, on the system its bound was measured on, \
         Metal naming no driver (its driver is the system's)"
    );
}

#[test]
fn rounds_a_time_to_whole_nanoseconds_above_zero() {
    // A time that rounds to 0 ns, or is no whole number of nanoseconds (not a number,
    // infinite, negative, beyond `u64::MAX`), is no time: never 0 standing for none, never
    // `u64::MAX` standing for one that every check would pass against.
    for (nanoseconds, whole) in [
        // Findings H16-2 of the fifth deep review of visual block 2 (#34): the ties, of both
        // parities, and the edge of what a `u64` holds.
        (0.5_f64, None),
        (1.5_f64, Some(2)),
        (2.5_f64, Some(2)),
        (3.5_f64, Some(4)),
        (
            f64::from_bits(0x43EF_FFFF_FFFF_FFFF),
            Some(18_446_744_073_709_549_568),
        ),
        (f64::from_bits(0x43F0_0000_0000_0000), None),
        (0.6_f64, Some(1)),
        (1.4_f64, Some(1)),
        (1.6_f64, Some(2)),
        (92_500.0_f64, Some(92_500)),
        (1.8e19_f64, Some(18_000_000_000_000_000_000)),
        (0.0_f64, None),
        (-0.0_f64, None),
        (0.4_f64, None),
        (f64::MIN_POSITIVE / 2.0_f64, None),
        (-1.0_f64, None),
        (f64::NAN, None),
        (f64::INFINITY, None),
        (f64::NEG_INFINITY, None),
        (1e30_f64, None),
    ] {
        assert_eq!(whole_nanoseconds(nanoseconds), whole, "{nanoseconds}");
    }
    // Renders of no time are left out of the median as unmeasured, as an unwritten
    // timestamp is.
    let mut count = 0_u32;
    assert_eq!(
        median_of(&mut || {
            count = count.saturating_add(1);
            Ok(Some(if count.is_multiple_of(2) {
                0.0_f64
            } else {
                5.0_f64
            }))
        })
        .map_err(|error| error.to_string()),
        Ok(timing(5, 30)),
        "the renders of 0 ns left out: fifteen of 5 ns in 30 tries"
    );
}

#[test]
fn keeps_exactly_the_clock_periods_that_can_give_a_time() {
    // The edges of the periods kept, derived from what makes a time of them: at the shortest,
    // the most ticks a sum holds round to 0 ns, and the next period above gives 1; at the
    // longest, one tick is past what a `u64` holds, and the next below gives a time.
    let time = |ticks: u64, period: f32| whole_nanoseconds(crate::nanoseconds(ticks, period));
    let (shortest, longest) = (f32::from_bits(0x1F00_0000), f32::from_bits(0x5F80_0000));
    assert_eq!(
        [
            time(u64::MAX, shortest),
            time(u64::MAX, shortest.next_up()),
            time(1, longest.next_down()),
            time(1, longest),
        ],
        [None, Some(1), Some(18_446_742_974_197_923_840), None]
    );
    for (period, kept) in [
        (shortest, false),
        (shortest.next_up(), true),
        (longest.next_down(), true),
        (longest, false),
    ] {
        assert_eq!(crate::clock_period(period).is_some(), kept, "{period}");
    }
}

#[test]
fn says_a_name_with_control_characters_on_its_own_lines_only() {
    // A GPU's name is a driver's text: its control characters (an escape, a line break)
    // are neither written to the terminal nor read from the baseline.
    let named = "GPU\u{1b}[2J\nA (metal)";
    let mut blessing = Fake {
        gpu: Some(Ok(("GPU A (metal)".to_owned(), "-".to_owned()))),
        measured: Some(Ok(times(named, 120))),
        ..Fake::default()
    };
    let (said, outcome) = command(&mut blessing, &["bless"]);
    assert_eq!(
        said,
        "GPU\\u{1b}[2J A (metal): render 64x36: 7000 ns (15 of 15 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 320x180: 12000 ns (15 of 16 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 640x360: 30000 ns (15 of 17 measured) (shown)\nGPU\\u{1b}[2J A (metal): display 1280x720: 40000 ns (15 of 18 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 1280x720: 120 ns (15 of 15 measured)\nwrote spec/gpu-time-baseline.txt\n"
    );
    assert_eq!((outcome, blessing.written.len()), (Ok(()), 1));
    let mut other = Fake {
        gpu: Some(Ok(("GPU\u{7}C\r(vulkan)".to_owned(), "-".to_owned()))),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut other, &["check"]),
        (
            "GPU\\u{7}C (vulkan): no baseline (the baseline holds GPU A (metal)): its time is not checked, nothing rendered\n".to_owned(),
            Ok(())
        )
    );
    // Finding H4-2 of the fifth deep review of visual block 2 (#34): bidirectional
    // formatting characters passed, and nothing bounded a line.
    let long = "x".repeat(1_100);
    for (given, start) in [
        (
            "GPU\u{202e}C\u{2028}(vulkan)",
            "GPU\\u{202e}C (vulkan): no baseline (the",
        ),
        (long.as_str(), "xxxx"),
    ] {
        let mut odd = Fake {
            gpu: Some(Ok((given.to_owned(), "-".to_owned()))),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        let (line, verdict) = command(&mut odd, &["check"]);
        assert_eq!(verdict, Ok(()), "{given:?}");
        assert!(
            line.starts_with(start) && line.chars().count() <= 1_001,
            "{line}"
        );
    }
    assert_eq!(
        command(&mut Fake::default(), &[&long]).1,
        Err(BenchError {
            code: 2,
            line: format!("unknown command \"{}…", "x".repeat(982))
        }),
        "a usage error bounded: 17 characters, 982 and the mark"
    );
    for (line, field, what) in [
        ("GPU\u{1b}A\t5\tmacOS\t-\n", "GPU\u{1b}A", "name"),
        ("A\t5\tmacOS\u{9b}\t-\n", "macOS\u{9b}", "system"),
        ("A\t5\tmacOS\t\u{7f}\n", "\u{7f}", "driver"),
        ("G\u{202e}A\t5\tmacOS\t-\n", "G\u{202e}A", "name"),
        ("A\t5\tmac\u{2028}OS\t-\n", "mac\u{2028}OS", "system"),
        ("A\t5\tmacOS\t-\u{2066}\n", "-\u{2066}", "driver"),
    ] {
        assert_eq!(
            parse_baseline(line),
            Err(format!(
                "line 1: {field:?} is not a GPU's {what}: {UNREADABLE}"
            )),
            "{line:?}"
        );
    }
}

#[test]
fn says_once_where_a_baseline_differs_from_what_bless_writes() {
    // A baseline whose line endings `bless` does not write: the line where it differs, once,
    // then the command restoring it.
    let crlf = held(&[("GPU A (metal)", 1)]).replace('\n', "\r\n");
    let first = crlf.split_inclusive('\n').next().expect("a header line");
    let mut fake = Fake {
        held: Some(Ok(crlf.clone())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut fake, &["check"]),
        (
            String::new(),
            failure(&format!(
                "{BASELINE}: line 1: {first:?}, not {:?}{RESTORE}",
                first.replace("\r\n", "\n")
            ))
        )
    );
}

#[test]
fn refuses_a_baseline_time_of_zero() {
    // A time of 0 is no time: every check against it but one of 0 would fail, and a clock
    // measuring 0 would pass.
    assert_eq!(
        parse_baseline("A\t0\tmacOS\t-\n"),
        Err(
            "line 1: \"0\" is not a whole number of nanoseconds above 0 a measured time rounds to"
                .to_owned()
        )
    );
}

#[test]
fn reads_only_a_time_bless_could_write() {
    // Finding J16-n2 of the fifth deep review of visual block 2 (#34): a time is a 64-bit
    // float rounded to whole nanoseconds, so a whole number no such float is, above 2⁵³, is
    // one bless never wrote.
    for (time, read) in [
        (1, true),
        (1 << 53_u32, true),
        ((1 << 53_u32) + 1, false),
        ((1 << 53_u32) + 2, true),
        (18_446_744_073_709_549_568, true),
        (18_446_744_073_709_549_569, false),
        (u64::MAX, false),
    ] {
        let expected = if read {
            Ok(BTreeMap::from([("A".to_owned(), entry(time))]))
        } else {
            Err(format!(
                "line 5: \"{time}\" is not a whole number of nanoseconds above 0 a measured time rounds to"
            ))
        };
        assert_eq!(parse_baseline(&held(&[("A", time)])), expected, "{time}");
    }
}

#[test]
fn compares_exactly_at_any_baseline() {
    // The bound's products in 128 bits: a baseline near `u64::MAX` neither saturates into
    // passing every time nor fails its own time.
    assert!(within(u64::MAX, u64::MAX, 10));
    assert!(!within(1, u64::MAX, 10), "far faster than its baseline");
    assert!(!within(u64::MAX, u64::MAX / 2, 10), "far slower");
    assert!(within(u64::MAX / 10 * 9, u64::MAX / 10 * 10, 10));
    assert!(!within(u64::MAX / 10 * 9 - 1, u64::MAX / 10 * 10, 10));
    // A bound of 0%: only a time equal to its baseline is within it.
    assert!(within(1000, 1000, 0) && !within(1001, 1000, 0) && !within(999, 1000, 0));
}

#[test]
fn measures_this_gpu_by_its_clock() {
    let gpu = GpuSquare::new(wgpu::Backends::all()).expect("a GPU (a software one in CI)");
    // The checks run on a GPU with a clock (`runs_the_checks_on_a_gpu_with_a_clock`): a time.
    let times = measure(&gpu).expect("measured: the GPU has a clock");
    assert_eq!(times.adapter, gpu.adapter());
    let measured_above = |timing: Timing, least: u64| matches!(timing, Timing::Measured { nanoseconds, tries } if nanoseconds >= least && (15..=TRIES).contains(&tries));
    // A size only shown may come back unmeasured, never a time of 0, idle as under load
    // (`SHOWN`). The checked size and the encoding must be measured.
    assert!(measured_above(times.checked, 1), "{times:?}");
    assert!(
        times.shown.iter().all(|&time| measured_above(time, 1)
            || matches!(time, Timing::Unmeasured { measured } if measured < REPEATS)),
        "{times:?}"
    );
    // Encoding 1280x720 for display moves 20 bytes a pixel through the GPU's memory (16
    // read, 4 written): 18,432,000 bytes, at least 184 ns even at 100 TB/s, more than ten
    // times the fastest GPU memory made (about 8 TB/s, 2026).
    assert!(measured_above(times.display, 100), "{times:?}");
    // A GPU without a clock is answered at once: a failure, nothing rendered (rendering
    // would measure nothing, every size unmeasured).
    let untimed = GpuSquare::open(wgpu::Backends::all(), |limits| limits, false)
        .expect("a GPU (a software one in CI)");
    assert_eq!(
        measure(&untimed).map_err(|error| error.to_string()),
        Err("this GPU has no clock its programs can read".to_owned())
    );
}

/// A directory of one test, removed with what it holds when the guard is dropped.
#[derive(Debug)]
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing better can be done in a drop.
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// A fresh, empty directory `visual-gpu-<name>-<process id>` ([`Scratch`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the one place visual-gpu's tests name the temporary directory: the guard \
              removes what it creates"
)]
fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("visual-gpu-{name}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&dir));
    std::fs::create_dir_all(dir.join("spec")).expect("scratch directory");
    Scratch(dir)
}

#[test]
fn removes_its_scratch_directory_whatever_the_test_does() {
    let kept = scratch("guard");
    let dropped = kept.0.clone();
    assert!(dropped.join("spec").is_dir(), "its spec directory");
    drop(kept);
    assert!(!dropped.exists(), "removed with what it holds when dropped");
}

#[test]
fn reads_and_writes_the_baseline_below_the_root() {
    let root = scratch("files");
    let mut system = System::new(root.0.clone());
    assert_eq!(
        system.read_baseline().map_err(|error| error.kind()),
        Ok(None)
    );
    system.write_baseline("A\t1\n").expect("written");
    assert_eq!(
        system.read_baseline().map_err(|error| error.kind()),
        Ok(Some("A\t1\n".to_owned()))
    );
    std::fs::remove_file(root.0.join(BASELINE)).expect("removed");
    std::fs::create_dir(root.0.join(BASELINE)).expect("a directory in its place");
    assert_eq!(
        system.read_baseline().map_err(|error| error.kind()),
        Err(io::ErrorKind::IsADirectory),
        "a baseline that cannot be read (EISDIR, on Linux and macOS)"
    );
    assert_eq!(
        system
            .measure()
            .map(|times| times.adapter.is_empty())
            .map_err(|error| error.to_string()),
        Ok(false),
        "the first GPU measured: a time, the checks running on a GPU with a clock"
    );
    assert!(system.gpu.is_some(), "the GPU measured kept open");
}

#[test]
fn opens_the_first_gpu_once_and_measures_the_one_opened() {
    let first = GpuSquare::new(wgpu::Backends::all()).expect("a GPU (a software one in CI)");
    let mut system = System::new(std::path::PathBuf::from("."));
    let opened = system.open().map_err(|error| error.to_string());
    assert_eq!(
        opened,
        Ok(Gpu {
            adapter: first.adapter().to_owned(),
            driver: first.driver().to_owned(),
            clock: true,
        })
    );
    assert!(system.gpu.is_some(), "kept open");
    let measured = system.measure().map_err(|error| error.to_string());
    assert_eq!(
        measured.map(|times| times.adapter),
        Ok(first.adapter().to_owned()),
        "the GPU opened, measured"
    );
}

#[test]
fn reads_the_systems_version_or_says_why_not() {
    let mut system = System::new(std::path::PathBuf::from("."));
    let version = system.system().map_err(|error| error.to_string());
    // `macOS 26.5.2` on the maintainer's Mac; `linux 6.8.0-…` on CI's runners.
    assert!(
        version.as_ref().is_ok_and(|text| text
            .strip_prefix(VERSION.system)
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(
                |number| number.starts_with(|digit: char| digit.is_ascii_digit())
                    && number.trim() == number
            )),
        "{version:?}"
    );
    // Each way the program can fail, through the command: one line naming the program,
    // with what it said on its error output.
    let root = scratch("version");
    let first = GpuSquare::new(wgpu::Backends::all()).expect("a GPU (a software one in CI)");
    std::fs::write(
        root.0.join(BASELINE),
        format_baseline(&BTreeMap::from([(
            first.adapter().to_owned(),
            Held::new(1, SYSTEM.to_owned(), first.driver().to_owned()),
        )])),
    )
    .expect("a baseline holding this GPU");
    drop(first);
    for (program, args, line) in [
        (
            "false",
            &[][..],
            "cannot read the system's version: false exit status: 1",
        ),
        (
            "sh",
            &["-c", "printf 'no\\nversion\\n' >&2; exit 3"][..],
            "cannot read the system's version: sh exit status: 3: no version",
        ),
        (
            "visual-gpu-no-such-program",
            &[][..],
            "cannot read the system's version: visual-gpu-no-such-program: No such file or directory (os error 2)",
        ),
        (
            "true",
            &[][..],
            "cannot read the system's version: true said no version",
        ),
        (
            "printf",
            &["  \\n"][..],
            "cannot read the system's version: printf said no version",
        ),
        (
            "printf",
            &["26.5.2\\033"][..],
            "cannot read the system's version: printf said \"26.5.2\\u{1b}\", which holds a control, bidirectional formatting or line separator character",
        ),
        (
            "printf",
            &["26\u{202e}5"][..],
            "cannot read the system's version: printf said \"26\\u{202e}5\", which holds a control, bidirectional formatting or line separator character",
        ),
    ] {
        let mut failing = System::new(root.0.clone());
        failing.version = Version {
            system: "none",
            program,
            args,
        };
        let mut out = Vec::new();
        assert_eq!(
            run(&["check".to_owned()], &mut failing, &mut out),
            failure(line),
            "{program}"
        );
    }
}

#[test]
fn takes_the_median_of_fifteen_measured_renders_after_the_warm_up() {
    // Renders take 1, 2, 3, … nanoseconds, in reverse order of the warm-up's count: the
    // fifteen measured take 15 down to 1, whose median is 8; any warm-up render counted
    // would take 16 or more.
    let mut count = 0_u32;
    let mut render = || {
        count = count.saturating_add(1);
        Ok(Some(f64::from(31_u32.saturating_sub(count))))
    };
    assert_eq!(
        median_of(&mut render).map_err(|error| error.to_string()),
        Ok(timing(8, 15))
    );
    assert_eq!(count, 30, "fifteen to warm up, then fifteen measured");
    // Every third render unmeasured (a pass whose timestamps the GPU did not write), each
    // taking its count: after the warm-up, renders 16 to 37 measure fifteen (16, 17, 19, 20,
    // 22, …, 35, 37) in 22 tries, whose median, the eighth, is 26.
    let mut every = 0_u32;
    let mut sometimes = || {
        every = every.saturating_add(1);
        Ok((!every.is_multiple_of(3)).then_some(f64::from(every)))
    };
    assert_eq!(
        median_of(&mut sometimes).map_err(|error| error.to_string()),
        Ok(timing(26, 22)),
        "fifteen measured, however many renders it takes"
    );
    // A clock that measures nothing, or too little, stops at the bound: unmeasured, how many
    // were.
    let mut never = 0_u32;
    assert_eq!(
        median_of(&mut || {
            never = never.saturating_add(1);
            Ok(None)
        })
        .map_err(|error| error.to_string()),
        Ok(Timing::Unmeasured { measured: 0 }),
        "nothing measured"
    );
    assert_eq!(never, 15 + 45, "the warm-up, then the bound");
    let mut rare = 0_u32;
    assert_eq!(
        median_of(&mut || {
            rare = rare.saturating_add(1);
            Ok(rare.is_multiple_of(4).then_some(1.0_f64))
        })
        .map_err(|error| error.to_string()),
        Ok(Timing::Unmeasured { measured: 12 }),
        "one in four of the 45 tries after the warm-up: renders 16 to 60 measure 12"
    );
    let mut fifteenth = 0_u32;
    assert_eq!(
        median_of(&mut || {
            fifteenth = fifteenth.saturating_add(1);
            Ok((fifteenth > 45).then_some(2.0_f64))
        })
        .map_err(|error| error.to_string()),
        Ok(timing(2, 45)),
        "the fifteenth measured at the last try"
    );
    assert_eq!(
        median_of(&mut || Err("lost".into())).map_err(|error| error.to_string()),
        Err("lost".to_owned()),
        "the first failure"
    );
}

#[test]
fn says_every_failure_on_one_line() {
    let mut fake = Fake {
        measured: Some(Err(
            "Validation Error\n\nCaused by:\n  In Queue::submit\u{1b}[0m".to_owned(),
        )),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut fake, &["check"]).1,
        failure("cannot measure the GPU: Validation Error Caused by: In Queue::submit\\u{1b}[0m"),
        "a multi-line error of wgpu, its control characters escaped, on one line"
    );
}

/// The cost benchmarks of the visual library's module, whose square, image and sizes the GPU
/// benchmark measures too.
const SUITE: &str = include_str!("../../../bindings/visual-wasm/benchmarks/suite.ts");

/// The one line of `text` that starts with `start`, if exactly one does.
fn only_line<'text>(text: &'text str, start: &str) -> Option<&'text str> {
    let mut lines = text.lines().filter(|line| line.starts_with(start));
    let line = lines.next()?;
    lines.next().is_none().then_some(line)
}

/// The sizes of `SIZES` in `text`, one `[width, height],` line each between its opening and
/// its closing line; none when it is written otherwise.
fn suite_sizes(text: &str) -> Option<Vec<(u32, u32)>> {
    let mut lines = text.lines().skip_while(|line| {
        *line != "export const SIZES: readonly (readonly [number, number])[] = ["
    });
    let _opening = lines.next()?;
    let mut sizes = Vec::new();
    for line in lines {
        if line == "];" {
            return Some(sizes);
        }
        let (width, height) = line
            .strip_prefix("  [")?
            .strip_suffix("],")?
            .split_once(", ")?;
        sizes.push((width.parse().ok()?, height.parse().ok()?));
    }
    None
}

#[test]
fn measures_the_square_image_and_sizes_of_the_cost_benchmarks() {
    let (rate, cycle) = (SQUARE_RATE, SQUARE_CYCLE);
    // The side as the suite writes it, its thousands grouped by `_`.
    let side = format!("{}_{:03}", SQUARE_SIDE / 1000, SQUARE_SIDE % 1000);
    assert_eq!(
        only_line(SUITE, "export const SQUARE = "),
        Some(
            format!(
                "export const SQUARE = {{ rate: [{}, {}], side: {side}, cycle: [{}, {}] }} as const;",
                rate.0, rate.1, cycle.0, cycle.1
            )
            .as_str()
        ),
        "the square"
    );
    assert_eq!(
        only_line(SUITE, "export const IMAGE = "),
        Some(format!("export const IMAGE = {};", IMAGE.0).as_str()),
        "the image"
    );
    let mut sizes = SHOWN.to_vec();
    sizes.push(CHECKED);
    assert_eq!(suite_sizes(SUITE), Some(sizes), "the sizes, smallest first");
    // The square measured is built of those values, and of nothing else: written out here
    // as the suite writes them, so that a body building another square fails.
    let suites = Square::new(
        ImageRate::new(60, 1).expect("a rate"),
        39_322,
        Seconds::new(1001, 1000).expect("a cycle"),
    )
    .expect("a square");
    assert_eq!(
        measured_square().map_err(|error| error.to_string()),
        Ok(suites),
        "the square measured"
    );
}

#[test]
fn reads_the_cost_benchmarks_only_as_they_are_written() {
    assert_eq!(only_line("a\nb\na\n", "a"), None, "a line written twice");
    assert_eq!(only_line("b\n", "a"), None, "no line");
    let sizes = |body: &str| {
        suite_sizes(&format!(
            "export const SIZES: readonly (readonly [number, number])[] = [\n{body}"
        ))
    };
    assert_eq!(
        sizes("  [1, 2],\n  [3, 4],\n];\n"),
        Some(vec![(1, 2), (3, 4)])
    );
    for other in [
        "  [1, 2],\n",
        "  [1, 2]\n];\n",
        "  [1,2],\n];\n",
        " [1, 2],\n];\n",
        "  [1, 2.5],\n];\n",
        "  [x, 2],\n];\n",
        "  [1, 2], // a size\n];\n",
    ] {
        assert_eq!(sizes(other), None, "{other:?}");
    }
    assert_eq!(suite_sizes("no sizes\n"), None, "no opening line");
}
