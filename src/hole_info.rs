use std::ops::Sub;

use crate::roll_calc::HoleState;

pub type Mass = i64;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, PartialOrd, Ord)]
pub struct MassRange {
    pub least: Mass,
    pub most: Mass,
}

impl MassRange {
    pub fn size(&self) -> Mass {
        // +1 because inclusive
        (self.most - self.least + 1).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.most < self.least
    }
}

impl Sub<Mass> for MassRange {
    type Output = Self;

    fn sub(self, rhs: Mass) -> Self::Output {
        Self {
            least: self.least - rhs,
            most: self.most - rhs,
        }
    }
}

pub struct HoleInfo {
    pub average_max_size: Mass,
    pub max_range: MassRange,
}

impl HoleInfo {
    pub const fn from_kg(mass: Mass) -> Self {
        let avg = kg(mass);
        let up = avg * 11 / 10;
        let down = avg * 9 / 10;
        Self {
            average_max_size: avg,
            max_range: mr(down, up),
        }
    }

    pub fn mass_range(&self, state: HoleState) -> MassRange {
        match state {
            HoleState::Full => self.full_mass_range(),
            HoleState::Shrink => self.shrink_mass_range(),
            HoleState::Crit => self.crit_mass_range(),
        }
    }

    pub fn full_mass_range(&self) -> MassRange {
        mr(self.max_range.least / 2, self.max_range.most)
    }

    pub fn shrink_mass_range(&self) -> MassRange {
        mr(self.max_range.least / 10, self.max_range.most / 2 - 1)
    }

    pub fn crit_mass_range(&self) -> MassRange {
        mr(1, self.max_range.most / 10 - 1)
    }
}

pub const fn mr(least: Mass, most: Mass) -> MassRange {
    MassRange { least, most }
}

pub const EMPTY_MR: MassRange = MassRange { least: 1, most: 0 };

pub const HOLES_INFO: [HoleInfo; 8] = [
    HoleInfo::from_kg(100_000_000),
    HoleInfo::from_kg(500_000_000),
    HoleInfo::from_kg(750_000_000),
    HoleInfo::from_kg(1_000_000_000),
    HoleInfo::from_kg(2_000_000_000),
    HoleInfo::from_kg(3_000_000_000),
    HoleInfo::from_kg(3_300_000_000),
    HoleInfo::from_kg(5_000_000_000),
];

pub const fn kg(kg: Mass) -> Mass {
    kg
}

pub const fn kgr(kg: MassRange) -> MassRange {
    kg
}
