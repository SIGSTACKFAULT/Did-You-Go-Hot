use std::cmp::Ordering;

use rand::RngExt;
use rustc_hash::FxHashMap;

use crate::{
    best_path_picker::{Priorities, Quality},
    chart_gen::{ConnectionPass, EdgeData, PassDecision, RollingChart},
    hole_info::{HoleInfo, Mass},
    roll_calc::{
        AvailabileShips, Direction, HoleState, PolorizationGuide, RollState, RollersUsed, Ship,
        get_best_roll_chart,
    },
};

#[derive(Debug, Clone, Copy)]
struct HoleData {
    remaining_mass: Mass,
    max_mass: Mass,
}

struct SimResult {
    rollout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimHoleState {
    Closed,
    Crit,
    Shrink,
    Full,
}

impl From<SimHoleState> for PassDecision {
    fn from(val: SimHoleState) -> Self {
        match val {
            SimHoleState::Closed => PassDecision::Closed,
            SimHoleState::Crit => PassDecision::Crit,
            SimHoleState::Shrink => PassDecision::Shrink,
            SimHoleState::Full => PassDecision::Full,
        }
    }
}

pub fn test_calced_roll_plans() {
    let average_masses = [3_000_000_000, 2_000_000_000, 1_000_000_000];
    let starting_masses = [HoleState::Full, HoleState::Shrink, HoleState::Crit];

    const NORMAL_ROLLERS: &[AvailabileShips] = &[
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
    const ONLY_BS: &[AvailabileShips] = &[AvailabileShips {
        ship: Ship {
            hot: 301_200_000,
            cold: 201_200_000,
        },
        max_num_out: 1,
        max_used: 99,
    }];
    let priorities = Priorities::new(vec![
        Quality::ROProbability,
        Quality::AvgNumPasses,
        Quality::MaxOut,
    ])
    .unwrap();

    let mut tests = vec![];
    for mass in average_masses {
        for state in starting_masses {
            for rollers in &[NORMAL_ROLLERS, ONLY_BS] {
                tests.push((mass, state, rollers));
            }
        }
    }
    let mut roll_plans = vec![];

    for (mass, starting_state, rollers) in tests {
        println!("Calcing {mass} {starting_state:?}");
        let hole = HoleInfo::from_kg(mass);
        let state = RollState {
            remaining_mass: hole.mass_range(starting_state),
            rollers_out: RollersUsed::new(),
            max_size_range: hole.max_range,
            highest_hole_state: starting_state,
            used_ships: RollersUsed::new(),
        };
        let starting_state = starting_state;
        let num_gigabytes = 16;
        let num_bytes = Some(num_gigabytes * 1024 * 1024 * 1024);
        let (chart, qualities) = get_best_roll_chart(
            rollers,
            state,
            starting_state,
            &priorities,
            num_bytes,
            PolorizationGuide::UpTo(0),
            0
        )
        .remove(0)
        .unwrap();
        roll_plans.push((chart, qualities, hole, starting_state));
    }

    let mut rng = rand::rng();

    const NUM_SIMS: usize = 5_000_000;
    for (chart, qualities, hole_info, mass_state) in roll_plans {
        let mut num_rollouts = 0;
        for _ in 0..NUM_SIMS {
            let max_mass = rng.random_range(hole_info.max_range.least..=hole_info.max_range.most);
            let threshold_low = match mass_state {
                HoleState::Full => max_mass.div_ceil(2),
                HoleState::Shrink => max_mass.div_ceil(10),
                HoleState::Crit => 1,
            };
            let threshold_high = match mass_state {
                HoleState::Full => max_mass,
                HoleState::Shrink => max_mass.div_floor(2) - 1,
                HoleState::Crit => max_mass.div_floor(10) - 1,
            };
            let hole = HoleData {
                remaining_mass: rng.random_range(threshold_low..=threshold_high),
                max_mass,
            };
            let result = simulate(hole, &chart).unwrap();
            if result.rollout {
                num_rollouts += 1;
            }
        }
        println!(
            "rollout rate: {} predicted {}",
            num_rollouts as f64 / NUM_SIMS as f64,
            qualities.roll_out_probability
        );
    }
}

fn simulate(mut hole: HoleData, plan: &RollingChart) -> Result<SimResult, ()> {
    let mut walker = plan.chart_walker();
    let mut rollers_out = FxHashMap::default();

    loop {
        let state = hole_state(hole);
        if state == SimHoleState::Closed {
            return Ok(SimResult {
                rollout: rollers_out.values().any(|x| *x > 0),
            });
        }
        let EdgeData { actions, .. } = walker.take_option(state.into()).unwrap();

        let mut passes: Vec<_> = actions.iter().collect();
        // Pass Out and largest first to maximize rollout chance. Chart should not allow this to matter.
        passes.sort_by(|(pass, _), (pass2, _)| order_out_then_in_then_largeest(pass, pass2));

        // pass all but 1 since due to ordering, if there is a rollout it will be present when only 1 guy is left.
        for (i, (action, count)) in passes.iter().enumerate() {
            let mut count_mod = 0;
            if i == passes.len() - 1 {
                count_mod = 1;
            }
            pass_ship(&mut hole, &mut rollers_out, action, **count - count_mod);
        }
        let (last_pass, _) = passes.last().unwrap();
        if hole.remaining_mass <= 0 {
            return Ok(SimResult {
                rollout: rollers_out.values().any(|x| *x > 0),
            });
        }
        pass_ship(&mut hole, &mut rollers_out, last_pass, 1);

        // If last was an out pass, check if we rolled out
        if hole.remaining_mass <= 0 {
            return Ok(SimResult {
                rollout: rollers_out.values().any(|x| *x > 0),
            });
        }
    }
}

fn pass_ship(
    hole: &mut HoleData,
    rollers_out: &mut FxHashMap<Ship, u16>,
    pass: &ConnectionPass,
    num: u16,
) {
    let entry = rollers_out.entry(pass.ship).or_default();
    if pass.direction == Direction::Out {
        *entry += num;
    } else {
        *entry -= num;
    }
    hole.remaining_mass -= pass.mass() * num as Mass;
}

fn hole_state(hole: HoleData) -> SimHoleState {
    if hole.remaining_mass <= 0 {
        return SimHoleState::Closed;
    }
    if hole.max_mass / 10 > hole.remaining_mass {
        return SimHoleState::Crit;
    }
    if hole.max_mass / 2 > hole.remaining_mass {
        return SimHoleState::Shrink;
    }
    SimHoleState::Full
}

pub fn order_out_then_in_then_largeest(pass: &ConnectionPass, pass2: &ConnectionPass) -> Ordering {
    // Prioritize Out passes over In passes
    if pass.direction == Direction::Out && pass2.direction == Direction::In {
        return Ordering::Less;
    } else if pass.direction == Direction::In && pass2.direction == Direction::Out {
        return Ordering::Greater;
    }
    // Prioritize larger mass first
    pass.mass().cmp(&pass2.mass()).reverse()
}
