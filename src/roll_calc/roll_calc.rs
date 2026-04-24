use std::{array, f64::EPSILON, hint};

use bumpalo::Bump;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use tinyvec::ArrayVec;

use crate::{
    best_path_picker::{BestOptions, PathPicker, Priorities, Qualities},
    chart_gen::RollingChart,
    hole_info::{EMPTY_MR, Mass, MassRange, mr},
    roll_calc::graph_builder::generate_roll_chart,
};

const EFFICIENT_NUM_ROLLERS: usize = 2;
const EFFICIENT_NUM_CACHED_STEPS: usize = 2;
const EFFICIENT_MAX_NUM_OUT: usize = 4;
const EFFICIENT_NUM_AVAILABLE_ROLLERS: usize = 2;
const EFFICIENT_NUM_SHIP_STATES: usize = 2;

type Memoizer<'a> = FxHashMap<RollState, SmallVec<[&'a RollPlan<'a>; EFFICIENT_NUM_CACHED_STEPS]>>;

struct StaticData {
    max_single_jump_mass: Mass,
}

#[derive(Debug, Clone)]
pub struct RollPlan<'a> {
    pub qualities: Qualities,
    pub decision: RollDecision<'a>,
    pub mass_range: MassRange,
    pub max_mass_range: MassRange,
}

#[derive(Debug, Clone)]
pub struct RollDecision<'a> {
    pub can_close: bool,
    pub crit: Option<&'a RollStep<'a>>,
    pub shrink: Option<&'a RollStep<'a>>,
    pub full: Option<&'a RollStep<'a>>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct RollState {
    pub remaining_mass: MassRange,
    pub rollers_out: RollersUsed,
    pub max_size_range: MassRange,
    pub highest_hole_state: HoleState,
    pub used_ships: RollersUsed,
}

#[derive(Debug, Clone)]
pub struct RollStep<'a> {
    pub next_plan: &'a RollPlan<'a>,
    pub direction: Direction,
    pub ship_state: ShipState,
    pub ship: Ship,
}

type RollerOutBacking = u64;
const MAX_NUM_ROLLERS: usize = size_of::<RollerOutBacking>();

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct RollersUsed(RollerOutBacking);

impl RollersUsed {
    pub fn new() -> Self {
        Self(0)
    }

    /// Increments the count for a given ship index (0 to 7)
    pub fn add(&mut self, ship_index: usize) {
        debug_assert!(ship_index < MAX_NUM_ROLLERS, "Too many ship types!");
        let shift = Self::shift(ship_index);
        let current_count = (self.0 >> shift) & 0xFF;

        // Clear the old byte and insert the new byte
        self.0 &= !(0xFF << shift);
        self.0 |= (current_count.strict_add(1)) << shift;
    }

    pub fn sub(&mut self, ship_index: usize) {
        debug_assert!(ship_index < MAX_NUM_ROLLERS, "Too many ship types!");
        let shift = Self::shift(ship_index);
        let current_count = (self.0 >> shift) & 0xFF;

        // Clear the old byte and insert the new byte
        self.0 &= !(0xFF << shift);
        self.0 |= (current_count - 1) << shift;
    }

    /// Gets the count for a given ship index
    pub fn get(&self, ship_index: usize) -> u8 {
        debug_assert!(ship_index < MAX_NUM_ROLLERS, "ship index out of bounds");
        ((self.0 >> (ship_index * 8)) & 0xFF) as u8
    }

    fn shift(ship_index: usize) -> usize {
        ship_index * 8
    }

    pub fn num_rollers_out(&self) -> u16 {
        let mut num = 0;
        for i in 0..MAX_NUM_ROLLERS {
            num += self.get(i) as u16;
        }
        num
    }

    pub fn any_out(&self) -> bool {
        self.num_rollers_out() > 0
    }
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum HoleState {
    Full,
    Shrink,
    Crit,
}

impl Default for HoleState {
    fn default() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Ship {
    pub hot: Mass,
    pub cold: Mass,
}

pub struct AvailabileShips {
    pub ship: Ship,
    pub max_num_out: u8,
    pub max_used: u8,
}

impl Ship {
    pub fn minimum_mass(&self) -> Mass {
        self.cold
    }

    pub fn maximum_mass(&self) -> Mass {
        self.hot
    }
}

pub fn get_best_roll_chart(
    available_rollers: &[AvailabileShips],
    state: RollState,
    starting_state: HoleState,
    priorities: &Priorities,
    max_memory: Option<usize>,
) -> (RollingChart, Qualities) {
    assert!(available_rollers.len() <= MAX_NUM_ROLLERS);
    let arena = Bump::new();
    arena.set_allocation_limit(max_memory);
    let mut memoization = FxHashMap::default();

    let max_single_jump_mass = available_rollers
        .iter()
        .map(|s| s.ship.maximum_mass()) // Assuming your ship has cold/hot mass properties
        .max()
        .unwrap() as Mass;
    let static_data = StaticData {
        max_single_jump_mass,
    };

    let plan = RollPlan::clone(&get_best_roll_plan_rec(
        available_rollers,
        state,
        priorities,
        Some(starting_state),
        &mut memoization,
        &arena,
        &static_data,
    ));
    println!("Num explored states = {}", memoization.len());

    (generate_roll_chart(&plan), plan.qualities)
}

fn get_best_roll_plan_rec<'a>(
    available_rollers: &[AvailabileShips],
    state: RollState,
    priorities: &Priorities,
    assume_state: Option<HoleState>,
    memoization: &mut Memoizer<'a>,
    arena: &'a Bump,
    static_data: &StaticData,
) -> &'a RollPlan<'a> {
    let base_max_num_out = state.rollers_out.num_rollers_out();

    // Fetch memoized best plan if it exists
    if let Some(cached_plan) = get_best_from_memoization(&state, base_max_num_out, memoization) {
        return cached_plan;
    }

    // calculate the possible masses that the hole could be if it were to become each state
    let max_mass_ranges = [
        crit_range(&state.max_size_range),
        shrink_range(&state.max_size_range),
        full_range(&state.max_size_range),
    ];
    let mut mass_ranges: [MassRange; 3] =
        array::from_fn(|i| overlap(&max_mass_ranges[i], &state.remaining_mass));
    let mass_range_for_closed = overlap(&closed_range(), &state.remaining_mass);

    let mut should_ignore_state = [false; 3];
    match state.highest_hole_state {
        HoleState::Crit => {
            should_ignore_state[1] = true;
            should_ignore_state[2] = true;
        }
        HoleState::Shrink => should_ignore_state[2] = true,
        HoleState::Full => (),
    }

    // If we are assuming a specific state (likely because the user knows the hole state) make all other possibilities not possible
    if let Some(starting_state) = assume_state {
        // Cold because should only ever happen in the first call.
        hint::cold_path();
        let stay_full = match starting_state {
            HoleState::Crit => 0,
            HoleState::Shrink => 1,
            HoleState::Full => 2,
        };
        for i in 0..3 {
            if i != stay_full {
                should_ignore_state[i] = true;
            }
        }
    }

    for (i, should_ignore) in should_ignore_state.iter().enumerate() {
        if *should_ignore {
            mass_ranges[i] = EMPTY_MR;
        }
    }

    let possible_closed_states = calc_possible_states(mass_range_for_closed, state.max_size_range);

    // Assuming it became that state, calculate the new max mass ranges for each hole based on the possible mass range.
    const MAX_RANGE_UPDATERS: [fn(&MassRange, &MassRange) -> MassRange; 3] = [
        update_max_range_from_crit,
        update_max_range_from_shrink,
        update_max_range_from_full,
    ];
    let new_max_ranges: [MassRange; 3] =
        array::from_fn(|i| MAX_RANGE_UPDATERS[i](&mass_ranges[i], &state.max_size_range));

    // Get the possible number of states that could result in each hole state.
    let possible_states: [u128; 3] = array::from_fn(|i| {
        if !mass_ranges[i].is_empty() {
            calc_possible_states(mass_ranges[i], new_max_ranges[i])
        } else {
            0
        }
    });

    let all_possible_states = possible_closed_states + possible_states.iter().sum::<u128>();

    // If the hole is closed, the rollout probability is 0 if all rollers are in. Otherwise, it's 1.
    let closed_roll_out_probability_raw = if !state.rollers_out.any_out() {
        0.0
    } else {
        1.0
    };

    if all_possible_states == possible_closed_states {
        return arena.alloc(RollPlan {
            qualities: Qualities {
                max_num_out: base_max_num_out,
                roll_out_probability: closed_roll_out_probability_raw,
                average_num_passes: 0.0,
            },
            decision: RollDecision {
                can_close: true,
                crit: None,
                shrink: None,
                full: None,
            },
            mass_range: state.remaining_mass,
            max_mass_range: state.max_size_range,
        });
    }

    let cared_about_states = array::from_fn(|i| possible_states[i] > 0);
    let mut best_paths = PathPicker::new(priorities, cared_about_states);

    let mut steps = get_steps(available_rollers, &state.rollers_out, &state.used_ships).into_iter();

    let first_step = steps.next().unwrap();

    let mut states_to_explore = ArrayVec::<[(usize, HoleState); 3]>::new();
    for i in 0..3 {
        if possible_states[i] > 0 {
            states_to_explore.push((i, [HoleState::Crit, HoleState::Shrink, HoleState::Full][i]));
        }
    }
    // Compute one step for each possible hole state to get a baseline to prune with.
    for (i, hole_state) in states_to_explore.iter() {
        compute_and_prune_or_add_step(
            &first_step,
            &mut best_paths,
            *hole_state,
            available_rollers,
            mass_ranges[*i],
            new_max_ranges[*i],
            &priorities,
            memoization,
            arena,
            static_data,
        );
    }

    // Compute other steps avoiding exploring any pathways guarentied to result in a worse plan.
    for step in steps {
        for (i, hole_state) in states_to_explore.iter() {
            compute_and_prune_or_add_step(
                &step,
                &mut best_paths,
                *hole_state,
                available_rollers,
                mass_ranges[*i],
                new_max_ranges[*i],
                &priorities,
                memoization,
                arena,
                static_data,
            );
        }
    }

    let BestOptions { best_paths, splits } = best_paths.best();

    let closed_probability = geometric_probability(possible_closed_states, all_possible_states);
    let probabilities: [f64; 3] =
        array::from_fn(|i| geometric_probability(possible_states[i], all_possible_states));

    let mut splits = splits.into_iter();
    let paths = best_paths.into_iter();
    let mut best_plans = SmallVec::new();
    let mut start;
    let mut end = Some(0);
    for steps in paths {
        // None end means infinite
        start = end.unwrap();
        end = splits.next();

        let plan = create_roll_plan(
            steps,
            &state,
            base_max_num_out,
            closed_probability * closed_roll_out_probability_raw,
            probabilities,
            possible_closed_states != 0,
            arena,
        );
        // When end is None the last one just has to be the worst case.
        for _ in start..(end.unwrap_or(start + 1)) {
            best_plans.push(&*plan);
        }
    }

    let best_for_this_run = get_best_plan_from_cached(base_max_num_out, &best_plans);
    memoization.insert(state, best_plans);

    best_for_this_run
}

fn get_best_from_memoization<'a>(
    state: &RollState,
    min_max_num_out: u16,
    memoization: &Memoizer<'a>,
) -> Option<&'a RollPlan<'a>> {
    if let Some(cached_plans) = memoization.get(&state) {
        Some(get_best_plan_from_cached(min_max_num_out, cached_plans))
    } else {
        None
    }
}

fn get_best_plan_from_cached<'a>(
    min_max_num_out: u16,
    cached_plans: &[&'a RollPlan<'a>],
) -> &'a RollPlan<'a> {
    &cached_plans[(min_max_num_out as usize).min(cached_plans.len() - 1)]
}

fn create_roll_plan<'a>(
    steps: [Option<RollStep<'a>>; 3],
    state: &RollState,
    mut min_max_num_out: u16,
    closed_roll_out_probability: f64,
    probabilities: [f64; 3],
    can_close: bool,
    arena: &'a Bump,
) -> &'a mut RollPlan<'a> {
    let mut steps = steps.into_iter();
    // Get the final best plan for each hole state.
    let plans = array::from_fn(|_| {
        steps
            .next()
            .unwrap()
            .map(|best_next| &*arena.alloc(best_next))
    });

    let rollout_probabilities: [f64; 3] = array::from_fn(|i| {
        if let Some(plan) = &plans[i] {
            plan.next_plan.qualities.roll_out_probability
        } else {
            0.0
        }
    });

    let average_passes: [f64; 3] = array::from_fn(|i| {
        if let Some(plan) = &plans[i] {
            plan.next_plan.qualities.average_num_passes
        } else {
            0.0
        }
    });

    let roll_out_probability = closed_roll_out_probability
        + probabilities[0] * rollout_probabilities[0]
        + probabilities[1] * rollout_probabilities[1]
        + probabilities[2] * rollout_probabilities[2];

    let average_num_passes = 1.0
        + probabilities[0] * average_passes[0]
        + probabilities[1] * average_passes[1]
        + probabilities[2] * average_passes[2];

    for plan in &plans {
        min_max_num_out = min_max_num_out.max(
            plan.as_ref()
                .map(|x| x.next_plan.qualities.max_num_out)
                .unwrap_or(0),
        );
    }
    let [crit, shrink, full] = plans;
    arena.alloc(RollPlan {
        qualities: Qualities {
            max_num_out: min_max_num_out,
            roll_out_probability,
            average_num_passes,
        },
        decision: RollDecision {
            can_close,
            crit,
            shrink,
            full,
        },
        mass_range: state.remaining_mass,
        max_mass_range: state.max_size_range,
    })
}

fn compute_and_prune_or_add_step<'a>(
    pass: &PotentialPass,
    best_paths: &mut PathPicker<'a, '_>,
    hole_state: HoleState,
    available_rollers: &[AvailabileShips],
    mass_range: MassRange,
    max_size_range: MassRange,
    priorities: &Priorities,
    memoization: &mut Memoizer<'a>,
    arena: &'a Bump,
    static_data: &StaticData,
) {
    let path_state = RollState {
        remaining_mass: mass_range - pass.mass,
        rollers_out: pass.new_out_rollers,
        max_size_range,
        highest_hole_state: hole_state,
        used_ships: pass.new_used_ships,
    };

    let minimum_qualities = minimum_possible_qualities(available_rollers, &path_state, static_data);

    if best_paths.should_prune(hole_state, &minimum_qualities) {
        return;
    }

    let next_plan = get_best_roll_plan_rec(
        available_rollers,
        path_state,
        priorities,
        None,
        memoization,
        arena,
        static_data,
    );

    best_paths.suggest(
        RollStep {
            next_plan,
            direction: pass.direction,
            ship_state: pass.ship_state,
            ship: pass.ship,
        },
        hole_state,
    );
}

fn minimum_possible_qualities(
    available_rollers: &[AvailabileShips],
    path_state: &RollState,
    static_data: &StaticData,
) -> Qualities {
    let mut minimum_mass_needed_to_go_though_without_closing = 0;
    let mut max_return_mass = 0;
    let mut largest_ship: Option<Ship> = None;
    for (i, ship) in available_rollers.iter().enumerate() {
        let num = path_state.rollers_out.get(i);
        minimum_mass_needed_to_go_though_without_closing += ship.ship.minimum_mass() * num as Mass;
        max_return_mass += ship.ship.maximum_mass() * num as Mass;

        if num == 0 {
            continue;
        }

        largest_ship = match largest_ship {
            Some(prev) if ship.ship.minimum_mass() > prev.minimum_mass() => Some(ship.ship),
            Some(prev) => Some(prev),
            None => Some(ship.ship),
        };
    }
    if let Some(ship) = largest_ship {
        minimum_mass_needed_to_go_though_without_closing -= ship.minimum_mass();
    }

    let minimum_rollout_chance = if !path_state.rollers_out.any_out() {
        0.0
    } else if minimum_mass_needed_to_go_though_without_closing >= path_state.remaining_mass.most {
        1.0
    } else if minimum_mass_needed_to_go_though_without_closing >= path_state.remaining_mass.least {
        // Might need to restrict to only crit holes because this calc may be slightly inacurate since
        // if not crit you may get info for when it does go crit/shrink
        let range_after_all_in =
            path_state.remaining_mass - minimum_mass_needed_to_go_though_without_closing;
        let num_rollout_masses = range_after_all_in.least.abs() + 1;
        let rollout_probability =
            num_rollout_masses as f64 / (range_after_all_in.most + num_rollout_masses) as f64;
        rollout_probability.max(EPSILON)
    } else {
        0.0
    };

    let num_rollers_out = path_state.rollers_out.num_rollers_out();

    let mut minimum_number_passes = num_rollers_out as f64;

    if max_return_mass < path_state.remaining_mass.least {
        // Even if everyone currently out returns Hot, the hole won't close.
        let mass_deficit = path_state.remaining_mass.least - max_return_mass;

        // Calculate the absolute minimum number of jumps required to clear the deficit
        // using the largest possible ship
        let required_extra_jumps =
            (mass_deficit as f64 / static_data.max_single_jump_mass as f64).ceil();

        minimum_number_passes += required_extra_jumps;
    } else if num_rollers_out == 0 && path_state.remaining_mass.least > 0 {
        // No ships out, no deficit, but hole is open. Must go out at least
        minimum_number_passes = 1.0;
    }

    Qualities {
        max_num_out: num_rollers_out,
        roll_out_probability: minimum_rollout_chance,
        average_num_passes: minimum_number_passes,
    }
}

struct PotentialPass {
    new_out_rollers: RollersUsed,
    mass: Mass,
    direction: Direction,
    ship_state: ShipState,
    ship: Ship,
    new_used_ships: RollersUsed,
}

const PREDICTED_NUM_POTENTIAL_PASSES: usize = (EFFICIENT_NUM_ROLLERS * EFFICIENT_MAX_NUM_OUT
    + EFFICIENT_NUM_AVAILABLE_ROLLERS)
    * EFFICIENT_NUM_SHIP_STATES;
fn get_steps(
    available_rollers: &[AvailabileShips],
    rollers_out: &RollersUsed,
    used_rollers: &RollersUsed,
) -> SmallVec<[PotentialPass; PREDICTED_NUM_POTENTIAL_PASSES]> {
    let mut potential_passes = SmallVec::new();
    for i in 0..available_rollers.len() {
        if rollers_out.get(i) == 0 {
            continue;
        }
        let mut new_out_rollers = *rollers_out;
        let in_ship = available_rollers[i].ship;
        new_out_rollers.sub(i);

        potential_passes.push(PotentialPass {
            new_out_rollers: new_out_rollers,
            mass: in_ship.cold,
            direction: Direction::In,
            ship_state: ShipState::Cold,
            ship: in_ship,
            new_used_ships: *used_rollers,
        });
        potential_passes.push(PotentialPass {
            new_out_rollers: new_out_rollers,
            mass: in_ship.hot,
            direction: Direction::In,
            ship_state: ShipState::Hot,
            ship: in_ship,
            new_used_ships: *used_rollers,
        });
    }

    for (i, out_ship) in available_rollers.iter().enumerate() {
        let out_ship = out_ship.ship;
        let mut new_out_rollers = *rollers_out;
        new_out_rollers.add(i);
        let mut new_used_rollers = *used_rollers;
        new_used_rollers.add(i);
        if new_used_rollers.get(i) > available_rollers[i].max_used {
            continue;
        }
        potential_passes.push(PotentialPass {
            new_out_rollers: new_out_rollers,
            mass: out_ship.cold,
            direction: Direction::Out,
            ship_state: ShipState::Cold,
            ship: out_ship,
            new_used_ships: new_used_rollers,
        });
        potential_passes.push(PotentialPass {
            new_out_rollers: new_out_rollers,
            mass: out_ship.hot,
            direction: Direction::Out,
            ship_state: ShipState::Hot,
            ship: out_ship,
            new_used_ships: new_used_rollers,
        })
    }

    potential_passes
}

fn geometric_probability(possibilities: u128, all_possibilities: u128) -> f64 {
    if all_possibilities == 0 {
        panic!()
    }
    possibilities as f64 / all_possibilities as f64
}

fn calc_possible_states(mass_range: MassRange, max_mass_range: MassRange) -> u128 {
    mass_range.size() as u128 * max_mass_range.size() as u128
}

fn update_max_range_from_crit(possible_mass: &MassRange, max_mass_range: &MassRange) -> MassRange {
    // +1 because if its shrink the threshold is ever so slightly more than it is.
    let minimum_shrink_threshold = possible_mass.least;
    let smallest_possible_hole = minimum_shrink_threshold * 10 + 1;
    mr(
        smallest_possible_hole.max(max_mass_range.least),
        max_mass_range.most,
    )
}

fn update_max_range_from_shrink(
    possible_mass: &MassRange,
    max_mass_range: &MassRange,
) -> MassRange {
    let largest_crit_threshold = possible_mass.most;
    let largest_possible_hole = largest_crit_threshold * 10;

    // +1 because if its shrink the threshold is ever so slightly more than it is.
    let minimum_shrink_threshold = possible_mass.least;
    let smallest_possible_hole = minimum_shrink_threshold * 2 + 1;
    mr(
        smallest_possible_hole.max(max_mass_range.least),
        largest_possible_hole.min(max_mass_range.most),
    )
}

fn update_max_range_from_full(possible_mass: &MassRange, max_mass_range: &MassRange) -> MassRange {
    let largest_shrink_threshold = possible_mass.most;
    let largest_possible_hole = largest_shrink_threshold * 2;
    mr(
        max_mass_range.least,
        largest_possible_hole.min(max_mass_range.most),
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
        roll_calc::roll_calc::{
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
            mr(1_800_000_000, 2_200_000_000)
        );

        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            update_max_range_from_full(&mr(900_000_000, 2_200_000_000), &hole.max_range),
            mr(1_800_000_000, 2_200_000_000)
        );

        let hole = HoleInfo::from_kg(2_000_000_000);
        assert_eq!(
            update_max_range_from_full(&mr(900_000_000, 1_000_000_000), &hole.max_range),
            mr(1_800_000_000, 2_000_000_000)
        );
    }
}
