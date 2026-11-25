//! Building and running the execution plan.
//!
//! A plan is built from a [`Patch`] outside the audio thread, with one known deviation:
//! in the browser, the engine processor builds its plan on the audio thread, once, before
//! it renders (#12 step 30). The patch is validated, its boxes are sorted in a
//! deterministic execution order and every box instance is created (all memory is
//! reserved here). Running the plan then only reads and writes buffers that already
//! exist.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::collections::btree_map::Entry;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::iter;

use crate::math::{Lanes, add_two, sum_blocks};
use crate::{
    BLOCK_SIZE, Binary64, Block, Capability, Connection, CoreError, Dispatch, ExecutionMode, Node,
    NodeId, ObjectSpec, ParamValues, Patch, Sample, SampleRate, flush,
};

/// One box instance in execution order, held as `I` ([`Dispatch`]), with its own input
/// buffers; it writes its outputs straight into its bus slots.
#[derive(Debug)]
struct Running<S: Sample, I, const N: usize> {
    /// The box this instance runs.
    node: NodeId,
    /// The box instance.
    object: I,
    /// Input buffers, one block per inlet, in one buffer on the heap read as its blocks (no
    /// block is built on the stack, whatever the engine block): a connected inlet is
    /// rewritten before every block, an unconnected one stays silent from the build on.
    inputs: Vec<S>,
    /// For each connected inlet, in inlet order, the inlet and how it is fed from the bus
    /// slots of its cables (in the canonical cable order). An unconnected inlet has none: it
    /// stays silent from the build on.
    feeds: Vec<(usize, Feed<usize>)>,
    /// First bus slot of this instance's outputs: its outlets write the `outlets` slots
    /// from there on (consecutive, and written by no other instance), handed to the box as
    /// the last block left them, which the box may not rely on (`Object::process`: it
    /// writes every sample).
    first_slot: usize,
    /// Number of outlets of the box.
    outlets: usize,
}

/// The sources of a block, in their summing order: the first, then the others. A block with
/// sources has a first one by construction.
type Sources<K> = (K, Vec<K>);

/// How a block is fed from its sources (bus slots, or the inlets of output boxes), decided
/// when the plan is built so that processing takes no decision: the reduction order of
/// decision 28 of `spec/engine-design.md` returns one term bit for bit and two as their
/// sum, so one source is a plain copy, two are added frame by frame, and only three or more
/// pay for the lanes of the reduction order.
#[derive(Debug)]
enum Feed<K> {
    /// A single source, copied bit for bit (`-0.0` and NaN payloads included).
    Single(K),
    /// Two sources, added frame by frame, the first plus the second: the reduction order of
    /// two terms (lane 0 and lane 1, joined by the last step of the tree), with no scratch.
    Two(K, K),
    /// Three or more sources, summed in the reduction order, in this order, with the plan's
    /// one set of scratch lanes, which only such a sum reaches.
    Several(Vec<K>),
}

impl<K: Copy> Feed<K> {
    /// How `first`, then `rest`, in their summing order, feed a block: a block with a feed
    /// has a first source by construction.
    fn new((first, rest): Sources<K>) -> Self {
        match *rest.as_slice() {
            [] => Self::Single(first),
            [second] => Self::Two(first, second),
            [..] => Self::Several(iter::once(first).chain(rest).collect()),
        }
    }

    /// Writes into `out` the block this feed gives, reading each source through `read`
    /// (which finds every source the plan built), with `lanes` as the scratch of a sum of
    /// three or more.
    ///
    /// `read` finds every source the plan built (`source_slot` gives every slot below the
    /// bus length; every channel source is an instance and one of its inlets, from `build`),
    /// so no source is ever missing. Were a plan built wrong, a sum of three or more would
    /// read the missing source as silence in its place (tested), the later terms keeping
    /// their lanes: the sum of the same patch with that cable silent. A single or two-term
    /// feed skips the write instead, leaving `out` as the last block left it: unreachable,
    /// and reviewed, not tested. Each arm takes the form measured the cheaper per block
    /// (`spec/benchmarks.md`, "Measured design choices"): reading as silence
    /// costs single and two-term feeds more than the skip, and saves sums of three or more
    /// instructions against it, for a larger module.
    ///
    /// The lanes reach the sum directly. The compiler then computes their addresses once
    /// per block, before the loop over the boxes: a cost per plan, which every plan with a
    /// box pays each block, a sum or not. An optimization barrier on them would make every
    /// sum of three or more pay a little each block instead: from two summing points on, the
    /// plan without the barrier does less work per block, saving more the more it sums, and
    /// its cost falls on the lightest patches, where the deadline is far (both sides dated
    /// in `spec/benchmarks.md`, "The lanes' barrier").
    fn write<'source, S: Sample + 'source, const N: usize>(
        &self,
        out: &mut Block<S, N>,
        lanes: &mut Lanes<S, N>,
        read: impl Fn(K) -> Option<&'source Block<S, N>>,
    ) {
        match self {
            Self::Single(source) => {
                // `read` finds every source the plan built: the block is always copied, in
                // place (an assignment of the array would go through a temporary on the
                // stack in a debug build, too large for a thread's stack at the largest
                // engine block).
                for (copy, block) in iter::once(out).zip(read(*source)) {
                    copy.copy_from_slice(block);
                }
            }
            Self::Two(first, second) => {
                // `read` finds both sources: the blocks are always added.
                for ((total, one), other) in iter::once(out).zip(read(*first)).zip(read(*second)) {
                    add_two(total, one, other);
                }
            }
            Self::Several(sources) => {
                // A source `read` cannot find is silence in its place.
                sum_blocks(out, lanes, sources.iter().map(|&source| read(source)));
            }
        }
    }
}

/// A validated, ready-to-run execution plan computing in `S` (the precision of the render,
/// [`Sample`]), advanced one block at a time by [`Plan::process`].
///
/// To render a number of frames offline, whatever the block boundaries, hand the plan to
/// a [`crate::Renderer`], which owns it: the two ways of advancing time can then never be
/// mixed on one plan.
///
/// `I` is how the plan holds and calls its box instances ([`Dispatch`]), always named, so
/// that no plan takes a slower path by omission: `Box<dyn Instance<S>>` calls each through
/// its virtual table, one indirect call per box per block; the engine's own box types are
/// called directly through `audio_boxes::BuiltIn` (its `EnginePlan`), which leaves the
/// virtual table to the box types loaded at run time (`spec/engine-design.md`, decision
/// 33). A plan without it does not compile:
///
/// ```compile_fail,E0107
/// fn channels(plan: &audio_core::Plan<audio_core::Binary64>) -> usize {
///     plan.channels()
/// }
/// ```
///
/// The same function naming it compiles, so the missing type is the only error above:
///
/// ```
/// use audio_core::{Binary64, Instance, Plan};
///
/// fn channels(plan: &Plan<Binary64, Box<dyn Instance<Binary64>>>) -> usize {
///     plan.channels()
/// }
/// ```
#[derive(Debug)]
pub struct Plan<S: Sample, I, const N: usize = BLOCK_SIZE> {
    /// Box instances in execution order.
    instances: Vec<Running<S, I, N>>,
    /// Every outlet of every box, boxes in id order: what connections read from.
    bus: Vec<S>,
    /// How each engine output channel is fed, one feed per channel, from the instances
    /// whose inputs go to the engine output (boxes with [`Capability::AudioOutput`]):
    /// channel `c` reads the inlets `c` of those that have one, by box id, as
    /// `(instance index, inlet)`.
    channel_feeds: Vec<Feed<(usize, usize)>>,
    /// For each engine output channel, whether its block can hold a subnormal sample in
    /// the plan's precision: whether it is a sum, of several output boxes or of several
    /// cables into the one output box inlet it reads. A channel that is not reads one box
    /// output through one cable, or silence, which never hold one (decision 11: every box
    /// flushes its outputs, the box law that no box outputs a subnormal), so delivery
    /// skips the flush there ([`Plan::channel_is_sum`]).
    channel_sums: Vec<bool>,
    /// The scratch lanes of every sum of three or more blocks (decision 28 of
    /// `spec/engine-design.md`), one set for the whole plan: reserved when it is built, so
    /// that summing never allocates, and reached only by such a sum, so that a plan without
    /// one never touches them while processing.
    lanes: Lanes<S, N>,
}

/// A box resolved against the registry.
#[derive(Debug)]
struct Resolved<'patch> {
    /// The box from the patch.
    node: &'patch Node,
    /// Its parameter values, in spec order, with its box type ([`ParamValues::spec`]): a
    /// parameter of format 1 is its binary32 value, exact in binary64.
    params: ParamValues,
    /// First bus slot of its outlets (outlets occupy consecutive slots, boxes in id
    /// order).
    first_slot: usize,
}

/// Resolves the box type and parameter values of a box.
///
/// A subnormal parameter, written or default, is a zero of its sign (decisions 5 and 11 of
/// `spec/engine-design.md`): flushed before its range is checked, and given to the box as
/// [`crate::Params`], so that a box is never given one. A parameter of format 1 is its binary32
/// value, exact in binary64, so this is also the rule "below 2^-126 in binary64, before any
/// rounding to binary32" that binary64 parameters will follow.
fn resolve<'patch>(
    node: &'patch Node,
    registry: &[&'static ObjectSpec],
    first_slot: usize,
) -> Result<Resolved<'patch>, CoreError> {
    let spec = registry
        .iter()
        .copied()
        .find(|spec| spec.name() == node.kind)
        .ok_or_else(|| CoreError::UnknownObject {
            node: node.id,
            kind: node.kind.clone(),
        })?;
    for (name, &value) in &node.params {
        let param = spec
            .params()
            .iter()
            .find(|param| param.name == name)
            .ok_or_else(|| CoreError::UnknownParam {
                node: node.id,
                name: name.clone(),
            })?;
        if !param.accepts(value) {
            return Err(CoreError::ParamOutOfRange {
                node: node.id,
                name: name.clone(),
                value,
                minimum: param.minimum,
                maximum: param.maximum,
            });
        }
    }
    let params = ParamValues::flushed(
        spec,
        spec.params()
            .iter()
            .map(|param| {
                Binary64::new(f64::from(flush::binary32(
                    node.params
                        .get(param.name)
                        .copied()
                        .unwrap_or(param.default_value),
                )))
            })
            .collect(),
    );
    Ok(Resolved {
        node,
        params,
        first_slot,
    })
}

/// Checks that a cable joins existing ports, and returns the bus slot it reads.
fn source_slot(
    connection: &Connection,
    nodes: &BTreeMap<NodeId, Resolved<'_>>,
) -> Result<usize, CoreError> {
    let from = nodes
        .get(&connection.from)
        .ok_or(CoreError::UnknownNode(connection.from))?;
    let to = nodes
        .get(&connection.to)
        .ok_or(CoreError::UnknownNode(connection.to))?;
    if connection.outlet >= from.params.spec().outlets().len() {
        return Err(CoreError::InvalidOutlet {
            node: connection.from,
            outlet: connection.outlet,
        });
    }
    if connection.inlet >= to.params.spec().inlets().len() {
        return Err(CoreError::InvalidInlet {
            node: connection.to,
            inlet: connection.inlet,
        });
    }
    // In bounds: the outlet exists, so the slot is below the bus length.
    Ok(from.first_slot.saturating_add(connection.outlet))
}

/// Sorts boxes so that every box runs after the boxes feeding it (Kahn's algorithm,
/// O((N + E) log N)).
///
/// Deterministic: among the boxes ready to run, the smallest id always runs first.
/// Every cable joins two of `ids` (checked before by [`source_slot`]).
fn execution_order(
    ids: &BTreeSet<NodeId>,
    connections: &[Connection],
) -> Result<Vec<NodeId>, CoreError> {
    // For each box, how many of its incoming cables come from boxes not placed yet.
    let mut waiting: BTreeMap<NodeId, usize> = ids.iter().map(|&id| (id, 0)).collect();
    let mut dependents: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for connection in connections {
        let count = waiting.entry(connection.to).or_default();
        *count = count.saturating_add(1);
        dependents
            .entry(connection.from)
            .or_default()
            .push(connection.to);
    }
    let mut ready: BTreeSet<NodeId> = waiting
        .iter()
        .filter(|&(_, &count)| count == 0)
        .map(|(&id, _)| id)
        .collect();
    let mut order = Vec::with_capacity(ids.len());
    // One box is placed per round, so `ids.len()` rounds always suffice: the bound is
    // the invariant, and it keeps a bug from looping forever.
    for _ in 0..ids.len() {
        let Some(id) = ready.pop_first() else {
            break;
        };
        order.push(id);
        for &next in dependents.get(&id).into_iter().flatten() {
            let count = waiting.entry(next).or_default();
            *count = count.saturating_sub(1);
            if *count == 0 {
                let _: bool = ready.insert(next);
            }
        }
    }
    if order.len() == ids.len() {
        Ok(order)
    } else {
        let placed: BTreeSet<NodeId> = order.into_iter().collect();
        Err(CoreError::Cycle(ids.difference(&placed).copied().collect()))
    }
}

impl<S: Sample, I: Dispatch<S, N>, const N: usize> Plan<S, I, N> {
    /// Validates `patch` against `registry` and builds its execution plan, computing in
    /// `S`.
    ///
    /// The plan runs in `mode` (decision 20): a live plan holds live-capable boxes only.
    ///
    /// # Errors
    ///
    /// Returns a [`CoreError`] describing the first problem found: unknown box type or
    /// parameter, out-of-range parameter, invalid connection or cycle, an offline-only box
    /// in a live plan ([`CoreError::OfflineOnly`]), or a box its instance type cannot create
    /// ([`CoreError::Unbuilt`]).
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), neither 0 nor one more:
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, ExecutionMode, Instance, Patch, Plan, SampleRate};
    /// let empty = Patch::new(vec![], vec![]).expect("an empty patch");
    /// let rate = SampleRate::new(48_000).expect("a valid rate");
    /// type Virtual = Box<dyn Instance<Binary64, 0>>;
    /// let _ = Plan::<Binary64, Virtual, 0>::build(&empty, &[], rate, ExecutionMode::Offline);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{Binary64, ExecutionMode, Instance, Patch, Plan, SampleRate};
    /// let empty = Patch::new(vec![], vec![]).expect("an empty patch");
    /// let rate = SampleRate::new(48_000).expect("a valid rate");
    /// type Virtual = Box<dyn Instance<Binary64, 1_048_577>>;
    /// let _ = Plan::<Binary64, Virtual, 1_048_577>::build(&empty, &[], rate, ExecutionMode::Offline);
    /// ```
    pub fn build(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
        mode: ExecutionMode,
    ) -> Result<Self, CoreError> {
        crate::block::engine_block::<N>();
        let mut nodes = BTreeMap::new();
        let mut bus_len = 0_usize;
        for node in patch.nodes() {
            let resolved = resolve(node, registry, bus_len)?;
            bus_len = bus_len.saturating_add(resolved.params.spec().outlets().len());
            // Ids are unique in a `Patch`: nothing is replaced.
            drop(nodes.insert(node.id, resolved));
        }
        // The slots summed into each connected inlet, in the canonical cable order: the
        // first, then the others.
        let mut sources: BTreeMap<(NodeId, usize), Sources<usize>> = BTreeMap::new();
        let connections: Vec<Connection> = patch.connections().copied().collect();
        for connection in &connections {
            let slot = source_slot(connection, &nodes)?;
            match sources.entry((connection.to, connection.inlet)) {
                Entry::Vacant(entry) => drop(entry.insert((slot, Vec::new()))),
                Entry::Occupied(mut entry) => entry.get_mut().1.push(slot),
            }
        }
        let ids: BTreeSet<NodeId> = nodes.keys().copied().collect();
        let order = execution_order(&ids, &connections)?;

        // The output boxes (boxes with `AudioOutput`), as (box id, instance index, inlet
        // count).
        let mut outputs: Vec<(NodeId, usize, usize)> = Vec::new();
        let mut instances: Vec<Running<S, I, N>> = Vec::with_capacity(order.len());
        // Every id of the order is a resolved box: `remove` always finds it.
        for resolved in order.iter().filter_map(|id| nodes.remove(id)) {
            let id = resolved.node.id;
            let inlets = resolved.params.spec().inlets().len();
            if resolved
                .params
                .spec()
                .capabilities()
                .contains(&Capability::AudioOutput)
            {
                outputs.push((id, instances.len(), inlets));
            }
            let spec = resolved.params.spec();
            if !spec.mode().runs_in(mode) {
                return Err(CoreError::OfflineOnly {
                    node: id,
                    kind: String::from(spec.name()),
                });
            }
            let created = I::create(rate, &resolved.params);
            created_or_unbuilt(created.is_some(), id, spec, N)?;
            // Created, as just checked: the push always runs. A loop over the option rather
            // than a `match` or a `let ... else`, so that this generic function holds no
            // branch on it, which each instance type would have to take both ways to be
            // covered per instantiation (#24 step 87): the refusal is the branch of
            // `created_or_unbuilt`, which is not generic, proven once for every instance type.
            for object in Option::into_iter(created) {
                instances.push(Running {
                    node: id,
                    object,
                    inputs: vec![S::ZERO; inlets.saturating_mul(N)],
                    feeds: (0..inlets)
                        .filter_map(|inlet| {
                            sources
                                .remove(&(id, inlet))
                                .map(|sources| (inlet, Feed::new(sources)))
                        })
                        .collect(),
                    first_slot: resolved.first_slot,
                    outlets: resolved.params.spec().outlets().len(),
                });
            }
        }
        // By box id (like buses, decision 6): the order a channel sums the output boxes in,
        // whatever the cables that set the execution order.
        outputs.sort_unstable_by_key(|&(id, _, _)| id);
        // The sources of each channel, in order, as (instance index, inlet): a channel is
        // opened by its first source, so it has one, and there are as many as the widest
        // output box has inlets (each output box gives its inlets 0, 1, … in order, so
        // channel `c` is the `c`-th opened).
        let mut channels: Vec<Sources<(usize, usize)>> = Vec::new();
        for &(_, index, inlets) in &outputs {
            for inlet in 0..inlets {
                match channels.get_mut(inlet) {
                    Some((_, rest)) => rest.push((index, inlet)),
                    None => channels.push(((index, inlet), Vec::new())),
                }
            }
        }
        // A channel is a sum when it reads several output boxes, or one output box inlet fed
        // by several cables; one cable, or none (silence), is not.
        let channel_sums = channels
            .iter()
            .map(|((index, inlet), rest)| {
                !rest.is_empty()
                    || instances.get(*index).is_some_and(|instance| {
                        instance
                            .feeds
                            .iter()
                            .any(|(fed, feed)| fed == inlet && !matches!(feed, Feed::Single(_)))
                    })
            })
            .collect();
        Ok(Self {
            instances,
            bus: vec![S::ZERO; bus_len.saturating_mul(N)],
            channel_feeds: channels.into_iter().map(Feed::new).collect(),
            channel_sums,
            lanes: Lanes::new(),
        })
    }

    /// Number of engine output channels: the largest inlet count among the boxes with
    /// [`Capability::AudioOutput`], or zero when there is none.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channel_feeds.len()
    }

    /// Whether output channel `channel` can hold a subnormal sample in the plan's
    /// precision, decided when the plan is built: `true` when it is a sum, of several output
    /// boxes or of several cables into the output box inlet it reads, which the plan does
    /// not flush (decision 11 of `spec/engine-design.md`); `false` when it reads one box
    /// output through one cable, flushed by its box (the box law that no box outputs a
    /// subnormal), or silence, and for a channel beyond [`Plan::channels`], which is
    /// silence. Delivery flushes a channel only where this is `true`, or where it rounds
    /// to binary32 ([`crate::host::Delivered`]): skipping the flush elsewhere changes no
    /// bit.
    #[must_use]
    pub fn channel_is_sum(&self, channel: usize) -> bool {
        self.channel_sums.get(channel).copied().unwrap_or(false)
    }

    /// Flushes, in place, the channels of `out` that are sums ([`Plan::channel_is_sum`]):
    /// a block [`Plan::process`] wrote, given to a host in the plan's own precision
    /// without a copy, as [`crate::host::Delivered::deliver`] gives each of its samples (a
    /// subnormal becomes a zero of its sign, decision 11 of `spec/engine-design.md`); the
    /// other channels are left as they are, where the flush would change no bit. A block
    /// beyond the plan's channels is silence, left as it is.
    pub fn flush_sums(&self, out: &mut [Block<S, N>]) {
        for (channel, block) in out.iter_mut().enumerate() {
            if self.channel_is_sum(channel) {
                for sample in block.iter_mut() {
                    *sample = sample.flush();
                }
            }
        }
    }

    /// The boxes in execution order.
    pub fn execution_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.instances.iter().map(|instance| instance.node)
    }

    /// Runs every box once, then writes one block per output channel into `out`.
    ///
    /// Channels beyond `out.len()` are dropped; extra blocks in `out` receive silence.
    /// The cables into an inlet, and the output boxes on a channel, are summed by the
    /// reduction order of decision 28 of `spec/engine-design.md` (cables in their canonical
    /// order, output boxes by box id): a single one passes its samples bit for
    /// bit (`-0.0` and NaN payloads included).
    /// Does not use the heap. The samples are written as the boxes computed them: never
    /// clipped to ±1, scaled or sanitized, so that every target renders the same bits. The
    /// output stage (non-finite samples replaced, the limiter) comes after the plan:
    /// [`crate::OutputStage`], which the renderer and the engine run on what it wrote
    /// (`spec/engine-design.md`, decision 12).
    pub fn process(&mut self, out: &mut [Block<S, N>]) {
        for instance in &mut self.instances {
            let bus: &[Block<S, N>] = self.bus.as_chunks::<N>().0;
            let inputs = instance.inputs.as_chunks_mut::<N>().0;
            for (inlet, feed) in &instance.feeds {
                // Every fed inlet exists, and every slot is below the bus length
                // (`source_slot`): each is found. Unreachable, and reviewed: a missing
                // inlet would keep its last block, as `Feed::write` says of its sources.
                for input in inputs.iter_mut().skip(*inlet).take(1) {
                    feed.write(input, &mut self.lanes, |slot| bus.get(slot));
                }
            }
            // The box's slots are in the bus (`build` gives every box its outlets' slots,
            // boxes in id order, and sizes the bus for all of them): they are always found,
            // so the box always runs. Unreachable, and reviewed: a plan built without them
            // would skip the box, leaving its slots as the last block left them.
            for slots in Option::into_iter(
                self.bus
                    .as_chunks_mut::<N>()
                    .0
                    .get_mut(instance.first_slot..)
                    .and_then(|rest| rest.get_mut(..instance.outlets)),
            ) {
                instance
                    .object
                    .process(instance.inputs.as_chunks::<N>().0, slots);
            }
        }
        let mut feeds = self.channel_feeds.iter();
        for channel in out.iter_mut() {
            match feeds.next() {
                // Every source is an instance and one of its inlets (`build`): each is read.
                Some(feed) => {
                    feed.write(channel, &mut self.lanes, |(index, inlet)| {
                        self.instances
                            .get(index)
                            .and_then(|instance| instance.inputs.as_chunks::<N>().0.get(inlet))
                    });
                }
                // A channel beyond `channels` has no source: silence.
                None => channel.fill(S::ZERO),
            }
        }
    }
}

/// Refuses a box its plan's instance type did not create (`created` false) in blocks of
/// `frames` frames: once for every instance type, so that the refusal is proven once.
///
/// Inlined into every plan's build, where a call cost 20 instructions more per box loaded
/// (`spec/benchmarks.md`, "The refusal of an uncreated box inlined"); its coverage stays
/// its own, being no generic function.
#[inline]
fn created_or_unbuilt(
    created: bool,
    node: NodeId,
    spec: &ObjectSpec,
    frames: usize,
) -> Result<(), CoreError> {
    if created {
        Ok(())
    } else {
        Err(CoreError::Unbuilt {
            node,
            kind: String::from(spec.name()),
            frames,
        })
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
