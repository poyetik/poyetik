//! Conformance cases: reading and rendering them.

use engine_core::{Plan, SampleRate};

use crate::{HEADER_LEN, wav_header};
use engine_hash::{RenderHash, render_hash};
use engine_objects::REGISTRY;
use serde::Deserialize;

/// A conformance case, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    /// Sample rate, in hertz.
    sample_rate: u32,
    /// Duration, in frames.
    frames: usize,
    /// The patch, in the patch format.
    patch: serde_json::Value,
}

/// The result of rendering a case.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// Sample rate.
    pub rate: SampleRate,
    /// Number of interleaved channels.
    pub channels: usize,
    /// The samples, channels interleaved.
    pub samples: Vec<f32>,
}

impl Rendered {
    /// The canonical hash of the render.
    #[must_use]
    pub fn hash(&self) -> RenderHash {
        render_hash(self.rate, self.channels, &self.samples)
    }
}

/// A conformance case, ready to render.
#[derive(Debug)]
pub struct Prepared {
    /// The validated execution plan.
    plan: Plan,
    /// Sample rate.
    rate: SampleRate,
    /// Duration, in frames.
    frames: usize,
    /// Header of the WAV file of this render: every prepared case fits a WAV file.
    wav_header: [u8; HEADER_LEN],
}

impl Prepared {
    /// Header of the WAV file of this render.
    #[must_use]
    pub const fn wav_header(&self) -> &[u8; HEADER_LEN] {
        &self.wav_header
    }

    /// Sample rate.
    #[must_use]
    pub const fn rate(&self) -> SampleRate {
        self.rate
    }

    /// Number of output channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.plan.channels()
    }

    /// Number of interleaved samples the render will produce.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.frames.saturating_mul(self.plan.channels())
    }

    /// Renders the case with the native engine.
    ///
    /// # Errors
    ///
    /// Returns a message when the samples cannot be held in memory.
    pub fn render(mut self) -> Result<Rendered, String> {
        let samples = self
            .plan
            .render(self.frames)
            .map_err(|error| error.to_string())?;
        Ok(Rendered {
            rate: self.rate,
            channels: self.plan.channels(),
            samples,
        })
    }
}

/// Reads a conformance case and builds its plan, without rendering it yet.
///
/// Also checks, before any rendering, that the render fits a WAV file.
///
/// # Errors
///
/// Returns a message when the case, its sample rate or its patch is invalid, when the
/// patch has no audio output, or when its render would not fit a WAV file.
pub fn prepare_case(json: &str) -> Result<Prepared, String> {
    let case: Case =
        serde_json::from_str(json).map_err(|error| format!("invalid case: {error}"))?;
    let rate = SampleRate::new(case.sample_rate).map_err(|error| error.to_string())?;
    let patch = engine_patch::parse_patch(&case.patch.to_string()).map_err(|e| e.to_string())?;
    let plan = Plan::build(&patch, &REGISTRY, rate).map_err(|error| error.to_string())?;
    if plan.channels() == 0 {
        return Err("the patch has no audio output: nothing to render".to_owned());
    }
    let samples = case.frames.saturating_mul(plan.channels());
    let wav_header = wav_header(rate.hz(), plan.channels(), samples)?;
    Ok(Prepared {
        plan,
        rate,
        frames: case.frames,
        wav_header,
    })
}

/// Reads a conformance case and renders it with the native engine.
///
/// # Errors
///
/// Returns a message when the case, its sample rate or its patch is invalid.
pub fn render_case(json: &str) -> Result<Rendered, String> {
    prepare_case(json).and_then(Prepared::render)
}
