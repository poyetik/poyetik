//! Tests of plan building and execution, using test-only boxes.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{
    BLOCK_SIZE, Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec,
    ParamSpec, Patch, Plan, PortSpec, SILENCE, SampleRate,
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
            for (s, v) in sum.iter_mut().zip(input) {
                *s += v;
            }
        }
        let mut outs = outputs.iter_mut();
        if let Some(first) = outs.next() {
            *first = sum;
        }
        if let Some(second) = outs.next() {
            for (s, v) in second.iter_mut().zip(sum) {
                *s = v * 10.0;
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

static REGISTRY: [&ObjectSpec; 6] = [&CONST, &RAMP, &ADD, &OUT, &MONO_OUT, &RATE];

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

fn with_param(mut n: Node, name: &str, value: f32) -> Node {
    let _: Option<f32> = n.params.insert(String::from(name), value);
    n
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
    Plan::build(&Patch { nodes, connections }, &REGISTRY, rate())
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
        "dependencies first, smallest id first among ready nodes"
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
    let mut plan = build(
        vec![node(1, "rate~"), node(2, "mono~")],
        vec![wire(1, 0, 2, 0)],
    )
    .expect("valid");
    assert_eq!(run(&mut plan, 1), vec![block(4800.0)], "48 kHz / 10");
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
