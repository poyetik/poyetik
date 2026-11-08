//! Building and running the execution plan.
//!
//! A plan is built from a [`Patch`] outside the audio thread: the patch is validated,
//! its nodes are sorted in a deterministic execution order and every box instance is
//! created (all memory is reserved here). Running the plan then only reads and writes
//! buffers that already exist.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use crate::{
    BLOCK_SIZE, Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec, Patch,
    SILENCE, SampleRate,
};

/// One box instance in execution order, with its own input and output buffers.
#[derive(Debug)]
struct Instance {
    /// The node this instance runs.
    node: NodeId,
    /// The box instance.
    object: Box<dyn Object>,
    /// Input scratch buffers, one per inlet, refilled before every block.
    inputs: Vec<Block>,
    /// Output buffers, one per outlet.
    outputs: Vec<Block>,
    /// For each inlet, the bus slots summed into it, in a fixed order.
    sources: Vec<Vec<usize>>,
    /// First bus slot holding this instance's outputs.
    first_slot: usize,
    /// Whether the inputs of this instance go to the engine output.
    is_output: bool,
}

/// A validated, ready-to-run execution plan.
#[derive(Debug)]
pub struct Plan {
    /// Box instances in execution order.
    instances: Vec<Instance>,
    /// Every outlet of every instance, in execution order: what connections read from.
    bus: Vec<Block>,
    /// Number of engine output channels.
    channels: usize,
    /// The last block rendered by [`Plan::render`], one per channel.
    last: Vec<Block>,
    /// Frames of `last` already returned by [`Plan::render`]; [`BLOCK_SIZE`] when none
    /// is left.
    used: usize,
}

/// A node resolved against the registry.
#[derive(Debug)]
struct Resolved<'patch> {
    /// The node from the patch.
    node: &'patch Node,
    /// Its box type.
    spec: &'static ObjectSpec,
    /// Its parameter values, in spec order.
    params: Vec<f32>,
    /// First bus slot of its outlets (outlets occupy consecutive slots, nodes in id
    /// order).
    first_slot: usize,
}

/// Resolves a node's box type and parameter values.
fn resolve<'patch>(
    node: &'patch Node,
    registry: &[&'static ObjectSpec],
    first_slot: usize,
) -> Result<Resolved<'patch>, CoreError> {
    let spec = registry
        .iter()
        .copied()
        .find(|spec| spec.name == node.kind)
        .ok_or_else(|| CoreError::UnknownObject {
            node: node.id,
            kind: node.kind.clone(),
        })?;
    for (name, &value) in &node.params {
        let param = spec
            .params
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
            });
        }
    }
    let params = spec
        .params
        .iter()
        .map(|param| {
            node.params
                .get(param.name)
                .copied()
                .unwrap_or(param.default)
        })
        .collect();
    Ok(Resolved {
        node,
        spec,
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
    if connection.outlet >= from.spec.outlets.len() {
        return Err(CoreError::InvalidOutlet {
            node: connection.from,
            outlet: connection.outlet,
        });
    }
    if connection.inlet >= to.spec.inlets.len() {
        return Err(CoreError::InvalidInlet {
            node: connection.to,
            inlet: connection.inlet,
        });
    }
    // In bounds: the outlet exists, so the slot is below the bus length.
    Ok(from.first_slot.saturating_add(connection.outlet))
}

/// Sorts nodes so that every node runs after the nodes feeding it (Kahn's algorithm,
/// O((N + E) log N)).
///
/// Deterministic: among the nodes ready to run, the smallest id always runs first.
/// Every cable joins two of `ids` (checked before by [`source_slot`]).
fn execution_order(
    ids: &BTreeSet<NodeId>,
    connections: &[Connection],
) -> Result<Vec<NodeId>, CoreError> {
    // For each node, how many of its incoming cables come from nodes not placed yet.
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
    // One node is placed per round, so `ids.len()` rounds always suffice: the bound is
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

impl Plan {
    /// Validates `patch` against `registry` and builds its execution plan.
    ///
    /// # Errors
    ///
    /// Returns a [`CoreError`] describing the first problem found: unknown box type or
    /// parameter, out-of-range parameter, invalid connection or cycle.
    pub fn build(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
    ) -> Result<Self, CoreError> {
        let mut nodes = BTreeMap::new();
        let mut bus_len = 0_usize;
        for node in patch.nodes() {
            let resolved = resolve(node, registry, bus_len)?;
            bus_len = bus_len.saturating_add(resolved.spec.outlets.len());
            // Ids are unique in a `Patch`: nothing is replaced.
            drop(nodes.insert(node.id, resolved));
        }
        // The slots summed into each inlet, in the canonical cable order.
        let mut sources: BTreeMap<(NodeId, usize), Vec<usize>> = BTreeMap::new();
        let connections: Vec<Connection> = patch.connections().copied().collect();
        for connection in &connections {
            let slot = source_slot(connection, &nodes)?;
            sources
                .entry((connection.to, connection.inlet))
                .or_default()
                .push(slot);
        }
        let ids: BTreeSet<NodeId> = nodes.keys().copied().collect();
        let order = execution_order(&ids, &connections)?;

        // Every id of the order is a resolved node: `remove` always finds it.
        let instances: Vec<Instance> = order
            .iter()
            .filter_map(|id| nodes.remove(id))
            .map(|resolved| {
                let id = resolved.node.id;
                Instance {
                    node: id,
                    object: (resolved.spec.create)(rate, &resolved.params),
                    inputs: vec![SILENCE; resolved.spec.inlets.len()],
                    outputs: vec![SILENCE; resolved.spec.outlets.len()],
                    sources: (0..resolved.spec.inlets.len())
                        .map(|inlet| sources.remove(&(id, inlet)).unwrap_or_default())
                        .collect(),
                    first_slot: resolved.first_slot,
                    is_output: resolved
                        .spec
                        .capabilities
                        .contains(&Capability::AudioOutput),
                }
            })
            .collect();
        let channels = instances
            .iter()
            .filter(|instance| instance.is_output)
            .map(|instance| instance.inputs.len())
            .max()
            .unwrap_or(0);
        Ok(Self {
            instances,
            bus: vec![SILENCE; bus_len],
            last: vec![SILENCE; channels],
            used: BLOCK_SIZE,
            channels,
        })
    }

    /// Number of engine output channels: the largest inlet count among the boxes with
    /// [`Capability::AudioOutput`], or zero when there is none.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// The nodes in execution order.
    pub fn execution_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.instances.iter().map(|instance| instance.node)
    }

    /// Renders the next `frames` frames offline and returns them interleaved by channel
    /// ([`Plan::channels`] samples per frame).
    ///
    /// Successive calls continue exactly where the previous one stopped, whatever their
    /// lengths: rendering `a` then `b` frames gives the same samples as rendering `a + b`
    /// (also across [`Plan::render_into`]). A plan without output renders nothing (and
    /// runs no box).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::RenderTooLong`] when the samples cannot be held in memory.
    pub fn render(&mut self, frames: usize) -> Result<Vec<f32>, CoreError> {
        let length = frames
            .checked_mul(self.channels)
            .ok_or(CoreError::RenderTooLong(frames))?;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| CoreError::RenderTooLong(frames))?;
        samples.resize(length, 0.0);
        self.render_into(&mut samples);
        Ok(samples)
    }

    /// Renders the next frames offline into `out`, interleaved by channel: as many whole
    /// frames as `out` holds ([`Plan::channels`] samples each). Samples after the last
    /// whole frame are left untouched; a plan without output writes nothing.
    ///
    /// Continues exactly where the previous render stopped, like [`Plan::render`]. Does
    /// not allocate.
    pub fn render_into(&mut self, out: &mut [f32]) {
        if self.channels == 0 {
            return;
        }
        // One frame per round: a fixed number of rounds, so it always ends.
        for frame in out.chunks_exact_mut(self.channels) {
            if self.used == BLOCK_SIZE {
                let mut last = core::mem::take(&mut self.last);
                self.process(&mut last);
                self.last = last;
                self.used = 0;
            }
            for (sample, channel) in frame.iter_mut().zip(&self.last) {
                *sample = channel.get(self.used).copied().unwrap_or(0.0);
            }
            self.used = self.used.saturating_add(1);
        }
    }

    /// Runs every box once, then writes one block per output channel into `out`.
    ///
    /// Channels beyond `out.len()` are dropped; extra blocks in `out` receive silence.
    /// Does not allocate. The samples are written as the boxes computed them: never
    /// clipped to ±1, scaled or sanitized, so that every target renders the same bits. The
    /// final output stage (limiting, clipping, handling non-finite values) belongs to the
    /// host, until the engine defines its own output policy (roadmap ticket #12).
    pub fn process(&mut self, out: &mut [Block]) {
        out.fill(SILENCE);
        for instance in &mut self.instances {
            for (input, slots) in instance.inputs.iter_mut().zip(&instance.sources) {
                *input = SILENCE;
                for &slot in slots {
                    let source = self.bus.get(slot).unwrap_or(&SILENCE);
                    for (sample, value) in input.iter_mut().zip(source) {
                        *sample += value;
                    }
                }
            }
            instance
                .object
                .process(&instance.inputs, &mut instance.outputs);
            for (slot, output) in self
                .bus
                .iter_mut()
                .skip(instance.first_slot)
                .zip(&instance.outputs)
            {
                *slot = *output;
            }
            if instance.is_output {
                for (channel, input) in out.iter_mut().zip(&instance.inputs) {
                    for (sample, value) in channel.iter_mut().zip(input) {
                        *sample += value;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
