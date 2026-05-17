#![feature(portable_simd)]
#![feature(int_roundings)]

use std::{fs::File, io::Write};

use eframe::egui::Ui;

use crate::{
    best_path_picker::{Priorities, Quality},
    hole_info::HoleInfo,
    hole_plan_tester::test_calced_roll_plans,
    roll_calc::{AvailabileShips, HoleState, RollState, RollersUsed, Ship, get_best_roll_chart},
    ui::run_app,
};

mod best_path_picker;
mod chart_gen;
mod hole_info;
mod hole_plan_tester;
mod roll_calc;
mod ui;

fn main() {
    run_app();


    let available_rollers = [
        AvailabileShips {
            ship: Ship {
                hot: 301_200_000,
                cold: 201_200_000,
            },
            max_num_out: 99,
            max_used: 3,
        },
        AvailabileShips {
            ship: Ship {
                hot: 126_000_000,
                cold: 26_000_000,
            },
            max_num_out: 99,
            max_used: 1,
        },
        // AvailabileShips {
        //     ship: Ship {
        //         hot: 130_000_000,
        //         cold: 768_000,
        //     },
        //     max_num_out: 99,
        //     max_used: 1,
        // },
        // AvailabileShips {
        //     ship: Ship {
        //         hot: 279_000_000,
        //         cold: 179_000_000,
        //     },
        //     max_num_out: 99,
        //     max_used: 1,
        // },
    ];

    let start_state = HoleState::Shrink;

    let num_polos = 4;
    let rollers_out = RollersUsed::new();
    let hole = HoleInfo::from_kg(3_300_000_000);
    let state = RollState {
        remaining_mass: hole.mass_range(start_state),
        rollers_out,
        max_size_range: hole.max_range,
        highest_hole_state: start_state,
        used_ships: RollersUsed::new(),
    };
    let priorities = Priorities::new(vec![
        Quality::ROProbability,
        Quality::AvgNumPasses,
        Quality::MaxOut,
    ])
    .unwrap();
    let num_gigabytes = 16;
    let num_bytes = Some(num_gigabytes * 1024 * 1024 * 1024);
    let mut plans = get_best_roll_chart(
        &available_rollers,
        state,
        start_state,
        &priorities,
        num_bytes,
        roll_calc::PolorizationGuide::UpTo(4),
        0,
    );
    for plan in plans {
        if let Some((chart, qualities)) = plan {
            println!(
                "{}% {} {} {}",
                qualities.roll_out_probability * 100.,
                qualities.average_num_passes,
                qualities.max_num_out,
                qualities.num_polorizations,
            );
        } else {
            println!("None")
        }
    }

    // let mut file = File::create("plan.txt").unwrap();
    // file.write_all(chart.to_text_chart().as_bytes()).unwrap();
}
