#![allow(dead_code)]
//! "正在保存工程"进度窗口。
//!
//! v5 保存包含全局排序 + 6 流 zstd 压缩，1.64 亿音符可达 30s+，
//! 必须给用户进度反馈。窗口不可交互（保存无法取消），用户关闭
//! 窗口只隐藏视图，保存照常完成。

use std::sync::{Arc, Mutex};

use eframe::egui;
use rust_i18n::t;
use yinhe_yin::YinProgressStage;

use crate::widgets::toast::model::ProgressSource;

/// 阶段 → 中文描述。
pub(crate) fn stage_label(stage: YinProgressStage) -> String {
    match stage {
        YinProgressStage::Collect => t!("dialog.saving.stage.collect").to_string(),
        YinProgressStage::Sort => t!("dialog.saving.stage.sort").to_string(),
        YinProgressStage::Compress => t!("dialog.saving.stage.compress").to_string(),
        YinProgressStage::Decompress => t!("dialog.saving.stage.decompress").to_string(),
        YinProgressStage::Rebuild => t!("dialog.saving.stage.rebuild").to_string(),
        YinProgressStage::Resort => t!("dialog.saving.stage.resort").to_string(),
    }
}

/// 保存进度的共享状态：poll 线程 drain channel 后写入，toast 渲染时 pull 读取。
pub(crate) type SharedSaveProgress = Arc<Mutex<Option<(YinProgressStage, f32)>>>;

/// 保存进度数据源：渲染时读共享状态，不再每帧拷贝文案。
pub(crate) struct SaveToastSource {
    pub state: SharedSaveProgress,
}

impl SaveToastSource {
    fn current(&self) -> Option<(YinProgressStage, f32)> {
        self.state.lock().ok().and_then(|s| *s)
    }
}

impl ProgressSource for SaveToastSource {
    fn title(&self) -> String {
        "正在保存".to_string()
    }
    fn message(&self) -> String {
        self.current()
            .map(|(stage, _)| stage_label(stage))
            .unwrap_or_else(|| "准备中…".to_string())
    }
    fn fraction(&self) -> f32 {
        self.current().map(|(_, f)| f).unwrap_or(0.0)
    }
    fn detail(&self) -> String {
        self.message()
    }
    fn cancel(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
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
                    fill: crate::theme::app_bg(),
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
                                    .desired_width(crate::theme::PROGRESS_BAR_WIDTH)
                                    .show_percentage(),
                            );
                            ui.label(
                                egui::RichText::new(stage_label(stage))
                                    .size(crate::theme::BODY_FONT),
                            );
                        });
                });
        },
    );

    ctx.request_repaint();
}
