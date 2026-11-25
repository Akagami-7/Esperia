//! Interactive Piano Roll & Velocity Lane editor for MIDI clips.
//!
//! Supports:
//! - Clicking to create notes (snapped to active quantization grid)
//! - Dragging notes to move pitch and time
//! - Dragging the right edge of a note to resize duration
//! - Right-clicking or pressing Delete/Backspace to delete notes
//! - Separate velocity editing lane below the note grid
//! - Quantize button + grid selector (1/4, 1/8, 1/16, triplets, etc.)
//! - Horizontal (time) and vertical (pitch) zoom
//! - Live instrument preview when clicking piano keys or creating/moving notes

use eframe::egui;
use esperia_audio_engine::EngineHandle;
use esperia_midi::{MidiClip, MidiMessage, PPQN, QuantizeGrid, quantize_notes, quantize_tick};

const KEYBOARD_WIDTH: f32 = 52.0;
const VELOCITY_LANE_HEIGHT: f32 = 68.0;
const MIN_PITCH: u8 = 24; // C1
const MAX_PITCH: u8 = 108; // C8

#[derive(Clone, Copy, PartialEq)]
enum NoteDragKind {
    Move {
        orig_start_tick: u64,
        orig_pitch: u8,
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

/// Persistent UI state for the Piano Roll editor.
pub struct PianoRollState {
    pub px_per_tick: f32,
    pub row_height: f32,
    pub quantize_grid: QuantizeGrid,
    pub snap_enabled: bool,
    pub selected_note_id: Option<u64>,
    drag: Option<ActiveNoteDrag>,
    previewing_note: Option<(u8, u8)>,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
}

impl Default for PianoRollState {
    fn default() -> Self {
        Self {
            px_per_tick: 0.06, // ~57.6 px per quarter note
            row_height: 14.0,
            quantize_grid: QuantizeGrid::Sixteenth,
            snap_enabled: true,
            selected_note_id: None,
            drag: None,
            previewing_note: None,
            scroll_offset_x: 0.0,
            // Start scrolled near middle C (C4 = 60)
            scroll_offset_y: ((MAX_PITCH - 68) as f32) * 14.0,
        }
    }
}

impl PianoRollState {
    /// Draw the Piano Roll + Velocity Lane for `clip`.
    /// Returns `true` if `clip` was modified and the MIDI schedule should be rebuilt.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        clip: &mut MidiClip,
        track_channel: u8,
        engine_handle: &EngineHandle,
    ) -> bool {
        let mut modified = false;

        // ── Toolbar ──────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.strong(format!("🎹 Piano Roll — {}", clip.name));
            ui.separator();

            ui.checkbox(&mut self.snap_enabled, "Snap");
            egui::ComboBox::from_id_salt("pr_grid")
                .selected_text(self.quantize_grid.label())
                .width(64.0)
                .show_ui(ui, |ui| {
                    for g in QuantizeGrid::ALL {
                        ui.selectable_value(&mut self.quantize_grid, g, g.label());
                    }
                });

            if ui.button("Quantize All").clicked() {
                quantize_notes(&mut clip.notes, self.quantize_grid, 1.0, false);
                modified = true;
            }
            if let Some(sel_id) = self.selected_note_id
                && ui.button("Quantize Sel").clicked()
                && let Some(note) = clip.notes.iter_mut().find(|n| n.id == sel_id)
            {
                note.start_tick = quantize_tick(note.start_tick, self.quantize_grid, 1.0);
                modified = true;
            }

            ui.separator();
            ui.label("Time Zoom:");
            ui.add(egui::Slider::new(&mut self.px_per_tick, 0.015..=0.25).show_value(false));
            ui.label("Pitch Zoom:");
            ui.add(egui::Slider::new(&mut self.row_height, 8.0..=24.0).show_value(false));

            if let Some(sel_id) = self.selected_note_id {
                ui.separator();
                if ui.button("🗑 Delete Note").clicked() && clip.delete_note(sel_id) {
                    self.selected_note_id = None;
                    modified = true;
                }
            }
        });

        // Handle Delete/Backspace keyboard shortcut
        if let Some(sel_id) = self.selected_note_id
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && clip.delete_note(sel_id)
        {
            self.selected_note_id = None;
            modified = true;
        }

        ui.separator();

        // ── Main note grid + piano keyboard ──────────────────────────────────
        let available = ui.available_size();
        let grid_height = (available.y - VELOCITY_LANE_HEIGHT - 16.0).max(120.0);

        let num_pitches = (MAX_PITCH - MIN_PITCH + 1) as usize;
        let total_content_height = num_pitches as f32 * self.row_height;
        let total_content_width =
            (clip.length_ticks.max(PPQN * 8) as f32 * self.px_per_tick).max(600.0);

        let (outer_rect, _) = ui.allocate_exact_size(
            egui::vec2(available.x.max(200.0), grid_height),
            egui::Sense::hover(),
        );

        let key_rect =
            egui::Rect::from_min_size(outer_rect.min, egui::vec2(KEYBOARD_WIDTH, grid_height));
        let canvas_rect = egui::Rect::from_min_max(
            egui::pos2(outer_rect.left() + KEYBOARD_WIDTH, outer_rect.top()),
            outer_rect.max,
        );

        // Mouse wheel scroll on canvas
        let canvas_resp = ui.interact(
            canvas_rect,
            ui.scope_id().with("pr_canvas"),
            egui::Sense::click_and_drag(),
        );
        if canvas_resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            self.scroll_offset_x = (self.scroll_offset_x - scroll.x)
                .clamp(0.0, (total_content_width - canvas_rect.width()).max(0.0));
            self.scroll_offset_y = (self.scroll_offset_y - scroll.y)
                .clamp(0.0, (total_content_height - canvas_rect.height()).max(0.0));
        }

        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(24));

        // Draw pitch rows & black-key shading
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
            } else if pitch % 12 == 0 {
                egui::Color32::from_rgb(30, 34, 42)
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

        // Draw vertical beat/grid lines
        let step_ticks = self.quantize_grid.ticks().max(1);
        let start_tick = (self.scroll_offset_x / self.px_per_tick).floor().max(0.0) as u64;
        let end_tick =
            ((self.scroll_offset_x + canvas_rect.width()) / self.px_per_tick).ceil() as u64;
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

        // Draw piano keys on the left
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
                egui::Color32::from_gray(215)
            };
            key_painter.rect_filled(r.shrink(0.5), 1.0, fill);
            if pitch % 12 == 0 {
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

        // Draw existing notes & detect hit-test
        let mut hit_note: Option<(u64, bool)> = None; // (note_id, is_resize_edge)
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());

        for note in &clip.notes {
            let row_idx = (MAX_PITCH.saturating_sub(note.pitch)) as f32;
            let x = canvas_rect.left() + note.start_tick as f32 * self.px_per_tick
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
                egui::Color32::from_rgba_unmultiplied(75, 180, 130, vel_alpha)
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
                        egui::Color32::from_rgb(40, 110, 80)
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

        // Right-click deletes a note
        if canvas_resp.secondary_clicked()
            && let Some((nid, _)) = hit_note
            && clip.delete_note(nid)
        {
            if self.selected_note_id == Some(nid) {
                self.selected_note_id = None;
            }
            modified = true;
        }

        // Left-click / drag handling on note grid
        if canvas_resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = canvas_resp.interact_pointer_pos()
        {
            if let Some((nid, is_resize)) = hit_note {
                self.selected_note_id = Some(nid);
                if let Some(n) = clip.notes.iter().find(|n| n.id == nid) {
                    let pointer_tick = ((pos.x - canvas_rect.left() + self.scroll_offset_x)
                        / self.px_per_tick)
                        .max(0.0) as i64;
                    let kind = if is_resize {
                        NoteDragKind::Resize {
                            orig_duration_ticks: n.duration_ticks,
                            start_x: pos.x,
                        }
                    } else {
                        NoteDragKind::Move {
                            orig_start_tick: n.start_tick,
                            orig_pitch: n.pitch,
                            grab_offset_ticks: pointer_tick - n.start_tick as i64,
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
                // Create a new note at clicked pitch & tick
                let raw_tick = ((pos.x - canvas_rect.left() + self.scroll_offset_x)
                    / self.px_per_tick)
                    .max(0.0) as u64;
                let start_tick = if self.snap_enabled {
                    let step = self.quantize_grid.ticks().max(1);
                    (raw_tick / step) * step
                } else {
                    raw_tick
                };
                let rel_y = pos.y - canvas_rect.top() + self.scroll_offset_y;
                let row_idx = (rel_y / self.row_height).floor().max(0.0) as u8;
                let pitch = MAX_PITCH
                    .saturating_sub(row_idx)
                    .clamp(MIN_PITCH, MAX_PITCH);
                let dur = self.quantize_grid.ticks().max(60);
                let new_id = clip.add_note(start_tick, dur, pitch, 100, track_channel);
                self.selected_note_id = Some(new_id);
                // Short live preview trigger
                engine_handle.send_live_midi(MidiMessage::NoteOn {
                    channel: track_channel,
                    note: pitch,
                    velocity: 100,
                });
                self.previewing_note = Some((pitch, track_channel));
                modified = true;
            }
        }

        if canvas_resp.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(pos)) = (&self.drag, canvas_resp.interact_pointer_pos())
        {
            match drag.kind {
                NoteDragKind::Move {
                    grab_offset_ticks, ..
                } => {
                    let pointer_tick = ((pos.x - canvas_rect.left() + self.scroll_offset_x)
                        / self.px_per_tick)
                        .max(0.0) as i64;
                    let raw_start = (pointer_tick - grab_offset_ticks).max(0) as u64;
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
                        modified = true;
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
                        modified = true;
                    }
                }
            }
        }

        if canvas_resp.drag_stopped() {
            self.drag = None;
        }

        // ── Separate Velocity Lane ───────────────────────────────────────────
        ui.add_space(4.0);
        let (vel_outer, _) = ui.allocate_exact_size(
            egui::vec2(available.x.max(200.0), VELOCITY_LANE_HEIGHT),
            egui::Sense::hover(),
        );
        let vel_label_rect = egui::Rect::from_min_size(
            vel_outer.min,
            egui::vec2(KEYBOARD_WIDTH, VELOCITY_LANE_HEIGHT),
        );
        let vel_canvas_rect = egui::Rect::from_min_max(
            egui::pos2(vel_outer.left() + KEYBOARD_WIDTH, vel_outer.top()),
            vel_outer.max,
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
            let x = vel_canvas_rect.left() + note.start_tick as f32 * self.px_per_tick
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
                egui::Color32::from_rgb(80, 185, 130)
            };
            vel_painter.rect_filled(bar_rect, 1.0, col);
        }

        // Edit velocity of nearest note horizontally when clicking/dragging in velocity lane
        if (vel_resp.clicked() || vel_resp.dragged())
            && let Some(pos) = vel_resp.interact_pointer_pos()
        {
            let click_tick = ((pos.x - vel_canvas_rect.left() + self.scroll_offset_x)
                / self.px_per_tick)
                .max(0.0) as u64;
            let new_vel = (((vel_canvas_rect.bottom() - pos.y) / (VELOCITY_LANE_HEIGHT - 6.0))
                .clamp(0.01, 1.0)
                * 127.0)
                .round() as u8;
            if let Some(nearest) = clip
                .notes
                .iter_mut()
                .min_by_key(|n| n.start_tick.abs_diff(click_tick))
                && (nearest.start_tick as f32 - click_tick as f32).abs() * self.px_per_tick <= 24.0
            {
                nearest.velocity = new_vel.clamp(1, 127);
                self.selected_note_id = Some(nearest.id);
                modified = true;
            }
        }

        modified
    }
}
