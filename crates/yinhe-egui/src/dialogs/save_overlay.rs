//! "正在保存工程"进度窗口。
//!
//! v5 保存包含全局排序 + 6 流 zstd 压缩，1.64 亿音符可达 30s+，
//! 必须给用户进度反馈。窗口不可交互（保存无法取消），用户关闭
//! 窗口只隐藏视图，保存照常完成。

use eframe::egui;
use rust_i18n::t;
use yinhe_yin::YinProgressStage;

/// 阶段 → 中文描述。
pub(crate) fn stage_label(stage: YinProgressStage) -> String {
    match stage {
        YinProgressStage::Collect => t!("dialog.saving.stage.collect").to_string(),
        YinProgressStage::Sort => t!("dialog.saving.stage.sort").to_string(),
        YinProgressStage::Encode => t!("dialog.saving.stage.encode").to_string(),
        YinProgressStage::Compress => t!("dialog.saving.stage.compress").to_string(),
        YinProgressStage::Decompress => t!("dialog.saving.stage.decompress").to_string(),
        YinProgressStage::Rebuild => t!("dialog.saving.stage.rebuild").to_string(),
        YinProgressStage::Resort => t!("dialog.saving.stage.resort").to_string(),
    }
}

/// 保存进度窗口。`fraction` 是阶段内 0.0~1.0。
pub(crate) fn show_viewport(ctx: &egui::Context, stage: YinProgressStage, fraction: f32) {
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("save_overlay_dialog"),
        crate::chrome::dialog::viewport_builder(
            t!("dialog.saving.title").as_ref(),
            [380.0, 110.0],
            false,
        ),
        move |vctx, _class| {
            let mut close = false;
            egui::CentralPanel::default()
                .frame(egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.saving.title").as_ref(),
                        &mut close,
                    );
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 12,
                            right: 12,
                            top: 0,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_width(330.0)
                                    .show_percentage(),
                            );
                            ui.label(egui::RichText::new(stage_label(stage)).size(12.0));
                        });
                });
        },
    );

    ctx.request_repaint();
}
