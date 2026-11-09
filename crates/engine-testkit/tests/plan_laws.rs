//! Laws of execution plans, over random patches in every precision and at every sample
//! rate of the matrix, different patches at each rate.
//!
//! The execution order is compared with a deliberately simple reference algorithm
//! (quadratic, obviously correct), and every render with a deliberately simple reference
//! renderer (the boxes run in that order, block by block, their inlets fed as the spec
//! says), bit for bit: a plan that routes, sums or hands boxes the rate wrongly fails,
//! not only one that disagrees with itself. Renders are also checked for independence
//! from the document order and from how the render is split (at a random point, and in
//! host buffers of every size of the matrix), finiteness, and for making no heap
//! operation (allocation, reallocation or release) from the first call on a fresh plan.

#![forbid(unsafe_code)]

/// Counts heap operations, so that rendering a built plan can be checked not to use the
/// heap.
#[global_allocator]
static ALLOCATOR: engine_testkit::CountingAllocator = engine_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::{BTreeMap, BTreeSet};

    use engine_core::host::Raw;
    use engine_core::{
        BLOCK_SIZE, Binary32, Binary64, Block, Capability, Connection, CoreError, Create,
        Delivered, Factory, Node, NodeId, Object, ObjectSpec, ParamSpec, Patch, Plan, PortSpec,
        Precision, Renderer, Sample, SampleRate,
    };
    use engine_testkit::heap_operations_during;
    use engine_testkit::matrix::{HOST_BUFFER_SIZES, PRECISIONS, sample_rates};
    use engine_testkit::runner::runner;
    use proptest::collection::vec;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{TestCaseError, TestRunner};

    const PORT: PortSpec = PortSpec {
        name: "x",
        description: "test port",
    };

    /// The samples whose renders these laws compare, in either precision: rendered as their
    /// own floats, read back bit for bit.
    trait Bits: Raw<Float: Delivered<Self>> {}
    impl Bits for Binary64 {}
    impl Bits for Binary32 {}

    /// `value`, a literal, in the precision `S`.
    fn of<S: Sample>(value: f64) -> S {
        S::from_binary64(Binary64::from_raw(value))
    }

    /// A rendered buffer of `S`'s floats, read back as samples.
    fn read_back<S: Bits>(floats: &[S::Float]) -> Vec<S> {
        floats.iter().map(|&float| S::from_raw(float)).collect()
    }

    /// Outputs a constant.
    #[derive(Debug)]
    struct Const(f64);
    impl<S: Sample> Object<S> for Const {
        fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
            for output in outputs {
                output.fill(of::<S>(self.0));
            }
        }
    }
    impl Create for Const {
        fn create<S: Sample>(_rate: SampleRate, params: &[Binary64]) -> Box<dyn Object<S>> {
            Box::new(Self(params.first().map_or(0.0, |param| param.raw())))
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
        create: Factory::new::<Const>(),
    };

    /// Outputs on both outlets a running count of its samples (state across blocks),
    /// scaled down, plus its inputs halved: stateful and order-revealing.
    #[derive(Debug)]
    struct Mix(f64);
    impl<S: Sample> Object<S> for Mix {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "S is f32 or f64: IEEE 754 arithmetic, which neither overflows nor panics"
        )]
        fn process(&mut self, inputs: &[Block<S>], outputs: &mut [Block<S>]) {
            let mut sum = S::SILENCE;
            for input in inputs {
                for (total, sample) in sum.iter_mut().zip(input) {
                    *total += *sample * of::<S>(0.5);
                }
            }
            for total in &mut sum {
                *total += of::<S>(self.0);
                self.0 += 1.0e-3_f64;
            }
            for output in outputs {
                *output = sum;
            }
        }
    }
    impl Create for Mix {
        fn create<S: Sample>(_rate: SampleRate, _params: &[Binary64]) -> Box<dyn Object<S>> {
            Box::new(Self(0.0))
        }
    }
    static MIX: ObjectSpec = ObjectSpec {
        name: "mix~",
        description: "stateful mix",
        inlets: &[PORT, PORT],
        outlets: &[PORT, PORT],
        params: &[],
        capabilities: &[],
        create: Factory::new::<Mix>(),
    };

    /// Sends its two inlets to the engine output.
    #[derive(Debug)]
    struct Out;
    impl<S: Sample> Object<S> for Out {
        fn process(&mut self, _inputs: &[Block<S>], _outputs: &mut [Block<S>]) {}
    }
    impl Create for Out {
        fn create<S: Sample>(_rate: SampleRate, _params: &[Binary64]) -> Box<dyn Object<S>> {
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
    };

    /// Outputs a tenth of the sample rate it was created with: a plan handing boxes a
    /// wrong rate changes the samples.
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
        fn create<S: Sample>(rate: SampleRate, _params: &[Binary64]) -> Box<dyn Object<S>> {
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

    fn bits<S: Bits>(samples: &[S]) -> Vec<u64> {
        samples.iter().map(|sample| sample.bits()).collect()
    }

    /// Adds `block` to `total`, or starts `total` with it, bit for bit, when empty.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "S is f32 or f64: IEEE 754 addition, which neither overflows nor panics"
    )]
    fn accumulate<S: Sample>(total: &mut Option<Block<S>>, block: &Block<S>) {
        match total {
            None => *total = Some(*block),
            Some(sum) => {
                for (sample, value) in sum.iter_mut().zip(block) {
                    *sample += *value;
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
    fn reference_render<S: Sample>(
        nodes: &[Node],
        connections: &[Connection],
        order: &[NodeId],
        rate: SampleRate,
        frames: usize,
    ) -> Vec<S> {
        let spec = |node: &Node| {
            REGISTRY
                .iter()
                .copied()
                .find(|spec| spec.name == node.kind)
                .expect("a registered box")
        };
        let by_id: BTreeMap<NodeId, &Node> = nodes.iter().map(|node| (node.id, node)).collect();
        let mut boxes: BTreeMap<NodeId, Box<dyn Object<S>>> = BTreeMap::new();
        for node in nodes {
            let params: Vec<Binary64> = spec(node)
                .params
                .iter()
                .map(|param| {
                    Binary64::new(f64::from(
                        node.params
                            .get(param.name)
                            .copied()
                            .unwrap_or(param.default),
                    ))
                })
                .collect();
            drop(boxes.insert(node.id, spec(node).create.create(rate, &params)));
        }
        let is_output = |node: &Node| spec(node).capabilities.contains(&Capability::AudioOutput);
        let channels = nodes
            .iter()
            .filter(|node| is_output(node))
            .map(|node| spec(node).inlets.len())
            .max()
            .unwrap_or(0);
        let cables: BTreeSet<Connection> = connections.iter().copied().collect();
        let mut outputs: BTreeMap<NodeId, Vec<Block<S>>> = BTreeMap::new();
        let mut samples = Vec::new();
        while samples.len() < frames.saturating_mul(channels) {
            let mut out: Vec<Option<Block<S>>> = vec![None; channels];
            for id in order {
                let node = by_id[id];
                let inputs: Vec<Block<S>> = (0..spec(node).inlets.len())
                    .map(|inlet| {
                        let mut sum: Option<Block<S>> = None;
                        for cable in cables
                            .iter()
                            .filter(|cable| cable.to == *id && cable.inlet == inlet)
                        {
                            let source = outputs[&cable.from][cable.outlet];
                            accumulate(&mut sum, &source);
                        }
                        sum.unwrap_or(S::SILENCE)
                    })
                    .collect();
                let mut produced = vec![S::SILENCE; spec(node).outlets.len()];
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
                samples.extend(
                    out.iter()
                        .map(|channel| channel.unwrap_or(S::SILENCE)[frame]),
                );
            }
        }
        samples.truncate(frames.saturating_mul(channels));
        samples
    }

    /// Renders `frames` frames of a fresh plan at once: no heap operation from its very
    /// first call.
    fn first_render<S: Bits>(plan: Plan<S>, frames: usize) -> Result<Vec<S>, String> {
        let mut renderer = Renderer::new(plan);
        let mut whole = vec![S::ZERO.raw(); frames.saturating_mul(renderer.channels())];
        let ((), operations) = heap_operations_during(|| renderer.render_into(&mut whole))?;
        if operations > 0 {
            return Err(format!("{operations} heap operations in a first render"));
        }
        Ok(read_back::<S>(&whole))
    }

    /// Processes a fresh plan block by block until `samples` interleaved samples: no
    /// heap operation from the first block on.
    fn first_blocks<S: Sample>(mut plan: Plan<S>, samples: usize) -> Result<Vec<S>, String> {
        let mut block = vec![S::SILENCE; plan.channels()];
        let mut processed =
            Vec::with_capacity(samples.saturating_add(BLOCK_SIZE.saturating_mul(block.len())));
        while processed.len() < samples {
            let ((), count) = heap_operations_during(|| plan.process(&mut block))?;
            if count > 0 {
                return Err(format!("{count} heap operations while processing a block"));
            }
            for frame in 0..BLOCK_SIZE {
                processed.extend(block.iter().map(|channel| channel[frame]));
            }
        }
        processed.truncate(samples);
        Ok(processed)
    }

    /// Every law, for one shape at one rate, rendering `frames` split at `split`.
    fn check<S: Bits>(
        shape: &Shape,
        rate: SampleRate,
        frames: usize,
        split: usize,
    ) -> Result<(), String> {
        let (nodes, connections) = patch_parts(shape);
        let patch =
            Patch::new(nodes.clone(), connections.clone()).map_err(|error| error.to_string())?;
        let built = Plan::<S>::build(&patch, &REGISTRY, rate);
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
        let reference = reference_render::<S>(&nodes, &connections, &order, rate, frames);

        let whole = first_render(plan, frames)?;
        if bits(&whole) != bits(&reference) {
            return Err("the plan differs from the reference renderer".to_owned());
        }
        if let Some(sample) = whole
            .iter()
            .find(|sample| !sample.to_binary64().raw().is_finite())
        {
            return Err(format!("non-finite sample {sample:?}"));
        }

        // Rendered in host buffers of every size of the matrix: the same samples.
        for size in HOST_BUFFER_SIZES {
            let mut chunked = Renderer::new(
                Plan::<S>::build(&patch, &REGISTRY, rate).map_err(|error| error.to_string())?,
            );
            let mut parts = vec![S::ZERO.raw(); whole.len()];
            for buffer in parts.chunks_mut(size.saturating_mul(chunked.channels()).max(1)) {
                chunked.render_into(buffer);
            }
            if bits(&read_back::<S>(&parts)) != bits(&whole) {
                return Err(format!(
                    "rendering in buffers of {size} frames changes the samples"
                ));
            }
        }

        // The same patch, advanced block by block from a fresh plan: the same samples.
        let processed = first_blocks(
            Plan::<S>::build(&patch, &REGISTRY, rate).map_err(|error| error.to_string())?,
            whole.len(),
        )?;
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
            Plan::<S>::build(&reversed, &REGISTRY, rate).map_err(|error| error.to_string())?,
        );
        let split_at = split.min(frames);
        let mut parts: Vec<S::Float> = again.render(split_at).map_err(|error| error.to_string())?;
        parts.extend(
            again
                .render::<S::Float>(frames.saturating_sub(split_at))
                .map_err(|error| error.to_string())?,
        );
        let parts = read_back::<S>(&parts);
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

    /// Every law over random patches at every rate, computing in `S`.
    fn obeys_the_laws_in<S: Bits>() -> Result<(), String> {
        at_every_rate(|runner, rate| {
            runner
                .run(
                    &(shapes(), 0_usize..300, 0_usize..300),
                    |(shape, frames, split)| {
                        check::<S>(&shape, rate, frames, split).map_err(TestCaseError::fail)
                    },
                )
                .map_err(|error| error.to_string())
        })
    }

    #[test]
    fn plans_obey_their_laws_in_every_precision_at_every_rate() {
        for precision in PRECISIONS {
            let result = match precision {
                Precision::Binary64 => obeys_the_laws_in::<Binary64>(),
                Precision::Binary32 => obeys_the_laws_in::<Binary32>(),
            };
            assert_eq!(result, Ok(()), "plan laws in binary{}", precision.bits());
        }
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
        let cables = [wire(1, 4, 0), wire(2, 4, 0), wire(3, 4, 1)];
        let samples = reference_render::<Binary64>(&nodes, &cables, &order, rate, 70);
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.raw())
                .collect::<Vec<_>>(),
            [0.75_f64, 800.0_f64].repeat(70),
            "summed, routed, at the rate"
        );
        let narrow = reference_render::<Binary32>(&nodes, &cables, &order, rate, 70);
        assert_eq!(
            narrow.iter().map(|sample| sample.raw()).collect::<Vec<_>>(),
            [0.75_f32, 800.0_f32].repeat(70),
            "in binary32 too"
        );
        let silent = reference_render::<Binary32>(&nodes[..1], &[], &order[..1], rate, 70);
        assert!(silent.is_empty(), "no output box, no channel");
    }

    /// Frees memory allocated when it was created, on its first call only.
    #[derive(Debug)]
    struct Freeing(Option<Vec<f32>>);
    impl<S: Sample> Object<S> for Freeing {
        fn process(&mut self, _inputs: &[Block<S>], outputs: &mut [Block<S>]) {
            drop(core::hint::black_box(self.0.take()));
            for output in outputs {
                output.fill(of::<S>(0.25));
            }
        }
    }
    impl Create for Freeing {
        fn create<S: Sample>(_rate: SampleRate, _params: &[Binary64]) -> Box<dyn Object<S>> {
            Box::new(Self(Some(vec![0.0; 8])))
        }
    }
    static FREEING: ObjectSpec = ObjectSpec {
        name: "free~",
        description: "frees memory on its first call",
        inlets: &[],
        outlets: &[PORT],
        params: &[],
        capabilities: &[],
        create: Factory::new::<Freeing>(),
    };

    #[test]
    fn counts_heap_operations() {
        assert_eq!(
            heap_operations_during(|| drop(Box::new(0_u8))),
            Ok(((), 2)),
            "the no-heap law is not vacuous: an allocation and its release"
        );
    }

    #[test]
    fn refuses_a_plan_whose_box_frees_memory() {
        // free~ into out~: the release in its first call is seen, rendered at once or
        // processed block by block.
        let patch = Patch::new(
            vec![
                Node {
                    id: NodeId(1),
                    kind: String::from("free~"),
                    params: BTreeMap::new(),
                },
                Node {
                    id: NodeId(2),
                    kind: String::from("out~"),
                    params: BTreeMap::new(),
                },
            ],
            vec![Connection {
                from: NodeId(1),
                outlet: 0,
                to: NodeId(2),
                inlet: 0,
            }],
        )
        .expect("valid patch");
        let rate = SampleRate::new(48_000).expect("valid rate");
        let plan = || Plan::<Binary64>::build(&patch, &[&FREEING, &OUT], rate).expect("valid plan");
        assert_eq!(
            first_render(plan(), 100),
            Err("1 heap operations in a first render".to_owned()),
            "rendered at once"
        );
        assert_eq!(
            first_blocks(plan(), 200),
            Err("1 heap operations while processing a block".to_owned()),
            "block by block"
        );
        let narrow = Plan::<Binary32>::build(&patch, &[&FREEING, &OUT], rate).expect("valid plan");
        assert_eq!(
            first_render(narrow, 100),
            Err("1 heap operations in a first render".to_owned()),
            "in binary32 too"
        );
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
                match Plan::<Binary64>::build(&patch, &REGISTRY, rate) {
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
