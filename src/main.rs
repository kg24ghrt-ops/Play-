use eframe::egui;
fn main() {
    let mut ui: egui::Ui = unsafe { std::mem::zeroed() };
    egui::CentralPanel::default().show_inside(&mut ui, |_| {});
}
