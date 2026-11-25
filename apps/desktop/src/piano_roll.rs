//! Logic Pro-inspired Piano Roll & Velocity Lane editor for MIDI clips.
//!
//! Features:
//! - Left Region/Notes inspector sidebar (Quantize, Strength, Transpose,
//!   Pattern length/extend/duplicate/clear, selected-note inspector)
//! - Absolute project-tick bar/beat ruler synced with the main timeline
//! - Green region header bar showing the clip's span on the musical timeline
//! - Synchronized playhead line + pentagon head; scrubbing the Piano Roll ruler
//!   seeks the main transport playhead
//! - Interactive vertical & horizontal scrollbars + mouse/trackpad scrolling
//! - Separate Velocity Lane below the note grid

use eframe::egui;
use esperia_audio_engine::EngineHandle;
use esperia_midi::{MidiClip, MidiMessage, PPQN, QuantizeGrid, quantize_notes, quantize_tick};

const INSPECTOR_WIDTH: f32 = 210.0;
const KEYBOARD_WIDTH: f32 = 46.0;
const RULER_HEIGHT: f32 = 22.0;
const REGION_BAR_HEIGHT: f32 = 16.0;
const SCROLLBAR_SIZE: f32 = 12.0;
const VELOCITY_LANE_HEIGHT: f32 = 58.0;
const MIN_PITCH: u8 = 24; // C1
const MAX_PITCH: u8 = 108; // C8

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Region,
    Notes,
}

#[derive(Clone, Copy, PartialEq)]
enum NoteDragKind {
    Move {
        grab_offset_ticks: i64,
    },
    Resize {
        orig_duration_ticks: u64,
        start_x: f32,
    },
}

struct ActiveNoteDrag {
    note_id: u64,
    kind: NoteDragKind,
}

/// Actions requested from the Piano Roll UI that affect the parent track/project.
#[derive(Default)]
pub struct PianoRollOutcome {
    pub clip_modified: bool,
    pub seek_tick: Option<u64>,
    pub create_new_pattern: bool,
    pub duplicate_pattern: bool,
}

/// Persistent UI state for the Piano Roll editor.
pub struct PianoRollState {
    pub tab: InspectorTab,
    pub px_per_tick: f32,
    pub row_height: f32,
    pub quantize_grid: QuantizeGrid,
    pub quantize_strength: f32,
    pub transpose_semitones: i32,
    pub last_applied_transpose: i32,
    pub snap_enabled: bool,
    pub show_velocity_lane: bool,
    pub selected_note_id: Option<u64>,
    drag: Option<ActiveNoteDrag>,
    previewing_note: Option<(u8, u8)>,
    scrubbing_ruler: bool,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            tab: InspectorTab::Region,
            px_per_tick: 0.06,
            row_height: 15.0,
            quantize_grid: QuantizeGrid::Sixteenth,
            quantize_strength: 100.0,
            transpose_semitones: 0,
            last_applied_transpose: 0,
            snap_enabled: true,
            show_velocity_lane: true,
            selected_note_id: None,
            drag: None,
            previewing_note: None,
            scrubbing_ruler: false,
            scroll_offset_x: 0.0,
            // Start scrolled near middle C (C4 = 60)
            scroll_offset_y: ((MAX_PITCH - 72) as f32) * 15.0,
        }
    }
}

impl PianoRollState {
    /// Scroll the Piano Roll horizontally so `tick` is visible near the left edge.
    pub fn scroll_to_tick(&mut self, tick: u64) {
        self.scroll_offset_x = (tick as f32 * self.px_per_tick - 24.0).max(0.0);
        self.transpose_semitones = 0;
        self.last_applied_transpose = 0;
        self.selected_note_id = None;
    }

    /// Draw the Logic Pro-style Piano Roll for `clip`.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        clip: &mut MidiClip,
        track_name: &str,
        track_channel: u8,
        playhead_tick: u64,
        engine_handle: &EngineHandle,
    ) -> PianoRollOutcome {
        let mut outcome = PianoRollOutcome::default();

        // Handle Delete/Backspace keyboard shortcut
        if let Some(sel_id) = self.selected_note_id
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && clip.delete_note(sel_id)
        {
            self.selected_note_id = None;
            outcome.clip_modified = true;
        }

        let available = ui.available_size();
        let vel_h = if self.show_velocity_lane {
            VELOCITY_LANE_HEIGHT + 6.0
        } else {
            0.0
        };
        let total_height = available.y.max(140.0);

        ui.horizontal_top(|ui| {
            // ── Left Inspector Sidebar (Region / Notes) ──────────────────────
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(44, 44, 46))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(INSPECTOR_WIDTH - 16.0);
                    ui.set_min_height((total_height - 8.0).max(120.0));

                    // Region header summary
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(65, 200, 95), "●");
                        ui.vertical(|ui| {
                            ui.strong(&clip.name);
                            ui.weak(format!("on {track_name}"));
                        });
                    });
                    ui.add_space(6.0);

                    // Region / Notes segmented tabs
                    ui.horizontal(|ui| {
                        let reg_sel = self.tab == InspectorTab::Region;
                        if ui
                            .add_sized(
                                [86.0, 22.0],
                                egui::Button::new("Region").fill(if reg_sel {
                                    egui::Color32::from_rgb(30, 110, 210)
                                } else {
                                    egui::Color32::from_gray(58)
                                }),
                            )
                            .clicked()
                        {
                            self.tab = InspectorTab::Region;
                        }
                        let notes_sel = self.tab == InspectorTab::Notes;
                        if ui
                            .add_sized(
                                [86.0, 22.0],
                                egui::Button::new("Notes").fill(if notes_sel {
                                    egui::Color32::from_rgb(30, 110, 210)
                                } else {
                                    egui::Color32::from_gray(58)
                                }),
                            )
                            .clicked()
                        {
                            self.tab = InspectorTab::Notes;
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();

                    match self.tab {
                        InspectorTab::Region => {
                            ui.label("Pattern Actions");
                            ui.horizontal(|ui| {
                                if ui.button("+ New").clicked() {
                                    outcome.create_new_pattern = true;
                                }
                                if ui.button("Duplicate").clicked() {
                                    outcome.duplicate_pattern = true;
                                }
                                if ui.button("Clear").clicked() && !clip.notes.is_empty() {
                                    clip.notes.clear();
                                    self.selected_note_id = None;
                                    outcome.clip_modified = true;
                                }
                            });

                            ui.add_space(4.0);
                            let bars = (clip.length_ticks as f32 / (PPQN * 4) as f32).max(0.25);
                            ui.label(format!("Length: {bars:.1} bars"));
                            ui.horizontal(|ui| {
                                if ui.button("−1 Bar").clicked() && clip.length_ticks > PPQN * 4 {
                                    clip.length_ticks -= PPQN * 4;
                                    outcome.clip_modified = true;
                                }
                                if ui.button("+1 Bar").clicked() {
                                    clip.length_ticks = clip.length_ticks.saturating_add(PPQN * 4);
                                    outcome.clip_modified = true;
                                }
                                if ui.button("+4 Bars").clicked() {
                                    clip.length_ticks = clip.length_ticks.saturating_add(PPQN * 16);
                                    outcome.clip_modified = true;
                                }
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.label("Time Quantize");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.snap_enabled, "Snap");
                                egui::ComboBox::from_id_salt("pr_grid")
                                    .selected_text(self.quantize_grid.label())
                                    .width(72.0)
                                    .show_ui(ui, |ui| {
                                        for g in QuantizeGrid::ALL {
                                            ui.selectable_value(
                                                &mut self.quantize_grid,
                                                g,
                                                g.label(),
                                            );
                                        }
                                    });
                            });

                            ui.horizontal(|ui| {
                                ui.label("Strength");
                                ui.add(
                                    egui::Slider::new(&mut self.quantize_strength, 0.0..=100.0)
                                        .suffix("%"),
                                );
                            });

                            let strength = (self.quantize_strength / 100.0).clamp(0.0, 1.0);
                            if ui
                                .add_sized([180.0, 22.0], egui::Button::new("Apply Quantize"))
                                .clicked()
                            {
                                quantize_notes(
                                    &mut clip.notes,
                                    self.quantize_grid,
                                    strength,
                                    false,
                                );
                                outcome.clip_modified = true;
                            }

                            ui.add_space(6.0);
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label("Transpose");
                                if ui
                                    .add(egui::Slider::new(&mut self.transpose_semitones, -24..=24))
                                    .changed()
                                {
                                    let delta =
                                        self.transpose_semitones - self.last_applied_transpose;
                                    if delta != 0 {
                                        for n in &mut clip.notes {
                                            let p = (i32::from(n.pitch) + delta).clamp(0, 127);
                                            n.pitch = p as u8;
                                        }
                                        self.last_applied_transpose = self.transpose_semitones;
                                        outcome.clip_modified = true;
                                    }
                                }
                            });
                        }
                        InspectorTab::Notes => {
                            if let Some(sel_id) = self.selected_note_id
                                && let Some(note) = clip.notes.iter_mut().find(|n| n.id == sel_id)
                            {
                                ui.strong(format!("Note {}", pitch_name(note.pitch)));
                                ui.label(format!(
                                    "Pos: {:.2} beats",
                                    (clip.start_tick + note.start_tick) as f32 / PPQN as f32
                                ));
                                let mut vel = note.velocity as i32;
                                if ui
                                    .add(egui::Slider::new(&mut vel, 1..=127).text("Velocity"))
                                    .changed()
                                {
                                    note.velocity = vel.clamp(1, 127) as u8;
                                    outcome.clip_modified = true;
                                }
                                let mut dur_beats = note.duration_ticks as f32 / PPQN as f32;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut dur_beats)
                                            .range(0.0625..=16.0)
                                            .speed(0.05)
                                            .prefix("Dur: ")
                                            .suffix(" beats"),
                                    )
                                    .changed()
                                {
                                    note.duration_ticks =
                                        ((dur_beats * PPQN as f32).round() as u64).max(30);
                                    outcome.clip_modified = true;
                                }
                                ui.add_space(4.0);
                                if ui.button("Quantize Selected").clicked() {
                                    let strength = (self.quantize_strength / 100.0).clamp(0.0, 1.0);
                                    note.start_tick = quantize_tick(
                                        note.start_tick,
                                        self.quantize_grid,
                                        strength,
                                    );
                                    outcome.clip_modified = true;
                                }
                                if ui.button("🗑 Delete Note").clicked() {
                                    clip.delete_note(sel_id);
                                    self.selected_note_id = None;
                                    outcome.clip_modified = true;
                                }
                            } else {
                                ui.weak("Click a note in the grid to inspect or edit it.");
                            }
                        }
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.checkbox(&mut self.show_velocity_lane, "Velocity Lane");
                    ui.horizontal(|ui| {
                        ui.label("H Zoom");
                        ui.add(
                            egui::Slider::new(&mut self.px_per_tick, 0.015..=0.25)
                                .show_value(false),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("V Zoom");
                        ui.add(
                            egui::Slider::new(&mut self.row_height, 9.0..=26.0).show_value(false),
                        );
                    });
                });

            // ── Right Piano Roll Grid + Synced Ruler + Velocity Lane ─────────
            ui.vertical(|ui| {
                let right_avail = ui.available_size();
                let grid_total_height = (total_height - vel_h - 6.0).max(110.0);
                let grid_body_height =
                    (grid_total_height - RULER_HEIGHT - REGION_BAR_HEIGHT - SCROLLBAR_SIZE)
                        .max(50.0);

                let num_pitches = (MAX_PITCH - MIN_PITCH + 1) as usize;
                let total_content_height = num_pitches as f32 * self.row_height;
                let clip_end_tick = clip.start_tick.saturating_add(clip.length_ticks);
                let total_ticks = clip_end_tick.max(PPQN * 64);
                let total_content_width =
                    (total_ticks as f32 * self.px_per_tick).max(right_avail.x);

                let (outer_rect, _) = ui.allocate_exact_size(
                    egui::vec2(right_avail.x.max(200.0), grid_total_height),
                    egui::Sense::hover(),
                );

                let canvas_width = (outer_rect.width() - KEYBOARD_WIDTH - SCROLLBAR_SIZE).max(80.0);

                let ruler_rect = egui::Rect::from_min_size(
                    egui::pos2(outer_rect.left() + KEYBOARD_WIDTH, outer_rect.top()),
                    egui::vec2(canvas_width, RULER_HEIGHT),
                );
                let region_bar_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        outer_rect.left() + KEYBOARD_WIDTH,
                        outer_rect.top() + RULER_HEIGHT,
                    ),
                    egui::vec2(canvas_width, REGION_BAR_HEIGHT),
                );
                let key_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        outer_rect.left(),
                        outer_rect.top() + RULER_HEIGHT + REGION_BAR_HEIGHT,
                    ),
                    egui::vec2(KEYBOARD_WIDTH, grid_body_height),
                );
                let canvas_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        outer_rect.left() + KEYBOARD_WIDTH,
                        outer_rect.top() + RULER_HEIGHT + REGION_BAR_HEIGHT,
                    ),
                    egui::vec2(canvas_width, grid_body_height),
                );
                let vscroll_rect = egui::Rect::from_min_size(
                    egui::pos2(canvas_rect.right(), canvas_rect.top()),
                    egui::vec2(SCROLLBAR_SIZE, grid_body_height),
                );
                let hscroll_rect = egui::Rect::from_min_size(
                    egui::pos2(canvas_rect.left(), canvas_rect.bottom()),
                    egui::vec2(canvas_width, SCROLLBAR_SIZE),
                );

                let max_scroll_x = (total_content_width - canvas_rect.width()).max(0.0);
                let max_scroll_y = (total_content_height - canvas_rect.height()).max(0.0);
                self.scroll_offset_x = self.scroll_offset_x.clamp(0.0, max_scroll_x);
                self.scroll_offset_y = self.scroll_offset_y.clamp(0.0, max_scroll_y);

                // Mouse wheel / trackpad / middle-drag scrolling
                let canvas_resp = ui.interact(
                    canvas_rect,
                    ui.scope_id().with("pr_canvas"),
                    egui::Sense::click_and_drag(),
                );
                if canvas_resp.hovered()
                    || key_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()))
                {
                    let scroll = ui.input(|i| i.smooth_scroll_delta);
                    if scroll.x.abs() > 0.01 || scroll.y.abs() > 0.01 {
                        self.scroll_offset_x =
                            (self.scroll_offset_x - scroll.x).clamp(0.0, max_scroll_x);
                        self.scroll_offset_y =
                            (self.scroll_offset_y - scroll.y).clamp(0.0, max_scroll_y);
                    }
                }
                if canvas_resp.dragged_by(egui::PointerButton::Middle) {
                    let delta = canvas_resp.drag_delta();
                    self.scroll_offset_x =
                        (self.scroll_offset_x - delta.x).clamp(0.0, max_scroll_x);
                    self.scroll_offset_y =
                        (self.scroll_offset_y - delta.y).clamp(0.0, max_scroll_y);
                }

                // ── Ruler (Absolute project bars & beats) + scrubbing ────────
                let ruler_resp = ui.interact(
                    ruler_rect,
                    ui.scope_id().with("pr_ruler"),
                    egui::Sense::click_and_drag(),
                );
                if ruler_resp.drag_started() || ruler_resp.clicked() {
                    self.scrubbing_ruler = true;
                }
                if self.scrubbing_ruler {
                    if let Some(pos) = ruler_resp.interact_pointer_pos() {
                        let tick = ((pos.x - ruler_rect.left() + self.scroll_offset_x)
                            / self.px_per_tick)
                            .max(0.0) as u64;
                        outcome.seek_tick = Some(tick);
                    }
                    if ruler_resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                        self.scrubbing_ruler = false;
                    }
                }

                let ruler_painter = ui.painter_at(ruler_rect);
                ruler_painter.rect_filled(ruler_rect, 0.0, egui::Color32::from_rgb(38, 38, 40));

                let step_ticks = self.quantize_grid.ticks().max(1);
                let start_tick = (self.scroll_offset_x / self.px_per_tick).floor().max(0.0) as u64;
                let end_tick =
                    ((self.scroll_offset_x + canvas_rect.width()) / self.px_per_tick).ceil() as u64;

                let first_beat = (start_tick / PPQN) * PPQN;
                let mut bt = first_beat;
                while bt <= end_tick {
                    let x = ruler_rect.left() + bt as f32 * self.px_per_tick - self.scroll_offset_x;
                    let bar = bt / (PPQN * 4) + 1;
                    let beat = (bt % (PPQN * 4)) / PPQN + 1;
                    let is_bar = bt.is_multiple_of(PPQN * 4);
                    ruler_painter.vline(
                        x,
                        (ruler_rect.bottom() - if is_bar { 12.0 } else { 6.0 })
                            ..=ruler_rect.bottom(),
                        egui::Stroke::new(1.0, egui::Color32::from_gray(145)),
                    );
                    let label = if is_bar {
                        format!("{bar}")
                    } else {
                        format!("{bar}.{beat}")
                    };
                    ruler_painter.text(
                        egui::pos2(x + 3.0, ruler_rect.top() + 3.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(195),
                    );
                    bt = bt.saturating_add(PPQN);
                }

                // ── Green Region Header Bar ──────────────────────────────────
                let rbar_painter = ui.painter_at(region_bar_rect);
                rbar_painter.rect_filled(region_bar_rect, 0.0, egui::Color32::from_gray(30));
                let rx0 = region_bar_rect.left() + clip.start_tick as f32 * self.px_per_tick
                    - self.scroll_offset_x;
                let rx1 = region_bar_rect.left() + clip_end_tick as f32 * self.px_per_tick
                    - self.scroll_offset_x;
                let green_span = egui::Rect::from_x_y_ranges(
                    rx0..=rx1,
                    (region_bar_rect.top() + 1.0)..=(region_bar_rect.bottom() - 1.0),
                );
                rbar_painter.rect_filled(green_span, 2.0, egui::Color32::from_rgb(35, 175, 75));
                rbar_painter.text(
                    egui::pos2(
                        rx0.max(region_bar_rect.left()) + 6.0,
                        region_bar_rect.center().y,
                    ),
                    egui::Align2::LEFT_CENTER,
                    &clip.name,
                    egui::FontId::proportional(10.5),
                    egui::Color32::WHITE,
                );

                // ── Note Grid ────────────────────────────────────────────────
                let painter = ui.painter_at(canvas_rect);
                painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(24));

                for pitch in (MIN_PITCH..=MAX_PITCH).rev() {
                    let row_idx = (MAX_PITCH - pitch) as f32;
                    let y = canvas_rect.top() + row_idx * self.row_height - self.scroll_offset_y;
                    if y + self.row_height < canvas_rect.top() || y > canvas_rect.bottom() {
                        continue;
                    }
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(canvas_rect.left(), y),
                        egui::vec2(canvas_rect.width(), self.row_height),
                    );
                    let is_black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
                    let bg = if is_black {
                        egui::Color32::from_gray(20)
                    } else if pitch.is_multiple_of(12) {
                        egui::Color32::from_rgb(30, 35, 44)
                    } else {
                        egui::Color32::from_gray(27)
                    };
                    painter.rect_filled(row_rect, 0.0, bg);
                    painter.hline(
                        canvas_rect.left()..=canvas_rect.right(),
                        y + self.row_height,
                        egui::Stroke::new(0.5, egui::Color32::from_gray(38)),
                    );
                }

                // Vertical beat/grid lines (absolute project ticks)
                let first_grid = (start_tick / step_ticks) * step_ticks;
                let mut t = first_grid;
                while t <= end_tick {
                    let x = canvas_rect.left() + t as f32 * self.px_per_tick - self.scroll_offset_x;
                    let is_bar = t.is_multiple_of(PPQN * 4);
                    let is_beat = t.is_multiple_of(PPQN);
                    let color = if is_bar {
                        egui::Color32::from_gray(85)
                    } else if is_beat {
                        egui::Color32::from_gray(55)
                    } else {
                        egui::Color32::from_gray(36)
                    };
                    painter.vline(
                        x,
                        canvas_rect.top()..=canvas_rect.bottom(),
                        egui::Stroke::new(1.0, color),
                    );
                    t = t.saturating_add(step_ticks);
                }

                // Region end boundary line
                let end_x = canvas_rect.left() + clip_end_tick as f32 * self.px_per_tick
                    - self.scroll_offset_x;
                painter.vline(
                    end_x,
                    canvas_rect.top()..=canvas_rect.bottom(),
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(45, 190, 90)),
                );

                // ── Piano Keyboard Strip ─────────────────────────────────────
                let key_painter = ui.painter_at(key_rect);
                key_painter.rect_filled(key_rect, 0.0, egui::Color32::from_gray(30));
                let key_resp = ui.interact(
                    key_rect,
                    ui.scope_id().with("pr_keys"),
                    egui::Sense::click_and_drag(),
                );
                for pitch in (MIN_PITCH..=MAX_PITCH).rev() {
                    let row_idx = (MAX_PITCH - pitch) as f32;
                    let y = key_rect.top() + row_idx * self.row_height - self.scroll_offset_y;
                    if y + self.row_height < key_rect.top() || y > key_rect.bottom() {
                        continue;
                    }
                    let r = egui::Rect::from_min_size(
                        egui::pos2(key_rect.left(), y),
                        egui::vec2(KEYBOARD_WIDTH, self.row_height),
                    );
                    let is_black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
                    let fill = if is_black {
                        egui::Color32::from_gray(35)
                    } else {
                        egui::Color32::from_gray(225)
                    };
                    key_painter.rect_filled(r.shrink(0.5), 1.0, fill);
                    if pitch.is_multiple_of(12) {
                        let octave = (pitch / 12) as i32 - 1;
                        key_painter.text(
                            egui::pos2(r.left() + 4.0, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            format!("C{octave}"),
                            egui::FontId::proportional(9.0),
                            egui::Color32::from_gray(40),
                        );
                    }
                }

                // Preview note when clicking/holding piano keys
                if (key_resp.is_pointer_button_down_on() || key_resp.dragged())
                    && let Some(pos) = key_resp.interact_pointer_pos()
                {
                    let rel_y = pos.y - key_rect.top() + self.scroll_offset_y;
                    let row_idx = (rel_y / self.row_height).floor().max(0.0) as u8;
                    let pitch = MAX_PITCH
                        .saturating_sub(row_idx)
                        .clamp(MIN_PITCH, MAX_PITCH);
                    if self.previewing_note.map(|(p, _)| p) != Some(pitch) {
                        if let Some((old_p, old_ch)) = self.previewing_note.take() {
                            engine_handle.send_live_midi(MidiMessage::NoteOff {
                                channel: old_ch,
                                note: old_p,
                                velocity: 0,
                            });
                        }
                        engine_handle.send_live_midi(MidiMessage::NoteOn {
                            channel: track_channel,
                            note: pitch,
                            velocity: 100,
                        });
                        self.previewing_note = Some((pitch, track_channel));
                    }
                } else if !ui.input(|i| i.pointer.primary_down())
                    && let Some((old_p, old_ch)) = self.previewing_note.take()
                {
                    engine_handle.send_live_midi(MidiMessage::NoteOff {
                        channel: old_ch,
                        note: old_p,
                        velocity: 0,
                    });
                }

                // ── Notes & Hit-testing (offset by clip.start_tick) ──────────
                let mut hit_note: Option<(u64, bool)> = None;
                let pointer_pos = ui.input(|i| i.pointer.interact_pos());

                for note in &clip.notes {
                    let abs_start = clip.start_tick.saturating_add(note.start_tick);
                    let row_idx = (MAX_PITCH.saturating_sub(note.pitch)) as f32;
                    let x = canvas_rect.left() + abs_start as f32 * self.px_per_tick
                        - self.scroll_offset_x;
                    let w = (note.duration_ticks as f32 * self.px_per_tick).max(6.0);
                    let y = canvas_rect.top() + row_idx * self.row_height - self.scroll_offset_y;
                    let note_rect = egui::Rect::from_min_size(
                        egui::pos2(x, y + 1.0),
                        egui::vec2(w, (self.row_height - 2.0).max(4.0)),
                    );

                    if !canvas_rect.intersects(note_rect) {
                        continue;
                    }

                    let selected = self.selected_note_id == Some(note.id);
                    let vel_alpha = ((note.velocity as f32 / 127.0) * 155.0 + 100.0) as u8;
                    let fill = if selected {
                        egui::Color32::from_rgb(100, 195, 255)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(45, 200, 95, vel_alpha)
                    };
                    painter.rect_filled(note_rect, 2.0, fill);
                    painter.rect_stroke(
                        note_rect,
                        2.0,
                        egui::Stroke::new(
                            if selected { 1.5 } else { 0.8 },
                            if selected {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::from_rgb(25, 115, 55)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );

                    if let Some(p) = pointer_pos
                        && note_rect.contains(p)
                    {
                        let on_right_edge = p.x >= note_rect.right() - 6.0;
                        hit_note = Some((note.id, on_right_edge));
                    }
                }

                // ── Synced Playhead + Pentagon Head in Piano Roll ────────────
                let ph_x = canvas_rect.left() + playhead_tick as f32 * self.px_per_tick
                    - self.scroll_offset_x;
                if ph_x >= canvas_rect.left() - 6.0 && ph_x <= canvas_rect.right() + 6.0 {
                    painter.vline(
                        ph_x,
                        canvas_rect.top()..=canvas_rect.bottom(),
                        egui::Stroke::new(1.5, egui::Color32::from_gray(235)),
                    );
                    rbar_painter.vline(
                        ph_x,
                        region_bar_rect.top()..=region_bar_rect.bottom(),
                        egui::Stroke::new(1.5, egui::Color32::from_gray(235)),
                    );
                    let head_top = ruler_rect.top() + 2.0;
                    let head_mid = ruler_rect.bottom() - 6.0;
                    let head_tip = ruler_rect.bottom();
                    let head_pts = vec![
                        egui::pos2(ph_x - 5.5, head_top),
                        egui::pos2(ph_x + 5.5, head_top),
                        egui::pos2(ph_x + 5.5, head_mid),
                        egui::pos2(ph_x, head_tip),
                        egui::pos2(ph_x - 5.5, head_mid),
                    ];
                    ruler_painter.add(egui::Shape::convex_polygon(
                        head_pts,
                        egui::Color32::from_gray(235),
                        egui::Stroke::new(1.0, egui::Color32::from_gray(40)),
                    ));
                }

                // Right-click deletes a note
                if canvas_resp.secondary_clicked()
                    && let Some((nid, _)) = hit_note
                    && clip.delete_note(nid)
                {
                    if self.selected_note_id == Some(nid) {
                        self.selected_note_id = None;
                    }
                    outcome.clip_modified = true;
                }

                // Left-click / drag handling on note grid
                if canvas_resp.drag_started_by(egui::PointerButton::Primary)
                    && let Some(pos) = canvas_resp.interact_pointer_pos()
                {
                    if let Some((nid, is_resize)) = hit_note {
                        self.selected_note_id = Some(nid);
                        if let Some(n) = clip.notes.iter().find(|n| n.id == nid) {
                            let pointer_abs_tick = ((pos.x - canvas_rect.left()
                                + self.scroll_offset_x)
                                / self.px_per_tick)
                                .max(0.0) as i64;
                            let rel_tick = pointer_abs_tick - clip.start_tick as i64;
                            let kind = if is_resize {
                                NoteDragKind::Resize {
                                    orig_duration_ticks: n.duration_ticks,
                                    start_x: pos.x,
                                }
                            } else {
                                NoteDragKind::Move {
                                    grab_offset_ticks: rel_tick - n.start_tick as i64,
                                }
                            };
                            self.drag = Some(ActiveNoteDrag { note_id: nid, kind });
                        }
                    }
                } else if canvas_resp.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = canvas_resp.interact_pointer_pos()
                {
                    if let Some((nid, _)) = hit_note {
                        self.selected_note_id = Some(nid);
                    } else {
                        let raw_abs_tick = ((pos.x - canvas_rect.left() + self.scroll_offset_x)
                            / self.px_per_tick)
                            .max(0.0) as u64;
                        let raw_rel_tick = raw_abs_tick.saturating_sub(clip.start_tick);
                        let start_tick = if self.snap_enabled {
                            let step = self.quantize_grid.ticks().max(1);
                            (raw_rel_tick / step) * step
                        } else {
                            raw_rel_tick
                        };
                        let rel_y = pos.y - canvas_rect.top() + self.scroll_offset_y;
                        let row_idx = (rel_y / self.row_height).floor().max(0.0) as u8;
                        let pitch = MAX_PITCH
                            .saturating_sub(row_idx)
                            .clamp(MIN_PITCH, MAX_PITCH);
                        let dur = self.quantize_grid.ticks().max(60);
                        // Auto-extend clip length if user paints past current end
                        if start_tick + dur > clip.length_ticks {
                            clip.length_ticks = start_tick + dur;
                        }
                        let new_id = clip.add_note(start_tick, dur, pitch, 100, track_channel);
                        self.selected_note_id = Some(new_id);
                        engine_handle.send_live_midi(MidiMessage::NoteOn {
                            channel: track_channel,
                            note: pitch,
                            velocity: 100,
                        });
                        self.previewing_note = Some((pitch, track_channel));
                        outcome.clip_modified = true;
                    }
                }

                if canvas_resp.dragged_by(egui::PointerButton::Primary)
                    && let (Some(drag), Some(pos)) =
                        (&self.drag, canvas_resp.interact_pointer_pos())
                {
                    match drag.kind {
                        NoteDragKind::Move { grab_offset_ticks } => {
                            let pointer_abs_tick = ((pos.x - canvas_rect.left()
                                + self.scroll_offset_x)
                                / self.px_per_tick)
                                .max(0.0) as i64;
                            let rel_tick = pointer_abs_tick - clip.start_tick as i64;
                            let raw_start = (rel_tick - grab_offset_ticks).max(0) as u64;
                            let new_start = if self.snap_enabled {
                                quantize_tick(raw_start, self.quantize_grid, 1.0)
                            } else {
                                raw_start
                            };
                            let rel_y = pos.y - canvas_rect.top() + self.scroll_offset_y;
                            let row_idx = (rel_y / self.row_height).floor().max(0.0) as u8;
                            let new_pitch = MAX_PITCH
                                .saturating_sub(row_idx)
                                .clamp(MIN_PITCH, MAX_PITCH);
                            if clip.move_note(drag.note_id, new_start, new_pitch) {
                                outcome.clip_modified = true;
                            }
                        }
                        NoteDragKind::Resize {
                            orig_duration_ticks,
                            start_x,
                        } => {
                            let dx_ticks = ((pos.x - start_x) / self.px_per_tick) as i64;
                            let raw_dur = (orig_duration_ticks as i64 + dx_ticks).max(30) as u64;
                            let new_dur = if self.snap_enabled {
                                let step = self.quantize_grid.ticks().max(1);
                                quantize_tick(raw_dur, self.quantize_grid, 1.0).max(step)
                            } else {
                                raw_dur
                            };
                            if clip.resize_note(drag.note_id, new_dur) {
                                outcome.clip_modified = true;
                            }
                        }
                    }
                }

                if canvas_resp.drag_stopped() {
                    self.drag = None;
                }

                // ── Vertical Scrollbar ───────────────────────────────────────
                let v_painter = ui.painter_at(vscroll_rect);
                v_painter.rect_filled(vscroll_rect, 2.0, egui::Color32::from_gray(22));
                if max_scroll_y > 0.0 {
                    let frac_vis = (canvas_rect.height() / total_content_height).clamp(0.08, 1.0);
                    let thumb_h = (vscroll_rect.height() * frac_vis).max(20.0);
                    let track_travel = (vscroll_rect.height() - thumb_h).max(1.0);
                    let thumb_y =
                        vscroll_rect.top() + (self.scroll_offset_y / max_scroll_y) * track_travel;
                    let thumb_rect = egui::Rect::from_min_size(
                        egui::pos2(vscroll_rect.left() + 2.0, thumb_y),
                        egui::vec2(SCROLLBAR_SIZE - 4.0, thumb_h),
                    );
                    let v_resp = ui.interact(
                        vscroll_rect,
                        ui.scope_id().with("pr_vscroll"),
                        egui::Sense::click_and_drag(),
                    );
                    if v_resp.dragged() {
                        let dy = v_resp.drag_delta().y;
                        self.scroll_offset_y = (self.scroll_offset_y
                            + (dy / track_travel) * max_scroll_y)
                            .clamp(0.0, max_scroll_y);
                    } else if v_resp.clicked()
                        && let Some(pos) = v_resp.interact_pointer_pos()
                    {
                        let rel = ((pos.y - vscroll_rect.top() - thumb_h * 0.5) / track_travel)
                            .clamp(0.0, 1.0);
                        self.scroll_offset_y = rel * max_scroll_y;
                    }
                    v_painter.rect_filled(
                        thumb_rect,
                        4.0,
                        if v_resp.hovered() || v_resp.dragged() {
                            egui::Color32::from_gray(145)
                        } else {
                            egui::Color32::from_gray(95)
                        },
                    );
                }

                // ── Horizontal Scrollbar ─────────────────────────────────────
                let h_painter = ui.painter_at(hscroll_rect);
                h_painter.rect_filled(hscroll_rect, 2.0, egui::Color32::from_gray(22));
                if max_scroll_x > 0.0 {
                    let frac_vis = (canvas_rect.width() / total_content_width).clamp(0.08, 1.0);
                    let thumb_w = (hscroll_rect.width() * frac_vis).max(24.0);
                    let track_travel = (hscroll_rect.width() - thumb_w).max(1.0);
                    let thumb_x =
                        hscroll_rect.left() + (self.scroll_offset_x / max_scroll_x) * track_travel;
                    let thumb_rect = egui::Rect::from_min_size(
                        egui::pos2(thumb_x, hscroll_rect.top() + 2.0),
                        egui::vec2(thumb_w, SCROLLBAR_SIZE - 4.0),
                    );
                    let h_resp = ui.interact(
                        hscroll_rect,
                        ui.scope_id().with("pr_hscroll"),
                        egui::Sense::click_and_drag(),
                    );
                    if h_resp.dragged() {
                        let dx = h_resp.drag_delta().x;
                        self.scroll_offset_x = (self.scroll_offset_x
                            + (dx / track_travel) * max_scroll_x)
                            .clamp(0.0, max_scroll_x);
                    } else if h_resp.clicked()
                        && let Some(pos) = h_resp.interact_pointer_pos()
                    {
                        let rel = ((pos.x - hscroll_rect.left() - thumb_w * 0.5) / track_travel)
                            .clamp(0.0, 1.0);
                        self.scroll_offset_x = rel * max_scroll_x;
                    }
                    h_painter.rect_filled(
                        thumb_rect,
                        4.0,
                        if h_resp.hovered() || h_resp.dragged() {
                            egui::Color32::from_gray(145)
                        } else {
                            egui::Color32::from_gray(95)
                        },
                    );
                }

                // ── Separate Velocity Lane ───────────────────────────────────
                if self.show_velocity_lane {
                    ui.add_space(4.0);
                    let (vel_outer, _) = ui.allocate_exact_size(
                        egui::vec2(right_avail.x.max(200.0), VELOCITY_LANE_HEIGHT),
                        egui::Sense::hover(),
                    );
                    let vel_label_rect = egui::Rect::from_min_size(
                        vel_outer.min,
                        egui::vec2(KEYBOARD_WIDTH, VELOCITY_LANE_HEIGHT),
                    );
                    let vel_canvas_rect = egui::Rect::from_min_size(
                        egui::pos2(vel_outer.left() + KEYBOARD_WIDTH, vel_outer.top()),
                        egui::vec2(canvas_width, VELOCITY_LANE_HEIGHT),
                    );

                    ui.painter_at(vel_label_rect).rect_filled(
                        vel_label_rect,
                        0.0,
                        egui::Color32::from_gray(28),
                    );
                    ui.painter_at(vel_label_rect).text(
                        vel_label_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Velocity",
                        egui::FontId::proportional(10.0),
                        egui::Color32::from_gray(170),
                    );

                    let vel_painter = ui.painter_at(vel_canvas_rect);
                    vel_painter.rect_filled(vel_canvas_rect, 0.0, egui::Color32::from_gray(18));

                    let vel_resp = ui.interact(
                        vel_canvas_rect,
                        ui.scope_id().with("pr_vel"),
                        egui::Sense::click_and_drag(),
                    );

                    for note in &clip.notes {
                        let abs_start = clip.start_tick.saturating_add(note.start_tick);
                        let x = vel_canvas_rect.left() + abs_start as f32 * self.px_per_tick
                            - self.scroll_offset_x;
                        if x < vel_canvas_rect.left() - 8.0 || x > vel_canvas_rect.right() + 8.0 {
                            continue;
                        }
                        let frac = f32::from(note.velocity) / 127.0;
                        let bar_h = frac * (VELOCITY_LANE_HEIGHT - 8.0);
                        let bar_rect = egui::Rect::from_min_size(
                            egui::pos2(x, vel_canvas_rect.bottom() - bar_h - 2.0),
                            egui::vec2(7.0, bar_h.max(2.0)),
                        );
                        let selected = self.selected_note_id == Some(note.id);
                        let col = if selected {
                            egui::Color32::from_rgb(100, 195, 255)
                        } else {
                            egui::Color32::from_rgb(65, 200, 95)
                        };
                        vel_painter.rect_filled(bar_rect, 1.0, col);
                    }

                    if (vel_resp.clicked() || vel_resp.dragged())
                        && let Some(pos) = vel_resp.interact_pointer_pos()
                    {
                        let click_abs_tick = ((pos.x - vel_canvas_rect.left()
                            + self.scroll_offset_x)
                            / self.px_per_tick)
                            .max(0.0) as u64;
                        let click_rel_tick = click_abs_tick.saturating_sub(clip.start_tick);
                        let new_vel = (((vel_canvas_rect.bottom() - pos.y)
                            / (VELOCITY_LANE_HEIGHT - 6.0))
                            .clamp(0.01, 1.0)
                            * 127.0)
                            .round() as u8;
                        if let Some(nearest) = clip
                            .notes
                            .iter_mut()
                            .min_by_key(|n| n.start_tick.abs_diff(click_rel_tick))
                            && (nearest.start_tick as f32 - click_rel_tick as f32).abs()
                                * self.px_per_tick
                                <= 24.0
                        {
                            nearest.velocity = new_vel.clamp(1, 127);
                            self.selected_note_id = Some(nearest.id);
                            outcome.clip_modified = true;
                        }
                    }
                }
            });
        });

        outcome
    }
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch / 12) as i32 - 1;
    let name = NAMES[usize::from(pitch % 12)];
    format!("{name}{octave}")
}
