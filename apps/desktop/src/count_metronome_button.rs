//! Tauri v2 Count-In & Metronome backend + HTML/CSS/JS frontend bridge.
//!
//! The UI itself is defined in `tauri-ui/index.html` using HTML/CSS/JS and
//! wired to Rust via Tauri commands (`toggle_countin`, `toggle_metronome`,
//! `get_count_metronome_state`).

use eframe::egui::{self, Color32, Response, Sense, Vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Embedded HTML/CSS/JS frontend source for the Count-in & Metronome widget.
#[allow(dead_code)]
pub const COUNT_METRONOME_HTML: &str = include_str!("../tauri-ui/index.html");

/// Thread-safe state shared between Tauri commands and the Esperia desktop app.
#[derive(Clone, Default)]
pub struct CountMetronomeState {
    pub countin_active: Arc<AtomicBool>,
    pub metronome_active: Arc<AtomicBool>,
}

impl CountMetronomeState {
    #[allow(dead_code)]
    pub fn get(&self) -> (bool, bool) {
        (
            self.countin_active.load(Ordering::Acquire),
            self.metronome_active.load(Ordering::Acquire),
        )
    }

    pub fn toggle_countin(&self) -> bool {
        let prev = self.countin_active.fetch_xor(true, Ordering::AcqRel);
        !prev
    }

    pub fn toggle_metronome(&self) -> bool {
        let prev = self.metronome_active.fetch_xor(true, Ordering::AcqRel);
        !prev
    }
}

#[allow(dead_code)]
#[tauri::command]
pub fn get_count_metronome_state(state: tauri::State<'_, CountMetronomeState>) -> (bool, bool) {
    state.get()
}

#[allow(dead_code)]
#[tauri::command]
pub fn toggle_countin(state: tauri::State<'_, CountMetronomeState>) -> bool {
    state.toggle_countin()
}

#[allow(dead_code)]
#[tauri::command]
pub fn toggle_metronome(state: tauri::State<'_, CountMetronomeState>) -> bool {
    state.toggle_metronome()
}

/// Build the Tauri v2 application configured with `tauri-ui/index.html` and
/// the Count-in / Metronome IPC handlers.
#[allow(dead_code)]
pub fn build_tauri_app(state: CountMetronomeState) -> tauri::Result<tauri::App> {
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_count_metronome_state,
            toggle_countin,
            toggle_metronome,
        ])
        .build(tauri::generate_context!())
}

/// Host handle used by `EsperiaApp` in `main.rs`.
#[derive(Default)]
pub struct CountMetronomeButton {
    pub countin_active: bool,
    pub metronome_active: bool,
    state: CountMetronomeState,
}

impl CountMetronomeButton {
    /// Access the shared Tauri-compatible atomic state.
    #[allow(dead_code)]
    pub fn shared_state(&self) -> CountMetronomeState {
        self.state.clone()
    }

    /// Toolbar control slot that stays synced with `CountMetronomeState`.
    pub fn ui(&mut self, ui: &mut egui::Ui, _background: Color32) -> Response {
        self.countin_active = self.state.countin_active.load(Ordering::Acquire);
        self.metronome_active = self.state.metronome_active.load(Ordering::Acquire);

        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(94.0, 40.0), Sense::hover());
        let left = egui::Rect::from_min_size(rect.min, Vec2::new(46.0, 40.0));
        let right = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 48.0, rect.min.y),
            Vec2::new(46.0, 40.0),
        );

        let r1 = ui.put(
            left,
            egui::Button::new("1234")
                .fill(if self.countin_active {
                    Color32::from_rgb(127, 44, 170)
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(14.0),
        );
        if r1.clicked() {
            self.countin_active = self.state.toggle_countin();
            response.mark_changed();
        }

        let r2 = ui.put(
            right,
            egui::Button::new("△")
                .fill(if self.metronome_active {
                    Color32::from_rgb(127, 44, 170)
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(14.0),
        );
        if r2.clicked() {
            self.metronome_active = self.state.toggle_metronome();
            response.mark_changed();
        }

        response
    }
}
