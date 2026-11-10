//! Building and running the execution plan.
//!
//! A plan is built from a [`Patch`] outside the audio thread, with one known deviation:
//! in the browser, the engine processor builds its plan on the audio thread, once, before
//! it renders (roadmap #12). The patch is validated, its boxes are sorted in a
//! deterministic execution order and every box instance is created (all memory is
//! reserved here). Running the plan then only reads and writes buffers that already
//! exist.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Lanes, sum_blocks};
use crate::{
    Binary64, Block, Capability, Connection, CoreError, Node, NodeId, Object, ObjectSpec, Patch,
    Sample, SampleRate, flush,
};

/// One box instance in execution order, with its own input and output buffers.
#[derive(Debug)]
struct Instance<S: Sample> {
    /// The box this instance runs.
    node: NodeId,
    /// The box instance.
    object: Box<dyn Object<S>>,
    /// Input buffers, one per inlet: a connected inlet is rewritten before every block,
    /// an unconnected one stays silent from the build on.
    inputs: Vec<Block<S>>,
    /// Output buffers, one per outlet.
    outputs: Vec<Block<S>>,
    /// For each inlet, the bus slots summed into it, in a fixed order (the canonical cable
    /// order), in the reduction order of decision 28: a single cable passes its samples
    /// bit for bit.
    sources: Vec<Vec<usize>>,
    /// First bus slot holding this instance's outputs.
    first_slot: usize,
    /// Whether the inputs of this instance go to the engine output.
    is_output: bool,
}

/// A validated, ready-to-run execution plan computing in `S` (the precision of the render,
/// [`Sample`]), advanced one block at a time by [`Plan::process`].
///
/// To render a number of frames offline, whatever the block boundaries, hand the plan to
/// a [`crate::Renderer`], which owns it: the two ways of advancing time can then never be
/// mixed on one plan.
#[derive(Debug)]
pub struct Plan<S: Sample> {
    /// Box instances in execution order.
    instances: Vec<Instance<S>>,
    /// Every outlet of every box, boxes in id order: what connections read from.
    bus: Vec<Block<S>>,
    /// The instances whose inputs go to the engine output (boxes with
    /// [`Capability::AudioOutput`]), in execution order: channel `c` sums the inlets `c` of
    /// those that have one, in that order.
    outputs: Vec<usize>,
    /// Number of engine output channels.
    channels: usize,
    /// The scratch lanes of every sum of blocks, reserved here so that summing never
    /// allocates.
    lanes: Lanes<S>,
}

/// A box resolved against the registry.
#[derive(Debug)]
struct Resolved<'patch> {
    /// The box from the patch.
    node: &'patch Node,
    /// Its box type.
    spec: &'static ObjectSpec,
    /// Its parameter values, in spec order, none subnormal, in binary64 (decision 10 of
    /// `spec/engine-design.md`): a parameter of format 1 is its binary32 value, exact in
    /// binary64.
    params: Vec<Binary64>,
    /// First bus slot of its outlets (outlets occupy consecutive slots, boxes in id
    /// order).
    first_slot: usize,
}

/// Resolves the box type and parameter values of a box.
///
/// A subnormal parameter, written or default, is a zero of its sign (decisions 5 and 11 of
/// `spec/engine-design.md`): flushed before its range is checked, so that a box is never
/// given one. A parameter of format 1 is its binary32 value, exact in binary64, so this is
/// also the rule "below 2^-126 in binary64, before any rounding to binary32" that binary64
/// parameters will follow.
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
        if !param.accepts(flush::binary32(value)) {
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
            Binary64::new(f64::from(flush::binary32(
                node.params
                    .get(param.name)
                    .copied()
                    .unwrap_or(param.default),
            )))
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

impl<S: Sample> Plan<S> {
    /// Validates `patch` against `registry` and builds its execution plan, computing in
    /// `S`.
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

        // Every id of the order is a resolved box: `remove` always finds it.
        let instances: Vec<Instance<S>> = order
            .iter()
            .filter_map(|id| nodes.remove(id))
            .map(|resolved| {
                let id = resolved.node.id;
                Instance {
                    node: id,
                    object: resolved.spec.create.create(rate, &resolved.params),
                    inputs: vec![S::SILENCE; resolved.spec.inlets.len()],
                    outputs: vec![S::SILENCE; resolved.spec.outlets.len()],
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
        // The output boxes, by their place in the execution order.
        let outputs: Vec<usize> = instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| instance.is_output)
            .map(|(index, _)| index)
            .collect();
        // As many channels as the widest output box has inlets.
        let channels = outputs
            .iter()
            .filter_map(|&index| instances.get(index))
            .map(|instance| instance.inputs.len())
            .max()
            .unwrap_or(0);
        Ok(Self {
            instances,
            bus: vec![S::SILENCE; bus_len],
            outputs,
            channels,
            lanes: Lanes::new(),
        })
    }

    /// Number of engine output channels: the largest inlet count among the boxes with
    /// [`Capability::AudioOutput`], or zero when there is none.
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channels
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
    /// order, output boxes in execution order): a single one passes its samples bit for
    /// bit (`-0.0` and NaN payloads included).
    /// Does not use the heap. The samples are written as the boxes computed them: never
    /// clipped to ±1, scaled or sanitized, so that every target renders the same bits. The
    /// final output stage (limiting, clipping, handling non-finite values) belongs to the
    /// host, until the engine defines its own output policy (roadmap ticket #12).
    pub fn process(&mut self, out: &mut [Block<S>]) {
        for instance in &mut self.instances {
            // An unconnected inlet has no source: it stays silent from the build on.
            for (input, slots) in instance
                .inputs
                .iter_mut()
                .zip(&instance.sources)
                .filter(|(_, slots)| !slots.is_empty())
            {
                // Every slot is below the bus length (`source_slot`): none is skipped.
                sum_blocks(
                    input,
                    &mut self.lanes,
                    slots.iter().filter_map(|&slot| self.bus.get(slot)),
                );
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
        }
        for (index, channel) in out.iter_mut().enumerate() {
            // Every output index is an instance; the boxes without that inlet are left
            // out, and a channel beyond `channels` has none: silence.
            let inputs = self.outputs.iter().filter_map(|&output| {
                self.instances
                    .get(output)
                    .and_then(|instance| instance.inputs.get(index))
            });
            sum_blocks(channel, &mut self.lanes, inputs);
        }
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
