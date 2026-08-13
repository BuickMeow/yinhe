//! AR 工程走带页：顶栏（主页/走带/工程名）+ 音轨面板 + GPU 音符视图。

use std::sync::Arc;

use eframe::egui;
use yinhe_audio::spawn::AudioCommand;

use crate::app::{Page, YinheApp};
use crate::pages::transport;
use crate::ui_common::{fill_page_background, icon_text, right_side_button, show_toolbar};

impl YinheApp {
    /// AR 首页：顶栏（主页 + 走带 + 工程名）+ 音轨面板 + GPU 音符视图。
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
            ui.separator();
            transport::bar(self, ui);
            // 右侧：工程名（过长截断），点击弹工程设置（同桌面端 project_info）。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let title = self
                    .doc
                    .as_ref()
                    .map(|d| d.model().meta.name.clone())
                    .unwrap_or_else(|| "未命名工程".to_string());
                if right_side_button(ui, &title).clicked() {
                    self.project_settings_open = !self.project_settings_open;
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                let overrides: Vec<yinhe_editor_core::TrackOverride> = self
                    .doc
                    .as_ref()
                    .map(|d| d.edit.track_overrides.clone())
                    .unwrap_or_default();
                let events = self.ar_view.ui(ui, self.safe_insets, &overrides);
                for ev in events {
                    match ev {
                        crate::ar_view::ArEvent::EnterPr(track) => {
                            log::info!("AR: 点击轨道 {track}，进入钢琴卷帘");
                            // 记录编辑轨：PR 顶栏右侧显示其名称，且该轨不可隐藏。
                            if let Some(doc) = &mut self.doc {
                                doc.edit.editing_track = Some(track);
                            }
                            self.page = Page::Pr;
                        }
                        crate::ar_view::ArEvent::ToggleMute(track) => {
                            self.toggle_mute(track);
                        }
                        crate::ar_view::ArEvent::ToggleSolo(track) => {
                            self.toggle_solo(track);
                        }
                    }
                }
            });
        // 工程设置弹窗（点击顶栏右侧工程名打开）。
        if self.project_settings_open {
            self.project_settings_ui(ui);
        }
    }

    /// 静音切换：写入 doc.edit.track_overrides 并同步音频 skip mask。
    fn toggle_mute(&mut self, track: u16) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        if let Some(ov) = doc.edit.track_overrides.get_mut(track as usize) {
            ov.muted = !ov.muted;
        }
        self.send_skip_mask();
    }

    /// 独奏切换：写入 doc.edit.track_overrides 并同步音频 skip mask。
    fn toggle_solo(&mut self, track: u16) {
        let Some(doc) = &mut self.doc else {
            return;
        };
        if let Some(ov) = doc.edit.track_overrides.get_mut(track as usize) {
            ov.soloed = !ov.soloed;
        }
        self.send_skip_mask();
    }

    /// 把当前 M/S 状态（compute_skip_mask）发送给音频引擎。
    fn send_skip_mask(&self) {
        let (Some(doc), Some(audio)) = (&self.doc, &self.audio) else {
            return;
        };
        let skip = doc.compute_skip_mask();
        audio.handle.send(AudioCommand::SkipTracks { skip });
    }

    /// 工程设置弹窗：与桌面端 project_info 面板对齐（工程名/艺术家/描述可编辑，
    /// PPQ/压缩等级只读——修改 PPQ 需重排音符，移动端暂不支持）。
    /// 改 meta 用 Arc::make_mut：大块数据（音符桶/轨道）都是 Arc 浅拷贝，
    /// 只有 meta 分叉，代价可忽略，且音频引擎/视图继续用旧 Arc 不受影响。
    fn project_settings_ui(&mut self, ui: &mut egui::Ui) {
        use yinhe_editor_core::history::{
            begin_edit, commit_artist, commit_description, commit_project_name,
        };

        egui::Window::new("工程设置")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ui.ctx(), |ui| {
                let Some(doc) = &mut self.doc else {
                    ui.label("未加载工程");
                    return;
                };
                let label = |ui: &mut egui::Ui, s: &str| {
                    ui.label(
                        egui::RichText::new(s)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                };
                // 与桌面端 project_info 相同的编辑模式：gained_focus 记录旧值，
                // changed 实时写 model，lost_focus 时 commit（变才 push undo）。
                label(ui, "工程名");
                let mut name = doc.data.model.meta.name.clone();
                let resp = ui.add_sized(
                    egui::vec2(ui.available_width(), 24.0),
                    egui::TextEdit::singleline(&mut name).hint_text("未命名工程"),
                );
                if resp.gained_focus() {
                    begin_edit(
                        &mut doc.edit.pending_edits,
                        resp.id.value(),
                        &doc.data.model.meta.name,
                    );
                }
                if resp.changed() {
                    let model = Arc::make_mut(&mut doc.data.model);
                    model.meta.name = name.clone();
                    // SMF 标准：track 0 name = song title，同步更新。
                    if let Some(track) = model.tracks.get_mut(0) {
                        Arc::make_mut(track).name = name.clone();
                    }
                }
                if resp.lost_focus() {
                    let name = doc.data.model.meta.name.clone();
                    commit_project_name(doc, resp.id.value(), &name);
                }
                ui.add_space(6.0);
                label(ui, "艺术家");
                let mut artist = doc.data.model.meta.artist.clone();
                let resp = ui.add_sized(
                    egui::vec2(ui.available_width(), 24.0),
                    egui::TextEdit::singleline(&mut artist),
                );
                if resp.gained_focus() {
                    begin_edit(
                        &mut doc.edit.pending_edits,
                        resp.id.value(),
                        &doc.data.model.meta.artist,
                    );
                }
                if resp.changed() {
                    Arc::make_mut(&mut doc.data.model).meta.artist = artist;
                }
                if resp.lost_focus() {
                    let artist = doc.data.model.meta.artist.clone();
                    commit_artist(doc, resp.id.value(), &artist);
                }
                ui.add_space(6.0);
                label(ui, "描述");
                let mut desc = doc.data.model.meta.description.clone();
                let resp = ui.add_sized(
                    egui::vec2(ui.available_width(), 56.0),
                    egui::TextEdit::multiline(&mut desc),
                );
                if resp.gained_focus() {
                    begin_edit(
                        &mut doc.edit.pending_edits,
                        resp.id.value(),
                        &doc.data.model.meta.description,
                    );
                }
                if resp.changed() {
                    Arc::make_mut(&mut doc.data.model).meta.description = desc;
                }
                if resp.lost_focus() {
                    let desc = doc.data.model.meta.description.clone();
                    commit_description(doc, resp.id.value(), &desc);
                }
                ui.add_space(6.0);
                ui.separator();
                let meta = &doc.data.model.meta;
                ui.label(format!("PPQ：{}（修改需重排音符，暂不支持）", meta.ppq));
                ui.label(format!("压缩等级：{}", meta.compression_level));
                ui.add_space(8.0);
                if ui.button("完成").clicked() {
                    self.project_settings_open = false;
                }
            });
    }
}
