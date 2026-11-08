//! Conformance cases: reading and rendering them.

use std::path::Path;

use engine_core::{Plan, SampleRate};

use crate::{HEADER_LEN, wav_header};
use engine_hash::{RenderHash, render_hash};
use engine_objects::REGISTRY;
use serde::Deserialize;

/// A conformance case, as written: how to render its patch, which is a separate file
/// read as is, so that every runner gives the engine exactly the same bytes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    /// Sample rate, in hertz.
    sample_rate: u32,
    /// Duration, in frames, from 1 to [`MAX_FRAMES`].
    frames: usize,
}

/// Longest case, in frames: 2^29, about three hours at 48 kHz. Shared with the oracle,
/// whose exact reference holds up to there; it also bounds the memory of a render.
pub const MAX_FRAMES: usize = 536_870_912;

/// Why a case cannot be rendered, and which of its two files is at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseError {
    /// The case file: invalid JSON or fields, an unsupported sample rate, or a duration
    /// whose render does not fit a WAV file.
    Case(String),
    /// The patch file: an invalid patch, or one without audio output.
    Patch(String),
}

impl CaseError {
    /// The message, prefixed by the path of the file at fault.
    #[must_use]
    pub fn in_files(&self, case: &Path, patch: &Path) -> String {
        match self {
            Self::Case(message) => format!("{}: {message}", case.display()),
            Self::Patch(message) => format!("{}: {message}", patch.display()),
        }
    }
}

impl core::fmt::Display for CaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Case(message) | Self::Patch(message) => f.write_str(message),
        }
    }
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
    /// The samples fit a WAV file (checked by [`prepare_case`]), at most 4 GiB. They are
    /// allocated at once: if memory cannot hold them, the process aborts, as any Rust
    /// allocation failure does.
    #[must_use]
    pub fn render(mut self) -> Rendered {
        let mut samples = vec![0.0_f32; self.sample_count()];
        self.plan.render_into(&mut samples);
        Rendered {
            rate: self.rate,
            channels: self.plan.channels(),
            samples,
        }
    }
}

/// Reads a conformance case and its patch (the text of the patch file, unchanged) and
/// builds its plan, without rendering it yet.
///
/// Also checks, before any rendering, that the render fits a WAV file.
///
/// # Errors
///
/// Returns [`CaseError::Case`] when the case, its duration (1 to [`MAX_FRAMES`] frames)
/// or its sample rate is invalid or when its
/// render would not fit a WAV file, and [`CaseError::Patch`] when the patch is invalid
/// or has no audio output.
pub fn prepare_case(case: &str, patch: &str) -> Result<Prepared, CaseError> {
    let case: Case = serde_json::from_str(case)
        .map_err(|error| CaseError::Case(format!("invalid case: {error}")))?;
    if !(1..=MAX_FRAMES).contains(&case.frames) {
        return Err(CaseError::Case(format!(
            "invalid case: frames must be between 1 and {MAX_FRAMES}, got {}",
            case.frames
        )));
    }
    let rate =
        SampleRate::new(case.sample_rate).map_err(|error| CaseError::Case(error.to_string()))?;
    let patch =
        engine_patch::parse_patch(patch).map_err(|error| CaseError::Patch(error.to_string()))?;
    let plan = Plan::build(&patch, &REGISTRY, rate)
        .map_err(|error| CaseError::Patch(error.to_string()))?;
    if plan.channels() == 0 {
        return Err(CaseError::Patch(
            "the patch has no audio output: nothing to render".to_owned(),
        ));
    }
    let samples = case.frames.saturating_mul(plan.channels());
    let wav_header = wav_header(rate.hz(), plan.channels(), samples).map_err(CaseError::Case)?;
    Ok(Prepared {
        plan,
        rate,
        frames: case.frames,
        wav_header,
    })
}

/// Reads a conformance case and its patch and renders it with the native engine.
///
/// # Errors
///
/// Returns the [`CaseError`] of [`prepare_case`].
pub fn render_case(case: &str, patch: &str) -> Result<Rendered, CaseError> {
    prepare_case(case, patch).map(Prepared::render)
}
