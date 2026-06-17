use crate::file_loader::FileLoader;
use yinhe_editor_core::progress::StageStatus;

pub(crate) fn show(ui: &mut eframe::egui::Ui, loader: &FileLoader) {
    if !loader.is_loading() {
        return;
    }

    let progress = match loader.load_progress().lock() {
        Ok(p) => p.clone(),
        Err(_) => return,
    };
    if !progress.visible {
        return;
    }

    let screen_rect = ui.ctx().content_rect();
    ui.ctx()
        .layer_painter(eframe::egui::LayerId::new(
            eframe::egui::Order::Foreground,
            "loading_overlay".into(),
        ))
        .rect_filled(
            screen_rect,
            0.0,
            eframe::egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
        );

    eframe::egui::Window::new("正在加载")
        .order(eframe::egui::Order::Tooltip)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(
            eframe::egui::Align2::CENTER_CENTER,
            eframe::egui::Vec2::ZERO,
        )
        .show(ui.ctx(), |ui| {
            ui.set_max_width(380.0);
            for stage in &progress.stages {
                ui.horizontal(|ui| {
                    let icon = match stage.status {
                        StageStatus::Done => "✅",
                        StageStatus::Active => "⏳",
                        StageStatus::Pending => "⬜",
                    };
                    ui.label(icon);
                    ui.add(
                        eframe::egui::ProgressBar::new(stage.progress)
                            .desired_width(200.0)
                            .show_percentage(),
                    );
                    ui.label(eframe::egui::RichText::new(&stage.label).size(12.0));
                });
                if !stage.detail.is_empty() {
                    ui.label(
                        eframe::egui::RichText::new(&stage.detail)
                            .size(10.0)
                            .color(eframe::egui::Color32::GRAY),
                    );
                }
            }
        });
}
