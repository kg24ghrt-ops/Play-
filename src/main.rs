use eframe::{egui, NativeOptions};
use egui::ViewportBuilder;

struct App;

impl Default for App {
    fn default() -> Self {
        Self
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Dark background rgb(30,30,30)
        ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*ctx.style()).clone();
        style.visuals.window_fill = egui::Color32::from_rgb(30, 30, 30);
        style.visuals.panel_fill = egui::Color32::from_rgb(30, 30, 30);
        ctx.set_style(style);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Editor will go here");
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_title("NovaCibes Editor")
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "NovaCibes Editor",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
