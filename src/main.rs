#![feature(portable_simd)]
#![feature(int_roundings)]

use std::{collections::HashMap, fs::File, io::Write};

use bumpalo::Bump;

use crate::{
    best_path_picker::{Priorities, Quality},
    chart_gen::{ConnectionPass, Destination, NodeData},
    hole_info::{HoleInfo, Mass},
    hole_plan_tester::test_calced_roll_plans,
    roll_calc::{
        AvailabileShips, HoleState, RollDecision, RollPlan, RollState, RollStep, RollersUsed, Ship,
        get_best_roll_chart,
    },
};

mod best_path_picker;
mod chart_gen;
mod hole_info;
mod hole_plan_tester;
mod roll_calc;

fn main() {
    test_calced_roll_plans();
    return;

    let available_rollers = [
        AvailabileShips {
            ship: Ship {
                hot: 301_200_000,
                cold: 201_200_000,
            },
            max_num_out: 1,
            max_used: 99,
        },
        AvailabileShips {
            ship: Ship {
                hot: 126_000_000,
                cold: 26_000_000,
            },
            max_num_out: 10,
            max_used: 98,
        },
    ];

    let rollers_out = RollersUsed::new();
    let hole = HoleInfo::from_kg(3_000_000_000);
    let state = RollState {
        remaining_mass: hole.full_mass_range(),
        rollers_out,
        max_size_range: hole.max_range,
        highest_hole_state: HoleState::Full,
        used_ships: RollersUsed::new(),
    };
    let start_state = HoleState::Full;
    let priorities = Priorities::new(vec![
        Quality::ROProbability,
        Quality::AvgNumPasses,
        Quality::MaxOut,
    ])
    .unwrap();
    let num_gigabytes = 16;
    let num_bytes = Some(num_gigabytes * 1024 * 1024 * 1024);
    let (chart, qualities) = get_best_roll_chart(
        &available_rollers,
        state,
        start_state,
        &priorities,
        num_bytes,
    );
    println!(
        "{}% {} {}",
        qualities.roll_out_probability * 100.,
        qualities.average_num_passes,
        qualities.max_num_out
    );

    let mut file = File::create("plan.txt").unwrap();
    file.write_all(chart.to_text_chart().as_bytes()).unwrap();
}
