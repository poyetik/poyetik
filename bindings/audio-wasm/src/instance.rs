//! The state of one WebAssembly module instance, behind the [`crate::ffi`] functions.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use audio_core::RenderFormat;

use crate::{Blocks, Engine};

/// The render format of a code of [`Instance::load`]: its index in [`RenderFormat::ALL`],
/// part of the C interface (0 binary64, 1 binary64-as-binary32, 2 binary32), pinned by
/// literal tests on both sides of it.
fn format_of(code: u32) -> Option<RenderFormat> {
    usize::try_from(code)
        .ok()
        .and_then(|index| RenderFormat::ALL.get(index).copied())
}

/// What one module instance holds: the input buffer JavaScript writes into, the loaded
/// engine and the last error message.
///
/// Every method is safe; [`crate::ffi`] only forwards to them.
#[derive(Debug, Default)]
pub struct Instance {
    /// Bytes written by JavaScript: the patch to load.
    input: Vec<u8>,
    /// The loaded engine, if the last load succeeded.
    engine: Option<Engine>,
    /// Why the last load failed; empty after a successful load.
    message: String,
}

impl Instance {
    /// An instance with no engine loaded.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            input: Vec::new(),
            engine: None,
            message: String::new(),
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
            .and_then(|known| Engine::new(&self.input, rate, known));
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

    /// Why the last load failed (UTF-8); empty after a successful load.
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
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
