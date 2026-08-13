//! AR 工程走带页：顶栏（主页/工程名/走带）+ 音轨面板 + GPU 音符视图。

use eframe::egui;
use yinhe_audio::spawn::AudioCommand;

use crate::app::{Page, YinheApp};
use crate::pages::transport;
use crate::ui_common::{fill_page_background, icon_text, show_toolbar};

impl YinheApp {
    /// AR 首页：顶栏（主页 + 工程名 + 走带）+ 音轨面板 + GPU 音符视图。
    pub(crate) fn ui_ar(&mut self, ui: &mut egui::Ui) {
        transport::update(self);
        // 每帧轮询后台加载结果（模型加载完成后留在 AR 页展示）。
        self.poll_midi_load();
        show_toolbar(ui, "ar_toolbar", self.safe_insets, |ui| {
            // 最左侧：主页按钮（进入菜单，文件夹/设置入口合并于此）。
            use egui_material_icons::icons::ICON_HOME;
            if ui
                .button(icon_text(ICON_HOME))
                .on_hover_text("主页")
                .clicked()
            {
                self.page = Page::Menu;
            }
            // 工程名。
            let title = self
                .model
                .as_ref()
                .map(|m| m.meta.name.clone())
                .unwrap_or_else(|| "未命名工程".to_string());
            ui.label(egui::RichText::new(title).strong());
            ui.separator();
            transport::bar(self, ui);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                let events = self.ar_view.ui(ui, self.safe_insets);
                for ev in events {
                    match ev {
                        crate::ar_view::ArEvent::EnterPr(track) => {
                            log::info!("AR: 点击轨道 {track}，进入钢琴卷帘");
                            self.page = Page::Pr;
                        }
                        crate::ar_view::ArEvent::SkipTracks(skip) => {
                            if let Some(a) = &self.audio {
                                a.handle.send(AudioCommand::SkipTracks { skip });
                            }
                        }
                    }
                }
            });
    }
}
