#![windows_subsystem = "windows"]

mod dns;
mod engine;
mod muxd;

use eframe::egui;
use engine::{Engine, Level};
use std::sync::Arc;
use std::time::Duration;

struct App {
    engine: Arc<Engine>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(400));
        let s = self.engine.snapshot();
        let color = match s.level {
            Level::Off => egui::Color32::GRAY,
            Level::Wait => egui::Color32::from_rgb(255, 170, 0),
            Level::Bad => egui::Color32::from_rgb(255, 90, 80),
            Level::Ok => egui::Color32::from_rgb(60, 200, 90),
        };
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("WiFi Share Receiver");
            ui.add_space(10.0);
            egui::Grid::new("info").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
                ui.label("Power:");
                ui.label(if s.on { "ON" } else { "OFF" });
                ui.end_row();
                ui.label("Status:");
                ui.colored_label(color, s.status.as_str());
                ui.end_row();
                ui.label("iPhone:");
                ui.label(s.phone.as_str());
                ui.end_row();
                ui.label("Proxy address:");
                ui.monospace(s.proxy.as_str());
                ui.end_row();
                ui.label("Network adapter:");
                ui.label(s.adapter.as_str());
                ui.end_row();
                ui.label("Active connections:");
                ui.label(s.active.to_string());
                ui.end_row();
            });
            if !s.note.is_empty() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(255, 120, 80), s.note.as_str());
            }
            if !s.detail.is_empty() {
                ui.add_space(6.0);
                ui.small(format!("Details: {}", s.detail));
            }
            if s.need_install {
                ui.add_space(10.0);
                ui.separator();
                ui.label("Apple USB components (needed to talk to the iPhone):");
                ui.horizontal(|ui| {
                    if ui.button("Install / repair (iTunes via winget)").clicked() {
                        self.engine.install_apple();
                    }
                    if ui.button("Open Apple Devices (Microsoft Store)").clicked() {
                        self.engine.open_store();
                    }
                });
                ui.small("After installing, this window detects it automatically.");
            }
            ui.add_space(14.0);
            let label = if s.on { "Turn OFF" } else { "Turn ON" };
            if ui.add_sized([ui.available_width(), 42.0], egui::Button::new(label)).clicked() {
                self.engine.toggle();
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let engine = Arc::new(Engine::new());
    engine.start();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 460.0]).with_resizable(false),
        ..Default::default()
    };
    let e2 = engine.clone();
    let r = eframe::run_native(
        "WiFi Share Receiver",
        opts,
        Box::new(move |_cc| Ok(Box::new(App { engine: e2 }))),
    );
    engine.stop();
    r
}
