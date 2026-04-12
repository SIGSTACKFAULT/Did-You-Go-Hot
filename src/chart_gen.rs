use std::collections::{HashMap, HashSet};

use crate::{roll_calc::{Direction, Ship, ShipState}};


#[derive(Debug, PartialEq)]
pub struct NodeData {
    pub rollout_probability: f64,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum Destination {
    Closed,
    Node(usize)
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct ConnectionPass {
    pub ship: Ship,
    pub state: ShipState,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum DecisionPath {
    Closed,
    Crit,
    Shrink,
    Full,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub enum DescisionBranches {
    All,
    Split(Vec<DecisionPath>)
}

pub struct ChartGen {
    ids: HashMap<usize, NodeData>,
    connections: HashMap<(usize, Destination, DescisionBranches), Option<HashMap<ConnectionPass, u32>>>,
    closed_connections: HashSet<usize>,
    unique_id: i32,
}

impl ChartGen {
    pub fn new() -> Self {
        Self {
            ids: HashMap::new(),
            connections: HashMap::new(),
            closed_connections: HashSet::new(),
            unique_id: -1
        }
    }

    pub fn add_node(&mut self, id: usize, data: NodeData) {
        if let Some(existing) = self.ids.get(&id) {
            assert_eq!(existing, &data);
            return;
        }
        self.ids.insert(id, data);
    }

    pub fn add_edge(&mut self, from: usize, to: Destination, data: Option<HashMap<ConnectionPass, u32>>, branches: DescisionBranches) {
        let key = (from, to, branches);
        if let Some(e) = self.connections.insert(key, data ) {
            panic!("{:?}", e);
        }
    }

    pub fn to_text_chart(&self) -> String {
        let mut chart = "---\ntitle: Roll Plan\n---\nflowchart TD\n".to_string();
        for (id, data) in &self.ids {
            chart.push_str(&format!("{id}[{}%]\n", data.rollout_probability * 100.0));
        }

        let mut unique_id = -1;
        for ((from, to, decisions), data) in &self.connections {
            chart.push_str(&format!("{from} -->|{}\n", paths_to_text(decisions)));
            if let Some(additional_data) = data {
                for (pass, num) in additional_data {
                    let mass = match pass.state {
                        ShipState::Cold => pass.ship.cold,
                        ShipState::Hot => pass.ship.hot,
                    } / 1000000;
                    let dir = match pass.direction {
                        Direction::In => "IN",
                        Direction::Out => "OUT",
                    };
                    chart.push_str(&format!("{dir} {mass}"));
                    if *num > 1 {
                        chart.push_str(&format!(" x{num}"));
                    }
                    chart.push_str("\n");
                }
            }
            
            chart.push_str(&format!("| {}\n", dest_to_text(to, &mut unique_id)));
        }
        
        chart
    }
}

fn dest_to_text(dest: &Destination, unique_id: &mut i32) -> String {
    match dest {
        Destination::Closed => {
            let out = format!("{unique_id}[Closed]");
            *unique_id -= 1;
            out
        },
        Destination::Node(n) => format!("{n}")
    }
}

fn paths_to_text(decisions: &DescisionBranches) -> String {
    let mut out = String::new();
    match decisions {
        DescisionBranches::All => (),
        DescisionBranches::Split(decisions) => {
            for decision in decisions {
                out.push_str(match decision {
                    DecisionPath::Closed => "",
                    DecisionPath::Crit => "Crit",
                    DecisionPath::Shrink => "Shrink",
                    DecisionPath::Full => "Full",
                });
            }
        }
    }
    out
}
