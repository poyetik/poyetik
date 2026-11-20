//! The GPU's time benchmark (`spec/benchmarks.md`, "Visual library"): the time the GPU takes
//! to render the square, by its own clock, held to a baseline per named GPU.
//!
//! A GPU's time depends on the GPU and on the machine's state: it is checked only on a GPU
//! whose baseline the repository holds (`spec/gpu-time-baseline.txt`: the maintainer's Mac,
//! blessed and checked on AC power, outside Low Power Mode), and shown on any other (the
//! software GPU of CI). The baseline is keyed by the GPU's name: another machine whose GPU
//! has the same name is checked against it. A GPU runner of CI, and a baseline keyed by the
//! machine, are roadmap #35 step 12's. What does not depend on
//! the GPU, the transfers and work of an image ([`render_cost`](crate::render_cost)), is held
//! exactly by the tests on every machine.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Write as _;
use std::io::{self, Write};

use visual_boxes::Square;
use visual_core::{DisplayImage, Image, ImageIndex, ImageRate, ImageSize, Seconds};

use crate::GpuSquare;

#[cfg(test)]
#[path = "bench_tests.rs"]
mod tests;

/// The baseline, from the repository root.
///
/// Written only by `bless` ([`format_baseline`]): a header of `#` lines, then one line per
/// GPU, by name, its name, a tab, then its time in nanoseconds; read only exactly so
/// ([`parse_baseline`]).
pub const BASELINE: &str = "spec/gpu-time-baseline.txt";

/// The size whose time is checked: the largest of the ladder.
///
/// There the image's pixels dominate what a submission costs once (measured 2026-11-16 on an Apple M2 Pro: the
/// median of [`REPEATS`] varied by 2.3% from run to run at 1280 × 720, by 40% at 320 × 180).
pub const CHECKED: (u32, u32) = (1280, 720);

/// The sizes only shown, smallest first.
///
/// A size only shown may come back unmeasured: under
/// load, Metal writes no timestamp for most passes of a few microseconds (64x36: about 60%
/// of them at a load average of 19 to 33, measured 2026-11-20 on an Apple M2 Pro), while
/// every pass of the larger sizes is timed; other GPUs are unmeasured (roadmap #35 step 12).
pub const SHOWN: [(u32, u32); 3] = [(64, 36), (320, 180), (640, 360)];

/// The renders before measuring, so that the GPU's clocks have risen.
pub const WARM_UP: usize = 15;

/// The renders measured; their median is the time.
pub const REPEATS: usize = 15;

/// The most renders tried after the warm-up to measure [`REPEATS`] of them.
///
/// Three times as many, where Metal leaves about one render in twelve unmeasured (measured
/// 2026-11-19 on an Apple M2 Pro), so that a clock measuring fewer is one that does not
/// work, never one unlucky run.
pub const TRIES: usize = 3 * REPEATS;

/// What the GPU's clock measured of one size: a median, or why there is none, never a time
/// of 0 standing for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Timing {
    /// The median nanoseconds of [`REPEATS`] measured renders.
    Measured {
        /// The median, in nanoseconds.
        nanoseconds: u64,
        /// The renders it took to measure [`REPEATS`] of them, at most [`TRIES`].
        tries: usize,
    },
    /// Fewer than [`REPEATS`] of [`TRIES`] renders measured (the GPU wrote no timestamps of
    /// the others).
    Unmeasured {
        /// How many were.
        measured: usize,
    },
}

impl fmt::Display for Timing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Measured { nanoseconds, tries } => {
                write!(f, "{nanoseconds} ns ({REPEATS} of {tries} measured)")
            }
            Self::Unmeasured { measured } => {
                write!(f, "unmeasured ({measured} of {TRIES} measured)")
            }
        }
    }
}

/// How far the checked time's medians spread from run to run on the machine holding the
/// baseline, in one condition: what a bound of [`check`](run) is derived from, never a
/// guess.
///
/// The statistic, exactly: an odd number `n` of runs of `visual-gpu check` in that
/// condition, each giving its median `t` of [`REPEATS`] renders at [`CHECKED`]; `M` the
/// median of the `n` medians; then `below_tenths = ⌈1000 × (M − min t) / M⌉` and
/// `above_tenths = ⌈1000 × (max t − M) / M⌉`, each in tenths of a percent of `M`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Spread {
    /// When it was measured.
    pub date: &'static str,
    /// The condition it was measured in.
    pub condition: &'static str,
    /// The largest deviation of a run's median below the median of the runs, in tenths of
    /// a percent of it, rounded up.
    pub below_tenths: u64,
    /// The largest deviation above it, likewise.
    pub above_tenths: u64,
}

/// The spread measured idle: 2.3% either way (2026-11-16, an Apple M2 Pro on AC power,
/// [`CHECKED`]).
///
/// That figure predates the definition of [`Spread`] and stands for it until the
/// measurement session of roadmap #34 step 6 measures it again by that definition.
pub const IDLE: Spread = Spread {
    date: "2026-11-16",
    condition: "idle",
    below_tenths: 23,
    above_tenths: 23,
};

/// The spread measured under the full check's load, the condition it runs in.
///
/// Every pre-push check runs this benchmark under that load. `None` until measured (roadmap
/// #34 step 6, the measurement session): the idle spread stands for it until then.
pub const LOADED: Option<Spread> = None;

/// The spread the upper bound is derived from: [`LOADED`], the idle one until it is
/// measured.
pub const UPPER_SPREAD: Spread = match LOADED {
    Some(loaded) => loaded,
    None => IDLE,
};

/// The safety factor a bound keeps over the worst ratio of the two medians it compares.
pub const SAFETY: u64 = 2;

/// The margin over a measured spread a bound allows: four times, two for the two medians
/// the check compares, each spreading by up to one spread, times [`SAFETY`].
pub const MARGIN: u64 = 4;

/// The bound, in whole percent, that [`MARGIN`] times a spread of `tenths` tenths of a
/// percent allows, rounded up.
///
/// The check compares two medians, the baseline's `b` (blessed) and the checked one `c`,
/// each within its own spread of the time `M` both measure: `c / b` then lies within
/// `[(1 − below_c) / (1 + above_b), (1 + above_c) / (1 − below_b)]`, about one spread of
/// each median, twice one spread, on either side. A bound of [`MARGIN`] times a spread
/// holds that worst ratio with a factor of [`SAFETY`] to spare, which a test checks on the
/// measured spreads themselves, for the regime the bless and the check are taken in: the
/// bless idle, the check idle for the lower bound and under load for the upper.
#[must_use]
pub const fn bound_percent(tenths: u64) -> u64 {
    MARGIN.saturating_mul(tenths).div_ceil(10)
}

/// How far below its baseline a checked time may be, in percent: from the idle spread, since
/// load only adds time, so that a pass that silently does less work still fails.
///
/// It holds only for a baseline blessed idle: one blessed under load reads an idle check as
/// faster than it is, beyond this bound.
pub const LOWER_PERCENT: u64 = bound_percent(IDLE.below_tenths);

/// How far above its baseline a checked time may be, in percent: from the spread under load,
/// the condition the check runs in ([`UPPER_SPREAD`]).
pub const UPPER_PERCENT: u64 = bound_percent(UPPER_SPREAD.above_tenths);

/// The condition a baseline is blessed in, the one its bounds hold for.
pub const BLESS_CONDITION: &str =
    "idle, on AC power, outside Low Power Mode, on the machine whose GPU it holds";

/// Where a usage error points: the command printing the usage, as run from the repository
/// root (its own workspace), the way `cargo xtask` runs it.
const USAGE_HINT: &str = "(see: cargo run --manifest-path crates/visual-gpu/Cargo.toml -- help)";

/// The bounds a check applies, and the spreads they are derived from, by date and condition.
fn bounds() -> String {
    format!(
        "-{LOWER_PERCENT}% to +{UPPER_PERCENT}% ({MARGIN} times the {} spread measured {} below, the {} spread measured {} above)",
        IDLE.condition, IDLE.date, UPPER_SPREAD.condition, UPPER_SPREAD.date
    )
}

/// Whether `checked` nanoseconds are at most `lower` percent below `base` and at most
/// `upper` percent above it.
const fn within(checked: u64, base: u64, lower: u64, upper: u64) -> bool {
    let bound = if checked < base { lower } else { upper };
    checked.abs_diff(base).saturating_mul(100) <= base.saturating_mul(bound)
}

/// The usage of the command, with the bounds it checks and the condition of a bless.
#[must_use]
pub fn usage() -> String {
    format!(
        "usage: visual-gpu <check|bless|help>
  check  measure this GPU's time and compare it with {BASELINE}, from {}, if it has a baseline
         (run on AC power, outside Low Power Mode)
  bless  measure this GPU's time and write it to {BASELINE}: only {BLESS_CONDITION}
  help   print this help",
        bounds()
    )
}

/// What the GPU took, by its own clock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Times {
    /// The GPU's name and API, the baseline's key.
    pub adapter: String,
    /// Rendering an image at [`CHECKED`].
    pub checked: Timing,
    /// Rendering at each of [`SHOWN`].
    pub shown: [Timing; 3],
    /// Encoding the rendered image at [`CHECKED`] for display: shown, not yet held to a
    /// baseline (its bounds come with the spread under load, roadmap #34 step 6).
    pub display: Timing,
}

/// Why a measurement failed: the GPU's error, or the library's.
pub type Failure = Box<dyn core::error::Error>;

/// The image rate of the square measured, numerator then denominator: the cost
/// benchmarks' (`bindings/visual-wasm/benchmarks/suite.ts`, `SQUARE`, which a test holds).
const SQUARE_RATE: (u32, u32) = (60, 1);

/// The side of the square measured, in 65 536ths of the height: the cost benchmarks'.
const SQUARE_SIDE: u32 = 39_322;

/// The cycle of the square measured, in seconds, numerator then denominator: the cost
/// benchmarks'.
const SQUARE_CYCLE: (u32, u32) = (1001, 1000);

/// The image measured: the cost benchmarks' (`IMAGE`, which a test holds).
const IMAGE: ImageIndex = ImageIndex(7);

/// The square measured: the cost benchmarks' ([`SQUARE_RATE`], [`SQUARE_SIDE`],
/// [`SQUARE_CYCLE`]).
fn measured_square() -> Result<Square, Failure> {
    let rate = ImageRate::new(SQUARE_RATE.0, SQUARE_RATE.1)?;
    let cycle = Seconds::new(SQUARE_CYCLE.0, SQUARE_CYCLE.1)?;
    Ok(Square::new(rate, SQUARE_SIDE, cycle)?)
}

/// What the GPU's clock measures of rendering [`IMAGE`] at `size` ([`median_of`]).
fn median(gpu: &GpuSquare, square: &Square, size: (u32, u32)) -> Result<Timing, Failure> {
    let mut image = Image::try_new(ImageSize::new(size.0, size.1)?)?;
    median_of(&mut || Ok(gpu.timed_render(square, IMAGE, &mut image)?))
}

/// The median of what `render` measures: run [`WARM_UP`] times unmeasured, then until
/// [`REPEATS`] runs are measured, at most [`TRIES`] times, so that the median is always of
/// [`REPEATS`], the middle one; [`Timing::Unmeasured`] when fewer are. A run the GPU wrote
/// no timestamp of is left out (`timed_render`), never counted as 0.
fn median_of(render: &mut dyn FnMut() -> Result<Option<f64>, Failure>) -> Result<Timing, Failure> {
    for _ in 0..WARM_UP {
        let _warming: Option<f64> = render()?;
    }
    let mut times = Vec::with_capacity(REPEATS);
    let mut tries = 0_usize;
    while times.len() < REPEATS && tries < TRIES {
        tries = tries.saturating_add(1);
        times.extend(render()?);
    }
    times.sort_by(f64::total_cmp);
    Ok(match times.get(REPEATS / 2) {
        Some(&median) if times.len() == REPEATS => Timing::Measured {
            nanoseconds: whole_nanoseconds(median),
            tries,
        },
        _ => Timing::Unmeasured {
            measured: times.len(),
        },
    })
}

/// `nanoseconds`, a time the GPU measured (never negative: ticks of a pass's end after its
/// start), rounded to whole nanoseconds by its decimal text (no `as` cast), at most
/// `u64::MAX`.
fn whole_nanoseconds(nanoseconds: f64) -> u64 {
    format!("{nanoseconds:.0}").parse().unwrap_or(u64::MAX)
}

/// Measures `gpu`'s times ([`Times`]): `None` at once on a GPU without a clock, nothing
/// rendered.
///
/// # Errors
///
/// Returns why the GPU could not render.
pub fn measure(gpu: &GpuSquare) -> Result<Option<Times>, Failure> {
    if gpu.period.is_none() {
        return Ok(None);
    }
    let square = measured_square()?;
    let checked = median(gpu, &square, CHECKED)?;
    let [small, medium, large] = SHOWN;
    let shown = [
        median(gpu, &square, small)?,
        median(gpu, &square, medium)?,
        median(gpu, &square, large)?,
    ];
    let display = encoding_median(gpu, &square, CHECKED)?;
    Ok(Some(Times {
        adapter: gpu.adapter().to_owned(),
        checked,
        shown,
        display,
    }))
}

/// What the GPU's clock measures of encoding [`IMAGE`] at `size` for display
/// ([`median_of`]).
fn encoding_median(gpu: &GpuSquare, square: &Square, size: (u32, u32)) -> Result<Timing, Failure> {
    let image_size = ImageSize::new(size.0, size.1)?;
    let mut image = Image::try_new(image_size)?;
    gpu.render(square, IMAGE, &mut image)?;
    let mut display = DisplayImage::try_new(image_size)?;
    median_of(&mut || Ok(gpu.timed_encode(&image, &mut display)?))
}

/// Reads a baseline ([`BASELINE`]): each GPU's checked time, by its name.
///
/// Read only exactly as `bless` writes it ([`format_baseline`]): a file `bless` did not
/// write is refused, never read as holding fewer times, since an emptied file would
/// otherwise check nothing and pass.
///
/// # Errors
///
/// Returns the first line, other than a comment, that is not a name, a tab and a whole
/// number, or whose name is empty, has a space around it or is written twice; then a file
/// holding no GPU's time (the header alone, or nothing); then any other text than `bless`
/// writes of the times read (a comment past the header, lines out of order, a number
/// written otherwise, other line endings, no final newline or a line past the end).
pub fn parse_baseline(text: &str) -> Result<BTreeMap<String, u64>, String> {
    let mut times = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        if line.starts_with('#') {
            continue;
        }
        let at = number.saturating_add(1);
        let (name, value) = line
            .split_once('\t')
            .ok_or_else(|| format!("line {at}: not a GPU's name, a tab and its time"))?;
        if name.is_empty() || name.trim() != name {
            return Err(format!(
                "line {at}: {name:?} is not a GPU's name: empty, or with a space around it"
            ));
        }
        let time = value
            .parse()
            .map_err(|_| format!("line {at}: {value:?} is not a whole number of nanoseconds"))?;
        if times.insert(name.to_owned(), time).is_some() {
            return Err(format!("line {at}: {name} written twice"));
        }
    }
    if times.is_empty() {
        return Err("no GPU's time".to_owned());
    }
    if format_baseline(&times) != text {
        return Err("not as `visual-gpu bless` writes it".to_owned());
    }
    Ok(times)
}

/// The baseline of `times`, as `bless` writes it: a comment, then one line per GPU, by name.
#[must_use]
pub fn format_baseline(times: &BTreeMap<String, u64>) -> String {
    let (width, height) = CHECKED;
    let mut text = format!(
        "# The GPU's time to render the square at {width}x{height}, in nanoseconds, by its own clock\n\
         # (spec/benchmarks.md, \"Visual library\"): one line per GPU, written by\n\
         # `visual-gpu bless`, only {BLESS_CONDITION}.\n",
    );
    for (name, time) in times {
        // Writing to a string cannot fail.
        let _: fmt::Result = writeln!(text, "{name}\t{time}");
    }
    text
}

/// What the command needs from the outside world.
pub trait BenchIo {
    /// Measures the GPU ([`measure`]).
    ///
    /// # Errors
    ///
    /// Returns why no GPU could be opened or render.
    fn measure(&mut self) -> Result<Option<Times>, Failure>;
    /// Reads the baseline, `None` when there is none.
    ///
    /// # Errors
    ///
    /// Returns why it exists but cannot be read.
    fn read_baseline(&mut self) -> io::Result<Option<String>>;
    /// Writes the baseline.
    ///
    /// # Errors
    ///
    /// Returns why it cannot be written.
    fn write_baseline(&mut self, text: &str) -> io::Result<()>;
}

/// A failure of the command: its exit code and its one line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BenchError {
    /// 1 when the work failed, 2 for a usage error.
    pub code: u8,
    /// What went wrong, one line, without its `error: ` prefix.
    pub line: String,
}

/// A failure of the work, on one line: a control character (a line break of wgpu's
/// multi-line errors, an escape) becomes a space, and every run of spaces one.
fn failed(line: &str) -> BenchError {
    let spaced: String = line
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    BenchError {
        code: 1,
        line: spaced.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

/// The baseline's times, and whether its file exists (none when it does not); a file not
/// as `bless` writes it is refused, with the command restoring it.
fn baseline(io: &mut dyn BenchIo) -> Result<Option<BTreeMap<String, u64>>, BenchError> {
    let text = io
        .read_baseline()
        .map_err(|error| failed(&format!("cannot read {BASELINE}: {error}")))?;
    text.map(|text| {
        parse_baseline(&text).map_err(|why| {
            failed(&format!(
                "{BASELINE}: {why}: not as `visual-gpu bless` writes it (restore it, from the repository root: git checkout -- {BASELINE})"
            ))
        })
    })
    .transpose()
}

/// Says each measured time on `out`, with how many renders its median rests on, a closed
/// stdout ignored.
fn report(out: &mut dyn Write, times: &Times) {
    let mut lines = String::new();
    for ((width, height), time) in SHOWN.into_iter().zip(times.shown) {
        let _: fmt::Result = writeln!(
            lines,
            "{}: render {width}x{height}: {time} (shown)",
            times.adapter
        );
    }
    let _: fmt::Result = writeln!(
        lines,
        "{}: display {}x{}: {} (shown)",
        times.adapter, CHECKED.0, CHECKED.1, times.display
    );
    let _: fmt::Result = writeln!(
        lines,
        "{}: render {}x{}: {}",
        times.adapter, CHECKED.0, CHECKED.1, times.checked
    );
    drop(out.write_all(lines.as_bytes()));
}

/// Runs the command `args` (the program name excluded), saying what it measured on `out`
/// (a closed stdout ignored, as every command-line tool of the repository does).
///
/// # Errors
///
/// Returns a usage error (code 2) for no command, another command or an argument, and a
/// failure (code 1) when no GPU renders, the baseline cannot be read or written or is not
/// as `bless` writes it ([`parse_baseline`]), `check` finds no baseline file, `bless` finds
/// no clock or a baseline that would not read back as written (a GPU's name with a tab, a
/// line break or a leading `#`, empty or with a space around it), a clock measures fewer than [`REPEATS`]
/// of [`TRIES`] renders at [`CHECKED`], or `check` finds the time more than
/// [`LOWER_PERCENT`] below its baseline or [`UPPER_PERCENT`] above it. A GPU the file holds no time of is shown, not
/// checked: the baseline is one machine's (bless locally, never commit another's).
pub fn run(args: &[String], io: &mut dyn BenchIo, out: &mut dyn Write) -> Result<(), BenchError> {
    let usage_error = |line: String| BenchError {
        code: 2,
        line: format!("{line} {USAGE_HINT}"),
    };
    let command = match args {
        [] => return Err(usage_error("no command given".to_owned())),
        [command] => command.as_str(),
        // Quoted, so that a line break or a control character in it stays on the one line.
        [command, ..] => return Err(usage_error(format!("{command:?} takes no argument"))),
    };
    if command == "help" {
        drop(writeln!(out, "{}", usage()));
        return Ok(());
    }
    if command != "check" && command != "bless" {
        return Err(usage_error(format!("unknown command {command:?}")));
    }
    let held = baseline(io)?;
    if command == "check" && held.is_none() {
        return Err(failed(&format!(
            "{BASELINE} is missing: no time can be checked (restore it, from the repository root: git checkout -- {BASELINE}; or bless it, only {BLESS_CONDITION}: cargo xtask bless-benchmarks)"
        )));
    }
    let mut times = held.unwrap_or_default();
    let measured = io
        .measure()
        .map_err(|error| failed(&format!("cannot measure the GPU: {error}")))?;
    let Some(measured) = measured else {
        drop(writeln!(
            out,
            "this GPU has no clock its programs can read: no time"
        ));
        return if command == "bless" {
            Err(failed("no time to bless: this GPU has no clock"))
        } else {
            Ok(())
        };
    };
    report(out, &measured);
    let checked = match measured.checked {
        Timing::Measured { nanoseconds, .. } => nanoseconds,
        Timing::Unmeasured { measured: count } => {
            return Err(failed(&format!(
                "{}: its clock measured {count} of {TRIES} renders at {}x{}, fewer than {REPEATS}: no time to {command} (run it again with the machine idle: {}; a clock failing idle is a defect to report)",
                measured.adapter,
                CHECKED.0,
                CHECKED.1,
                if command == "check" {
                    "cargo xtask check"
                } else {
                    "cargo xtask bless-benchmarks"
                }
            )));
        }
    };
    if command == "bless" {
        let _replaced = times.insert(measured.adapter.clone(), checked);
        let text = format_baseline(&times);
        // Written only if read back the same: a name the driver gives with a tab, a line
        // break or a leading `#` would read back as another line, or a comment, and an empty
        // one or one with a space around it is refused.
        if parse_baseline(&text).as_ref() != Ok(&times) {
            return Err(failed(&format!(
                "cannot bless {:?}: its baseline would not read back as written (a tab, a line break, a leading #, an empty name or a space around it)",
                measured.adapter
            )));
        }
        io.write_baseline(&text)
            .map_err(|error| failed(&format!("cannot write {BASELINE}: {error}")))?;
        drop(writeln!(out, "wrote {BASELINE}"));
        return Ok(());
    }
    let Some(&base) = times.get(&measured.adapter) else {
        // The GPUs it holds named: a driver update that renames this one shows here.
        let holds = if times.is_empty() {
            "none".to_owned()
        } else {
            times.keys().cloned().collect::<Vec<_>>().join(", ")
        };
        drop(writeln!(
            out,
            "{}: no baseline (the baseline holds {holds}): shown, not checked",
            measured.adapter
        ));
        return Ok(());
    };
    if !within(checked, base, LOWER_PERCENT, UPPER_PERCENT) {
        return Err(failed(&format!(
            "{}: {checked} ns, baseline {base} ns: beyond {} (on the blessing machine, on AC power, outside Low Power Mode: bless it, only {BLESS_CONDITION}, with the reason, if the work changed)",
            measured.adapter,
            bounds()
        )));
    }
    Ok(())
}

/// The real outside world of the command: the first GPU, and the baseline at `root`.
#[derive(Debug)]
#[non_exhaustive]
pub struct System {
    /// The repository root, where the baseline is.
    pub root: std::path::PathBuf,
}

impl System {
    /// The outside world of a command run from `root`, the repository root.
    #[must_use]
    pub const fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}

impl BenchIo for System {
    fn measure(&mut self) -> Result<Option<Times>, Failure> {
        measure(&GpuSquare::new(wgpu::Backends::all())?)
    }

    fn read_baseline(&mut self) -> io::Result<Option<String>> {
        match std::fs::read_to_string(self.root.join(BASELINE)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write_baseline(&mut self, text: &str) -> io::Result<()> {
        std::fs::write(self.root.join(BASELINE), text)
    }
}
