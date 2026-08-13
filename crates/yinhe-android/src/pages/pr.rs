//! PR 钢琴卷帘页：顶栏（返回 + 走带 + 轨道名）+ GPU 音符视图。

use eframe::egui;

use crate::app::{Page, Tool, YinheApp};
use crate::pages::transport;
use crate::ui_common::{fill_page_background, icon_text, right_side_button, show_toolbar};

impl YinheApp {
    /// PR 钢琴卷帘页：顶部工具条（返回 + 走带控制 + 轨道名）+ 视图。
    pub(crate) fn ui_pr(&mut self, ui: &mut egui::Ui) {
        transport::update(self);
        show_toolbar(ui, "pr_toolbar", self.safe_insets, |ui| {
            use egui_material_icons::icons::ICON_ARROW_BACK;
            if ui
                .button(icon_text(ICON_ARROW_BACK))
                .on_hover_text("返回")
                .clicked()
            {
                self.page = Page::Ar;
            }
            ui.separator();
            transport::bar(self, ui);
            // 右侧：当前编辑轨名称（过长截断），点击弹轨道显隐列表。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let name = self
                    .doc
                    .as_ref()
                    .and_then(|d| {
                        let t = d.edit.editing_track?;
                        d.model().tracks.get(t as usize)
                    })
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Track 1".to_string());
                if right_side_button(ui, &name).clicked() {
                    self.track_list_open = !self.track_list_open;
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                let (tv, et) = self
                    .doc
                    .as_ref()
                    .map(|d| (d.edit.track_visible.clone(), d.edit.editing_track))
                    .unwrap_or_default();
                self.pr_view.ui(ui, self.safe_insets, &tv, et);
            });
        // 轨道显隐列表（点击顶栏右侧轨道名打开）。
        if self.track_list_open {
            self.track_list_ui(ui);
        }
        // 工具选择弹窗：屏幕中央、横向排列（选择/铅笔/橡皮）。
        if self.tool_picker_open {
            egui::Window::new("工具")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        for t in Tool::ALL {
                            let icon = t.icon();
                            let text =
                                egui::RichText::new(format!("{}\n{}", icon.codepoint, t.name()))
                                    .family(icon.font_family())
                                    .size(26.0)
                                    .text_style(egui::TextStyle::Body);
                            if ui.selectable_label(self.tool == t, text).clicked() {
                                self.tool = t;
                                self.tool_picker_open = false;
                            }
                        }
                    });
                });
        }
    }

    /// 轨道显隐列表弹窗："全选" + 每轨勾选（多选）。
    /// 只控制显示/隐藏，不改正在编辑的轨道——编辑轨始终可见、不可取消勾选。
    /// 显隐变化走 GPU cull 掩码（upload_track_mask），无需重建音符。
    fn track_list_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            return;
        };
        let names: Vec<&str> = doc.model().tracks.iter().map(|t| t.name.as_str()).collect();
        let editing = doc.edit.editing_track;
        let n = names.len();
        // 弹窗内只借用 doc 字段（track 名已预取），避免闭包捕获整 self。
        let mut new_visible: Option<Vec<bool>> = None;
        egui::Window::new("显示轨道")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ui.ctx(), |ui| {
                let visible = self.doc.as_ref().map_or(&[][..], |d| &d.edit.track_visible);
                let mut all = visible.iter().all(|&v| v);
                if ui.checkbox(&mut all, format!("全选（{n} 轨）")).changed() {
                    new_visible = Some(vec![true; n]);
                }
                ui.separator();
                let max_h = ui
                    .ctx()
                    .input(|i| i.raw.screen_rect.map_or(600.0, |r| r.height() * 0.5))
                    .max(120.0);
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .show(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            let is_editing = Some(i as u16) == editing;
                            let mut vis = visible.get(i).copied().unwrap_or(true);
                            let label = if is_editing {
                                format!("{name}（编辑中，不可隐藏）")
                            } else {
                                (*name).to_string()
                            };
                            let changed = if is_editing {
                                // 编辑轨：勾选态固定为可见，控件禁用防误触。
                                ui.add_enabled(false, egui::Checkbox::new(&mut vis, label))
                                    .changed()
                            } else {
                                ui.checkbox(&mut vis, label).changed()
                            };
                            if changed && !is_editing {
                                let mut v = new_visible.take().unwrap_or_else(|| visible.to_vec());
                                v[i] = vis;
                                new_visible = Some(v);
                            }
                        }
                    });
            });
        if let Some(v) = new_visible
            && let Some(doc) = &mut self.doc
        {
            doc.edit.track_visible = v;
        }
    }
}
