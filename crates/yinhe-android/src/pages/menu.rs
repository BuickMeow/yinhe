//! 菜单页（启动页）：左上角返回 AR；左侧歌曲卡片 + 本地打开；右侧设置。

use eframe::egui;

use crate::app::{Page, YinheApp};
use crate::file_picker;
use crate::ui_common::{fill_page_background, icon_text, show_toolbar};

impl YinheApp {
    /// 菜单页（启动页）：左上角返回 AR；左侧歌曲卡片 + 本地打开；右侧设置。
    pub(crate) fn ui_menu(&mut self, ui: &mut egui::Ui) {
        // 每帧轮询：选歌后加载完成自动进入 AR；音色/文件选择结果在此消费。
        self.poll_midi_load();
        self.poll_sf_load();
        if let Some(path) = file_picker::take_picked_path() {
            self.start_midi_load(&path);
        }
        // 顶栏：返回（回 AR）+ 标题，默认背景色 + 挖孔避让 + 对称内边距。
        show_toolbar(ui, "menu_toolbar", self.safe_insets, |ui| {
            use egui_material_icons::icons::ICON_ARROW_BACK;
            if ui
                .button(icon_text(ICON_ARROW_BACK))
                .on_hover_text("返回工程")
                .clicked()
            {
                self.page = Page::Ar;
            }
            ui.label(egui::RichText::new("菜单").strong());
        });
        // 左右分栏：左侧选歌，右侧设置。
        let [sl, st, sr, sb] = self.safe_insets;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let avail = ui.available_rect_before_wrap();
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                let inner = egui::Rect::from_min_max(
                    avail.min + egui::vec2(sl, st),
                    avail.max - egui::vec2(sr, sb),
                );
                if inner.width() <= 0.0 || inner.height() <= 0.0 {
                    return;
                }
                let left_w = (inner.width() * 0.55).clamp(280.0, 520.0);
                let left_rect = egui::Rect::from_min_max(
                    inner.min,
                    egui::pos2(inner.min.x + left_w, inner.max.y),
                );
                let right_rect = egui::Rect::from_min_max(
                    egui::pos2(inner.min.x + left_w + 12.0, inner.min.y),
                    inner.max,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                    self.menu_songs_ui(ui);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                    self.menu_settings_ui(ui);
                });
            });
    }

    /// 菜单左侧：歌曲卡片（测试曲目）+ 本地打开（SAF 文件选择器）。
    fn menu_songs_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("选择歌曲");
        ui.add_space(10.0);
        let card_w = ui.available_width();
        let cards = [
            ("小曲", "test.mid（链路验证）", crate::TEST_MIDI_PATH),
            ("大曲", "big.mid（性能测试）", crate::BIG_MIDI_PATH),
        ];
        for (title, desc, path) in cards {
            if ui
                .add_sized(
                    [card_w, 64.0],
                    egui::Button::new(egui::RichText::new(title).size(18.0).strong()),
                )
                .on_hover_text(desc)
                .clicked()
            {
                self.start_midi_load(path);
            }
            ui.label(
                egui::RichText::new(desc)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(8.0);
        }
        // 本地打开：SAF 系统文件选择器（MainActivity 桥）。
        if ui
            .add_sized(
                [card_w, 56.0],
                egui::Button::new(egui::RichText::new("本地打开 MIDI").size(16.0)),
            )
            .clicked()
        {
            file_picker::open_file_picker();
        }
        ui.add_space(10.0);
        // 加载进度/结果。
        if !self.midi_stats.is_empty() {
            ui.label(
                egui::RichText::new(&self.midi_stats).color(egui::Color32::from_rgb(140, 200, 255)),
            );
        }
    }

    /// 菜单右侧：设置（音色库 + 音频状态）。
    fn menu_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("设置");
        ui.add_space(10.0);
        ui.label(egui::RichText::new("音色库").strong());
        ui.label(&self.audio_status);
        if self.sf_load_start.is_some() {
            ui.label("音色加载中...");
        }
        if ui.button("重新加载音色库").clicked() {
            self.load_soundfont();
        }
        ui.separator();
        ui.label(egui::RichText::new("音频").strong());
        let sr = self.audio.as_ref().map(|a| a.sample_rate).unwrap_or(0);
        ui.label(format!("采样率: {sr} Hz"));
        let playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        ui.label(format!(
            "播放状态: {}",
            if playing { "播放中" } else { "停止" }
        ));
        if self.audio.is_none() && ui.button("初始化音频").clicked() {
            self.init_audio();
        }
    }
}
