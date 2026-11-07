//! Tests of the laws: each law must catch a box that breaks it.

use core::hint::black_box;
use core::sync::atomic::{AtomicU16, Ordering};

use engine_core::{BLOCK_SIZE, Block, Object, ObjectSpec, ParamSpec, PortSpec, SampleRate};

use super::{check_laws, check_metadata, check_registry};
use crate::allocations_during;

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

/// Multiplies its input by a gain: respects every law.
#[derive(Debug)]
struct Gain(f32);
impl Object for Gain {
    fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            for (out, value) in output.iter_mut().zip(input) {
                *out = value * self.0;
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

/// Allocates while processing.
#[derive(Debug)]
struct Allocating;
impl Object for Allocating {
    fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {
        drop(black_box(Vec::<u8>::with_capacity(8)));
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
fn catches_non_deterministic_output() {
    let spec = spec_with(|_, _| {
        let n = INSTANCES.fetch_add(1, Ordering::Relaxed);
        Box::new(Counter(f32::from(n % 1000)))
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
        error.contains("allocates while processing (8 allocations)"),
        "{error}"
    );
}

#[test]
fn tries_every_case_at_every_rate_with_four_parameter_sets() {
    static CREATED: AtomicU16 = AtomicU16::new(0);
    let spec = spec_with(|_, _| {
        let _: u16 = CREATED.fetch_add(1, Ordering::Relaxed);
        Box::new(Gain(1.0))
    });
    assert_eq!(check_laws(&spec), Ok(()), "a correct box");
    // Per rate: (special inputs + 32 random cases) x 4 parameter sets x 2 runs, plus a
    // long run per parameter set.
    assert_eq!(
        usize::from(CREATED.load(Ordering::Relaxed)),
        crate::matrix::SAMPLE_RATES.len() * ((1 + 32) * 4 * 2 + 4),
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
            let negative = inputs.iter().flatten().any(|v| *v < 0.0);
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
            let second = inputs.get(1).is_some_and(|b| b.iter().any(|v| *v != 0.0));
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
                .any(|v| v.is_subnormal() || v.to_bits() == (-0.0_f32).to_bits());
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
            let middle = inputs.iter().flatten().any(|v| *v > 0.25 && *v < 0.75);
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
fn runs_long_enough_to_reveal_drift() {
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

#[test]
fn counts_allocations_but_not_deallocations() {
    let ((), boxed) = allocations_during(|| drop(black_box(Box::new(1_u64))));
    assert_eq!(boxed, 1, "alloc");
    let ((), zeroed) = allocations_during(|| drop(black_box(vec![0_u8; 32])));
    assert_eq!(zeroed, 1, "alloc_zeroed");
    let mut grown = Vec::<u8>::with_capacity(1);
    grown.push(1);
    let ((), resized) = allocations_during(|| {
        grown.extend_from_slice(&[0; BLOCK_SIZE]);
        let _: &Vec<u8> = black_box(&grown);
    });
    assert_eq!(resized, 1, "realloc");
    let prebuilt = Box::new(2_u64);
    let ((), freed) = allocations_during(|| drop(black_box(prebuilt)));
    assert_eq!(freed, 0, "dealloc is not an allocation");
    let (value, none) = allocations_during(|| 6_i32 * 7_i32);
    assert_eq!((value, none), (42_i32, 0_usize), "returns the result");
}

#[test]
fn refuses_to_check_without_counted_allocations() {
    assert_eq!(
        super::require_counting(0),
        Err("CountingAllocator is not installed as the global allocator".to_owned()),
        "no vacuous pass"
    );
    assert_eq!(super::require_counting(1), Ok(()), "counted");
}
