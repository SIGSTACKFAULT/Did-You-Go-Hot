use std::{array, cmp::Ordering, collections::HashSet};

use crate::{hole_info::Mass, roll_calc::{HoleState, RollPlan, RollStep, ShipState}};

pub type PathCmpFn<'a> = &'a dyn Fn(&RollPlan, &RollPlan) -> Ordering;

pub struct PathPicker<'a> {
    best_paths: [Vec<RollStep>; 3],
    cmp_fn: PathCmpFn<'a>,
}

impl<'a> PathPicker<'a> {
    pub fn new(path_cmp: PathCmpFn<'a>) -> Self {
        Self {
            best_paths: Default::default(),
            cmp_fn: path_cmp,
        }
    }

    pub fn suggest(&mut self, step: RollStep, hole_state: HoleState) {
        let paths = match hole_state {
            HoleState::Crit => &mut self.best_paths[0],
            HoleState::Shrink => &mut self.best_paths[1],
            HoleState::Full => &mut self.best_paths[2],
        };
        if let Some(best_sample) = paths.first() {
            match (self.cmp_fn)(&best_sample.next_plan, &step.next_plan) {
                Ordering::Equal => paths.push(step),
                Ordering::Less => {
                    paths.clear();
                    paths.push(step);
                },
                Ordering::Greater => (),
            }
        } else {
            paths.push(step);
        }
    }

    pub fn best(self) -> [Option<RollStep>; 3] {
        // Tries to make the chart less confusing by trying to match closing jumps by prioritizing higher mass.
        let mut all_options = self.best_paths.into_iter();
        let best_combination: [Option<RollStep>; 3] = array::from_fn(|_| {
            let mut options = all_options.next().unwrap();
            let mut best = options.pop()?;
            let mut best_mass = step_mass(&best);
            for option in options {
                let option_mass = step_mass(&option);
                if best_mass < option_mass {
                    best = option;
                    best_mass = option_mass;
                }
            }
            Some(best)
        });
        best_combination
    }
}


fn step_mass(step: &RollStep) -> Mass {
    match step.ship_state {
        ShipState::Cold => step.ship.cold,
        ShipState::Hot => step.ship.hot,
    }
}