//! Conformance cases: reading and rendering them.

use std::path::Path;

use audio_core::{Binary32, Binary64, MODE, Plan, RenderFormat, Renderer, SampleRate};
use audio_fpenv::DefaultMode;

use crate::{HEADER_LEN, wav_header};
use audio_boxes::{EnginePlan, REGISTRY};
use audio_hash::{NotWholeFrames, RenderHash, Samples, render_hash};
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

/// The samples of a render, channels interleaved, in its render format.
///
/// It has no equality: one on its floats would be IEEE equality (`-0.0 == 0.0`, a NaN never
/// equal to itself), never the bits a render is compared by; compare renders by their
/// [`Rendered::hash`] or [`Rendered::wav`] bytes:
///
/// ```compile_fail,E0369
/// let samples = audio_cli::RenderedSamples::Binary64(vec![0.0]);
/// let _ = samples == audio_cli::RenderedSamples::Binary64(vec![-0.0]);
/// ```
#[derive(Debug, Clone)]
pub enum RenderedSamples {
    /// Computed and given in binary64.
    Binary64(Vec<f64>),
    /// Computed in binary64, given in binary32.
    Binary64AsBinary32(Vec<f32>),
    /// Computed and given in binary32.
    Binary32(Vec<f32>),
}

impl RenderedSamples {
    /// The samples, as the render hash reads them.
    #[must_use]
    pub fn as_samples(&self) -> Samples<'_> {
        match self {
            Self::Binary64(samples) => Samples::Binary64(samples),
            Self::Binary64AsBinary32(samples) => Samples::Binary64AsBinary32(samples),
            Self::Binary32(samples) => Samples::Binary32(samples),
        }
    }

    /// Appends every sample's bits, little-endian, to `bytes`: what a WAV file holds.
    pub fn write_le(&self, bytes: &mut Vec<u8>) {
        match self {
            Self::Binary64(samples) => {
                for sample in samples {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
            }
            Self::Binary64AsBinary32(samples) | Self::Binary32(samples) => {
                for sample in samples {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
    }
}

/// The result of rendering a case, with the header of its WAV file.
///
/// The two come from one prepared case, so a WAV file never pairs a header with other
/// samples: only [`Prepared::render`] builds one outside this crate's tests, and its
/// fields are private, read through [`Rendered::rate`], [`Rendered::channels`] and
/// [`Rendered::samples`], so that no one changes the samples under their header:
///
/// ```compile_fail,E0616
/// let case = r#"{"sample_rate":48000,"frames":100}"#;
/// let patch = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
///   "connections":[{"from":[1,0],"to":[2,0]}]}"#;
/// let format = audio_core::RenderFormat::Binary64;
/// audio_fpenv::with_default_mode(|mode| {
///     let prepared = audio_cli::prepare_case(case, patch, format, mode).expect("a case");
///     let mut rendered = prepared.render(mode);
///     rendered.samples = audio_cli::RenderedSamples::Binary32(vec![0.0; 3]);
///     rendered.wav()
/// });
/// ```
///
/// It has no equality, as its samples have none ([`RenderedSamples`]):
///
/// ```compile_fail,E0369
/// let case = r#"{"sample_rate":48000,"frames":100}"#;
/// let patch = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
///   "connections":[{"from":[1,0],"to":[2,0]}]}"#;
/// let format = audio_core::RenderFormat::Binary64;
/// audio_fpenv::with_default_mode(|mode| {
///     let render =
///         || audio_cli::prepare_case(case, patch, format, mode).map(|it| it.render(mode));
///     render().expect("a case") == render().expect("a case")
/// });
/// ```
#[derive(Debug, Clone)]
pub struct Rendered {
    /// Sample rate.
    pub(crate) rate: SampleRate,
    /// Number of interleaved channels.
    pub(crate) channels: usize,
    /// The samples, channels interleaved, in their render format.
    pub(crate) samples: RenderedSamples,
    /// Header of the WAV file of these samples, computed when the case was prepared.
    pub(crate) wav_header: [u8; HEADER_LEN],
}

impl Rendered {
    /// Its sample rate.
    #[must_use]
    pub const fn rate(&self) -> SampleRate {
        self.rate
    }

    /// Its number of interleaved channels.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// Its samples, channels interleaved, in their render format.
    #[must_use]
    pub const fn samples(&self) -> &RenderedSamples {
        &self.samples
    }

    /// The canonical hash of the render, computed in this build's arithmetic mode
    /// ([`MODE`]).
    ///
    /// # Errors
    ///
    /// Returns [`NotWholeFrames`] when its samples are not whole frames of its channels:
    /// never outside this crate, where only [`Prepared::render`] builds one; possibly for
    /// one built by hand in this crate's tests.
    pub fn hash(&self) -> Result<RenderHash, NotWholeFrames> {
        render_hash(self.rate, self.channels, self.samples.as_samples(), MODE)
    }

    /// The WAV file of the render: its header, then its samples, little-endian.
    #[must_use]
    pub fn wav(&self) -> Vec<u8> {
        let mut bytes = self.wav_header.to_vec();
        self.samples.write_le(&mut bytes);
        bytes
    }
}

/// A validated plan, in the precision of its render format, calling the built-in boxes
/// directly.
#[derive(Debug)]
enum Built {
    /// Rendered and given in binary64.
    Binary64(EnginePlan<Binary64>),
    /// Rendered in binary64, given in binary32.
    Binary64AsBinary32(EnginePlan<Binary64>),
    /// Rendered and given in binary32.
    Binary32(EnginePlan<Binary32>),
}

impl Built {
    /// Number of output channels.
    const fn channels(&self) -> usize {
        match self {
            Self::Binary64(plan) | Self::Binary64AsBinary32(plan) => plan.channels(),
            Self::Binary32(plan) => plan.channels(),
        }
    }
}

/// A conformance case, ready to render in one render format.
#[derive(Debug)]
pub struct Prepared {
    /// The validated execution plan, in its render format.
    plan: Built,
    /// Sample rate.
    rate: SampleRate,
    /// Duration, in frames.
    frames: usize,
    /// Header of the WAV file of this render: every prepared case fits a WAV file.
    wav_header: [u8; HEADER_LEN],
}

impl Prepared {
    /// Number of interleaved samples the render will produce.
    #[must_use]
    pub const fn sample_count(&self) -> usize {
        self.frames.saturating_mul(self.plan.channels())
    }

    /// Renders the case with the native engine, in the default floating-point mode that
    /// `_mode` proves (`audio_fpenv`): a render without it does not compile.
    ///
    /// The samples fit a WAV file (checked by [`prepare_case`]), at most 4 GiB. They are
    /// allocated at once: if memory cannot hold them, the process aborts, as any Rust
    /// allocation failure does.
    #[must_use]
    pub fn render(self, _mode: DefaultMode<'_>) -> Rendered {
        let count = self.sample_count();
        let channels = self.plan.channels();
        let samples = match self.plan {
            Built::Binary64(plan) => {
                let mut samples = vec![0.0_f64; count];
                Renderer::new(plan).render_into(&mut samples);
                RenderedSamples::Binary64(samples)
            }
            Built::Binary64AsBinary32(plan) => {
                let mut samples = vec![0.0_f32; count];
                Renderer::new(plan).render_into(&mut samples);
                RenderedSamples::Binary64AsBinary32(samples)
            }
            Built::Binary32(plan) => {
                let mut samples = vec![0.0_f32; count];
                Renderer::new(plan).render_into(&mut samples);
                RenderedSamples::Binary32(samples)
            }
        };
        Rendered {
            rate: self.rate,
            channels,
            samples,
            wav_header: self.wav_header,
        }
    }
}

/// Reads a conformance case and its patch (the text of the patch file, unchanged) and
/// builds its plan in the precision of `format`, without rendering it yet.
///
/// It runs in the default floating-point mode that `_mode` proves (`audio_fpenv`):
/// parsing and building do float work, so a preparation without the proof does not compile:
///
/// ```compile_fail,E0061
/// let case = r#"{"sample_rate":48000,"frames":100}"#;
/// let patch = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
///   "connections":[{"from":[1,0],"to":[2,0]}]}"#;
/// let format = audio_core::RenderFormat::Binary64;
/// let _ = audio_cli::prepare_case(case, patch, format);
/// ```
///
/// while inside [`audio_fpenv::with_default_mode`] it does:
///
/// ```
/// let case = r#"{"sample_rate":48000,"frames":100}"#;
/// let patch = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
///   "connections":[{"from":[1,0],"to":[2,0]}]}"#;
/// let format = audio_core::RenderFormat::Binary64;
/// let prepared = audio_fpenv::with_default_mode(|mode| {
///     audio_cli::prepare_case(case, patch, format, mode)
/// });
/// assert_eq!(prepared.map(|prepared| prepared.sample_count()), Ok(200));
/// ```
///
/// Also checks, before any rendering, that the render fits a WAV file.
///
/// # Errors
///
/// Returns [`CaseError::Case`] when the case, its duration (1 to [`MAX_FRAMES`] frames)
/// or its sample rate is invalid or when its
/// render would not fit a WAV file, and [`CaseError::Patch`] when the patch is invalid
/// or has no audio output.
pub fn prepare_case(
    case: &str,
    patch: &str,
    format: RenderFormat,
    _mode: DefaultMode<'_>,
) -> Result<Prepared, CaseError> {
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
        audio_patch::parse_patch(patch).map_err(|error| CaseError::Patch(error.to_string()))?;
    let built = |error: audio_core::CoreError| CaseError::Patch(error.to_string());
    let plan = match format {
        RenderFormat::Binary64 => {
            Built::Binary64(Plan::build(&patch, &REGISTRY, rate).map_err(built)?)
        }
        RenderFormat::Binary64AsBinary32 => {
            Built::Binary64AsBinary32(Plan::build(&patch, &REGISTRY, rate).map_err(built)?)
        }
        RenderFormat::Binary32 => {
            Built::Binary32(Plan::build(&patch, &REGISTRY, rate).map_err(built)?)
        }
    };
    if plan.channels() == 0 {
        return Err(CaseError::Patch(
            "the patch has no audio output: nothing to render".to_owned(),
        ));
    }
    let samples = case.frames.saturating_mul(plan.channels());
    let wav_header =
        wav_header(rate.hz(), plan.channels(), samples, format).map_err(CaseError::Case)?;
    Ok(Prepared {
        plan,
        rate,
        frames: case.frames,
        wav_header,
    })
}
