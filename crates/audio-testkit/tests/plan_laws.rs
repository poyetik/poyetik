//! Laws of execution plans, over random patches in every precision and at every sample
//! rate of the matrix, different patches at each rate.
//!
//! The execution order is compared with a deliberately simple reference algorithm
//! (quadratic, obviously correct), and every render with a deliberately simple reference
//! renderer (the boxes run in that order, block by block, their inlets fed as the spec
//! says), bit for bit: a plan that routes, sums or hands boxes the rate wrongly fails,
//! not only one that disagrees with itself. Renders are also checked for independence
//! from the document order and from how the render is split (at a random point, and in
//! host buffers of every size of the matrix) and from the engine block, the output stage's
//! limiter included (with every detector, protected and switchable, past its latency),
//! finiteness, and for making no heap operation (allocation, reallocation or release) from
//! the first call on a fresh plan.

#![forbid(unsafe_code)]

/// Counts heap operations, so that rendering a built plan can be checked not to use the
/// heap.
#[global_allocator]
static ALLOCATOR: audio_testkit::CountingAllocator = audio_testkit::CountingAllocator;

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::{BTreeMap, BTreeSet};

    use audio_boxes::BuiltIn;
    use audio_core::host::{Delivered, ParamBounds, Raw};
    use audio_core::{
        AlgorithmVersion, BLOCK_SIZE, Binary32, Binary64, Block, Capability, Connection, CoreError,
        Create, Detector, Dispatch, ENGINE_BLOCKS, LimiterSettings, Node, NodeId, Object,
        ObjectSpec, OutputStage, ParamSpec, ParamValues, Params, Patch, Plan, PortSpec, Precision,
        Renderer, Replaced, Sample, SampleRate, TruePeakLimiter,
    };
    use audio_testkit::matrix::{ENGINE_BLOCK_SIZES, HOST_BUFFER_SIZES, PRECISIONS, sample_rates};
    use audio_testkit::runner::{fork, runner};
    use audio_testkit::{heap_bytes_during, heap_operations_during, sample_of as of};
    use proptest::collection::vec;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::{TestCaseError, TestRunner};

    const PORT: PortSpec = PortSpec {
        name: "x",
        description: "test port",
    };

    /// Every box type through its virtual table: the instance type of a plan that names no
    /// box type it calls directly. The laws run on it and on the engine's [`BuiltIn`], which
    /// routes these laws' test box types to its arm for the box types outside the registry:
    /// they prove the plan the engine ships and that arm, while `osc~` and `dac~`'s own arms
    /// are proven by the parse laws, `audio-boxes`' tests and conformance.
    type Virtual<S> = Box<dyn Object<S>>;

    /// The samples whose renders these laws compare, in either precision: rendered as their
    /// own floats, read back bit for bit.
    trait Bits: Raw<Float: Delivered<Self>> {}
    impl Bits for Binary64 {}
    impl Bits for Binary32 {}

    /// A rendered buffer of `S`'s floats, read back as samples.
    fn read_back<S: Bits>(floats: &[S::Float]) -> Vec<S> {
        floats.iter().map(|&float| S::from_raw(float)).collect()
    }

    /// Outputs a constant.
    #[derive(Debug)]
    struct Const(f64);
    impl<S: Sample, const N: usize> Object<S, N> for Const {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            for output in outputs {
                output.fill(of::<S>(self.0));
            }
        }
    }
    impl Create for Const {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] =
            &[audio_core::param!("value", "value", 0.5, -1.0, 1.0)];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self(params.first().map_or(0.0, |param| param.raw())))
        }
    }
    static CONST: ObjectSpec = ObjectSpec::new::<Const>(
        "const~",
        "constant",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    /// Outputs on both outlets a running count of its samples (state across blocks),
    /// scaled down, plus its inputs halved: stateful and order-revealing.
    #[derive(Debug)]
    struct Mix(f64);
    impl<S: Sample, const N: usize> Object<S, N> for Mix {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "S is f32 or f64: IEEE 754 arithmetic, which neither overflows nor panics"
        )]
        fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            let mut sum = [S::ZERO; N];
            for input in inputs {
                for (total, sample) in sum.iter_mut().zip(input) {
                    *total += *sample * of::<S>(0.5);
                }
            }
            for total in &mut sum {
                *total += of::<S>(self.0);
                self.0 += 1.0e-3_f64;
                // Every box flushes its outputs (decision 11, the box law that no box outputs
                // a subnormal), which delivery relies on where a channel is no sum.
                *total = total.flush();
            }
            for output in outputs {
                *output = sum;
            }
        }
    }
    impl Create for Mix {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self(0.0))
        }
    }
    static MIX: ObjectSpec = ObjectSpec::new::<Mix>(
        "mix~",
        "stateful mix",
        &[PORT, PORT],
        &[PORT, PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    /// Sends its two inlets to the engine output.
    #[derive(Debug)]
    struct Out;
    impl<S: Sample, const N: usize> Object<S, N> for Out {
        fn process(&mut self, _inputs: &[Block<S, N>], _outputs: &mut [Block<S, N>]) {}
    }
    impl Create for Out {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self)
        }
    }
    static OUT: ObjectSpec = ObjectSpec::new::<Out>(
        "out~",
        "output",
        &[PORT, PORT],
        &[],
        &[Capability::AudioOutput],
        AlgorithmVersion::FIRST,
    );

    /// Outputs a tenth of the sample rate it was created with: a plan handing boxes a
    /// wrong rate changes the samples.
    #[derive(Debug)]
    struct Rate(f64);
    impl<S: Sample, const N: usize> Object<S, N> for Rate {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            for output in outputs {
                output.fill(of::<S>(self.0));
            }
        }
    }
    impl Create for Rate {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self(f64::from(rate.hz() / 10)))
        }
    }
    static RATE: ObjectSpec = ObjectSpec::new::<Rate>(
        "rate~",
        "sample rate",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    /// Outputs +∞, -∞, NaN, then 1 in turn, frame by frame from frame 0, whatever the
    /// engine block (its place in the cycle kept across blocks): a defect no real box has,
    /// for the output stage to replace (decision 12), through every sum and every `mix~` it
    /// reaches (∞ - ∞ is NaN there).
    #[derive(Debug)]
    struct NonFinite(usize);
    impl<S: Sample, const N: usize> Object<S, N> for NonFinite {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            let values = [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 1.0_f64];
            for output in outputs {
                let cycle = values.iter().cycle().skip(self.0);
                for (sample, value) in output.iter_mut().zip(cycle) {
                    *sample = of::<S>(*value);
                }
            }
            self.0 = self
                .0
                .wrapping_add(N)
                .checked_rem(values.len())
                .unwrap_or(0);
        }
    }
    impl Create for NonFinite {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self(0))
        }
    }
    static NON_FINITE: ObjectSpec = ObjectSpec::new::<NonFinite>(
        "nonfinite~",
        "non-finite samples",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    static REGISTRY: [&ObjectSpec; 5] = [&CONST, &MIX, &OUT, &RATE, &NON_FINITE];

    /// Random patches per rate.
    const CASES: u32 = 64;

    /// A random patch: box kinds and values, cables as index pairs (may form cycles).
    type Shape = (Vec<(u8, f32)>, Vec<(u8, u8, u8, u8)>);

    /// Random shapes: up to 11 boxes of the five kinds, up to 31 cables.
    fn shapes() -> impl Strategy<Value = Shape> {
        (
            vec((0_u8..5, -1.0_f32..=1.0), 1..12),
            vec((0_u8..12, 0_u8..2, 0_u8..12, 0_u8..2), 0..32),
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
                    3 => ("rate~", BTreeMap::new()),
                    _ => ("nonfinite~", BTreeMap::new()),
                };
                Node {
                    id: NodeId(index.wrapping_mul(5).checked_rem(13).expect("13")),
                    kind: String::from(name),
                    params,
                }
            })
            .collect();
        let outlets = |node: &Node| match node.kind.as_str() {
            "const~" | "rate~" | "nonfinite~" => 1,
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

    /// The sum of `terms` frame by frame, written literally from decision 28 of
    /// `spec/engine-design.md`, apart from the engine's helper: no term is silence (+0); term
    /// `k` goes to lane `k mod 16`, each lane summing its terms in increasing `k` from -0;
    /// then lane `i` with lane `i + 8`, `i + 4`, `i + 2`, `i + 1`, for `i` from 0.
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::integer_division_remainder_used,
        reason = "the rule as the specification writes it, on indices below 16 and frames below the engine block"
    )]
    fn reduce<S: Sample>(terms: &[Block<S>]) -> Block<S> {
        if terms.is_empty() {
            return S::SILENCE;
        }
        core::array::from_fn(|frame| {
            let mut lanes = [-S::ZERO; 16];
            for (index, term) in terms.iter().enumerate() {
                lanes[index % 16] += term[frame];
            }
            for half in [8, 4, 2, 1] {
                for i in 0..half {
                    lanes[i] += lanes[i + half];
                }
            }
            lanes[0]
        })
    }

    /// The reference renderer: `frames` frames of the patch at `rate`, interleaved.
    ///
    /// Deliberately simple and independent of the plan: the boxes run in `order`, one
    /// block at a time; each inlet sums its cables in the canonical cable order (source,
    /// outlet), and is silent without one; each output channel sums the matching inlets of
    /// the output boxes, by box id, and is silent without one, every sum in the
    /// reduction order ([`reduce`]); there are as many channels as the widest output box
    /// has inlets.
    fn reference_render<S: Bits>(
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
                .find(|spec| spec.name() == node.kind)
                .expect("a registered box")
        };
        let by_id: BTreeMap<NodeId, &Node> = nodes.iter().map(|node| (node.id, node)).collect();
        let mut boxes: BTreeMap<NodeId, Box<dyn Object<S>>> = BTreeMap::new();
        for node in nodes {
            let values: Vec<Binary64> = spec(node)
                .params()
                .iter()
                .map(|param| {
                    Binary64::new(f64::from(
                        node.params
                            .get(param.name())
                            .copied()
                            .unwrap_or_else(|| param.default_value()),
                    ))
                })
                .collect();
            let params = ParamValues::of(spec(node), &values).expect("values the plan accepted");
            drop(
                boxes.insert(
                    node.id,
                    <Box<dyn Object<S>> as Dispatch<S>>::create(rate, &params)
                        .expect("a test build creates every box type"),
                ),
            );
        }
        let is_output = |node: &Node| spec(node).capabilities().contains(&Capability::AudioOutput);
        let channels = nodes
            .iter()
            .filter(|node| is_output(node))
            .map(|node| spec(node).inlets().len())
            .max()
            .unwrap_or(0);
        let cables: BTreeSet<Connection> = connections.iter().copied().collect();
        let mut outputs: BTreeMap<NodeId, Vec<Block<S>>> = BTreeMap::new();
        let mut samples = Vec::new();
        while samples.len() < frames.saturating_mul(channels) {
            let mut fed: BTreeMap<NodeId, Vec<Block<S>>> = BTreeMap::new();
            for id in order {
                let node = by_id[id];
                let inputs: Vec<Block<S>> = (0..spec(node).inlets().len())
                    .map(|inlet| {
                        let sources: Vec<Block<S>> = cables
                            .iter()
                            .filter(|cable| cable.to == *id && cable.inlet == inlet)
                            .map(|cable| outputs[&cable.from][cable.outlet])
                            .collect();
                        reduce(&sources)
                    })
                    .collect();
                // Fresh buffers every block: a box writes every sample of every outlet, so
                // what its outputs hold on entry is no part of the reference
                // (`Object::process`, which the box laws hold every box to).
                let mut produced = vec![S::SILENCE; spec(node).outlets().len()];
                boxes
                    .get_mut(id)
                    .expect("created")
                    .process(&inputs, &mut produced);
                drop(outputs.insert(*id, produced));
                if is_output(node) {
                    drop(fed.insert(*id, inputs));
                }
            }
            let summed: Vec<Block<S>> = (0..channels)
                .map(|channel| {
                    let terms: Vec<Block<S>> = fed
                        .values()
                        .filter_map(|inputs| inputs.get(channel))
                        .copied()
                        .collect();
                    reduce(&terms)
                })
                .collect();
            for frame in 0..BLOCK_SIZE {
                samples.extend(summed.iter().map(|channel| delivered(channel[frame])));
            }
        }
        samples.truncate(frames.saturating_mul(channels));
        samples
    }

    /// `sample` as a host is given it in its own precision: the output stage gives a
    /// non-finite sample (all its exponent bits set) as +0 (decision 12), and delivery is a
    /// flush point (decision 11), every sample subnormal in `S` given as a zero of its
    /// sign; written here from the bits, apart from the engine's stage and flush and from
    /// its choice of the channels it may skip.
    fn delivered<S: Bits>(sample: S) -> S {
        let (exponent, sign) = match S::PRECISION {
            Precision::Binary64 => (0x7ff0_0000_0000_0000_u64, 0x8000_0000_0000_0000_u64),
            Precision::Binary32 => (0x7f80_0000, 0x8000_0000),
        };
        let bits = sample.bits();
        if bits & exponent == exponent {
            S::from_binary64(Binary64::new(0.0))
        } else if bits & exponent == 0 && bits & !sign != 0 {
            S::from_binary64(Binary64::new(if bits & sign == 0 { 0.0 } else { -0.0 }))
        } else {
            sample
        }
    }

    /// Renders `frames` frames of a fresh plan at once: no heap operation from its very
    /// first call.
    fn first_render<S: Bits, I: Dispatch<S>>(
        plan: Plan<S, I>,
        frames: usize,
    ) -> Result<Vec<S>, String> {
        let mut renderer = Renderer::new(plan);
        let mut whole = vec![S::ZERO.raw(); frames.saturating_mul(renderer.channels())];
        let (outcome, operations) = heap_operations_during(|| renderer.render_into(&mut whole))?;
        outcome.map_err(|error| error.to_string())?;
        if operations > 0 {
            return Err(format!("{operations} heap operations in a first render"));
        }
        Ok(read_back::<S>(&whole))
    }

    /// Processes a fresh plan block by block, each block through a fresh output stage
    /// (decision 12), until `samples` interleaved samples: no heap operation from the first
    /// block on.
    fn first_blocks<S: Bits, I: Dispatch<S>>(
        mut plan: Plan<S, I>,
        samples: usize,
    ) -> Result<Vec<S>, String> {
        let mut stage = OutputStage::<S, S::Float>::new(plan.channels());
        // Each block adds a sample per channel and frame: none without a channel, so that
        // the loop below would never end.
        if plan.channels() == 0 && samples > 0 {
            return Err(format!(
                "{samples} samples asked of a plan without channels"
            ));
        }
        let mut processed =
            Vec::with_capacity(samples.saturating_add(BLOCK_SIZE.saturating_mul(plan.channels())));
        while processed.len() < samples {
            let ((), count) = heap_operations_during(|| {
                let _ = stage.apply(|block| plan.process(block));
            })?;
            if count > 0 {
                return Err(format!("{count} heap operations while processing a block"));
            }
            let block = stage.blocks();
            for frame in 0..BLOCK_SIZE {
                processed.extend(block.iter().map(|channel| channel[frame]));
            }
        }
        processed.truncate(samples);
        Ok(processed)
    }

    /// Every law, for one shape at one rate, rendering `frames` split at `split`, the plan
    /// holding its box instances as `I`.
    fn check<S: Bits, I: Dispatch<S>>(
        shape: &Shape,
        rate: SampleRate,
        frames: usize,
        split: usize,
    ) -> Result<(), String> {
        let (nodes, connections) = patch_parts(shape);
        let patch =
            Patch::new(nodes.clone(), connections.clone()).map_err(|error| error.to_string())?;
        let built =
            Plan::<S, I>::build(&patch, &REGISTRY, rate, audio_core::ExecutionMode::Offline);
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
                Plan::<S, I>::build(&patch, &REGISTRY, rate, audio_core::ExecutionMode::Offline)
                    .map_err(|error| error.to_string())?,
            );
            let mut parts = vec![S::ZERO.raw(); whole.len()];
            for buffer in parts.chunks_mut(size.saturating_mul(chunked.channels()).max(1)) {
                chunked
                    .render_into(buffer)
                    .map_err(|error| error.to_string())?;
            }
            if bits(&read_back::<S>(&parts)) != bits(&whole) {
                return Err(format!(
                    "rendering in buffers of {size} frames changes the samples"
                ));
            }
        }

        // Rendered at every engine block of the matrix but the largest, raw and through a
        // limiter: the same samples (the engine-block law, decision 2), those of the default
        // size the plan's.
        let default_size =
            engine_block_law::<S>(&patch, &REGISTRY, rate, frames, Limiting::RANDOM_PATCHES)?
                .samples;
        if bits(&default_size) != bits(&whole) {
            return Err("the engine's instance type differs from the plan's".to_owned());
        }

        // Through a simulated live host, the offline samples (decision 20).
        live_host_law::<S, I>(&patch, rate, &whole)?;

        // The same patch, advanced block by block from a fresh plan: the same samples.
        let processed = first_blocks(
            Plan::<S, I>::build(&patch, &REGISTRY, rate, audio_core::ExecutionMode::Offline)
                .map_err(|error| error.to_string())?,
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
        let mut again = Renderer::<_, _, S::Float>::new(
            Plan::<S, I>::build(
                &reversed,
                &REGISTRY,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .map_err(|error| error.to_string())?,
        );
        let split_at = split.min(frames);
        let mut parts: Vec<S::Float> = again.render(split_at).map_err(|error| error.to_string())?;
        parts.extend(
            again
                .render(frames.saturating_sub(split_at))
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

    /// The live-host law (decision 20): a live plan of `patch` at `rate`, rendered in Web
    /// Audio's quanta of 128 frames, gives `whole`, the offline samples.
    fn live_host_law<S: Bits, I: Dispatch<S>>(
        patch: &Patch,
        rate: SampleRate,
        whole: &[S],
    ) -> Result<(), String> {
        let mut live = Renderer::new(
            Plan::<S, I>::build(patch, &REGISTRY, rate, audio_core::ExecutionMode::Live)
                .map_err(|error| error.to_string())?,
        );
        let mut quanta = vec![S::ZERO.raw(); whole.len()];
        for buffer in quanta.chunks_mut(128_usize.saturating_mul(live.channels()).max(1)) {
            live.render_into(buffer)
                .map_err(|error| error.to_string())?;
        }
        if bits(&read_back::<S>(&quanta)) == bits(whole) {
            Ok(())
        } else {
            Err("a live plan in a live host's quanta changes the samples".to_owned())
        }
    }

    /// What the engine-block law compared: the raw and the limited samples, those of every
    /// size, and the engine blocks it rendered at, on the engine's instance type and
    /// directly.
    struct Compared<S> {
        /// The raw samples.
        samples: Vec<S>,
        /// The samples through the limiter.
        protected: Vec<S>,
        /// The engine blocks rendered on the engine's instance type ([`BuiltIn`]), in order.
        built_in: Vec<usize>,
        /// Those rendered on [`Direct`], in order.
        direct: Vec<usize>,
    }

    /// The engine-block law (decision 2): `frames` frames of `patch` at `rate`, with the box
    /// types of `registry`, are the same at every engine block of the test matrix but the
    /// largest, raw and through the limiter of `limiting`. At the sizes a build offers on the
    /// engine's instance type, [`BuiltIn`]; at the others, where a box type through its
    /// virtual table does not exist (`CoreError::Unbuilt`), with the box types created
    /// directly ([`Direct`]). Blocks of `MAX_ENGINE_BLOCK` frames, a block of 2^20 frames
    /// per box and per render in a debug build, are too slow for every random patch: they
    /// are rendered by audio-core's, audio-boxes' and audio-host's tests (a conformance
    /// case among them).
    fn engine_block_law<S: Bits>(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
        frames: usize,
        limiting: Limiting,
    ) -> Result<Compared<S>, String> {
        let built_in = [
            at_block::<S, BuiltIn<S, 16>, 16>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 32>, 32>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 64>, 64>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 128>, 128>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 256>, 256>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 512>, 512>(patch, registry, rate, frames, limiting)?,
            at_block::<S, BuiltIn<S, 1024>, 1024>(patch, registry, rate, frames, limiting)?,
        ];
        let direct = [
            at_block::<S, Direct<S, 1>, 1>(patch, registry, rate, frames, limiting)?,
            at_block::<S, Direct<S, 441>, 441>(patch, registry, rate, frames, limiting)?,
        ];
        compared(&built_in, &direct)
    }

    /// The comparison of the engine-block law, apart from the renders it compares, so that
    /// a cheat's renders can be given to it: every render of `built_in` (on the engine's
    /// instance type) and `direct` gives the same samples as the default engine block's,
    /// raw and limited, and the same reports of the output stage.
    fn compared<S: Bits>(
        built_in: &[Rendered<S>],
        direct: &[Rendered<S>],
    ) -> Result<Compared<S>, String> {
        let mut sizes: Vec<&Rendered<S>> = built_in.iter().chain(direct).collect();
        sizes.sort_by_key(|rendered| rendered.frames);
        let default = built_in
            .iter()
            .find(|rendered| rendered.frames == BLOCK_SIZE)
            .ok_or("the default engine block is rendered")?;
        let (raw, protected) = (bits(&default.raw), bits(&default.protected));
        for rendered in sizes {
            let size = rendered.frames;
            if bits(&rendered.raw) != raw {
                return Err(format!(
                    "an engine block of {size} frames changes the samples"
                ));
            }
            if bits(&rendered.protected) != protected {
                return Err(format!(
                    "an engine block of {size} frames changes the limited samples"
                ));
            }
            if rendered.replaced != default.replaced {
                return Err(format!(
                    "an engine block of {size} frames changes what the output stage reports: \
                     {:?} for {:?}",
                    rendered.replaced, default.replaced
                ));
            }
        }
        Ok(Compared {
            samples: default.raw.clone(),
            protected: default.protected.clone(),
            built_in: built_in.iter().map(|rendered| rendered.frames).collect(),
            direct: direct.iter().map(|rendered| rendered.frames).collect(),
        })
    }

    /// The limiter of the engine-block law: its maximum lookahead, its lookahead too, in
    /// milliseconds, its detector, and its mode: protected from its first frame, or
    /// switchable, built raw and switched to protected before its first frame, so that its
    /// crossfade runs across the blocks. Its other settings are the defaults.
    #[derive(Debug, Clone, Copy)]
    struct Limiting {
        /// The maximum lookahead, and the lookahead, in milliseconds.
        lookahead: f64,
        /// The peak detector.
        detector: Detector,
        /// Whether it is built raw, then switched to protected.
        switchable: bool,
    }

    impl Limiting {
        /// The limiter of the laws over random patches: 1.5 ms, the standard detector,
        /// protected.
        const RANDOM_PATCHES: Self = Self {
            lookahead: 1.5,
            detector: Detector::Standard,
            switchable: false,
        };

        /// Its limiter at `rate` for `channels` channels, protected from its first frame or
        /// switched to protected before it.
        fn build<S: Sample>(
            self,
            rate: SampleRate,
            channels: u16,
        ) -> Result<TruePeakLimiter<S>, String> {
            let lookahead = Binary64::new(self.lookahead);
            let settings = LimiterSettings::DEFAULT
                .with_lookahead(lookahead)
                .map_err(|error| error.to_string())?;
            let mut limiter = TruePeakLimiter::new(
                rate,
                channels,
                lookahead,
                self.detector,
                settings,
                !self.switchable,
            )
            .map_err(|error| error.to_string())?;
            if self.switchable {
                limiter.set_protected(true);
            }
            Ok(limiter)
        }
    }

    /// A render of the engine-block law: its engine block, raw, then through a limiter.
    struct Rendered<S> {
        /// The engine block, in frames.
        frames: usize,
        /// The raw samples.
        raw: Vec<S>,
        /// The samples through the limiter.
        protected: Vec<S>,
        /// What each stage replaced in the frames rendered, raw then protected.
        replaced: [Replaced; 2],
    }

    /// `frames` frames of `patch` at `rate` in an engine block of `N` frames, the plan
    /// holding its box instances as `I`, with the box types of `registry`: raw, then through
    /// the limiter of `limiting`.
    fn at_block<S: Bits, I: Dispatch<S, N>, const N: usize>(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
        frames: usize,
        limiting: Limiting,
    ) -> Result<Rendered<S>, String> {
        let build = || {
            Plan::<S, I, N>::build(patch, registry, rate, audio_core::ExecutionMode::Offline)
                .map_err(|error| error.to_string())
        };
        // Each stage reports on the frames rendered only, as a render of a known length does:
        // what the last block computes past them is never delivered.
        let until = u64::try_from(frames).map_err(|error| error.to_string())?;
        let raw_plan = build()?;
        let channels = raw_plan.channels();
        let mut raw_renderer = Renderer::lasting(raw_plan, OutputStage::new(channels), until)
            .map_err(|error| error.to_string())?;
        let unlimited: Vec<S::Float> = raw_renderer
            .render(frames)
            .map_err(|error| error.to_string())?;
        let plan = build()?;
        let limiter = limiting.build(
            rate,
            u16::try_from(plan.channels()).map_err(|error| error.to_string())?,
        )?;
        let mut protected_renderer = Renderer::lasting(plan, OutputStage::limited(limiter), until)
            .map_err(|error| error.to_string())?;
        let protected: Vec<S::Float> = protected_renderer
            .render(frames)
            .map_err(|error| error.to_string())?;
        Ok(Rendered {
            frames: N,
            raw: read_back::<S>(&unlimited),
            protected: read_back::<S>(&protected),
            replaced: [raw_renderer.replaced(), protected_renderer.replaced()],
        })
    }

    /// Every box type of these laws, [`BLOCKY`] included, created directly by its own
    /// `Create::create`, at any engine block: as `audio_boxes::BuiltIn` creates the engine's
    /// own, where through their virtual table they exist only at the sizes a build holds.
    /// Told by the address of their spec; another box type is not created.
    #[derive(Debug)]
    struct Direct<S: Sample, const N: usize>(Box<dyn Object<S, N>>);
    impl<S: Sample, const N: usize> Object<S, N> for Direct<S, N> {
        fn process(&mut self, inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            self.0.process(inputs, outputs);
        }
    }
    impl<S: Sample, const N: usize> Dispatch<S, N> for Direct<S, N> {
        fn create(rate: SampleRate, values: &ParamValues) -> Option<Self> {
            let spec = values.spec();
            let create: fn(SampleRate, Params<'_>) -> Box<dyn Object<S, N>> =
                if core::ptr::eq(spec, &raw const CONST) {
                    Const::create::<S, N>
                } else if core::ptr::eq(spec, &raw const MIX) {
                    Mix::create::<S, N>
                } else if core::ptr::eq(spec, &raw const OUT) {
                    Out::create::<S, N>
                } else if core::ptr::eq(spec, &raw const RATE) {
                    Rate::create::<S, N>
                } else if core::ptr::eq(spec, &raw const NON_FINITE) {
                    NonFinite::create::<S, N>
                } else if core::ptr::eq(spec, &raw const BLOCKY) {
                    Blocky::create::<S, N>
                } else {
                    return None;
                };
            Some(Self(create(rate, values.params())))
        }
    }

    /// Runs `law` on [`CASES`] random cases at each rate of the matrix, each rate with a
    /// runner of its own forked from one random stream: different patches at each rate,
    /// and all its cases at each (a runner counts its successes across its runs).
    fn at_every_rate(
        mut law: impl FnMut(&mut TestRunner, SampleRate) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut stream = runner(CASES);
        for rate in sample_rates() {
            law(&mut fork(&mut stream, CASES), rate)
                .map_err(|problem| format!("at {} Hz: {problem}", rate.hz()))?;
        }
        Ok(())
    }

    /// Every law over random patches at every rate, computing in `S`, the plans holding
    /// their box instances as `I`.
    fn obeys_the_laws_in<S: Bits, I: Dispatch<S>>() -> Result<(), String> {
        at_every_rate(|runner, rate| {
            runner
                .run(
                    &(shapes(), 0_usize..300, 0_usize..300),
                    |(shape, frames, split)| {
                        check::<S, I>(&shape, rate, frames, split).map_err(TestCaseError::fail)
                    },
                )
                .map_err(|error| error.to_string())
        })
    }

    #[test]
    fn plans_obey_their_laws_in_every_precision_at_every_rate() {
        for precision in PRECISIONS {
            let [shipped, virtual_table] = match precision {
                Precision::Binary64 => [
                    obeys_the_laws_in::<Binary64, BuiltIn<Binary64>>(),
                    obeys_the_laws_in::<Binary64, Virtual<Binary64>>(),
                ],
                Precision::Binary32 => [
                    obeys_the_laws_in::<Binary32, BuiltIn<Binary32>>(),
                    obeys_the_laws_in::<Binary32, Virtual<Binary32>>(),
                ],
            };
            assert_eq!(
                shipped,
                Ok(()),
                "plan laws in binary{}, the engine's plan",
                precision.bits()
            );
            assert_eq!(
                virtual_table,
                Ok(()),
                "plan laws in binary{}, every box through its virtual table",
                precision.bits()
            );
        }
    }

    /// Outputs 1 at the first frame of every block it is given and 0 elsewhere: a box whose
    /// output depends on the engine block, the cheat the engine-block law refuses.
    #[derive(Debug)]
    struct Blocky;
    impl<S: Sample, const N: usize> Object<S, N> for Blocky {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            for output in outputs {
                output.fill(S::ZERO);
                for first in output.iter_mut().take(1) {
                    *first = of::<S>(1.0);
                }
            }
        }
    }
    impl Create for Blocky {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self)
        }
    }
    static BLOCKY: ObjectSpec = ObjectSpec::new::<Blocky>(
        "blocky~",
        "the first frame of each block",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    #[test]
    fn the_engine_block_law_refuses_a_box_that_sees_its_block() {
        // A box marking each block's first frame into an output box: the law sees it at the
        // first size whose blocks start elsewhere than the default's, 1 frame (created
        // directly).
        let rate = SampleRate::new(48_000).expect("valid rate");
        let node = |id: u32, kind: &str| Node {
            id: NodeId(id),
            kind: String::from(kind),
            params: BTreeMap::new(),
        };
        let cable = Connection {
            from: NodeId(1),
            outlet: 0,
            to: NodeId(2),
            inlet: 0,
        };
        let cheat = Patch::new(vec![node(1, "blocky~"), node(2, "out~")], vec![cable])
            .expect("a valid patch");
        assert_eq!(
            engine_block_law::<Binary64>(
                &cheat,
                &[&BLOCKY, &OUT],
                rate,
                200,
                Limiting::RANDOM_PATCHES
            )
            .map(|_| ()),
            Err("an engine block of 1 frames changes the samples".to_owned()),
            "the cheat refused"
        );
        // The same patch with a box that does not see its block passes, at every engine
        // block of the matrix but the largest: on the engine's instance type at each size a
        // build offers, directly at the others.
        let fair = Patch::new(vec![node(1, "const~"), node(2, "out~")], vec![cable])
            .expect("a valid patch");
        let compared = engine_block_law::<Binary64>(
            &fair,
            &[&CONST, &OUT],
            rate,
            200,
            Limiting::RANDOM_PATCHES,
        )
        .expect("the fair patch accepted");
        assert_eq!(
            compared.built_in, ENGINE_BLOCKS,
            "every size a build offers, on the engine's instance type"
        );
        assert_eq!(
            compared.built_in.len(),
            ENGINE_BLOCKS.len(),
            "one instantiation per size of ENGINE_BLOCKS"
        );
        let mut every: Vec<usize> = compared
            .built_in
            .into_iter()
            .chain(compared.direct)
            .collect();
        every.sort_unstable();
        let (largest, below) = ENGINE_BLOCK_SIZES.split_last().expect("a matrix of sizes");
        assert_eq!(
            every, below,
            "every engine block of the matrix but the largest"
        );
        assert_eq!(
            *largest,
            audio_core::MAX_ENGINE_BLOCK,
            "the largest left out"
        );
    }

    /// Random patches per rate, detector and mode of the output stage's engine-block law.
    const LIMITED_CASES: u32 = 2;

    /// Random shapes made loud, each with a plan: up to 9 random boxes, then a `nonfinite~`
    /// into both inlets of an `out~` and a `rate~` into its first. Before what the random
    /// cables add, each channel holds three samples the output stage replaces by zero (∞,
    /// -∞, NaN), then one it keeps, 1 on the right and a tenth of the rate plus 1 on the
    /// left, above the ceiling: the limiter reduces from its first frames, on a signal its
    /// detector interpolates. A shape whose cables form a cycle, which has no plan, is drawn
    /// again.
    fn loud_shapes() -> impl Strategy<Value = Shape> {
        (
            vec((0_u8..5, -1.0_f32..=1.0), 0..10),
            vec((0_u8..13, 0_u8..2, 0_u8..13, 0_u8..2), 0..32),
        )
            .prop_map(|(mut kinds, mut cables)| {
                let first = u8::try_from(kinds.len()).expect("at most 9 random boxes");
                let (rate, out) = (first.saturating_add(1), first.saturating_add(2));
                kinds.extend([(4, 0.0), (3, 0.0), (2, 0.0)]);
                cables.extend([(first, 0, out, 0), (first, 0, out, 1), (rate, 0, out, 0)]);
                (kinds, cables)
            })
            .prop_filter("a patch with a plan", |shape| {
                let (nodes, connections) = patch_parts(shape);
                Patch::new(nodes, connections).is_ok_and(|patch| {
                    Plan::<Binary64, BuiltIn<Binary64>>::build(
                        &patch,
                        &REGISTRY,
                        SampleRate::new(48_000).expect("a valid rate"),
                        audio_core::ExecutionMode::Offline,
                    )
                    .is_ok()
                })
            })
    }

    /// Frames in `milliseconds` at `rate`, rounded to the nearest, at least one: the laws'
    /// own reference, counting whole frames until the next would pass the exact duration
    /// less half a frame. A copy of audio-core's `frames_at` in its limiter's tests (this
    /// target cannot reach that one), with its own test below.
    fn frames_at(milliseconds: f64, rate: SampleRate) -> usize {
        let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
        let count = (0_u32..=u32::MAX)
            .find(|count| f64::from(*count) + 0.5_f64 > exact)
            .expect("a count");
        usize::try_from(count).expect("small").max(1)
    }

    #[test]
    fn counts_the_frames_of_a_duration_to_the_nearest_and_at_least_one() {
        let rate = SampleRate::new(48_000).expect("a rate");
        for (milliseconds, frames) in [
            (1.5_f64, 72),
            (5.0_f64, 240),
            (0.01_f64, 1),
            // Half a frame: the next one.
            (1.0_f64 / 96.0_f64, 1),
            (1.5_f64 / 48.0_f64, 2),
            (0.0_f64, 1),
        ] {
            assert_eq!(frames_at(milliseconds, rate), frames, "{milliseconds} ms");
        }
    }

    /// The latency of the limiter of `limiting` at `rate`, counted apart: its maximum
    /// lookahead in frames ([`frames_at`]) and its detector's delay, half its taps, as
    /// literals (audio-core's limiter tests pin them).
    fn latency_of(limiting: Limiting, rate: SampleRate) -> usize {
        let (_, delay) = [("standard", 6), ("precise", 16), ("mastering", 64)]
            .into_iter()
            .find(|(name, _)| *name == limiting.detector.name())
            .expect("a delay counted for each detector");
        frames_at(limiting.lookahead, rate).saturating_add(delay)
    }

    /// The default ceiling, -1 dBTP, as an amplitude in binary64, computed apart: the
    /// binary32 one, rounded toward zero, lies at or below it.
    const CEILING: f64 = 0.891_250_938_133_745_6;

    /// The engine-block law on the output stage's limiter for one random loud patch at
    /// `rate`, in `S`: its frames run `past` frames beyond the latency of the limiter of
    /// `limiting`, read from a limiter built with it once it equals the latency counted
    /// apart. Its positive control, on the default engine block ([`limited_past_the_latency`]),
    /// so that a stage left raw, or a limiter that is never built, fails it.
    fn limits_alike_past_the_latency<S: Bits>(
        shape: &Shape,
        rate: SampleRate,
        past: usize,
        limiting: Limiting,
    ) -> Result<(), String> {
        let (nodes, connections) = patch_parts(shape);
        let patch = Patch::new(nodes, connections).map_err(|error| error.to_string())?;
        let latency = limiting.build::<S>(rate, 1)?.latency();
        if latency != latency_of(limiting, rate) {
            return Err(format!(
                "a latency of {latency} frames, {} counted apart",
                latency_of(limiting, rate)
            ));
        }
        let frames = latency.saturating_add(past);
        let compared = engine_block_law::<S>(&patch, &REGISTRY, rate, frames, limiting)?;
        limited_past_the_latency(
            &compared.samples,
            &compared.protected,
            frames,
            latency,
            limiting.switchable,
        )
    }

    /// The positive control of the limiter's engine-block law, on `frames` frames of `raw`
    /// and `limited` samples (interleaved) of a loud patch: each limited sample at or below
    /// the ceiling, unless the stage is `switchable` (its crossfade from raw then runs over
    /// every frame rendered); and, once the frames reach past `latency` the first raw frame
    /// the output stage keeps (the fourth: [`loud_shapes`]), some limited sample other than
    /// the raw one it delays.
    fn limited_past_the_latency<S: Bits>(
        raw: &[S],
        limited: &[S],
        frames: usize,
        latency: usize,
        switchable: bool,
    ) -> Result<(), String> {
        let channels = limited.len().div_euclid(frames.max(1));
        if let Some(above) = limited
            .iter()
            .map(|sample| sample.to_binary64().raw().abs())
            .find(|magnitude| !switchable && *magnitude > CEILING)
        {
            return Err(format!("a limited sample of {above}, above the ceiling"));
        }
        if frames <= latency.saturating_add(3) {
            return Ok(());
        }
        let shifted = latency.saturating_mul(channels);
        let delayed = limited.get(shifted..).unwrap_or_default();
        if bits(delayed) == bits(&raw[..delayed.len().min(raw.len())]) {
            return Err("the limited samples are the raw ones, delayed".to_owned());
        }
        Ok(())
    }

    #[test]
    fn the_limiter_obeys_the_engine_block_law_past_its_latency_with_every_detector_and_mode() {
        // Decision 2 on the output stage itself: the limited samples are the same at every
        // engine block of the matrix but the largest, in every precision, at every rate,
        // through every detector, protected and switchable, at a random lookahead of its
        // whole range, over frames past the limiter's latency, so that what it gives out
        // is its own work, not the silence of its delay: up to three of the largest engine
        // blocks rendered past it, so that at every rate the limited span crosses a boundary
        // of every size.
        let mut stream = runner(LIMITED_CASES);
        for rate in sample_rates() {
            for detector in Detector::ALL {
                for switchable in [false, true] {
                    let result = fork(&mut stream, LIMITED_CASES).run(
                        &(loud_shapes(), 1_usize..=3 * 1_024, 0.5_f64..=5.0_f64),
                        |(shape, past, lookahead)| {
                            let limiting = Limiting {
                                lookahead,
                                detector,
                                switchable,
                            };
                            for precision in PRECISIONS {
                                match precision {
                                    Precision::Binary64 => {
                                        limits_alike_past_the_latency::<Binary64>(
                                            &shape, rate, past, limiting,
                                        )
                                    }
                                    Precision::Binary32 => {
                                        limits_alike_past_the_latency::<Binary32>(
                                            &shape, rate, past, limiting,
                                        )
                                    }
                                }
                                .map_err(TestCaseError::fail)?;
                            }
                            Ok(())
                        },
                    );
                    assert_eq!(
                        result.map_err(|error| error.to_string()),
                        Ok(()),
                        "{} Hz, {}, switchable {switchable}",
                        rate.hz(),
                        detector.name()
                    );
                }
            }
        }
    }

    /// The cheat the limiter's engine-block law must refuse: `frames` frames of `patch` at
    /// `rate`, with the box types of `registry`, in blocks of `N` frames on the engine's
    /// instance type, each block through a fresh stage of the limiter of `limiting`, its
    /// history lost at every block.
    fn reset_at_every_block<S: Bits, const N: usize>(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
        frames: usize,
        limiting: Limiting,
    ) -> Result<Vec<S>, String> {
        let mut plan = Plan::<S, BuiltIn<S, N>, N>::build(
            patch,
            registry,
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .map_err(|error| error.to_string())?;
        let channels = plan.channels();
        let count = u16::try_from(channels).map_err(|error| error.to_string())?;
        let mut samples = Vec::new();
        for _ in 0..frames.div_ceil(N) {
            let mut stage =
                OutputStage::<S, S::Float, N>::limited(limiting.build::<S>(rate, count)?);
            let blocks = stage.apply(|blocks| plan.process(blocks));
            for frame in 0..N {
                samples.extend(blocks.iter().map(|block| block[frame]));
            }
        }
        samples.truncate(frames.saturating_mul(channels));
        Ok(samples)
    }

    #[test]
    fn the_limiters_engine_block_law_refuses_a_limiter_reset_at_every_block() {
        // A loud patch (a tenth of the rate on both channels) through the limiter of the
        // random patches, past its latency: the renders at 16 and 64 frames pass the law
        // and its positive control; a limiter reset at every block of 16 frames, kept as
        // the cheat, is refused; and the control refuses a stage left raw.
        let rate = SampleRate::new(48_000).expect("valid rate");
        let limiting = Limiting::RANDOM_PATCHES;
        let latency = limiting
            .build::<Binary64>(rate, 1)
            .expect("a limiter")
            .latency();
        assert_eq!(latency, latency_of(limiting, rate), "counted apart");
        assert_eq!(latency, 72 + 6, "1.5 ms at 48 kHz and the standard delay");
        let node = |id: u32, kind: &str| Node {
            id: NodeId(id),
            kind: String::from(kind),
            params: BTreeMap::new(),
        };
        let cable = |inlet: usize| Connection {
            from: NodeId(1),
            outlet: 0,
            to: NodeId(2),
            inlet,
        };
        let patch = Patch::new(
            vec![node(1, "rate~"), node(2, "out~")],
            vec![cable(0), cable(1)],
        )
        .expect("a valid patch");
        let registry: [&'static ObjectSpec; 2] = [&RATE, &OUT];
        let frames = latency + 200;
        let default = at_block::<Binary64, BuiltIn<Binary64, 64>, 64>(
            &patch, &registry, rate, frames, limiting,
        )
        .expect("rendered");
        let honest = at_block::<Binary64, BuiltIn<Binary64, 16>, 16>(
            &patch, &registry, rate, frames, limiting,
        )
        .expect("rendered");
        let cheat = Rendered {
            protected: reset_at_every_block::<Binary64, 16>(
                &patch, &registry, rate, frames, limiting,
            )
            .expect("rendered"),
            raw: honest.raw.clone(),
            ..honest
        };
        let fair = compared(&[default, honest], &[]).expect("the honest renders accepted");
        assert_eq!(
            limited_past_the_latency(&fair.samples, &fair.protected, frames, latency, false),
            Ok(()),
            "limited past the latency"
        );
        let again = at_block::<Binary64, BuiltIn<Binary64, 64>, 64>(
            &patch, &registry, rate, frames, limiting,
        )
        .expect("rendered");
        assert_eq!(
            compared(&[again, cheat], &[]).map(|_| ()),
            Err("an engine block of 16 frames changes the limited samples".to_owned()),
            "the cheat refused"
        );
        // A stage left raw: the loud samples themselves, then a quiet signal delayed.
        assert_eq!(
            limited_past_the_latency(&fair.samples, &fair.samples, frames, latency, false),
            Err("a limited sample of 4800, above the ceiling".to_owned()),
            "raw, loud"
        );
        let quiet: Vec<Binary64> = (0..2 * frames)
            .map(|index| Binary64::new(f64::from(u32::try_from(index).expect("small")) / 1e4))
            .collect();
        let mut delayed = vec![Binary64::ZERO; 2 * latency];
        delayed.extend_from_slice(&quiet[..2 * (frames - latency)]);
        assert_eq!(
            limited_past_the_latency(&quiet, &delayed, frames, latency, true),
            Err("the limited samples are the raw ones, delayed".to_owned()),
            "raw, quiet and delayed"
        );
    }

    #[test]
    fn runs_every_case_at_each_rate() {
        // A runner counts the cases that passed across its runs: one runner shared by the
        // rates ran every case at the first rate and none at the others.
        let mut counts = Vec::new();
        let result = at_every_rate(|runner, rate| {
            let count = core::cell::Cell::new(0_u32);
            runner
                .run(&shapes(), |_shape| {
                    count.set(count.get().saturating_add(1));
                    Ok(())
                })
                .map_err(|error| error.to_string())?;
            counts.push((rate.hz(), count.get()));
            Ok(())
        });
        assert_eq!(result, Ok(()), "every rate");
        let expected: Vec<(u32, u32)> = sample_rates()
            .into_iter()
            .map(|rate| (rate.hz(), CASES))
            .collect();
        assert_eq!(counts, expected, "every case at each rate");
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

    /// Sums of every count around the multiples of 16 the random patches do not reach: `n`
    /// constants into one inlet, and `n` output boxes on one channel, rendered by the plan
    /// as by the reference renderer, bit for bit, in both precisions; on some count the
    /// sequential sum differs, so the comparison sees the order.
    #[test]
    fn sums_many_cables_and_outputs_like_the_reference() {
        fn sums<S: Bits>(count: u32) -> Result<bool, String> {
            // Values of every sign, their magnitudes spread from 2^-1 to 2^-60, so that
            // sums round in both precisions: sign, exponent and fraction from the index.
            let value = |index: u32| {
                let sign = (index % 2) << 31_u32;
                let exponent = (126 - index.wrapping_mul(7) % 60) << 23_u32;
                f32::from_bits(sign | exponent | (index.wrapping_mul(0x9e37) % (1 << 23)))
            };
            let consts: Vec<Node> = (1..=count)
                .map(|id| Node {
                    id: NodeId(id),
                    kind: String::from("const~"),
                    params: BTreeMap::from([(String::from("value"), value(id))]),
                })
                .collect();
            let output = |id: u32| Node {
                id: NodeId(id),
                kind: String::from("out~"),
                params: BTreeMap::new(),
            };
            let wire = |from: u32, to: u32| Connection {
                from: NodeId(from),
                outlet: 0,
                to: NodeId(to),
                inlet: 0,
            };
            let rate = SampleRate::new(8_000).map_err(|error| error.to_string())?;
            let into_one = (
                [consts.clone(), vec![output(count + 1)]].concat(),
                (1..=count)
                    .map(|id| wire(id, count + 1))
                    .collect::<Vec<_>>(),
            );
            let on_one = (
                [consts, (1..=count).map(|id| output(count + id)).collect()].concat(),
                (1..=count)
                    .map(|id| wire(id, count + id))
                    .collect::<Vec<_>>(),
            );
            let mut revealed = false;
            for (nodes, connections) in [into_one, on_one] {
                let patch = Patch::new(nodes.clone(), connections.clone())
                    .map_err(|error| error.to_string())?;
                let plan = Plan::<S, BuiltIn<S>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                )
                .map_err(|error| error.to_string())?;
                let order: Vec<NodeId> = plan.execution_order().collect();
                let expected = reference_render::<S>(&nodes, &connections, &order, rate, 1);
                let rendered = first_render(plan, 1)?;
                if bits(&rendered) != bits(&expected) {
                    return Err(format!(
                        "{count} terms: the plan differs from the reference"
                    ));
                }
                let sequential = (1..=count)
                    .map(|id| of::<S>(f64::from(value(id))))
                    .fold(S::ZERO, |total, term| total + term);
                revealed |= bits(&rendered).first() != Some(&sequential.bits());
            }
            Ok(revealed)
        }
        for precision in PRECISIONS {
            let mut revealed = false;
            for count in [2, 3, 15, 16, 17, 31, 32, 33, 48, 49] {
                revealed |= match precision {
                    Precision::Binary64 => sums::<Binary64>(count),
                    Precision::Binary32 => sums::<Binary32>(count),
                }
                .expect("the plan sums like the reference");
            }
            assert!(revealed, "the order shows in {precision:?}");
        }
    }

    /// A plan reserves exactly one set of scratch lanes, whatever it sums: a plan of no box
    /// keeps those lanes and the block of silence a sum reads for a missing source, and
    /// nothing else, on the heap, so a set reserved in every plan, or lanes moved where the
    /// count does not see them, fails here, which no comparison of two plans could show.
    #[test]
    fn holds_exactly_one_set_of_lanes() {
        /// The reduction order's lanes but the result's (decision 28 of
        /// `spec/engine-design.md`: sixteen lanes, the result in the first), in `S`: the one set
        /// of scratch lanes a plan reserves for all its sums.
        fn one_set_of_lanes<S: Sample>() -> usize {
            (16 - 1) * size_of::<Block<S>>() + size_of::<Block<S>>()
        }
        fn kept<S: Sample>() -> Result<usize, String> {
            let rate = SampleRate::new(44_100).map_err(|error| error.to_string())?;
            let patch = Patch::new(Vec::new(), Vec::new()).map_err(|error| error.to_string())?;
            let (plan, bytes) = heap_bytes_during(|| {
                Plan::<S, BuiltIn<S>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                )
            })?;
            plan.map(|_| bytes).map_err(|error| error.to_string())
        }
        assert_eq!(
            kept::<Binary64>(),
            Ok(one_set_of_lanes::<Binary64>()),
            "binary64"
        );
        assert_eq!(
            kept::<Binary32>(),
            Ok(one_set_of_lanes::<Binary32>()),
            "binary32"
        );
    }

    /// Building a plan reserves one set of scratch lanes for all its sums, however many
    /// (`sums_in_the_lanes_of_the_plan`, with `holds_exactly_one_set_of_lanes`).
    /// Checked for two terms (summed without lanes), three and seventeen, in both
    /// precisions at small sizes (one output box, whose plan fed by one cable sums nothing,
    /// and 64), and at each limit of a patch, where one set of lanes per
    /// sum would reserve about a gigabyte: the box limit in binary64, the cable limit in
    /// binary32. The bound depends on the precision only through the size of a block,
    /// which the small sizes try in both.
    #[test]
    fn reserves_one_set_of_lanes_for_every_sum() {
        /// The bytes a plan of the engine (`BuiltIn`) computing in `S` keeps on the heap once
        /// built from `outputs` output boxes whose two inlets are each fed by `fed_by` of
        /// `sources` constants.
        fn kept_by<S: Sample>(outputs: u32, sources: u32, fed_by: u32) -> Result<usize, String> {
            let node = |id: u32, kind: &str| Node {
                id: NodeId(id),
                kind: String::from(kind),
                params: BTreeMap::new(),
            };
            let nodes: Vec<Node> = (1..=sources)
                .map(|id| node(id, "const~"))
                .chain((1..=outputs).map(|output| node(sources + output, "out~")))
                .collect();
            let wires: Vec<Connection> = (1..=outputs)
                .flat_map(|output| {
                    (0..2).flat_map(move |inlet| {
                        (1..=fed_by).map(move |from| Connection {
                            from: NodeId(from),
                            outlet: 0,
                            to: NodeId(sources + output),
                            inlet,
                        })
                    })
                })
                .collect();
            let rate = SampleRate::new(44_100).map_err(|error| error.to_string())?;
            let patch = Patch::new(nodes, wires).map_err(|error| error.to_string())?;
            let (plan, bytes) = heap_bytes_during(|| {
                Plan::<S, BuiltIn<S>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                )
            })?;
            plan.map(|_| bytes).map_err(|error| error.to_string())
        }

        /// Checks that a plan whose every inlet sums `sources` cables keeps, compared with the
        /// same boxes fed by one cable per inlet, at most one word per cable more: its sums
        /// share the one set of lanes every plan holds (`holds_exactly_one_set_of_lanes`),
        /// and a second set, even reserved only for sums of three or more, is more than that.
        fn sums_in_the_lanes_of_the_plan<S: Sample>(
            outputs: u32,
            sources: u32,
        ) -> Result<(), String> {
            let (summed, single) = (
                kept_by::<S>(outputs, sources, sources)?,
                kept_by::<S>(outputs, sources, 1)?,
            );
            let cables =
                usize::try_from(outputs * 2 * sources).map_err(|error| error.to_string())?;
            let bound = cables * size_of::<usize>();
            if summed > single + bound {
                return Err(format!(
                    "{outputs} output boxes summing {sources} cables per inlet keep {} bytes more \
                     than fed by one, beyond a word per cable ({bound} bytes)",
                    summed - single
                ));
            }
            Ok(())
        }
        // One output box: no channel sums, so that the plan fed by one cable per inlet
        // sums nothing at all; 64: channel sums in both plans.
        for (outputs, sources) in [(1, 3), (1, 17), (64, 2), (64, 3), (64, 17)] {
            sums_in_the_lanes_of_the_plan::<Binary64>(outputs, sources)
                .expect("one set of lanes, binary64");
            sums_in_the_lanes_of_the_plan::<Binary32>(outputs, sources)
                .expect("one set of lanes, binary32");
        }
        // The most boxes: two sources and as many output boxes as the limit leaves.
        let boxes = u32::try_from(Patch::MAX_NODES).expect("a box count") - 2;
        sums_in_the_lanes_of_the_plan::<Binary64>(boxes, 2).expect("at the box limit");
        // The most cables of this shape, six per output box (two inlets, three sources):
        // as many output boxes as the cable limit holds.
        let cabled = u32::try_from(Patch::MAX_CONNECTIONS / 6).expect("a box count");
        sums_in_the_lanes_of_the_plan::<Binary32>(cabled, 3).expect("at the cable limit");
        let (at_boxes, at_cables) = (
            usize::try_from(boxes + 2).expect("a count"),
            usize::try_from(cabled * 6).expect("a count"),
        );
        assert_eq!(at_boxes, Patch::MAX_NODES, "at the box limit");
        assert!(
            Patch::MAX_CONNECTIONS - at_cables < 6,
            "{at_cables} cables: no more output boxes fit"
        );
        // One more of either is refused: these are the limits.
        assert_eq!(
            kept_by::<Binary64>(boxes + 1, 2, 1),
            Err("too many boxes: 65537 (at most 65536)".to_owned()),
            "one box past the limit"
        );
        assert_eq!(
            kept_by::<Binary32>(cabled + 1, 3, 3),
            Err("too many cables: 262146 (at most 262144)".to_owned()),
            "one output box of cables past the limit"
        );
    }

    /// A box's outlet costs the plan less than two blocks: its bus slot, which the cables
    /// read, is where the box writes, so a second block per outlet (a copy the box would
    /// write first) fails here. Checked by adding one cable-free box of one outlet and no
    /// inlet to plans of 1 and 64 such boxes, in both precisions, on the engine's
    /// `BuiltIn`: what the box adds beside its slot (its instance and state) is far below
    /// a block.
    #[test]
    fn writes_each_outlet_into_its_bus_slot() {
        /// The bytes a plan of the engine (`BuiltIn`) computing in `S` keeps on the heap once
        /// built from `count` unconnected constants.
        fn kept<S: Sample>(count: u32) -> Result<usize, String> {
            let nodes: Vec<Node> = (1..=count)
                .map(|id| Node {
                    id: NodeId(id),
                    kind: String::from("const~"),
                    params: BTreeMap::new(),
                })
                .collect();
            let rate = SampleRate::new(44_100).map_err(|error| error.to_string())?;
            let patch = Patch::new(nodes, Vec::new()).map_err(|error| error.to_string())?;
            let (plan, bytes) = heap_bytes_during(|| {
                Plan::<S, BuiltIn<S>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                )
            })?;
            plan.map(|_| bytes).map_err(|error| error.to_string())
        }
        fn one_more_outlet<S: Sample>(count: u32) -> Result<(), String> {
            let added = kept::<S>(count + 1)?.saturating_sub(kept::<S>(count)?);
            let bound = 2 * size_of::<Block<S>>();
            if added < bound {
                Ok(())
            } else {
                Err(format!(
                    "a box of one outlet added to {count} costs {added} bytes, not below two \
                     blocks ({bound} bytes)"
                ))
            }
        }
        for count in [1, 64] {
            one_more_outlet::<Binary64>(count).expect("binary64");
            one_more_outlet::<Binary32>(count).expect("binary32");
        }
    }

    /// Outputs the smallest subnormal of its precision: a box breaking the box law that no
    /// box outputs a subnormal, the cheat the delivery's skip trusts that law against.
    #[derive(Debug)]
    struct Leaky;
    impl<S: Sample, const N: usize> Object<S, N> for Leaky {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            let smallest = match S::PRECISION {
                Precision::Binary64 => f64::from_bits(1),
                Precision::Binary32 => f64::from(f32::from_bits(1)),
            };
            for output in outputs {
                output.fill(of::<S>(smallest));
            }
        }
    }
    impl Create for Leaky {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self)
        }
    }
    static LEAKY: ObjectSpec = ObjectSpec::new::<Leaky>(
        "leaky~",
        "outputs a subnormal",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

    #[test]
    fn delivery_skips_the_flush_only_where_the_box_law_makes_it_change_no_bit() {
        // The cheat: a box that outputs a subnormal. The box law refuses it, so no box the
        // engine ships does this; the plan's skip trusts that law: through one cable alone
        // on its channel, the subnormal is delivered in its own precision as it is (what the
        // law forbids would reach the host), while two of them summed on a channel, a sum
        // the plan does not flush, are flushed at delivery, in both precisions.
        fn delivered_bits<S: Bits>(outputs: u32) -> Result<Vec<u64>, String> {
            let registry: [&ObjectSpec; 2] = [&LEAKY, &OUT];
            let node = |id: u32, kind: &str| Node {
                id: NodeId(id),
                kind: String::from(kind),
                params: BTreeMap::new(),
            };
            let mut nodes = vec![node(1, "leaky~")];
            let mut connections = Vec::new();
            for output in 2..outputs.saturating_add(2) {
                nodes.push(node(output, "out~"));
                connections.push(Connection {
                    from: NodeId(1),
                    outlet: 0,
                    to: NodeId(output),
                    inlet: 0,
                });
            }
            let patch = Patch::new(nodes, connections).map_err(|error| error.to_string())?;
            let rate = SampleRate::new(48_000).map_err(|error| error.to_string())?;
            let plan = Plan::<S, Virtual<S>>::build(
                &patch,
                &registry,
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .map_err(|error| error.to_string())?;
            let floats: Vec<S::Float> = Renderer::new(plan)
                .render(2)
                .map_err(|error| error.to_string())?;
            Ok(bits(&read_back::<S>(&floats)))
        }
        assert!(
            audio_testkit::check_laws(&LEAKY).is_err(),
            "the box law refuses a box that outputs a subnormal"
        );
        // One output box: left channel the subnormal as it is, right channel silence.
        assert_eq!(delivered_bits::<Binary64>(1), Ok(vec![1, 0, 1, 0]));
        assert_eq!(delivered_bits::<Binary32>(1), Ok(vec![1, 0, 1, 0]));
        // Two output boxes summed on the left channel: 2 · the smallest subnormal, still
        // subnormal, flushed to +0.
        assert_eq!(delivered_bits::<Binary64>(2), Ok(vec![0; 4]));
        assert_eq!(delivered_bits::<Binary32>(2), Ok(vec![0; 4]));
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
        // Three cables, 1, -1 and 2^-60: the reduction order adds the third to the first,
        // (1 + 2^-60) - 1 = +0, where a sequential sum would keep 2^-60.
        let three = [
            node(1, "const~", Some(1.0)),
            node(2, "const~", Some(-1.0)),
            node(3, "const~", Some(8.673_617e-19)),
            node(4, "out~", None),
        ];
        let summed = reference_render::<Binary64>(
            &three,
            &[wire(1, 4, 0), wire(2, 4, 0), wire(3, 4, 0)],
            &order[..4],
            rate,
            1,
        );
        assert_eq!(bits(&summed), [0, 0], "three cables in the reduction order");
        let silent = reference_render::<Binary32>(&nodes[..1], &[], &order[..1], rate, 70);
        assert!(silent.is_empty(), "no output box, no channel");
    }

    /// Frees memory allocated when it was created, on its first call only.
    #[derive(Debug)]
    struct Freeing(Option<Vec<f32>>);
    impl<S: Sample, const N: usize> Object<S, N> for Freeing {
        fn process(&mut self, _inputs: &[Block<S, N>], outputs: &mut [Block<S, N>]) {
            drop(core::hint::black_box(self.0.take()));
            for output in outputs {
                output.fill(of::<S>(0.25));
            }
        }
    }
    impl Create for Freeing {
        const BOX_MODE: audio_core::BoxMode = audio_core::BoxMode::LiveCapable;
        const PARAMS: &'static [ParamSpec] = &[];

        fn create<S: Sample, const N: usize>(
            _rate: SampleRate,
            _params: Params<'_>,
        ) -> Box<dyn Object<S, N>> {
            Box::new(Self(Some(vec![0.0; 8])))
        }
    }
    static FREEING: ObjectSpec = ObjectSpec::new::<Freeing>(
        "free~",
        "frees memory on its first call",
        &[],
        &[PORT],
        &[],
        AlgorithmVersion::FIRST,
    );

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
        let plan = || {
            Plan::<Binary64, BuiltIn<Binary64>>::build(
                &patch,
                &[&FREEING, &OUT],
                rate,
                audio_core::ExecutionMode::Offline,
            )
            .expect("valid plan")
        };
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
        let narrow = Plan::<Binary32, BuiltIn<Binary32>>::build(
            &patch,
            &[&FREEING, &OUT],
            rate,
            audio_core::ExecutionMode::Offline,
        )
        .expect("valid plan");
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
        // Inlets with three cables or more: the reduction order is exercised, not only
        // pairs (longer sums: `sums_many_cables_and_outputs_like_the_reference`).
        let mut three = 0_u32;
        let mut kinds = BTreeSet::new();
        let result = at_every_rate(|runner, rate| {
            for _ in 0..CASES {
                let shape = shapes()
                    .new_tree(runner)
                    .map_err(|error| error.to_string())?
                    .current();
                let (nodes, connections) = patch_parts(&shape);
                cables += connections.len();
                let mut into: BTreeMap<(NodeId, usize), usize> = BTreeMap::new();
                for cable in &connections {
                    *into.entry((cable.to, cable.inlet)).or_default() += 1;
                }
                three += u32::try_from(into.values().filter(|&&count| count >= 3).count())
                    .expect("few inlets");
                kinds.extend(nodes.iter().map(|node| node.kind.clone()));
                let patch = Patch::new(nodes, connections).map_err(|error| error.to_string())?;
                match Plan::<Binary64, Virtual<Binary64>>::build(
                    &patch,
                    &REGISTRY,
                    rate,
                    audio_core::ExecutionMode::Offline,
                ) {
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
        assert!(three >= 40, "{three} inlets with three cables or more");
        assert_eq!(kinds.len(), REGISTRY.len(), "every kind of box: {kinds:?}");
    }
}
