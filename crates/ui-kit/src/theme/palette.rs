use egui::Color32;

/// Semantic graphite and violet colors shared by all Esperia UI components.
#[derive(Clone, Debug)]
pub struct Palette {
    pub canvas: Color32,
    pub shell: Color32,
    pub panel: Color32,
    pub panel_raised: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub control_pressed: Color32,
    pub glass: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_active: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub cyan: Color32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            canvas: Color32::from_rgb(17, 20, 26),
            shell: Color32::from_rgb(21, 26, 34),
            panel: Color32::from_rgb(26, 32, 41),
            panel_raised: Color32::from_rgb(34, 42, 53),
            control: Color32::from_rgb(40, 49, 62),
            control_hover: Color32::from_rgb(50, 61, 76),
            control_pressed: Color32::from_rgb(32, 39, 50),
            glass: Color32::from_rgba_unmultiplied(34, 42, 53, 236),
            border: Color32::from_rgb(52, 61, 73),
            border_strong: Color32::from_rgb(75, 86, 102),
            text_primary: Color32::from_rgb(237, 239, 244),
            text_secondary: Color32::from_rgb(166, 174, 188),
            text_muted: Color32::from_rgb(119, 129, 145),
            accent: Color32::from_rgb(172, 112, 255),
            accent_hover: Color32::from_rgb(194, 148, 255),
            accent_active: Color32::from_rgb(104, 62, 160),
            success: Color32::from_rgb(74, 204, 132),
            warning: Color32::from_rgb(236, 177, 77),
            danger: Color32::from_rgb(239, 82, 91),
            cyan: Color32::from_rgb(75, 195, 218),
        }
    }
}
