//! Tests of the laws: each law must catch a box that breaks it.

use core::hint::black_box;
use core::sync::atomic::{AtomicU16, Ordering};

use engine_core::{Block, Object, ObjectSpec, ParamSpec, PortSpec, SampleRate};

use super::{check_laws, check_metadata, check_registry};

const PORT: PortSpec = PortSpec {
    name: "in",
    description: "a signal",
};
const OUT: PortSpec = PortSpec {
    name: "out",
    description: "a signal",
};
const GAIN: ParamSpec = ParamSpec {
    name: "gain",
    description: "gain",
    default: 0.5,
    min: 0.0,
    max: 1.0,
};

/// Multiplies its input by a gain, flushing subnormal products: respects every law.
#[derive(Debug)]
struct Gain(f32);
impl Object for Gain {
    fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                *out = engine_core::flush(value * self.0);
            }
        }
    }
}

const GOOD: ObjectSpec = ObjectSpec {
    name: "gain~",
    description: "multiplies by a gain",
    inlets: &[PORT],
    outlets: &[OUT],
    params: &[GAIN],
    capabilities: &[],
    create: |_, params| Box::new(Gain(params.first().copied().unwrap_or(1.0))),
};

/// Outputs NaN.
#[derive(Debug)]
struct Nan;
impl Object for Nan {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs {
            output.fill(f32::NAN);
        }
    }
}

/// Outputs a value that differs for every instance.
#[derive(Debug)]
struct Counter(f32);
impl Object for Counter {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs {
            output.fill(self.0);
        }
    }
}
static INSTANCES: AtomicU16 = AtomicU16::new(0);

/// Allocates and frees while processing.
#[derive(Debug)]
struct Allocating;
impl Object for Allocating {
    fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {
        drop(black_box(Vec::<u8>::with_capacity(8)));
    }
}

/// Frees memory allocated when it was created, on its first call only: it never
/// allocates while processing.
#[derive(Debug)]
struct Freeing(Option<Vec<u8>>);
impl Object for Freeing {
    fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {
        drop(black_box(self.0.take()));
    }
}

fn spec_with(create: engine_core::Factory) -> ObjectSpec {
    ObjectSpec { create, ..GOOD }
}

#[test]
fn a_correct_box_passes_every_law() {
    assert_eq!(check_laws(&GOOD), Ok(()), "gain~ respects the laws");
    let no_ports = ObjectSpec {
        inlets: &[],
        outlets: &[],
        params: &[],
        ..GOOD
    };
    assert_eq!(check_laws(&no_ports), Ok(()), "a box without ports");
}

#[test]
fn catches_non_finite_output() {
    let error = check_laws(&spec_with(|_, _| Box::new(Nan))).expect_err("NaN is caught");
    assert!(
        error.contains("non-finite output with parameters [0.0]"),
        "{error}"
    );
}

#[test]
fn catches_subnormal_output() {
    // A constant subnormal output, of either sign: finite and deterministic, but not
    // flushed.
    let positive = check_laws(&spec_with(|_, _| Box::new(Counter(f32::from_bits(1)))))
        .expect_err("a positive subnormal is caught");
    assert!(
        positive.contains("subnormal output with parameters [0.0]"),
        "{positive}"
    );
    let negative = check_laws(&spec_with(|_, _| {
        Box::new(Counter(f32::from_bits(0x8040_0000)))
    }))
    .expect_err("a negative subnormal is caught");
    assert!(
        negative.contains("subnormal output with parameters [0.0]"),
        "{negative}"
    );
    // The smallest normal and zeros of both signs are not subnormal.
    for value in [f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 0.0, -0.0] {
        let spec = ObjectSpec {
            inlets: &[],
            params: &[],
            create: match value.to_bits() {
                0x0080_0000 => |_, _| Box::new(Counter(f32::MIN_POSITIVE)),
                0x8080_0000 => |_, _| Box::new(Counter(-f32::MIN_POSITIVE)),
                0x0000_0000 => |_, _| Box::new(Counter(0.0)),
                _ => |_, _| Box::new(Counter(-0.0)),
            },
            ..GOOD
        };
        assert_eq!(check_laws(&spec), Ok(()), "{value:e} is not subnormal");
    }
}

#[test]
fn catches_non_deterministic_output() {
    let spec = spec_with(|_, _| {
        let instance = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Box::new(Counter(f32::from(instance % 1000)))
    });
    let error = check_laws(&spec).expect_err("instances differ");
    assert!(
        error.contains("not deterministic with parameters [0.0]"),
        "{error}"
    );
}

#[test]
fn catches_allocation_while_processing() {
    let error = check_laws(&spec_with(|_, _| Box::new(Allocating))).expect_err("allocation");
    assert!(
        error.contains("uses the heap while processing (16 heap operations)"),
        "{error}"
    );
}

#[test]
fn catches_a_release_while_processing() {
    let spec = spec_with(|_, _| Box::new(Freeing(Some(vec![0_u8; 8]))));
    let error = check_laws(&spec).expect_err("a release");
    assert!(
        error.contains("uses the heap while processing (1 heap operations)"),
        "{error}"
    );
}

#[test]
fn checks_parameters_at_negative_zero_and_the_smallest_subnormal_and_normal() {
    // NaN only for a parameter of -0.0, then only for a subnormal parameter, then only for
    // the smallest normal: none is a minimum, default, midpoint or maximum.
    let negative_zero = spec_with(|_, params| {
        if params.first().map(|value| value.to_bits()) == Some(0x8000_0000) {
            Box::new(Nan)
        } else {
            Box::new(Gain(1.0))
        }
    });
    let error = check_laws(&negative_zero).expect_err("-0.0 is tried");
    assert!(error.contains("with parameters [-0.0]"), "{error}");
    let subnormal = spec_with(|_, params| {
        if params.first().is_some_and(|value| value.is_subnormal()) {
            Box::new(Nan)
        } else {
            Box::new(Gain(1.0))
        }
    });
    let failure = check_laws(&subnormal).expect_err("a subnormal is tried");
    assert!(failure.contains("with parameters [1e-45]"), "{failure}");
    let normal = spec_with(|_, params| {
        if params.first().map(|value| value.to_bits()) == Some(0x0080_0000) {
            Box::new(Nan)
        } else {
            Box::new(Gain(1.0))
        }
    });
    let smallest = check_laws(&normal).expect_err("the smallest normal is tried");
    assert!(
        smallest.contains("with parameters [1.1754944e-38]"),
        "{smallest}"
    );
}

#[test]
fn tries_special_parameter_values_only_within_the_range() {
    const POSITIVE: ParamSpec = ParamSpec {
        default: 2.0,
        min: 1.0,
        max: 3.0,
        ..GAIN
    };
    const NEGATIVE: ParamSpec = ParamSpec {
        default: -2.0,
        min: -3.0,
        max: -1.0,
        ..GAIN
    };
    let bits = |param: &'static ParamSpec| {
        super::param_sets(&ObjectSpec {
            params: core::slice::from_ref(param),
            ..GOOD
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
fn tries_every_case_at_every_rate_with_seven_parameter_sets() {
    static CREATED: AtomicU16 = AtomicU16::new(0);
    let spec = spec_with(|_, _| {
        let _: u16 = CREATED.fetch_add(1, Ordering::Relaxed);
        Box::new(Gain(1.0))
    });
    assert_eq!(check_laws(&spec), Ok(()), "a correct box");
    // Per rate: (special inputs + 32 random cases) x 7 parameter sets x 2 runs, plus a
    // long run per parameter set.
    assert_eq!(
        usize::from(CREATED.load(Ordering::Relaxed)),
        crate::matrix::SAMPLE_RATES.len() * ((1 + 32) * 7 * 2 + 7),
        "every rate, case and parameter set"
    );
}

#[test]
fn feeds_inputs_across_the_whole_range() {
    // NaN as soon as an input is negative: the law must try negative inputs.
    #[derive(Debug)]
    struct NegativeIsNan;
    impl Object for NegativeIsNan {
        fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
            let negative = inputs.iter().flatten().any(|sample| *sample < 0.0);
            for output in outputs {
                output.fill(if negative { f32::NAN } else { 0.0 });
            }
        }
    }
    let error = check_laws(&spec_with(|_, _| Box::new(NegativeIsNan))).expect_err("negatives");
    assert!(error.contains("non-finite output"), "{error}");
}

#[test]
fn checks_parameters_at_minimum_default_and_maximum() {
    // NaN only at the maximum gain: the law must try the maximum too.
    let spec = spec_with(|_, params| {
        if params.first().copied() == Some(1.0) {
            Box::new(Nan)
        } else {
            Box::new(Gain(1.0))
        }
    });
    let error = check_laws(&spec).expect_err("maximum is checked");
    assert!(
        error.contains("non-finite output with parameters [1.0]"),
        "{error}"
    );
}

#[test]
fn feeds_inputs_to_every_inlet_and_reads_every_outlet() {
    // NaN only when the second inlet carries a non-zero signal.
    #[derive(Debug)]
    struct SecondInlet;
    impl Object for SecondInlet {
        fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
            let second = inputs
                .get(1)
                .is_some_and(|block| block.iter().any(|sample| *sample != 0.0));
            for output in outputs.iter_mut().skip(1) {
                output.fill(if second { f32::NAN } else { 0.0 });
            }
        }
    }
    const TWO: [PortSpec; 2] = [
        PortSpec {
            name: "a",
            description: "a",
        },
        PortSpec {
            name: "b",
            description: "b",
        },
    ];
    let spec = ObjectSpec {
        inlets: &TWO,
        outlets: &TWO,
        params: &[],
        create: |_, _| Box::new(SecondInlet),
        ..GOOD
    };
    let error = check_laws(&spec).expect_err("the second inlet and outlet are exercised");
    assert!(error.contains("non-finite output"), "{error}");
}

#[test]
fn checks_every_sample_rate_of_the_matrix() {
    // NaN at every rate but 48 kHz: the laws must try the other rates.
    #[derive(Debug)]
    struct Rate(f32);
    impl Object for Rate {
        fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
            for output in outputs {
                output.fill(self.0);
            }
        }
    }
    let spec = spec_with(|rate: SampleRate, _| {
        Box::new(Rate(if rate.hz() == 48_000 { 0.0 } else { f32::NAN }))
    });
    let error = check_laws(&spec).expect_err("other rates are checked");
    assert!(error.starts_with("at 8000 Hz: "), "{error}");
    for hz in crate::matrix::SAMPLE_RATES {
        let only = spec_with(|rate: SampleRate, _| {
            Box::new(Rate(if rate.hz() == ONLY.load(Ordering::Relaxed) {
                f32::NAN
            } else {
                0.0
            }))
        });
        ONLY.store(hz, Ordering::Relaxed);
        let failure = check_laws(&only).expect_err("each rate is checked");
        assert!(failure.starts_with(&format!("at {hz} Hz: ")), "{failure}");
    }
}

/// The only rate at which a fixture box fails, set by the test.
static ONLY: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[test]
fn feeds_the_special_inputs() {
    // NaN only on a negative zero or a subnormal input: random inputs never hit them.
    #[derive(Debug)]
    struct Special;
    impl Object for Special {
        fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
            let special = inputs
                .iter()
                .flatten()
                .any(|sample| sample.is_subnormal() || sample.to_bits() == (-0.0_f32).to_bits());
            for output in outputs {
                output.fill(if special { f32::NAN } else { 0.0 });
            }
        }
    }
    let error = check_laws(&spec_with(|_, _| Box::new(Special))).expect_err("special inputs");
    assert!(
        error.contains("special inputs: non-finite output"),
        "{error}"
    );
}

#[test]
fn feeds_random_inputs() {
    // NaN only on inputs strictly between 0.25 and 0.75: no special input, no silence,
    // only the random inputs reach it.
    #[derive(Debug)]
    struct Middle;
    impl Object for Middle {
        fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
            let middle = inputs
                .iter()
                .flatten()
                .any(|sample| *sample > 0.25 && *sample < 0.75);
            for output in outputs {
                output.fill(if middle { f32::NAN } else { 0.0 });
            }
        }
    }
    let error = check_laws(&spec_with(|_, _| Box::new(Middle))).expect_err("random inputs");
    assert!(error.starts_with("at 8000 Hz: "), "{error}");
    assert!(error.contains("non-finite output"), "{error}");
    assert!(
        !error.contains("special inputs") && !error.contains("long run"),
        "found by the random inputs: {error}"
    );
}

#[test]
fn runs_long_enough_to_reveal_late_failures() {
    // Breaks only after 512 blocks: the short runs never see it.
    #[derive(Debug)]
    struct Drifts(u32);
    impl Object for Drifts {
        fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
            self.0 = self.0.saturating_add(1);
            for output in outputs {
                output.fill(if self.0 > 512 { f32::INFINITY } else { 0.0 });
            }
        }
    }
    let error = check_laws(&spec_with(|_, _| Box::new(Drifts(0)))).expect_err("long run");
    assert!(error.contains("long run: non-finite output"), "{error}");
}

#[test]
fn checks_parameters_away_from_their_default() {
    // NaN only at 0.5, away from the default 0.25 and from both limits.
    const QUARTER: ParamSpec = ParamSpec {
        default: 0.25,
        ..GAIN
    };
    let spec = ObjectSpec {
        params: &[QUARTER],
        create: |_, params| {
            if params.first().copied() == Some(0.5) {
                Box::new(Nan)
            } else {
                Box::new(Gain(1.0))
            }
        },
        ..GOOD
    };
    let error = check_laws(&spec).expect_err("the midpoint is checked");
    assert!(error.contains("with parameters [0.5]"), "{error}");
    // With the default at the midpoint (GAIN: 0.5 in [0, 1]), 0.75 is tried instead.
    let shifted = spec_with(|_, params| {
        if params.first().copied() == Some(0.75) {
            Box::new(Nan)
        } else {
            Box::new(Gain(1.0))
        }
    });
    let failure = check_laws(&shifted).expect_err("a value away from the default");
    assert!(failure.contains("with parameters [0.75]"), "{failure}");
    // A range not starting at zero: the midpoint of [-1, 1] is 0.
    let centered = ParamSpec {
        default: 0.5,
        min: -1.0,
        max: 1.0,
        ..GAIN
    };
    assert_eq!(
        super::away_from_default(&centered).to_bits(),
        0.0_f32.to_bits(),
        "midpoint of [-1, 1]"
    );
    assert_eq!(
        super::away_from_default(&ParamSpec {
            default: 0.0,
            ..centered
        }),
        0.5,
        "halfway from the default 0 to the maximum"
    );
}

/// Fixtures for the metadata cases.
static DUP_PORTS: [PortSpec; 2] = [PORT, PORT];
static DUP_PARAMS: [ParamSpec; 2] = [GAIN, GAIN];
static UNDOCUMENTED_PORT: [PortSpec; 1] = [PortSpec {
    name: "x",
    description: " ",
}];
static EMPTY_PORT: [PortSpec; 1] = [PortSpec {
    name: "",
    description: "x",
}];

/// Specs with one metadata problem each, and the expected message.
static INVALID_METADATA: [(ObjectSpec, &str); 13] = [
    (ObjectSpec { name: "", ..GOOD }, "invalid box name \"\""),
    (
        ObjectSpec {
            name: "two words",
            ..GOOD
        },
        "invalid box name \"two words\"",
    ),
    (
        ObjectSpec {
            description: "  ",
            ..GOOD
        },
        "missing description",
    ),
    (
        ObjectSpec {
            inlets: &DUP_PORTS,
            ..GOOD
        },
        "duplicate inlet \"in\"",
    ),
    (
        ObjectSpec {
            outlets: &DUP_PORTS,
            ..GOOD
        },
        "duplicate outlet \"in\"",
    ),
    (
        ObjectSpec {
            inlets: &EMPTY_PORT,
            ..GOOD
        },
        "inlet with an empty name",
    ),
    (
        ObjectSpec {
            params: &DUP_PARAMS,
            ..GOOD
        },
        "duplicate parameter \"gain\"",
    ),
    (
        ObjectSpec {
            inlets: &UNDOCUMENTED_PORT,
            ..GOOD
        },
        "port \"x\" has no description",
    ),
    (
        ObjectSpec {
            outlets: &UNDOCUMENTED_PORT,
            ..GOOD
        },
        "port \"x\" has no description",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec {
                description: "",
                ..GAIN
            }],
            ..GOOD
        },
        "parameter \"gain\" has no description",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec {
                default: 2.0,
                ..GAIN
            }],
            ..GOOD
        },
        "parameter \"gain\": default 2 outside finite range [0, 1]",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec {
                min: f32::NEG_INFINITY,
                ..GAIN
            }],
            ..GOOD
        },
        "parameter \"gain\": default 0.5 outside finite range [-inf, 1]",
    ),
    (
        ObjectSpec {
            params: &[ParamSpec {
                max: f32::INFINITY,
                ..GAIN
            }],
            ..GOOD
        },
        "parameter \"gain\": default 0.5 outside finite range [0, inf]",
    ),
];

#[test]
fn rejects_invalid_metadata() {
    for (spec, message) in INVALID_METADATA {
        assert_eq!(check_metadata(&spec), Err(message.to_owned()), "{message}");
        assert_eq!(
            check_laws(&spec),
            Err(message.to_owned()),
            "laws check metadata first"
        );
    }
    assert_eq!(check_metadata(&GOOD), Ok(()), "valid metadata");
}

#[test]
fn checks_a_registry() {
    let other = ObjectSpec {
        name: "other~",
        ..GOOD
    };
    assert_eq!(check_registry(&[&GOOD, &other]), Ok(()), "valid registry");
    assert_eq!(check_registry(&[]), Ok(()), "empty registry");
    assert_eq!(
        check_registry(&[&GOOD, &GOOD]),
        Err("duplicate box \"gain~\"".to_owned()),
        "unique names"
    );
    let bad = ObjectSpec {
        description: "",
        ..other
    };
    assert_eq!(
        check_registry(&[&GOOD, &bad]),
        Err("other~: missing description".to_owned()),
        "every box is checked"
    );
}
