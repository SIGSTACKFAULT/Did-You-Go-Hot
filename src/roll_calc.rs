use std::{cmp::Ordering, rc::Rc};

use rustc_hash::FxHashMap;

use crate::{
    best_path_picker::{PathCmpFn, PathPicker},
    hole_info::{EMPTY_MR, Mass, MassRange, mr},
};

#[derive(Debug, Clone)]
pub struct RollPlan {
    pub max_num_out: u32,
    pub roll_out_probability: f64,
    pub average_num_passes: f64,
    pub decision: RollDecision,
}

#[derive(Debug, Clone)]
pub struct RollDecision {
    pub can_close: bool,
    pub crit: Option<Box<RollStep>>,
    pub shrink: Option<Box<RollStep>>,
    pub full: Option<Box<RollStep>>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct RollState {
    pub remaining_mass: MassRange,
    pub rollers_out: Vec<usize>,
    pub max_size_range: MassRange,
}

#[derive(Debug, Clone)]
pub struct RollStep {
    pub next_plan: Rc<RollPlan>,
    pub direction: Direction,
    pub ship_state: ShipState,
    pub ship: Ship,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum ShipState {
    Hot,
    Cold,
}

#[derive(Debug, Clone, Copy)]
pub enum HoleState {
    Full,
    Shrink,
    Crit,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Ship {
    pub hot: Mass,
    pub cold: Mass,
}

pub fn get_best_roll_plan(
    available_rollers: &[Ship],
    state: RollState,
    starting_state: HoleState,
    path_cmp: PathCmpFn,
) -> RollPlan {
    RollPlan::clone(&get_best_roll_plan_rec(
        available_rollers,
        state,
        path_cmp,
        Some(starting_state),
        &mut FxHashMap::default(),
    ))
}

// Returns None if impossible
pub fn get_best_roll_plan_rec(
    available_rollers: &[Ship],
    state: RollState,
    path_cmp: PathCmpFn,
    assume_state: Option<HoleState>,
    memoization: &mut FxHashMap<RollState, Rc<RollPlan>>,
) -> Rc<RollPlan> {
    if let Some(cached_plan) = memoization.get(&state) {
        return Rc::clone(cached_plan);
    }

    let mut mass_range_for_closed = overlap(&closed_range(), &state.remaining_mass);
    let mut mass_range_for_crit =
        overlap(&crit_range(&state.max_size_range), &state.remaining_mass);
    let mut mass_range_for_shrink =
        overlap(&shrink_range(&state.max_size_range), &state.remaining_mass);
    let mut mass_range_for_full =
        overlap(&full_range(&state.max_size_range), &state.remaining_mass);

    if let Some(starting_state) = assume_state {
        match starting_state {
            HoleState::Crit => {
                mass_range_for_closed = EMPTY_MR;
                mass_range_for_shrink = EMPTY_MR;
                mass_range_for_full = EMPTY_MR;
            }
            HoleState::Shrink => {
                mass_range_for_closed = EMPTY_MR;
                mass_range_for_crit = EMPTY_MR;
                mass_range_for_full = EMPTY_MR;
            }
            HoleState::Full => {
                mass_range_for_closed = EMPTY_MR;
                mass_range_for_crit = EMPTY_MR;
                mass_range_for_shrink = EMPTY_MR;
            }
        }
    }

    let possible_closed_states = possible_states(mass_range_for_closed, state.max_size_range);
    let mut max_num_out = state.rollers_out.iter().sum::<usize>() as u32;

    let mut best_paths = PathPicker::new(path_cmp);

    let possible_crit_states = if !mass_range_for_crit.is_empty() {
        let new_max_range = update_max_range_from_crit(&mass_range_for_crit, &state.max_size_range);
        add_best_steps(
            available_rollers,
            mass_range_for_crit,
            new_max_range,
            &state.rollers_out,
            &mut best_paths,
            HoleState::Crit,
            path_cmp,
            memoization,
        );
        possible_states(mass_range_for_crit, new_max_range)
    } else {
        0
    };
    let possible_shrink_states = if !mass_range_for_shrink.is_empty() {
        let new_max_range =
            update_max_range_from_shrink(&mass_range_for_shrink, &state.max_size_range);
        add_best_steps(
            available_rollers,
            mass_range_for_shrink,
            new_max_range,
            &state.rollers_out,
            &mut best_paths,
            HoleState::Shrink,
            path_cmp,
            memoization,
        );
        possible_states(mass_range_for_shrink, new_max_range)
    } else {
        0
    };
    let possible_full_states = if !mass_range_for_full.is_empty() {
        let new_max_range = update_max_range_from_full(&mass_range_for_full, &state.max_size_range);
        add_best_steps(
            available_rollers,
            mass_range_for_full,
            new_max_range,
            &state.rollers_out,
            &mut best_paths,
            HoleState::Full,
            path_cmp,
            memoization,
        );
        possible_states(mass_range_for_full, new_max_range)
    } else {
        0
    };

    let final_states = best_paths.best();

    let mut paths = final_states.into_iter();
    let crit_plan = paths.next().unwrap().map(|best_next| {
        max_num_out = max_num_out.max(best_next.next_plan.max_num_out);
        Box::new(best_next)
    });
    let shrink_plan = paths.next().unwrap().map(|best_next| {
        max_num_out = max_num_out.max(best_next.next_plan.max_num_out);
        Box::new(best_next)
    });
    let full_plan = paths.next().unwrap().map(|best_next| {
        max_num_out = max_num_out.max(best_next.next_plan.max_num_out);
        Box::new(best_next)
    });

    let closed_roll_out_probability = if state.rollers_out.iter().all(|x| *x == 0) {
        0.0
    } else {
        1.0
    };
    let (crit_rollout_probability, crit_average_passes) = if let Some(plan) = &crit_plan {
        (
            plan.next_plan.roll_out_probability,
            plan.next_plan.average_num_passes,
        )
    } else {
        (0.0, 0.0)
    };
    let (shrink_rollout_probability, shrink_average_passes) = if let Some(plan) = &shrink_plan {
        (
            plan.next_plan.roll_out_probability,
            plan.next_plan.average_num_passes,
        )
    } else {
        (0.0, 0.0)
    };
    let (full_rollout_probability, full_average_passes) = if let Some(plan) = &full_plan {
        (
            plan.next_plan.roll_out_probability,
            plan.next_plan.average_num_passes,
        )
    } else {
        (0.0, 0.0)
    };

    let possible_states = possible_closed_states
        + possible_crit_states
        + possible_shrink_states
        + possible_full_states;

    let closed_probability = geometric_probability(possible_closed_states, possible_states);
    let crit_probability = geometric_probability(possible_crit_states, possible_states);
    let shrink_probability = geometric_probability(possible_shrink_states, possible_states);
    let full_probability = geometric_probability(possible_full_states, possible_states);
    let total_probability =
        closed_probability + crit_probability + shrink_probability + full_probability;
    if (total_probability * 10_000.0).round() / 10_000.0 != 1. {
        println!(
            "{total_probability}, l{closed_probability}, c{crit_probability}, s{shrink_probability}, f{full_probability}"
        );
    }

    let roll_out_probability = closed_roll_out_probability * closed_probability
        + crit_rollout_probability * crit_probability
        + shrink_rollout_probability * shrink_probability
        + full_rollout_probability * full_probability;

    let average_num_passes = 1.0
        + crit_probability * crit_average_passes
        + shrink_probability * shrink_average_passes
        + full_probability * full_average_passes;

    let best_plan = Rc::new(RollPlan {
        max_num_out,
        roll_out_probability,
        average_num_passes,
        decision: RollDecision {
            can_close: possible_closed_states != 0,
            crit: crit_plan,
            shrink: shrink_plan,
            full: full_plan,
        },
    });

    memoization.insert(state, Rc::clone(&best_plan));
    best_plan
}

fn add_best_steps(
    available_rollers: &[Ship],
    mass_range: MassRange,
    new_max_mass_range: MassRange,
    rollers_out: &Vec<usize>,
    best_paths: &mut PathPicker,
    hole_state: HoleState,
    path_cmp: PathCmpFn,
    memoization: &mut FxHashMap<RollState, Rc<RollPlan>>,
) {
    let in_passes = (0..rollers_out.len())
        .flat_map(|i| {
            if rollers_out[i] == 0 {
                return None;
            }
            let mut new_out_rollers = rollers_out.clone();
            let in_ship = available_rollers[i];
            new_out_rollers[i] -= 1;

            Some([
                (
                    new_out_rollers.clone(),
                    in_ship.cold,
                    Direction::In,
                    ShipState::Cold,
                    in_ship,
                ),
                (
                    new_out_rollers,
                    in_ship.hot,
                    Direction::In,
                    ShipState::Hot,
                    in_ship,
                ),
            ])
        })
        .flatten();

    let out_passes = available_rollers
        .iter()
        .enumerate()
        .flat_map(|(i, out_ship)| {
            let mut new_out_rollers = rollers_out.clone();
            new_out_rollers[i] += 1;
            [
                (
                    new_out_rollers.clone(),
                    out_ship.cold,
                    Direction::Out,
                    ShipState::Cold,
                    *out_ship,
                ),
                (
                    new_out_rollers,
                    out_ship.hot,
                    Direction::Out,
                    ShipState::Hot,
                    *out_ship,
                ),
            ]
        });

    let mut possibility_iter = in_passes.into_iter().chain(out_passes).map(
        |(out_rollers, mass, direction, ship_state, ship)| RollStep {
            next_plan: get_best_roll_plan_rec(
                available_rollers,
                RollState {
                    remaining_mass: mass_range - mass,
                    rollers_out: out_rollers,
                    max_size_range: new_max_mass_range,
                },
                path_cmp,
                None,
                memoization,
            ),
            direction,
            ship_state,
            ship,
        },
    );

    for possibility in possibility_iter {
        best_paths.suggest(possibility, hole_state);
    }
}

fn geometric_probability(possibilities: u128, all_possibilities: u128) -> f64 {
    possibilities as f64 / all_possibilities as f64
}

fn possible_states(mass_range: MassRange, max_mass_range: MassRange) -> u128 {
    mass_range.size() as u128 * max_mass_range.size() as u128
}

fn update_max_range_from_crit(possible_mass: &MassRange, max_mass_range: &MassRange) -> MassRange {
    // +1 because if its crit the threshold is ever so slightly more than it is.
    let minimum_crit_threshold = possible_mass.least + 1;
    let smallest_possible_hole = minimum_crit_threshold * 10;
    overlap(
        max_mass_range,
        &mr(smallest_possible_hole, max_mass_range.most),
    )
}

fn update_max_range_from_shrink(
    possible_mass: &MassRange,
    max_mass_range: &MassRange,
) -> MassRange {
    let largest_crit_threshold = possible_mass.least;
    let largest_possible_hole = largest_crit_threshold * 10;

    // +1 because if its shrink the threshold is ever so slightly more than it is.
    let minimum_shrink_threshold = possible_mass.most + 1;
    let smallest_possible_hole = minimum_shrink_threshold * 2;
    overlap(
        max_mass_range,
        &mr(smallest_possible_hole, largest_possible_hole),
    )
}

fn update_max_range_from_full(possible_mass: &MassRange, max_mass_range: &MassRange) -> MassRange {
    let largest_shrink_threshold = possible_mass.least;
    let largest_possible_hole = largest_shrink_threshold * 2;
    overlap(
        max_mass_range,
        &mr(max_mass_range.least, largest_possible_hole),
    )
}

fn full_range(max_size_range: &MassRange) -> MassRange {
    let smallest_non_shrink = max_size_range.least / 2;
    mr(smallest_non_shrink, max_size_range.most)
}

fn shrink_range(max_size_range: &MassRange) -> MassRange {
    let largest_shrink = max_size_range.most / 2 - 1;
    let smallest_non_crit = max_size_range.least / 10;
    mr(smallest_non_crit, largest_shrink)
}

fn crit_range(max_size_range: &MassRange) -> MassRange {
    let largest_crit = max_size_range.most / 10 - 1;
    mr(1, largest_crit)
}

fn closed_range() -> MassRange {
    mr(Mass::MIN, 0)
}

fn overlap(range1: &MassRange, range2: &MassRange) -> MassRange {
    mr(range1.least.max(range2.least), range1.most.min(range2.most))
}

#[cfg(test)]
mod tests {
    use crate::{
        hole_info::{HoleInfo, mr},
        roll_calc::{
            crit_range, full_range, overlap, shrink_range, update_max_range_from_crit,
            update_max_range_from_full, update_max_range_from_shrink,
        },
    };

    #[test]
    fn test_full_range() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(full_range(&hole.max_range), mr(900_000_000, 2_200_000_000));
        let hole = HoleInfo::from_kg(1_000_000_000);
        assert_eq!(full_range(&hole.max_range), mr(450_000_000, 1_100_000_000));
    }

    #[test]
    fn test_shrink_range() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            shrink_range(&hole.max_range),
            mr(180_000_000, 1_100_000_000 - 1)
        );
        let hole = HoleInfo::from_kg(1_000_000_000);
        assert_eq!(
            shrink_range(&hole.max_range),
            mr(90_000_000, 550_000_000 - 1)
        )
    }

    #[test]
    fn test_crit_range() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(crit_range(&hole.max_range), mr(1, 220_000_000 - 1));
        let hole = HoleInfo::from_kg(1_000_000_000);
        assert_eq!(crit_range(&hole.max_range), mr(1, 110_000_000 - 1));
    }

    #[test]
    fn test_overlap() {
        assert_eq!(overlap(&mr(0, 10), &mr(3, 14)), mr(3, 10));
        assert_eq!(overlap(&mr(0, 10), &mr(3, 6)), mr(3, 6));
        assert_eq!(overlap(&mr(0, 10), &mr(-3, 7)), mr(0, 7));
        assert_eq!(overlap(&mr(4, 4), &mr(0, 10)), mr(4, 4));
    }

    #[test]
    fn test_max_from_crit() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            update_max_range_from_crit(&mr(200_000_000, 220_000_000), &hole.max_range),
            mr(2_000_000_010, 2_200_000_000)
        );
        assert_eq!(
            update_max_range_from_crit(&mr(190_000_000, 220_000_000), &hole.max_range),
            mr(1_900_000_010, 2_200_000_000)
        );
        let hole = HoleInfo::from_kg(1_000_000_000);
        assert_eq!(
            update_max_range_from_crit(&mr(100_000_000, 110_000_000), &hole.max_range),
            mr(1_000_000_010, 1_100_000_000)
        )
    }

    #[test]
    fn test_max_from_shrink() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            update_max_range_from_shrink(&mr(200_000_000, 700_000_000), &hole.max_range),
            mr(1_800_000_000, 2_000_000_000)
        );
        assert_eq!(
            update_max_range_from_shrink(&mr(300_000_000, 1_000_000_000), &hole.max_range),
            mr(2_000_000_002, 2_200_000_000)
        );
    }

    #[test]
    fn test_max_from_full() {
        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            update_max_range_from_full(&mr(1_000_000_000, 1_200_000_000), &hole.max_range),
            mr(1_800_000_000, 2_000_000_000)
        );
    }
}
