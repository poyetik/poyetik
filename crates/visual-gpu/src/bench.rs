//! The GPU's time benchmark (`spec/benchmarks.md`, "Visual library"): the time the GPU takes
//! to render the square, by its own clock, held to a baseline per named GPU.
//!
//! A GPU's time depends on the GPU, its system and driver, and the machine's state: it is
//! checked only on a GPU whose baseline the repository holds (`spec/gpu-time-baseline.txt`:
//! the maintainer's Mac, blessed and checked on AC power, outside Low Power Mode), on the
//! system and driver the baseline records, a change of either failing the check with the
//! procedure measuring its bound again; on any other GPU (the software GPU of CI) nothing is
//! rendered and its time is not checked. The baseline is keyed by the GPU's name: another
//! machine whose GPU has the same name is checked against it. A GPU runner of CI, and a
//! baseline keyed by the machine, are roadmap #35 step 12's. What does not depend on the GPU,
//! the transfers and work of an image ([`render_cost`](crate::render_cost)), is held exactly
//! by the tests on every machine.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Write as _;
use core::num::NonZeroU64;
use std::io::{self, Write};

use visual_boxes::Square;
use visual_core::{DisplayImage, Image, ImageIndex, ImageRate, ImageSize, Seconds};

use crate::safe::{needs_escape, safe_message};
use crate::{GpuSquare, exact_f64};

#[cfg(test)]
#[path = "bench_tests.rs"]
mod tests;

/// The baseline, from the repository root.
///
/// Written only by `bless` ([`format_baseline`]): a header of `#` lines, then one line per
/// GPU, by name: its name, its time in nanoseconds, the system and the GPU's driver it was
/// blessed on, separated by tabs; read only exactly so ([`parse_baseline`]).
pub const BASELINE: &str = "spec/gpu-time-baseline.txt";

/// The size whose time is checked: the largest of the ladder.
///
/// There the image's pixels dominate what a submission costs once (measured 2026-11-16 on an
/// Apple M2 Pro: the median of [`REPEATS`] varied by 40% from run to run at 320 × 180); how
/// far it varies at 1280 × 720 is in [`RUNS`].
pub const CHECKED: (u32, u32) = (1280, 720);

/// The sizes only shown, smallest first.
///
/// A size only shown may come back unmeasured, idle as under load: Metal writes no
/// timestamp for many passes of a few microseconds ([`RUNS`] holds how many of each run's
/// renders were measured, on an Apple M2 Pro; `spec/benchmarks.md` states them); other GPUs
/// are unmeasured (roadmap #35 step 12).
pub const SHOWN: [(u32, u32); 3] = [(64, 36), (320, 180), (640, 360)];

/// The renders before measuring, so that the GPU's clocks have risen.
pub const WARM_UP: usize = 15;

/// The renders measured; their median is the time.
pub const REPEATS: usize = 15;

/// The most renders tried after the warm-up to measure [`REPEATS`] of them.
///
/// Three times as many: rendering at [`CHECKED`], every run recorded ([`RUNS`]) measured its
/// first [`REPEATS`], so that a clock measuring fewer there does not work, never one unlucky
/// run; a size only shown may run out.
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

/// The runs whose median a bless writes: their middle one, so that two runs of five far from
/// the others never set the baseline, where a check compares one run.
pub const BLESS_RUNS: usize = 5;

/// Every run recorded, from the repository root, read by the tests alone.
///
/// Each run with its session, time, load, power and condition (the idle session's, those of
/// another session that were not idle and why, those under a full check's load), as they were
/// measured: the law tests recompute from it [`IDLE`], the bound ([`bound_percent`]) and the
/// figures `states_the_runs_figures_as_the_record_gives_them` lists.
pub const RUNS: &str = "spec/gpu-time-runs.txt";

/// Runs of `visual-gpu check` in one condition: what the bound is derived from, never a guess.
///
/// Measured on the machine, system and GPU driver the baseline holds; the machine, the
/// system, and what to do when either changes, are written once, in `spec/benchmarks.md`
/// ("Visual library").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Runs {
    /// When they were measured.
    pub date: &'static str,
    /// The condition they were measured in.
    pub condition: &'static str,
    /// How many runs, each giving its median of [`REPEATS`] renders at [`CHECKED`].
    pub count: u64,
    /// The lowest median of a run, in nanoseconds.
    pub lowest: u64,
    /// The highest median of a run, in nanoseconds.
    pub highest: u64,
    /// The lowest and the highest median of [`BLESS_RUNS`] of the runs, what a bless could
    /// have written, in nanoseconds.
    pub blessed: (u64, u64),
}

/// The runs of the dedicated idle session ([`RUNS`], "idle"), the bound's source.
///
/// On AC power, outside Low Power Mode:
/// 51 idle runs (one session, 88 125 to 94 708 ns; medians of 5 of them from 90 125 to 94 542 ns).
pub const IDLE: Runs = Runs {
    date: "2026-11-20",
    condition: "idle",
    count: 51,
    lowest: 88_125,
    highest: 94_708,
    blessed: (90_125, 94_542),
};

/// The safety factor of the bound over the largest deviation measured, numerator then
/// denominator: 1.5.
pub const SAFETY: (u64, u64) = (3, 2);

/// [`SAFETY`] times the `deviation` of a time from `base`, both in nanoseconds, in whole
/// percent of `base`, rounded up: 0 for a base of 0, which no bound can be derived from and
/// within which only a time equal to its baseline is.
const fn safe_percent(deviation: u64, base: u64) -> u64 {
    match NonZeroU64::new(base.saturating_mul(SAFETY.1)) {
        Some(divisor) => deviation
            .saturating_mul(100)
            .saturating_mul(SAFETY.0)
            .div_ceil(divisor.get()),
        None => 0,
    }
}

/// The bound, in whole percent either way, derived from `runs`.
///
/// It is [`SAFETY`] times the largest deviation a check could have seen among them, one run
/// compared with the median of any [`BLESS_RUNS`] of them, below or above, rounded up. The largest below is the lowest run's from the highest median of [`BLESS_RUNS`], the
/// largest above the highest run's from the lowest such median: each in percent of that
/// median, as the check compares.
#[must_use]
pub const fn bound_percent(runs: Runs) -> u64 {
    let below = safe_percent(runs.blessed.1.saturating_sub(runs.lowest), runs.blessed.1);
    let above = safe_percent(runs.highest.saturating_sub(runs.blessed.0), runs.blessed.0);
    // The larger of the two, with no comparison to choose on a tie.
    below.saturating_add(above.saturating_sub(below))
}

/// How far below or above its baseline a checked time may be, in percent: derived from the
/// idle runs ([`bound_percent`] of [`IDLE`]):
/// 1.5 × 6.79% = 10.18%, rounded up to 11%.
pub const BOUND_PERCENT: u64 = bound_percent(IDLE);

/// The power a GPU's time is measured on, by a check or a bless: written once, so that
/// [`POWER_CONDITION`] and [`BLESS_CONDITION`] cannot differ.
macro_rules! power_condition {
    () => {
        "on AC power, outside Low Power Mode"
    };
}

/// The power a check measures on.
pub const POWER_CONDITION: &str = power_condition!();

/// The condition a baseline is blessed in: [`POWER_CONDITION`], on the machine whose GPU it
/// holds.
pub const BLESS_CONDITION: &str =
    concat!(power_condition!(), ", on the machine whose GPU it holds");

/// Where the bound, the machine and system it was measured on, and the procedure measuring
/// it again are written.
const PROCEDURE: &str = "spec/benchmarks.md, \"Visual library\"";

/// Where a usage error points: the command printing the usage, this crate (a workspace of its
/// own) named from the repository's root, so that it runs from any directory of it.
const USAGE_HINT: &str = "(see: cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- help)";

/// The bound a check applies, and the runs it is derived from.
fn bounds() -> String {
    format!(
        "±{BOUND_PERCENT}% ({}/{} times the largest deviation of one of {} {} runs from the median of any {BLESS_RUNS} of them, measured {}; {PROCEDURE})",
        SAFETY.0, SAFETY.1, IDLE.count, IDLE.condition, IDLE.date
    )
}

/// Whether `checked` nanoseconds are at most `percent` percent below or above `base`.
///
/// Each side measured apart, one of them 0, so that no comparison decides which side
/// applies: a time equal to its baseline is within either, and such a comparison's mutant
/// (`<` for `<=`) no test could tell apart. The products are in 128 bits, exact for every
/// `u64` time and percent, so that none saturates into passing.
fn within(checked: u64, base: u64, percent: u64) -> bool {
    let faster = u128::from(base.saturating_sub(checked));
    let slower = u128::from(checked.saturating_sub(base));
    // Below 2⁶⁴ × 2⁶⁴: never saturates.
    let allowed = u128::from(base).saturating_mul(u128::from(percent));
    faster.saturating_mul(100) <= allowed && slower.saturating_mul(100) <= allowed
}

/// The usage of the command, with the bound it checks and the condition of a bless.
#[must_use]
pub fn usage() -> String {
    format!(
        "usage: visual-gpu <check|bless|help>
  check  measure this GPU's time and compare it with {BASELINE}, within {}, if it holds
         this GPU, on the system and driver it records (run {POWER_CONDITION})
  bless  measure this GPU's time {BLESS_RUNS} times and write the median of those runs to {BASELINE}, with the system and driver: only
         {BLESS_CONDITION}, with its runs measured again on a change ({PROCEDURE})
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
    /// Encoding the rendered image at [`CHECKED`] for display: shown, never held to a
    /// baseline, since its time is not stable from run to run (`spec/benchmarks.md`,
    /// "Visual library").
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
/// no timestamp of (`timed_render`), or whose time is no whole nanoseconds above 0
/// ([`whole_nanoseconds`]), is left out, never counted as 0.
fn median_of(render: &mut dyn FnMut() -> Result<Option<f64>, Failure>) -> Result<Timing, Failure> {
    for _ in 0..WARM_UP {
        let _warming: Option<f64> = render()?;
    }
    let mut times = Vec::with_capacity(REPEATS);
    let mut tries = 0_usize;
    while times.len() < REPEATS && tries < TRIES {
        tries = tries.saturating_add(1);
        times.extend(render()?.and_then(whole_nanoseconds));
    }
    times.sort_unstable();
    Ok(match times.get(REPEATS / 2) {
        Some(&nanoseconds) if times.len() == REPEATS => Timing::Measured { nanoseconds, tries },
        _ => Timing::Unmeasured {
            measured: times.len(),
        },
    })
}

/// `nanoseconds`, a time the GPU measured, rounded to whole nanoseconds by its decimal text
/// (no `as` cast; to the nearest, ties to even, as Rust's `{:.0}` rounds the exact value):
/// `None` when that is 0 or no number a `u64` holds (not a number, infinite,
/// negative, or beyond: no float rounds to `u64::MAX` itself), so that no time stands for
/// none and none passes every check.
fn whole_nanoseconds(nanoseconds: f64) -> Option<u64> {
    format!("{nanoseconds:.0}")
        .parse()
        .ok()
        .filter(|&whole| whole > 0)
}

/// What a GPU without a clock is: one without timestamp queries, or whose period no pass can
/// be timed with (`clock_period`).
const NO_CLOCK: &str = "this GPU has no clock its programs can read";

/// Measures `gpu`'s times ([`Times`]).
///
/// # Errors
///
/// Returns, at once and with nothing rendered, that a GPU has no clock its programs can
/// read; then why the GPU could not render, or an image or a display image the
/// machine cannot hold (the library's error, [`visual_core::VisualError::OutOfMemory`]).
pub fn measure(gpu: &GpuSquare) -> Result<Times, Failure> {
    if gpu.period.is_none() {
        return Err(NO_CLOCK.into());
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
    Ok(Times {
        adapter: gpu.adapter().to_owned(),
        checked,
        shown,
        display,
    })
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

/// What the baseline holds of one GPU: its checked time, its system and its driver.
///
/// The system and the driver are what the time and its bound were measured on beside the
/// GPU, so that a change of either fails the check rather than compare against a bound
/// measured elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Held {
    /// The checked time, in nanoseconds: the median of [`BLESS_RUNS`] runs.
    pub time: u64,
    /// The system it was blessed on, its name and version ([`BenchIo::system`]).
    pub system: String,
    /// The GPU's driver it was blessed with, as wgpu names it ([`GpuSquare::driver`]).
    pub driver: String,
}

impl Held {
    /// What the baseline holds of a GPU blessed at `time` nanoseconds on `system`, with
    /// `driver`.
    #[must_use]
    pub const fn new(time: u64, system: String, driver: String) -> Self {
        Self {
            time,
            system,
            driver,
        }
    }
}

/// The text fields of a baseline's line, after the GPU's name and its time, by what they
/// hold.
const TEXT_FIELDS: [&str; 3] = ["name", "system", "driver"];

/// Why a baseline holding `fields` (a GPU's [`TEXT_FIELDS`]) would not read back as written,
/// if so: the first of them that is empty, has a space around it, holds a control (a tab, a
/// line break, an escape), bidirectional formatting or line separator character, or is a name
/// starting with `#`, which would read as a comment.
fn unreadable(fields: [&str; 3]) -> Option<String> {
    fields
        .into_iter()
        .zip(TEXT_FIELDS)
        .find(|&(field, what)| {
            field.is_empty()
                || field.trim() != field
                || field.contains(needs_escape)
                || (what == "name" && field.starts_with('#'))
        })
        .map(|(field, what)| {
            format!(
                "{field:?} is not a GPU's {what}: empty, with a space around it, with a control, bidirectional formatting or line separator character, or, for a name, starting with #"
            )
        })
}

/// Reads a baseline ([`BASELINE`]): each GPU's checked time, system and driver, by its name.
///
/// Read only exactly as `bless` writes it ([`format_baseline`]): a file `bless` did not
/// write is refused, never read as holding fewer times, since an emptied file would
/// otherwise check nothing and pass.
///
/// # Errors
///
/// Returns the first line, other than a comment, that is not four fields separated by tabs
/// (a name, a whole number above 0 that a measured time rounds to (`whole_nanoseconds`), a
/// system, a driver), whose name, system or driver is
/// empty, has a space around it or holds a control, bidirectional formatting or line separator
/// character (an escape, a line break), or whose name is written twice; then a file holding no GPU's time
/// (the header alone, or nothing); then any other text than `bless` writes of the times read
/// (a comment past the header, lines out of order, a number written otherwise, other line
/// endings, no final newline or a line past the end), naming the first line that differs
/// and what `bless` writes there.
pub fn parse_baseline(text: &str) -> Result<BTreeMap<String, Held>, String> {
    let mut held = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        if line.starts_with('#') {
            continue;
        }
        let at = number.saturating_add(1);
        let mut fields = line.split('\t');
        let (Some(name), Some(value), Some(system), Some(driver), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(format!(
                "line {at}: not a GPU's name, its time, its system and its driver, separated by tabs"
            ));
        };
        if let Some(why) = unreadable([name, system, driver]) {
            return Err(format!("line {at}: {why}"));
        }
        let time = value
            .parse()
            .ok()
            .filter(|&time| whole_nanoseconds(exact_f64(time)) == Some(time))
            .ok_or_else(|| {
                format!(
                    "line {at}: {value:?} is not a whole number of nanoseconds above 0 a measured time rounds to"
                )
            })?;
        let entry = Held {
            time,
            system: system.to_owned(),
            driver: driver.to_owned(),
        };
        if held.insert(name.to_owned(), entry).is_some() {
            return Err(format!("line {at}: {name} written twice"));
        }
    }
    if held.is_empty() {
        return Err("no GPU's time".to_owned());
    }
    let written = format_baseline(&held);
    if written != text {
        return Err(difference(text, &written));
    }
    Ok(held)
}

/// Where `text` first differs from `written`, the text `bless` writes of its times: the
/// line's number, then both lines with their endings, quoted (`""` past the end of either).
fn difference(text: &str, written: &str) -> String {
    let found: Vec<&str> = text.split_inclusive('\n').collect();
    let expected: Vec<&str> = written.split_inclusive('\n').collect();
    let same = found
        .iter()
        .zip(&expected)
        .take_while(|(line, other)| line == other)
        .count();
    format!(
        "line {}: {:?}, not {:?}",
        same.saturating_add(1),
        found.get(same).copied().unwrap_or_default(),
        expected.get(same).copied().unwrap_or_default()
    )
}

/// The baseline of `held`, as `bless` writes it: a comment, then one line per GPU, by name.
#[must_use]
pub fn format_baseline(held: &BTreeMap<String, Held>) -> String {
    let (width, height) = CHECKED;
    let mut text = format!(
        "# The GPU's time to render the square at {width}x{height}, in nanoseconds, by its own clock\n\
         # ({PROCEDURE}): one line per GPU, its name, its time, the system and the GPU's driver\n\
         # it was blessed on, separated by tabs; written by `visual-gpu bless`, only\n\
         # {BLESS_CONDITION}.\n",
    );
    for (name, entry) in held {
        // Writing to a string cannot fail.
        let _: fmt::Result = writeln!(
            text,
            "{name}\t{}\t{}\t{}",
            entry.time, entry.system, entry.driver
        );
    }
    text
}

/// The GPU the command opened: its name, the baseline's key, its driver, and whether it
/// has a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Gpu {
    /// Its name and API ([`GpuSquare::adapter`]).
    pub adapter: String,
    /// Its driver ([`GpuSquare::driver`]).
    pub driver: String,
    /// Whether it has a clock its programs can read (timestamp queries, of a period a pass
    /// can be timed with), without which no time is measured.
    pub clock: bool,
}

/// The seal of [`BenchIo`]: a trait no other crate can name, so none can implement it.
mod sealed {
    /// Implemented only by this crate's outside worlds: [`System`](super::System) and its
    /// tests' fake.
    pub trait Sealed {}
}

/// What the command needs from the outside world.
///
/// Sealed: only this crate implements it ([`System`], and its tests' fake), since the
/// [`Gpu`] and [`Times`] it returns are built only here.
pub trait BenchIo: sealed::Sealed {
    /// Opens the first GPU, rendering nothing, and names it.
    ///
    /// # Errors
    ///
    /// Returns why no GPU could be opened.
    fn open(&mut self) -> Result<Gpu, Failure>;
    /// The system's name and version, as the baseline records it (`macOS 26.5.2`).
    ///
    /// # Errors
    ///
    /// Returns why it cannot be read.
    fn system(&mut self) -> io::Result<String>;
    /// Measures the GPU ([`measure`]), the one opened if any, then closes it: each run
    /// measures a GPU freshly opened, as each run of `check` does.
    ///
    /// # Errors
    ///
    /// Returns why no GPU could be opened or render, or that it has no clock.
    fn measure(&mut self) -> Result<Times, Failure>;
    /// Reads the baseline, `None` when there is none.
    ///
    /// # Errors
    ///
    /// Returns why it exists but cannot be read, or that there is no `spec/` to hold it.
    fn read_baseline(&mut self) -> io::Result<Option<String>>;
    /// Whether the baseline can be written, changing nothing: asked before the measurements.
    /// Its directory's permission to create a file in it is not asked: the write says it.
    ///
    /// # Errors
    ///
    /// Returns why its file cannot be opened for writing, or that there is no `spec/` to hold
    /// it.
    fn writable(&mut self) -> io::Result<()>;
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

/// `text` on one line: every run of white space (a line break of wgpu's multi-line errors)
/// becomes one space, then the shared rule of untrusted text applies ([`safe_message`]: an
/// escape or a bidirectional control of a driver's name is shown escaped, the line bounded).
/// Every line the command says goes through it or, its names quoted, through the shared rule
/// alone, so that no driver's text drives the terminal.
fn one_line(text: &str) -> String {
    safe_message(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A failure of the work, on one line ([`one_line`]).
fn failed(line: &str) -> BenchError {
    BenchError {
        code: 1,
        line: one_line(line),
    }
}

/// A failure of the work, `reason` made one bounded line ([`one_line`], [`safe_message`]),
/// then its `remedy` whole: no reason, however long, cuts the command that helps.
fn remedied(reason: &str, remedy: &str) -> BenchError {
    BenchError {
        code: 1,
        line: format!("{reason}{remedy}"),
    }
}

/// What writing on stdout gave: a closed stdout is no failure, as in every command-line tool
/// of the repository; any other is one.
fn written(result: io::Result<()>) -> Result<(), BenchError> {
    match result {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(failed(&format!("cannot write to stdout: {error}")))
        }
        _ => Ok(()),
    }
}

/// Says `line` on `out`, on one line ([`one_line`]), a closed stdout ignored ([`written`]).
fn say(out: &mut dyn Write, line: &str) -> Result<(), BenchError> {
    written(writeln!(out, "{}", one_line(line)))
}

/// The baseline's times, as `command` reads them, and whether its file exists (none when it
/// does not); a file not as `bless` writes it is refused, with the command restoring it.
fn baseline(
    io: &mut dyn BenchIo,
    command: Measuring,
) -> Result<Option<BTreeMap<String, Held>>, BenchError> {
    let text = io.read_baseline().map_err(|error| {
        remedied(
            &one_line(&format!("cannot read {BASELINE}: {error}")),
            &read_remedy(error.kind(), command),
        )
    })?;
    text.map(|text| {
        parse_baseline(&text).map_err(|why| {
            remedied(
                &one_line(&format!("{BASELINE}: {why}")),
                &not_as_bless_writes(),
            )
        })
    })
    .transpose()
}

/// A baseline that cannot be written, asked before measuring or found at the write.
fn unwritable(error: &io::Error) -> BenchError {
    failed(&format!("cannot write {BASELINE}: {error}"))
}

/// The restore of the baseline, as a runnable hint: from the last commit, by its path from
/// the repository's root, so that it runs from any directory of it.
fn restore() -> String {
    format!("(see: git checkout HEAD -- :/{BASELINE})")
}

/// What a baseline that is not as bless writes it asks: restored, by `check` and by `bless`
/// alike, which keeps the other GPUs' times and so reads them.
fn not_as_bless_writes() -> String {
    format!(": not as bless writes it: restore it {}", restore())
}

/// What a baseline that cannot be read asks of `command`, by the kind of its error (`: ` and
/// its remedy, or nothing): one that is not text restored, a directory in its place moved away
/// or a permission made readable, then the command run again (never a restore beside a
/// directory: git would delete it with the untracked files it holds); any other failure, a
/// disk's, which no command of the repository fixes, the error alone. A missing one is no
/// error ([`BenchIo::read_baseline`]).
fn read_remedy(kind: io::ErrorKind, command: Measuring) -> String {
    let again = format!("then {} again (see: {})", command.name(), command.again());
    if kind == io::ErrorKind::InvalidData {
        not_as_bless_writes()
    } else if kind == io::ErrorKind::IsADirectory {
        format!(": a directory stands where the file should be: move it away, {again}")
    } else if kind == io::ErrorKind::PermissionDenied {
        format!(": make it readable, {again}")
    } else {
        String::new()
    }
}

/// The command checking the GPU's time alone, with nothing running beside it, this crate named
/// from the repository's root, where it is run: elsewhere it says so
/// ([`BenchIo::read_baseline`]).
const CHECK: &str = "cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- check";

/// Says each measured time on `out`, with how many renders its median rests on ([`say`]).
fn report(out: &mut dyn Write, times: &Times) -> Result<(), BenchError> {
    let adapter = &times.adapter;
    for ((width, height), time) in SHOWN.into_iter().zip(times.shown) {
        say(
            out,
            &format!("{adapter}: render {width}x{height}: {time} (shown)"),
        )?;
    }
    let (width, height) = CHECKED;
    say(
        out,
        &format!(
            "{adapter}: display {width}x{height}: {} (shown)",
            times.display
        ),
    )?;
    say(
        out,
        &format!("{adapter}: render {width}x{height}: {}", times.checked),
    )
}

/// The GPU `io` opens, rendering nothing.
fn open(io: &mut dyn BenchIo) -> Result<Gpu, BenchError> {
    io.open()
        .map_err(|error| failed(&format!("cannot open the GPU: {error}")))
}

/// The system's name and version, read through `io`.
fn system(io: &mut dyn BenchIo) -> Result<String, BenchError> {
    io.system()
        .map_err(|error| failed(&format!("cannot read the system's version: {error}")))
}

/// What a change of `what` from `blessed` to `now` tells: the bound was measured on the old
/// one, and how to measure it again; both quoted and escaped, their spaces kept, so that two
/// that differ never read the same.
fn changed(adapter: &str, what: &str, blessed: &str, now: &str) -> BenchError {
    remedied(
        &safe_message(&format!(
            "{}: the {what} changed, {blessed:?} → {now:?}: the baseline and its bound were measured on {blessed:?}, so this time is not checked against them",
            one_line(adapter)
        )),
        &format!(
            " (measure the runs again ({PROCEDURE}), then bless (see: cargo xtask bless-benchmarks))"
        ),
    )
}

/// The command measuring the GPU, parsed once from the arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Measuring {
    /// `check`: compare the time with the baseline.
    Check,
    /// `bless`: write the time to the baseline.
    Bless,
}

impl Measuring {
    /// The command's name, as given.
    const fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Bless => "bless",
        }
    }

    /// The command of the repository running it again, as a hint names it.
    const fn again(self) -> &'static str {
        match self {
            Self::Check => CHECK,
            Self::Bless => "cargo xtask bless-benchmarks",
        }
    }
}

/// Measures the GPU `adapter` through `io` and says its times on `out`: the checked time.
///
/// # Errors
///
/// Returns why the GPU could not be measured, that the GPU measured is another than `adapter`
/// (the one opened), or a clock measuring too few renders at [`CHECKED`] to `command`.
fn checked_time(
    io: &mut dyn BenchIo,
    out: &mut dyn Write,
    command: Measuring,
    adapter: &str,
) -> Result<u64, BenchError> {
    let measured = io
        .measure()
        .map_err(|error| failed(&format!("cannot measure the GPU: {error}")))?;
    report(out, &measured)?;
    if measured.adapter != adapter {
        return Err(remedied(
            &safe_message(&format!(
                "the GPU measured is {:?}, not {adapter:?}, the one opened: no time to {}",
                measured.adapter,
                command.name()
            )),
            &format!(" (run it again (see: {}))", command.again()),
        ));
    }
    match measured.checked {
        Timing::Measured { nanoseconds, .. } => Ok(nanoseconds),
        Timing::Unmeasured { measured: count } => Err(remedied(
            &one_line(&format!(
                "{}: its clock measured {count} of {TRIES} renders at {}x{}, fewer than {REPEATS}: no time to {}",
                measured.adapter,
                CHECKED.0,
                CHECKED.1,
                command.name()
            )),
            &format!(
                " (run it again with no other heavy work running (see: {}); a clock failing so is a defect to report)",
                command.again()
            ),
        )),
    }
}

/// Runs the command `args` (the program name excluded), saying what it measured on `out`
/// (a closed stdout ignored, as every command-line tool of the repository does; any other
/// failure to write there is one).
///
/// `check` measures only a GPU the baseline holds: on any other (CI's software GPU, another
/// machine), it renders nothing and says the GPU has no baseline, so that its time is not
/// checked.
///
/// # Errors
///
/// Returns a usage error (code 2) for no command, another command or an argument, and a
/// failure (code 1) when no GPU opens or renders, the baseline cannot be read or written (`bless` asks before it opens a GPU) or
/// is not as `bless` writes it ([`parse_baseline`]), `check` finds no baseline file, the
/// system's version cannot be read, the GPU has no clock (`check` on a GPU the baseline
/// holds, before rendering; `bless` on any), `bless` finds a baseline that would not read
/// back as written, before measuring (a name, system or driver with a control, bidirectional formatting
/// or line separator character or a leading `#`, empty or with a space around it), the GPU measured is another than the one opened, a clock measures fewer than [`REPEATS`] of [`TRIES`]
/// renders at [`CHECKED`], or `check` finds the system or the driver changed since the bless,
/// or the time more than [`BOUND_PERCENT`] below or above its baseline.
pub fn run(args: &[String], io: &mut dyn BenchIo, out: &mut dyn Write) -> Result<(), BenchError> {
    let usage_error = |line: String| BenchError {
        code: 2,
        line: format!("{} {USAGE_HINT}", safe_message(&line)),
    };
    let command = match args {
        [] => return Err(usage_error("no command given".to_owned())),
        [command] => command.as_str(),
        // Quoted, so that a line break or a control character in it shows as written.
        [command, ..] => return Err(usage_error(format!("{command:?} takes no argument"))),
    };
    let measuring = match command {
        "help" => {
            return written(writeln!(out, "{}", usage()));
        }
        "check" => Measuring::Check,
        "bless" => Measuring::Bless,
        _ => return Err(usage_error(format!("unknown command {command:?}"))),
    };
    let held = baseline(io, measuring)?;
    match measuring {
        Measuring::Check => check(io, out, held),
        Measuring::Bless => {
            io.writable().map_err(|error| unwritable(&error))?;
            let gpu = open(io)?;
            bless(io, out, held.unwrap_or_default(), gpu)
        }
    }
}

/// `check`: measures the GPU, if `held` holds it, and compares its time with its baseline.
///
/// # Errors
///
/// As [`run`]'s `check`.
fn check(
    io: &mut dyn BenchIo,
    out: &mut dyn Write,
    held: Option<BTreeMap<String, Held>>,
) -> Result<(), BenchError> {
    let Some(held) = held else {
        return Err(failed(&format!(
            "{BASELINE} is missing: no time can be checked: restore it {}, or bless it, only {BLESS_CONDITION} (see: cargo xtask bless-benchmarks)",
            restore()
        )));
    };
    let gpu = open(io)?;
    let Some(blessed) = held.get(&gpu.adapter) else {
        // The GPUs it holds named, at least one (`parse_baseline`): a driver update that
        // renames this one shows here. Each quoted, its spaces kept, so that two that differ
        // never read the same.
        let holds: Vec<_> = held.keys().map(|name| format!("{name:?}")).collect();
        let line = format!(
            "{:?}: no baseline (the baseline holds {}): its time is not checked, nothing rendered",
            gpu.adapter,
            holds.join(", ")
        );
        return written(writeln!(out, "{}", safe_message(&line)));
    };
    let now = system(io)?;
    if blessed.system != now {
        return Err(changed(&gpu.adapter, "system", &blessed.system, &now));
    }
    if blessed.driver != gpu.driver {
        return Err(changed(
            &gpu.adapter,
            "driver",
            &blessed.driver,
            &gpu.driver,
        ));
    }
    if !gpu.clock {
        return Err(failed(&format!(
            "{}: {NO_CLOCK}, though the baseline holds its time: no time to check (a wgpu or driver change that took its clock is a defect to report)",
            gpu.adapter
        )));
    }
    let checked = checked_time(io, out, Measuring::Check, &gpu.adapter)?;
    if !within(checked, blessed.time, BOUND_PERCENT) {
        return Err(remedied(
            &one_line(&format!(
                "{}: {checked} ns, baseline {} ns",
                gpu.adapter, blessed.time
            )),
            &format!(
                ": beyond {} (run it again with no other heavy work running (see: {CHECK}); still beyond, it is a regression of the work, or a change of the machine the baseline does not record: measure the runs again ({PROCEDURE}), then bless with the reason (see: cargo xtask bless-benchmarks))",
                bounds()
            ),
        ));
    }
    Ok(())
}

/// The time a bless writes of the GPU `adapter`: the median of [`BLESS_RUNS`] runs, each
/// measured as a check's one run is, said on `out`.
///
/// # Errors
///
/// As [`checked_time`], at the first run that fails.
fn blessed_time(
    io: &mut dyn BenchIo,
    out: &mut dyn Write,
    adapter: &str,
) -> Result<u64, BenchError> {
    let mut times = [0; BLESS_RUNS];
    for (slot, run) in times.iter_mut().zip(1_usize..) {
        say(out, &format!("run {run} of {BLESS_RUNS}"))?;
        *slot = checked_time(io, out, Measuring::Bless, adapter)?;
    }
    let time = median_of_runs(times);
    say(
        out,
        &format!("{adapter}: the median of {BLESS_RUNS} runs: {time} ns"),
    )?;
    Ok(time)
}

/// The median of the times of [`BLESS_RUNS`] runs: the middle one.
fn median_of_runs(mut times: [u64; BLESS_RUNS]) -> u64 {
    times.sort_unstable();
    times[BLESS_RUNS / 2]
}

/// `bless`: measures the GPU [`BLESS_RUNS`] times and writes their median, with the system
/// and its driver, beside the others `held`.
///
/// # Errors
///
/// As [`run`]'s `bless`.
fn bless(
    io: &mut dyn BenchIo,
    out: &mut dyn Write,
    mut held: BTreeMap<String, Held>,
    gpu: Gpu,
) -> Result<(), BenchError> {
    if !gpu.clock {
        return Err(failed(&format!(
            "{}: {NO_CLOCK}: no time to bless",
            gpu.adapter
        )));
    }
    // Refused before measuring: a baseline that would not read back as written (a property of
    // the tests holds the rest to it).
    let system = system(io)?;
    if let Some(why) = unreadable([&gpu.adapter, &system, &gpu.driver]) {
        return Err(failed(&format!("cannot bless {:?}: {why}", gpu.adapter)));
    }
    let entry = Held {
        time: blessed_time(io, out, &gpu.adapter)?,
        system,
        driver: gpu.driver,
    };
    let _replaced = held.insert(gpu.adapter, entry);
    io.write_baseline(&format_baseline(&held))
        .map_err(|error| unwritable(&error))?;
    say(out, &format!("wrote {BASELINE}"))
}

/// The command saying the system's version, and the system's name it is written after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Version {
    /// The system's name.
    system: &'static str,
    /// The program saying its version, alone on its output.
    program: &'static str,
    /// Its arguments.
    args: &'static [&'static str],
}

/// macOS's version, which its GPU drivers ship with.
#[cfg(target_os = "macos")]
const VERSION: Version = Version {
    system: "macOS",
    program: "sw_vers",
    args: &["-productVersion"],
};

/// Another system's kernel release.
#[cfg(not(target_os = "macos"))]
const VERSION: Version = Version {
    system: std::env::consts::OS,
    program: "uname",
    args: &["-r"],
};

/// The real outside world of the command: the first GPU, and the baseline at `root`.
#[derive(Debug)]
#[non_exhaustive]
pub struct System {
    /// The repository root, where the baseline is.
    root: std::path::PathBuf,
    /// The GPU opened, until it is measured.
    gpu: Option<GpuSquare>,
    /// The command saying the system's version.
    version: Version,
}

impl System {
    /// The outside world of a command run from `root`, the repository root.
    #[must_use]
    pub const fn new(root: std::path::PathBuf) -> Self {
        Self {
            root,
            gpu: None,
            version: VERSION,
        }
    }

    /// That a baseline not found can be: an error where no `spec/` stands to hold it.
    fn missing(&self) -> io::Result<()> {
        if self.root.join("spec").is_dir() {
            Ok(())
        } else {
            Err(io::Error::other(
                "no spec directory here: run it from the repository's root",
            ))
        }
    }

    /// The GPU opened, or the first GPU opened now.
    fn opened(&mut self) -> Result<GpuSquare, Failure> {
        match self.gpu.take() {
            Some(gpu) => Ok(gpu),
            None => Ok(GpuSquare::new(wgpu::Backends::all())?),
        }
    }
}

impl sealed::Sealed for System {}

impl BenchIo for System {
    fn open(&mut self) -> Result<Gpu, Failure> {
        let gpu = self.opened()?;
        let named = Gpu {
            adapter: gpu.adapter().to_owned(),
            driver: gpu.driver().to_owned(),
            clock: gpu.period.is_some(),
        };
        self.gpu = Some(gpu);
        Ok(named)
    }

    fn system(&mut self) -> io::Result<String> {
        let program = self.version.program;
        let output = std::process::Command::new(program)
            .args(self.version.args)
            .output()
            .map_err(|error| io::Error::new(error.kind(), format!("{program}: {error}")))?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.trim();
            return Err(io::Error::other(if said.is_empty() {
                format!("{program} {}", output.status)
            } else {
                format!("{program} {}: {said}", output.status)
            }));
        }
        let version = String::from_utf8_lossy(&output.stdout);
        let version = version.trim();
        if version.is_empty() {
            return Err(io::Error::other(format!("{program} said no version")));
        }
        if version.contains(needs_escape) {
            return Err(io::Error::other(format!(
                "{program} said {version:?}, which holds a control, bidirectional formatting or line separator character"
            )));
        }
        Ok(format!("{} {version}", self.version.system))
    }

    fn measure(&mut self) -> Result<Times, Failure> {
        let gpu = self.opened()?;
        measure(&gpu)
    }

    fn read_baseline(&mut self) -> io::Result<Option<String>> {
        match std::fs::read_to_string(self.root.join(BASELINE)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.missing().map(|()| None),
            Err(error) => Err(error),
        }
    }

    fn writable(&mut self) -> io::Result<()> {
        // Opened without creating or truncating it, then closed as it was.
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .open(self.root.join(BASELINE));
        match opened {
            Ok(_file) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.missing(),
            Err(error) => Err(error),
        }
    }

    fn write_baseline(&mut self, text: &str) -> io::Result<()> {
        std::fs::write(self.root.join(BASELINE), text)
    }
}
