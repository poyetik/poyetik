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
struct Step {
    /// The node this step runs.
    node: NodeId,
    /// The box instance.
    object: Box<dyn Object>,
    /// Input scratch buffers, one per inlet, refilled before every block.
    inputs: Vec<Block>,
    /// Output buffers, one per outlet.
    outputs: Vec<Block>,
    /// For each inlet, the bus slots summed into it, in a fixed order.
    sources: Vec<Vec<usize>>,
    /// First bus slot holding this step's outputs.
    first_slot: usize,
    /// Whether the inputs of this step go to the engine output.
    is_output: bool,
}

/// A validated, ready-to-run execution plan.
#[derive(Debug)]
pub struct Plan {
    /// Steps in execution order.
    steps: Vec<Step>,
    /// Every outlet of every step, in step order: what connections read from.
    bus: Vec<Block>,
    /// Number of engine output channels.
    channels: usize,
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
}

/// Resolves a node's box type and parameter values.
fn resolve<'patch>(
    node: &'patch Node,
    registry: &[&'static ObjectSpec],
) -> Result<Resolved<'patch>, CoreError> {
    let spec = registry
        .iter()
        .copied()
        .find(|s| s.name == node.kind)
        .ok_or_else(|| CoreError::UnknownObject {
            node: node.id,
            kind: node.kind.clone(),
        })?;
    for (name, &value) in &node.params {
        let param =
            spec.params
                .iter()
                .find(|p| p.name == name)
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
        .map(|p| node.params.get(p.name).copied().unwrap_or(p.default))
        .collect();
    Ok(Resolved { node, spec, params })
}

/// Checks that a connection joins existing ports.
fn check_connection(
    connection: &Connection,
    nodes: &BTreeMap<NodeId, Resolved<'_>>,
) -> Result<(), CoreError> {
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
    Ok(())
}

/// Sorts nodes so that every node runs after the nodes feeding it.
///
/// Deterministic: among the nodes ready to run, the smallest id always runs first.
fn execution_order(
    ids: &BTreeSet<NodeId>,
    connections: &[Connection],
) -> Result<Vec<NodeId>, CoreError> {
    let sources: BTreeMap<NodeId, Vec<NodeId>> = ids
        .iter()
        .map(|&id| {
            (
                id,
                connections
                    .iter()
                    .filter(|c| c.to == id)
                    .map(|c| c.from)
                    .collect(),
            )
        })
        .collect();
    let mut placed = BTreeSet::new();
    let mut order = Vec::with_capacity(ids.len());
    // At most one node is placed per round, so `ids.len()` rounds always suffice.
    for _ in 0..ids.len() {
        let Some((&id, _)) = sources
            .iter()
            .find(|&(id, from)| !placed.contains(id) && from.iter().all(|f| placed.contains(f)))
        else {
            break;
        };
        let _: bool = placed.insert(id);
        order.push(id);
    }
    if order.len() == ids.len() {
        Ok(order)
    } else {
        Err(CoreError::Cycle(ids.difference(&placed).copied().collect()))
    }
}

impl Plan {
    /// Validates `patch` against `registry` and builds its execution plan.
    ///
    /// # Errors
    ///
    /// Returns a [`CoreError`] describing the first problem found: duplicate node,
    /// unknown box type or parameter, out-of-range parameter, invalid connection or
    /// cycle.
    pub fn build(
        patch: &Patch,
        registry: &[&'static ObjectSpec],
        rate: SampleRate,
    ) -> Result<Self, CoreError> {
        let mut nodes = BTreeMap::new();
        for node in &patch.nodes {
            if nodes.insert(node.id, resolve(node, registry)?).is_some() {
                return Err(CoreError::DuplicateNode(node.id));
            }
        }
        for connection in &patch.connections {
            check_connection(connection, &nodes)?;
        }
        let ids: BTreeSet<NodeId> = nodes.keys().copied().collect();
        let order = execution_order(&ids, &patch.connections)?;

        // Resolved nodes in execution order.
        let position: BTreeMap<NodeId, usize> = order.iter().copied().zip(0..).collect();
        let mut resolved: Vec<Resolved<'_>> = nodes.into_values().collect();
        resolved.sort_by_key(|r| position.get(&r.node.id).copied());

        // Each step's outlets occupy consecutive bus slots, in execution order.
        let first_slots: BTreeMap<NodeId, usize> = resolved
            .iter()
            .scan(0_usize, |next, r| {
                let first = *next;
                *next = next.saturating_add(r.spec.outlets.len());
                Some((r.node.id, first))
            })
            .collect();
        let bus_len = resolved.iter().map(|r| r.spec.outlets.len()).sum();

        let mut connections = patch.connections.clone();
        connections.sort_unstable();
        let slot_of = |c: &Connection| {
            first_slots
                .get(&c.from)
                .map_or(usize::MAX, |first| first.saturating_add(c.outlet))
        };
        let steps: Vec<Step> = resolved
            .iter()
            .map(|r| {
                let id = r.node.id;
                let sources = (0..r.spec.inlets.len())
                    .map(|inlet| {
                        connections
                            .iter()
                            .filter(|c| c.to == id && c.inlet == inlet)
                            .map(slot_of)
                            .collect()
                    })
                    .collect();
                Step {
                    node: id,
                    object: (r.spec.create)(rate, &r.params),
                    inputs: vec![SILENCE; r.spec.inlets.len()],
                    outputs: vec![SILENCE; r.spec.outlets.len()],
                    sources,
                    first_slot: first_slots.get(&id).copied().unwrap_or(bus_len),
                    is_output: r.spec.capabilities.contains(&Capability::AudioOutput),
                }
            })
            .collect();
        let channels = steps
            .iter()
            .filter(|s| s.is_output)
            .map(|s| s.inputs.len())
            .max()
            .unwrap_or(0);
        Ok(Self {
            steps,
            bus: vec![SILENCE; bus_len],
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
        self.steps.iter().map(|step| step.node)
    }

    /// Renders `frames` frames offline and returns them interleaved by channel
    /// ([`Plan::channels`] samples per frame).
    ///
    /// Processes whole blocks; the samples of the last block beyond `frames` are dropped.
    #[must_use]
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut samples = Vec::with_capacity(frames.saturating_mul(self.channels));
        let mut block = vec![SILENCE; self.channels];
        // A fixed number of blocks: the loop cannot run forever.
        for index in 0..frames.div_ceil(BLOCK_SIZE) {
            self.process(&mut block);
            let take = frames
                .saturating_sub(index.saturating_mul(BLOCK_SIZE))
                .min(BLOCK_SIZE);
            for frame in 0..take {
                samples.extend(block.iter().filter_map(|channel| channel.get(frame)));
            }
        }
        samples
    }

    /// Runs every box once, then writes one block per output channel into `out`.
    ///
    /// Channels beyond `out.len()` are dropped; extra blocks in `out` receive silence.
    /// Does not allocate.
    pub fn process(&mut self, out: &mut [Block]) {
        out.fill(SILENCE);
        for step in &mut self.steps {
            for (input, slots) in step.inputs.iter_mut().zip(&step.sources) {
                *input = SILENCE;
                for &slot in slots {
                    let source = self.bus.get(slot).unwrap_or(&SILENCE);
                    for (sample, value) in input.iter_mut().zip(source) {
                        *sample += value;
                    }
                }
            }
            step.object.process(&step.inputs, &mut step.outputs);
            for (slot, output) in self.bus.iter_mut().skip(step.first_slot).zip(&step.outputs) {
                *slot = *output;
            }
            if step.is_output {
                for (channel, input) in out.iter_mut().zip(&step.inputs) {
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
