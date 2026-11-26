use eframe::egui::{self, Color32, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

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

#[derive(Clone, Copy, Default)]
struct Node {
    x: f32,
    vx: f32,
}

pub struct VolumeGlider {
    initialized: bool,

    target_x: f32,

    nodes: [Node; 4],

    last_time: f64,

    accumulator: f32,

    pub value: f32,
}

impl VolumeGlider {
    pub fn new() -> Self {
        Self {
            initialized: false,
            target_x: PADDING,
            nodes: [Node::default(); 4],
            last_time: 0.0,
            accumulator: 0.0,
            value: 0.0,
        }
    }

    // ---------------------------------------------------------
    // MAIN WIDGET
    // ---------------------------------------------------------

    pub fn show(&mut self, ui: &mut Ui) -> Response {
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(WIDTH, HEIGHT), Sense::click_and_drag());

        let painter = ui.painter();

        let min_x = PADDING;
        let max_x = rect.width() - PADDING;

        // -----------------------------------------------------
        // INITIALIZATION
        // -----------------------------------------------------

        let time = ui.input(|i| i.time);

        if !self.initialized {
            let initial_x = min_x + self.value.clamp(0.0, 1.0) * (max_x - min_x);

            self.target_x = initial_x;

            self.nodes = [Node {
                x: initial_x,
                vx: 0.0,
            }; 4];

            self.last_time = time;
            self.initialized = true;
        }

        // -----------------------------------------------------
        // FRAME TIMING
        // -----------------------------------------------------

        let frame_time = ((time - self.last_time) as f32).clamp(0.0, MAX_FRAME_TIME);

        self.last_time = time;

        self.accumulator += frame_time;

        // -----------------------------------------------------
        // POINTER INPUT
        // -----------------------------------------------------

        if response.is_pointer_button_down_on()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.target_x = (pointer.x - rect.left()).clamp(min_x, max_x);
        }
        // -----------------------------------------------------
        // FIXED TIMESTEP SPRING PHYSICS
        // -----------------------------------------------------

        while self.accumulator >= FIXED_STEP {
            self.simulate_step(min_x, max_x);

            self.accumulator -= FIXED_STEP;
        }

        // -----------------------------------------------------
        // CURRENT VALUE
        // -----------------------------------------------------

        let previous_value = self.value;

        self.value = ((self.nodes[0].x - min_x) / (max_x - min_x)).clamp(0.0, 1.0);

        // Inform egui that the widget value has changed.
        if (self.value - previous_value).abs() > 0.00001 {
            response.mark_changed();
        }

        // -----------------------------------------------------
        // STRETCH CALCULATION
        // -----------------------------------------------------

        let min_node = self.nodes.iter().map(|n| n.x).fold(f32::INFINITY, f32::min);

        let max_node = self
            .nodes
            .iter()
            .map(|n| n.x)
            .fold(f32::NEG_INFINITY, f32::max);

        let stretch = (max_node - min_node).clamp(0.0, MAX_STRETCH);

        let thumb_width = BASE_SIZE + stretch;

        let thumb_height = (BASE_SIZE - stretch * 0.16).max(15.0);

        // -----------------------------------------------------
        // TRACK HOUSING
        // -----------------------------------------------------

        // Subtle track shadow.
        // painter.rect_filled(
        //     rect.translate(vec2(0.0, 2.0)),
        //     HEIGHT / 2.0,
        //     Color32::from_rgba_unmultiplied(0, 0, 0, 55),
        // );

        // Main capsule.
        painter.rect_filled(rect, HEIGHT / 2.0, TRACK_COLOR);

        // -----------------------------------------------------
        // METER GEOMETRY
        // -----------------------------------------------------

        let meter_x = rect.left() + PADDING;

        let meter_width = rect.width() - PADDING * 2.0;

        let filled_width = meter_width * self.value;

        // -----------------------------------------------------
        // STEREO METERS
        // -----------------------------------------------------

        for row in 0..2 {
            let meter_y = rect.center().y - 3.75;
            let y = meter_y + row as f32 * 5.0;

            self.draw_gradient_meter(painter, meter_x, y, meter_width, filled_width);

            // -------------------------------------------------
            // RED PEAK INDICATORS
            // -------------------------------------------------

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

        // -----------------------------------------------------
        // THUMB POSITION
        // -----------------------------------------------------

        let thumb_rect = Rect::from_center_size(
            pos2(
                rect.left() + self.nodes[0].x.clamp(min_x, max_x),
                rect.center().y,
            ),
            vec2(thumb_width, thumb_height),
        );

        let thumb_radius = thumb_height / 2.0;

        // -----------------------------------------------------
        // THUMB SHADOW
        // -----------------------------------------------------

        painter.rect_filled(
            thumb_rect.translate(vec2(0.0, 2.0)),
            thumb_radius,
            Color32::from_rgba_unmultiplied(0, 0, 0, 90),
        );

        // -----------------------------------------------------
        // THUMB BODY
        // -----------------------------------------------------

        painter.rect_filled(
            thumb_rect,
            thumb_radius,
            Color32::from_rgba_unmultiplied(216, 218, 221, 199),
        );

        // -----------------------------------------------------
        // THUMB BORDER
        // -----------------------------------------------------

        painter.rect_stroke(
            thumb_rect,
            thumb_radius,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 71)),
            egui::StrokeKind::Inside,
        );

        // -----------------------------------------------------
        // ANIMATION REPAINT
        // -----------------------------------------------------

        let animating = self.nodes.iter().any(|n| n.vx.abs() > 0.01)
            || (self.nodes[0].x - self.target_x).abs() > 0.05;

        if animating || response.dragged() || response.clicked() {
            ui.ctx().request_repaint();
        }

        response
    }

    // ---------------------------------------------------------
    // FIXED TIMESTEP PHYSICS
    // ---------------------------------------------------------

    fn simulate_step(&mut self, min_x: f32, max_x: f32) {
        // Use a critically damped spring in seconds so motion is stable and
        // smooth across display refresh rates.
        let dt = FIXED_STEP;

        // -----------------------------------------------------
        // NODE 1: LEADING EDGE
        // -----------------------------------------------------

        let head = &mut self.nodes[0];

        const SPRING: f32 = 420.0;
        const DAMPING: f32 = 40.987_804;

        let head_acceleration = (self.target_x - head.x) * SPRING - head.vx * DAMPING;
        head.vx += head_acceleration * dt;
        head.x += head.vx * dt;

        Self::clamp_node(head, min_x, max_x);

        // -----------------------------------------------------
        // NODES 2, 3, 4: FOLLOWING CHAIN
        // -----------------------------------------------------

        for i in 1..4 {
            let leader_x = self.nodes[i - 1].x;

            let follower = &mut self.nodes[i];

            let acceleration = (leader_x - follower.x) * SPRING - follower.vx * DAMPING;
            follower.vx += acceleration * dt;
            follower.x += follower.vx * dt;

            Self::clamp_node(follower, min_x, max_x);
        }

        // -----------------------------------------------------
        // SETTLE MICRO VELOCITIES
        // -----------------------------------------------------

        if (self.target_x - self.nodes[0].x).abs() < 0.05
            && (self.nodes[0].x - self.nodes[3].x).abs() < 0.05
            && self.nodes[3].vx.abs() < 0.05
        {
            for node in &mut self.nodes {
                node.x = self.target_x;
                node.vx = 0.0;
            }
        }
    }

    // ---------------------------------------------------------
    // STRAIGHT HORIZONTAL GRADIENT METER
    // ---------------------------------------------------------

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

        let segments = METER_SEGMENTS;

        for i in 0..segments {
            let t1 = i as f32 / segments as f32;
            let t2 = (i + 1) as f32 / segments as f32;

            let x1 = x + t1 * width;
            let x2 = x + t2 * width;

            if x1 >= x + filled_width {
                break;
            }

            // Clip final segment to current volume.
            let end_x = x2.min(x + filled_width);

            let color = Self::gradient_color((t1 + t2) * 0.5);

            // IMPORTANT:
            // Zero rounding keeps every segment perfectly
            // rectangular, creating a straight meter bar.
            // Slight overlap prevents tiny rendering gaps.

            painter.rect_filled(
                Rect::from_min_max(pos2(x1, y), pos2(end_x + 0.15, y + METER_HEIGHT)),
                0.0,
                color,
            );
        }
    }

    // ---------------------------------------------------------
    // CONTINUOUS 7:2:1 COLOR GRADIENT
    // ---------------------------------------------------------

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

                return Color32::from_rgb(
                    Self::lerp_channel(c1.r(), c2.r(), f),
                    Self::lerp_channel(c1.g(), c2.g(), f),
                    Self::lerp_channel(c1.b(), c2.b(), f),
                );
            }
        }

        Color32::RED
    }

    fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
        (a as f32 * (1.0 - t) + b as f32 * t)
            .round()
            .clamp(0.0, 255.0) as u8
    }

    // ---------------------------------------------------------
    // BOUNDARY CONSTRAINTS
    // ---------------------------------------------------------

    fn clamp_node(node: &mut Node, min: f32, max: f32) {
        if node.x < min {
            node.x = min;
            node.vx = 0.0;
        } else if node.x > max {
            node.x = max;
            node.vx = 0.0;
        }
    }

    // ---------------------------------------------------------
    // COMPATIBILITY WITH MAIN.RS
    // ---------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui) -> Response {
        self.show(ui)
    }

    // ---------------------------------------------------------
    // GET CURRENT VOLUME
    // ---------------------------------------------------------

    pub fn value(&self) -> f32 {
        self.value
    }
}

impl Default for VolumeGlider {
    fn default() -> Self {
        Self::new()
    }
}
