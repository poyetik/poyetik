//! Tests of the GPU's time benchmark: the command on a fake outside world, the baseline's
//! text, and the measurement and files of the real one.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use std::io;

use super::{
    BASELINE, BenchError, BenchIo, CHECKED, Failure, IMAGE, SHOWN, SQUARE_CYCLE, SQUARE_RATE,
    SQUARE_SIDE, System, TOLERANCE_PERCENT, Times, USAGE, format_baseline, measure, median_of,
    parse_baseline, run, whole_nanoseconds,
};
use crate::GpuSquare;

/// A fake outside world: what measuring gives, the baseline held, and what was written.
#[derive(Debug, Default)]
struct Fake {
    /// What measuring gives: a time, none (no clock), or a failure's words.
    measured: Option<Result<Option<Times>, String>>,
    /// The baseline's text, `None` when there is no file; `Err` when it cannot be read.
    held: Option<Result<String, String>>,
    /// Whether writing fails.
    read_only: bool,
    /// What was written.
    written: Vec<String>,
}

impl BenchIo for Fake {
    fn measure(&mut self) -> Result<Option<Times>, Failure> {
        match self.measured.clone().unwrap_or(Ok(None)) {
            Ok(times) => Ok(times),
            Err(words) => Err(words.into()),
        }
    }

    fn read_baseline(&mut self) -> io::Result<Option<String>> {
        match self.held.clone() {
            None => Ok(None),
            Some(Ok(text)) => Ok(Some(text)),
            Some(Err(words)) => Err(io::Error::other(words)),
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

/// Times of the GPU `adapter`, `checked` nanoseconds at the checked size.
fn times(adapter: &str, checked: u64) -> Times {
    Times {
        adapter: adapter.to_owned(),
        checked,
        shown: [7_000, 12_000, 30_000],
        display: 40_000,
    }
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
        (&[][..], "no command given (see: visual-gpu help)"),
        (
            &["run"][..],
            "unknown command \"run\" (see: visual-gpu help)",
        ),
        (
            &["check", "now"][..],
            "`check` takes no argument (see: visual-gpu help)",
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
        (format!("{USAGE}\n"), Ok(()))
    );
    assert!(
        USAGE.contains(&format!("within ±{TOLERANCE_PERCENT}%,")),
        "the usage states the tolerance the check applies"
    );
}

#[test]
fn checks_a_gpu_that_has_a_baseline_within_its_tolerance_and_shows_any_other() {
    let shown = "GPU A (metal): render 64x36: 7000 ns (shown)\nGPU A (metal): render 320x180: 12000 ns (shown)\nGPU A (metal): render 640x360: 30000 ns (shown)\nGPU A (metal): display 1280x720: 40000 ns (shown)\n";
    let baseline = "# a comment\nGPU A (metal)\t100000\nGPU B (vulkan)\t5\n".to_owned();
    for (checked, outcome) in [
        (110_000, Ok(())),
        (90_000, Ok(())),
        (
            110_001,
            failure(
                "GPU A (metal): 110001 ns, baseline 100000 ns: beyond ±10% (bless it, with the reason, if the work changed)",
            ),
        ),
        (
            89_999,
            failure(
                "GPU A (metal): 89999 ns, baseline 100000 ns: beyond ±10% (bless it, with the reason, if the work changed)",
            ),
        ),
    ] {
        let mut fake = Fake {
            measured: Some(Ok(Some(times("GPU A (metal)", checked)))),
            held: Some(Ok(baseline.clone())),
            ..Fake::default()
        };
        assert_eq!(
            command(&mut fake, &["check"]),
            (
                format!("{shown}GPU A (metal): render 1280x720: {checked} ns\n"),
                outcome
            ),
            "{checked}"
        );
        assert!(fake.written.is_empty(), "check writes nothing");
    }
    assert_eq!(TOLERANCE_PERCENT, 10);
    let mut unknown = Fake {
        measured: Some(Ok(Some(times("GPU C (vulkan)", 1)))),
        held: Some(Ok(baseline)),
        ..Fake::default()
    };
    let (said, outcome) = command(&mut unknown, &["check"]);
    assert_eq!(outcome, Ok(()), "a GPU without a baseline passes");
    assert!(
        said.ends_with("GPU C (vulkan): render 1280x720: 1 ns\nGPU C (vulkan): no baseline (the baseline holds GPU A (metal), GPU B (vulkan)): shown, not checked\n"),
        "the GPUs it holds named, so that a GPU renamed by its driver is seen: {said}"
    );
    let mut missing = Fake {
        measured: Some(Ok(Some(times("GPU A (metal)", 100_000)))),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut missing, &["check"]).1,
        failure(&format!(
            "{BASELINE} is missing: no time can be checked (`visual-gpu bless` writes it)"
        )),
        "no file: the check fails, never passes as shown"
    );
    let mut empty = Fake {
        measured: Some(Ok(Some(times("GPU C (vulkan)", 1)))),
        held: Some(Ok("# Only comments.\n".to_owned())),
        ..Fake::default()
    };
    assert!(
        command(&mut empty, &["check"]).0.ends_with(
            "GPU C (vulkan): no baseline (the baseline holds none): shown, not checked\n"
        ),
        "a file holding no GPU"
    );
}

#[test]
fn blesses_this_gpus_time_beside_the_others() {
    let mut fake = Fake {
        measured: Some(Ok(Some(times("GPU A (metal)", 120)))),
        held: Some(Ok("GPU A (metal)\t100\nGPU B (vulkan)\t5\n".to_owned())),
        ..Fake::default()
    };
    let (said, outcome) = command(&mut fake, &["bless"]);
    assert_eq!(outcome, Ok(()));
    assert!(said.ends_with(&format!("wrote {BASELINE}\n")), "{said}");
    assert_eq!(
        fake.written,
        [format_baseline(&BTreeMap::from([
            ("GPU A (metal)".to_owned(), 120),
            ("GPU B (vulkan)".to_owned(), 5)
        ]))],
        "its line replaced, the others kept"
    );
    let mut first = Fake {
        measured: Some(Ok(Some(times("GPU A (metal)", 120)))),
        ..Fake::default()
    };
    assert_eq!(command(&mut first, &["bless"]).1, Ok(()));
    assert_eq!(
        parse_baseline(first.written.first().expect("written")),
        Ok(BTreeMap::from([("GPU A (metal)".to_owned(), 120)])),
        "the first baseline"
    );
}

#[test]
fn shows_no_time_on_a_gpu_without_a_clock_and_blesses_none() {
    let line = "this GPU has no clock its programs can read: no time\n";
    let mut held = Fake {
        held: Some(Ok(String::new())),
        ..Fake::default()
    };
    assert_eq!(command(&mut held, &["check"]), (line.to_owned(), Ok(())));
    assert_eq!(
        command(&mut Fake::default(), &["bless"]),
        (
            line.to_owned(),
            failure("no time to bless: this GPU has no clock")
        )
    );
}

#[test]
fn names_what_it_cannot_measure_read_or_write() {
    let mut unmeasured = Fake {
        measured: Some(Err("no GPU adapter: none".to_owned())),
        held: Some(Ok(String::new())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut unmeasured, &["check"]).1,
        failure("cannot measure the GPU: no GPU adapter: none")
    );
    let mut unreadable = Fake {
        held: Some(Err("denied".to_owned())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut unreadable, &["check"]).1,
        failure("cannot read spec/gpu-time-baseline.txt: denied")
    );
    let mut invalid = Fake {
        held: Some(Ok("GPU A 100\n".to_owned())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut invalid, &["bless"]).1,
        failure("spec/gpu-time-baseline.txt: line 1: not a GPU's name, a tab and its time")
    );
    let mut read_only = Fake {
        measured: Some(Ok(Some(times("GPU A (metal)", 1)))),
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
    let times = BTreeMap::from([("A (metal)".to_owned(), 1), ("B (vulkan)".to_owned(), 22)]);
    let text = format_baseline(&times);
    assert!(text.starts_with('#') && text.ends_with("A (metal)\t1\nB (vulkan)\t22\n"));
    assert_eq!(parse_baseline(&text), Ok(times));
    assert_eq!(parse_baseline(""), Ok(BTreeMap::new()));
    for (bad, why) in [
        ("A 1\n", "line 1: not a GPU's name, a tab and its time"),
        (
            "# c\nA\t1.5\n",
            "line 2: \"1.5\" is not a whole number of nanoseconds",
        ),
        (
            "A\t-1\n",
            "line 1: \"-1\" is not a whole number of nanoseconds",
        ),
        ("A\t1\nA\t2\n", "line 2: A written twice"),
    ] {
        assert_eq!(parse_baseline(bad), Err(why.to_owned()), "{bad:?}");
    }
}

#[test]
fn rounds_a_time_to_whole_nanoseconds() {
    for (nanoseconds, whole) in [
        (0.0_f64, 0),
        (1.4_f64, 1),
        (1.6_f64, 2),
        (92_500.0_f64, 92_500),
        (1e30_f64, u64::MAX),
    ] {
        assert_eq!(whole_nanoseconds(nanoseconds), whole, "{nanoseconds}");
    }
}

#[test]
fn measures_this_gpu_by_its_clock() {
    let gpu = GpuSquare::new(wgpu::Backends::all()).expect("a GPU (a software one in CI)");
    let measured = measure(&gpu).expect("measured");
    assert_eq!(
        measured.is_some(),
        gpu.timer.is_some(),
        "a time exactly when the GPU has a clock"
    );
    if let Some(times) = measured {
        assert_eq!(times.adapter, gpu.adapter());
        assert!(
            times.checked > 0 && times.shown.iter().all(|&time| time > 0),
            "{times:?}"
        );
        // Encoding 1280x720 for display moves 20 bytes a pixel through the GPU's memory (16
        // read, 4 written): 18,432,000 bytes, at least 184 ns even at 100 TB/s, more than ten
        // times the fastest GPU memory made (about 8 TB/s, 2026).
        assert!(times.display >= 100, "{times:?}");
    }
    let untimed = GpuSquare::open(wgpu::Backends::all(), |limits| limits, false)
        .expect("a GPU (a software one in CI)");
    assert_eq!(
        measure(&untimed).map_err(|error| error.to_string()),
        Ok(None)
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
    assert!(
        system.read_baseline().is_err(),
        "a baseline that cannot be read"
    );
    let clock = GpuSquare::new(wgpu::Backends::all())
        .expect("a GPU (a software one in CI)")
        .timer
        .is_some();
    assert_eq!(
        system
            .measure()
            .map(|times| times.is_some())
            .map_err(|error| error.to_string()),
        Ok(clock),
        "the first GPU measured, a time exactly when it has a clock"
    );
}

#[test]
fn takes_the_median_of_the_renders_after_the_warm_up() {
    // Renders take 0, 1, 2, … nanoseconds, in reverse order of the warm-up's count: the
    // fifteen measured take 14 down to 0, whose median is 7; any warm-up render counted
    // would take 15 or more.
    let mut count = 0_u32;
    let mut render = || {
        count = count.saturating_add(1);
        Ok(Some(f64::from(30_u32.saturating_sub(count))))
    };
    assert_eq!(
        median_of(&mut render).map_err(|error| error.to_string()),
        Ok(Some(7))
    );
    assert_eq!(count, 30, "fifteen to warm up, then fifteen measured");
    assert_eq!(
        median_of(&mut || Ok(None)).map_err(|error| error.to_string()),
        Ok(None),
        "nothing measured"
    );
    // Every third render unmeasured (a pass whose timestamps the GPU did not write): of the
    // fifteen after the warm-up (renders 16 to 30, taking 30 less their count), the ten
    // measured take 14, 13, 11, 10, 8, 7, 5, 4, 2 and 1, whose median, the sixth smallest,
    // is 8.
    let mut every = 0_u32;
    let mut sometimes = || {
        every = every.saturating_add(1);
        Ok((!every.is_multiple_of(3)).then(|| f64::from(30_u32.saturating_sub(every))))
    };
    assert_eq!(
        median_of(&mut sometimes).map_err(|error| error.to_string()),
        Ok(Some(8)),
        "the renders measured alone"
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
        held: Some(Ok(String::new())),
        ..Fake::default()
    };
    assert_eq!(
        command(&mut fake, &["check"]).1,
        failure("cannot measure the GPU: Validation Error Caused by: In Queue::submit [0m"),
        "a multi-line error of wgpu, its control characters as spaces, on one line"
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
