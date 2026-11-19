//! The GPU's time benchmark (`spec/benchmarks.md`, "Visual library"): the time the GPU takes
//! to render the square, by its own clock, held to a baseline per named GPU.
//!
//! A GPU's time depends on the GPU: it is checked only on a GPU whose baseline the
//! repository holds (`spec/gpu-time-baseline.txt`: the maintainer's Mac first, a GPU runner
//! of CI later), and shown on any other (the software GPU of CI). What does not depend on
//! the GPU, the transfers and work of an image ([`render_cost`](crate::render_cost)), is held
//! exactly by the tests on every machine.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use std::io::{self, Write};

use visual_boxes::Square;
use visual_core::{Image, ImageIndex, ImageRate, ImageSize, Seconds};

use crate::GpuSquare;

#[cfg(test)]
#[path = "bench_tests.rs"]
mod tests;

/// The baseline, from the repository root: one line per GPU, its name, a tab, then its
/// time in nanoseconds; `#` lines are comments.
pub const BASELINE: &str = "spec/gpu-time-baseline.txt";

/// The size whose time is checked: the largest of the ladder.
///
/// There the image's pixels dominate what a submission costs once (measured 2026-11-16 on an Apple M2 Pro: the
/// median of [`REPEATS`] varied by 2.3% from run to run at 1280 × 720, by 40% at 320 × 180).
pub const CHECKED: (u32, u32) = (1280, 720);

/// The sizes only shown, smallest first.
pub const SHOWN: [(u32, u32); 3] = [(64, 36), (320, 180), (640, 360)];

/// The renders before measuring, so that the GPU's clocks have risen.
pub const WARM_UP: usize = 15;

/// The renders measured; their median is the time.
pub const REPEATS: usize = 15;

/// How far a checked time may move from its baseline, in percent, either way: four times
/// the largest run-to-run variation measured (2.3%, [`CHECKED`]), so that only a change of
/// the work fails it.
pub const TOLERANCE_PERCENT: u64 = 10;

/// The usage of the command.
pub const USAGE: &str = "usage: visual-gpu <check|bless|help>
  check  measure this GPU's time and compare it with spec/gpu-time-baseline.txt, within ±10%, if it has a baseline
  bless  measure this GPU's time and write it to spec/gpu-time-baseline.txt
  help   print this help";

/// What the GPU took, by its own clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Times {
    /// The GPU's name and API, the baseline's key.
    pub adapter: String,
    /// The median nanoseconds of rendering an image at [`CHECKED`].
    pub checked: u64,
    /// The median nanoseconds at each of [`SHOWN`].
    pub shown: [u64; 3],
}

/// Why a measurement failed: the GPU's error, or the library's.
pub type Failure = Box<dyn core::error::Error>;

/// The square measured: the cost benchmarks' (`bindings/visual-wasm/benchmarks/suite.ts`).
fn measured_square() -> Result<Square, Failure> {
    let rate = ImageRate::new(60, 1)?;
    let cycle = Seconds::new(1001, 1000)?;
    Ok(Square::new(rate, 39_322, cycle)?)
}

/// The median nanoseconds the GPU takes to render image 7 at `size`, after [`WARM_UP`]
/// renders: `None` on a GPU without a clock.
fn median(gpu: &GpuSquare, square: &Square, size: (u32, u32)) -> Result<Option<u64>, Failure> {
    let mut image = Image::try_new(ImageSize::new(size.0, size.1)?)?;
    median_of(&mut || Ok(gpu.timed_render(square, ImageIndex(7), &mut image)?))
}

/// The median of what `render` measures, run [`WARM_UP`] times unmeasured, then
/// [`REPEATS`] times, of those measured: `None` when it measures nothing (a GPU without a
/// clock).
fn median_of(
    render: &mut dyn FnMut() -> Result<Option<f64>, Failure>,
) -> Result<Option<u64>, Failure> {
    let mut times = Vec::with_capacity(REPEATS);
    for at in 0..WARM_UP.saturating_add(REPEATS) {
        let took = render()?;
        if at >= WARM_UP {
            times.push(took);
        }
    }
    // The renders measured: a render the GPU wrote no timestamp of is left out
    // (`timed_render`), never counted as 0.
    let mut times: Vec<f64> = times.into_iter().flatten().collect();
    times.sort_by(f64::total_cmp);
    Ok(times
        .get(times.len() / 2)
        .map(|&nanoseconds| whole_nanoseconds(nanoseconds)))
}

/// `nanoseconds`, a time the GPU measured (never negative: ticks of a pass's end after its
/// start), rounded to whole nanoseconds by its decimal text (no `as` cast), at most
/// `u64::MAX`.
fn whole_nanoseconds(nanoseconds: f64) -> u64 {
    format!("{nanoseconds:.0}").parse().unwrap_or(u64::MAX)
}

/// Measures `gpu`'s times ([`Times`]): `None` on a GPU without a clock.
///
/// # Errors
///
/// Returns why the GPU could not render.
pub fn measure(gpu: &GpuSquare) -> Result<Option<Times>, Failure> {
    let square = measured_square()?;
    let Some(checked) = median(gpu, &square, CHECKED)? else {
        return Ok(None);
    };
    let mut shown = [0; 3];
    for (slot, size) in shown.iter_mut().zip(SHOWN) {
        *slot = median(gpu, &square, size)?.unwrap_or_default();
    }
    Ok(Some(Times {
        adapter: gpu.adapter().to_owned(),
        checked,
        shown,
    }))
}

/// Reads a baseline ([`BASELINE`]): each GPU's checked time, by its name.
///
/// # Errors
///
/// Returns the first line that is neither a comment nor a name, a tab and a whole number,
/// and a name written twice.
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
        let time = value
            .parse()
            .map_err(|_| format!("line {at}: {value:?} is not a whole number of nanoseconds"))?;
        if times.insert(name.to_owned(), time).is_some() {
            return Err(format!("line {at}: {name} written twice"));
        }
    }
    Ok(times)
}

/// The baseline of `times`, as `bless` writes it: a comment, then one line per GPU, by name.
#[must_use]
pub fn format_baseline(times: &BTreeMap<String, u64>) -> String {
    let mut text = String::from(
        "# The GPU's time to render the square at 1280x720, in nanoseconds, by its own clock\n\
         # (spec/benchmarks.md, \"Visual library\"): one line per GPU, written by\n\
         # `visual-gpu bless`.\n",
    );
    for (name, time) in times {
        // Writing to a string cannot fail.
        let _: core::fmt::Result = writeln!(text, "{name}\t{time}");
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
pub struct BenchError {
    /// 1 when the work failed, 2 for a usage error.
    pub code: u8,
    /// What went wrong, one line, without its `error: ` prefix.
    pub line: String,
}

/// A failure of the work.
const fn failed(line: String) -> BenchError {
    BenchError { code: 1, line }
}

/// The baseline's times, none when there is no file.
fn baseline(io: &mut dyn BenchIo) -> Result<BTreeMap<String, u64>, BenchError> {
    let text = io
        .read_baseline()
        .map_err(|error| failed(format!("cannot read {BASELINE}: {error}")))?;
    parse_baseline(&text.unwrap_or_default()).map_err(|why| failed(format!("{BASELINE}: {why}")))
}

/// Says each measured time on `out`, a closed stdout ignored.
fn report(out: &mut dyn Write, times: &Times) {
    let mut lines = String::new();
    for ((width, height), time) in SHOWN.into_iter().zip(times.shown) {
        let _: core::fmt::Result = writeln!(
            lines,
            "{}: render {width}x{height}: {time} ns (shown)",
            times.adapter
        );
    }
    let _: core::fmt::Result = writeln!(
        lines,
        "{}: render {}x{}: {} ns",
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
/// failure (code 1) when no GPU renders, the baseline cannot be read or written, `bless`
/// finds no clock, or `check` finds the time beyond [`TOLERANCE_PERCENT`] of its baseline.
pub fn run(args: &[String], io: &mut dyn BenchIo, out: &mut dyn Write) -> Result<(), BenchError> {
    let usage = |line: String| BenchError {
        code: 2,
        line: format!("{line} (see: visual-gpu help)"),
    };
    let command = match args {
        [] => return Err(usage("no command given".to_owned())),
        [command] => command.as_str(),
        [command, ..] => return Err(usage(format!("`{command}` takes no argument"))),
    };
    if command == "help" {
        drop(writeln!(out, "{USAGE}"));
        return Ok(());
    }
    if command != "check" && command != "bless" {
        return Err(usage(format!("unknown command {command:?}")));
    }
    let mut times = baseline(io)?;
    let measured = io
        .measure()
        .map_err(|error| failed(format!("cannot measure the GPU: {error}")))?;
    let Some(measured) = measured else {
        drop(writeln!(
            out,
            "this GPU has no clock its programs can read: no time"
        ));
        return if command == "bless" {
            Err(failed("no time to bless: this GPU has no clock".to_owned()))
        } else {
            Ok(())
        };
    };
    report(out, &measured);
    if command == "bless" {
        let _replaced = times.insert(measured.adapter.clone(), measured.checked);
        io.write_baseline(&format_baseline(&times))
            .map_err(|error| failed(format!("cannot write {BASELINE}: {error}")))?;
        drop(writeln!(out, "wrote {BASELINE}"));
        return Ok(());
    }
    let Some(&base) = times.get(&measured.adapter) else {
        drop(writeln!(
            out,
            "{}: no baseline: shown, not checked",
            measured.adapter
        ));
        return Ok(());
    };
    let moved = measured.checked.abs_diff(base).saturating_mul(100);
    if moved > base.saturating_mul(TOLERANCE_PERCENT) {
        return Err(failed(format!(
            "{}: {} ns, baseline {base} ns: beyond ±{TOLERANCE_PERCENT}% (bless it, with the reason, if the work changed)",
            measured.adapter, measured.checked
        )));
    }
    Ok(())
}

/// The real outside world of the command: the first GPU, and the baseline at `root`.
#[derive(Debug)]
pub struct System {
    /// The repository root, where the baseline is.
    pub root: std::path::PathBuf,
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
