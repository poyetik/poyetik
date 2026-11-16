//! Conformance cases: reading and rendering them.

use std::path::Path;

use audio_core::{
    Binary32, Binary64, Detector, LimiterError, LimiterSettings, MODE, ObjectSpec, OutputStage,
    Plan, RenderFormat, Renderer, Replaced, Sample, SampleRate, TruePeakLimiter,
};
use fpenv::DefaultMode;

use crate::{HEADER_LEN, wav_header};
use audio_boxes::{EnginePlan, EngineRenderer, REGISTRY};
use audio_hash::{NotWholeFrames, RenderHash, Samples, render_hash};
use serde::Deserialize;

/// What a case must be, as its refusals say it.
const CASE_SHAPE: &str = "a case: a JSON object with sample_rate, frames and optionally output";

/// What a case's output stage must be, as its refusals say it.
const OUTPUT_SHAPE: &str =
    "an output stage: a JSON object with mode, and lookahead and detector when protected";

/// A JSON object read as `T`, its fields by name, refused when written in any other form: a
/// derived struct also takes a JSON array of its fields in order (`[48000, 1]` for a case),
/// which the oracle refuses, so every reader of the format accepts the same documents.
#[derive(Debug, Clone, Copy)]
struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Object<T> {
    /// Reads `T` from `deserializer`, a JSON object only, `shape` saying what was expected.
    fn read<D: serde::Deserializer<'de>>(
        deserializer: D,
        shape: &'static str,
    ) -> Result<T, D::Error> {
        /// The visitor of a JSON object, handing its entries to `T`'s own reader.
        struct Entries<T>(core::marker::PhantomData<T>, &'static str);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Entries<T> {
            type Value = T;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str(self.1)
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
            }
        }
        deserializer.deserialize_map(Entries(core::marker::PhantomData, shape))
    }
}

impl<'de> Deserialize<'de> for Object<Case> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::read(deserializer, CASE_SHAPE).map(Self)
    }
}

impl<'de> Deserialize<'de> for Object<CaseOutput> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::read(deserializer, OUTPUT_SHAPE).map(Self)
    }
}

/// A conformance case, as written: how to render its patch, which is a separate file
/// read as is, so that every runner gives the engine exactly the same bytes. A JSON object
/// only ([`Object`]).
#[derive(Debug, Deserialize)]
#[serde(
    deny_unknown_fields,
    expecting = "a case: a JSON object with sample_rate, frames and optionally output"
)]
struct Case {
    /// Sample rate, in hertz.
    sample_rate: u32,
    /// Duration, in frames, from 1 to [`MAX_FRAMES`].
    frames: usize,
    /// The output stage of the render (`spec/engine-design.md`, decision 12): raw when the
    /// field is absent; never `null`.
    #[serde(default, deserialize_with = "present")]
    output: Option<Object<CaseOutput>>,
}

/// An optional field that, when present, holds a value: `null` refused, as the oracle
/// refuses it.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// The fields of a case's output stage, as written: its mode, and for a protected one its
/// limiter's lookahead in milliseconds (its maximum too) and its detector, the limiter's
/// other settings their defaults. A raw one carries no setting. Read into a [`CaseOutput`],
/// which refuses a setting on a raw stage and a missing one on a protected stage.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(
    deny_unknown_fields,
    expecting = "an output stage: a JSON object with mode, and lookahead and detector when protected"
)]
struct OutputFields {
    /// Raw or protected.
    mode: CaseMode,
    /// The limiter's lookahead and its maximum, in milliseconds (0.5 to 5): its latency
    /// with the detector's. Never `null`.
    #[serde(default, deserialize_with = "present")]
    lookahead: Option<f64>,
    /// The limiter's peak detector. Never `null`.
    #[serde(default, deserialize_with = "present")]
    detector: Option<CaseDetector>,
}

/// The output stage of a case: raw, which carries no setting, or protected, through the
/// limiter with its lookahead in milliseconds (its maximum too) and its detector, its other
/// settings their defaults (`spec/engine-design.md`, decision 12). A JSON object only
/// ([`Object`]), read from its [`OutputFields`].
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "OutputFields")]
enum CaseOutput {
    /// Without a limiter: non-finite samples replaced, nothing else.
    Raw,
    /// Through the true-peak limiter.
    Protected {
        /// The limiter's lookahead and its maximum, in milliseconds (0.5 to 5).
        lookahead: f64,
        /// The limiter's peak detector.
        detector: CaseDetector,
    },
}

impl TryFrom<OutputFields> for CaseOutput {
    type Error = String;

    fn try_from(fields: OutputFields) -> Result<Self, String> {
        match (fields.mode, fields.lookahead, fields.detector) {
            (CaseMode::Raw, None, None) => Ok(Self::Raw),
            (CaseMode::Raw, Some(_), _) => Err("a raw output stage takes no lookahead".to_owned()),
            (CaseMode::Raw, None, Some(_)) => {
                Err("a raw output stage takes no detector".to_owned())
            }
            (CaseMode::Protected, Some(lookahead), Some(detector)) => Ok(Self::Protected {
                lookahead,
                detector,
            }),
            (CaseMode::Protected, None, _) => Err("missing field `lookahead`".to_owned()),
            (CaseMode::Protected, Some(_), None) => Err("missing field `detector`".to_owned()),
        }
    }
}

/// The mode of a case's output stage, read from its name only (a string, as the oracle
/// reads it: never serde's other form of a unit variant, a JSON object of one key).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
enum CaseMode {
    /// Without a limiter: non-finite samples replaced, nothing else.
    Raw,
    /// Through the true-peak limiter, with its default settings.
    Protected,
}

/// The detector of a case's limiter, read from its name only, as [`CaseMode`]: one of
/// [`Detector::ALL`] by its [`Detector::name`], so that a detector added to the engine is
/// read by its name with no change here.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct CaseDetector(pub(crate) Detector);

impl TryFrom<String> for CaseMode {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        match name.as_str() {
            "raw" => Ok(Self::Raw),
            "protected" => Ok(Self::Protected),
            _ => Err(format!(
                "unknown variant `{name}`, expected `raw` or `protected`"
            )),
        }
    }
}

impl TryFrom<String> for CaseDetector {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        Detector::ALL
            .into_iter()
            .find(|detector| detector.name() == name)
            .map(Self)
            .ok_or_else(|| {
                let names: Vec<String> = Detector::ALL
                    .iter()
                    .map(|detector| format!("`{}`", detector.name()))
                    .collect();
                format!(
                    "unknown variant `{name}`, expected one of {}",
                    names.join(", ")
                )
            })
    }
}

impl CaseOutput {
    /// The stage of this output for `channels` channels at `rate`, in `S`.
    fn stage<S: Sample>(
        self,
        rate: SampleRate,
        channels: usize,
    ) -> Result<OutputStage<S>, CaseError> {
        match self {
            Self::Raw => Ok(OutputStage::new()),
            Self::Protected {
                lookahead,
                detector: CaseDetector(detector),
            } => {
                let lookahead = <Binary64 as audio_core::host::Raw>::from_raw(lookahead);
                let invalid =
                    |error: LimiterError| CaseError::Case(format!("invalid case: {error}"));
                let settings = LimiterSettings::DEFAULT
                    .with_lookahead(lookahead)
                    .map_err(invalid)?;
                // A count beyond `u16` is beyond the limiter's largest too: refused as the
                // largest `u16`.
                let channels = u16::try_from(channels).unwrap_or(u16::MAX);
                TruePeakLimiter::new(rate, channels, lookahead, detector, settings, true)
                    .map(OutputStage::limited)
                    .map_err(invalid)
            }
        }
    }
}

/// The stage of `output` (raw when absent) for `channels` channels at `rate`, in `S`.
fn stage_of<S: Sample>(
    output: Option<CaseOutput>,
    rate: SampleRate,
    channels: usize,
) -> Result<OutputStage<S>, CaseError> {
    output.map_or_else(
        || Ok(OutputStage::new()),
        |output| output.stage(rate, channels),
    )
}

/// The box types a patch may use: [`REGISTRY`], the engine's, for every command; the tests
/// also give one with test-only box types (a box giving a NaN, so that the commands meet a
/// render whose output stage replaced a sample).
pub(crate) type Registry = &'static [&'static ObjectSpec];

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
/// fpenv::with_default_mode(|mode| {
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
/// fpenv::with_default_mode(|mode| {
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
    /// What the output stage replaced: the non-finite samples, each rendered as zero.
    pub(crate) replaced: Replaced,
}

impl Rendered {
    /// What the output stage replaced (`spec/engine-design.md`, decision 12): the
    /// non-finite samples of the render, each rendered as zero, and the first of them.
    #[must_use]
    pub const fn replaced(&self) -> Replaced {
        self.replaced
    }

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

/// The renderer of a validated plan through its output stage, in the precision of its
/// render format, calling the built-in boxes directly.
#[derive(Debug)]
enum Built {
    /// Rendered and given in binary64.
    Binary64(EngineRenderer<Binary64>),
    /// Rendered in binary64, given in binary32.
    Binary64AsBinary32(EngineRenderer<Binary64>),
    /// Rendered and given in binary32.
    Binary32(EngineRenderer<Binary32>),
}

impl Built {
    /// Number of output channels.
    const fn channels(&self) -> usize {
        match self {
            Self::Binary64(renderer) | Self::Binary64AsBinary32(renderer) => renderer.channels(),
            Self::Binary32(renderer) => renderer.channels(),
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
    /// `_mode` proves (`fpenv`): a render without it does not compile.
    ///
    /// The samples fit a WAV file (checked by [`prepare_case`]), at most 4 GiB. They are
    /// allocated at once: if memory cannot hold them, the process aborts, as any Rust
    /// allocation failure does.
    #[must_use]
    pub fn render(self, _mode: DefaultMode<'_>) -> Rendered {
        self.render_unproven()
    }

    /// [`Prepared::render`] without the proof: in the floating-point mode of the caller, for
    /// the tests that render under a host's mode on purpose (`host_modes_tests`), so that
    /// they run the code every command runs, the proof apart.
    pub(crate) fn render_unproven(self) -> Rendered {
        let count = self.sample_count();
        let channels = self.plan.channels();
        let (samples, replaced) = match self.plan {
            Built::Binary64(mut renderer) => {
                let mut samples = vec![0.0_f64; count];
                renderer.render_into(&mut samples);
                (RenderedSamples::Binary64(samples), renderer.replaced())
            }
            Built::Binary64AsBinary32(mut renderer) => {
                let mut samples = vec![0.0_f32; count];
                renderer.render_into(&mut samples);
                (
                    RenderedSamples::Binary64AsBinary32(samples),
                    renderer.replaced(),
                )
            }
            Built::Binary32(mut renderer) => {
                let mut samples = vec![0.0_f32; count];
                renderer.render_into(&mut samples);
                (RenderedSamples::Binary32(samples), renderer.replaced())
            }
        };
        Rendered {
            rate: self.rate,
            channels,
            samples,
            wav_header: self.wav_header,
            replaced,
        }
    }
}

/// Reads a conformance case and its patch (the text of the patch file, unchanged) and
/// builds its plan in the precision of `format`, without rendering it yet.
///
/// It runs in the default floating-point mode that `_mode` proves (`fpenv`):
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
/// while inside [`fpenv::with_default_mode`] it does:
///
/// ```
/// let case = r#"{"sample_rate":48000,"frames":100}"#;
/// let patch = r#"{"format":1,"nodes":[{"id":1,"box":"osc~"},{"id":2,"box":"dac~"}],
///   "connections":[{"from":[1,0],"to":[2,0]}]}"#;
/// let format = audio_core::RenderFormat::Binary64;
/// let prepared = fpenv::with_default_mode(|mode| {
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
    mode: DefaultMode<'_>,
) -> Result<Prepared, CaseError> {
    prepare_in(case, patch, format, &REGISTRY, mode)
}

/// [`prepare_case`] with the box types of `registry`: how every command prepares a case,
/// with the engine's registry, which the tests replace.
///
/// # Errors
///
/// Those of [`prepare_case`].
pub(crate) fn prepare_in(
    case: &str,
    patch: &str,
    format: RenderFormat,
    registry: Registry,
    _mode: DefaultMode<'_>,
) -> Result<Prepared, CaseError> {
    prepare_unproven(case, patch, format, registry)
}

/// [`prepare_case`] without the proof: in the floating-point mode of the caller, for the
/// tests that prepare under a host's mode on purpose (`host_modes_tests`), so that they run
/// the code every command runs, the proof apart.
///
/// # Errors
///
/// Those of [`prepare_case`].
pub(crate) fn prepare_unproven(
    case: &str,
    patch: &str,
    format: RenderFormat,
    registry: Registry,
) -> Result<Prepared, CaseError> {
    let Object(case): Object<Case> = serde_json::from_str(case)
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
    let output = case.output.map(|Object(output)| output);
    // Reporting on the case's frames only: a sample computed past them, or still in the
    // limiter's delay at the end, is never delivered, whatever the engine block.
    let frames = u64::try_from(case.frames).unwrap_or(u64::MAX);
    let plan = match format {
        RenderFormat::Binary64 => {
            let plan: EnginePlan<Binary64> =
                Plan::build(&patch, registry, rate, audio_core::ExecutionMode::Offline)
                    .map_err(built)?;
            let stage = stage_of(output, rate, plan.channels())?.reporting_until(frames);
            Built::Binary64(Renderer::with_stage(plan, stage).map_err(built)?)
        }
        RenderFormat::Binary64AsBinary32 => {
            let plan: EnginePlan<Binary64> =
                Plan::build(&patch, registry, rate, audio_core::ExecutionMode::Offline)
                    .map_err(built)?;
            let stage = stage_of(output, rate, plan.channels())?.reporting_until(frames);
            Built::Binary64AsBinary32(Renderer::with_stage(plan, stage).map_err(built)?)
        }
        RenderFormat::Binary32 => {
            let plan: EnginePlan<Binary32> =
                Plan::build(&patch, registry, rate, audio_core::ExecutionMode::Offline)
                    .map_err(built)?;
            let stage = stage_of(output, rate, plan.channels())?.reporting_until(frames);
            Built::Binary32(Renderer::with_stage(plan, stage).map_err(built)?)
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
