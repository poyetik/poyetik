//! Laws of execution plans, over random patches at every sample rate of the matrix.
//!
//! The execution order is compared with a deliberately simple reference algorithm
//! (quadratic, obviously correct); renders are checked for determinism, independence
//! from the document order, independence from how the render is split, and finiteness.

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::{BTreeMap, BTreeSet};

    use engine_core::{
        Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec, ParamSpec,
        Patch, Plan, PortSpec, SampleRate,
    };
    use engine_testkit::matrix::sample_rates;
    use proptest::collection::vec;
    use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};

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
            let mut sum = [0.0_f32; engine_core::BLOCK_SIZE];
            for input in inputs {
                for (s, v) in sum.iter_mut().zip(input) {
                    *s += v * 0.5;
                }
            }
            for s in &mut sum {
                *s += self.0;
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

    static REGISTRY: [&ObjectSpec; 3] = [&CONST, &MIX, &OUT];

    /// A random patch: node kinds and values, cables as index pairs (may form cycles).
    type Shape = (Vec<(u8, f32)>, Vec<(u8, u8, u8, u8)>);

    /// `value` modulo `count` (never zero in these tests).
    fn wrap(value: u8, count: usize) -> usize {
        usize::from(value)
            .checked_rem(count)
            .expect("count is not zero")
    }

    /// The patch for a shape. Node ids are a permutation of the positions (×5 modulo
    /// 13, at most 12 nodes), so that id order differs from document order; cables are
    /// deduplicated and only join existing ports.
    fn patch_parts(shape: &Shape) -> (Vec<Node>, Vec<Connection>) {
        let (kinds, cables) = shape;
        let nodes: Vec<Node> = kinds
            .iter()
            .zip(0_u32..)
            .map(|(&(kind, value), index)| {
                let (name, params) = match kind % 3 {
                    0 => ("const~", BTreeMap::from([(String::from("value"), value)])),
                    1 => ("mix~", BTreeMap::new()),
                    _ => ("out~", BTreeMap::new()),
                };
                Node {
                    id: NodeId(index.wrapping_mul(5).checked_rem(13).expect("13")),
                    kind: String::from(name),
                    params,
                }
            })
            .collect();
        let outlets = |node: &Node| match node.kind.as_str() {
            "const~" => 1,
            "mix~" => 2,
            _ => 0,
        };
        let inlets = |node: &Node| if node.kind == "const~" { 0 } else { 2 };
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

    /// The reference execution order: repeatedly place the smallest-id node whose
    /// sources are all placed. Quadratic and obviously right.
    fn reference_order(
        nodes: &[Node],
        connections: &[Connection],
    ) -> Result<Vec<NodeId>, CoreError> {
        let ids: BTreeSet<NodeId> = nodes.iter().map(|n| n.id).collect();
        let mut placed = BTreeSet::new();
        let mut order = Vec::new();
        while let Some(&id) = ids.iter().find(|id| {
            !placed.contains(*id)
                && connections
                    .iter()
                    .filter(|c| c.to == **id)
                    .all(|c| placed.contains(&c.from))
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
        samples.iter().map(|s| s.to_bits()).collect()
    }

    /// Every law, for one shape at one rate, rendering `frames` split at `split`.
    fn check(shape: &Shape, rate: SampleRate, frames: usize, split: usize) -> Result<(), String> {
        let (nodes, connections) = patch_parts(shape);
        let patch = Patch::new(nodes.clone(), connections.clone()).map_err(|e| e.to_string())?;
        let built = Plan::build(&patch, &REGISTRY, rate);
        let expected = reference_order(&nodes, &connections);
        let mut plan = match (built, expected) {
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
                    built.map(|p| p.execution_order().collect::<Vec<_>>())
                ));
            }
        };
        let whole = plan.render(frames).map_err(|e| e.to_string())?;

        // Same patch, document listed backwards: same samples.
        let mut reversed_nodes = nodes;
        reversed_nodes.reverse();
        let mut reversed_connections = connections;
        reversed_connections.reverse();
        let reversed =
            Patch::new(reversed_nodes, reversed_connections).map_err(|e| e.to_string())?;
        let mut again = Plan::build(&reversed, &REGISTRY, rate).map_err(|e| e.to_string())?;
        let split_at = split.min(frames);
        let mut parts = again.render(split_at).map_err(|e| e.to_string())?;
        parts.extend(
            again
                .render(frames.saturating_sub(split_at))
                .map_err(|e| e.to_string())?,
        );
        if bits(&parts) != bits(&whole) {
            return Err(format!(
                "render split at {split_at} or document order changes the samples"
            ));
        }
        if let Some(sample) = whole.iter().find(|s| !s.is_finite()) {
            return Err(format!("non-finite sample {sample}"));
        }
        Ok(())
    }

    #[test]
    fn plans_obey_their_laws_at_every_rate() {
        for rate in sample_rates() {
            let config = Config {
                cases: 64,
                ..Config::default()
            };
            let mut runner =
                TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha));
            let shapes = (
                vec((0_u8..3, -1.0_f32..=1.0), 1..12),
                vec((0_u8..12, 0_u8..2, 0_u8..12, 0_u8..2), 0..24),
            );
            let result = runner.run(
                &(shapes, 0_usize..300, 0_usize..300),
                |(shape, frames, split)| {
                    check(&shape, rate, frames, split).map_err(TestCaseError::fail)
                },
            );
            assert!(result.is_ok(), "at {} Hz: {result:?}", rate.hz());
        }
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
        // graphs: count them over the same seeded cases as the laws.
        let mut runner = TestRunner::new_with_rng(
            Config {
                cases: 64,
                ..Config::default()
            },
            TestRng::deterministic_rng(RngAlgorithm::ChaCha),
        );
        let (mut cycles, mut plans, mut cables) = (0_u32, 0_u32, 0_usize);
        let shapes = (
            vec((0_u8..3, -1.0_f32..=1.0), 1..12),
            vec((0_u8..12, 0_u8..2, 0_u8..12, 0_u8..2), 0..24),
        );
        let rate = SampleRate::HZ_48000;
        for _ in 0_u8..64_u8 {
            let tree = proptest::strategy::Strategy::new_tree(&shapes, &mut runner).expect("tree");
            let shape = proptest::strategy::ValueTree::current(&tree);
            let (nodes, connections) = patch_parts(&shape);
            cables += connections.len();
            let patch = Patch::new(nodes, connections).expect("valid patch");
            match Plan::build(&patch, &REGISTRY, rate) {
                Ok(_) => plans += 1,
                Err(CoreError::Cycle(_)) => cycles += 1,
                Err(other) => panic!("unexpected error {other}"),
            }
        }
        assert!(cycles >= 5, "{cycles} cycles");
        assert!(plans >= 5, "{plans} valid plans");
        assert!(cables >= 64 * 3, "{cables} cables in all");
    }
}
