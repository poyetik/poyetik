//! Tests of the GPU's time benchmark: the command on a fake outside world, the baseline's
//! text, and the measurement and files of the real one.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use std::io;

use visual_boxes::Square;
use visual_core::{ImageRate, Seconds};

use super::{
    BASELINE, BLESS_CONDITION, BLESS_RUNS, BOUND_PERCENT, BenchError, BenchIo, CHECKED, Failure,
    Gpu, Held, IDLE, IMAGE, POWER_CONDITION, REPEATS, RUNS, Runs, SAFETY, SHOWN, SQUARE_CYCLE,
    SQUARE_RATE, SQUARE_SIDE, System, TRIES, Times, Timing, VERSION, Version, bound_percent,
    format_baseline, measure, measured_square, median_of, median_of_runs, one_line, parse_baseline,
    root_from, run, usage, whole_nanoseconds, within,
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
    /// The checked time of each measure in turn, over `measured`'s; the last one's past the
    /// end, none for `measured`'s alone.
    runs: Vec<Timing>,
    /// The baseline's text, `None` when there is no file; `Err` when it cannot be read.
    held: Option<Result<String, io::ErrorKind>>,
    /// The words of a baseline that cannot be read; `refused` for none.
    refusal: String,
    /// Whether the baseline cannot be written, which the world says when asked.
    read_only: bool,
    /// Whether the write itself fails, the world having said the baseline can be written.
    write_fails: bool,
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
            Ok(mut times) => {
                let run = self.measures.saturating_sub(1);
                if let Some(&checked) = self.runs.get(run).or_else(|| self.runs.last()) {
                    times.checked = checked;
                }
                Ok(times)
            }
            Err(words) => Err(words.into()),
        }
    }

    fn read_baseline(&mut self) -> io::Result<Option<String>> {
        match self.held.clone() {
            None => Ok(None),
            Some(Ok(text)) => Ok(Some(text)),
            Some(Err(kind)) if self.refusal.is_empty() => Err(io::Error::new(kind, "refused")),
            Some(Err(kind)) => Err(io::Error::new(kind, self.refusal.clone())),
        }
    }

    fn writable(&mut self) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::other("read-only"));
        }
        Ok(())
    }

    fn write_baseline(&mut self, text: &str) -> io::Result<()> {
        if self.write_fails {
            return Err(io::Error::other("disk full"));
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

/// Where a usage error points.
const HELP: &str = "(see: cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- help)";

/// The check of the GPU's time alone, as a hint names it.
const CHECK: &str = "cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- check";

#[test]
fn refuses_no_command_another_and_an_argument_and_prints_its_usage() {
    for (args, reason) in [
        (&[][..], "no command given"),
        (&["run"][..], "unknown command \"run\""),
        (&["check", "now"][..], "\"check\" takes no argument"),
        // A command holding a line break or a control character stays on one line, quoted.
        (
            &["che\nck\u{1b}", "now"][..],
            "\"che\\nck\\u{1b}\" takes no argument",
        ),
    ] {
        let mut fake = Fake::default();
        assert_eq!(
            command(&mut fake, args),
            (
                String::new(),
                Err(BenchError {
                    code: 2,
                    line: format!("{reason} {HELP}")
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
  check  measure this GPU's time and compare it with spec/gpu-time-baseline.txt, within ±11% (3/2 times the largest deviation of one of 51 idle runs from the median of any 5 of them, measured 2026-11-20; spec/benchmarks.md, \"Visual library\"), if it holds
         this GPU, on the system and driver it records (run on AC power, outside Low Power Mode)
  bless  measure this GPU's time 5 times and write the median of those runs to spec/gpu-time-baseline.txt, with the system and driver: only
         on AC power, outside Low Power Mode, on the machine whose GPU it holds, with its runs measured again on a change (spec/benchmarks.md, \"Visual library\")
  help   print this help
It reads the baseline at the root of the repository it runs in; a hint naming cargo xtask runs
from any directory of it but crates/visual-gpu and below, a workspace of its own.",
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

/// Every run recorded ([`RUNS`]).
const RECORDED: &str = include_str!("../../../spec/gpu-time-runs.txt");

/// The columns of [`RECORDED`], as its one header line writes them.
const COLUMNS: [&str; 13] = [
    "session",
    "utc",
    "load-1min",
    "power",
    "low-power",
    "condition",
    "render-1280x720-ns",
    "display-1280x720-ns",
    "render-64x36",
    "render-320x180",
    "render-640x360",
    "render-1280x720",
    "display-1280x720",
];

/// The conditions a run is recorded in.
const CONDITIONS: [&str; 7] = [
    "idle",
    "not-idle",
    "loaded",
    "between-steps",
    "bless",
    "step",
    "after-bless",
];

/// A run recorded: its session, condition, medians in nanoseconds (the render's, the display
/// encoding's), and the renders measured, then tried, of each size (none tried: not logged).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recorded {
    session: String,
    utc: String,
    condition: String,
    render: u64,
    display: u64,
    counts: Vec<(u64, u64)>,
}

/// Whether `text` is a UTC time `2026-11-20T16:03:54Z`, or `-` where it was not logged.
fn utc(text: &str) -> bool {
    let shape: String = text
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                '0'
            } else {
                character
            }
        })
        .collect();
    shape == "0000-00-00T00:00:00Z" || text == "-"
}

/// The renders measured, then tried, of `text` (`15/22`): none tried where they were not
/// logged (`-`).
fn counts(text: &str) -> Option<(u64, u64)> {
    if text == "-" {
        return Some((0, 0));
    }
    let (measured, tried) = text.split_once('/')?;
    let (measured, tried) = (measured.parse().ok()?, tried.parse().ok()?);
    (measured <= tried && tried > 0_u64).then_some((measured, tried))
}

/// The run `line` records, if it is written as [`COLUMNS`] say: a session, a time, a load, AC
/// power, Low Power Mode off, a condition, two medians above 0 and five counts.
fn recorded_run(line: &str) -> Option<Recorded> {
    let fields: [&str; 13] = line.split('\t').collect::<Vec<_>>().try_into().ok()?;
    let [
        session,
        time,
        load,
        power,
        low_power,
        condition,
        render,
        display,
        rest @ ..,
    ] = fields;
    let written = session.parse::<u64>().is_ok_and(|session| session > 0)
        && utc(time)
        && (load == "-" || load.parse::<f64>().is_ok_and(|load| load >= 0.0_f64))
        && (power, low_power) == ("AC", "off")
        && CONDITIONS.contains(&condition);
    let above_zero = |text: &str| text.parse().ok().filter(|&nanoseconds| nanoseconds > 0_u64);
    written.then_some(())?;
    Some(Recorded {
        session: session.to_owned(),
        utc: time.to_owned(),
        condition: condition.to_owned(),
        render: above_zero(render)?,
        display: above_zero(display)?,
        counts: rest.into_iter().map(counts).collect::<Option<_>>()?,
    })
}

/// The runs `text` records, one a line after its header: none when a line is written
/// otherwise.
fn recorded(text: &str) -> Option<Vec<Recorded>> {
    let mut lines = text.lines().skip_while(|line| line.starts_with('#'));
    (lines.next()?.split('\t').collect::<Vec<_>>() == COLUMNS).then_some(())?;
    lines.map(recorded_run).collect()
}

/// The runs recorded in `condition`.
fn runs_in(condition: &str) -> Vec<Recorded> {
    recorded(RECORDED)
        .expect("the runs as their header says")
        .into_iter()
        .filter(|run| run.condition == condition)
        .collect()
}

/// The render medians of the runs recorded in `condition`, sorted.
fn renders(condition: &str) -> Vec<u64> {
    let mut times: Vec<u64> = runs_in(condition).iter().map(|run| run.render).collect();
    times.sort_unstable();
    times
}

/// The lowest and the highest median of [`BLESS_RUNS`] of `times`, sorted: those of the
/// lowest and of the highest [`BLESS_RUNS`] of them, by the median a bless takes.
fn blessed_span(times: &[u64]) -> (u64, u64) {
    let lowest = times.first_chunk().expect("as many runs as a bless");
    let highest = times.last_chunk().expect("as many runs as a bless");
    (median_of_runs(*lowest), median_of_runs(*highest))
}

/// Every median of [`BLESS_RUNS`] of `times`, sorted: each run but the two lowest and the two
/// highest, two of five below it and two above.
fn blessed_times(times: &[u64]) -> Vec<u64> {
    let kept = BLESS_RUNS / 2;
    times
        .get(kept..times.len().saturating_sub(kept))
        .expect("as many runs as a bless")
        .to_vec()
}

/// `deviation` over `base`, in hundredths of a percent rounded to the nearest (half up), as
/// the prose writes it: `⌊(20 000 × deviation + base) / (2 × base)⌋`.
fn hundredths(deviation: u64, base: u64) -> u64 {
    deviation
        .saturating_mul(20_000)
        .saturating_add(base)
        .div_euclid(base.saturating_mul(2))
}

/// Hundredths as the prose writes them: `6.79`.
fn percent(hundredths: u64) -> String {
    format!(
        "{}.{:02}",
        hundredths.div_euclid(100),
        hundredths.rem_euclid(100)
    )
}

/// A time as the prose writes it, its thousands grouped by a space: `88 125`.
fn grouped(time: u64) -> String {
    format!("{} {:03}", time.div_euclid(1000), time.rem_euclid(1000))
}

/// The share of the renders tried that were not measured, of size `at` of the counts, over
/// `runs`: whole percent, to the nearest.
fn unmeasured_percent(runs: &[Recorded], at: usize) -> u64 {
    let sum = |of: fn(&(u64, u64)) -> u64| -> u64 {
        runs.iter()
            .filter_map(|run| run.counts.get(at))
            .map(of)
            .sum()
    };
    let (measured, tried) = (sum(|count| count.0), sum(|count| count.1));
    hundredths(tried.saturating_sub(measured), tried)
        .saturating_add(50)
        .div_euclid(100)
}

/// The runs of `runs` whose size `at` is unmeasured: fewer than [`REPEATS`] renders.
fn unmeasured_runs(runs: &[Recorded], at: usize) -> usize {
    runs.iter()
        .filter_map(|run| run.counts.get(at))
        .filter(|(measured, _)| *measured < u64::try_from(REPEATS).expect("64 bits"))
        .count()
}

/// The figures of the runs, each as every text stating it writes it, from [`RECORDED`] alone:
/// the idle runs and the bound, which the module's own documentation states too; then the
/// margin, the loaded runs, their spread and the one beyond the bound, the display encoding's
/// and the sizes only shown, which the spec states.
fn figures() -> Vec<String> {
    let (idle, loaded) = (renders("idle"), renders("loaded"));
    let (idle_runs, loaded_runs) = (runs_in("idle"), runs_in("loaded"));
    let blessed = blessed_span(&idle);
    let (lowest, highest) = (*idle.first().expect("a run"), *idle.last().expect("a run"));
    let (below, above) = (blessed.1.abs_diff(lowest), highest.abs_diff(blessed.0));
    let (deviation, base) = if hundredths(below, blessed.1) > hundredths(above, blessed.0) {
        (below, blessed.1)
    } else {
        (above, blessed.0)
    };
    let median = *idle.get(idle.len() / 2).expect("a middle run");

    let kept: Vec<u64> = loaded
        .iter()
        .copied()
        .filter(|&time| within(time, median, BOUND_PERCENT))
        .collect();
    let beyond = *loaded.last().expect("a run");
    let mut displays: Vec<u64> = recorded(RECORDED)
        .expect("the runs as their header says")
        .iter()
        .map(|run| run.display)
        .collect();
    displays.sort_unstable();
    let (fast, usual): (Vec<u64>, Vec<u64>) = displays.iter().partition(|&&time| time < 450_000);
    alloc::vec![
        format!(
            "{} idle runs (one session, {} to {} ns; medians of {BLESS_RUNS} of them from {} to {} ns)",
            idle.len(),
            grouped(lowest),
            grouped(highest),
            grouped(blessed.0),
            grouped(blessed.1)
        ),
        format!(
            "1.5 × {}.{:03}…% = {}%, rounded up to {BOUND_PERCENT}%",
            deviation
                .saturating_mul(100_000)
                .div_euclid(base)
                .div_euclid(1000),
            deviation
                .saturating_mul(100_000)
                .div_euclid(base)
                .rem_euclid(1000),
            percent(hundredths(
                deviation.saturating_mul(SAFETY.0),
                base.saturating_mul(SAFETY.1)
            ))
        ),
        format!(
            "a margin of {} over the worst comparison of the idle runs",
            percent(hundredths(
                base.saturating_mul(BOUND_PERCENT),
                deviation.saturating_mul(10_000)
            ))
        ),
        format!(
            "{} of {} runs under a full check's load",
            kept.len(),
            loaded.len()
        ),
        format!(
            "from {}% below to {}% above the idle median of {} ns",
            percent(hundredths(
                median.abs_diff(*kept.first().expect("a run")),
                median
            )),
            percent(hundredths(
                median.abs_diff(*kept.last().expect("a run")),
                median
            )),
            grouped(median)
        ),
        format!(
            "one of {} runs read {}% above the idle median",
            loaded.len(),
            percent(hundredths(beyond.abs_diff(median), median))
        ),
        format!(
            "{} of {} runs recorded, the lowest {} ns, the others from {} ns",
            fast.len(),
            displays.len(),
            grouped(*fast.first().expect("a run")),
            grouped(*usual.first().expect("a run"))
        ),
        format!(
            "at 64×36 about {}% of the renders tried in the idle runs and {}% in the loaded ones, {} of {} and {} of {} of their runs unmeasured",
            unmeasured_percent(&idle_runs, 0),
            unmeasured_percent(&loaded_runs, 0),
            unmeasured_runs(&idle_runs, 0),
            idle_runs.len(),
            unmeasured_runs(&loaded_runs, 0),
            loaded_runs.len()
        ),
        format!(
            "{} runs of another session being recorded as not idle",
            runs_in("not-idle").len()
        ),
    ]
}

/// The texts stating the runs' figures, and how many of [`figures`] each states. The
/// changelog names their sources instead.
const STATING: [(&str, &str, usize); 2] = [
    (
        "spec/benchmarks.md",
        include_str!("../../../spec/benchmarks.md"),
        9,
    ),
    (
        "crates/visual-gpu/src/bench.rs",
        include_str!("bench.rs"),
        2,
    ),
];

#[test]
fn states_the_runs_figures_as_the_record_gives_them() {
    let figures = figures();
    assert_eq!(
        figures,
        [
            "51 idle runs (one session, 88 125 to 94 708 ns; medians of 5 of them from 90 125 to 94 542 ns)",
            "1.5 × 6.787…% = 10.18%, rounded up to 11%",
            "a margin of 1.62 over the worst comparison of the idle runs",
            "61 of 62 runs under a full check's load",
            "from 2.45% below to 0.89% above the idle median of 93 709 ns",
            "one of 62 runs read 17.07% above the idle median",
            "5 of 142 runs recorded, the lowest 367 208 ns, the others from 509 333 ns",
            "at 64×36 about 51% of the renders tried in the idle runs and 74% in the loaded ones, 12 of 51 and 42 of 62 of their runs unmeasured",
            "16 runs of another session being recorded as not idle",
        ]
    );
    // Each text states every figure it is to state: a figure changed in the record fails here
    // until every text says it (the figures retired are retired-patterns.txt's).
    for (file, text, stated) in STATING {
        for figure in figures.iter().take(stated) {
            assert!(text.contains(figure.as_str()), "{file}: {figure}");
        }
    }
    // The sizes shown between: at most 4% of the renders tried unmeasured, every run measured.
    for condition in ["idle", "loaded"] {
        let runs = runs_in(condition);
        for at in [1, 2] {
            assert!(unmeasured_percent(&runs, at) <= 4, "{condition} {at}");
            assert_eq!(unmeasured_runs(&runs, at), 0, "{condition} {at}");
        }
    }
    // Rendering at 1280×720 measured its first 15 renders in every run recorded, which `TRIES`
    // and the texts rest on (finding K16-5 of the sixth deep review of visual block 2 (#34)).
    let all = recorded(RECORDED).expect("the runs as their header says");
    assert!(
        all.iter().all(|run| run.counts.get(3) == Some(&(15, 15))),
        "every run's render at 1280x720"
    );
    // The prose's rounding, half up, at its edges (finding H16-3 of the fifth deep review of
    // visual block 2 (#34): a base of 10 000 gave no half, so that any rounding passed).
    assert_eq!(
        [1, 2, 3, 5, 0].map(|deviation| hundredths(deviation, 20_000)),
        [1, 1, 2, 3, 0],
        "half a hundredth, a whole one, one and a half, two and a half, none"
    );
    assert_eq!(
        (percent(7), grouped(20_005)),
        ("0.07".to_owned(), "20 005".to_owned())
    );
}

#[test]
fn reads_the_runs_recorded_only_as_they_are_written() {
    let header = COLUMNS.join("\t");
    let line = "2\t2026-11-20T16:03:54Z\t2.38\tAC\toff\tidle\t93208\t527292\t10/45\t15/15\t15/15\t15/15\t-";
    let read = |row: &str| recorded(&format!("# a comment\n{header}\n{row}\n"));
    let expected = Recorded {
        session: "2".to_owned(),
        utc: "2026-11-20T16:03:54Z".to_owned(),
        condition: "idle".to_owned(),
        render: 93_208,
        display: 527_292,
        counts: alloc::vec![(10, 45), (15, 15), (15, 15), (15, 15), (0, 0)],
    };
    assert_eq!(read(line), Some(alloc::vec![expected]));
    assert!(
        read(&line.replace("2026-11-20T16:03:54Z\t2.38", "-\t-")).is_some(),
        "a run whose time and load were not logged"
    );
    for (from, to) in [
        ("2\t2026", "0\t2026"),
        ("2\t2026", "x\t2026"),
        ("T16:03:54Z", " 16:03:54"),
        ("2.38", "x"),
        ("2.38", "-1"),
        ("AC", "battery"),
        ("off", "on"),
        ("idle", "quiet"),
        ("93208", "0"),
        ("93208", "9.3"),
        ("527292", "0"),
        ("10/45", "46/45"),
        ("10/45", "0/0"),
        ("10/45", "10"),
        ("10/45", "x/45"),
        ("10/45", "10/x"),
        ("\t-", ""),
        ("\t-", "\t-\t-"),
    ] {
        assert_eq!(read(&line.replacen(from, to, 1)), None, "{from} as {to}");
    }
    assert_eq!(
        recorded(&format!("session\n{line}\n")),
        None,
        "another header"
    );
    assert_eq!(recorded("# a comment\n"), None, "no header");
}

#[test]
fn takes_the_idle_runs_from_the_idle_session_alone() {
    // The rule, decided on 2026-11-21 (findings F16-1, F16-2 and F16-7 of the fourth deep
    // review of visual block 2 (#34)): the idle runs are those of the dedicated idle session,
    // which the constants hold; session 1's 16 others are recorded as not idle.
    let all = recorded(RECORDED).expect("the runs as their header says");
    let idle = renders("idle");
    assert_eq!((RUNS, all.len()), ("spec/gpu-time-runs.txt", 142));
    assert_eq!(
        CONDITIONS.map(|condition| runs_in(condition).len()),
        [51, 16, 62, 4, 7, 1, 1]
    );
    assert!(
        runs_in("idle").iter().all(|run| run.session == "2"
            && ("2026-11-20T16:03:54Z"..="2026-11-20T16:11:36Z").contains(&run.utc.as_str())),
        "the idle session's"
    );
    assert!(
        runs_in("not-idle").iter().all(|run| run.session == "1"),
        "session 1's"
    );
    assert_eq!(
        IDLE,
        Runs {
            date: "2026-11-20",
            condition: "idle",
            count: u64::try_from(idle.len()).expect("64 bits"),
            lowest: *idle.first().expect("a run"),
            highest: *idle.last().expect("a run"),
            blessed: blessed_span(&idle),
        }
    );
    assert_eq!(BLESS_RUNS, 5);
    // The extremes of the medians a bless could take are those of the five lowest and the
    // five highest runs.
    let medians = blessed_times(&idle);
    assert_eq!(
        (medians.first().copied(), medians.last().copied()),
        (Some(IDLE.blessed.0), Some(IDLE.blessed.1))
    );
}

/// The name, system and driver of the GPU whose runs `runs` records, separated by tabs, as
/// its one key line says: none without one, or with another.
fn key(runs: &str) -> Option<&str> {
    let mut keys = runs.lines().filter_map(|line| line.strip_prefix("# Key: "));
    keys.next().filter(|_| keys.next().is_none())
}

/// Whether `runs` is the record of the one GPU `baseline` holds, on its system and driver.
fn records_its_gpu(runs: &str, baseline: &str) -> bool {
    let held = parse_baseline(baseline).expect("the baseline as bless writes it");
    let held: Vec<String> = held
        .iter()
        .map(|(name, entry)| format!("{name}\t{}\t{}", entry.system, entry.driver))
        .collect();
    key(runs).is_some_and(|key| held == [key])
}

#[test]
fn records_the_runs_of_the_gpu_system_and_driver_of_the_baseline_alone() {
    // Finding K15-1 of the sixth deep review of visual block 2 (#34): runs of two systems in
    // one record would be pooled. A record is of one key, so that another starts it anew.
    assert!(records_its_gpu(RECORDED, COMMITTED), "{:?}", key(RECORDED));
    let line = "# Key: Apple M2 Pro (metal)\tmacOS 26.5.2\t-\n";
    for (from, to) in [
        ("Apple M2 Pro", "Apple M3"),
        ("26.5.2", "26.6"),
        ("\t-\n", "\t1.0\n"),
        ("\t-\n", "\n"),
        (line, ""),
        (line, &line.repeat(2)),
    ] {
        let runs = RECORDED.replacen(line, &line.replacen(from, to, 1), 1);
        assert!(!records_its_gpu(&runs, COMMITTED), "{from} as {to}");
    }
    let two = format!("{COMMITTED}B (vulkan)\t1\tLinux\t-\n");
    assert!(!records_its_gpu(RECORDED, &two), "a second GPU's time");
}

#[test]
fn holds_the_baseline_to_the_median_of_its_bless_as_recorded() {
    // Finding K16-3 of the sixth deep review of visual block 2 (#34): the baseline's own runs
    // were in a commit message alone. The newest session that blessed holds its runs, as many
    // as a bless takes, whose median is the time the baseline holds.
    let blesses = runs_in("bless");
    let newest = blesses.last().expect("a bless").session.clone();
    let runs: Vec<u64> = blesses
        .iter()
        .filter(|run| run.session == newest)
        .map(|run| run.render)
        .collect();
    let held: Vec<u64> = parse_baseline(COMMITTED)
        .expect("the baseline as bless writes it")
        .into_values()
        .map(|entry| entry.time)
        .collect();
    assert_eq!(
        <[u64; BLESS_RUNS]>::try_from(runs).map(|runs| alloc::vec![median_of_runs(runs)]),
        Ok(held)
    );
}

#[test]
fn derives_its_bound_from_the_idle_runs_recorded() {
    // The bound recomputed with the comparison `check` makes: every idle run, its deviation
    // stretched 1.5 times, within the bound of every median of five idle runs, and not within
    // one percent less.
    let idle = renders("idle");
    let stretched_within = |percent: u64| {
        blessed_times(&idle).iter().all(|&base| {
            idle.iter().all(|&run| {
                let stretched = if run >= base {
                    SAFETY.1 * base + SAFETY.0 * (run - base)
                } else {
                    SAFETY.1 * base - SAFETY.0 * (base - run)
                };
                within(stretched, SAFETY.1 * base, percent)
            })
        })
    };
    assert_eq!(SAFETY, (3, 2), "1.5");
    assert!(
        stretched_within(BOUND_PERCENT) && !stretched_within(BOUND_PERCENT - 1),
        "{BOUND_PERCENT}"
    );
    assert_eq!((bound_percent(IDLE), BOUND_PERCENT), (11, 11));
    // Each side of the largest deviation decides it, rounded up, each over its own median, and
    // runs measuring 0 ns give a bound of 0%, never a division by zero.
    let runs = |lowest: u64, highest: u64, blessed: (u64, u64)| Runs {
        lowest,
        highest,
        blessed,
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
            bound_percent(runs(lowest, highest, (1000, 1000))),
            percent,
            "{lowest}..{highest}"
        );
    }
    assert_eq!(
        bound_percent(runs(900, 1100, (1000, 2000))),
        83,
        "below: 1100 of 2000 ns, 82.5%"
    );
    assert_eq!(
        bound_percent(runs(1000, 2000, (1000, 1000))),
        150,
        "above: 1000 of 1000 ns"
    );
    assert_eq!(
        bound_percent(runs(950, 2000, (1000, 1000))),
        150,
        "above, though below deviates too"
    );
    assert_eq!(bound_percent(runs(0, 0, (0, 0))), 0);
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
fn holds_every_run_but_one_within_the_bound_of_every_idle_bless() {
    // Every run recorded but one is within the bound of every median of five idle runs: the
    // loaded ones (under a full check's load), those that were not idle, those set apart and
    // the blesses. The one beyond, loaded too, reads 17.07% above the idle median, its cause
    // unknown: a time beyond the bound is first run again, as the failure line says.
    let idle = renders("idle");
    let beyond: Vec<Recorded> = recorded(RECORDED)
        .expect("the runs as their header says")
        .into_iter()
        .filter(|run| {
            blessed_times(&idle)
                .iter()
                .any(|&base| !within(run.render, base, BOUND_PERCENT))
        })
        .collect();
    assert_eq!(
        beyond
            .iter()
            .map(|run| (run.condition.as_str(), run.utc.as_str(), run.render))
            .collect::<Vec<_>>(),
        [("loaded", "2026-11-20T13:26:06Z", 109_708)]
    );
    assert!(
        blessed_times(&idle)
            .iter()
            .all(|&base| !within(109_708, base, BOUND_PERCENT)),
        "beyond the bound of every bless"
    );
    // The loaded runs but that one are inside the idle runs' span.
    let mut outside = renders("loaded");
    outside.retain(|time| !(IDLE.lowest..=IDLE.highest).contains(time));
    assert_eq!(outside, [109_708]);
    // A bless under load is one an idle bless could have written: it needs no idle machine,
    // only AC power.
    let (lowest, highest) = blessed_span(&renders("loaded"));
    assert!(
        IDLE.blessed.0 <= lowest && highest <= IDLE.blessed.1,
        "{lowest}..{highest}"
    );
    assert_eq!(
        BLESS_CONDITION,
        "on AC power, outside Low Power Mode, on the machine whose GPU it holds"
    );
}

#[test]
fn blesses_the_median_of_five_runs() {
    // Each run measured as a check's one run is and said, then their middle one written,
    // whatever their order, where one run would have written the first.
    let runs = [110, 90, 105, 80, 100].map(|time| timing(time, 15));
    let mut fake = Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        runs: runs.to_vec(),
        ..Fake::default()
    };
    let (said, outcome) = command(&mut fake, &["bless"]);
    assert_eq!((outcome, fake.measures), (Ok(()), BLESS_RUNS));
    let shown = "GPU A (metal): render 64x36: 7000 ns (15 of 15 measured) (shown)\nGPU A (metal): render 320x180: 12000 ns (15 of 16 measured) (shown)\nGPU A (metal): render 640x360: 30000 ns (15 of 17 measured) (shown)\nGPU A (metal): display 1280x720: 40000 ns (15 of 18 measured) (shown)\n";
    let mut expected = String::new();
    for (run, time) in (1_usize..).zip([110_u64, 90, 105, 80, 100]) {
        expected.extend([format!(
            "run {run} of 5\n{shown}GPU A (metal): render 1280x720: {time} ns (15 of 15 measured)\n"
        )]);
    }
    assert_eq!(
        said,
        format!(
            "{expected}GPU A (metal): the median of 5 runs: 100 ns\nwrote spec/gpu-time-baseline.txt\n"
        )
    );
    assert_eq!(
        fake.written,
        [held(&[("GPU A (metal)", 100)])],
        "the middle run's time"
    );
    // A run whose clock measured too few renders ends the bless: nothing written.
    let mut unmeasured = Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        runs: alloc::vec![timing(100, 15), Timing::Unmeasured { measured: 3 }],
        ..Fake::default()
    };
    assert_eq!(
        command(&mut unmeasured, &["bless"]).1,
        failure(
            "GPU A (metal): its clock measured 3 of 45 renders at 1280x720, fewer than 15: no time to bless (run it again with no other heavy work running (see: cargo xtask bless-benchmarks); a clock failing so is a defect to report)"
        ),
        "the second run"
    );
    assert_eq!((unmeasured.measures, unmeasured.written.len()), (2, 0));
}

#[test]
fn fails_when_the_gpu_measured_is_not_the_one_opened() {
    // Each run opens the GPU afresh: another one is never compared nor written under the
    // name of the one opened.
    for (args, again) in [("check", CHECK), ("bless", "cargo xtask bless-benchmarks")] {
        let mut fake = Fake {
            gpu: Some(Ok(("GPU A (metal)".to_owned(), "-".to_owned()))),
            measured: Some(Ok(times("GPU\u{1b} B", 1))),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut fake, &[args]).1,
            failure(&format!(
                "the GPU measured is \"GPU\\u{{1b}} B\", not \"GPU A (metal)\", the one opened: no time to {args} (run it again (see: {again}))"
            )),
            "{args}"
        );
        assert_eq!((fake.measures, fake.written.len()), (1, 0), "{args}");
    }
}

#[test]
fn takes_the_middle_of_the_runs_of_a_bless() {
    for times in [
        [1, 2, 3, 4, 5],
        [5, 4, 3, 2, 1],
        [3, 900, 1, 800, 2],
        [3, 3, 3, 1, 9],
    ] {
        assert_eq!(median_of_runs(times), 3, "{times:?}");
    }
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
    assert_eq!((fastest, slowest), (89_000, 111_000), "today's bound, ±11%");
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
            "\"GPU C (vulkan)\": no baseline (the baseline holds \"GPU A (metal)\", \"GPU B (vulkan)\"): its time is not checked, nothing rendered\n".to_owned(),
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
    // The second remedy followed: the bless writes the file, on which the check then passes.
    let mut blessing = Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        ..Fake::default()
    };
    assert_eq!(command(&mut blessing, &["bless"]).1, Ok(()), "blessed");
    blessing.held = blessing.written.pop().map(Ok);
    assert_eq!(command(&mut blessing, &["check"]).1, Ok(()), "then checked");
    // The first remedy followed: the file restored by its hint, the check passes on the maintainer's
    // Mac, its system and driver.
    let mut restored = Fake {
        measured: Some(Ok(times("Apple M2 Pro (metal)", 93_125))),
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
        "GPU A (metal): {checked} ns, baseline 100000 ns: beyond ±11% (3/2 times the largest deviation of one of 51 idle runs from the median of any 5 of them, measured 2026-11-20; spec/benchmarks.md, \"Visual library\") (run it again with no other heavy work running (see: {CHECK}); still beyond, it is a regression of the work, or a change of the machine the baseline does not record: measure the runs again (spec/benchmarks.md, \"Visual library\"), then bless with the reason (see: cargo xtask bless-benchmarks))"
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

/// What a baseline not as `bless` writes it is told to do, after its reason: restored alone,
/// since `bless` reads the other GPUs' times (finding K1-2 of the sixth deep review of visual
/// block 2 (#34): it named a bless, which ends in the same refusal).
const RESTORE: &str =
    ": not as bless writes it: restore it (see: git checkout HEAD -- :/spec/gpu-time-baseline.txt)";

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
    // A GPU the baseline holds whose clock is gone (a wgpu or driver change, whatever took
    // it) fails the check before rendering: the gate is never silently vacuous on the machine
    // it exists for.
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
                "GPU A (metal): this GPU has no clock its programs can read, though the baseline holds its time: no time to check (a wgpu or driver change that took its clock is a defect to report)"
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
            "\"GPU C (vulkan)\": no baseline (the baseline holds \"GPU A (metal)\"): its time is not checked, nothing rendered\n".to_owned(),
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
            CHECK
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
            said.trim_start_matches("run 1 of 5\n").starts_with(
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
}

#[test]
fn says_what_helps_a_baseline_it_cannot_read_by_the_kind_of_its_error() {
    // Findings H13-4 and J1-n1 of the fifth deep review of visual block 2 (#34): every read
    // failure was told to restore the file, a permission or a directory in its place
    // included. A remedy per kind of error, never a restore beside a directory, whose
    // untracked files git deletes (K13-3 of the sixth), and the command run again is the one
    // given (K7-14).
    for (args, again) in [
        ("check", format!("then check again (see: {CHECK})")),
        (
            "bless",
            "then bless again (see: cargo xtask bless-benchmarks)".to_owned(),
        ),
    ] {
        for (kind, remedy) in [
            (io::ErrorKind::InvalidData, RESTORE.to_owned()),
            (
                io::ErrorKind::IsADirectory,
                format!(": a directory stands where the file should be: move it away, {again}"),
            ),
            (
                io::ErrorKind::PermissionDenied,
                format!(": make it readable, {again}"),
            ),
            (io::ErrorKind::NotADirectory, String::new()),
            (io::ErrorKind::Other, String::new()),
        ] {
            let mut unreadable = Fake {
                held: Some(Err(kind)),
                ..Fake::default()
            };
            assert_eq!(
                command(&mut unreadable, &[args]).1,
                failure(&format!(
                    "cannot read spec/gpu-time-baseline.txt: refused{remedy}"
                )),
                "{args}: {kind:?}"
            );
        }
    }
    for args in ["check", "bless"] {
        let mut invalid = Fake {
            held: Some(Ok("GPU A 100\n".to_owned())),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut invalid, &[args]).1,
            failure(&format!(
                "spec/gpu-time-baseline.txt: line 1: not a GPU's name, its time, its system and its driver, separated by tabs{RESTORE}"
            )),
            "{args}"
        );
    }
}

#[test]
fn asks_whether_it_can_write_before_it_measures() {
    // Findings K13-11 and K3-2 of the sixth deep review of visual block 2 (#34): the write
    // was first tried after every run was measured. Refused before the GPU is opened.
    let mut read_only = Fake {
        gpu: Some(Err("not to be opened".to_owned())),
        read_only: true,
        ..Fake::default()
    };
    assert_eq!(
        command(&mut read_only, &["bless"]),
        (
            String::new(),
            failure("cannot write spec/gpu-time-baseline.txt: read-only")
        )
    );
    assert_eq!(read_only.measures, 0, "nothing measured");
    // The write itself stays checked, whatever the world said before measuring.
    let mut full = Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        write_fails: true,
        ..Fake::default()
    };
    assert_eq!(
        command(&mut full, &["bless"]).1,
        failure("cannot write spec/gpu-time-baseline.txt: disk full")
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
fn reads_the_baseline_only_as_bless_writes_it() {
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
            entry(93_125)
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
        "run 1 of 5\nGPU\\u{1b}[2J A (metal): render 64x36: 7000 ns (15 of 15 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 320x180: 12000 ns (15 of 16 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 640x360: 30000 ns (15 of 17 measured) (shown)\nGPU\\u{1b}[2J A (metal): display 1280x720: 40000 ns (15 of 18 measured) (shown)\nGPU\\u{1b}[2J A (metal): render 1280x720: 120 ns (15 of 15 measured)\n"
    );
    assert_eq!(
        (outcome, blessing.written.len()),
        (
            failure(
                "the GPU measured is \"GPU\\u{1b}[2J\\nA (metal)\", not \"GPU A (metal)\", the one opened: no time to bless (run it again (see: cargo xtask bless-benchmarks))"
            ),
            0
        ),
        "its times said, escaped, none written"
    );
    let mut other = Fake {
        gpu: Some(Ok(("GPU\u{7}C\r(vulkan)".to_owned(), "-".to_owned()))),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut other, &["check"]),
        (
            "\"GPU\\u{7}C\\r(vulkan)\": no baseline (the baseline holds \"GPU A (metal)\"): its time is not checked, nothing rendered\n".to_owned(),
            Ok(())
        )
    );
    // Finding H4-2 of the fifth deep review of visual block 2 (#34): bidirectional
    // formatting characters passed, and nothing bounded a line.
    let long = "x".repeat(1_100);
    for (given, line) in [
        (
            "GPU\u{202e}C\u{2028}(vulkan)",
            "\"GPU\\u{202e}C\\u{2028}(vulkan)\": no baseline (the baseline holds \"GPU A (metal)\"): its time is not checked, nothing rendered\n"
                .to_owned(),
        ),
        (long.as_str(), format!("\"{}…\n", "x".repeat(998))),
        // Finding K4-3 of the sixth deep review of visual block 2 (#34): two names that differ
        // by their spaces alone read the same.
        (
            "GPU  A (metal)",
            "\"GPU  A (metal)\": no baseline (the baseline holds \"GPU A (metal)\"): its time is not checked, nothing rendered\n"
                .to_owned(),
        ),
        (
            "GPU\u{a0}A (metal)",
            "\"GPU\\u{a0}A (metal)\": no baseline (the baseline holds \"GPU A (metal)\"): its time is not checked, nothing rendered\n"
                .to_owned(),
        ),
    ] {
        let mut odd = Fake {
            gpu: Some(Ok((given.to_owned(), "-".to_owned()))),
            held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut odd, &["check"]),
            (line, Ok(())),
            "{given:?}"
        );
    }
    assert_eq!(
        command(&mut Fake::default(), &[&long]).1,
        Err(BenchError {
            code: 2,
            line: format!("unknown command \"{}… {HELP}", "x".repeat(982))
        }),
        "its reason bounded (17 characters, 982 and the mark), its hint whole"
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
fn cuts_no_remedy_by_a_long_reason() {
    // Findings K13-2, K14-5 and L5-n3 of the sixth deep review of visual block 2 (#34): the
    // line was bounded whole, so a driver's name, a damaged file's line or an argument hid the
    // command that helps. Each reason of 2000 characters, its remedy whole after it.
    let long = "x".repeat(2_000);
    let name = format!("GPU {long}");
    let fake = |adapter: &str, driver: &str, measured: &str, checked: Timing, text: String| {
        let mut all = times(measured, 1);
        all.checked = checked;
        Fake {
            gpu: Some(Ok((adapter.to_owned(), driver.to_owned()))),
            measured: Some(Ok(all)),
            held: Some(Ok(text)),
            ..Fake::default()
        }
    };
    let baseline = held(&[(name.as_str(), 100_000)]);
    let few = Timing::Unmeasured { measured: 3 };
    let mut unread = fake("A", "-", "A", few, String::new());
    unread.held = Some(Err(io::ErrorKind::InvalidData));
    unread.refusal.clone_from(&long);
    for (mut odd, reason, remedy) in [
        (
            unread,
            "cannot read spec/gpu-time-baseline.txt: ",
            RESTORE.to_owned(),
        ),
        (
            fake("A", "-", "A", few, baseline.trim_end().to_owned()),
            "spec/gpu-time-baseline.txt: line 5: \"GPU ",
            RESTORE.to_owned(),
        ),
        (
            fake(&name, &long, &name, few, baseline.clone()),
            "GPU ",
            " (measure the runs again (spec/benchmarks.md, \"Visual library\"), then bless (see: cargo xtask bless-benchmarks))".to_owned(),
        ),
        (
            fake(&name, "-", "B", few, baseline.clone()),
            "the GPU measured is \"B\", not \"GPU ",
            format!(" (run it again (see: {CHECK}))"),
        ),
        (
            fake(&name, "-", &name, few, baseline.clone()),
            "GPU ",
            format!(" (run it again with no other heavy work running (see: {CHECK}); a clock failing so is a defect to report)"),
        ),
        (
            fake(&name, "-", &name, timing(1, 15), baseline),
            "GPU ",
            beyond(1).replace("GPU A (metal): 1 ns, baseline 100000 ns", ""),
        ),
    ] {
        let kept: String = format!("{reason}{long}").chars().take(999).collect();
        assert_eq!(
            command(&mut odd, &["check"]).1.map_err(|error| error.line),
            Err(format!("{kept}…{remedy}"))
        );
    }
}

/// A stdout that cannot be written to, for an error of this kind, from the text holding these
/// words on.
#[derive(Debug)]
struct Broken(io::ErrorKind, &'static str);

impl io::Write for Broken {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if String::from_utf8_lossy(bytes).contains(self.1) {
            Err(io::Error::new(self.0, "broken"))
        } else {
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn fails_on_a_stdout_it_cannot_write_to_but_a_closed_one() {
    // Finding K13-10 of the sixth deep review of visual block 2 (#34): every failure to write
    // to stdout was dropped, a full disk's included.
    let passes = || Fake {
        measured: Some(Ok(times("GPU A (metal)", 1))),
        held: Some(Ok(held(&[("GPU A (metal)", 1)]))),
        ..Fake::default()
    };
    let unheld = || Fake {
        held: Some(Ok(held(&[("GPU B", 1)]))),
        ..passes()
    };
    // Each line the command says, the only one that cannot be written.
    for (args, mut fake, line) in [
        ("help", passes(), "usage: "),
        ("check", passes(), "render 64x36"),
        ("check", passes(), "display 1280x720"),
        ("check", passes(), "render 1280x720"),
        ("check", unheld(), "no baseline"),
        ("bless", passes(), "run 1 of 5"),
        ("bless", passes(), "the median of 5 runs"),
        ("bless", passes(), "wrote "),
    ] {
        let words = [args.to_owned()];
        let mut said = |kind| run(&words, &mut fake, &mut Broken(kind, line));
        assert_eq!(
            said(io::ErrorKind::BrokenPipe),
            Ok(()),
            "{args}, {line}: a closed stdout"
        );
        assert_eq!(
            said(io::ErrorKind::StorageFull),
            failure("cannot write to stdout: broken"),
            "{args}, {line}"
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
    // No file yet, and a directory to write it in: it can be written, and asking creates none.
    assert_eq!(system.writable().map_err(|error| error.kind()), Ok(()));
    assert!(!root.0.join(BASELINE).exists(), "nothing created");
    system.write_baseline("A\t1\n").expect("written");
    assert_eq!(system.writable().map_err(|error| error.kind()), Ok(()));
    assert_eq!(
        system.read_baseline().map_err(|error| error.kind()),
        Ok(Some("A\t1\n".to_owned())),
        "asking whether it can be written truncates nothing"
    );
    std::fs::remove_file(root.0.join(BASELINE)).expect("removed");
    std::fs::create_dir(root.0.join(BASELINE)).expect("a directory in its place");
    assert_eq!(
        system.read_baseline().map_err(|error| error.kind()),
        Err(io::ErrorKind::IsADirectory),
        "a baseline that cannot be read (EISDIR, on Linux and macOS)"
    );
    assert_eq!(
        system.writable().map_err(|error| error.kind()),
        Err(io::ErrorKind::IsADirectory),
        "nor written"
    );
    assert_eq!(
        system
            .measure()
            .map(|times| times.adapter.is_empty())
            .map_err(|error| error.to_string()),
        Ok(false),
        "the first GPU measured: a time, the checks running on a GPU with a clock"
    );
    assert!(
        system.gpu.is_none(),
        "the GPU measured closed, the next run opening it afresh"
    );
}

#[test]
fn refuses_a_directory_without_spec() {
    // Finding K13-5 of the sixth deep review of visual block 2 (#34): run elsewhere than from
    // the repository's root, the check said the baseline was missing, with a restore that
    // succeeds and changes nothing there.
    let root = scratch("elsewhere");
    std::fs::remove_dir(root.0.join("spec")).expect("removed");
    for args in ["check", "bless"] {
        let mut system = System::new(root.0.clone());
        let mut out = Vec::new();
        assert_eq!(
            run(&[args.to_owned()], &mut system, &mut out),
            failure(&format!(
                "cannot read {BASELINE}: no spec directory at the root of the repository it runs in (here when it runs in none): run it inside the repository that holds the baseline"
            )),
            "{args}"
        );
        assert!(system.gpu.is_none(), "{args}: no GPU opened");
    }
    assert_eq!(
        System::new(root.0.clone())
            .writable()
            .map_err(|error| error.to_string()),
        Err("no spec directory at the root of the repository it runs in (here when it runs in none): run it inside the repository that holds the baseline".to_owned())
    );
}

#[test]
fn finds_the_root_from_any_directory_of_the_repository_alone() {
    // A new finding of the seventh deep review of visual block 2 (#34): the check hint, run
    // from a subdirectory, read `spec/` there, where `cargo run` keeps the directory it runs in.
    // Finding PG-5 of the eighth deep review of visual block 2 (#34): the root is bounded by
    // the nearest `.git` alone, so that no stray `spec/` below it or above it is read.
    let outside = scratch("walk");
    let root = outside.0.join("repo");
    let below = root.join("a/b");
    std::fs::create_dir_all(below.join("spec")).expect("a stray spec/ below the root");
    assert_eq!(
        root_from(&below),
        below,
        "no repository: where it runs alone"
    );
    std::fs::create_dir(root.join(".git")).expect("its git");
    assert_eq!(
        [root_from(&below), root_from(&root)],
        [root.clone(), root.clone()],
        "the repository's root, whatever spec/ is below it or above it (the scratch's)"
    );
    std::fs::remove_dir(root.join(".git")).expect("no git directory");
    std::fs::write(root.join(".git"), "gitdir: /elsewhere\n").expect("a git file");
    assert_eq!(
        root_from(&below),
        root,
        "a linked worktree's root, whose .git is a file"
    );
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
        // Finding K4-4 of the sixth deep review of visual block 2 (#34): two versions not
        // UTF-8 read the same.
        (
            "printf",
            &["26\\377"][..],
            "cannot read the system's version: printf wrote a version that is not UTF-8 text",
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

/// The sizes of `SIZES` in `text`, one line each between its opening and its closing line,
/// `[width, height],` or `LARGEST,` (its one declaration, `const LARGEST = [width, height] as
/// const;`); none when it is written otherwise, which fails the test rather than skip a size.
fn suite_sizes(text: &str) -> Option<Vec<(u32, u32)>> {
    let size = |pair: &str| {
        let (width, height) = pair.split_once(", ")?;
        Some((width.parse().ok()?, height.parse().ok()?))
    };
    let largest = || {
        only_line(text, "const LARGEST = ")?
            .strip_prefix("const LARGEST = [")?
            .strip_suffix("] as const;")
    };
    let mut lines = text.lines().skip_while(|line| {
        *line != "export const SIZES: readonly (readonly [number, number])[] = ["
    });
    let _opening = lines.next()?;
    let mut sizes = Vec::new();
    for line in lines {
        if line == "];" {
            return Some(sizes);
        }
        let pair = match line {
            "  LARGEST," => largest()?,
            _ => line.strip_prefix("  [")?.strip_suffix("],")?,
        };
        sizes.push(size(pair)?);
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
    // Commit cbff7316 wrote the largest size by its name, which this read as no size at all.
    let named = |declared: &str| sizes(&format!("  [1, 2],\n  LARGEST,\n];\n{declared}"));
    assert_eq!(
        named("const LARGEST = [3, 4] as const;\n"),
        Some(vec![(1, 2), (3, 4)]),
        "the largest by its name"
    );
    for declared in [
        "",
        "const LARGEST = [3, 4];\n",
        "const LARGEST = [3, 4] as const;\n".repeat(2).as_str(),
    ] {
        assert_eq!(
            named(declared),
            None,
            "{declared:?}: not its one declaration"
        );
    }
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
