//! The state of one WebAssembly module instance, behind the [`crate::ffi`] functions.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use audio_core::host::Raw;
use audio_core::{Binary64, Detector, LimiterError, LimiterSettings, RenderFormat, Setting};

use crate::{Blocks, Engine, Output, OutputMode};

/// The render format of a code of [`Instance::load`]: its index in [`RenderFormat::ALL`],
/// part of the C interface (0 binary64, 1 binary64-as-binary32, 2 binary32), pinned by
/// literal tests on both sides of it.
fn format_of(code: u32) -> Option<RenderFormat> {
    usize::try_from(code)
        .ok()
        .and_then(|index| RenderFormat::ALL.get(index).copied())
}

/// The output mode of a code of [`Instance::output`], part of the C interface: 0 raw
/// without a limiter, 1 protected, 2 switchable (raw first, with the limiter's latency).
const fn mode_of(code: u32) -> Option<OutputMode> {
    match code {
        0 => Some(OutputMode::Raw),
        1 => Some(OutputMode::Protected),
        2 => Some(OutputMode::Switchable),
        _ => None,
    }
}

/// The detector of a code of [`Instance::output`], part of the C interface: 0 standard
/// (BS.1770), 1 precise, 2 mastering.
const fn detector_of(code: u32) -> Option<Detector> {
    match code {
        0 => Some(Detector::Standard),
        1 => Some(Detector::Precise),
        2 => Some(Detector::Mastering),
        _ => None,
    }
}

/// The report of an instance without an engine: nothing replaced.
static NOTHING_REPLACED: [u32; 4] = [0; 4];

/// What one module instance holds: the input buffer JavaScript writes into, the loaded
/// engine and the last error message.
///
/// Every method is safe; [`crate::ffi`] only forwards to them.
#[derive(Debug)]
pub struct Instance {
    /// Bytes written by JavaScript: the patch to load.
    input: Vec<u8>,
    /// The loaded engine, if the last load succeeded.
    engine: Option<Engine>,
    /// Why the last call failed; empty after a successful one.
    message: String,
    /// The output stage every later load builds.
    output: Output,
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
        Self {
            input: Vec::new(),
            engine: None,
            message: String::new(),
            output: Output::RAW,
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
            .and_then(|known| Engine::new(&self.input, rate, known, self.output));
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

    /// Sets the output stage every later load builds: its mode, of code `mode` (0 raw
    /// without a limiter, 1 protected, 2 switchable), the maximum lookahead in
    /// milliseconds and the detector, of code `detector` (0 standard, 1 precise, 2
    /// mastering), which fix a limiter's latency. The loaded engine keeps its own.
    ///
    /// Returns whether it succeeded; on failure nothing changed and [`Instance::message`]
    /// explains why (an unknown mode or detector, a maximum out of range or below the
    /// lookahead set).
    pub fn output(&mut self, mode: u32, maximum_lookahead: f64, detector: u32) -> bool {
        let maximum = Binary64::from_raw(maximum_lookahead);
        let set = match (mode_of(mode), detector_of(detector)) {
            (None, _) => Err(alloc::format!(
                "unknown output mode {mode} (0 raw, 1 protected, 2 switchable)"
            )),
            (_, None) => Err(alloc::format!(
                "unknown detector {detector} (0 standard, 1 precise, 2 mastering)"
            )),
            (Some(known), Some(precision)) => {
                let (low, high) = Setting::MaximumLookahead.range();
                if !(maximum >= low && maximum <= high) {
                    Err(LimiterError::OutOfRange(Setting::MaximumLookahead, maximum).to_string())
                } else if self.output.settings.lookahead() > maximum {
                    Err(LimiterError::AboveMaximum {
                        lookahead: self.output.settings.lookahead(),
                        maximum,
                    }
                    .to_string())
                } else {
                    self.output.mode = known;
                    self.output.maximum_lookahead = maximum;
                    self.output.detector = precision;
                    Ok(())
                }
            }
        };
        self.outcome(set)
    }

    /// Sets the limiter's settings: a ceiling in dBTP, a lookahead and a release in
    /// milliseconds, an adaptivity from 0 to 1. They act live on the loaded engine, and on
    /// every later load.
    ///
    /// Returns whether it succeeded; on failure nothing changed and [`Instance::message`]
    /// explains why (a value out of range, a lookahead above the maximum, a loaded engine
    /// whose stage is raw, without a limiter).
    pub fn limiter(&mut self, ceiling: f64, lookahead: f64, release: f64, adaptivity: f64) -> bool {
        let set = LimiterSettings::new(
            Binary64::from_raw(ceiling),
            Binary64::from_raw(lookahead),
            Binary64::from_raw(release),
            Binary64::from_raw(adaptivity),
        )
        .map_err(|error| error.to_string())
        .and_then(|settings| {
            if settings.lookahead() > self.output.maximum_lookahead {
                return Err(LimiterError::AboveMaximum {
                    lookahead: settings.lookahead(),
                    maximum: self.output.maximum_lookahead,
                }
                .to_string());
            }
            if let Some(engine) = &mut self.engine {
                engine.set_limiter(settings)?;
            }
            self.output.settings = settings;
            Ok(())
        });
        self.outcome(set)
    }

    /// Switches the loaded engine's limiter toward protected (`protected` 1) or raw (0), by a
    /// crossfade.
    ///
    /// Returns whether it succeeded; on failure [`Instance::message`] explains why (no
    /// engine loaded, a raw stage without a limiter, a code other than 0 or 1).
    pub fn protect(&mut self, protected: u32) -> bool {
        let set = match (&mut self.engine, protected) {
            (None, _) => Err("no engine is loaded".to_owned()),
            (Some(engine), 0 | 1) => engine.set_protected(protected == 1),
            (Some(_), _) => Err(alloc::format!(
                "unknown protection {protected} (0 raw, 1 protected)"
            )),
        };
        self.outcome(set)
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
    pub fn process(&mut self) -> Blocks<'_> {
        self.engine
            .as_mut()
            .map_or(Blocks::Binary32(&[]), Engine::process)
    }

    /// The address of what the output stage of the loaded engine replaced since its first
    /// block, as four `u32` words ([`Engine::replaced_report`]); four zeros when no engine is
    /// loaded.
    pub fn replaced_report(&mut self) -> *const u32 {
        self.engine
            .as_mut()
            .map_or(NOTHING_REPLACED.as_ptr(), Engine::replaced_report)
    }
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
