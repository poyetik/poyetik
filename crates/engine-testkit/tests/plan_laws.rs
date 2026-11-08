//! Laws of execution plans, over random patches at every sample rate of the matrix,
//! different patches at each rate.
//!
//! The execution order is compared with a deliberately simple reference algorithm
//! (quadratic, obviously correct), and every render with a deliberately simple reference
//! renderer (the boxes run in that order, block by block, their inlets fed as the spec
//! says), bit for bit: a plan that routes, sums or hands boxes the rate wrongly fails,
//! not only one that disagrees with itself. Renders are also checked for independence
//! from the document order and from how the render is split (at a random point, and in
//! host buffers of every size of the matrix), finiteness, and for not allocating from the
//! first call on a fresh plan.

#![forbid(unsafe_code)]

/// Counts allocations, so that rendering a built plan can be checked not to allocate.
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::{BTreeMap, BTreeSet};

    use engine_core::{
        BLOCK_SIZE, Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec,
        ParamSpec, Patch, Plan, PortSpec, Renderer, SILENCE, SampleRate,
    };
    use engine_testkit::allocations_during;
    use engine_testkit::matrix::{HOST_BUFFER_SIZES, sample_rates};
    use engine_testkit::runner::runner;
    use proptest::collection::vec;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{TestCaseError, TestRunner};

    const PORT: PortSpec = PortSpec {
        name: "x",
        description: "test port",
    };

    /// Outputs a constant.
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
            default: 0.5,
            min: -1.0,
            max: 1.0,
        }],
        capabilities: &[],
        create: |_, params| Box::new(Const(params.first().copied().unwrap_or(0.0))),
    };

    /// Outputs on both outlets a running count of its samples (state across blocks),
    /// scaled down, plus its inputs halved: stateful and order-revealing.
    #[derive(Debug)]
    struct Mix(f32);
    impl Object for Mix {
        fn process(&mut self, inputs: &[Block], outputs: &mut [Block]) {
            let mut sum = [0.0_f32; BLOCK_SIZE];
            for input in inputs {
                for (total, sample) in sum.iter_mut().zip(input) {
                    *total += sample * 0.5;
                }
            }
            for total in &mut sum {
                *total += self.0;
                self.0 += 1.0e-3;
            }
            for output in outputs {
                *output = sum;
            }
        }
    }
    static MIX: ObjectSpec = ObjectSpec {
        name: "mix~",
        description: "stateful mix",
        inlets: &[PORT, PORT],
        outlets: &[PORT, PORT],
        params: &[],
        capabilities: &[],
        create: |_, _| Box::new(Mix(0.0)),
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

    /// Outputs a tenth of the sample rate it was created with: a plan handing boxes a
    /// wrong rate changes the samples.
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
        create: |rate, _| {
            Box::new(Rate(f32::from(
                u16::try_from(rate.hz() / 10).expect("below 655,360 Hz"),
            )))
        },
    };

    static REGISTRY: [&ObjectSpec; 4] = [&CONST, &MIX, &OUT, &RATE];

    /// Random patches per rate.
    const CASES: u32 = 64;

    /// A random patch: box kinds and values, cables as index pairs (may form cycles).
    type Shape = (Vec<(u8, f32)>, Vec<(u8, u8, u8, u8)>);

    /// Random shapes: up to 11 boxes of the four kinds, up to 23 cables.
    fn shapes() -> impl Strategy<Value = Shape> {
        (
            vec((0_u8..4, -1.0_f32..=1.0), 1..12),
            vec((0_u8..12, 0_u8..2, 0_u8..12, 0_u8..2), 0..24),
        )
    }

    /// `value` modulo `count` (never zero in these tests).
    fn wrap(value: u8, count: usize) -> usize {
        usize::from(value)
            .checked_rem(count)
            .expect("count is not zero")
    }

    /// The patch for a shape. Box ids are a permutation of the positions (×5 modulo
    /// 13, at most 12 boxes), so that id order differs from document order; cables are
    /// deduplicated and only join existing ports.
    fn patch_parts(shape: &Shape) -> (Vec<Node>, Vec<Connection>) {
        let (kinds, cables) = shape;
        let nodes: Vec<Node> = kinds
            .iter()
            .zip(0_u32..)
            .map(|(&(kind, value), index)| {
                let (name, params) = match kind {
                    0 => ("const~", BTreeMap::from([(String::from("value"), value)])),
                    1 => ("mix~", BTreeMap::new()),
                    2 => ("out~", BTreeMap::new()),
                    _ => ("rate~", BTreeMap::new()),
                };
                Node {
                    id: NodeId(index.wrapping_mul(5).checked_rem(13).expect("13")),
                    kind: String::from(name),
                    params,
                }
            })
            .collect();
        let outlets = |node: &Node| match node.kind.as_str() {
            "const~" | "rate~" => 1,
            "mix~" => 2,
            _ => 0,
        };
        let inlets = |node: &Node| match node.kind.as_str() {
            "mix~" | "out~" => 2,
            _ => 0,
        };
        let connections: BTreeSet<Connection> = cables
            .iter()
            .filter_map(|&(from, outlet, to, inlet)| {
                let source = &nodes[wrap(from, nodes.len())];
                let target = &nodes[wrap(to, nodes.len())];
                let (outs, ins) = (outlets(source), inlets(target));
                (outs > 0 && ins > 0).then(|| Connection {
                    from: source.id,
                    outlet: wrap(outlet, outs),
                    to: target.id,
                    inlet: wrap(inlet, ins),
                })
            })
            .collect();
        (nodes, connections.into_iter().collect())
    }

    /// The reference execution order: repeatedly place the smallest-id box whose
    /// sources are all placed. Quadratic and obviously right.
    fn reference_order(
        nodes: &[Node],
        connections: &[Connection],
    ) -> Result<Vec<NodeId>, CoreError> {
        let ids: BTreeSet<NodeId> = nodes.iter().map(|node| node.id).collect();
        let mut placed = BTreeSet::new();
        let mut order = Vec::new();
        while let Some(&id) = ids.iter().find(|id| {
            !placed.contains(*id)
                && connections
                    .iter()
                    .filter(|connection| connection.to == **id)
                    .all(|connection| placed.contains(&connection.from))
        }) {
            let _: bool = placed.insert(id);
            order.push(id);
        }
        if order.len() == ids.len() {
            Ok(order)
        } else {
            Err(CoreError::Cycle(ids.difference(&placed).copied().collect()))
        }
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|sample| sample.to_bits()).collect()
    }

    /// Adds `block` to `total`, or starts `total` with it, bit for bit, when empty.
    fn accumulate(total: &mut Option<Block>, block: &Block) {
        match total {
            None => *total = Some(*block),
            Some(sum) => {
                for (sample, value) in sum.iter_mut().zip(block) {
                    *sample += value;
                }
            }
        }
    }

    /// The reference renderer: `frames` frames of the patch at `rate`, interleaved.
    ///
    /// Deliberately simple and independent of the plan: the boxes run in `order`, one
    /// block at a time; each inlet copies its first cable and adds the others, in the
    /// canonical cable order (source, outlet), and is silent without one; each output
    /// channel is the sum of the matching inlets of the output boxes, in execution order,
    /// starting from the first, and silent without one; there are as many channels as
    /// the widest output box has inlets.
    fn reference_render(
        nodes: &[Node],
        connections: &[Connection],
        order: &[NodeId],
        rate: SampleRate,
        frames: usize,
    ) -> Vec<f32> {
        let spec = |node: &Node| {
            REGISTRY
                .iter()
                .copied()
                .find(|spec| spec.name == node.kind)
                .expect("a registered box")
        };
        let by_id: BTreeMap<NodeId, &Node> = nodes.iter().map(|node| (node.id, node)).collect();
        let mut boxes: BTreeMap<NodeId, Box<dyn Object>> = BTreeMap::new();
        for node in nodes {
            let params: Vec<f32> = spec(node)
                .params
                .iter()
                .map(|param| {
                    node.params
                        .get(param.name)
                        .copied()
                        .unwrap_or(param.default)
                })
                .collect();
            drop(boxes.insert(node.id, (spec(node).create)(rate, &params)));
        }
        let is_output = |node: &Node| spec(node).capabilities.contains(&Capability::AudioOutput);
        let channels = nodes
            .iter()
            .filter(|node| is_output(node))
            .map(|node| spec(node).inlets.len())
            .max()
            .unwrap_or(0);
        let cables: BTreeSet<Connection> = connections.iter().copied().collect();
        let mut outputs: BTreeMap<NodeId, Vec<Block>> = BTreeMap::new();
        let mut samples = Vec::new();
        while samples.len() < frames.saturating_mul(channels) {
            let mut out: Vec<Option<Block>> = vec![None; channels];
            for id in order {
                let node = by_id[id];
                let inputs: Vec<Block> = (0..spec(node).inlets.len())
                    .map(|inlet| {
                        let mut sum: Option<Block> = None;
                        for cable in cables
                            .iter()
                            .filter(|cable| cable.to == *id && cable.inlet == inlet)
                        {
                            let source = outputs[&cable.from][cable.outlet];
                            accumulate(&mut sum, &source);
                        }
                        sum.unwrap_or(SILENCE)
                    })
                    .collect();
                let mut produced = vec![SILENCE; spec(node).outlets.len()];
                boxes
                    .get_mut(id)
                    .expect("created")
                    .process(&inputs, &mut produced);
                drop(outputs.insert(*id, produced));
                if is_output(node) {
                    for (channel, input) in out.iter_mut().zip(&inputs) {
                        accumulate(channel, input);
                    }
                }
            }
            for frame in 0..BLOCK_SIZE {
                samples.extend(out.iter().map(|channel| channel.unwrap_or(SILENCE)[frame]));
            }
        }
        samples.truncate(frames.saturating_mul(channels));
        samples
    }

    /// Every law, for one shape at one rate, rendering `frames` split at `split`.
    fn check(shape: &Shape, rate: SampleRate, frames: usize, split: usize) -> Result<(), String> {
        let (nodes, connections) = patch_parts(shape);
        let patch =
            Patch::new(nodes.clone(), connections.clone()).map_err(|error| error.to_string())?;
        let built = Plan::build(&patch, &REGISTRY, rate);
        let expected = reference_order(&nodes, &connections);
        let plan = match (built, expected) {
            (Err(error), Err(reference)) if error == reference => return Ok(()),
            (Ok(plan), Ok(order)) => {
                let actual: Vec<NodeId> = plan.execution_order().collect();
                if actual != order {
                    return Err(format!("order {actual:?} instead of {order:?}"));
                }
                plan
            }
            (built, expected) => {
                return Err(format!(
                    "built {:?}, reference {expected:?}",
                    built.map(|built_plan| built_plan.execution_order().collect::<Vec<_>>())
                ));
            }
        };
        let order: Vec<NodeId> = plan.execution_order().collect();
        let reference = reference_render(&nodes, &connections, &order, rate, frames);

        // A fresh plan renders without allocating, from its very first call.
        let mut renderer = Renderer::new(plan);
        let mut whole = vec![0.0_f32; frames.saturating_mul(renderer.channels())];
        let ((), allocations) = allocations_during(|| renderer.render_into(&mut whole));
        if allocations > 0 {
            return Err(format!("{allocations} allocations in a first render"));
        }
        if bits(&whole) != bits(&reference) {
            return Err("the plan differs from the reference renderer".to_owned());
        }
        if let Some(sample) = whole.iter().find(|sample| !sample.is_finite()) {
            return Err(format!("non-finite sample {sample}"));
        }

        // Rendered in host buffers of every size of the matrix: the same samples.
        for size in HOST_BUFFER_SIZES {
            let mut chunked = Renderer::new(
                Plan::build(&patch, &REGISTRY, rate).map_err(|error| error.to_string())?,
            );
            let mut parts = vec![0.0_f32; whole.len()];
            for buffer in parts.chunks_mut(size.saturating_mul(chunked.channels()).max(1)) {
                chunked.render_into(buffer);
            }
            if bits(&parts) != bits(&whole) {
                return Err(format!(
                    "rendering in buffers of {size} frames changes the samples"
                ));
            }
        }

        // The same patch, advanced block by block from a fresh plan: the same samples,
        // and no allocation from the first block on.
        let mut blocks = Plan::build(&patch, &REGISTRY, rate).map_err(|error| error.to_string())?;
        let mut block = vec![SILENCE; blocks.channels()];
        let mut processed = Vec::with_capacity(
            whole
                .len()
                .saturating_add(BLOCK_SIZE.saturating_mul(block.len())),
        );
        while processed.len() < whole.len() {
            let ((), count) = allocations_during(|| blocks.process(&mut block));
            if count > 0 {
                return Err(format!("{count} allocations while processing a block"));
            }
            for frame in 0..BLOCK_SIZE {
                processed.extend(block.iter().map(|channel| channel[frame]));
            }
        }
        processed.truncate(whole.len());
        if bits(&processed) != bits(&whole) {
            return Err("processing block by block differs from rendering".to_owned());
        }

        // Same patch, document listed backwards, rendered in two parts: same samples.
        let mut reversed_nodes = nodes;
        reversed_nodes.reverse();
        let mut reversed_connections = connections;
        reversed_connections.reverse();
        let reversed =
            Patch::new(reversed_nodes, reversed_connections).map_err(|error| error.to_string())?;
        let mut again = Renderer::new(
            Plan::build(&reversed, &REGISTRY, rate).map_err(|error| error.to_string())?,
        );
        let split_at = split.min(frames);
        let mut parts = again.render(split_at).map_err(|error| error.to_string())?;
        parts.extend(
            again
                .render(frames.saturating_sub(split_at))
                .map_err(|error| error.to_string())?,
        );
        if bits(&parts) != bits(&whole) {
            return Err(format!(
                "render split at {split_at} or document order changes the samples"
            ));
        }
        Ok(())
    }

    /// Runs `law` on [`CASES`] random cases at each rate of the matrix, with one runner
    /// whose random stream goes on from rate to rate: different patches at each rate.
    fn at_every_rate(
        mut law: impl FnMut(&mut TestRunner, SampleRate) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut runner = runner(CASES);
        for rate in sample_rates() {
            law(&mut runner, rate).map_err(|problem| format!("at {} Hz: {problem}", rate.hz()))?;
        }
        Ok(())
    }

    #[test]
    fn plans_obey_their_laws_at_every_rate() {
        let result = at_every_rate(|runner, rate| {
            runner
                .run(
                    &(shapes(), 0_usize..300, 0_usize..300),
                    |(shape, frames, split)| {
                        check(&shape, rate, frames, split).map_err(TestCaseError::fail)
                    },
                )
                .map_err(|error| error.to_string())
        });
        assert_eq!(result, Ok(()), "plan laws");
    }

    #[test]
    fn tries_different_patches_at_each_rate() {
        let mut first_shapes = Vec::new();
        let result = at_every_rate(|runner, _| {
            let tree = shapes()
                .new_tree(runner)
                .map_err(|error| error.to_string())?;
            first_shapes.push(tree.current());
            Ok(())
        });
        assert_eq!(result, Ok(()), "every rate");
        let distinct: BTreeSet<String> = first_shapes
            .iter()
            .map(|shape| format!("{shape:?}"))
            .collect();
        assert_eq!(distinct.len(), sample_rates().len(), "{first_shapes:?}");
    }

    #[test]
    fn the_reference_renderer_follows_the_spec() {
        // const~ 0.25 and const~ 0.5 into inlet 0 of out~, rate~ into its inlet 1, and
        // nothing into a second output's inlet 0: channel 0 = 0.75, channel 1 = rate / 10.
        let node = |id: u32, kind: &str, value: Option<f32>| Node {
            id: NodeId(id),
            kind: String::from(kind),
            params: value
                .map(|value| BTreeMap::from([(String::from("value"), value)]))
                .unwrap_or_default(),
        };
        let wire = |from: u32, to: u32, inlet: usize| Connection {
            from: NodeId(from),
            outlet: 0,
            to: NodeId(to),
            inlet,
        };
        let nodes = [
            node(1, "const~", Some(0.25)),
            node(2, "const~", Some(0.5)),
            node(3, "rate~", None),
            node(4, "out~", None),
            node(5, "out~", None),
        ];
        let rate = SampleRate::new(8_000).expect("valid rate");
        let order = [NodeId(1), NodeId(2), NodeId(3), NodeId(4), NodeId(5)];
        let samples = reference_render(
            &nodes,
            &[wire(1, 4, 0), wire(2, 4, 0), wire(3, 4, 1)],
            &order,
            rate,
            70,
        );
        assert_eq!(
            samples,
            [0.75, 800.0].repeat(70),
            "summed, routed, at the rate"
        );
        let silent = reference_render(&nodes[..1], &[], &order[..1], rate, 70);
        assert_eq!(silent, Vec::<f32>::new(), "no output box, no channel");
    }

    #[test]
    fn counts_allocations() {
        let ((), allocations) = allocations_during(|| drop(Box::new(0_u8)));
        assert_eq!(allocations, 1, "the no-allocation law is not vacuous");
    }

    #[test]
    fn the_reference_order_detects_cycles_and_orders() {
        let wire = |from: u32, to: u32| Connection {
            from: NodeId(from),
            outlet: 0,
            to: NodeId(to),
            inlet: 0,
        };
        let node = |id: u32| Node {
            id: NodeId(id),
            kind: String::from("mix~"),
            params: BTreeMap::new(),
        };
        assert_eq!(
            reference_order(&[node(2), node(1), node(3)], &[wire(3, 1)]),
            Ok(vec![NodeId(2), NodeId(3), NodeId(1)]),
            "sources first, then smallest id"
        );
        assert_eq!(
            reference_order(&[node(1), node(2)], &[wire(1, 2), wire(2, 1)]),
            Err(CoreError::Cycle(vec![NodeId(1), NodeId(2)])),
            "cycle"
        );
    }

    #[test]
    fn random_patches_do_reach_cycles_and_valid_plans() {
        // The generator is useful only if it produces both outcomes and non-trivial
        // graphs with every kind of box: count them over the cases of the laws.
        let (mut cycles, mut plans, mut cables) = (0_u32, 0_u32, 0_usize);
        let mut kinds = BTreeSet::new();
        let result = at_every_rate(|runner, rate| {
            for _ in 0..CASES {
                let shape = shapes()
                    .new_tree(runner)
                    .map_err(|error| error.to_string())?
                    .current();
                let (nodes, connections) = patch_parts(&shape);
                cables += connections.len();
                kinds.extend(nodes.iter().map(|node| node.kind.clone()));
                let patch = Patch::new(nodes, connections).map_err(|error| error.to_string())?;
                match Plan::build(&patch, &REGISTRY, rate) {
                    Ok(_) => plans += 1,
                    Err(CoreError::Cycle(_)) => cycles += 1,
                    Err(other) => return Err(format!("unexpected error {other}")),
                }
            }
            Ok(())
        });
        assert_eq!(result, Ok(()), "only cycles and valid plans");
        assert!(cycles >= 40, "{cycles} cycles");
        assert!(plans >= 40, "{plans} valid plans");
        assert!(cables >= 512 * 3, "{cables} cables in all");
        assert_eq!(kinds.len(), REGISTRY.len(), "every kind of box: {kinds:?}");
    }
}
