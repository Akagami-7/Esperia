#[cfg(not(feature = "component-lab"))]
mod app;

#[cfg(feature = "component-lab")]
#[path = "ui/mod.rs"]
mod ui;

#[cfg(not(feature = "component-lab"))]
fn main() -> eframe::Result<()> {
    app::run()
}

#[cfg(feature = "component-lab")]
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Esperia — UI Laboratory",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1_280.0, 800.0])
                .with_min_inner_size([800.0, 500.0])
                .with_title("Esperia — UI Laboratory"),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ui::component_lab::ComponentLabApp::new(cc)))),
    )
}
