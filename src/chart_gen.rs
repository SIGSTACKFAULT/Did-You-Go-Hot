use petgraph::{graph::NodeIndex, prelude::StableDiGraph, visit::IntoEdgeReferences};
use std::collections::HashMap;
use std::fmt::Write;

use crate::{
    hole_info::Mass,
    roll_calc::{Direction, HoleState, Ship, ShipState},
};
use petgraph::algo::toposort;
use petgraph::visit::EdgeRef;

enum Node {
    Closed,
    Node(NodeData),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeData {
    pub rollout_probability: f64,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct ConnectionPass {
    pub ship: Ship,
    pub state: ShipState,
    pub direction: Direction,
}

pub struct NodeConnectionData {
    pub to: usize,
    pub pass: ConnectionPass,
}

pub enum Destination {
    Closed,
    Node(NodeConnectionData),
}

#[derive(Debug, Clone)]
struct EdgeData {
    decision: PassDecision,
    actions: HashMap<ConnectionPass, u32>,
}

pub struct RollingChart {
    head: NodeIndex<u32>,
    closed: NodeIndex<u32>,
    ids: HashMap<usize, NodeIndex<u32>>,
    graph: StableDiGraph<Node, EdgeData>,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum PassDecision {
    Closed,
    Crit,
    Shrink,
    Full,
    NotClosed,
}

const MASS_DIVISOR: Mass = 1000000;

impl RollingChart {
    pub fn new(head: usize, data: NodeData) -> Self {
        let mut graph = StableDiGraph::new();
        let head_idx = graph.add_node(Node::Node(data));
        let closed_idx = graph.add_node(Node::Closed);
        Self {
            head: head_idx,
            closed: closed_idx,
            ids: HashMap::from([(head, head_idx)]),
            graph: graph,
        }
    }

    pub fn add_node(&mut self, id: usize, data: NodeData) {
        if let Some(node) = self.ids.get(&id) {
            match self.graph.node_weight(*node).unwrap() {
                Node::Node(n) => {
                    assert!(n == &data)
                }
                _ => unreachable!(),
            }

            return;
        }
        let idx = self.graph.add_node(Node::Node(data));
        self.ids.insert(id, idx);
    }

    pub fn add_edge(
        &mut self,
        from: usize,
        to: Destination,
        decision: PassDecision,
    ) -> Result<(), ()> {
        let Some(from_idx) = self.ids.get(&from).copied() else {
            return Err(());
        };
        let (to_idx, pass_o) = match to {
            Destination::Closed => (self.closed, None),
            Destination::Node(data) => {
                let Some(to_idx) = self.ids.get(&data.to) else {
                    return Err(());
                };
                (*to_idx, Some(data.pass))
            }
        };

        if let Some(existing) = self
            .graph
            .edges_connecting(from_idx, to_idx)
            .find(|edge| edge.weight().decision == decision)
        {
            let Some(pass) = pass_o else {
                // It is direct to closed.
                return Ok(());
            };
            if *existing.weight().actions.keys().next().unwrap() != pass {
                return Err(());
            }
        }

        let actions = if let Some(pass) = pass_o {
            HashMap::from([(pass, 1)])
        } else {
            HashMap::new()
        };
        self.graph
            .add_edge(from_idx, to_idx, EdgeData { decision, actions });

        Ok(())
    }

    fn compress_direct_to_closed(&mut self) {
        // Find nodes whose ONLY path out is directly to `self.closed`.
        let mut to_remove = Vec::new();
        for node in self.graph.node_indices() {
            // Skip structural anchors
            if node == self.closed || node == self.head {
                continue;
            }

            let out_edges: Vec<_> = self
                .graph
                .edges_directed(node, petgraph::Direction::Outgoing)
                .collect();

            // If the node has exactly one path out, and it's to Closed
            if out_edges.len() == 1 && out_edges[0].target() == self.closed {
                to_remove.push(node);
            }
        }

        // Apply removals and redirect edges directly to closed
        for node in to_remove {
            if self.graph.node_weight(node).is_none() {
                continue;
            }

            let in_edges: Vec<_> = self
                .graph
                .edges_directed(node, petgraph::Direction::Incoming)
                .collect();

            // Clone edge data to avoid borrowing conflicts when mutating the graph
            let mut edges_to_add = Vec::new();
            for in_edge in in_edges {
                edges_to_add.push((in_edge.source(), in_edge.weight().clone()));
            }

            // Removing the middleman automatically destroys its old incoming/outgoing edges
            self.graph.remove_node(node);

            // Reattach the parents directly to the closed node
            for (source, actions) in edges_to_add {
                // Create new direct connection using the parent's incoming pass details
                self.graph.add_edge(source, self.closed, actions);
            }
        }
    }

    fn compress_no_decision_chains(&mut self) {
        // Since it's a DAG, we can safely process top-down.
        let topo_order = toposort(&self.graph, None)
            .expect("Graph contains a cycle, violating the DAG assumption");

        for node in topo_order {
            // The node might have been removed in an earlier iteration of this loop
            if self.graph.node_weight(node).is_none() {
                continue;
            }

            let in_edges: Vec<_> = self
                .graph
                .edges_directed(node, petgraph::Direction::Incoming)
                .collect();
            let out_edges: Vec<_> = self
                .graph
                .edges_directed(node, petgraph::Direction::Outgoing)
                .collect();

            // Compress condition: exactly one path in, one path out.
            // This naturally skips `head` (in_degree = 0) and `closed` (out_degree = 0).
            if in_edges.len() == 1 && out_edges.len() == 1 {
                let in_edge = in_edges[0];
                let out_edge = out_edges[0];

                let u = in_edge.source();
                let v = out_edge.target();

                // Do not touch edges pointing directly to closed because the last ship out matters
                if v == self.closed {
                    continue;
                }

                // Combine the actions/weights
                let mut merged_actions = in_edge.weight().actions.clone();
                let original_decision = in_edge.weight().decision;
                for (pass, count) in &out_edge.weight().actions {
                    *merged_actions.entry(*pass).or_default() += count;
                }

                // Remove the pass-through node (automatically removes old edges)
                self.graph.remove_node(node);

                // Add the direct edge (u, v)
                self.graph.add_edge(
                    u,
                    v,
                    EdgeData {
                        decision: original_decision,
                        actions: merged_actions,
                    },
                );
            }
        }
    }

    fn combine_identical_decision_paths(&mut self) {
        let topo_order = toposort(&self.graph, None)
            .expect("Graph contains a cycle, violating the DAG assumption");

        'next_node: for node in topo_order {
            // The node might have been removed in an earlier iteration of this loop
            if self.graph.node_weight(node).is_none() {
                continue;
            }

            let actual_decisions: Vec<_> = self
                .graph
                .edges_directed(node, petgraph::Direction::Outgoing)
                .filter(|x| x.weight().decision != PassDecision::Closed)
                .collect();

            if actual_decisions.len() <= 1 {
                continue;
            }

            // Not implementing support for combining 2 out of 3 identical paths because that is super unlikely to ever be useful.
            let first_actions = actual_decisions[0].weight().actions.clone();
            for decision in &actual_decisions[1..] {
                if decision.weight().actions != first_actions {
                    // Not all immediate actions are the same so must skip.
                    continue 'next_node;
                }
            }
            let mut depth_decisions = vec![];
            let are_identical = actual_decisions
                .iter()
                .all(|path| self.is_identical_decisions(path.target(), 0, &mut depth_decisions));

            if are_identical {
                let to_prune: Vec<_> = actual_decisions[1..]
                    .iter()
                    .map(|edge| edge.target())
                    .collect();
                let main_edge_id = actual_decisions[0].id();
                drop(actual_decisions);
                self.graph.edge_weight_mut(main_edge_id).unwrap().decision =
                    PassDecision::NotClosed;
                for prune in to_prune {
                    self.prune_path_rec(prune);
                }
            }
        }
    }

    fn prune_path_rec(&mut self, node: NodeIndex<u32>) {
        if node == self.closed
            || self
                .graph
                .edges_directed(node, petgraph::Direction::Incoming)
                .count()
                > 1
        {
            return;
        }

        let edge_idxs: Vec<_> = self
            .graph
            .edges_directed(node, petgraph::Direction::Outgoing)
            .map(|e| e.id())
            .collect();
        for child in edge_idxs {
            let target = self.graph.edge_endpoints(child).unwrap().1;
            self.prune_path_rec(target);
            // It should delete itself in most cases which would delete the edge, but may not if closed or in degree > 1
            // Delete after so it knows not to kill itself
            self.graph.remove_edge(child);
        }

        self.graph.remove_node(node);
        todo!()
        // Fix bug where if a decision is in one path but not the other it will get lost when pruned.
        // In addition fix that same bug buf for closed nodes that might disapear.
    }

    fn is_identical_decisions(
        &self,
        node: NodeIndex<u32>,
        depth: usize,
        depth_decisions: &mut Vec<HashMap<PassDecision, HashMap<ConnectionPass, u32>>>,
    ) -> bool {
        let Some(Node::Node(_)) = self.graph.node_weight(node) else {
            // No decisions at closed node or doesn't exist
            return true;
        };

        let all_decisions: Vec<_> = self
            .graph
            .edges_directed(node, petgraph::Direction::Outgoing)
            .collect();

        let this_level = if let Some(level) = depth_decisions.get_mut(depth) {
            level
        } else {
            depth_decisions.push(HashMap::new());
            depth_decisions.last_mut().unwrap()
        };

        for decision in &all_decisions {
            if let Some(other_decision) = this_level.get(&decision.weight().decision) {
                if other_decision != &decision.weight().actions {
                    return false;
                }
            } else {
                this_level.insert(
                    decision.weight().decision,
                    decision.weight().actions.clone(),
                );
            };
        }

        for path in &all_decisions {
            if !self.is_identical_decisions(path.target(), depth + 1, depth_decisions) {
                return false;
            }
        }

        true
    }

    pub fn compress(&mut self) {
        self.compress_direct_to_closed();
        self.compress_no_decision_chains();
        self.combine_identical_decision_paths();
        // Rerun compress_no_decision_chains to clean up any new chains created by combine_identical_decision_paths
        self.compress_no_decision_chains();

        // Clean up the index map to discard no longer necessary, dropped nodes.
        self.ids
            .retain(|_, &mut idx| self.graph.node_weight(idx).is_some());
    }

    pub fn to_text_chart(&self) -> String {
        let mut chart = "---\ntitle: Roll Plan\n---\nflowchart TD\n".to_string();

        // Output the node definitions using their NodeIndex
        for node in self.graph.node_indices() {
            if let Some(Node::Node(data)) = self.graph.node_weight(node) {
                let _ = writeln!(
                    chart,
                    "n_{}[{}%]",
                    node.index(),
                    data.rollout_probability * 100.0,
                );
            }
        }

        let mut unique_closed_id = 0;

        // Output the connection edges
        for edge in self.graph.edge_references() {
            let from_idx = edge.source();
            let to_idx = edge.target();

            let decision_text = match edge.weight().decision {
                PassDecision::Closed => "",
                PassDecision::Crit => "Crit",
                PassDecision::Shrink => "Shrink",
                PassDecision::Full => "Full",
                PassDecision::NotClosed => "Not Closed",
            };

            // Start formatting the edge text block
            let _ = write!(chart, "n_{} -->|{}\n", from_idx.index(), decision_text);

            // Add pass details
            for (pass, num) in &edge.weight().actions {
                let mass = match pass.state {
                    ShipState::Cold => pass.ship.cold,
                    ShipState::Hot => pass.ship.hot,
                } / MASS_DIVISOR;

                let dir = match pass.direction {
                    Direction::In => "IN",
                    Direction::Out => "OUT",
                };

                let _ = write!(chart, "{} {}", dir, mass);
                if *num > 1 {
                    let _ = write!(chart, " x{}", num);
                }
                let _ = writeln!(chart);
            }

            // Determine destination text.
            // For Closed, we generate a unique ID so lines do not converge.
            if to_idx == self.closed {
                let _ = writeln!(chart, "| c_{}[Closed]", unique_closed_id);
                unique_closed_id += 1;
            } else {
                let _ = writeln!(chart, "| n_{}", to_idx.index());
            }
        }

        chart
    }
}
