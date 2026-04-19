use std::{array, cmp::Ordering};

use smallvec::SmallVec;

use crate::{
    hole_info::Mass,
    roll_calc::{HoleState, RollStep, ShipState},
};

const EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER: usize = 2;
const EFFICIENT_NUM_BEST_OPTIONS: usize = 2;

// Invariants:
// best_paths contains paths of equal quality or thoes with the potential for higher or equal qulaity if max out decreases
// best_paths each vec must be sorted such that the left most element is best if highest_max_out = its max out
//      E.G. As highest_max_out decreases, the left most elements are better than the right most paths
pub struct PathPicker<'a, 'b> {
    best_paths: [SmallVec<[RollStep<'a>; EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER]>; 3],
    priorities: &'b Priorities,
    care_about_state: [bool; 3],
}

struct ComplexComparison<'a> {
    new_best_path: SmallVec<[RollStep<'a>; EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER]>,
    insert_new_at: usize,
}

enum Comparison<'a> {
    Equal,
    StrictlyBetter,
    StrictlyWorse,
    Complex(ComplexComparison<'a>),
}

enum ComparisonTheoretical {
    Equal,
    StrictlyBetter,
    StrictlyWorse,
    PotentiallyBetter,
}

impl<'a, 'b> PathPicker<'a, 'b> {
    pub fn new(priorities: &'b Priorities, care_about_state: [bool; 3]) -> Self {
        Self {
            best_paths: Default::default(),
            priorities,
            care_about_state,
        }
    }

    pub fn should_prune(&self, hole_state: HoleState, minimum_qualities: &Qualities) -> bool {
        let path_i = match hole_state {
            HoleState::Crit => 0,
            HoleState::Shrink => 1,
            HoleState::Full => 2,
        };
        let Some(representative_sample) = self.best_paths[path_i].first() else {
            return false;
        };

        matches!(
            cmp_theoretical(
                &representative_sample.next_plan.qualities,
                minimum_qualities,
                &self.priorities.qualities,
                &self.best_paths,
                path_i,
                self.have_full_info(),
            ),
            ComparisonTheoretical::StrictlyWorse,
        )
    }

    fn have_full_info(&self) -> bool {
        self.care_about_state
            .iter()
            .enumerate()
            .all(|(i, care)| !*care || !self.best_paths[i].is_empty())
    }

    pub fn suggest(&mut self, step: RollStep<'a>, hole_state: HoleState) {
        let path_i = match hole_state {
            HoleState::Crit => 0,
            HoleState::Shrink => 1,
            HoleState::Full => 2,
        };
        if self.best_paths[path_i].is_empty() {
            self.best_paths[path_i].push(step);
        } else {
            let representative_sample = self.best_paths[path_i].first().unwrap();
            let have_full_info = self.have_full_info();
            match cmp(
                &representative_sample.next_plan.qualities,
                &step.next_plan.qualities,
                &self.priorities.qualities,
                &self.best_paths,
                path_i,
                have_full_info,
            ) {
                Comparison::Equal => {
                    self.best_paths[path_i].push(step);
                }
                Comparison::StrictlyBetter => {
                    self.best_paths[path_i].clear();
                    self.best_paths[path_i].push(step);
                }
                Comparison::StrictlyWorse => (),
                Comparison::Complex(data) => {
                    self.best_paths[path_i] = data.new_best_path;
                    self.best_paths[path_i].insert(data.insert_new_at, step);
                }
            }
        }
    }

    // Returns all best paths for each minimum max out floors.
    // Vec index = minimum max out floor
    pub fn best(mut self) -> BestOptions<'a> {
        // Get it to a single plan per max_num_out value
        // Keeping the highest mass being passed to try to improve path similarity.
        for path in self.best_paths.iter_mut() {
            let mut i = 1;
            while i < path.len() {
                if path[i - 1].next_plan.qualities.max_num_out
                    == path[i].next_plan.qualities.max_num_out
                {
                    match step_mass(&path[i - 1]).cmp(&step_mass(&path[i])) {
                        Ordering::Equal | Ordering::Greater => path.remove(i),
                        Ordering::Less => path.remove(i - 1),
                    };
                } else {
                    i += 1;
                }
            }
        }

        // The highest floor that would actually change the best is the highest max_num_out in the best paths
        let Some(highest_significant_max_out_floor) = self
            .best_paths
            .iter()
            .flatten()
            .map(|x| x.next_plan.qualities.max_num_out)
            .max()
        else {
            return BestOptions {
                best_paths: SmallVec::new(),
                splits: SmallVec::new(),
            };
        };

        let mut possible_combos: SmallVec<[[Option<RollStep>; 3]; EFFICIENT_NUM_BEST_OPTIONS]> =
            SmallVec::new();
        let mut splits = SmallVec::new();

        // push starting best combo
        possible_combos.push(array::from_fn(|j| self.best_paths[j].get(0).cloned()));

        let mut highest_so_far = [0; 3];
        for i in 0..=highest_significant_max_out_floor {
            let mut found_split = false;
            for j in 0..3 {
                if let Some(next_possible_step) = self.best_paths[j].get(highest_so_far[j] + 1) {
                    if next_possible_step.next_plan.qualities.max_num_out <= i {
                        highest_so_far[j] += 1;
                        if !found_split {
                            splits.push(i as usize);
                            found_split = true;
                        }
                    }
                }
            }

            if found_split {
                possible_combos.push(array::from_fn(|j| {
                    self.best_paths[j].get(highest_so_far[j]).cloned()
                }));
            }
        }

        BestOptions {
            best_paths: possible_combos,
            splits,
        }
    }
}

// best_paths[path_i] must be properly filtered such all elements with higher prioirty qualities which are better are not included.
// best_paths must be sorted to match the invariants of PathPicker.
fn cmp<'a>(
    existing: &Qualities,
    new: &Qualities,
    priorities: &[Quality],
    best_paths: &[SmallVec<[RollStep<'a>; EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER]>; 3],
    path_i: usize,
    have_full_info: bool,
) -> Comparison<'a> {
    if priorities.len() == 0 {
        return Comparison::Equal;
    }

    let comparison = match priorities[0] {
        Quality::ROProbability => {
            float_cmp_lower_better(existing.roll_out_probability, new.roll_out_probability)
        }
        Quality::AvgNumPasses => {
            float_cmp_lower_better(existing.average_num_passes, new.average_num_passes)
        }
        Quality::MaxOut => {
            // Forcefully return Equal if we don't have full info because in the future a high max out may be fine
            if !have_full_info {
                return Comparison::Equal;
            }
            // Either find out we are strictly worse than someone else or find all paths we are strictly better than.
            let mut strictly_better_than =
                SmallVec::<[usize; EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER]>::new();
            let mut insert_at = 0;
            for (i, path) in best_paths[path_i].iter().enumerate() {
                if path.next_plan.qualities.max_num_out < new.max_num_out {
                    insert_at = i + 1;
                }

                match cmp(
                    &path.next_plan.qualities,
                    new,
                    &priorities[1..],
                    best_paths,
                    path_i,
                    have_full_info,
                ) {
                    Comparison::StrictlyWorse => {
                        // Found a path with less or equal max_num_out with better other qualities.
                        // This is strictly better even if max_num_out is reduced
                        if path.next_plan.qualities.max_num_out <= new.max_num_out {
                            return Comparison::StrictlyWorse;
                        }
                    }
                    Comparison::StrictlyBetter => {
                        // it has an equal or better max_num_out so if all lower priorities are strictly better, it's strictly better
                        // Not all later ones will be strictly worse, so we remember which ones we are better than.
                        if path.next_plan.qualities.max_num_out >= new.max_num_out {
                            strictly_better_than.push(i);
                        }
                    }
                    Comparison::Equal => {
                        // If all else is equal but our max_num_out is worse, we are strictly worse.
                        if path.next_plan.qualities.max_num_out < new.max_num_out {
                            return Comparison::StrictlyWorse;
                        } else if path.next_plan.qualities.max_num_out > new.max_num_out {
                            strictly_better_than.push(i);
                        }
                    }
                    Comparison::Complex(_) => {
                        unreachable!()
                    }
                }
            }

            if strictly_better_than.is_empty() {
                return Comparison::Equal;
            }

            let mut new_paths: SmallVec<[RollStep<'a>; 2]> = SmallVec::new();
            let mut better_than_i = 0;
            for (i, path) in best_paths[path_i].iter().enumerate() {
                if better_than_i < strictly_better_than.len()
                    && i == strictly_better_than[better_than_i]
                {
                    better_than_i += 1;
                    continue;
                }
                new_paths.push(path.clone());
            }

            Comparison::Complex(ComplexComparison {
                new_best_path: new_paths,
                insert_new_at: insert_at,
            })
        }
    };
    if matches!(comparison, Comparison::Equal) {
        cmp(
            existing,
            new,
            &priorities[1..],
            best_paths,
            path_i,
            have_full_info,
        )
    } else {
        comparison
    }
}

// best_paths[path_i] must be properly filtered such all elements with higher prioirty qualities which are better are not included.
// best_paths must be sorted to match the invariants of PathPicker.
fn cmp_theoretical(
    existing: &Qualities,
    new: &Qualities,
    priorities: &[Quality],
    best_paths: &[SmallVec<[RollStep; EFFICIENT_NUM_EQUAL_OR_POTENTIALLY_BETTER]>; 3],
    path_i: usize,
    have_full_info: bool,
) -> ComparisonTheoretical {
    if priorities.len() == 0 {
        return ComparisonTheoretical::Equal;
    }

    let comparison = match priorities[0] {
        Quality::ROProbability => float_cmp_lower_better_theoretical(
            existing.roll_out_probability,
            new.roll_out_probability,
        ),
        Quality::AvgNumPasses => {
            float_cmp_lower_better_theoretical(existing.average_num_passes, new.average_num_passes)
        }
        Quality::MaxOut => {
            // Forcefully return Equal if we don't have full info because in the future a high max out may be fine
            if !have_full_info {
                return ComparisonTheoretical::Equal;
            }
            // Either find out we are strictly worse than someone else or find all paths we are strictly better than.
            for path in &best_paths[path_i] {
                match cmp_theoretical(
                    &path.next_plan.qualities,
                    new,
                    &priorities[1..],
                    best_paths,
                    path_i,
                    have_full_info,
                ) {
                    ComparisonTheoretical::StrictlyWorse => {
                        // Found a path with less or equal max_num_out with better other qualities.
                        // This is strictly better even if max_num_out is reduced
                        if path.next_plan.qualities.max_num_out <= new.max_num_out {
                            return ComparisonTheoretical::StrictlyWorse;
                        }
                    }
                    ComparisonTheoretical::StrictlyBetter => {
                        // it has an equal or better max_num_out so if all lower priorities are strictly better, it's strictly better
                        if path.next_plan.qualities.max_num_out >= new.max_num_out {
                            return ComparisonTheoretical::PotentiallyBetter;
                        }
                    }
                    ComparisonTheoretical::Equal => {
                        // If all else is equal but our max_num_out is worse, we are strictly worse.
                        if path.next_plan.qualities.max_num_out < new.max_num_out {
                            return ComparisonTheoretical::StrictlyWorse;
                        } else if path.next_plan.qualities.max_num_out > new.max_num_out {
                            return ComparisonTheoretical::PotentiallyBetter;
                        }
                    }
                    ComparisonTheoretical::PotentiallyBetter => {
                        unreachable!()
                    }
                }
            }
            ComparisonTheoretical::Equal
        }
    };
    if matches!(comparison, ComparisonTheoretical::Equal) {
        cmp_theoretical(
            existing,
            new,
            &priorities[1..],
            best_paths,
            path_i,
            have_full_info,
        )
    } else {
        comparison
    }
}

pub struct BestOptions<'a> {
    pub best_paths: SmallVec<[[Option<RollStep<'a>>; 3]; EFFICIENT_NUM_BEST_OPTIONS]>,
    pub splits: SmallVec<[usize; EFFICIENT_NUM_BEST_OPTIONS]>,
}

fn float_cmp_lower_better<'a>(a: f64, b: f64) -> Comparison<'a> {
    if a < b {
        Comparison::StrictlyWorse
    } else if a > b {
        Comparison::StrictlyBetter
    } else {
        Comparison::Equal
    }
}

fn float_cmp_lower_better_theoretical(a: f64, b: f64) -> ComparisonTheoretical {
    if a < b {
        ComparisonTheoretical::StrictlyWorse
    } else if a > b {
        ComparisonTheoretical::StrictlyBetter
    } else {
        ComparisonTheoretical::Equal
    }
}

fn step_mass(step: &RollStep) -> Mass {
    match step.ship_state {
        ShipState::Cold => step.ship.cold,
        ShipState::Hot => step.ship.hot,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    MaxOut,
    ROProbability,
    AvgNumPasses,
}

pub struct Priorities {
    qualities: Vec<Quality>,
}

impl Priorities {
    pub fn new(qualities: Vec<Quality>) -> Option<Self> {
        for i in 0..qualities.len() {
            for j in i + 1..qualities.len() {
                if qualities[i] == qualities[j] {
                    return None;
                }
            }
        }
        Some(Self { qualities })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Qualities {
    pub max_num_out: u16,
    pub roll_out_probability: f64,
    pub average_num_passes: f64,
}
