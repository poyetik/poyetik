//! Tests of the fingerprint: stable for the same bits, changed by a bit of any run of the
//! battery.

use engine_core::host::Raw;
use engine_core::{
    AlgorithmVersion, Binary64, Block, Create, Factory, Object, ObjectSpec, ParamSpec, PortSpec,
    Precision, Sample, SampleRate,
};

use super::{FINGERPRINT_BATTERY, fingerprint, seeded_signal};
use crate::inputs::param_sets;

const IN: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};
const GAIN: ParamSpec = ParamSpec::new("gain", "gain", 0.5, 0.0, 1.0);

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
}

/// Multiplies its input by its gain, then adds a quarter to its output where its quirk
/// says.
#[derive(Debug)]
struct Echo {
    /// Its gain.
    gain: f64,
    /// Whether its quirk applies to this instance (rate, precision, parameters).
    quirky: bool,
    /// Its quirk.
    quirk: Quirk,
    /// The blocks processed so far.
    blocks: u32,
}

impl<S: Sample> Object<S> for Echo {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 multiplication, which neither overflows nor panics"
    )]
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        self.blocks = self.blocks.saturating_add(1);
        let gain = S::from_binary64(Binary64::from_raw(self.gain));
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                let wide = value.to_binary64().raw();
                let flip = self.quirky
                    && match self.quirk {
                        Quirk::Random => wide > 0.5_f64 && wide < 0.75_f64,
                        Quirk::LastBlock => self.blocks == 8,
                        Quirk::Special => {
                            wide.to_bits() == f64::MIN_POSITIVE.to_bits()
                                || wide.to_bits() == f64::from(f32::MIN_POSITIVE).to_bits()
                        }
                        Quirk::None | Quirk::HighestRate | Quirk::Binary32 | Quirk::Maximum => true,
                    };
                let product = (*value * gain).to_binary64().raw();
                // A quarter: seen in either precision.
                let changed = if flip { product + 0.25_f64 } else { product };
                *out = S::from_binary64(Binary64::from_raw(changed));
            }
        }
    }
}

/// The spec of an [`Echo`] with `quirk`.
fn echo(quirk: Quirk) -> ObjectSpec {
    macro_rules! with {
        ($quirk:expr) => {{
            /// The box type of one quirk.
            struct Chosen;
            impl Create for Chosen {
                fn create<S: Sample>(rate: SampleRate, params: &[Binary64]) -> Box<dyn Object<S>> {
                    let gain = params.first().map_or(0.0_f64, |param| param.raw());
                    let quirky = match $quirk {
                        Quirk::None => false,
                        Quirk::HighestRate => rate.hz() == 384_000,
                        Quirk::Binary32 => S::PRECISION == Precision::Binary32,
                        Quirk::Maximum => gain.to_bits() == 1.0_f64.to_bits(),
                        Quirk::Random | Quirk::LastBlock | Quirk::Special => true,
                    };
                    Box::new(Echo {
                        gain,
                        quirky,
                        quirk: $quirk,
                        blocks: 0,
                    })
                }
            }
            Factory::new::<Chosen>()
        }};
    }
    let create = match quirk {
        Quirk::None => with!(Quirk::None),
        Quirk::HighestRate => with!(Quirk::HighestRate),
        Quirk::Binary32 => with!(Quirk::Binary32),
        Quirk::Maximum => with!(Quirk::Maximum),
        Quirk::Random => with!(Quirk::Random),
        Quirk::LastBlock => with!(Quirk::LastBlock),
        Quirk::Special => with!(Quirk::Special),
    };
    ObjectSpec {
        name: "echo~",
        description: "multiplies by a gain",
        inlets: &[IN],
        outlets: &[OUT],
        params: &[GAIN],
        capabilities: &[],
        create,
        algorithm_version: AlgorithmVersion::FIRST,
    }
}

#[test]
fn is_64_lowercase_hex_digits_the_same_at_every_run() {
    let first = fingerprint(&echo(Quirk::None));
    assert_eq!(first.len(), 64, "{first}");
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{first}"
    );
    assert_eq!(first, fingerprint(&echo(Quirk::None)), "deterministic");
}

#[test]
fn sees_one_bit_changed_in_any_run_of_the_battery() {
    let plain = fingerprint(&echo(Quirk::None));
    let mut seen = vec![plain.clone()];
    for quirk in [
        Quirk::HighestRate,
        Quirk::Binary32,
        Quirk::Maximum,
        Quirk::Random,
        Quirk::LastBlock,
        Quirk::Special,
    ] {
        let changed = fingerprint(&echo(quirk));
        assert_ne!(changed, plain, "{quirk:?}");
        assert!(!seen.contains(&changed), "{quirk:?}: its own fingerprint");
        seen.push(changed);
    }
}

#[test]
fn hashes_the_battery_version_first() {
    // A box without outlet renders nothing: its fingerprint is the battery version alone,
    // as an independent SHA-256 of its four little-endian bytes gives it (Python's
    // hashlib over `01 00 00 00`).
    assert_eq!(FINGERPRINT_BATTERY, 1, "the first battery");
    let silent = ObjectSpec {
        outlets: &[],
        ..echo(Quirk::None)
    };
    assert_eq!(
        fingerprint(&silent),
        "67abdd721024f0ff4e0b3f4c2fc13bc5bad42d0b7851d456d88d203d15aaa450"
    );
}

#[test]
fn draws_the_seeded_signal_of_splitmix64_from_zero() {
    // The first outputs of SplitMix64 from the seed 0, their top 32 bits scaled to
    // [-1, 1): computed with Python, apart from this code.
    assert_eq!(
        seeded_signal(4),
        [
            0.766_621_616_203_337_9_f64,
            -0.136_944_006_197_154_52_f64,
            -0.947_132_457_047_700_9_f64,
            0.941_763_956_099_748_6_f64,
        ]
    );
    assert!(seeded_signal(0).is_empty(), "none");
}

#[test]
fn tries_special_parameter_values_only_within_the_range() {
    const POSITIVE: ParamSpec = ParamSpec::new("gain", "gain", 2.0, 1.0, 3.0);
    const NEGATIVE: ParamSpec = ParamSpec::new("gain", "gain", -2.0, -3.0, -1.0);
    let bits = |param: &'static ParamSpec| {
        param_sets(&ObjectSpec {
            params: core::slice::from_ref(param),
            ..echo(Quirk::None)
        })
        .map(|set| set.iter().map(|value| value.to_bits()).collect::<Vec<_>>())
    };
    assert_eq!(
        bits(&GAIN),
        [
            vec![0.0_f32.to_bits()],
            vec![0.5_f32.to_bits()],
            vec![0.75_f32.to_bits()],
            vec![1.0_f32.to_bits()],
            vec![0x8000_0000],
            vec![0x0000_0001],
            vec![0x0080_0000],
        ],
        "[0, 1]: minimum, default, away from the default, maximum, -0.0, 1e-45, 2^-126"
    );
    assert_eq!(
        bits(&POSITIVE)[4..],
        [
            vec![2.0_f32.to_bits()],
            vec![2.0_f32.to_bits()],
            vec![2.0_f32.to_bits()]
        ],
        "above zero: the default instead"
    );
    assert_eq!(
        bits(&NEGATIVE)[4..],
        [
            vec![(-2.0_f32).to_bits()],
            vec![(-2.0_f32).to_bits()],
            vec![(-2.0_f32).to_bits()]
        ],
        "below zero: the default instead"
    );
}

#[test]
fn tries_a_value_away_from_the_default() {
    // The midpoint of the range, unless it is the default: then halfway from the default
    // to the maximum.
    const QUARTER: ParamSpec = ParamSpec::new("gain", "gain", 0.25, 0.0, 1.0);
    const CENTERED: ParamSpec = ParamSpec::new("gain", "gain", 0.5, -1.0, 1.0);
    const ZERO: ParamSpec = ParamSpec::new("gain", "gain", 0.0, -1.0, 1.0);
    let away = |param: &'static ParamSpec| {
        let [_, _, away, ..] = param_sets(&ObjectSpec {
            params: core::slice::from_ref(param),
            ..echo(Quirk::None)
        });
        away.iter().map(|value| value.to_bits()).collect::<Vec<_>>()
    };
    assert_eq!(away(&QUARTER), [0.5_f32.to_bits()], "the midpoint");
    assert_eq!(
        away(&GAIN),
        [0.75_f32.to_bits()],
        "the default is the midpoint"
    );
    assert_eq!(away(&CENTERED), [0.0_f32.to_bits()], "midpoint of [-1, 1]");
    assert_eq!(
        away(&ZERO),
        [0.5_f32.to_bits()],
        "halfway from the default 0 to the maximum"
    );
}
