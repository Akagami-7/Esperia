#[derive(Clone, Copy, Debug)]
pub struct Typography {
    pub workspace_title: f32,
    pub panel_title: f32,
    pub label: f32,
    pub control_label: f32,
    pub numeric: f32,
    pub metadata: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            workspace_title: 18.0,
            panel_title: 13.0,
            label: 13.0,
            control_label: 12.0,
            numeric: 13.0,
            metadata: 10.0,
        }
    }
}
