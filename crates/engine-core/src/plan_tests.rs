//! Tests of plan building and execution, using test-only boxes.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{
    BLOCK_SIZE, Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec,
    ParamSpec, Patch, Plan, PortSpec, Renderer, SILENCE, SampleRate,
};

const PORT: PortSpec = PortSpec {
    name: "x",
    description: "test port",
};

/// Outputs a constant value.
#[derive(Debug)]
struct Const(f32);
impl Object for Const {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs {
            output.fill(self.0);
        }
    }
}
static CONST: ObjectSpec = ObjectSpec {
    name: "const~",
    description: "constant",
    inlets: &[],
    outlets: &[PORT],
    params: &[ParamSpec {
        name: "value",
        description: "value",
        default: 1.0,
        min: -10.0,
        max: 10.0,
    }],
    capabilities: &[],
    create: |_, params| Box::new(Const(params.first().copied().unwrap_or(0.0))),
};

/// Outputs 0, 1, 2… continuing across blocks: checks that state is kept.
#[derive(Debug)]
struct Ramp(f32);
impl Object for Ramp {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs {
            for sample in output.iter_mut() {
                *sample = self.0;
                self.0 += 1.0;
            }
        }
    }
}
static RAMP: ObjectSpec = ObjectSpec {
    name: "ramp~",
    description: "ramp",
    inlets: &[],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: |_, _| Box::new(Ramp(0.0)),
};

/// Outputs on outlet 0 the sum of its inlets, and on outlet 1 ten times that sum.
#[derive(Debug)]
struct Add;
impl Object for Add {
    fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
        let mut sum = SILENCE;
        for input in inputs {
            for (total, sample) in sum.iter_mut().zip(input) {
                *total += sample;
            }
        }
        let mut outs = outputs.iter_mut();
        if let Some(first) = outs.next() {
            *first = sum;
        }
        if let Some(second) = outs.next() {
            for (scaled, summed) in second.iter_mut().zip(sum) {
                *scaled = summed * 10.0;
            }
        }
    }
}
static ADD: ObjectSpec = ObjectSpec {
    name: "add~",
    description: "sum",
    inlets: &[PORT, PORT],
    outlets: &[PORT, PORT],
    params: &[],
    capabilities: &[],
    create: |_, _| Box::new(Add),
};

/// Sends its two inlets to the engine output.
#[derive(Debug)]
struct Out;
impl Object for Out {
    fn process(&mut self, _inputs: &[Block], _outputs: &mut [Block]) {}
}
static OUT: ObjectSpec = ObjectSpec {
    name: "out~",
    description: "output",
    inlets: &[PORT, PORT],
    outlets: &[],
    params: &[],
    capabilities: &[Capability::AudioOutput],
    create: |_, _| Box::new(Out),
};

/// A mono output, to check the channel count.
static MONO_OUT: ObjectSpec = ObjectSpec {
    name: "mono~",
    description: "mono output",
    inlets: &[PORT],
    outlets: &[],
    params: &[],
    capabilities: &[Capability::AudioOutput],
    create: |_, _| Box::new(Out),
};

/// Records the sample rate it was created with on its outlet.
#[derive(Debug)]
struct Rate(f32);
impl Object for Rate {
    fn process(&mut self, _inputs: &[Block], outputs: &mut [Block]) {
        for output in outputs {
            output.fill(self.0);
        }
    }
}
static RATE: ObjectSpec = ObjectSpec {
    name: "rate~",
    description: "sample rate",
    inlets: &[],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: |rate, _| Box::new(Rate(f32::from(u16::try_from(rate.hz() / 10).unwrap_or(0)))),
};

/// Copies its inlet to its outlet, bit for bit.
#[derive(Debug)]
struct Pass;
impl Object for Pass {
    fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
        for (output, input) in outputs.iter_mut().zip(inputs) {
            *output = *input;
        }
    }
}
static PASS: ObjectSpec = ObjectSpec {
    name: "pass~",
    description: "identity",
    inlets: &[PORT],
    outlets: &[PORT],
    params: &[],
    capabilities: &[],
    create: |_, _| Box::new(Pass),
};

static REGISTRY: [&ObjectSpec; 7] = [&CONST, &RAMP, &ADD, &OUT, &MONO_OUT, &RATE, &PASS];

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

fn build(nodes: Vec<Node>, connections: Vec<Connection>) -> Result<Plan, CoreError> {
    Patch::new(nodes, connections).and_then(|patch| Plan::build(&patch, &REGISTRY, rate()))
}

fn run(plan: &mut Plan, channels: usize) -> Vec<Block> {
    let mut out = vec![[99.0_f32; BLOCK_SIZE]; channels];
    plan.process(&mut out);
    out
}

fn block(value: f32) -> Block {
    [value; BLOCK_SIZE]
}

#[test]
fn routes_a_box_to_the_output() {
    let mut plan = build(
        vec![with_param(node(1, "const~"), "value", 0.5), node(2, "out~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid patch");
    assert_eq!(plan.channels(), 2, "out~ has two inlets");
    assert_eq!(
        run(&mut plan, 2),
        vec![block(0.5), SILENCE],
        "unconnected inlet is silent"
    );
}

#[test]
fn uses_parameter_defaults() {
    let mut plan = build(
        vec![node(1, "const~"), node(2, "out~")],
        vec![wire(1, 0, 2, 1)],
    )
    .expect("valid");
    assert_eq!(
        run(&mut plan, 2),
        vec![SILENCE, block(1.0)],
        "default value is 1"
    );
}

#[test]
fn sums_cables_into_an_inlet_and_fans_out() {
    let mut plan = build(
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
        vec![block(0.75), block(0.5)],
        "fan-in sums, fan-out copies"
    );
}

#[test]
fn chains_boxes_and_reads_every_outlet() {
    let mut plan = build(
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
        vec![block(55.0), block(550.0)],
        "(2 + 3) * 10 + (2 + 3) = 55 on outlet 0, ten times that on outlet 1"
    );
}

#[test]
fn orders_independent_nodes_by_id() {
    let plan = build(
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

#[test]
fn keeps_box_state_across_blocks() {
    let mut plan = build(
        vec![node(1, "ramp~"), node(2, "mono~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid");
    let first = run(&mut plan, 1);
    let second = run(&mut plan, 1);
    assert_eq!(first[0][0], 0.0, "starts at zero");
    assert_eq!(first[0][BLOCK_SIZE - 1], 63.0, "counts within a block");
    assert_eq!(second[0][0], 64.0, "continues in the next block");
}

#[test]
fn passes_the_sample_rate_to_boxes() {
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
    assert!(rates.len() >= 8, "{rates:?}");
    for hz in rates {
        let patch = Patch::new(
            vec![node(1, "rate~"), node(2, "mono~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch");
        let rate = SampleRate::new(hz).expect("a supported rate");
        let mut plan = Plan::build(&patch, &REGISTRY, rate).expect("valid plan");
        let tenth = f32::from(u16::try_from(hz / 10).expect("small"));
        assert_eq!(run(&mut plan, 1), vec![block(tenth)], "{hz} Hz / 10");
    }
}

#[test]
fn sums_several_outputs_and_uses_the_widest_channel_count() {
    let mut plan = build(
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
        vec![block(1.0), block(0.5)],
        "outputs are summed"
    );
}

#[test]
fn adapts_to_the_output_slice_length() {
    let mut plan = build(
        vec![with_param(node(1, "const~"), "value", 0.5), node(2, "out~")],
        vec![wire(1, 0, 2, 0), wire(1, 0, 2, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        run(&mut plan, 1),
        vec![block(0.5)],
        "extra channels are dropped"
    );
    assert_eq!(
        run(&mut plan, 3),
        vec![block(0.5), block(0.5), SILENCE],
        "extra slots are silent"
    );
    assert_eq!(
        run(&mut plan, 0),
        Vec::<Block>::new(),
        "no output slice is fine"
    );
}

#[test]
fn has_no_channel_without_an_output_box() {
    let mut plan = build(vec![node(1, "ramp~")], vec![]).expect("valid patch");
    assert_eq!(plan.channels(), 0, "no output box");
    assert_eq!(run(&mut plan, 1), vec![SILENCE], "nothing is written");
    let mut empty = build(vec![], vec![]).expect("empty patch is valid");
    assert_eq!(
        run(&mut empty, 2),
        vec![SILENCE, SILENCE],
        "empty patch is silent"
    );
}

#[test]
fn rejects_duplicate_nodes() {
    assert_eq!(
        build(vec![node(1, "ramp~"), node(1, "out~")], vec![]).map(|_| ()),
        Err(CoreError::DuplicateNode(NodeId(1))),
        "ids are unique"
    );
}

#[test]
fn rejects_unknown_boxes_and_parameters() {
    assert_eq!(
        build(vec![node(1, "nope~")], vec![]).map(|_| ()),
        Err(CoreError::UnknownObject {
            node: NodeId(1),
            kind: String::from("nope~")
        }),
        "unknown box"
    );
    assert_eq!(
        build(vec![with_param(node(2, "const~"), "level", 1.0)], vec![]).map(|_| ()),
        Err(CoreError::UnknownParam {
            node: NodeId(2),
            name: String::from("level")
        }),
        "unknown parameter"
    );
}

#[test]
fn rejects_out_of_range_parameters() {
    for value in [10.5_f32, -10.5_f32, f32::INFINITY] {
        assert_eq!(
            build(vec![with_param(node(1, "const~"), "value", value)], vec![]).map(|_| ()),
            Err(CoreError::ParamOutOfRange {
                node: NodeId(1),
                name: String::from("value"),
                value
            }),
            "{value} is out of range"
        );
    }
    let nan = build(
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
            build(vec![with_param(node(1, "const~"), "value", value)], vec![]).is_ok(),
            "{value} is within range"
        );
    }
}

#[test]
fn rejects_invalid_connections() {
    let nodes = || vec![node(1, "const~"), node(2, "out~")];
    assert_eq!(
        build(nodes(), vec![wire(3, 0, 2, 0)]).map(|_| ()),
        Err(CoreError::UnknownNode(NodeId(3))),
        "unknown source"
    );
    assert_eq!(
        build(nodes(), vec![wire(1, 0, 4, 0)]).map(|_| ()),
        Err(CoreError::UnknownNode(NodeId(4))),
        "unknown destination"
    );
    assert_eq!(
        build(nodes(), vec![wire(1, 1, 2, 0)]).map(|_| ()),
        Err(CoreError::InvalidOutlet {
            node: NodeId(1),
            outlet: 1
        }),
        "const~ has one outlet"
    );
    assert_eq!(
        build(nodes(), vec![wire(1, 0, 2, 2)]).map(|_| ()),
        Err(CoreError::InvalidInlet {
            node: NodeId(2),
            inlet: 2
        }),
        "out~ has two inlets"
    );
}

#[test]
fn rejects_cycles() {
    assert_eq!(
        build(
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
        build(vec![node(1, "add~")], vec![wire(1, 0, 1, 0)]).map(|_| ()),
        Err(CoreError::Cycle(vec![NodeId(1)])),
        "a box feeding itself"
    );
}

#[test]
fn renders_interleaved_frames_offline() {
    let mut renderer = Renderer::new(
        build(
            vec![
                node(1, "ramp~"),
                with_param(node(2, "const~"), "value", -1.0),
                node(3, "out~"),
            ],
            vec![wire(1, 0, 3, 0), wire(2, 0, 3, 1)],
        )
        .expect("valid patch"),
    );
    let samples = renderer.render(130).expect("small render");
    assert_eq!(samples.len(), 260, "130 frames of 2 channels");
    assert_eq!(
        &samples[..6],
        &[0.0, -1.0, 1.0, -1.0, 2.0, -1.0],
        "interleaved left, right"
    );
    assert_eq!(
        &samples[256..],
        &[128.0, -1.0, 129.0, -1.0],
        "continues across blocks"
    );
    let next = renderer.render(1).expect("small render");
    assert_eq!(
        next,
        vec![130.0, -1.0],
        "the next render continues exactly where the previous one stopped"
    );
}

#[test]
fn renders_nothing_for_zero_frames_or_channels() {
    let mut renderer = Renderer::new(
        build(
            vec![node(1, "ramp~"), node(2, "mono~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch"),
    );
    assert_eq!(renderer.render(0), Ok(Vec::<f32>::new()), "zero frames");
    assert_eq!(
        renderer.render(64).expect("small render"),
        (0..64_u16).map(f32::from).collect::<Vec<_>>(),
        "nothing was processed before"
    );
    let mut silent = Renderer::new(build(vec![node(1, "ramp~")], vec![]).expect("valid patch"));
    assert_eq!(
        silent.render(usize::MAX),
        Ok(Vec::<f32>::new()),
        "no output channel: nothing to render, at once"
    );
}

#[test]
fn refuses_renders_too_long_for_memory() {
    let mut renderer = Renderer::new(
        build(
            vec![node(1, "ramp~"), node(2, "out~")],
            vec![wire(1, 0, 2, 0)],
        )
        .expect("valid patch"),
    );
    assert_eq!(
        renderer.render(usize::MAX),
        Err(CoreError::RenderTooLong(usize::MAX)),
        "frames times channels overflows"
    );
    assert_eq!(
        renderer.render(usize::MAX / 4),
        Err(CoreError::RenderTooLong(usize::MAX / 4)),
        "fits the arithmetic, not the memory"
    );
    assert_eq!(
        renderer.render(2).expect("small render"),
        vec![0.0, 0.0, 1.0, 0.0],
        "nothing was rendered by the refused calls"
    );
}

#[test]
fn sums_cables_in_source_order_whatever_the_document_order() {
    // Float addition is not associative: with these values, (1 + e) + e = 1 while
    // e + (e + 1) and (e + e) + 1 give the next float above 1. The sum must follow
    // the source ids (1, then 2, then 3) for every order of the cables.
    let tiny = 3.0e-8_f32;
    let nodes = || {
        vec![
            with_param(node(1, "const~"), "value", 1.0),
            with_param(node(2, "const~"), "value", tiny),
            with_param(node(3, "const~"), "value", tiny),
            node(4, "mono~"),
        ]
    };
    let in_order = (1.0_f32 + tiny) + tiny;
    assert_ne!(
        in_order.to_bits(),
        ((tiny + tiny) + 1.0_f32).to_bits(),
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
            build(nodes(), order.iter().map(|&i| cables[i]).collect()).expect("valid patch");
        let out = run(&mut plan, 1);
        assert_eq!(
            out[0][0].to_bits(),
            in_order.to_bits(),
            "cables listed as {order:?}"
        );
    }
}

#[test]
fn accepts_patches_up_to_the_limits_only() {
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
    let plan = Plan::build(&patch, &REGISTRY, rate()).expect("valid plan");
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
        Renderer::new(plan).render(1).map(|block| block.len()),
        Ok(2),
        "renders"
    );
}

#[test]
fn rejects_repeated_cables() {
    assert_eq!(
        build(
            vec![node(1, "ramp~"), node(2, "out~")],
            vec![wire(1, 0, 2, 0), wire(1, 0, 2, 1), wire(1, 0, 2, 0)]
        )
        .map(|_| ()),
        Err(CoreError::DuplicateConnection(wire(1, 0, 2, 0))),
        "the same cable twice"
    );
}

#[test]
fn renders_into_a_buffer_continuing_like_render() {
    let patch = || {
        Renderer::new(
            build(
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
    let expected = whole.render(100).expect("small render");
    let mut parts = patch();
    let mut first = [7.0_f32; 61];
    parts.render_into(&mut first);
    assert_eq!(
        first[60], 7.0,
        "the sample after the last whole frame is untouched"
    );
    let mut rest = parts.render(70).expect("small render");
    let mut actual = first[..60].to_vec();
    actual.append(&mut rest);
    assert_eq!(
        actual, expected,
        "30 frames into a buffer, then 70 rendered"
    );

    let mut silent = Renderer::new(build(vec![node(1, "ramp~")], vec![]).expect("valid patch"));
    let mut untouched = [7.0_f32; 4];
    silent.render_into(&mut untouched);
    assert_eq!(untouched, [7.0; 4], "no output: nothing written");
}

/// The bit patterns of every sample of `blocks`.
fn block_bits(blocks: &[Block]) -> Vec<u32> {
    blocks
        .iter()
        .flatten()
        .map(|sample| sample.to_bits())
        .collect()
}

#[test]
fn a_single_cable_passes_its_samples_bit_for_bit() {
    // -0.0 through one cable into a box, then through one cable to the output: unchanged.
    // Summing onto silence would turn it into +0.0 (+0.0 + -0.0 = +0.0).
    let chain = || {
        build(
            vec![
                with_param(node(1, "const~"), "value", -0.0),
                node(2, "pass~"),
                node(3, "mono~"),
            ],
            vec![wire(1, 0, 2, 0), wire(2, 0, 3, 0)],
        )
        .expect("valid patch")
    };
    assert_eq!(
        block_bits(&run(&mut chain(), 1)),
        vec![0x8000_0000; BLOCK_SIZE],
        "-0.0 at the inlet of pass~ and at the output, through process"
    );
    let rendered: Vec<u32> = Renderer::new(chain())
        .render(BLOCK_SIZE + 3)
        .expect("small render")
        .iter()
        .map(|sample| sample.to_bits())
        .collect();
    assert_eq!(
        rendered,
        vec![0x8000_0000; BLOCK_SIZE + 3],
        "and through render"
    );
}

#[test]
fn several_cables_are_summed_starting_from_the_first() {
    let sum = |first: f32, second: f32| {
        let mut plan = build(
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
        vec![0x8000_0000; BLOCK_SIZE],
        "-0.0 + -0.0 = -0.0"
    );
    assert_eq!(sum(-0.0, 0.0), vec![0; BLOCK_SIZE], "-0.0 + +0.0 = +0.0");
}

#[test]
fn several_output_boxes_are_summed_starting_from_the_first() {
    // Two outputs, a mono one and a stereo one, each fed -0.0: channel 0 sums both
    // (-0.0 + -0.0), channel 1 comes from the stereo one alone; a third slot is silent.
    let mut plan = build(
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
        vec![0x8000_0000; 2 * BLOCK_SIZE],
        "both channels keep -0.0"
    );
    assert_eq!(block_bits(&out[2..]), vec![0; BLOCK_SIZE], "+0.0 beyond");
    // A stereo output fed on its second inlet only: its first channel is silence, +0.0.
    let mut half = build(
        vec![
            with_param(node(1, "const~"), "value", -0.0),
            node(2, "out~"),
        ],
        vec![wire(1, 0, 2, 1)],
    )
    .expect("valid patch");
    assert_eq!(
        block_bits(&run(&mut half, 2)),
        [vec![0; BLOCK_SIZE], vec![0x8000_0000; BLOCK_SIZE]].concat(),
        "an unconnected inlet is +0.0, a connected one is passed as is"
    );
}
