//! The state of one WebAssembly module instance, behind the [`crate::ffi`] functions.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Engine;

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

    /// Loads the patch in the input buffer at `rate` hertz, replacing any loaded engine.
    ///
    /// Returns whether it succeeded; on failure no engine is loaded and
    /// [`Instance::message`] explains why.
    pub fn load(&mut self, rate: u32) -> bool {
        match Engine::new(&self.input, rate) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.message.clear();
                true
            }
            Err(message) => {
                self.engine = None;
                self.message = message;
                false
            }
        }
    }

    /// Why the last load failed (UTF-8); empty after a successful load.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Number of output channels of the loaded engine; zero when none is loaded.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.engine.as_ref().map_or(0, Engine::channels)
    }

    /// Renders the next block: `channels()` blocks of planar samples, empty when no
    /// engine is loaded.
    pub fn process(&mut self) -> &[engine_core::Block] {
        self.engine.as_mut().map_or(&[], Engine::process)
    }
}

#[cfg(test)]
mod tests {
    use super::Instance;

    /// A 440 Hz sine on one channel.
    const MONO: &[u8] =
        br#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]}]}"#;

    fn loaded() -> Instance {
        let mut instance = Instance::new();
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(48_000), "{}", instance.message());
        instance
    }

    #[test]
    fn starts_empty() {
        let mut instance = Instance::default();
        assert_eq!(instance.channels(), 0, "no engine");
        assert!(instance.process().is_empty(), "no blocks");
        assert_eq!(instance.message(), "", "no error");
    }

    #[test]
    fn input_is_exactly_the_requested_zeroed_length() {
        let mut instance = Instance::new();
        instance.input(4).copy_from_slice(b"abcd");
        assert_eq!(instance.input(2), &[0, 0], "shrunk and zeroed");
        assert_eq!(instance.input(3), &[0, 0, 0], "grown and zeroed");
    }

    #[test]
    fn loads_and_renders() {
        let mut instance = loaded();
        assert_eq!(instance.channels(), 2, "dac~ has two inlets");
        assert_eq!(instance.message(), "", "no error");
        let blocks = instance.process();
        assert_eq!(blocks.len(), 2, "one block per channel");
        assert!(blocks[0].iter().any(|&s| s != 0.0), "sound on the left");
        assert!(blocks[1].iter().all(|&s| s == 0.0), "silence on the right");
    }

    #[test]
    fn a_failed_load_unloads_and_explains() {
        let mut instance = loaded();
        instance.input(1).copy_from_slice(b"x");
        assert!(!instance.load(48_000), "invalid patch");
        assert_eq!(instance.channels(), 0, "unloaded");
        assert!(instance.process().is_empty(), "no blocks");
        assert!(!instance.message().is_empty(), "explained");
    }

    #[test]
    fn a_successful_load_clears_the_previous_error() {
        let mut instance = Instance::new();
        assert!(!instance.load(48_000), "empty input");
        instance.input(MONO.len()).copy_from_slice(MONO);
        assert!(instance.load(48_000), "valid");
        assert_eq!(instance.message(), "", "cleared");
    }
}
