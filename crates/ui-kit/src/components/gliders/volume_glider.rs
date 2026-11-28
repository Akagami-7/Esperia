use egui::{self, Color32, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::animation::{SpringNode, SpringParams, lerp_color};

const WIDTH: f32 = 200.0;
const HEIGHT: f32 = 24.0;
const PADDING: f32 = 12.0;
const BASE_SIZE: f32 = 18.0;
const MAX_STRETCH: f32 = 6.0;
const FIXED_STEP: f32 = 1.0 / 120.0;
const MAX_FRAME_TIME: f32 = 0.05;
const METER_HEIGHT: f32 = 2.5;
const METER_SEGMENTS: usize = 128;
const TRACK_COLOR: Color32 = Color32::from_rgb(50, 51, 53);

const GLIDER_SPRING: SpringParams = SpringParams {
    stiffness: 420.0,
    damping: 40.987_804,
};

/// Elastic spring-physics horizontal volume glider with dual stereo meter strips.
pub struct VolumeGlider {
    initialized: bool,
    target_x: f32,
    nodes: [SpringNode; 4],
    last_time: f64,
    accumulator: f32,
    pub value: f32,
}

impl VolumeGlider {
    pub fn new() -> Self {
        Self {
            initialized: false,
            target_x: PADDING,
            nodes: [SpringNode::default(); 4],
            last_time: 0.0,
            accumulator: 0.0,
            value: 0.0,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) -> Response {
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(WIDTH, HEIGHT), Sense::click_and_drag());

        let painter = ui.painter();
        let min_x = PADDING;
        let max_x = rect.width() - PADDING;
        let time = ui.input(|i| i.time);

        if !self.initialized {
            let initial_x = min_x + self.value.clamp(0.0, 1.0) * (max_x - min_x);
            self.target_x = initial_x;
            self.nodes = [SpringNode::at(initial_x); 4];
            self.last_time = time;
            self.initialized = true;
        }

        let frame_time = ((time - self.last_time) as f32).clamp(0.0, MAX_FRAME_TIME);
        self.last_time = time;
        self.accumulator += frame_time;

        if response.is_pointer_button_down_on()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.target_x = (pointer.x - rect.left()).clamp(min_x, max_x);
        }

        while self.accumulator >= FIXED_STEP {
            self.simulate_step(min_x, max_x);
            self.accumulator -= FIXED_STEP;
        }

        let previous_value = self.value;
        self.value = ((self.nodes[0].position - min_x) / (max_x - min_x)).clamp(0.0, 1.0);
        if (self.value - previous_value).abs() > 0.00001 {
            response.mark_changed();
        }

        let min_node = self
            .nodes
            .iter()
            .map(|n| n.position)
            .fold(f32::INFINITY, f32::min);
        let max_node = self
            .nodes
            .iter()
            .map(|n| n.position)
            .fold(f32::NEG_INFINITY, f32::max);
        let stretch = (max_node - min_node).clamp(0.0, MAX_STRETCH);
        let thumb_width = BASE_SIZE + stretch;
        let thumb_height = (BASE_SIZE - stretch * 0.16).max(15.0);

        painter.rect_filled(rect, HEIGHT / 2.0, TRACK_COLOR);

        let meter_x = rect.left() + PADDING;
        let meter_width = rect.width() - PADDING * 2.0;
        let filled_width = meter_width * self.value;

        for row in 0..2 {
            let meter_y = rect.center().y - 3.75;
            let y = meter_y + row as f32 * 5.0;

            self.draw_gradient_meter(painter, meter_x, y, meter_width, filled_width);

            if self.value >= 0.90 {
                let peak_color = if row == 0 {
                    Color32::from_rgb(239, 68, 68)
                } else {
                    Color32::from_rgb(220, 38, 38)
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(meter_x + meter_width - 3.0, y),
                        vec2(3.0, METER_HEIGHT),
                    ),
                    1.0,
                    peak_color,
                );
            }
        }

        let thumb_rect = Rect::from_center_size(
            pos2(
                rect.left() + self.nodes[0].position.clamp(min_x, max_x),
                rect.center().y,
            ),
            vec2(thumb_width, thumb_height),
        );
        let thumb_radius = thumb_height / 2.0;

        painter.rect_filled(
            thumb_rect.translate(vec2(0.0, 2.0)),
            thumb_radius,
            Color32::from_rgba_unmultiplied(0, 0, 0, 90),
        );
        painter.rect_filled(
            thumb_rect,
            thumb_radius,
            Color32::from_rgba_unmultiplied(216, 218, 221, 199),
        );
        painter.rect_stroke(
            thumb_rect,
            thumb_radius,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 71)),
            egui::StrokeKind::Inside,
        );

        let animating = self.nodes.iter().any(|n| n.velocity.abs() > 0.01)
            || (self.nodes[0].position - self.target_x).abs() > 0.05;
        if animating || response.dragged() || response.clicked() {
            ui.ctx().request_repaint();
        }

        response
    }

    fn simulate_step(&mut self, min_x: f32, max_x: f32) {
        self.nodes[0].step(self.target_x, FIXED_STEP, GLIDER_SPRING);
        Self::clamp_node(&mut self.nodes[0], min_x, max_x);

        for i in 1..4 {
            let leader_x = self.nodes[i - 1].position;
            self.nodes[i].step(leader_x, FIXED_STEP, GLIDER_SPRING);
            Self::clamp_node(&mut self.nodes[i], min_x, max_x);
        }

        if self.nodes[0].is_settled(self.target_x, 0.05)
            && self.nodes[3].is_settled(self.nodes[0].position, 0.05)
        {
            for node in &mut self.nodes {
                node.position = self.target_x;
                node.velocity = 0.0;
            }
        }
    }

    fn draw_gradient_meter(
        &self,
        painter: &egui::Painter,
        x: f32,
        y: f32,
        width: f32,
        filled_width: f32,
    ) {
        if filled_width <= 0.0 {
            return;
        }

        for i in 0..METER_SEGMENTS {
            let t1 = i as f32 / METER_SEGMENTS as f32;
            let t2 = (i + 1) as f32 / METER_SEGMENTS as f32;
            let x1 = x + t1 * width;
            let x2 = x + t2 * width;
            if x1 >= x + filled_width {
                break;
            }
            let end_x = x2.min(x + filled_width);
            let color = Self::gradient_color((t1 + t2) * 0.5);
            painter.rect_filled(
                Rect::from_min_max(pos2(x1, y), pos2(end_x + 0.15, y + METER_HEIGHT)),
                0.0,
                color,
            );
        }
    }

    fn gradient_color(t: f32) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        let stops = [
            (0.00, Color32::from_rgb(34, 197, 94)),
            (0.55, Color32::from_rgb(60, 208, 84)),
            (0.80, Color32::from_rgb(238, 212, 28)),
            (0.92, Color32::from_rgb(249, 115, 22)),
            (1.00, Color32::from_rgb(239, 68, 68)),
        ];

        for i in 0..stops.len() - 1 {
            let (p1, c1) = stops[i];
            let (p2, c2) = stops[i + 1];
            if t <= p2 {
                let f = ((t - p1) / (p2 - p1)).clamp(0.0, 1.0);
                return lerp_color(c1, c2, f);
            }
        }
        Color32::RED
    }

    fn clamp_node(node: &mut SpringNode, min: f32, max: f32) {
        if node.position < min {
            node.position = min;
            node.velocity = 0.0;
        } else if node.position > max {
            node.position = max;
            node.velocity = 0.0;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Response {
        self.show(ui)
    }

    pub fn value(&self) -> f32 {
        self.value
    }
}

impl Default for VolumeGlider {
    fn default() -> Self {
        Self::new()
    }
}
