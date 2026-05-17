use std::{array, fmt::Display, io::Write, mem};

use base64::engine::general_purpose::URL_SAFE;
use eframe::egui::{
    CentralPanel, Color32, ComboBox, DragValue, Grid, Layout, Panel, ScrollArea, Spinner, TextEdit,
};
use flate2::{Compression, write::ZlibEncoder};
use serde::Serialize;

use crate::{
    best_path_picker::{Priorities, Qualities, Quality},
    chart_gen::{PassDecision, PeakedOptions, RollingChart, actions_to_text},
    hole_info::{HoleInfo, Mass},
    roll_calc::{
        AvailabileShips, HoleState, PolorizationGuide, RollState, RollersUsed, Ship,
        get_best_roll_chart,
    },
};

pub fn run_app() {
    eframe::run_native(
        "Did You Go Hot?",
        Default::default(),
        Box::new(|cc| Ok(Box::new(RollApp::new(cc)))),
    )
    .expect("Failed to run UI");
}

impl Display for HoleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoleState::Full => write!(f, "Full"),
            HoleState::Shrink => write!(f, "Shrink"),
            HoleState::Crit => write!(f, "Crit"),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone, PartialEq, Eq, Copy)]
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

impl Default for PolorizationGuide {
    fn default() -> Self {
        PolorizationGuide::FirstPossiblePlusN(1)
    }
}

impl Display for Quality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Quality::MaxOut => write!(f, "Max Out"),
            Quality::ROProbability => write!(f, "RO Probability"),
            Quality::AvgNumPasses => write!(f, "Avg Num Passes"),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
struct ProvidedShip {
    ship: Ship,
    name: String,
    enabled: bool,
    number_available: u8,
    #[serde(skip)]
    already_outside: u8,
}

struct ChartGuide {
    chart: RollingChart,
    next_options: CachedNextOptions,
    previous_options: Vec<String>,
    path: Vec<PassDecision>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct RollApp {
    #[serde(skip)]
    guide: Option<ChartGuide>,
    #[serde(skip)]
    calculated_plans: Option<Vec<(RollingChart, Qualities)>>,

    selected_hole: Option<HollSizes>,
    selected_state: HoleState,
    polarization_guide: PolorizationGuide,
    quality_priority: [Quality; 3],
    ships: Vec<ProvidedShip>,

    #[serde(skip)]
    advanced_min_mass: String,
    #[serde(skip)]
    advanced_max_mass: String,

    #[serde(skip)]
    adding_hot: String,
    #[serde(skip)]
    adding_cold: String,
    #[serde(skip)]
    adding_name: String,

    #[serde(skip)]
    error: String,

    // Thread handle returning the calculation result.
    #[serde(skip)]
    calculation_handle: Option<std::thread::JoinHandle<Vec<Option<(RollingChart, Qualities)>>>>,
}

impl Default for RollApp {
    fn default() -> Self {
        let out = Self {
            guide: None,
            calculated_plans: None,
            selected_hole: None,
            selected_state: HoleState::default(),
            polarization_guide: PolorizationGuide::default(),
            quality_priority: [
                Quality::ROProbability,
                Quality::AvgNumPasses,
                Quality::MaxOut,
            ],
            ships: Vec::new(),
            advanced_min_mass: String::new(),
            advanced_max_mass: String::new(),
            adding_hot: String::new(),
            adding_cold: String::new(),
            adding_name: String::new(),
            error: String::new(),
            calculation_handle: None,
        };

        out
    }
}

impl eframe::App for RollApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(eframe::APP_KEY, serde_json::to_string(self).unwrap());
    }

    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        Panel::left("Selection_panel")
            .resizable(true)
            .min_size(250.0)
            .show_inside(ui, |ui| {
                self.show_selection_panel(ui);
            });
        CentralPanel::default_margins().show_inside(ui, |ui| {
            self.walkthrough_roll(ui);
        });
    }
}

impl RollApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load persistent state if available
        if let Some(storage) = cc.storage {
            if let Some(json) = storage.get_string(eframe::APP_KEY) {
                return serde_json::from_str(&json).unwrap_or_default();
            }
        }
        Default::default()
    }

    fn calculate(&mut self, ctx: eframe::egui::Context) {
        self.error.clear();
        self.calculated_plans = None;
        self.guide = None;

        let Some(hole_size) = self.selected_hole else {
            self.error = "Hole size must be selected".to_string();
            return;
        };

        // Parse advanced mass limitations
        let parsed_min_mass_passed = if self.advanced_min_mass.is_empty() {
            None
        } else {
            match entered_tons_to_mass(&self.advanced_min_mass) {
                Ok(mass) => Some(mass),
                Err(_) => {
                    self.error = "Advanced Min Mass Passed must be a valid number.".to_string();
                    return;
                }
            }
        };

        let parsed_max_mass_passed = if self.advanced_max_mass.is_empty() {
            None
        } else {
            match entered_tons_to_mass(&self.advanced_max_mass) {
                Ok(mass) => Some(mass),
                Err(_) => {
                    self.error = "Advanced Max Mass Passed must be a valid number.".to_string();
                    return;
                }
            }
        };

        let hole = hole_size.hole_info();
        let mut mass_range = hole.mass_range(self.selected_state);
        // 1. Cross-validate min and max if both are present
        if let (Some(min), Some(max)) = (parsed_min_mass_passed, parsed_max_mass_passed) {
            if min > max {
                self.error =
                    "Minimum mass passed cannot be greater than maximum mass passed.".to_string();
                return;
            }
        }

        // 2. Safely apply the minimum passed mass constraint
        if let Some(min_passed_mass) = parsed_min_mass_passed {
            if min_passed_mass > hole.max_range.most {
                self.error =
                    "Minimum mass passed exceeds the maximum possible total mass of this hole."
                        .to_string();
                return;
            }
            let highest_mass = hole.max_range.most - min_passed_mass;
            mass_range.most = mass_range.most.min(highest_mass);
        }

        // 3. Safely apply the maximum passed mass constraint using saturating_sub
        if let Some(max_passed_mass) = parsed_max_mass_passed {
            // If max_passed_mass > least capacity, lowest remaining mass constraint from this input is 0
            let lowest_mass = hole.max_range.least.saturating_sub(max_passed_mass);
            mass_range.least = mass_range.least.max(lowest_mass);
        }

        // 4. Check if the constraints conflict with the current visual state of the hole
        if mass_range.is_empty() {
            self.error =
                "The entered mass passed is not possible for this wormhole's current visual state."
                    .to_string();
        }

        let mut rollers_out = RollersUsed::new();
        let mut available_rollers = vec![];
        for ship in &self.ships {
            if ship.enabled {
                for _ in 0..ship.already_outside {
                    rollers_out.add(available_rollers.len());
                }
                available_rollers.push(AvailabileShips {
                    ship: ship.ship,
                    max_num_out: ship.number_available,
                    max_used: ship.number_available,
                });
            }
        }
        let state = RollState {
            remaining_mass: mass_range,
            rollers_out,
            max_size_range: hole.max_range,
            highest_hole_state: self.selected_state,
            used_ships: RollersUsed::new(),
        };
        let starting_state = self.selected_state;
        let priorities = Priorities::new(self.quality_priority.to_vec()).unwrap();
        let polo_guide = self.polarization_guide;
        let max_memory = None;
        let start_polo_num = 0;

        self.calculation_handle = Some(std::thread::spawn(move || {
            let output = get_best_roll_chart(
                &available_rollers,
                state,
                starting_state,
                &priorities,
                max_memory,
                polo_guide,
                start_polo_num,
            );
            for plan in &output {
                if let Some((_chart, qualities)) = plan {
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
            ctx.request_repaint();
            output
        }));
    }

    fn add_chart_guide(&mut self, chart: RollingChart) {
        let (first_move_decision, first_actions) = match chart.chart_walker().peak_options() {
            PeakedOptions::Closed => unreachable!(),
            PeakedOptions::Options(options) => {
                assert!(options.len() == 1);
                (options[0].decision, actions_to_text(&options[0].actions))
            }
        };

        self.guide = Some(ChartGuide {
            chart,
            next_options: CachedNextOptions::Closed, // Temporary next_options which will be overwritten before being shown
            previous_options: vec![first_actions],
            path: vec![],
        });

        self.take_decision(first_move_decision);
    }

    fn take_decision(&mut self, decision: PassDecision) {
        let guide = self.guide.as_mut().unwrap();
        guide.path.push(decision);
        let previous_next_options = mem::replace(
            &mut guide.next_options,
            next_options(&guide.chart, &guide.path),
        );
        let taken_actions = match previous_next_options {
            CachedNextOptions::Closed => return,
            CachedNextOptions::Options(options) => {
                let i = OPTIONS_ORDER.iter().position(|x| *x == decision).unwrap();
                options.into_iter().skip(i).next().unwrap().unwrap()
            }
        };
        guide.previous_options.push(taken_actions);
    }

    fn show_selection_panel(&mut self, ui: &mut eframe::egui::Ui) {
        // Allocate space from bottom up for the error text first
        ui.with_layout(Layout::bottom_up(eframe::egui::Align::LEFT), |ui| {
            // Display Error at the absolute bottom if it exists
            if !self.error.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(Color32::RED, &self.error);
                ui.separator();
            }

            // Fill the rest of the vertical space from the top down and wrap in a ScrollArea
            ui.with_layout(Layout::top_down(eframe::egui::Align::LEFT), |ui| {
                ScrollArea::vertical()
                    .id_salt("selection_panel_scroll")
                    .show(ui, |ui| {
                        // Top Action Buttons
                        if ui.button("Calculate").clicked() {
                            self.calculate(ui.ctx().clone());
                        }

                        ui.separator();

                        // Hole Properties
                        ui.heading("Hole Status");
                        Grid::new("hole_status_grid")
                            .num_columns(2)
                            .spacing([20.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Size:");
                                ComboBox::from_id_salt("hole_size_combo")
                                    .selected_text(if let Some(hole) = &self.selected_hole {
                                        format!("{hole}")
                                    } else {
                                        "Select...".to_string()
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
                                            ui.selectable_value(
                                                &mut self.selected_hole,
                                                Some(hole),
                                                format!("{hole}"),
                                            );
                                        }
                                    });
                                ui.end_row();

                                ui.label("State:");
                                ComboBox::from_id_salt("hole_state_combo")
                                    .selected_text(format!("{}", self.selected_state))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.selected_state,
                                            HoleState::Full,
                                            "Full",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_state,
                                            HoleState::Shrink,
                                            "Shrink",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_state,
                                            HoleState::Crit,
                                            "Crit",
                                        );
                                    });
                                ui.end_row();
                            });

                        ui.separator();

                        const UP_TO_TEXT: &str = "Up to N polorizations";
                        const FIRST_POSSIBLE_TEXT: &str = "First possible + N";
                        // Polarization Strategy
                        ui.heading("Polorization Strategy");
                        Grid::new("polarization_grid")
                            .num_columns(2)
                            .spacing([20.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Limit Type:");
                                ComboBox::from_id_salt("polarization_combo")
                                    .selected_text(match self.polarization_guide {
                                        PolorizationGuide::UpTo(_) => UP_TO_TEXT,
                                        PolorizationGuide::FirstPossiblePlusN(_) => {
                                            FIRST_POSSIBLE_TEXT
                                        }
                                    })
                                    .show_ui(ui, |ui| {
                                        let is_upto = matches!(
                                            self.polarization_guide,
                                            PolorizationGuide::UpTo(_)
                                        );
                                        if ui.selectable_label(is_upto, UP_TO_TEXT).clicked()
                                            && !is_upto
                                        {
                                            self.polarization_guide = PolorizationGuide::UpTo(3);
                                        }

                                        let is_first = matches!(
                                            self.polarization_guide,
                                            PolorizationGuide::FirstPossiblePlusN(_)
                                        );
                                        if ui
                                            .selectable_label(is_first, FIRST_POSSIBLE_TEXT)
                                            .clicked()
                                            && !is_first
                                        {
                                            self.polarization_guide =
                                                PolorizationGuide::FirstPossiblePlusN(1);
                                        }
                                    });
                                ui.end_row();

                                ui.label(match self.polarization_guide {
                                    PolorizationGuide::UpTo(_) => "Max Allowed:",
                                    PolorizationGuide::FirstPossiblePlusN(_) => {
                                        "Extra After Found:"
                                    }
                                });
                                match &mut self.polarization_guide {
                                    PolorizationGuide::UpTo(val)
                                    | PolorizationGuide::FirstPossiblePlusN(val) => {
                                        ui.add(DragValue::new(val).range(0..=u8::MAX));
                                    }
                                }
                                ui.end_row();
                            });

                        ui.separator();

                        // Quality Priority Form
                        ui.heading("Quality Priority");
                        ui.label(
                            eframe::egui::RichText::new("Order from highest (1) to lowest (3):")
                                .weak(),
                        );

                        let mut swap_indices = None;
                        for i in 0..3 {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(i > 0, eframe::egui::Button::new("⬆"))
                                    .clicked()
                                {
                                    swap_indices = Some((i, i - 1));
                                }
                                if ui
                                    .add_enabled(i < 2, eframe::egui::Button::new("⬇"))
                                    .clicked()
                                {
                                    swap_indices = Some((i, i + 1));
                                }
                                ui.label(format!("{}. {}", i + 1, self.quality_priority[i]));
                            });
                        }

                        if let Some((a, b)) = swap_indices {
                            self.quality_priority.swap(a, b);
                        }

                        ui.separator();

                        // Add Ship Form
                        ui.heading("Add Rolling Ship");
                        Grid::new("add_ship_grid")
                            .num_columns(2)
                            .spacing([20.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Name:");
                                ui.add(
                                    TextEdit::singleline(&mut self.adding_name)
                                        .hint_text("e.g. Typhoon"),
                                );
                                ui.end_row();

                                ui.label("Hot (tons):");
                                ui.add(
                                    TextEdit::singleline(&mut self.adding_hot)
                                        .hint_text("e.g. 100000"),
                                );
                                ui.end_row();

                                ui.label("Cold (tons):");
                                ui.add(
                                    TextEdit::singleline(&mut self.adding_cold)
                                        .hint_text("e.g. 100000"),
                                );
                                ui.end_row();
                            });

                        ui.add_space(4.0);
                        if ui.button("➕ Add Ship").clicked() {
                            if self.adding_name.is_empty() {
                                self.error = "Ship name cannot be empty.".to_string();
                            } else {
                                let hot_res = entered_tons_to_mass(&self.adding_hot);
                                let cold_res = entered_tons_to_mass(&self.adding_cold);

                                match (hot_res, cold_res) {
                                    (Ok(hot), Ok(cold)) if hot > 0 && cold > 0 => {
                                        self.error.clear();
                                        self.ships.push(ProvidedShip {
                                            ship: Ship { hot, cold },
                                            name: self.adding_name.clone(),
                                            enabled: true,
                                            number_available: 1, // Start with 1 available
                                            already_outside: 0,
                                        });

                                        // Clear inputs on success
                                        self.adding_name.clear();
                                        self.adding_hot.clear();
                                        self.adding_cold.clear();
                                    }
                                    _ => {
                                        self.error =
                                            "Mass inputs must be valid numbers greater than 0."
                                                .to_string();
                                    }
                                }
                            }
                        }

                        ui.separator();

                        // Fleet Management
                        ui.heading("Fleet Configuration");
                        if self.ships.is_empty() {
                            ui.label(
                                eframe::egui::RichText::new("No ships added.")
                                    .weak()
                                    .italics(),
                            );
                        } else {
                            Grid::new("ships_grid")
                                .num_columns(4)
                                .spacing([15.0, 8.0])
                                .show(ui, |ui| {
                                    let mut to_remove = None;
                                    for (i, ship) in self.ships.iter_mut().enumerate() {
                                        ui.checkbox(&mut ship.enabled, &ship.name);
                                        ui.label("Qty:");
                                        ui.add(
                                            DragValue::new(&mut ship.number_available)
                                                .range(0..=u8::MAX)
                                                .speed(0.1),
                                        );
                                        if ui.button("❌").on_hover_text("Delete Ship").clicked() {
                                            to_remove = Some(i);
                                        }
                                        ui.end_row();
                                    }

                                    if let Some(idx) = to_remove {
                                        self.ships.remove(idx);
                                    }
                                });
                        }

                        ui.separator();

                        // Advanced Controls
                        ui.collapsing("Advanced", |ui| {
                            Grid::new("advanced_mass_grid")
                                .num_columns(2)
                                .spacing([20.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("Min Mass Passed (tons):");
                                    ui.add(
                                        TextEdit::singleline(&mut self.advanced_min_mass)
                                            .hint_text("Unknown"),
                                    );
                                    ui.end_row();

                                    ui.label("Max Mass Passed (tons):");
                                    ui.add(
                                        TextEdit::singleline(&mut self.advanced_max_mass)
                                            .hint_text("Unknown"),
                                    );
                                    ui.end_row();
                                });

                            ui.add_space(8.0);
                            ui.label("Ships Already Outside:");

                            if self.ships.is_empty() {
                                ui.label(
                                    eframe::egui::RichText::new("No ships added.")
                                        .weak()
                                        .italics(),
                                );
                            } else {
                                Grid::new("advanced_ships_grid")
                                    .num_columns(2)
                                    .spacing([15.0, 8.0])
                                    .show(ui, |ui| {
                                        for ship in self.ships.iter_mut() {
                                            if ship.enabled {
                                                ui.label(&ship.name);
                                                ui.add(
                                                    DragValue::new(&mut ship.already_outside)
                                                        .range(0..=ship.number_available)
                                                        .speed(0.1),
                                                );
                                                ui.end_row();
                                            }
                                        }
                                    });
                            }
                        });
                    });
            });
        });
    }

    fn walkthrough_roll(&mut self, ui: &mut eframe::egui::Ui) {
        // 1. Check if we are currently waiting on a calculation thread
        if let Some(is_finished) = self.calculation_handle.as_ref().map(|h| h.is_finished()) {
            if is_finished {
                let handle = self.calculation_handle.take().unwrap();
                match handle.join() {
                    Ok(result) => {
                        self.calculated_plans = Some(filter_plans(result));
                    }
                    Err(_) => {
                        self.error =
                            "Calculation thread panicked or failed unexpectedly.".to_string();
                    }
                }
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 2.0 - 50.0);
                    ui.heading("Calculating...");
                    ui.add_space(10.0);
                    ui.add(Spinner::new().size(40.0));
                });
                return;
            }
        }

        // 2. Display the plan selection if plans are calculated and guide is not set yet
        if self.calculated_plans.is_some() && self.guide.is_none() {
            let mut chosen_index = None;

            ui.heading("Select a Rolling Plan");
            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                if let Some(plans) = &self.calculated_plans {
                    if plans.is_empty() {
                        ui.label("No viable plans found for this configuration.");
                    } else {
                        for (i, (_, q)) in plans.iter().enumerate() {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.heading(format!("Plan {}", i + 1));
                                    ui.with_layout(
                                        Layout::right_to_left(eframe::egui::Align::Center),
                                        |ui| {
                                            if ui.button("Select Plan").clicked() {
                                                chosen_index = Some(i);
                                            }
                                        },
                                    );
                                });
                                ui.add_space(4.0);

                                Grid::new(format!("plan_grid_{}", i))
                                    .num_columns(2)
                                    .spacing([40.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Roll Out Probability:");
                                        ui.label(format_probability(q.roll_out_probability));
                                        ui.end_row();

                                        ui.label("Max Ships Out:");
                                        ui.label(format!("{}", q.max_num_out));
                                        ui.end_row();

                                        ui.label("Average Passes:");
                                        ui.label(format!("{:.2}", q.average_num_passes));
                                        ui.end_row();

                                        ui.label("Polarizations Required:");
                                        ui.label(format!("{}", q.num_polorizations));
                                        ui.end_row();
                                    });
                            });
                            ui.add_space(8.0);
                        }
                    }
                }
            });

            if let Some(idx) = chosen_index {
                let mut plans = self.calculated_plans.take().unwrap();
                let (chart, _) = plans.remove(idx);
                self.add_chart_guide(chart);
            }

            return;
        }

        if self.guide.is_some() {
            ui.horizontal(|ui| {
                if ui.button("Reset Roll Guide").clicked() {
                    self.error.clear();
                    self.calculated_plans = None;
                    if let Some(old_guide) = self.guide.take() {
                        self.add_chart_guide(old_guide.chart);
                    }
                }
                if ui.button("Open Visual Flowchart in the Browser").clicked() {
                    self.open_flowchart();
                }
            });
        }
        // 3. Existing standard logic
        let Some(guide) = &self.guide else {
            if self.calculated_plans.is_none() && self.calculation_handle.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        eframe::egui::RichText::new("Enter details and click Calculate to begin.")
                            .weak(),
                    );
                });
            }
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

    fn open_flowchart(&mut self) {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct MermaidState {
            code: String,
            mermaid: MermaidConfig,
            auto_sync: bool,
            update_editor: bool,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct MermaidConfig {
            theme: String,
        }

        let code = self.guide.as_ref().unwrap().chart.to_text_chart();
        let state = MermaidState {
            code: code.to_string(),
            mermaid: MermaidConfig {
                theme: "default".to_string(),
            },
            auto_sync: true,
            update_editor: true,
        };

        let json_str = serde_json::to_string(&state).unwrap();

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(json_str.as_bytes()).unwrap();
        let compressed_bytes = encoder.finish().unwrap();

        // 4. Encode to URL-safe Base64
        let base64_str = base64::Engine::encode(&URL_SAFE, compressed_bytes);

        let url = format!("https://mermaid.live/edit#pako:{}", base64_str);
        if webbrowser::open(&url).is_err() {
            self.error = format!("Failed to open browser with url");
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
        walker.take_option(*option).unwrap();
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

fn entered_tons_to_mass(value: &str) -> Result<Mass, ()> {
    value
        .parse::<f64>()
        .map(|v| (v * 1000.0).round() as Mass)
        .map_err(|_| ())
}

fn format_probability(prob: f64) -> String {
    if prob == 0.0 {
        "0.000%".to_string()
    } else if prob > 0.0 && (prob * 100.0) < 0.001 {
        "< 0.001%".to_string()
    } else {
        format!("{:.3}%", prob * 100.0)
    }
}

fn filter_plans(
    raw_plans: Vec<Option<(RollingChart, Qualities)>>,
) -> Vec<(RollingChart, Qualities)> {
    // 1. Filter out all None results
    let mut valid: Vec<(RollingChart, Qualities)> = raw_plans.into_iter().flatten().collect();

    // 2. Identify redundant plans where qualities are equal except for higher num_polorizations
    let mut to_remove = Vec::new();
    let eps = 1e-9;

    for i in 0..valid.len() {
        for j in 0..valid.len() {
            if i == j {
                continue;
            }
            let qi = &valid[i].1;
            let qj = &valid[j].1;

            let same_core = qi.max_num_out == qj.max_num_out
                && (qi.roll_out_probability - qj.roll_out_probability).abs() < eps
                && (qi.average_num_passes - qj.average_num_passes).abs() < eps;

            if same_core {
                // If plan `i` requires strictly more polarizations than plan `j` with the same stats, flag it
                if qi.num_polorizations > qj.num_polorizations {
                    to_remove.push(i);
                }
            }
        }
    }

    // 3. Apply the removal flags
    to_remove.sort_unstable();
    to_remove.dedup();
    for &idx in to_remove.iter().rev() {
        valid.remove(idx);
    }

    valid
}
