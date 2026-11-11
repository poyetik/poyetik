//! Tests of plan building and execution, using test-only boxes, each run in both precisions
//! (`in_both_precisions!`): every instantiation of the generic plan is covered on its own.

use crate::tests::in_both_precisions;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::Feed;
use crate::Params;
use crate::host::{Delivered, Raw};
use crate::{
    AlgorithmVersion, BLOCK_SIZE, Binary32, Binary64, Block, Capability, Connection, CoreError,
    Create, Factory, Node, NodeId, Object, ObjectSpec, ParamSpec, Patch, Plan, PortSpec, Precision,
    Renderer, Sample, SampleRate,
};

const PORT: PortSpec = PortSpec {
    name: "x",
    description: "test port",
};

/// The samples these tests read the bits of, in either precision, rendered as their own
/// floats.
trait Bits: Raw<Float: Delivered<Self> + PartialEq + core::fmt::Debug> {}
impl Bits for Binary64 {}
impl Bits for Binary32 {}

/// `value`, a literal, in the precision `S`.
fn of<S: Sample>(value: f64) -> S {
    S::from_binary64(Binary64::new(value))
}

/// Outputs a constant value: its parameter, in the precision of the render.
#[derive(Debug)]
struct Const(Binary64);
impl<S: Sample> Object<S> for Const {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs {
            output.fill(S::from_binary64(self.0));
        }
    }
}
impl Create for Const {
    fn create<S: Sample>(_rate: SampleRate, params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self(params.first().copied().unwrap_or(Binary64::ONE)))
    }
}
static CONST: ObjectSpec = ObjectSpec {
    name: "const~",
    description: "constant",
    inlets: &[],
    outlets: &[PORT],
    params: &[ParamSpec::new("value", "value", 1.0, -10.0, 10.0)],
    capabilities: &[],
    create: Factory::new::<Const>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Outputs 0, 1, 2… continuing across blocks: checks that state is kept.
#[derive(Debug)]
struct Ramp(f64);
impl<S: Sample> Object<S> for Ramp {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs {
            for sample in output.iter_mut() {
                *sample = of::<S>(self.0);
                self.0 += 1.0_f64;
            }
        }
    }
}
impl Create for Ramp {
    fn create<S: Sample>(_rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self(0.0))
    }
}
static RAMP: ObjectSpec = ObjectSpec {
    name: "ramp~",
    description: "ramp",
    inlets: &[],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: Factory::new::<Ramp>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Outputs on outlet 0 the sum of its inlets, and on outlet 1 ten times that sum.
#[derive(Debug)]
struct Add;
impl<S: Sample> Object<S> for Add {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 arithmetic, which neither overflows nor panics"
    )]
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        let mut sum = S::SILENCE;
        for input in inputs {
            for (total, sample) in sum.iter_mut().zip(input) {
                *total += *sample;
            }
        }
        let mut outs = outputs.iter_mut();
        if let Some(first) = outs.next() {
            *first = sum;
        }
        if let Some(second) = outs.next() {
            for (scaled, summed) in second.iter_mut().zip(sum) {
                *scaled = summed * of::<S>(10.0);
            }
        }
    }
}
impl Create for Add {
    fn create<S: Sample>(_rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
}
static ADD: ObjectSpec = ObjectSpec {
    name: "add~",
    description: "sum",
    inlets: &[PORT, PORT],
    outlets: &[PORT, PORT],
    params: &[],
    capabilities: &[],
    create: Factory::new::<Add>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Sends its two inlets to the engine output.
#[derive(Debug)]
struct Out;
impl<S: Sample> Object<S> for Out {
    fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
}
impl Create for Out {
    fn create<S: Sample>(_rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
}
static OUT: ObjectSpec = ObjectSpec {
    name: "out~",
    description: "output",
    inlets: &[PORT, PORT],
    outlets: &[],
    params: &[],
    capabilities: &[Capability::AudioOutput],
    create: Factory::new::<Out>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// A mono output, to check the channel count.
static MONO_OUT: ObjectSpec = ObjectSpec {
    name: "mono~",
    description: "mono output",
    inlets: &[PORT],
    outlets: &[],
    params: &[],
    capabilities: &[Capability::AudioOutput],
    create: Factory::new::<Out>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Records the sample rate it was created with on its outlet.
#[derive(Debug)]
struct Rate(f64);
impl<S: Sample> Object<S> for Rate {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for output in outputs {
            output.fill(of::<S>(self.0));
        }
    }
}
impl Create for Rate {
    fn create<S: Sample>(rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self(f64::from(rate.hz() / 10)))
    }
}
static RATE: ObjectSpec = ObjectSpec {
    name: "rate~",
    description: "sample rate",
    inlets: &[],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: Factory::new::<Rate>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Copies its inlet to its outlet, bit for bit.
#[derive(Debug)]
struct Pass;
impl<S: Sample> Object<S> for Pass {
    fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            *output = *input;
        }
    }
}
impl Create for Pass {
    fn create<S: Sample>(_rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
}
static PASS: ObjectSpec = ObjectSpec {
    name: "pass~",
    description: "identity",
    inlets: &[PORT],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: Factory::new::<Pass>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// Outputs its level, which cannot be negative, with the smallest subnormal as default: a
/// spec no real box should have, to show that defaults are flushed too.
static LEVEL: ObjectSpec = ObjectSpec {
    name: "level~",
    description: "a constant level",
    inlets: &[],
    outlets: &[PORT],
    params: &[ParamSpec::new("level", "level", 1.0e-45, 0.0, 1.0)],
    capabilities: &[],
    create: Factory::new::<Const>(),
    algorithm_version: AlgorithmVersion::FIRST,
};

/// A constant whose range excludes zero: a subnormal written for it is refused.
static ABOVE: ObjectSpec = ObjectSpec {
    name: "above~",
    params: &[ParamSpec::new("value", "value", 1.0, 1.0, 2.0)],
    ..CONST
};

static REGISTRY: [&ObjectSpec; 8] = [&CONST, &RAMP, &ADD, &OUT, &MONO_OUT, &RATE, &PASS, &LEVEL];

fn rate() -> SampleRate {
    SampleRate::new(48_000).expect("valid sample rate")
}

fn node(id: u32, kind: &str) -> Node {
    Node {
        id: NodeId(id),
        kind: String::from(kind),
        params: BTreeMap::new(),
    }
}

fn with_param(mut target: Node, name: &str, value: f32) -> Node {
    let _: Option<f32> = target.params.insert(String::from(name), value);
    target
}

fn wire(from: u32, outlet: usize, to: u32, inlet: usize) -> Connection {
    Connection {
        from: NodeId(from),
        outlet,
        to: NodeId(to),
        inlet,
    }
}

fn build<S: Sample>(nodes: Vec<Node>, connections: Vec<Connection>) -> Result<Plan<S>, CoreError> {
    Patch::new(nodes, connections).and_then(|patch| Plan::<S>::build(&patch, &REGISTRY, rate()))
}

fn run<S: Sample>(plan: &mut Plan<S>, channels: usize) -> Vec<Block<S>> {
    let mut out = vec![[of::<S>(99.0); BLOCK_SIZE]; channels];
    plan.process(&mut out);
    out
}

fn block<S: Sample>(value: f64) -> Block<S> {
    [of::<S>(value); BLOCK_SIZE]
}

/// `values`, in the precision `S`, as its renders give them.
fn samples<S: Bits>(values: &[f64]) -> Vec<S::Float> {
    values.iter().map(|&value| of::<S>(value).raw()).collect()
}

/// The bits of `value`, a binary32 parameter, as a box computing in `S` outputs it.
fn bits_of<S: Bits>(value: f32) -> u64 {
    of::<S>(f64::from(value)).bits()
}

fn routes_a_box_to_the_output<S: Bits>() {
    let mut plan = build::<S>(
        vec![with_param(node(1, "const~"), "value", 0.5), node(2, "out~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid patch");
    assert_eq!(plan.channels(), 2, "out~ has two inlets");
    assert_eq!(
        run(&mut plan, 2),
        vec![block::<S>(0.5), S::SILENCE],
        "unconnected inlet is silent"
    );
}

fn uses_parameter_defaults<S: Bits>() {
    let mut plan = build::<S>(
        vec![node(1, "const~"), node(2, "out~")],
        vec![wire(1, 0, 2, 1)],
    )
    .expect("valid");
    assert_eq!(
        run(&mut plan, 2),
        vec![S::SILENCE, block::<S>(1.0)],
        "default value is 1"
    );
}

fn sums_cables_into_an_inlet_and_fans_out<S: Bits>() {
    let mut plan = build::<S>(
        vec![
            with_param(node(1, "const~"), "value", 0.25),
            with_param(node(2, "const~"), "value", 0.5),
            node(3, "out~"),
        ],
        vec![wire(1, 0, 3, 0), wire(2, 0, 3, 0), wire(2, 0, 3, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        run(&mut plan, 2),
        vec![block::<S>(0.75), block::<S>(0.5)],
        "fan-in sums, fan-out copies"
    );
}

fn chains_boxes_and_reads_every_outlet<S: Bits>() {
    let mut plan = build::<S>(
        vec![
            with_param(node(5, "const~"), "value", 2.0),
            with_param(node(4, "const~"), "value", 3.0),
            node(3, "add~"),
            node(2, "add~"),
            node(1, "out~"),
        ],
        vec![
            wire(5, 0, 3, 0),
            wire(4, 0, 3, 1),
            wire(3, 1, 2, 0),
            wire(3, 0, 2, 1),
            wire(2, 0, 1, 0),
            wire(2, 1, 1, 1),
        ],
    )
    .expect("valid patch");
    assert_eq!(
        plan.execution_order().collect::<Vec<_>>(),
        vec![NodeId(4), NodeId(5), NodeId(3), NodeId(2), NodeId(1)],
        "dependencies first, smallest id first among ready boxes"
    );
    assert_eq!(
        run(&mut plan, 2),
        vec![block::<S>(55.0), block::<S>(550.0)],
        "(2 + 3) * 10 + (2 + 3) = 55 on outlet 0, ten times that on outlet 1"
    );
}

fn orders_independent_nodes_by_id<S: Bits>() {
    let plan = build::<S>(
        vec![
            node(9, "ramp~"),
            node(3, "ramp~"),
            node(7, "out~"),
            node(1, "ramp~"),
        ],
        vec![wire(9, 0, 7, 0), wire(1, 0, 7, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        plan.execution_order().collect::<Vec<_>>(),
        vec![NodeId(1), NodeId(3), NodeId(9), NodeId(7)],
        "deterministic order"
    );
}

fn keeps_box_state_across_blocks<S: Bits>() {
    let mut plan = build::<S>(
        vec![node(1, "ramp~"), node(2, "mono~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid");
    let first = run(&mut plan, 1);
    let second = run(&mut plan, 1);
    assert_eq!(first[0][0], of::<S>(0.0), "starts at zero");
    assert_eq!(
        first[0][BLOCK_SIZE - 1],
        of::<S>(63.0),
        "counts within a block"
    );
    assert_eq!(second[0][0], of::<S>(64.0), "continues in the next block");
}

fn passes_the_sample_rate_to_boxes<S: Bits>() {
    // Every rate of the shared test matrix: a plan that hands boxes a fixed rate fails at
    // all of them but one.
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../conformance/test-matrix.json"))
            .expect("valid JSON");
    let rates: Vec<u32> = json["sample_rates"]
        .as_array()
        .expect("a list of rates")
        .iter()
        .map(|hz| u32::try_from(hz.as_u64().expect("a whole number")).expect("a u32"))
        .collect();
    assert_eq!(
        rates,
        [
            8_000, 11_025, 16_000, 22_050, 44_100, 48_000, 96_000, 384_000
        ],
        "every rate of the matrix"
    );
    for hz in rates {
        let patch = Patch::new(
            vec![node(1, "rate~"), node(2, "mono~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch");
        let rate = SampleRate::new(hz).expect("a supported rate");
        let mut plan = Plan::<S>::build(&patch, &REGISTRY, rate).expect("valid plan");
        let tenth = f64::from(u16::try_from(hz / 10).expect("small"));
        assert_eq!(run(&mut plan, 1), vec![block::<S>(tenth)], "{hz} Hz / 10");
    }
}

fn sums_several_outputs_and_uses_the_widest_channel_count<S: Bits>() {
    let mut plan = build::<S>(
        vec![
            with_param(node(1, "const~"), "value", 0.5),
            node(2, "mono~"),
            node(3, "out~"),
        ],
        vec![wire(1, 0, 2, 0), wire(1, 0, 3, 0), wire(1, 0, 3, 1)],
    )
    .expect("valid patch");
    assert_eq!(plan.channels(), 2, "widest output");
    assert_eq!(
        run(&mut plan, 2),
        vec![block::<S>(1.0), block::<S>(0.5)],
        "outputs are summed"
    );
}

fn adapts_to_the_output_slice_length<S: Bits>() {
    let mut plan = build::<S>(
        vec![with_param(node(1, "const~"), "value", 0.5), node(2, "out~")],
        vec![wire(1, 0, 2, 0), wire(1, 0, 2, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        run(&mut plan, 1),
        vec![block::<S>(0.5)],
        "extra channels are dropped"
    );
    assert_eq!(
        run(&mut plan, 3),
        vec![block::<S>(0.5), block::<S>(0.5), S::SILENCE],
        "extra slots are silent"
    );
    assert_eq!(
        run(&mut plan, 0),
        Vec::<Block<S>>::new(),
        "no output slice is fine"
    );
}

fn has_no_channel_without_an_output_box<S: Bits>() {
    let mut plan = build::<S>(vec![node(1, "ramp~")], vec![]).expect("valid patch");
    assert_eq!(plan.channels(), 0, "no output box");
    assert_eq!(run(&mut plan, 1), vec![S::SILENCE], "nothing is written");
    let mut empty = build::<S>(vec![], vec![]).expect("empty patch is valid");
    assert_eq!(
        run(&mut empty, 2),
        vec![S::SILENCE, S::SILENCE],
        "empty patch is silent"
    );
}

fn rejects_duplicate_nodes<S: Bits>() {
    assert_eq!(
        build::<S>(vec![node(1, "ramp~"), node(1, "out~")], vec![]).map(|_| ()),
        Err(CoreError::DuplicateNode(NodeId(1))),
        "ids are unique"
    );
}

fn rejects_unknown_boxes_and_parameters<S: Bits>() {
    assert_eq!(
        build::<S>(vec![node(1, "nope~")], vec![]).map(|_| ()),
        Err(CoreError::UnknownObject {
            node: NodeId(1),
            kind: String::from("nope~")
        }),
        "unknown box"
    );
    assert_eq!(
        build::<S>(vec![with_param(node(2, "const~"), "level", 1.0)], vec![]).map(|_| ()),
        Err(CoreError::UnknownParam {
            node: NodeId(2),
            name: String::from("level")
        }),
        "unknown parameter"
    );
}

fn rejects_out_of_range_parameters<S: Bits>() {
    for value in [10.5_f32, -10.5_f32, f32::INFINITY] {
        assert_eq!(
            build::<S>(vec![with_param(node(1, "const~"), "value", value)], vec![]).map(|_| ()),
            Err(CoreError::ParamOutOfRange {
                node: NodeId(1),
                name: String::from("value"),
                value
            }),
            "{value} is out of range"
        );
    }
    let nan = build::<S>(
        vec![with_param(node(1, "const~"), "value", f32::NAN)],
        vec![],
    );
    assert!(
        matches!(
            nan,
            Err(CoreError::ParamOutOfRange {
                node: NodeId(1),
                ..
            })
        ),
        "NaN is rejected"
    );
    for value in [10.0_f32, -10.0_f32] {
        assert!(
            build::<S>(vec![with_param(node(1, "const~"), "value", value)], vec![]).is_ok(),
            "{value} is within range"
        );
    }
}

fn treats_subnormal_parameters_as_zero<S: Bits>() {
    // What a box is given, by bits, for a parameter written as `value`.
    let given = |kind: &str, name: &str, value: Option<f32>| {
        let source = node(1, kind);
        let source = match value {
            Some(value) => with_param(source, name, value),
            None => source,
        };
        let mut plan = build::<S>(vec![source, node(2, "mono~")], vec![wire(1, 0, 2, 0)])
            .expect("valid patch");
        run::<S>(&mut plan, 1)[0][0].bits()
    };
    // Subnormals of both signs, from the smallest to the largest, 1e-40 among them: a zero
    // of their sign (decisions 5 and 11 of spec/engine-design.md).
    for bits in [0x0000_0001_u32, 0x0001_16c2, 0x007f_ffff] {
        for sign in [0_u32, 0x8000_0000] {
            assert_eq!(
                given("const~", "value", Some(f32::from_bits(sign | bits))),
                bits_of::<S>(f32::from_bits(sign)),
                "{:#010x}",
                sign | bits
            );
        }
    }
    // From the smallest normal on, and zeros, the value is kept as written.
    for bits in [0x0080_0000_u32, 0x8080_0000, 0x3f80_0000, 0, 0x8000_0000] {
        assert_eq!(
            given("const~", "value", Some(f32::from_bits(bits))),
            bits_of::<S>(f32::from_bits(bits)),
            "{bits:#010x}"
        );
    }
    // A negative subnormal is a zero, within a range that starts at 0: accepted.
    assert_eq!(
        given("level~", "level", Some(-1.0e-40)),
        bits_of::<S>(-0.0),
        "-1e-40 is -0"
    );
    // A subnormal default is flushed too.
    assert_eq!(given("level~", "level", None), 0, "the default 1e-45 is +0");
    // A subnormal outside the range once flushed is refused, reported as written.
    let patch = Patch::new(
        vec![with_param(node(1, "above~"), "value", 1.0e-40)],
        vec![],
    )
    .expect("a valid patch");
    assert_eq!(
        Plan::<S>::build(&patch, &[&ABOVE], rate()).map(|_| ()),
        Err(CoreError::ParamOutOfRange {
            node: NodeId(1),
            name: String::from("value"),
            value: 1.0e-40,
        }),
        "zero is below the range"
    );
}

fn rejects_invalid_connections<S: Bits>() {
    let nodes = || vec![node(1, "const~"), node(2, "out~")];
    assert_eq!(
        build::<S>(nodes(), vec![wire(3, 0, 2, 0)]).map(|_| ()),
        Err(CoreError::UnknownNode(NodeId(3))),
        "unknown source"
    );
    assert_eq!(
        build::<S>(nodes(), vec![wire(1, 0, 4, 0)]).map(|_| ()),
        Err(CoreError::UnknownNode(NodeId(4))),
        "unknown destination"
    );
    assert_eq!(
        build::<S>(nodes(), vec![wire(1, 1, 2, 0)]).map(|_| ()),
        Err(CoreError::InvalidOutlet {
            node: NodeId(1),
            outlet: 1
        }),
        "const~ has one outlet"
    );
    assert_eq!(
        build::<S>(nodes(), vec![wire(1, 0, 2, 2)]).map(|_| ()),
        Err(CoreError::InvalidInlet {
            node: NodeId(2),
            inlet: 2
        }),
        "out~ has two inlets"
    );
}

fn rejects_cycles<S: Bits>() {
    assert_eq!(
        build::<S>(
            vec![
                node(1, "add~"),
                node(2, "add~"),
                node(3, "out~"),
                node(4, "ramp~")
            ],
            vec![
                wire(1, 0, 2, 0),
                wire(2, 0, 1, 0),
                wire(2, 0, 3, 0),
                wire(4, 0, 1, 1)
            ],
        )
        .map(|_| ()),
        Err(CoreError::Cycle(vec![NodeId(1), NodeId(2), NodeId(3)])),
        "the cycle and what depends on it"
    );
    assert_eq!(
        build::<S>(vec![node(1, "add~")], vec![wire(1, 0, 1, 0)]).map(|_| ()),
        Err(CoreError::Cycle(vec![NodeId(1)])),
        "a box feeding itself"
    );
}

fn renders_interleaved_frames_offline<S: Bits>() {
    let mut renderer = Renderer::new(
        build::<S>(
            vec![
                node(1, "ramp~"),
                with_param(node(2, "const~"), "value", -1.0),
                node(3, "out~"),
            ],
            vec![wire(1, 0, 3, 0), wire(2, 0, 3, 1)],
        )
        .expect("valid patch"),
    );
    let interleaved: Vec<S::Float> = renderer.render(130).expect("small render");
    assert_eq!(interleaved.len(), 260, "130 frames of 2 channels");
    assert_eq!(
        &interleaved[..6],
        samples::<S>(&[0.0_f64, -1.0_f64, 1.0_f64, -1.0_f64, 2.0_f64, -1.0_f64]),
        "interleaved left, right"
    );
    assert_eq!(
        &interleaved[256..],
        samples::<S>(&[128.0_f64, -1.0_f64, 129.0_f64, -1.0_f64]),
        "continues across blocks"
    );
    let next: Vec<S::Float> = renderer.render(1).expect("small render");
    assert_eq!(
        next,
        samples::<S>(&[130.0_f64, -1.0_f64]),
        "the next render continues exactly where the previous one stopped"
    );
}

fn renders_nothing_for_zero_frames_or_channels<S: Bits>() {
    let mut renderer = Renderer::new(
        build::<S>(
            vec![node(1, "ramp~"), node(2, "mono~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch"),
    );
    assert_eq!(
        renderer.render(0),
        Ok(Vec::<S::Float>::new()),
        "zero frames"
    );
    assert_eq!(
        renderer.render::<S::Float>(64).expect("small render"),
        (0..64_u16)
            .map(|value| of::<S>(f64::from(value)).raw())
            .collect::<Vec<_>>(),
        "nothing was processed before"
    );
    let mut silent =
        Renderer::new(build::<S>(vec![node(1, "ramp~")], vec![]).expect("valid patch"));
    assert_eq!(
        silent.render(usize::MAX),
        Ok(Vec::<S::Float>::new()),
        "no output channel: nothing to render, at once"
    );
}

fn refuses_renders_too_long_for_memory<S: Bits>() {
    let mut renderer = Renderer::new(
        build::<S>(
            vec![node(1, "ramp~"), node(2, "out~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch"),
    );
    assert_eq!(
        renderer.render::<S::Float>(usize::MAX),
        Err(CoreError::RenderTooLong(usize::MAX)),
        "frames times channels overflows"
    );
    assert_eq!(
        renderer.render::<S::Float>(usize::MAX / 4),
        Err(CoreError::RenderTooLong(usize::MAX / 4)),
        "fits the arithmetic, not the memory"
    );
    assert_eq!(
        renderer.render::<S::Float>(2).expect("small render"),
        samples::<S>(&[0.0_f64, 0.0_f64, 1.0_f64, 0.0_f64]),
        "nothing was rendered by the refused calls"
    );
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "S is f32 or f64: IEEE 754 addition, which neither overflows nor panics"
)]
fn sums_cables_in_source_order_whatever_the_document_order<S: Bits>() {
    // Float addition is not associative: with these values, (1 + e) + e = 1 while
    // e + (e + 1) and (e + e) + 1 give the next float above 1. The sum must follow
    // the source ids (1, then 2, then 3) for every order of the cables. `e` is between a
    // quarter and half an ulp of 1, in the precision of the render: 3e-8 in binary32,
    // 3 · 2^-55 in binary64 (a binary32 parameter too).
    let tiny = match S::PRECISION {
        Precision::Binary64 => 8.326_673e-17_f32,
        Precision::Binary32 => 3.0e-8_f32,
    };
    let nodes = || {
        vec![
            with_param(node(1, "const~"), "value", 1.0),
            with_param(node(2, "const~"), "value", tiny),
            with_param(node(3, "const~"), "value", tiny),
            node(4, "mono~"),
        ]
    };
    let (one, small) = (of::<S>(1.0), of::<S>(f64::from(tiny)));
    // The reduction order (decision 28) of the terms 1, e, e: lane 0 (1) with lane 2 (e),
    // then lane 1 (e).
    let in_order = (one + small) + small;
    assert_eq!(in_order.bits(), one.bits(), "(1 + e) + e = 1");
    assert_ne!(
        in_order.bits(),
        ((small + small) + one).bits(),
        "the values do reveal the order"
    );
    let cables = [wire(1, 0, 4, 0), wire(2, 0, 4, 0), wire(3, 0, 4, 0)];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut plan =
            build::<S>(nodes(), order.iter().map(|&i| cables[i]).collect()).expect("valid patch");
        let out = run(&mut plan, 1);
        assert_eq!(
            out[0][0].bits(),
            in_order.bits(),
            "cables listed as {order:?}"
        );
    }
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "counts of boxes near the patch limits, 2^16, far from overflowing u32"
)]
fn accepts_patches_up_to_the_limits_only<S: Bits>() {
    /// The four cables joining both outlets of `from` to both inlets of `to`.
    fn all_ports(from: u32, to: u32) -> [Connection; 4] {
        [
            wire(from, 0, to, 0),
            wire(from, 0, to, 1),
            wire(from, 1, to, 0),
            wire(from, 1, to, 1),
        ]
    }
    let adders = |count: u32| -> Vec<Node> { (0..count).map(|id| node(id, "add~")).collect() };
    let max_nodes = u32::try_from(Patch::MAX_NODES).expect("fits u32");
    assert_eq!(
        Patch::new(adders(max_nodes + 1), vec![]).map(|_| ()),
        Err(CoreError::TooManyNodes(Patch::MAX_NODES + 1)),
        "one box too many"
    );
    let too_many = Patch::MAX_CONNECTIONS + 1;
    assert_eq!(
        Patch::new(vec![], vec![wire(1, 0, 2, 0); too_many]).map(|_| ()),
        Err(CoreError::TooManyConnections(too_many)),
        "one cable too many, refused before looking for repetitions"
    );

    // The largest patch: a chain of 65,535 add~ boxes joined port to port, an output
    // fed by the last one, and one shortcut: exactly 65,536 boxes and 262,144 cables.
    let last = max_nodes - 2;
    let mut nodes = adders(max_nodes - 1);
    nodes.push(node(u32::MAX, "out~"));
    let mut cables: Vec<Connection> = (0..last).flat_map(|id| all_ports(id, id + 1)).collect();
    cables.extend([
        wire(last, 0, u32::MAX, 0),
        wire(last, 0, u32::MAX, 1),
        wire(last, 1, u32::MAX, 0),
        wire(last, 1, u32::MAX, 1),
    ]);
    cables.extend(all_ports(0, 2));
    assert_eq!(cables.len(), Patch::MAX_CONNECTIONS, "exactly the limit");
    let patch = Patch::new(nodes, cables).expect("within the limits");
    let plan = Plan::<S>::build(&patch, &REGISTRY, rate()).expect("valid plan");
    assert_eq!(
        plan.execution_order().count(),
        Patch::MAX_NODES,
        "every box"
    );
    assert_eq!(
        plan.execution_order().last(),
        Some(NodeId(u32::MAX)),
        "the output runs last"
    );
    assert_eq!(
        Renderer::new(plan)
            .render::<S::Float>(1)
            .map(|rendered| rendered.len()),
        Ok(2),
        "renders"
    );
}

fn rejects_repeated_cables<S: Bits>() {
    assert_eq!(
        build::<S>(
            vec![node(1, "ramp~"), node(2, "out~")],
            vec![wire(1, 0, 2, 0), wire(1, 0, 2, 1), wire(1, 0, 2, 0)]
        )
        .map(|_| ()),
        Err(CoreError::DuplicateConnection(wire(1, 0, 2, 0))),
        "the same cable twice"
    );
}

fn renders_into_a_buffer_continuing_like_render<S: Bits>() {
    let patch = || {
        Renderer::new(
            build::<S>(
                vec![
                    node(1, "ramp~"),
                    with_param(node(2, "const~"), "value", -1.0),
                    node(3, "out~"),
                ],
                vec![wire(1, 0, 3, 0), wire(2, 0, 3, 1)],
            )
            .expect("valid patch"),
        )
    };
    let mut whole = patch();
    let expected: Vec<S::Float> = whole.render(100).expect("small render");
    let mut parts = patch();
    let mut first = [of::<S>(7.0).raw(); 61];
    parts.render_into(&mut first);
    assert_eq!(
        first[60],
        of::<S>(7.0).raw(),
        "the sample after the last whole frame is untouched"
    );
    let mut rest: Vec<S::Float> = parts.render(70).expect("small render");
    let mut actual = first[..60].to_vec();
    actual.append(&mut rest);
    assert_eq!(
        actual, expected,
        "30 frames into a buffer, then 70 rendered"
    );

    let mut silent =
        Renderer::new(build::<S>(vec![node(1, "ramp~")], vec![]).expect("valid patch"));
    let mut untouched = [of::<S>(7.0).raw(); 4];
    silent.render_into(&mut untouched);
    assert_eq!(
        untouched,
        [of::<S>(7.0).raw(); 4],
        "no output: nothing written"
    );
}

/// The bit patterns of every sample of `blocks`.
fn block_bits<S: Bits>(blocks: &[Block<S>]) -> Vec<u64> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.bits())
        .collect()
}

fn a_single_cable_passes_its_samples_bit_for_bit<S: Bits>() {
    // -0.0 through one cable into a box, then through one cable to the output: unchanged.
    // Summing onto silence would turn it into +0.0 (+0.0 + -0.0 = +0.0).
    let chain = || {
        build::<S>(
            vec![
                with_param(node(1, "const~"), "value", -0.0),
                node(2, "pass~"),
                node(3, "mono~"),
            ],
            vec![wire(1, 0, 2, 0), wire(2, 0, 3, 0)],
        )
        .expect("valid patch")
    };
    let negative_zero = bits_of::<S>(-0.0);
    assert_eq!(
        block_bits(&run(&mut chain(), 1)),
        vec![negative_zero; BLOCK_SIZE],
        "-0.0 at the inlet of pass~ and at the output, through process"
    );
    let rendered: Vec<u64> = Renderer::new(chain())
        .render::<S::Float>(BLOCK_SIZE + 3)
        .expect("small render")
        .iter()
        .map(|sample| S::from_raw(*sample).bits())
        .collect();
    assert_eq!(
        rendered,
        vec![negative_zero; BLOCK_SIZE + 3],
        "and through render"
    );
}

fn several_cables_are_summed_starting_from_the_first<S: Bits>() {
    let sum = |first: f32, second: f32| {
        let mut plan = build::<S>(
            vec![
                with_param(node(1, "const~"), "value", first),
                with_param(node(2, "const~"), "value", second),
                node(3, "mono~"),
            ],
            vec![wire(1, 0, 3, 0), wire(2, 0, 3, 0)],
        )
        .expect("valid patch");
        block_bits(&run(&mut plan, 1))
    };
    assert_eq!(
        sum(-0.0, -0.0),
        vec![bits_of::<S>(-0.0); BLOCK_SIZE],
        "-0.0 + -0.0 = -0.0"
    );
    assert_eq!(sum(-0.0, 0.0), vec![0; BLOCK_SIZE], "-0.0 + +0.0 = +0.0");
}

fn several_output_boxes_are_summed_starting_from_the_first<S: Bits>() {
    // Two outputs, a mono one and a stereo one, each fed -0.0: channel 0 sums both
    // (-0.0 + -0.0), channel 1 comes from the stereo one alone; a third slot is silent.
    let mut plan = build::<S>(
        vec![
            with_param(node(1, "const~"), "value", -0.0),
            node(2, "mono~"),
            node(3, "out~"),
        ],
        vec![wire(1, 0, 2, 0), wire(1, 0, 3, 0), wire(1, 0, 3, 1)],
    )
    .expect("valid patch");
    let out = run(&mut plan, 3);
    assert_eq!(
        block_bits(&out[..2]),
        vec![bits_of::<S>(-0.0); 2 * BLOCK_SIZE],
        "both channels keep -0.0"
    );
    assert_eq!(block_bits(&out[2..]), vec![0; BLOCK_SIZE], "+0.0 beyond");
    // A stereo output fed on its second inlet only: its first channel is silence, +0.0.
    let mut half = build::<S>(
        vec![
            with_param(node(1, "const~"), "value", -0.0),
            node(2, "out~"),
        ],
        vec![wire(1, 0, 2, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut half, 2)),
        [vec![0; BLOCK_SIZE], vec![bits_of::<S>(-0.0); BLOCK_SIZE]].concat(),
        "an unconnected inlet is +0.0, a connected one is passed as is"
    );
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "S is f32 or f64: IEEE 754 addition, which neither overflows nor panics"
)]
fn sums_output_boxes_in_box_id_order_whatever_the_execution_order<S: Bits>() {
    // Output boxes 1, 2 and 3 fed by constants 9 (e), 4 (1) and 5 (e): they run in the
    // order 2, 3, 1 (each after its source, the smallest ready id first), but a channel sums
    // them by box id, like buses: terms e, 1, e, so (e + e) + 1 in the reduction order, the
    // float just above 1, where the execution order's (1 + e) + e is 1. `e` is between a
    // quarter and half an ulp of 1, in the precision of the render.
    let tiny = match S::PRECISION {
        Precision::Binary64 => 8.326_673e-17_f32,
        Precision::Binary32 => 3.0e-8_f32,
    };
    let mut plan = build::<S>(
        vec![
            node(1, "mono~"),
            node(2, "mono~"),
            node(3, "mono~"),
            with_param(node(4, "const~"), "value", 1.0),
            with_param(node(5, "const~"), "value", tiny),
            with_param(node(9, "const~"), "value", tiny),
        ],
        vec![wire(9, 0, 1, 0), wire(4, 0, 2, 0), wire(5, 0, 3, 0)],
    )
    .expect("valid patch");
    assert_eq!(
        plan.execution_order().collect::<Vec<_>>(),
        [4, 2, 5, 3, 9, 1].map(NodeId),
        "the outputs run in the order 2, 3, 1"
    );
    let (one, small) = (of::<S>(1.0), of::<S>(f64::from(tiny)));
    let by_id = (small + small) + one;
    assert_ne!(
        by_id.bits(),
        ((one + small) + small).bits(),
        "the values reveal the order"
    );
    assert_eq!(run(&mut plan, 1)[0][0].bits(), by_id.bits(), "by box id");
}

fn delivers_a_binary64_render_rounded_to_binary32<S: Bits>()
where
    f32: Delivered<S>,
{
    // 0.1 in binary64, given to a host that takes binary32: rounded once, correctly; a
    // binary32 render given as binary32 is the same samples.
    let mut renderer = Renderer::new(
        build::<S>(
            vec![
                with_param(node(1, "const~"), "value", 0.1),
                node(2, "mono~"),
            ],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch"),
    );
    let delivered: Vec<f32> = renderer.render(3).expect("small render");
    let mut more = [0.0_f32; 2];
    renderer.render_into(&mut more);
    let expected = 0.1_f32.to_bits();
    assert!(
        delivered
            .iter()
            .chain(&more)
            .all(|sample| sample.to_bits() == expected),
        "{delivered:?} {more:?}"
    );
}

fn keeps_the_subnormals_of_sums_and_of_delivery<S: Bits>()
where
    f32: Delivered<S>,
{
    // Neither the plan's sums nor delivery flush (decision 11): 1.5 · 2^-126 and -2^-126,
    // both normal, sum exactly to 2^-127, a binary32 subnormal, summed into one inlet or
    // onto one channel from two output boxes, and given to a binary32 host as it is, in
    // both precisions.
    let (high, low) = (f32::from_bits(0x00c0_0000), f32::from_bits(0x8080_0000));
    let consts = || {
        vec![
            with_param(node(1, "const~"), "value", high),
            with_param(node(2, "const~"), "value", low),
        ]
    };
    let into_one_inlet = build::<S>(
        [consts(), vec![node(3, "mono~")]].concat(),
        vec![wire(1, 0, 3, 0), wire(2, 0, 3, 0)],
    )
    .expect("valid patch");
    let onto_one_channel = build::<S>(
        [consts(), vec![node(3, "mono~"), node(4, "mono~")]].concat(),
        vec![wire(1, 0, 3, 0), wire(2, 0, 4, 0)],
    )
    .expect("valid patch");
    for (plan, place) in [
        (into_one_inlet, "into one inlet"),
        (onto_one_channel, "onto one channel"),
    ] {
        let delivered: Vec<f32> = Renderer::new(plan).render(2).expect("small render");
        assert_eq!(
            delivered
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            [0x0040_0000; 2],
            "2^-127 summed {place}, delivered as it is"
        );
    }
}

/// Outputs `a × a - 1` through the math layer's `mul_then_add`, with `a` chosen per
/// precision where one rounding and two differ (`1 + 2^-27`, `1 + 2^-12`): what the
/// arithmetic mode of the build makes of a box's product then sum.
#[derive(Debug)]
struct ProductThenSum;
impl<S: Sample> Object<S> for ProductThenSum {
    fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
        let near_one = match S::PRECISION {
            Precision::Binary64 => of::<S>(f64::from_bits(0x3ff0_0000_0200_0000)),
            Precision::Binary32 => of::<S>(f64::from(f32::from_bits(0x3f80_0800))),
        };
        output_all(outputs, near_one.mul_then_add(near_one, of::<S>(-1.0)));
    }
}
impl Create for ProductThenSum {
    fn create<S: Sample>(_rate: SampleRate, _params: &Params) -> Box<dyn Object<S>> {
        Box::new(Self)
    }
}

/// Fills every output with `value`.
fn output_all<S: Sample>(outputs: &mut [Block<S>], value: S) {
    for output in outputs {
        output.fill(value);
    }
}

/// The bits of `a × a - 1` in each precision, product and sum rounded apart (the strict
/// mode): 2^-26 in binary64, 2^-11 in binary32, worked out in `math_tests.rs`.
#[cfg(not(poyetik_math = "fused"))]
const PRODUCT_THEN_SUM: [u64; 2] = [0x3e50_0000_0000_0000, 0x3a00_0000];

/// The same with one rounding, in xtask's fused test build: 2^-26 + 2^-54, 2^-11 + 2^-24.
#[cfg(poyetik_math = "fused")]
const PRODUCT_THEN_SUM: [u64; 2] = [0x3e50_0000_0100_0000, 0x3a00_0400];

fn renders_a_product_then_a_sum_in_the_mode_of_the_build<S: Bits>() {
    // The positive control of the arithmetic mode: a box's `mul_then_add`, rendered through
    // a plan, gives what the mode of the build says, and only that function changes it.
    static PRODUCT: ObjectSpec = ObjectSpec {
        name: "product~",
        description: "a product then a sum",
        inlets: &[],
        outlets: &[PORT],
        params: &[],
        capabilities: &[],
        create: Factory::new::<ProductThenSum>(),
        algorithm_version: AlgorithmVersion::FIRST,
    };
    let patch = Patch::new(
        vec![node(1, "product~"), node(2, "mono~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid patch");
    let plan = Plan::<S>::build(&patch, &[&PRODUCT, &MONO_OUT], rate()).expect("valid plan");
    let rendered: Vec<u64> = Renderer::new(plan)
        .render::<S::Float>(3)
        .expect("small render")
        .iter()
        .map(|sample| S::from_raw(*sample).bits())
        .collect();
    let index = match S::PRECISION {
        Precision::Binary64 => 0,
        Precision::Binary32 => 1,
    };
    assert_eq!(
        rendered,
        vec![PRODUCT_THEN_SUM[index]; 3],
        "{:?}",
        S::PRECISION
    );
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "S is f32 or f64: IEEE 754 addition, which neither overflows nor panics"
)]
fn sums_three_cables_and_three_outputs_in_the_reduction_order<S: Bits>() {
    // 1, -1 and 2^-60: the reduction order (decision 28) adds the third term to the first,
    // (1 + 2^-60) + -1 = +0, where the sequential sum (1 + -1) + 2^-60 keeps 2^-60.
    let values = [1.0_f32, -1.0, 8.673_617e-19];
    let consts = || {
        (1_u32..=3)
            .zip(values)
            .map(|(id, value)| with_param(node(id, "const~"), "value", value))
            .collect::<Vec<Node>>()
    };
    let mut cables = build::<S>(
        [consts(), vec![node(4, "mono~")]].concat(),
        vec![wire(1, 0, 4, 0), wire(2, 0, 4, 0), wire(3, 0, 4, 0)],
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut cables, 1)),
        vec![0; BLOCK_SIZE],
        "three cables into one inlet"
    );
    // Three output boxes on one channel, run in the order of their ids (4, 5, 6), each fed
    // by one constant: the same three terms.
    let mut outputs = build::<S>(
        [
            consts(),
            vec![node(4, "mono~"), node(5, "mono~"), node(6, "mono~")],
        ]
        .concat(),
        vec![wire(1, 0, 4, 0), wire(2, 0, 5, 0), wire(3, 0, 6, 0)],
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut outputs, 1)),
        vec![0; BLOCK_SIZE],
        "three output boxes on one channel"
    );
    let tiny = of::<S>(f64::from(values[2]));
    let sequential = (of::<S>(1.0) + of::<S>(-1.0)) + tiny;
    assert_eq!(
        sequential.bits(),
        tiny.bits(),
        "the values reveal the order: the sequential sum differs"
    );
}

fn sums_seventeen_cables_and_seventeen_outputs_in_the_reduction_order<S: Bits>() {
    // Terms 0, 8 and 16 are 1, -1 and 2^-60, the others 0: term 16 joins lane 0 before the
    // tree, (1 + 2^-60) + -1 = +0, where a sequential sum (or 32 lanes) keeps 2^-60.
    let value = |index: u32| match index {
        0 => 1.0_f32,
        8 => -1.0,
        16 => 8.673_617e-19,
        _ => 0.0,
    };
    let consts = || {
        (0_u32..17)
            .map(|index| {
                with_param(
                    node(index.saturating_add(1), "const~"),
                    "value",
                    value(index),
                )
            })
            .collect::<Vec<Node>>()
    };
    let mut cables = build::<S>(
        [consts(), vec![node(18, "mono~")]].concat(),
        (1_u32..=17).map(|id| wire(id, 0, 18, 0)).collect(),
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut cables, 1)),
        vec![0; BLOCK_SIZE],
        "seventeen cables into one inlet"
    );
    let mut outputs = build::<S>(
        [
            consts(),
            (18_u32..=34).map(|id| node(id, "mono~")).collect(),
        ]
        .concat(),
        (1_u32..=17)
            .map(|id| wire(id, 0, id.saturating_add(17), 0))
            .collect(),
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut outputs, 1)),
        vec![0; BLOCK_SIZE],
        "seventeen output boxes on one channel"
    );
}

/// The plan decides when it is built how each inlet and each output channel is fed: no
/// source leaves it silent (no feed), one source is a copy (the reduction order of one term,
/// bit for bit), two are a pair added frame by frame (the order of two terms, without
/// lanes), three or more go through the sum of decision 28. The choice is the plan's
/// structure, read here, not a branch taken while processing.
fn decides_how_each_block_is_fed_when_built<S: Bits>() {
    // Seventeen const~ (ids 1 to 17, bus slots 0 to 16), then out~ 20, out~ 21 and mono~ 22
    // (instances 17, 18 and 19).
    let consts: Vec<Node> = (1..=17).map(|id| node(id, "const~")).collect();
    let plan = build::<S>(
        [
            consts.clone(),
            vec![node(20, "out~"), node(21, "out~"), node(22, "mono~")],
        ]
        .concat(),
        [
            vec![
                wire(1, 0, 20, 0),
                wire(1, 0, 20, 1),
                wire(2, 0, 20, 1),
                wire(1, 0, 21, 0),
                wire(2, 0, 21, 0),
                wire(3, 0, 21, 0),
            ],
            (1..=17).map(|id| wire(id, 0, 21, 1)).collect(),
        ]
        .concat(),
    )
    .expect("valid patch");
    let feeds: Vec<Vec<(usize, String)>> = plan
        .instances
        .iter()
        .map(|instance| {
            instance
                .feeds
                .iter()
                .map(|(inlet, feed)| (*inlet, shape(feed)))
                .collect()
        })
        .collect();
    let fed = |inlet: usize, text: &str| (inlet, String::from(text));
    assert_eq!(
        feeds,
        [
            vec![vec![]; 17],
            vec![
                vec![fed(0, "copy 0"), fed(1, "pair 0 1")],
                vec![
                    fed(0, "sum [0, 1, 2]"),
                    fed(
                        1,
                        "sum [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]"
                    ),
                ],
                vec![],
            ],
        ]
        .concat(),
        "out~ 20 copies one cable and adds two; out~ 21 sums three and seventeen; mono~ is \
         unconnected"
    );
    assert_eq!(
        plan.channel_feeds.iter().map(shape).collect::<Vec<_>>(),
        ["sum [(17, 0), (18, 0), (19, 0)]", "pair (17, 1) (18, 1)"],
        "channel 0 sums out~ 20, out~ 21 and mono~; channel 1 adds the two out~"
    );
    let single = build::<S>(
        vec![node(1, "const~"), node(2, "out~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid patch");
    assert_eq!(
        single.channel_feeds.iter().map(shape).collect::<Vec<_>>(),
        ["copy (1, 0)", "copy (1, 1)"],
        "one output box: each channel copies its inlet"
    );
    assert_eq!(
        build::<S>(consts, vec![])
            .expect("valid patch")
            .channel_feeds
            .len(),
        0,
        "no output box, no channel"
    );
}

/// How `feed` is fed, in words: `copy <source>`, `pair <source> <source>` or
/// `sum [<sources>]`.
fn shape<K: core::fmt::Debug>(feed: &Feed<K>) -> String {
    match feed {
        Feed::Single(source) => alloc::format!("copy {source:?}"),
        Feed::Two(first, second) => alloc::format!("pair {first:?} {second:?}"),
        Feed::Several(sources) => alloc::format!("sum {sources:?}"),
    }
}

in_both_precisions!(
    routes_a_box_to_the_output,
    uses_parameter_defaults,
    sums_cables_into_an_inlet_and_fans_out,
    chains_boxes_and_reads_every_outlet,
    orders_independent_nodes_by_id,
    keeps_box_state_across_blocks,
    passes_the_sample_rate_to_boxes,
    sums_several_outputs_and_uses_the_widest_channel_count,
    adapts_to_the_output_slice_length,
    has_no_channel_without_an_output_box,
    rejects_duplicate_nodes,
    rejects_unknown_boxes_and_parameters,
    rejects_out_of_range_parameters,
    treats_subnormal_parameters_as_zero,
    rejects_invalid_connections,
    rejects_cycles,
    renders_interleaved_frames_offline,
    renders_nothing_for_zero_frames_or_channels,
    refuses_renders_too_long_for_memory,
    sums_cables_in_source_order_whatever_the_document_order,
    accepts_patches_up_to_the_limits_only,
    rejects_repeated_cables,
    renders_into_a_buffer_continuing_like_render,
    a_single_cable_passes_its_samples_bit_for_bit,
    several_cables_are_summed_starting_from_the_first,
    several_output_boxes_are_summed_starting_from_the_first,
    sums_output_boxes_in_box_id_order_whatever_the_execution_order,
    delivers_a_binary64_render_rounded_to_binary32,
    keeps_the_subnormals_of_sums_and_of_delivery,
    renders_a_product_then_a_sum_in_the_mode_of_the_build,
    sums_three_cables_and_three_outputs_in_the_reduction_order,
    sums_seventeen_cables_and_seventeen_outputs_in_the_reduction_order,
    decides_how_each_block_is_fed_when_built,
);
