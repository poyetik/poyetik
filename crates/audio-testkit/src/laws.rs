//! The laws every box must respect.

use alloc::collections::BTreeMap;

use audio_core::host::{Fault, Field, ParamBounds as _, Part, Raw, TextKind, text_fault};
use audio_core::{
    BLOCK_SIZE, Binary32, Binary64, Block, BoxInstance, BoxMode, Dispatch, Object, ObjectSpec,
    ParamSpec, ParamValues, Precision, Sample as _, SampleRate,
};
use proptest::array::uniform;
use proptest::collection::vec;
use proptest::test_runner::TestCaseError;

use audio_battery::{BLOCKS, BatterySample, param_sets, run_blocks, special_input};

use crate::heap_operations_during;
use crate::matrix::{PRECISIONS, sample_rates};

/// What the laws read of a sample, in either precision.
pub(crate) trait LawSample: BatterySample {
    /// Whether it is neither NaN nor infinite.
    fn finite(self) -> bool;

    /// Whether it is subnormal in its precision.
    fn subnormal(self) -> bool;
}

impl LawSample for Binary64 {
    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

impl LawSample for Binary32 {
    fn finite(self) -> bool {
        self.raw().is_finite()
    }

    fn subnormal(self) -> bool {
        self.raw().is_subnormal()
    }
}

/// Blocks rendered by the long run of each sample rate and parameter set: 65,536 frames,
/// 1.4 s at 48 kHz and 0.17 s at 384 kHz. Enough to reveal state that overflows, becomes
/// non-finite or uses the heap late; the run is compared with no reference, so slow drift
/// is not detected here (a drift check against the exact phase is #12 step 20).
const LONG_RUN_BLOCKS: usize = 1024;

/// Random input cases tried per law.
const CASES: u32 = 32;

/// Checks that a list of names, each of kind `kind`, holds none refused ([`check_text`]) and
/// none twice: the name at position `index` of the list is said as `what(index)`, a name
/// repeated with the position of the first, as a module's description is refused
/// (`audio_core::host::Field`, `audio_core::host::Fault::Repeated`).
///
/// Takes `dyn` arguments, not generics: one compiled function, so that the coverage of each
/// line is measured over every caller together.
fn unique_names(
    what: &dyn Fn(u32) -> String,
    kind: TextKind,
    names: &mut dyn Iterator<Item = &str>,
) -> Result<(), String> {
    let mut seen = BTreeMap::new();
    for (index, name) in (0_u32..).zip(names) {
        let field = what(index);
        check_text(name, kind, &field)?;
        if let Some(first) = seen.insert(name, index) {
            return Err(format!("{field} {name:?} {}", Fault::Repeated { first }));
        }
    }
    Ok(())
}

/// Checks `text`, of kind `kind`, by the box contract's rule for every text of a description
/// (`audio_core::host::text_fault`).
///
/// # Errors
///
/// Returns why `what` is refused, the text quoted escaped unless it shows nothing.
fn check_text(text: &str, kind: TextKind, what: &str) -> Result<(), String> {
    match text_fault(text, kind) {
        None => Ok(()),
        Some(Fault::Blank) => Err(format!("{what} {}", Fault::Blank)),
        Some(fault) => Err(format!("{what} {text:?} {fault}")),
    }
}

/// Checks the static description of a box type, by the rules of a module's description
/// (`spec/box-contract.md`, section 4) where they apply to it.
///
/// # Errors
///
/// Returns the first problem found: a text the box contract refuses
/// (`audio_core::host::text_fault`: the name and the description text of the box type, of a
/// port or a parameter), a port or parameter name or a capability given twice. A port, a
/// parameter or a capability is named by its position, as a module's description is refused
/// (`audio_core::host::Field`), and one given twice with the position of the first
/// (`audio_core::host::Fault::Repeated`). A parameter's
/// range and default need no law: a `audio_core::ParamSpec` is built only with a finite range
/// holding its default.
pub fn check_description(spec: &ObjectSpec) -> Result<(), String> {
    check_text(spec.name(), TextKind::BoxTypeName, "box type name")?;
    check_text(
        spec.description(),
        TextKind::DescriptionText,
        "description text",
    )?;
    let kind = TextKind::Name;
    let inlet = |index, part| Field::Inlet { index, part };
    let outlet = |index, part| Field::Outlet { index, part };
    let param = |index, part| Field::Param { index, part };
    unique_names(
        &|index| inlet(index, Part::Name).to_string(),
        kind,
        &mut spec.inlets().iter().map(|port| port.name),
    )?;
    unique_names(
        &|index| outlet(index, Part::Name).to_string(),
        kind,
        &mut spec.outlets().iter().map(|port| port.name),
    )?;
    unique_names(
        &|index| param(index, Part::Name).to_string(),
        kind,
        &mut spec.params().iter().map(ParamSpec::name),
    )?;
    let texts = |field: &dyn Fn(u32, Part) -> Field, texts: &mut dyn Iterator<Item = &str>| {
        (0_u32..).zip(texts).try_for_each(|(index, text)| {
            let what = field(index, Part::DescriptionText).to_string();
            check_text(text, TextKind::DescriptionText, &what)
        })
    };
    texts(
        &inlet,
        &mut spec.inlets().iter().map(|port| port.description),
    )?;
    texts(
        &outlet,
        &mut spec.outlets().iter().map(|port| port.description),
    )?;
    texts(
        &param,
        &mut spec.params().iter().map(ParamSpec::description),
    )?;
    let mut asked = BTreeMap::new();
    for (index, capability) in (0_u32..).zip(spec.capabilities()) {
        if let Some(first) = asked.insert(capability, index) {
            return Err(format!(
                "{} {capability:?} {}",
                Field::Capability { index },
                Fault::Repeated { first }
            ));
        }
    }
    Ok(())
}

/// Checks that a patch can write each default and bound of the box type's parameters.
///
/// A patch writes its values in binary32 (format version 1), and a range is in binary64: a
/// bound that is not a binary32 refuses the value a patch writes for it, or is one no patch
/// reaches, and a default that is not one differs once written. Until a patch's values are
/// binary64 (roadmap #3 step 7).
///
/// A value below 2^-126 in magnitude is the zero of its sign that the engine holds in its
/// place (a default, as `audio_core::ParamValues::defaults` flushes it; no bound is one,
/// which `audio_core::ParamSpec::new` refuses), which a patch writes exactly: it is told
/// by its exponent alone, as the engine tells it, so that a value of the binade at 2^-126
/// that is not a binary32 is refused.
///
/// # Errors
///
/// Returns the first value that is not a binary32.
fn check_written(spec: &ObjectSpec) -> Result<(), String> {
    let exponent = f64::INFINITY.to_bits();
    let smallest_normal = f64::from(f32::MIN_POSITIVE).to_bits();
    for param in spec.params() {
        for (what, value) in [
            ("default", param.default_value()),
            ("minimum", param.minimum()),
            ("maximum", param.maximum()),
        ] {
            if value.to_bits() & exponent < smallest_normal {
                continue;
            }
            let given = Binary64::new(value);
            if Binary32::from_binary64(given).to_binary64().bits() != given.bits() {
                return Err(format!(
                    "parameter \"{}\": its {what} {value:?} is not a binary32, as a patch of \
                     format version 1 writes a value (roadmap #3 step 7)",
                    param.name()
                ));
            }
        }
    }
    Ok(())
}

/// Checks every box of a registry, that their names are unique, and that a patch can write
/// every default and bound of their parameters exactly.
///
/// # Errors
///
/// Returns the first problem found: a box type name the box contract refuses
/// (`audio_core::host::text_fault`) or one given twice, named by the box type's position in
/// the registry and unprefixed, since the names are checked before any box; then the first
/// problem of a box, prefixed by its name.
pub fn check_registry(registry: &[&ObjectSpec]) -> Result<(), String> {
    let mut names = registry.iter().map(|spec| spec.name());
    unique_names(
        &|index| format!("box type {index}, its name"),
        TextKind::BoxTypeName,
        &mut names,
    )?;
    for spec in registry {
        check_description(spec)
            .and_then(|()| check_written(spec))
            .map_err(|problem| format!("{}: {problem}", spec.name()))?;
    }
    Ok(())
}

/// Renders `blocks` blocks of a fresh instance computing in `S`, feeding consecutive input
/// blocks taken from `input` (then silence), every output sample set to `entry` before every
/// block ([`run_blocks`]). Returns the outputs and the heap operations made while
/// processing.
///
/// The parameters are given as a plan gives them, as `audio_core::ParamValues`.
///
/// # Errors
///
/// Returns [`unbuilt`]'s refusal when the box type is not built for engine blocks of
/// [`BLOCK_SIZE`] frames, and why the heap operations cannot be counted.
fn render<S: LawSample>(
    params: &ParamValues,
    rate: SampleRate,
    input: &[Block<S>],
    blocks: usize,
    entry: S,
) -> Result<(Vec<Block<S>>, usize), String> {
    let mut object = crate::driven::create(
        <Box<dyn BoxInstance<S>> as Dispatch<S>>::create,
        rate,
        params,
    )?;
    let mut operations = 0_usize;
    let rendered = run_blocks(params.spec(), input, blocks, entry, |given, outputs| {
        let ((), count) = heap_operations_during(|| object.process(given, outputs))?;
        operations = operations.saturating_add(count);
        Ok::<(), String>(())
    });
    Ok((rendered?, operations))
}

/// The refusal of a box type its instance type does not create in engine blocks of `frames`
/// frames, worded as the plan's (`audio_core::CoreError::Unbuilt`).
pub(crate) fn unbuilt(frames: usize) -> String {
    format!("the box type is not built for engine blocks of {frames} frame(s)")
}

/// Bit patterns of rendered samples, so that `-0.0` and NaN payloads are compared exactly.
pub(crate) fn bits<S: LawSample>(blocks: &[Block<S>]) -> Vec<u64> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.bits())
        .collect()
}

/// The parameter values `params`, as the laws name them: in binary64, as a box is given them.
pub(crate) fn shown(params: &ParamValues) -> Vec<f64> {
    params.params().iter().map(|&value| value.raw()).collect()
}

/// Why the output laws refuse a rendered sample: one cause each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unlawful {
    /// It is NaN or infinite.
    NotFinite,
    /// It is subnormal in its precision.
    Subnormal,
}

/// The first sample of `rendered`, in the order it is rendered, that is not finite, else the
/// first that is subnormal, with its index and why: the one check of the samples the output
/// laws make, those of every box ([`check_laws`]) and those of a driven box
/// (`crate::check_driven_laws`).
pub(crate) fn unlawful<S: LawSample>(rendered: &[Block<S>]) -> Option<(usize, S, Unlawful)> {
    let samples = || rendered.iter().flatten().copied().enumerate();
    samples()
        .find(|&(_, sample)| !sample.finite())
        .map(|(index, sample)| (index, sample, Unlawful::NotFinite))
        .or_else(|| {
            samples()
                .find(|&(_, sample)| sample.subnormal())
                .map(|(index, sample)| (index, sample, Unlawful::Subnormal))
        })
}

/// Checks that a box type of mode `mode` made no heap operation (no allocation,
/// reallocation or release) while processing, `operations` counted: the one check of the
/// heap the laws make, of every box and of a driven box. The real-time rules hold
/// live-capable boxes only: an offline-only one may allocate while processing, and is held
/// to every other law (decision 20).
pub(crate) fn check_heap(operations: usize, mode: BoxMode) -> Result<(), String> {
    if operations > 0 && mode == BoxMode::LiveCapable {
        Err(format!(
            "uses the heap while processing ({operations} heap operations)"
        ))
    } else {
        Ok(())
    }
}

/// Checks that rendered blocks are finite and hold no subnormal ([`unlawful`]), and that
/// rendering made no heap operation ([`check_heap`]). A sample that is not finite is said as
/// `non_finite`, which names every cause the render allows.
fn check_rendered<S: LawSample>(
    rendered: &[Block<S>],
    operations: usize,
    params: &ParamValues,
    non_finite: &str,
) -> Result<(), String> {
    match unlawful(rendered) {
        Some((_, _, Unlawful::NotFinite)) => {
            Err(format!("{non_finite} with parameters {:?}", shown(params)))
        }
        Some((_, _, Unlawful::Subnormal)) => Err(format!(
            "subnormal output with parameters {:?}",
            shown(params)
        )),
        None => check_heap(operations, params.spec().mode()),
    }
}

/// What the second render of [`check_input`] and the long run ([`check_long_run`]) set
/// every output sample to before every block, where the first render sets +0: a quiet NaN
/// with a payload of its own, as close to no value a box computes as a sample can be.
const ENTRY_BITS: u64 = 0x7ff8_0000_dead_beef;

/// [`ENTRY_BITS`] in the precision `S`.
pub(crate) fn entry<S: LawSample>() -> S {
    S::from_binary64(Binary64::from_raw(f64::from_bits(ENTRY_BITS)))
}

/// Checks one input against the determinism, outputs-on-entry, finite-output,
/// no-subnormal and no-heap laws.
///
/// The determinism law renders each case twice, from two fresh instances: the first with
/// its outputs +0 on entry to every block, the second with them [`ENTRY_BITS`], which the
/// box contract leaves unspecified (a box writes every sample of every outlet,
/// `audio_core::Object::process`), so the two must give the same bits. Only when they do
/// not, a third render with +0, from a third instance, tells a box whose renders differ
/// with their entry alone from one that is not deterministic: when the third matches the
/// first, the box reads its outputs, leaves some of their samples unwritten, or differs
/// between instances in a way three renders cannot tell from those (every other instance
/// alike), and the law names the three. A box reading its outputs through a function
/// that maps both values alike (the larger of the output and its input, which NaN and +0
/// can both lose to) escapes the law: review only.
fn check_input<S: LawSample>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    input: &[Block<S>],
) -> Result<(), String> {
    for params in param_sets(spec) {
        let (first, operations) = render(&params, rate, input, BLOCKS, S::ZERO)?;
        let (second, _) = render(&params, rate, input, BLOCKS, entry::<S>())?;
        if bits(&first) != bits(&second) {
            let (third, _) = render(&params, rate, input, BLOCKS, S::ZERO)?;
            let cause = if bits(&first) == bits(&third) {
                "reads its outputs, leaves some of their samples unwritten, or differs between \
                 instances,"
            } else {
                "not deterministic"
            };
            return Err(format!("{cause} with parameters {:?}", shown(&params)));
        }
        check_rendered(&first, operations, &params, "non-finite output")?;
    }
    Ok(())
}

/// Checks a long run of silence-fed blocks, per parameter set: finite, without subnormal
/// and without heap operations.
///
/// Every output sample is [`ENTRY_BITS`] on entry to every block, so that a box that stops
/// writing its outputs once fed silence (one skipping its work in silence, a tail ended),
/// which no block of the inputs [`check_input`] renders shows, leaves a NaN the finite law
/// sees: a box writing every sample of every outlet at every block gives the same bits
/// whatever its outputs hold on entry (`audio_core::Object::process`). A NaN found is said
/// with both causes it can have.
fn check_long_run<S: LawSample>(spec: &'static ObjectSpec, rate: SampleRate) -> Result<(), String> {
    for params in param_sets(spec) {
        let (rendered, operations) =
            render::<S>(&params, rate, &[], LONG_RUN_BLOCKS, entry::<S>())?;
        check_rendered(
            &rendered,
            operations,
            &params,
            "non-finite output, or outputs read or left unwritten (a NaN on entry),",
        )
        .map_err(|problem| format!("long run: {problem}"))?;
    }
    Ok(())
}

/// Checks every law at one sample rate, computing in `S`.
fn check_rate<S: LawSample>(
    spec: &'static ObjectSpec,
    rate: SampleRate,
    blocks: usize,
) -> Result<(), String> {
    check_input(spec, rate, &special_input::<S>(blocks))
        .map_err(|problem| format!("special inputs: {problem}"))?;
    check_long_run::<S>(spec, rate)?;
    crate::runner::runner(CASES)
        .run(
            &vec(uniform::<_, BLOCK_SIZE>(-1.0_f64..=1.0_f64), blocks),
            |input| {
                let samples: Vec<Block<S>> = input
                    .iter()
                    .map(|block| block.map(|value| S::from_binary64(Binary64::from_raw(value))))
                    .collect();
                check_input(spec, rate, &samples).map_err(TestCaseError::fail)
            },
        )
        .map_err(|error| error.to_string())
}

/// Checks every law at every sample rate of the [`crate::matrix`], computing in `S`.
fn check_precision<S: LawSample>(spec: &'static ObjectSpec, blocks: usize) -> Result<(), String> {
    for rate in sample_rates() {
        check_rate::<S>(spec, rate, blocks)
            .map_err(|problem| format!("at {} Hz: {problem}", rate.hz()))?;
    }
    Ok(())
}

/// Checks every law on a box type in both precisions and at every sample rate.
///
/// The precisions and rates of the [`crate::matrix`]: special inputs, a long run, and random
/// inputs from a fixed seed (in binary64, rounded to binary32 for the binary32 instance).
///
/// # Errors
///
/// Returns a description of the first broken law, prefixed by the precision and the sample
/// rate (`binary32, at 8000 Hz: …`). Also fails when [`crate::CountingAllocator`] is not
/// installed, since the no-heap law could not be checked.
pub fn check_laws(spec: &'static ObjectSpec) -> Result<(), String> {
    let ((), _) = heap_operations_during(|| ())?;
    check_description(spec)?;
    let blocks = BLOCKS.saturating_mul(spec.inlets().len());
    for precision in PRECISIONS {
        match precision {
            Precision::Binary64 => check_precision::<Binary64>(spec, blocks),
            Precision::Binary32 => check_precision::<Binary32>(spec, blocks),
        }
        .map_err(|problem| format!("binary{}, {problem}", precision.bits()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
