//! The state of one engine instance behind a C interface: what its exported functions
//! forward to.

#![forbid(unsafe_code)]

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use audio_core::host::Raw;
use audio_core::{Binary64, Detector, LimiterSettings, RenderFormat, Replaced};

use crate::{Blocks, ENGINE_BLOCK, Engine, LimiterStage, Output, OutputMode};

/// The item of `all` a code of the C interface names: its index, none past the list.
fn code_in<T: Copy>(code: u32, all: &[T]) -> Option<T> {
    usize::try_from(code)
        .ok()
        .and_then(|index| all.get(index).copied())
}

/// The render format of a code of [`Instance::load`]: its index in [`RenderFormat::ALL`],
/// part of the C interface (0 binary64, 1 binary64-as-binary32, 2 binary32), pinned by
/// literal tests on both sides of it.
fn format_of(code: u32) -> Option<RenderFormat> {
    code_in(code, &RenderFormat::ALL)
}

/// The output mode of a code of [`Instance::output`], part of the C interface: its index in
/// [`OutputMode::ALL`] (0 raw without a limiter, 1 protected, 2 switchable, raw first with
/// the limiter's latency), pinned by literal tests on both sides of it.
fn mode_of(code: u32) -> Option<OutputMode> {
    code_in(code, &OutputMode::ALL)
}

/// The detector of a code of [`Instance::output`], part of the C interface: its index in
/// [`Detector::ALL`] (0 standard, BS.1770's, 1 precise, 2 mastering), pinned by literal tests
/// on both sides of it.
fn detector_of(code: u32) -> Option<Detector> {
    code_in(code, &Detector::ALL)
}

/// Why a call that needs a loaded engine failed without one.
const NO_ENGINE: &str = "no engine is loaded";

/// The limiter's stage of a code of [`Instance::output`]: the detector of code `detector`, a
/// maximum lookahead of `maximum_lookahead` milliseconds and `settings`, whose lookahead it
/// holds, checked together ([`LimiterSettings::within_maximum`]).
fn limiter_stage(
    maximum_lookahead: f64,
    detector: u32,
    settings: [f64; 4],
) -> Result<LimiterStage, String> {
    let known = detector_of(detector).ok_or_else(|| {
        let [first, second, third] = Detector::ALL.map(Detector::name);
        alloc::format!("unknown detector {detector} (0 {first}, 1 {second}, 2 {third})")
    })?;
    let maximum = Binary64::from_raw(maximum_lookahead);
    limiter_settings(settings)
        .and_then(|limiter| {
            limiter
                .within_maximum(maximum)
                .map(|()| LimiterStage::new(maximum, known, limiter))
        })
        .map_err(|error| error.to_string())
}

/// The limiter's settings of the C interface's four numbers: a ceiling in dBTP, a lookahead
/// and a release in milliseconds, an adaptivity from 0 to 1, each checked against its range.
fn limiter_settings(
    [ceiling, lookahead, release, adaptivity]: [f64; 4],
) -> Result<LimiterSettings, audio_core::LimiterError> {
    LimiterSettings::new(
        Binary64::from_raw(ceiling),
        Binary64::from_raw(lookahead),
        Binary64::from_raw(release),
        Binary64::from_raw(adaptivity),
    )
}

/// What one engine instance holds: the input buffer its caller writes into, the loaded
/// engine and the last error message.
///
/// Every method is safe; a C interface only forwards to them.
#[derive(Debug)]
pub struct Instance {
    /// Bytes written by the caller: the patch to load.
    input: Vec<u8>,
    /// The loaded engine, if the last load succeeded.
    engine: Option<Engine>,
    /// Why the last call failed; empty after a successful one.
    message: String,
    /// The output stage every later load builds, its limiter's settings included, checked
    /// whole when set ([`Instance::output`]).
    next: Output,
    /// What the loaded engine's stage replaced, as the four words of the C interface
    /// ([`Instance::replaced_report`]), written when asked.
    report: [u32; 4],
}

impl Default for Instance {
    fn default() -> Self {
        Self::new()
    }
}

impl Instance {
    /// An instance with no engine loaded, whose loads build a raw output stage.
    #[must_use]
    pub const fn new() -> Self {
        Self::made()
    }

    /// [`Instance::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    const fn made() -> Self {
        Self {
            input: Vec::new(),
            engine: None,
            message: String::new(),
            next: Output::Raw,
            report: [0; 4],
        }
    }

    /// Makes the input buffer exactly `len` zero bytes long and returns it, for the
    /// caller to fill.
    pub fn input(&mut self, len: usize) -> &mut [u8] {
        self.input.clear();
        self.input.resize(len, 0);
        &mut self.input
    }

    /// Loads the patch in the input buffer at `rate` hertz, rendering in the format of code
    /// `format` (its index in [`RenderFormat::ALL`]: 0 binary64, 1 binary64 given as
    /// binary32, 2 binary32), replacing any loaded engine.
    ///
    /// Returns whether it succeeded; on failure no engine is loaded and
    /// [`Instance::message`] explains why.
    pub fn load(&mut self, rate: u32, format: u32) -> bool {
        let loaded = format_of(format)
            .ok_or_else(|| {
                // From `RenderFormat::ALL`, whose order is the codes: a fourth format does not
                // compile until it is named here.
                let [first, second, third] = RenderFormat::ALL.map(RenderFormat::name);
                alloc::format!("unknown render format {format} (0 {first}, 1 {second}, 2 {third})")
            })
            .and_then(|known| Engine::new(&self.input, rate, known, self.next));
        match loaded {
            Ok(engine) => {
                self.engine = Some(engine);
                self.message.clear();
                true
            }
            Err(message) => {
                self.engine = None;
                self.message = audio_patch::safe_message(&message);
                false
            }
        }
    }

    /// Records the outcome of a call: its message on failure, cleared on success.
    fn outcome(&mut self, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => {
                self.message.clear();
                true
            }
            Err(message) => {
                self.message = audio_patch::safe_message(&message);
                false
            }
        }
    }

    /// Sets the output stage every later load builds, the loaded engine keeping its own: its
    /// mode, of code `mode` (its index in [`OutputMode::ALL`]: 0 raw without a limiter, 1
    /// protected, 2 switchable), and for a mode with a limiter, the limiter's stage: the
    /// maximum lookahead in milliseconds and the detector, of code `detector` (its index in
    /// [`Detector::ALL`]: 0 standard, 1 precise, 2 mastering), which fix its latency, and its
    /// settings, a ceiling in dBTP, a lookahead and a release in milliseconds, an adaptivity
    /// from 0 to 1, the lookahead checked against the maximum in the same call. A raw stage
    /// has no limiter: none of them is read.
    ///
    /// Returns whether it succeeded; on failure nothing changed and [`Instance::message`]
    /// explains why (an unknown mode or detector, a value out of range, a lookahead above
    /// the maximum).
    #[expect(
        clippy::too_many_arguments,
        reason = "the scalars of the C interface's `engine_output`, forwarded as it takes them"
    )]
    pub fn output(
        &mut self,
        mode: u32,
        maximum_lookahead: f64,
        detector: u32,
        ceiling: f64,
        lookahead: f64,
        release: f64,
        adaptivity: f64,
    ) -> bool {
        let settings = [ceiling, lookahead, release, adaptivity];
        let set = match mode_of(mode) {
            None => {
                let [first, second, third] = OutputMode::ALL.map(OutputMode::name);
                Err(alloc::format!(
                    "unknown output mode {mode} (0 {first}, 1 {second}, 2 {third})"
                ))
            }
            Some(OutputMode::Raw) => Ok(Output::Raw),
            Some(OutputMode::Protected) => {
                limiter_stage(maximum_lookahead, detector, settings).map(Output::Protected)
            }
            Some(OutputMode::Switchable) => {
                limiter_stage(maximum_lookahead, detector, settings).map(Output::Switchable)
            }
        }
        .map(|output| self.next = output);
        self.outcome(set)
    }

    /// Changes the loaded engine's limiter live: a ceiling in dBTP, a lookahead and a release
    /// in milliseconds, an adaptivity from 0 to 1, the lookahead checked against the maximum
    /// of the loaded engine's limiter only. The stage stored for later loads keeps its own
    /// settings ([`Instance::output`]).
    ///
    /// Returns whether it succeeded; on failure nothing changed and [`Instance::message`]
    /// explains why (no engine loaded, a value out of range, a lookahead above the loaded
    /// engine's maximum, a loaded engine whose stage is raw, without a limiter).
    pub fn limiter(&mut self, ceiling: f64, lookahead: f64, release: f64, adaptivity: f64) -> bool {
        let set = self.engine.as_mut().map_or_else(
            || Err(NO_ENGINE.to_owned()),
            |engine| {
                limiter_settings([ceiling, lookahead, release, adaptivity])
                    .map_err(|error| error.to_string())
                    .and_then(|settings| engine.set_limiter(settings))
            },
        );
        self.outcome(set)
    }

    /// Switches the loaded engine's limiter toward protected (`protected` 1) or raw (0), by a
    /// crossfade: a switchable stage only goes raw ([`Engine::set_protected`]).
    ///
    /// Returns whether it succeeded; on failure [`Instance::message`] explains why (no
    /// engine loaded, a raw stage without a limiter, a protected stage told to go raw, a code
    /// other than 0 or 1).
    pub fn protect(&mut self, protected: u32) -> bool {
        let set = match (&mut self.engine, protected) {
            (None, _) => Err(NO_ENGINE.to_owned()),
            (Some(engine), 0 | 1) => engine.set_protected(protected == 1),
            (Some(_), _) => Err(alloc::format!(
                "unknown protection {protected} (0 raw, 1 protected)"
            )),
        };
        self.outcome(set)
    }

    /// Whether the loaded engine's limiter has a lookahead pending
    /// ([`Engine::pending_lookahead`]): set while its input needs a reduction (a switchable
    /// stage's switched to raw too), it takes effect at the limiter's next rest; false when
    /// none is loaded or its stage is built raw, without a limiter. Does not use the heap.
    #[must_use]
    pub fn lookahead_pending(&self) -> bool {
        self.engine
            .as_ref()
            .and_then(Engine::pending_lookahead)
            .is_some()
    }

    /// The latency of the loaded engine's output stage, in frames; zero when none is loaded
    /// or its stage is raw.
    #[must_use]
    pub fn latency(&self) -> usize {
        self.engine.as_ref().map_or(0, Engine::latency)
    }

    /// Why the last failed call failed (UTF-8); empty after a successful one.
    ///
    /// One line, safe to show: the message quotes untrusted text from the patch (box
    /// types, parameter and field names), made safe once here by
    /// [`audio_patch::safe_message`] (control and bidirectional characters escaped, at
    /// most [`audio_patch::MAX_MESSAGE_CHARS`] characters).
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Number of output channels of the loaded engine; zero when none is loaded.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.engine.as_ref().map_or(0, Engine::channels)
    }

    /// Renders the next block: `channels()` blocks of planar samples in the render format,
    /// none when no engine is loaded.
    pub fn process(&mut self) -> Blocks<'_, ENGINE_BLOCK> {
        self.engine
            .as_mut()
            .map_or(Blocks::Binary32(&[]), Engine::process)
    }

    /// The address of what the output stage of the loaded engine replaced since its first
    /// block ([`Engine::replaced`]), as four `u32` words, in order: the count, saturating at
    /// `u32::MAX`; the first sample's frame, its low then its high 32 bits; its channel,
    /// saturating at `u32::MAX`; the first sample's three words 0 while none was replaced,
    /// and four zeros when no engine is loaded. Valid until the next call into the instance.
    pub fn replaced_report(&mut self) -> *const u32 {
        self.report = self
            .engine
            .as_ref()
            .map_or([0; 4], |engine| report_words(engine.replaced()));
        self.report.as_ptr()
    }
}

/// `replaced` as the four words of the C interface (`engine_replaced`), in order: the count,
/// saturating at `u32::MAX`; the first sample's frame, its low then its high 32 bits; its
/// channel, saturating at `u32::MAX`; the first sample's three words 0 while none was
/// replaced.
#[must_use]
fn report_words(replaced: Replaced) -> [u32; 4] {
    let saturated = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
    let (frame, channel) = replaced.first().map_or((0, 0), |first| {
        (
            first.frame,
            u64::try_from(first.channel).unwrap_or(u64::MAX),
        )
    });
    [
        saturated(replaced.count()),
        saturated(frame & u64::from(u32::MAX)),
        saturated(frame >> 32_u32),
        saturated(channel),
    ]
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
