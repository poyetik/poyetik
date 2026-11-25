//! Tests of the fingerprint: stable for the same bits, changed by a bit of any run of the
//! battery.

use audio_core::DrivenValues;
use audio_core::host::{Delivered, Raw};
use audio_core::{
    AlgorithmVersion, BLOCK_SIZE, Binary32, Binary64, Block, BoxInstance, Create, Dispatch, Driven,
    Object, ObjectSpec, ParamSpec, ParamValues, PortSpec, Precision, Sample, SampleRate,
};

use core::convert::Infallible;

use super::{FINGERPRINT_BATTERY, fingerprint, run_blocks, seeded_signal, splitmix64};
use crate::inputs::{BLOCKS, param_sets};
use sha2::{Digest, Sha256};

const IN: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};
const GAIN: ParamSpec = audio_core::param!("gain", "gain", 0.5, 0.0, 1.0);

/// Every box through its virtual table: the instance type of these tests, whose box types
/// are none of the engine's.
type Virtual<S> = Box<dyn BoxInstance<S>>;

/// The fingerprint of a copy of `spec`, made a static as a plan's box types are: one small
/// copy per call, kept for the test process, through its factory.
fn print(spec: ObjectSpec) -> String {
    fingerprint::<Virtual<Binary64>, Virtual<Binary32>>(Box::leak(Box::new(spec)))
        .expect("a test build creates every box type")
}

/// Where a quirk of [`Echo`] changes its output: under one condition only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quirk {
    /// Nowhere: the echo itself.
    None,
    /// At the highest sample rate of the matrix only.
    HighestRate,
    /// In binary32 only.
    Binary32,
    /// With its gain at its maximum only (one parameter set of the laws).
    Maximum,
    /// On the seeded random signal only (an input strictly between 0.5 and 0.75, which no
    /// special input of the matrix is).
    Random,
    /// On the last block only.
    LastBlock,
    /// On a special input of the matrix only: the smallest normal of either precision,
    /// which the seeded random signal never is.
    Special,
    /// With its gain nonzero and below 2^-126, which no parameter given as `Params` is.
    SubnormalGain,
}

/// Multiplies its input by its gain, then flips the lowest bit of its output, in the
/// precision of the render, where its quirk says: the least change a battery can see.
#[derive(Debug)]
struct Echo {
    /// Its gain.
    gain: f64,
    /// Whether its quirk applies to this instance (rate, precision, parameters).
    quirky: bool,
    /// Its quirk.
    quirk: Quirk,
    /// The blocks processed so far.
    blocks: usize,
}

/// `sample` with the lowest bit of its precision flipped.
fn flip_lowest_bit<S: Sample>(sample: S) -> S {
    let wide = sample.to_binary64().raw();
    let flipped = match S::PRECISION {
        Precision::Binary64 => f64::from_bits(wide.to_bits() ^ 1),
        Precision::Binary32 => {
            let narrow = <f32 as Delivered<Binary64>>::deliver(Binary64::from_raw(wide));
            f64::from(f32::from_bits(narrow.to_bits() ^ 1))
        }
    };
    S::from_binary64(Binary64::from_raw(flipped))
}

impl<S: Sample, const N: usize> Object<S, N> for Echo {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 multiplication, which neither overflows nor panics"
    )]
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        self.blocks = self.blocks.saturating_add(1);
        let gain = S::from_binary64(Binary64::from_raw(self.gain));
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                let wide = value.to_binary64().raw();
                let flip = self.quirky
                    && match self.quirk {
                        Quirk::Random => wide > 0.5_f64 && wide < 0.75_f64,
                        Quirk::LastBlock => self.blocks == BLOCKS,
                        Quirk::Special => {
                            wide.to_bits() == f64::MIN_POSITIVE.to_bits()
                                || wide.to_bits() == f64::from(f32::MIN_POSITIVE).to_bits()
                        }
                        Quirk::None
                        | Quirk::HighestRate
                        | Quirk::Binary32
                        | Quirk::Maximum
                        | Quirk::SubnormalGain => true,
                    };
                let product = *value * gain;
                *out = if flip {
                    flip_lowest_bit(product)
                } else {
                    product
                };
            }
        }
    }
}

impl<S: Sample, const N: usize> Driven<S, N, 1> for Echo {
    fn set(&mut self, [gain]: [Binary64; 1]) {
        self.gain = gain.raw();
    }

    /// Multiplies each frame by its gain, without its quirk: the battery drives no parameter.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 multiplication, which neither overflows nor panics"
    )]
    fn process_driven(
        &mut self,
        inputs: &[Block<S, N>],
        outputs: &mut [Block<S, N>],
        [gains]: &[Block<Binary64, N>; 1],
    ) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for ((out, value), &gain) in output.iter_mut().zip(input).zip(gains) {
                *out = *value * S::from_binary64(gain);
            }
        }
    }
}

/// The spec of an [`Echo`] with `quirk`.
fn echo(quirk: Quirk) -> ObjectSpec {
    echo_to(quirk, &[OUT])
}

/// The spec of an [`Echo`] with `quirk` and the outlets `outlets`.
fn echo_to(quirk: Quirk, outlets: &'static [PortSpec]) -> ObjectSpec {
    macro_rules! with {
        ($quirk:expr) => {{
            /// The box type of one quirk.
            struct Chosen;
            impl Create<1> for Chosen {
                const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
                const PARAMS: &'static [ParamSpec; 1] = &[GAIN];

                fn create<S: Sample, const N: usize>(
                    rate: SampleRate,
                    [gain]: [Binary64; 1],
                ) -> impl Driven<S, N, 1> + 'static {
                    let gain = gain.raw();
                    let quirky = match $quirk {
                        Quirk::None => false,
                        Quirk::HighestRate => rate.hz() == 384_000,
                        Quirk::Binary32 => S::PRECISION == Precision::Binary32,
                        Quirk::Maximum => gain.to_bits() == 1.0_f64.to_bits(),
                        Quirk::SubnormalGain => {
                            gain != 0.0_f64 && gain.abs() < f64::from(f32::MIN_POSITIVE)
                        }
                        Quirk::Random | Quirk::LastBlock | Quirk::Special => true,
                    };
                    Echo {
                        gain,
                        quirky,
                        quirk: $quirk,
                        blocks: 0,
                    }
                }
            }
            ObjectSpec::new::<Chosen, 1>(
                "echo~",
                "multiplies by a gain",
                &[IN],
                outlets,
                &[],
                AlgorithmVersion::FIRST,
            )
        }};
    }
    match quirk {
        Quirk::None => with!(Quirk::None),
        Quirk::HighestRate => with!(Quirk::HighestRate),
        Quirk::Binary32 => with!(Quirk::Binary32),
        Quirk::Maximum => with!(Quirk::Maximum),
        Quirk::Random => with!(Quirk::Random),
        Quirk::LastBlock => with!(Quirk::LastBlock),
        Quirk::Special => with!(Quirk::Special),
        Quirk::SubnormalGain => with!(Quirk::SubnormalGain),
    }
}

/// A box type `$name` of the one parameter `$param`, creating a plain [`Echo`].
macro_rules! gain_of {
    ($name:ident, $param:expr) => {
        /// A box type of one parameter of its own.
        struct $name;
        impl Create<1> for $name {
            const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
            const PARAMS: &'static [ParamSpec; 1] = &[$param];

            fn create<S: Sample, const N: usize>(
                _rate: SampleRate,
                [gain]: [Binary64; 1],
            ) -> impl Driven<S, N, 1> + 'static {
                Echo {
                    gain: gain.raw(),
                    quirky: false,
                    quirk: Quirk::None,
                    blocks: 0,
                }
            }
        }
    };
}

#[test]
fn is_64_lowercase_hex_digits_the_same_at_every_run() {
    let first = print(echo(Quirk::None));
    assert_eq!(first.len(), 64, "{first}");
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{first}"
    );
    assert_eq!(first, print(echo(Quirk::None)), "deterministic");
}

#[test]
fn sees_one_bit_changed_in_any_run_of_the_battery() {
    let plain = print(echo(Quirk::None));
    let mut seen = vec![plain.clone()];
    for quirk in [
        Quirk::HighestRate,
        Quirk::Binary32,
        Quirk::Maximum,
        Quirk::Random,
        Quirk::LastBlock,
        Quirk::Special,
    ] {
        let changed = print(echo(quirk));
        assert_ne!(changed, plain, "{quirk:?}");
        assert!(!seen.contains(&changed), "{quirk:?}: its own fingerprint");
        seen.push(changed);
    }
}

#[test]
fn hashes_the_battery_version_first() {
    // A box without outlet renders nothing: its fingerprint is the battery version alone,
    // as an independent SHA-256 of its four little-endian bytes gives it (Python's
    // hashlib over `02 00 00 00`).
    assert_eq!(FINGERPRINT_BATTERY, 2, "the second battery");
    let silent = echo_to(Quirk::None, &[]);
    assert_eq!(
        print(silent),
        "26b25d457597a7b0463f9620f666dd10aa2c4373a505967c7c8d70922a2d6ece"
    );
}

#[test]
fn draws_splitmix64_from_its_published_values() {
    // SplitMix64's first outputs from the state 0, computed apart from this code.
    let mut state = 0_u64;
    assert_eq!(
        [splitmix64(&mut state), splitmix64(&mut state)],
        [0xe220_a839_7b1d_cdaf, 0x6e78_9e6a_a1b9_65f4]
    );
}

#[test]
fn draws_the_seeded_signal_of_splitmix64_from_zero() {
    // The first outputs of SplitMix64 from the seed 0, their top 32 bits scaled to
    // [-1, 1): computed with Python, apart from this code.
    let signal = seeded_signal(1);
    assert_eq!(
        signal.first().and_then(|block| block.get(..4)),
        Some(
            &[
                0.766_621_616_203_337_9_f64,
                -0.136_944_006_197_154_52_f64,
                -0.947_132_457_047_700_9_f64,
                0.941_763_956_099_748_6_f64,
            ][..]
        )
    );
    assert_eq!(signal.len(), 1, "one block");
    assert!(seeded_signal(0).is_empty(), "none");
}

#[test]
fn tries_special_parameter_values_only_within_the_range() {
    const POSITIVE: ParamSpec = audio_core::param!("gain", "gain", 2.0, 1.0, 3.0);
    const NEGATIVE: ParamSpec = audio_core::param!("gain", "gain", -2.0, -3.0, -1.0);
    gain_of!(Gain, GAIN);
    gain_of!(Positive, POSITIVE);
    gain_of!(Negative, NEGATIVE);
    let wide = |value: f32| vec![f64::from(value).to_bits()];
    assert_eq!(
        given_sets::<Gain>(),
        [
            wide(0.0_f32),
            wide(0.5_f32),
            wide(0.75_f32),
            wide(1.0_f32),
            vec![0x8000_0000_0000_0000],
            vec![0x3810_0000_0000_0000],
        ],
        "[0, 1]: minimum, default, away from the default, maximum, -0.0, 2^-126"
    );
    assert_eq!(
        given_sets::<Positive>()[4..],
        [wide(2.0_f32), wide(2.0_f32)],
        "above zero: the default instead"
    );
    assert_eq!(
        given_sets::<Negative>()[4..],
        [wide(-2.0_f32), wide(-2.0_f32)],
        "below zero: the default instead"
    );
}

#[test]
fn tries_a_value_away_from_the_default() {
    // The midpoint of the range, unless it is the default: then halfway from the default
    // to the maximum.
    const QUARTER: ParamSpec = audio_core::param!("gain", "gain", 0.25, 0.0, 1.0);
    const CENTERED: ParamSpec = audio_core::param!("gain", "gain", 0.5, -1.0, 1.0);
    const ZERO: ParamSpec = audio_core::param!("gain", "gain", 0.0, -1.0, 1.0);
    gain_of!(Quarter, QUARTER);
    gain_of!(Gain, GAIN);
    gain_of!(Centered, CENTERED);
    gain_of!(Zero, ZERO);
    let wide = |value: f64| vec![value.to_bits()];
    assert_eq!(given_sets::<Quarter>()[2], wide(0.5_f64), "the midpoint");
    assert_eq!(
        given_sets::<Gain>()[2],
        wide(0.75_f64),
        "the default is the midpoint"
    );
    assert_eq!(
        given_sets::<Centered>()[2],
        wide(0.0_f64),
        "midpoint of [-1, 1]"
    );
    assert_eq!(
        given_sets::<Zero>()[2],
        wide(0.5_f64),
        "halfway from the default 0 to the maximum"
    );
}

#[test]
fn tries_a_value_away_from_the_default_even_just_above_the_smallest_normal() {
    // A range of a few binary32 units of the last place above 2^-126: halving its bounds is
    // exact in binary64, so the value away from the default is the midpoint, within it. A
    // range of one value has no other: its default.
    const ONE: f64 = f64::from_bits(0x3810_0000_2000_0000);
    const DEGENERATE: ParamSpec = audio_core::param!("gain", "gain", ONE, ONE, ONE);
    const NEGATIVE: ParamSpec = audio_core::param!("gain", "gain", -ONE, -ONE, -ONE);
    const TWO: f64 = f64::from_bits(0x3810_0000_4000_0000);
    const THREE: f64 = f64::from_bits(0x3810_0000_6000_0000);
    const NARROW: ParamSpec = audio_core::param!("gain", "gain", THREE, TWO, THREE);
    gain_of!(Degenerate, DEGENERATE);
    gain_of!(Negative, NEGATIVE);
    gain_of!(Narrow, NARROW);
    for (sets, away) in [
        (given_sets::<Degenerate>(), 0x3810_0000_2000_0000_u64),
        (given_sets::<Negative>(), 0xb810_0000_2000_0000),
        (given_sets::<Narrow>(), 0x3810_0000_5000_0000),
    ] {
        for set in &sets {
            assert_eq!(set.len(), 1, "one value per parameter");
        }
        assert_eq!(
            sets[2],
            [away],
            "{away:#x}: the value away from the default"
        );
    }
}

/// The bits of every parameter set the battery gives a box of the box type `T`, as the box
/// is given them.
fn given_sets<T: Create<1>>() -> Vec<Vec<u64>> {
    let spec: &'static ObjectSpec = Box::leak(Box::new(ObjectSpec::new::<T, 1>(
        "echo~",
        "multiplies by a gain",
        &[IN],
        &[OUT],
        &[],
        AlgorithmVersion::FIRST,
    )));
    param_sets(spec)
        .iter()
        .map(|set| {
            let given = ParamValues::of(spec, &set.params()).expect("a set of the box type");
            assert!(core::ptr::eq(set.spec(), spec), "of the box type");
            given.params().iter().map(|value| value.bits()).collect()
        })
        .collect()
}

#[test]
fn gives_the_parameters_as_a_plan_does() {
    // A box that changes only for a nonzero gain below 2^-126, which no parameter given as
    // `Params` is: no plan can reach that change, so its fingerprint is the plain one.
    assert_eq!(
        print(echo(Quirk::SubnormalGain)),
        print(echo(Quirk::None)),
        "no parameter below 2^-126 in magnitude but zero"
    );
}

/// Copies each inlet to an outlet: inlet 0 to outlet `first`, inlet 1 to the other.
#[derive(Debug)]
struct Route {
    /// The outlet inlet 0 is copied to.
    first: usize,
}

impl<S: Sample, const N: usize> Object<S, N> for Route {
    fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
        let order = if self.first == 0 { [0, 1] } else { [1, 0] };
        for (input, outlet) in inputs.iter().zip(order) {
            if let Some(output) = outputs.get_mut(outlet) {
                *output = *input;
            }
        }
    }
}

/// The spec of a [`Route`] of two inlets and two outlets, inlet 0 to outlet `FIRST`.
fn route<const FIRST: usize>() -> ObjectSpec {
    /// The box type routing inlet 0 to outlet `FIRST`.
    struct Routed<const FIRST: usize>;
    impl<const FIRST: usize> Create<0> for Routed<FIRST> {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec; 0] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            []: [Binary64; 0],
        ) -> impl Driven<S, N, 0> + 'static {
            Route { first: FIRST }
        }
    }
    ObjectSpec::new::<Routed<FIRST>, 0>(
        "route~",
        "copies each inlet to an outlet",
        &[IN, IN],
        &[OUT, OUT],
        &[],
        AlgorithmVersion::FIRST,
    )
}

#[test]
fn feeds_the_inlets_in_turn_and_hashes_the_outlets_in_order() {
    // Inlet 0 is given the first block of the signal, inlet 1 the next one, and the outputs
    // come outlet by outlet at each block.
    let input: Vec<Block<Binary64>> = seeded_signal(BLOCKS * 2)
        .iter()
        .map(|block| block.map(Binary64::from_raw))
        .collect();
    let mut straight = Route { first: 0 };
    let Ok(rendered) = run_blocks::<Binary64, Infallible>(
        &route::<0>(),
        &input,
        BLOCKS,
        Binary64::ZERO,
        |given, outputs| {
            straight.process(given, outputs);
            Ok(())
        },
    );
    assert_eq!(rendered.len(), BLOCKS * 2, "two outlets per block");
    for (index, block) in rendered.iter().enumerate() {
        assert!(
            input.get(index).is_some_and(|given| given == block),
            "block {index}: inlet by inlet, then outlet by outlet"
        );
    }
    assert_ne!(
        rendered.first(),
        rendered.get(1),
        "the two inlets are given different signals"
    );
    assert_ne!(
        print(route::<0>()),
        print(route::<1>()),
        "swapping the inlets, or the outlets, is seen"
    );
}

#[test]
fn sets_every_output_on_entry_to_every_block() {
    // Each outlet adds its inlet to what it holds on entry: 7 plus the inlet at every block,
    // never what the block before left. Inlet 0 is fed the first block of the input,
    // inlet 1 the next, then silence.
    let input: Vec<Block<Binary64>> = (0..3)
        .map(|block| {
            core::array::from_fn(|frame| {
                let index = block * BLOCK_SIZE + frame;
                Binary64::from_raw(f64::from(u32::try_from(index).expect("small")))
            })
        })
        .collect();
    let mut calls = 0_usize;
    let rendered = run_blocks::<Binary64, Infallible>(
        &route::<0>(),
        &input,
        3,
        Binary64::from_raw(7.0_f64),
        |given, outputs| {
            calls = calls.saturating_add(1);
            for (output, inlet) in outputs.iter_mut().zip(given) {
                for (out, value) in output.iter_mut().zip(inlet) {
                    *out = Binary64::from_raw(out.raw() + value.raw());
                }
            }
            Ok(())
        },
    );
    let expected: Vec<Vec<u64>> = [7.0_f64, 71.0_f64, 135.0_f64, 7.0_f64, 7.0_f64, 7.0_f64]
        .iter()
        .zip([1.0_f64, 1.0_f64, 1.0_f64, 0.0_f64, 0.0_f64, 0.0_f64])
        .map(|(&first, step)| {
            (0..BLOCK_SIZE)
                .map(|frame| {
                    (first + step * f64::from(u32::try_from(frame).expect("small"))).to_bits()
                })
                .collect()
        })
        .collect();
    assert_eq!(
        rendered.map(|blocks| {
            blocks
                .iter()
                .map(|block| block.iter().map(|sample| sample.bits()).collect())
                .collect::<Vec<Vec<u64>>>()
        }),
        Ok(expected),
        "7 on entry to each of the 3 blocks, inlet by inlet, then outlet by outlet"
    );
    assert_eq!(calls, 3, "one call per block");
}

#[test]
fn ends_a_run_at_the_first_error() {
    let mut calls = 0_usize;
    let rendered =
        run_blocks::<Binary32, usize>(&echo(Quirk::None), &[], BLOCKS, Binary32::ZERO, |_, _| {
            calls = calls.saturating_add(1);
            if calls == 2 { Err(calls) } else { Ok(()) }
        });
    assert_eq!(
        rendered.map(|blocks| blocks.len()),
        Err(2),
        "the error of the second block"
    );
    assert_eq!(calls, 2, "no block after it");
}

/// An instance type that creates every box through its factory, then flips the lowest bit
/// of the first sample of its first outlet at every block: an instance type that behaves
/// otherwise than the factory's, as a built-in arm could.
#[derive(Debug)]
struct Altered<S: Sample>(Box<dyn BoxInstance<S>>);

impl<S: Sample> Object<S> for Altered<S> {
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        self.0.process(inputs, outputs);
        for sample in outputs.iter_mut().flatten().take(1) {
            *sample = flip_lowest_bit(*sample);
        }
    }
}

impl<S: Sample> BoxInstance<S> for Altered<S> {
    fn receive(&mut self, values: &ParamValues) -> bool {
        self.0.receive(values)
    }

    fn run_driven(
        &mut self,
        inputs: &[Block<S>],
        outputs: &mut [Block<S>],
        values: &DrivenValues,
    ) -> bool {
        self.0.run_driven(inputs, outputs, values)
    }
}

impl<S: Sample> Dispatch<S> for Altered<S> {
    fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
        <Box<dyn BoxInstance<S>> as Dispatch<S>>::create(rate, values).map(Self)
    }
}

#[test]
fn measures_the_instances_of_each_precision() {
    // Each precision through the instance type given for it: an instance type behaving
    // otherwise in one precision is seen there, so a fingerprint taken through the factory
    // for one precision, whatever the instance type given, would fail here.
    let spec: &'static ObjectSpec = Box::leak(Box::new(echo(Quirk::None)));
    let through = |altered: [bool; 2]| {
        match altered {
            [false, false] => fingerprint::<Virtual<Binary64>, Virtual<Binary32>>(spec),
            [true, false] => fingerprint::<Altered<Binary64>, Virtual<Binary32>>(spec),
            [false, true] => fingerprint::<Virtual<Binary64>, Altered<Binary32>>(spec),
            [true, true] => fingerprint::<Altered<Binary64>, Altered<Binary32>>(spec),
        }
        .expect("a test build creates every box type")
    };
    let prints: Vec<String> = [[false, false], [true, false], [false, true], [true, true]]
        .into_iter()
        .map(through)
        .collect();
    let distinct: alloc::collections::BTreeSet<&String> = prints.iter().collect();
    assert_eq!(
        distinct.len(),
        4,
        "binary64 and binary32 each measured through their own instance type: {prints:?}"
    );
}

#[test]
fn pins_the_inputs_of_the_battery() {
    // A digest of every input the battery reads, written for battery 2: changing one of
    // them without raising FINGERPRINT_BATTERY (and this digest) fails here, rather than
    // as a box's outputs changed at the same version.
    let mut hasher = Sha256::new();
    for precision in crate::matrix::PRECISIONS {
        hasher.update(precision.bits().to_le_bytes());
    }
    for rate in crate::matrix::sample_rates() {
        hasher.update(rate.hz().to_le_bytes());
    }
    for count in [BLOCKS, BLOCK_SIZE] {
        hasher.update(u64::try_from(count).expect("small").to_le_bytes());
    }
    for value in crate::special_input::<Binary64>(1)[0].iter().take(8) {
        hasher.update(value.bits().to_le_bytes());
    }
    for value in crate::special_input::<Binary32>(1)[0].iter().take(8) {
        hasher.update(value.bits().to_le_bytes());
    }
    for spec in [echo(Quirk::None), route::<0>()] {
        for set in param_sets(Box::leak(Box::new(spec))) {
            hasher.update(
                u64::try_from(set.params().len())
                    .expect("small")
                    .to_le_bytes(),
            );
            for &value in set.params().iter() {
                // Each a binary32 value widened exactly: hashed as its binary32 bits.
                let narrow = u32::try_from(Binary32::from_binary64(value).bits()).expect("32 bits");
                hasher.update(narrow.to_le_bytes());
            }
        }
    }
    for value in seeded_signal(BLOCKS).iter().flatten() {
        hasher.update(value.to_bits().to_le_bytes());
    }
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _written = core::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
            text
        });
    assert_eq!(FINGERPRINT_BATTERY, 2, "the battery this digest is of");
    assert_eq!(
        digest, "f3ae291af4c69bc1fc6751206f4817706b7bb1f27b7939af747aa96a4dcabae0",
        "the battery's inputs changed: raise FINGERPRINT_BATTERY and write this digest"
    );
}
