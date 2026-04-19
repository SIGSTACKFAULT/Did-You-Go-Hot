use std::{cmp::Ordering, collections::HashMap, fs::File, io::Write, rc::Rc};

use bumpalo::Bump;

use crate::{
    best_path_picker::{Priorities, Quality},
    chart_gen::{ChartGen, ConnectionPass, DecisionPath, DescisionBranches, Destination, NodeData},
    hole_info::{HoleInfo, Mass},
    roll_calc::{
        Direction, HoleState, RollDecision, RollPlan, RollState, RollStep, Ship, ShipState,
        get_best_roll_plan,
    },
};

mod best_path_picker;
mod chart_gen;
mod hole_info;
mod hole_plan_tester;
mod roll_calc;

fn main() {
    let available_rollers = [
        Ship {
            hot: 301_200_000,
            cold: 201_200_000,
        },
        Ship {
            hot: 126_000_000,
            cold: 26_000_000,
        },
    ];

    let rollers_out = (0..(available_rollers.len())).map(|_| 0).collect();
    let hole = HoleInfo::from_kg(3_000_000_000);
    let state = RollState {
        remaining_mass: hole.full_mass_range(),
        rollers_out,
        max_size_range: hole.max_range,
    };
    let start_state = HoleState::Full;
    let priorities = Priorities::new(vec![
        Quality::MaxOut,
        Quality::ROProbability,
        Quality::AvgNumPasses,
    ])
    .unwrap();
    let arena = Bump::new();
    let plan = get_best_roll_plan(&available_rollers, state, start_state, &priorities, &arena);
    println!(
        "{}% {} {}",
        plan.qualities.roll_out_probability * 100.,
        plan.qualities.average_num_passes,
        plan.qualities.max_num_out
    );

    // let mut file = File::create("plan.txt").unwrap();
    // file.write_all(generate_flowchart(&plan).as_bytes())
    //     .unwrap();
}

fn generate_flowchart(plan: &RollPlan) -> String {
    let mut chart = ChartGen::new();

    let id = plan as *const RollPlan as usize;
    chart.add_node(id, plan_to_data(plan));

    let mut memoize = HashMap::new();
    generate_flowchart_rec(&mut chart, plan, true, &mut memoize);

    chart.to_text_chart()
}

#[derive(Debug, Clone)]
enum Connection {
    Decision,
    OnlyOption((HashMap<ConnectionPass, u32>, Destination)),
    DirectToClosed,
}

fn generate_flowchart_rec(
    chart: &mut ChartGen,
    plan: &RollPlan,
    force_links: bool,
    memoize: &mut HashMap<usize, Connection>,
) -> Connection {
    let current_id = plan_id(plan);

    if let Some(cached) = memoize.get(&current_id) {
        return cached.clone();
    }

    let RollPlan {
        decision:
            RollDecision {
                crit: crit_o,
                shrink: shrink_o,
                full: full_o,
                ..
            },
        ..
    } = plan;
    let mut options = vec![];
    if let Some(crit) = crit_o {
        options.push((DecisionPath::Crit, crit));
    }
    if let Some(shrink) = shrink_o {
        options.push((DecisionPath::Shrink, shrink));
    }
    if let Some(full) = full_o {
        options.push((DecisionPath::Full, full));
    }

    if plan.decision.can_close {
        if options.len() == 0 {
            return Connection::DirectToClosed;
        } else {
            chart.add_edge(
                current_id,
                chart_gen::Destination::Closed,
                None,
                DescisionBranches::Split(vec![DecisionPath::Closed]),
            );
        }
    }

    let mut edges = vec![];
    let no_decision = options.len() == 1 && !force_links && !plan.decision.can_close;
    for (paths, next_step) in options {
        let (mut down_chain, final_id) =
            match generate_flowchart_rec(chart, &next_step.next_plan, false, memoize) {
                Connection::OnlyOption(chain) => chain,
                Connection::Decision => {
                    add_node(chart, &next_step.next_plan);
                    (
                        HashMap::new(),
                        Destination::Node(plan_id(&next_step.next_plan)),
                    )
                }
                Connection::DirectToClosed => (HashMap::new(), Destination::Closed),
            };
        add_to_chain(&mut down_chain, &next_step);
        if no_decision {
            memoize.insert(current_id, Connection::OnlyOption((down_chain, final_id)));
            return memoize.get(&current_id).unwrap().clone();
        }
        edges.push((final_id, Some(down_chain), paths));
    }

    let mut combined_edges = vec![];
    for (dest, edge, paths) in edges {
        let mut found = false;
        for (edest, eedge, epaths) in combined_edges.iter_mut() {
            if edest == &dest && eedge == &edge {
                match epaths {
                    DescisionBranches::Split(v) => v.push(paths),
                    DescisionBranches::All => unreachable!(),
                }
                found = true;
                break;
            }
        }
        if !found {
            combined_edges.push((dest, edge, DescisionBranches::Split(vec![paths])));
        }
    }

    if combined_edges.len() == 1 {
        combined_edges[0].2 = DescisionBranches::All
    }

    for (final_id, chain, paths) in combined_edges {
        chart.add_edge(current_id, final_id, chain, paths);
    }

    memoize.insert(current_id, Connection::Decision);
    Connection::Decision
}

fn add_to_chain(chain: &mut HashMap<ConnectionPass, u32>, step: &RollStep) {
    let key = ConnectionPass {
        ship: step.ship,
        state: step.ship_state,
        direction: step.direction,
    };
    if let Some(option) = chain.get_mut(&key) {
        *option += 1;
    } else {
        chain.insert(key, 1);
    }
}

const MASS_DIVISOR: Mass = 1000000;

fn plan_to_data(plan: &RollPlan) -> NodeData {
    NodeData {
        rollout_probability: plan.qualities.roll_out_probability,
        extra_info: format!(
            "mass: {}-{}\nmax: {}-{}",
            plan.mass_range.least / MASS_DIVISOR,
            plan.mass_range.most / MASS_DIVISOR,
            plan.max_mass_range.least / MASS_DIVISOR,
            plan.max_mass_range.most / MASS_DIVISOR
        ),
    }
}

fn step_to_text(step: &RollStep, hole_stage: &str) -> String {
    let pass = match step.direction {
        Direction::In => "in",
        Direction::Out => "out",
    };
    let mut mass = match step.ship_state {
        ShipState::Cold => step.ship.cold,
        ShipState::Hot => step.ship.hot,
    };
    mass /= MASS_DIVISOR;
    format!("{hole_stage}\n{pass} {mass}")
}

fn plan_id(plan: &RollPlan) -> usize {
    plan as *const RollPlan as usize
}

fn add_node(chart: &mut ChartGen, next: &RollPlan) {
    let next_id = plan_id(next);
    chart.add_node(next_id, plan_to_data(&next));
}
