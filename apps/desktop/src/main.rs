#[cfg(not(feature = "component-lab"))]
mod app;
#[cfg(not(feature = "component-lab"))]
mod platform;
#[cfg(not(feature = "component-lab"))]
mod views;

#[cfg(feature = "component-lab")]
mod lab;

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
        Box::new(|cc| Ok(Box::new(lab::component_lab::ComponentLabApp::new(cc)))),
    )
}
