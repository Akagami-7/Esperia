use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
struct Node {
    x: f32,
    vx: f32,
}

pub struct VolumeGlider {
    pub value: f32, // 0.0 to 1.0
    target_x: f32,
    nodes: [Node; 4],
    initialized: bool,
}

impl Default for VolumeGlider {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl VolumeGlider {
    pub fn new(initial_value: f32) -> Self {
        Self {
            value: initial_value.clamp(0.0, 1.0),
            target_x: 0.0,
            nodes: [Node { x: 0.0, vx: 0.0 }; 4],
            initialized: false,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Response {
        let desired_size = Vec2::new(300.0, 36.0);
        let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        let padding = 18.0; // Symmetrical 2.5px concentric gap on all sides
        let base_size = 31.0;
        let max_stretch = 18.0;

        let min_x = rect.left() + padding;
        let max_x = rect.right() - padding;
        let usable_width = (max_x - min_x).max(1.0);

        if !self.initialized {
            let start_x = min_x + self.value * usable_width;
            self.target_x = start_x;
            for node in &mut self.nodes {
                node.x = start_x;
                node.vx = 0.0;
            }
            self.initialized = true;
        }

        // Handle pointer click or drag
        if response.is_pointer_button_down_on()
            && let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos())
        {
            self.target_x = pointer_pos.x.clamp(min_x, max_x);
        }

        // Frame-rate normalized 4-part follow-through spring physics
        let dt = (ui.input(|i| i.stable_dt) * 60.0).clamp(0.1, 2.0);

        // Part 1: Leading front edge
        let head = &mut self.nodes[0];
        let head_force = (self.target_x - head.x) * 0.45 * dt;
        head.vx = (head.vx + head_force) * 0.48_f32.powf(dt);
        head.x = (head.x + head.vx * dt).clamp(min_x, max_x);

        // Parts 2, 3, & 4: Trailing body & tail
        for i in 1..4 {
            let leader_x = self.nodes[i - 1].x;
            let follower = &mut self.nodes[i];
            let force = (leader_x - follower.x) * 0.32 * dt;
            follower.vx = (follower.vx + force) * 0.50_f32.powf(dt);
            follower.x = (follower.x + follower.vx * dt).clamp(min_x, max_x);
        }

        // Keep repainting while the 4-part chain is still settling
        let is_moving = (self.target_x - self.nodes[0].x).abs() > 0.05
            || (self.nodes[0].x - self.nodes[3].x).abs() > 0.05
            || self.nodes[3].vx.abs() > 0.05;

        if is_moving {
            ui.ctx().request_repaint();
        } else {
            for node in &mut self.nodes {
                node.x = self.target_x;
                node.vx = 0.0;
            }
        }

        // Update normalized volume value (0.0 .. 1.0)
        let new_val = ((self.nodes[0].x - min_x) / usable_width).clamp(0.0, 1.0);
        if (new_val - self.value).abs() > 0.001 {
            self.value = new_val;
            response.mark_changed();
        }

        // --- Drawing ---
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // 1. Dark charcoal capsule housing (#323335)
            painter.rect_filled(rect, CornerRadius::same(18), Color32::from_rgb(50, 51, 53));

            // 2. Stereo 7:2:1 gradient meter bars
            let bar_height = 5.5;
            let gap = 4.0;
            let top_bar_y = rect.center().y - gap * 0.5 - bar_height;
            let bot_bar_y = rect.center().y + gap * 0.5;
            let fill_width = self.value * usable_width;

            if fill_width > 0.5 {
                let top_rect = Rect::from_min_size(
                    Pos2::new(min_x, top_bar_y),
                    Vec2::new(fill_width, bar_height),
                );
                let bot_rect = Rect::from_min_size(
                    Pos2::new(min_x, bot_bar_y),
                    Vec2::new(fill_width, bar_height),
                );
                draw_gradient_bar(painter, top_rect, usable_width);
                draw_gradient_bar(painter, bot_rect, usable_width);
            }

            // 3. Red clip / peak hold ticks (>= 90%)
            if self.value >= 0.90 {
                let tick_x = max_x - 3.0;
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(tick_x, top_bar_y), Vec2::new(3.0, bar_height)),
                    CornerRadius::same(1),
                    Color32::from_rgb(239, 68, 68),
                );
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(tick_x, bot_bar_y), Vec2::new(3.0, bar_height)),
                    CornerRadius::same(1),
                    Color32::from_rgb(220, 38, 38),
                );
            }

            // 4. Stretchy frosted circular thumb
            let min_node_x = self.nodes.iter().map(|n| n.x).fold(f32::INFINITY, f32::min);
            let max_node_x = self
                .nodes
                .iter()
                .map(|n| n.x)
                .fold(f32::NEG_INFINITY, f32::max);

            let stretch_span = (max_node_x - min_node_x).min(max_stretch);
            let current_width = base_size + stretch_span;
            let current_height = (base_size - stretch_span * 0.16).max(26.0);

            let min_left_edge = min_x - base_size * 0.5;
            let max_right_edge = max_x + base_size * 0.5;
            let left_edge =
                (min_node_x - base_size * 0.5).clamp(min_left_edge, max_right_edge - current_width);

            let thumb_rect = Rect::from_center_size(
                Pos2::new(left_edge + current_width * 0.5, rect.center().y),
                Vec2::new(current_width, current_height),
            );

            let fill_alpha = if response.is_pointer_button_down_on() {
                225
            } else {
                198
            };
            painter.rect_filled(
                thumb_rect,
                CornerRadius::same((current_height * 0.5) as u8),
                Color32::from_rgba_unmultiplied(216, 218, 221, fill_alpha),
            );
            painter.rect_stroke(
                thumb_rect,
                CornerRadius::same((current_height * 0.5) as u8),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 72)),
                egui::StrokeKind::Inside,
            );
        }

        response
    }
}

/// Renders the locked 7:2:1 Green -> Yellow -> Red gradient clipped to the current volume width
fn draw_gradient_bar(painter: &egui::Painter, rect: Rect, full_width: f32) {
    let stops: &[(f32, Color32)] = &[
        (0.00, Color32::from_rgb(34, 197, 94)),  // Green
        (0.55, Color32::from_rgb(60, 208, 84)),  // Bright Green
        (0.80, Color32::from_rgb(238, 212, 28)), // Yellow
        (0.92, Color32::from_rgb(249, 115, 22)), // Orange
        (1.00, Color32::from_rgb(239, 68, 68)),  // Red
    ];

    let segments = 24;
    for i in 0..segments {
        let t0 = i as f32 / segments as f32;
        let t1 = (i + 1) as f32 / segments as f32;
        let seg_x0 = rect.left() + t0 * rect.width();
        let seg_x1 = rect.left() + t1 * rect.width();

        // Sample color based on absolute position along the full track (not stretched)
        let track_t = ((seg_x0 + seg_x1) * 0.5 - rect.left()) / full_width;
        let color = sample_gradient(stops, track_t.clamp(0.0, 1.0));

        let seg_rect = Rect::from_min_max(
            Pos2::new(seg_x0, rect.top()),
            Pos2::new(seg_x1 + 0.5, rect.bottom()),
        );
        let rounding = if i == 0 || i == segments - 1 {
            CornerRadius::same(3)
        } else {
            CornerRadius::ZERO
        };
        painter.rect_filled(seg_rect, rounding, color);
    }
}

fn sample_gradient(stops: &[(f32, Color32)], t: f32) -> Color32 {
    for window in stops.windows(2) {
        let (t0, c0) = window[0];
        let (t1, c1) = window[1];
        if t >= t0 && t <= t1 {
            let local_t = (t - t0) / (t1 - t0);
            return Color32::from_rgb(
                (c0.r() as f32 + (c1.r() as f32 - c0.r() as f32) * local_t) as u8,
                (c0.g() as f32 + (c1.g() as f32 - c0.g() as f32) * local_t) as u8,
                (c0.b() as f32 + (c1.b() as f32 - c0.b() as f32) * local_t) as u8,
            );
        }
    }
    stops.last().unwrap().1
}
