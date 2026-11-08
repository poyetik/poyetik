//! The state of one WebAssembly module instance, behind the [`crate::ffi`] functions.

#![forbid(unsafe_code)]

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
                self.message = engine_patch::safe_message(&message);
                false
            }
        }
    }

    /// Why the last load failed (UTF-8); empty after a successful load.
    ///
    /// One line, safe to show: the message quotes untrusted text from the patch (box
    /// types, parameter and field names), made safe once here by
    /// [`engine_patch::safe_message`] (control and bidirectional characters escaped, at
    /// most [`engine_patch::MAX_MESSAGE_CHARS`] characters).
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
        assert!(
            blocks[0].iter().any(|&sample| sample != 0.0),
            "sound on the left"
        );
        assert!(
            blocks[1].iter().all(|&sample| sample == 0.0),
            "silence on the right"
        );
    }

    #[test]
    fn loads_at_the_requested_rate() {
        let patch = engine_patch::parse_patch(core::str::from_utf8(MONO).expect("UTF-8"))
            .expect("valid patch");
        for rate in engine_testkit::matrix::sample_rates() {
            let mut instance = Instance::new();
            instance.input(MONO.len()).copy_from_slice(MONO);
            assert!(instance.load(rate.hz()), "{}", instance.message());
            let expected = engine_core::Renderer::new(
                engine_core::Plan::build(&patch, &engine_objects::REGISTRY, rate)
                    .expect("valid plan"),
            )
            .render(engine_core::BLOCK_SIZE)
            .expect("one block");
            let left: Vec<f32> = expected.iter().step_by(2).copied().collect();
            assert_eq!(
                instance.process()[0].to_vec(),
                left,
                "the first block at {} Hz",
                rate.hz()
            );
        }
    }

    #[test]
    fn renders_without_a_heap_operation() {
        let mut instance = loaded();
        let operations = engine_testkit::heap_operations_during(|| {
            for _ in 0_u16..100_u16 {
                let _: &[engine_core::Block] = instance.process();
            }
        });
        assert_eq!(operations, Ok(((), 0)), "100 blocks");
        let mut empty = Instance::new();
        let none = engine_testkit::heap_operations_during(|| {
            let _: &[engine_core::Block] = empty.process();
        });
        assert_eq!(none, Ok(((), 0)), "nothing loaded");
    }

    #[test]
    fn a_failed_load_unloads_and_explains() {
        let mut instance = loaded();
        instance.input(1).copy_from_slice(b"x");
        assert!(!instance.load(48_000), "invalid patch");
        assert_eq!(instance.channels(), 0, "unloaded");
        assert!(instance.process().is_empty(), "no blocks");
        assert_eq!(
            instance.message(),
            "invalid patch: expected value at line 1 column 1",
            "explained"
        );
    }

    #[test]
    fn explains_in_one_escaped_bounded_line() {
        let mut instance = Instance::new();
        let hostile = br#"{"format":1,"nodes":[{"id":1,"box":"x\u001b[31m\nsecond\u202e"}],"connections":[]}"#;
        instance.input(hostile.len()).copy_from_slice(hostile);
        assert!(!instance.load(48_000), "unknown box");
        assert_eq!(
            instance.message(),
            "box 1: unknown box type \"x\\u{1b}[31m\\u{a}second\\u{202e}\"",
            "a box type from the patch, escaped"
        );
        let field = br#"{"format":1,"nodes":[],"connections":[],"a\u001b\n":1}"#;
        instance.input(field.len()).copy_from_slice(field);
        assert!(!instance.load(48_000), "unknown field");
        assert!(
            instance
                .message()
                .starts_with("invalid patch: unknown field `a\\u{1b}\\u{a}`"),
            "a field name echoed by the JSON reader, escaped: {}",
            instance.message()
        );
        let long = alloc::format!(
            r#"{{"format":1,"nodes":[{{"id":1,"box":"{}"}}],"connections":[]}}"#,
            "y".repeat(5_000)
        );
        instance.input(long.len()).copy_from_slice(long.as_bytes());
        assert!(!instance.load(48_000), "unknown box");
        assert_eq!(
            instance.message().chars().count(),
            engine_patch::MAX_MESSAGE_CHARS,
            "bounded: {}",
            instance.message()
        );
        assert!(instance.message().ends_with('…'), "marked as cut");
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
