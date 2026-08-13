//! PR 钢琴卷帘页：顶栏（返回 + 走带 + 轨道名）+ GPU 音符视图。

use eframe::egui;

use crate::app::{Page, Tool, YinheApp};
use crate::pages::transport;
use crate::ui_common::{fill_page_background, icon_text, show_toolbar};

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
            // 右侧编辑区：Track 名 → 撤销 → 重做 → 工具 → 量化。
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
                let q = self
                    .doc
                    .as_ref()
                    .map(|d| d.edit.quantize_pianoroll)
                    .unwrap_or(yinhe_editor_core::quantize::QuantizePreset::Fraction(1, 16));
                let (name_clicked, q_clicked) =
                    crate::ui_common::right_edit_area(ui, self, &name, q);
                if name_clicked {
                    self.track_list_open = !self.track_list_open;
                }
                if q_clicked {
                    self.pr_quantize_open = !self.pr_quantize_open;
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                fill_page_background(ui);
                let (tv, et, sel, q) = self
                    .doc
                    .as_ref()
                    .map(|d| {
                        (
                            d.edit.track_visible.clone(),
                            d.edit.editing_track,
                            d.edit.selected.clone(),
                            d.edit.quantize_pianoroll,
                        )
                    })
                    .unwrap_or_default();
                let events = self.pr_view.ui(
                    ui,
                    self.safe_insets,
                    &tv,
                    et,
                    self.tool == crate::app::Tool::Hand,
                    self.tool,
                    &sel,
                    q,
                );
                for ev in events {
                    self.handle_pr_event(ev);
                }
            });
        // 轨道显隐列表（点击顶栏右侧轨道名打开）。
        if self.track_list_open {
            self.track_list_ui(ui);
        }
        // 量化弹窗（PR 独立量化：quantize_pianoroll）。
        if self.pr_quantize_open {
            let ppq = self.doc.as_ref().map(|d| d.model().meta.ppq).unwrap_or(480);
            let current = self
                .doc
                .as_ref()
                .map(|d| d.edit.quantize_pianoroll)
                .unwrap_or(yinhe_editor_core::quantize::QuantizePreset::Fraction(1, 16));
            if let Some(q) = crate::ui_common::quantize_popup(ui.ctx(), "pr_quantize", ppq, current)
                && let Some(doc) = &mut self.doc
            {
                doc.edit.quantize_pianoroll = q;
            }
        }
        // 工具选择弹窗：屏幕中央、横向排列（选择/铅笔/橡皮/抓手）。
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

    /// 消费 PR 编辑事件：写 doc（选区/音符编辑）+ undo 事务 + 音频联动。
    fn handle_pr_event(&mut self, ev: crate::pr_view::PrEvent) {
        use yinhe_core::NoteEvent;
        use yinhe_editor_core::Selection;
        match ev {
            crate::pr_view::PrEvent::AddNote {
                start_tick,
                end_tick,
                key,
            } => {
                let Some(track) = self.doc.as_ref().and_then(|d| d.edit.editing_track) else {
                    return;
                };
                let vel = self
                    .doc
                    .as_ref()
                    .map(|d| d.edit.default_velocity(track))
                    .unwrap_or(100);
                self.with_undo("画音符", |doc| {
                    doc.add_note(
                        track,
                        NoteEvent {
                            id: 0,
                            start_tick,
                            end_tick,
                            key,
                            velocity: vel,
                        },
                    )
                });
            }
            crate::pr_view::PrEvent::RetuneNote {
                track,
                start_tick,
                key,
                delta_keys,
            } => {
                self.with_undo("改音高", |doc| {
                    doc.pencil_drag_note(&yinhe_editor_core::PencilNoteDrag::Move {
                        track,
                        start_tick,
                        key,
                        delta_ticks: 0,
                        delta_keys,
                    })
                });
            }
            crate::pr_view::PrEvent::MoveNotes {
                delta_ticks,
                delta_keys,
            } => {
                self.with_undo("移动音符", |doc| {
                    doc.move_selected_notes(delta_ticks, delta_keys)
                });
            }
            crate::pr_view::PrEvent::SelectNote { track, tick, key } => {
                if let Some(doc) = &mut self.doc {
                    let mut sel = Selection::default();
                    sel.add_rect_track(tick, tick + 1, key, key, track, track);
                    doc.edit.selected = sel;
                }
            }
            crate::pr_view::PrEvent::SelectRect { t0, t1, k0, k1 } => {
                let Some(track) = self.doc.as_ref().and_then(|d| d.edit.editing_track) else {
                    return;
                };
                if let Some(doc) = &mut self.doc {
                    let mut sel = Selection::default();
                    sel.add_rect_track(t0, t1, k0, k1, track, track);
                    doc.edit.selected = sel;
                }
            }
            crate::pr_view::PrEvent::EraseRect { t0, t1, k0, k1 } => {
                let Some(track) = self.doc.as_ref().and_then(|d| d.edit.editing_track) else {
                    return;
                };
                self.with_undo("擦除", |doc| {
                    let mut sel = Selection::default();
                    sel.add_rect_track(t0, t1, k0, k1, track, track);
                    doc.edit.selected = sel;
                    doc.delete_selected()
                });
            }
            crate::pr_view::PrEvent::EraseNote { track, tick, key } => {
                self.with_undo("擦除", |doc| {
                    let mut sel = Selection::default();
                    sel.add_rect_track(tick, tick + 1, key, key, track, track);
                    doc.edit.selected = sel;
                    doc.delete_selected()
                });
            }
            crate::pr_view::PrEvent::ClearSelection => {
                if let Some(doc) = &mut self.doc {
                    doc.edit.selected.clear();
                }
            }
        }
    }
}
