use std::{array, fmt::Display, mem, ops::RangeInclusive};

use eframe::{
    egui::{CentralPanel, ComboBox, Grid, Panel, Slider, TextBuffer, Ui},
    emath::Numeric,
};

use crate::{
    chart_gen::{PassDecision, PeakedOptions, RollingChart, actions_to_text},
    hole_info::{HoleInfo, Mass},
    roll_calc::Ship,
};

const MAX_NUM_AVAILABLE: usize = 10;
const MAX_MAX_USES: usize = 15;
const DEFAULT_MASS_TEXT: &str = "Enter mass in tons";
const DEFAULT_NAME_TEXT: &str = "Enter ship name";

pub fn run_app() {
    eframe::run_native(
        "Roll App",
        Default::default(),
        Box::new(|cc| Ok(Box::new(RollApp::new()))),
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
enum HollSizes {
    M100,
    M500,
    M750,
    B1,
    B2,
    B3,
    B3_3,
    B5,
}

impl Display for HollSizes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HollSizes::M100 => write!(f, "100M"),
            HollSizes::M500 => write!(f, "500M"),
            HollSizes::M750 => write!(f, "750M"),
            HollSizes::B1 => write!(f, "1B"),
            HollSizes::B2 => write!(f, "2B"),
            HollSizes::B3 => write!(f, "3B"),
            HollSizes::B3_3 => write!(f, "3.3B"),
            HollSizes::B5 => write!(f, "5B"),
        }
    }
}

impl HollSizes {
    fn hole_info(&self) -> HoleInfo {
        match self {
            HollSizes::M100 => HoleInfo::from_kg(100_000_000),
            HollSizes::M500 => HoleInfo::from_kg(500_000_000),
            HollSizes::M750 => HoleInfo::from_kg(750_000_000),
            HollSizes::B1 => HoleInfo::from_kg(1_000_000_000),
            HollSizes::B2 => HoleInfo::from_kg(2_000_000_000),
            HollSizes::B3 => HoleInfo::from_kg(3_000_000_000),
            HollSizes::B3_3 => HoleInfo::from_kg(3_300_000_000),
            HollSizes::B5 => HoleInfo::from_kg(5_000_000_000),
        }
    }
}

struct ProvidedShip {
    ship: Ship,
    name: String,
    enabled: bool,
    number_available: Option<usize>,
    max_uses: Option<usize>,
}

struct ChartGuide {
    chart: RollingChart,
    next_options: CachedNextOptions,
    previous_options: Vec<String>,
    path: Vec<PassDecision>,
}

struct RollApp {
    guide: Option<ChartGuide>,

    selected_hole: Option<HollSizes>,
    ships: Vec<ProvidedShip>,
    adding_hot: String,
    adding_cold: String,
    adding_name: String,
}

impl eframe::App for RollApp {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        Panel::left("Selection pannel").show_inside(ui, |ui| {
            self.show_selection_panel(ui);
        });
        CentralPanel::default_margins().show_inside(ui, |ui| {
            self.walkthrough_roll(ui);
        });
    }
}

impl RollApp {
    pub fn new() -> Self {
        let out = Self {
            selected_hole: None,
            ships: Vec::new(),
            adding_hot: DEFAULT_MASS_TEXT.to_string(),
            adding_cold: DEFAULT_MASS_TEXT.to_string(),
            adding_name: DEFAULT_NAME_TEXT.to_string(),
            guide: None,
        };
        out
    }

    fn add_chart_guide(&mut self, chart: RollingChart) {
        let first_move = match chart.chart_walker().peak_options() {
            PeakedOptions::Closed => unreachable!(),
            PeakedOptions::Options(options) => {
                assert!(options.len() == 1);
                options[0].decision
            }
        };

        self.guide = Some(ChartGuide {
            chart,
            next_options: CachedNextOptions::Closed, // Temporary next_options which will be overwritten before being shown
            previous_options: vec![],
            path: vec![],
        });

        self.take_decision(first_move);
    }

    fn take_decision(&mut self, decision: PassDecision) {
        let guide = self.guide.as_mut().unwrap();
        guide.path.push(decision);
        let previous_next_options = mem::replace(
            &mut guide.next_options,
            next_options(&guide.chart, &guide.path),
        );
        let taken_actions = match previous_next_options {
            CachedNextOptions::Closed => unreachable!(),
            CachedNextOptions::Options(options) => {
                let i = OPTIONS_ORDER.iter().position(|x| *x == decision).unwrap();
                options.into_iter().skip(i).next().unwrap().unwrap()
            }
        };
        guide.previous_options.push(taken_actions);
    }

    fn show_selection_panel(&mut self, ui: &mut eframe::egui::Ui) {
        if ui.button("Reset").clicked() {
            if let Some(old_guide) = self.guide.take() {
                self.add_chart_guide(old_guide.chart);
            }
        }
        ComboBox::from_label("Select hole size")
            .selected_text(if let Some(hole) = &self.selected_hole {
                format!("{hole}")
            } else {
                "None".to_string()
            })
            .show_ui(ui, |ui| {
                for hole in [
                    HollSizes::M100,
                    HollSizes::M500,
                    HollSizes::M750,
                    HollSizes::B1,
                    HollSizes::B2,
                    HollSizes::B3,
                    HollSizes::B3_3,
                    HollSizes::B5,
                ] {
                    ui.selectable_value(&mut self.selected_hole, Some(hole), format!("{hole}"));
                }
            });

        Grid::new("ships grid").show(ui, |ui| {
            for ship in self.ships.iter_mut() {
                ui.checkbox(&mut ship.enabled, &ship.name);
                value_to_max_or_infinite(ui, &mut ship.number_available, 1..=MAX_NUM_AVAILABLE, 4);
                value_to_max_or_infinite(ui, &mut ship.max_uses, 1..=MAX_MAX_USES, 4);
            }
        });

        ui.label("Add rolling ships\nWrite in tons exactly\nas seen on the fitting window");
        ui.label("Hot:");
        get_mass(ui, &mut self.adding_hot);
        ui.label("Cold:");
        get_mass(ui, &mut self.adding_cold);
        ui.text_edit_singleline(&mut self.adding_name);
        if ui.button("Add ship").clicked() {
            if self.adding_name.is_empty()
                || self.adding_cold == DEFAULT_MASS_TEXT
                || self.adding_hot == DEFAULT_MASS_TEXT
            {
                return;
            }
            self.ships.push(ProvidedShip {
                ship: Ship {
                    hot: entered_tons_to_mass(&self.adding_hot),
                    cold: entered_tons_to_mass(&self.adding_cold),
                },
                name: self.adding_name.take(),
                enabled: true,
                number_available: None,
                max_uses: None,
            });
        }
    }

    fn walkthrough_roll(&mut self, ui: &mut eframe::egui::Ui) {
        let Some(guide) = &self.guide else {
            return;
        };
        for (i, previous_actions) in guide.previous_options.iter().enumerate() {
            ui.label(previous_actions);
            if i != guide.previous_options.len() - 1 {
                ui.label("---------------------");
            }
        }
        ui.label("");

        let mut to_take = None;
        match &guide.next_options {
            CachedNextOptions::Closed => {
                ui.label("Closed!");
            }
            CachedNextOptions::Options(options) => {
                Grid::new("decisions_grid")
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for (i, option) in OPTIONS_ORDER.iter().enumerate() {
                            let text = format!("{:?}", option);
                            if options[i].is_some() {
                                if ui.button(text).clicked() {
                                    to_take = Some(*option);
                                }
                            } else {
                                ui.label(text);
                            }
                        }
                        ui.end_row();
                        for i in 0..OPTIONS_ORDER.len() {
                            if let Some(option_text) = &options[i] {
                                ui.label(option_text);
                            } else {
                                ui.label("");
                            }
                        }
                    });
            }
        }
        if let Some(option) = to_take {
            self.take_decision(option);
        }
    }
}

const OPTIONS_ORDER: [PassDecision; 4] = [
    PassDecision::Closed,
    PassDecision::Crit,
    PassDecision::Shrink,
    PassDecision::Full,
];

enum CachedNextOptions {
    Closed,
    Options([Option<String>; 4]),
}

fn next_options(chart: &RollingChart, path: &[PassDecision]) -> CachedNextOptions {
    let mut walker = chart.chart_walker();
    for option in path {
        walker.take_option(*option);
    }
    match walker.peak_options() {
        PeakedOptions::Closed => return CachedNextOptions::Closed,
        PeakedOptions::Options(options) => {
            let out = array::from_fn(|i| {
                if let Some(actual_path) = options.iter().find(|o| o.decision == OPTIONS_ORDER[i]) {
                    Some(actions_to_text(&actual_path.actions))
                } else {
                    None
                }
            });
            return CachedNextOptions::Options(out);
        }
    }
}

fn value_to_max_or_infinite<T: Copy + Numeric>(
    ui: &mut Ui,
    value: &mut Option<T>,
    range: RangeInclusive<T>,
    default: T,
) {
    let mut available_is_infinite = value.is_none();
    if ui
        .checkbox(&mut available_is_infinite, "Infinite")
        .changed()
    {
        if available_is_infinite {
            *value = None;
        } else {
            *value = Some(default); // Give it a sensible default when unchecked
        }
    }

    // 2. Handle the Number Slider (Disabled if Infinite is checked)
    ui.add_enabled_ui(!available_is_infinite, |ui| {
        // We use a temporary value to bind to the slider if it's currently None
        let mut temp_val = value.unwrap_or(default);

        // Use a Slider (or DragValue)
        if ui.add(Slider::new(&mut temp_val, range)).changed() {
            *value = Some(temp_val);
        }
    });
}

fn get_mass(ui: &mut Ui, value: &mut String) {
    if ui.text_edit_singleline(value).lost_focus() {
        if value.parse::<f64>().is_err() {
            *value = DEFAULT_MASS_TEXT.to_string();
        }
    }
}

fn entered_tons_to_mass(value: &str) -> Mass {
    (value.parse::<f64>().unwrap_or_default() * 1000.0).round() as Mass
}
