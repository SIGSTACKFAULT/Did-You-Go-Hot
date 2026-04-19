use std::{collections::HashMap, fs::File, io::Write};

use bumpalo::Bump;

use crate::{
    best_path_picker::{Priorities, Quality},
    chart_gen::{ChartGen, ConnectionPass, DecisionPath, DescisionBranches, Destination, NodeData},
    hole_info::{HoleInfo, Mass},
    roll_calc::{
        HoleState, RollDecision, RollPlan, RollState, RollStep, RollersOut, Ship,
        get_best_roll_plan, graph_builder::generate_flowchart,
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

    let rollers_out = RollersOut::new();
    let hole = HoleInfo::from_kg(2_000_000_000);
    let state = RollState {
        remaining_mass: hole.full_mass_range(),
        rollers_out,
        max_size_range: hole.max_range,
    };
    let start_state = HoleState::Full;
    let priorities = Priorities::new(vec![
        Quality::ROProbability,
        Quality::AvgNumPasses,
        Quality::MaxOut,
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

    let mut file = File::create("plan.txt").unwrap();
    file.write_all(generate_flowchart(&plan).as_bytes())
        .unwrap();
}
