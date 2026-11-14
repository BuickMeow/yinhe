use std::sync::{Arc, atomic::AtomicBool};

use eframe::egui;
use egui_material_icons::icons::{ICON_FOLDER_OPEN, ICON_PAUSE, ICON_PLAY_ARROW, ICON_STOP};

use super::super::kind::ToastKind;
use super::super::model::{Notification, ToastAction, ToastActionKind};
use super::Notifications;

/// 标题栏通知条的固定高度（标题栏 32px，上下各留 5px）。
const BAR_H: f32 = 22.0;
const MIN_W: f32 = 160.0;
const MAX_W: f32 = 480.0;
const SIDE_PAD: f32 = 8.0;
const TASKS_W: f32 = 340.0;

/// 一帧的渲染快照：避免在闭包里与 `&mut self` 打架。
struct Row {
    id: u64,
    kind: ToastKind,
    title: String,
    message: String,
    progress: Option<f32>,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
    action: Option<ToastAction>,
    cancelling: bool,
    running: bool,
}

impl Row {
    fn new(n: &Notification) -> Self {
        let (title, message, progress, _detail) = n.resolve();
        Self {
            id: n.id,
            kind: n.kind,
            title,
            message,
            progress,
            cancel: n.cancel_flag(),
            pause: n.pause_flag(),
            action: n.action.clone(),
            cancelling: n.cancelling,
            running: n.source.is_some(),
        }
    }

    fn text(&self) -> String {
        if self.message.is_empty() {
            self.title.clone()
        } else {
            format!("{} · {}", self.title, self.message)
        }
    }
}

fn perform_action(action: &ToastAction) {
    match &action.kind {
        ToastActionKind::RevealInFolder(path) => crate::platform::open_containing_folder(path),
    }
}

impl Notifications {
    fn primary_row(&self) -> Option<Row> {
        self.primary
            .and_then(|id| self.items.iter().find(|n| n.id == id && !n.dismissed))
            .map(Row::new)
    }

    fn task_rows(&self) -> Vec<Row> {
        self.items
            .iter()
            .filter(|n| n.source.is_some() && !n.dismissed)
            .map(Row::new)
            .collect()
    }

    /// 在标题栏可用区 `area`（工程 tab 右缘 → 右侧窗口按钮之前）内绘制通知。
    /// 返回实际占用的矩形；没有通知时返回 `Rect::NOTHING`。
    pub fn show_title_bar(&mut self, ui: &mut egui::Ui, area: egui::Rect) -> egui::Rect {
        let now = std::time::Instant::now();
        self.tick(now);
        // 进行中的任务需要持续刷新百分比；结果通知到点后需要一次重绘来消失。
        let next_repaint = self
            .items
            .iter()
            .filter_map(|n| {
                if n.source.is_some() {
                    Some(now + std::time::Duration::from_millis(50))
                } else {
                    n.expires_at
                }
            })
            .min();
        if let Some(at) = next_repaint {
            ui.ctx()
                .request_repaint_after(at.saturating_duration_since(now));
        }
        let Some(row) = self.primary_row() else {
            self.tasks_open = false;
            return egui::Rect::NOTHING;
        };
        let tasks = self.task_rows();
        let task_count = tasks.len();
        if task_count == 0 {
            self.tasks_open = false;
        }
        let extra = if row.running {
            task_count.saturating_sub(1)
        } else {
            task_count
        };

        let w = area.width().min(MAX_W);
        if w < MIN_W {
            return egui::Rect::NOTHING;
        }
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.min.x, area.center().y - BAR_H / 2.0),
            egui::vec2(w, BAR_H),
        );

        let hovered = ui.rect_contains_pointer(rect);
        if hovered {
            ui.painter()
                .rect_filled(rect, 4.0, crate::theme::hover_color(crate::theme::app_bg()));
            crate::widgets::hint::set(ui.ctx(), rust_i18n::t!("hint.title_bar.notification"));
        }

        // 右侧控件宽度：stop / action / +N / 百分比。
        let stop_w = (row.running && row.cancel.is_some()).then_some(18.0);
        let action_w = row.action.is_some().then_some(18.0);
        let extra_w = (extra > 0).then_some(26.0);
        let pct_w = (row.progress.is_some() && row.running).then_some(34.0);
        let ctrl_w = [stop_w, action_w, extra_w, pct_w]
            .into_iter()
            .flatten()
            .sum::<f32>()
            + if stop_w.is_some() || action_w.is_some() || extra_w.is_some() {
                6.0
            } else {
                0.0
            };
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + SIDE_PAD, rect.min.y),
            egui::pos2(
                rect.max.x - SIDE_PAD - ctrl_w - pct_w.unwrap_or(0.0),
                rect.max.y,
            ),
        );
        let ctrl_rect = egui::Rect::from_min_max(
            egui::pos2(text_rect.max.x, rect.min.y),
            egui::pos2(rect.max.x - SIDE_PAD, rect.max.y),
        );

        // ── 左侧：图标 + 标题/详情（单行截断）──
        if text_rect.width() > 24.0 {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(text_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(row.kind.icon().codepoint)
                                .family(row.kind.icon().font_family())
                                .size(crate::theme::ICON_FONT_SM)
                                .color(row.kind.color()),
                        )
                        .selectable(false),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(row.text())
                                .size(crate::theme::SMALL_FONT)
                                .color(crate::theme::text_primary()),
                        )
                        .truncate()
                        .selectable(false),
                    );
                },
            );
        }

        // ── 右侧：百分比 + 取消/多任务/操作 ──
        let mut toggle_tasks = false;
        let mut dismiss = false;
        let mut run_action = false;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(ctrl_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                if let Some(cancel) = &row.cancel
                    && row.running
                {
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        ICON_STOP.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT_SM, ICON_STOP.font_family()),
                        crate::theme::text_muted(),
                        false,
                        rust_i18n::t!("hint.toast.cancel").as_ref(),
                    );
                    crate::widgets::hint::hover(
                        ui.ctx(),
                        &resp,
                        rust_i18n::t!("hint.toast.cancel"),
                    );
                    if resp.clicked() && !row.cancelling {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        dismiss = true;
                    }
                }
                if let Some(action) = &row.action {
                    let icon = action.icon.unwrap_or(ICON_FOLDER_OPEN);
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        icon.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                        crate::theme::text_muted(),
                        false,
                        &action.label,
                    );
                    crate::widgets::hint::hover(ui.ctx(), &resp, &action.label);
                    if resp.clicked() {
                        run_action = true;
                    }
                }
                if extra > 0 {
                    let resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("+{extra}"))
                                .size(crate::theme::SMALL_LABEL_FONT)
                                .color(crate::theme::text_muted()),
                        )
                        .sense(egui::Sense::click())
                        .selectable(false),
                    );
                    crate::widgets::hint::hover(
                        ui.ctx(),
                        &resp,
                        rust_i18n::t!("hint.title_bar.notification_tasks"),
                    );
                    if resp.clicked() {
                        toggle_tasks = true;
                    }
                }
                if let Some(p) = pct_w.and(row.progress) {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{:.0}%", p * 100.0))
                                .size(crate::theme::SMALL_LABEL_FONT)
                                .color(crate::theme::text_muted()),
                        )
                        .selectable(false),
                    );
                }
            },
        );

        if toggle_tasks {
            self.tasks_open = !self.tasks_open;
        }
        if dismiss {
            self.dismiss(row.id);
        }
        if run_action && let Some(action) = &row.action {
            perform_action(action);
        }

        // 进度条贴底边（只在进行中显示）。
        if row.running && row.progress.is_some_and(|p| p < 0.999) {
            let y = rect.max.y - 1.5;
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.min.x + 4.0, y),
                egui::pos2(rect.max.x - 4.0, y + 1.5),
            );
            ui.painter()
                .rect_filled(bar, 0.0, row.kind.color().gamma_multiply(0.85));
        }

        if self.tasks_open && task_count > 0 {
            self.show_tasks_popover(ui, rect);
        }

        // 仅图标在极窄时可点：让窗口拖动区让位由 title_bar 处理。
        rect
    }

    fn show_tasks_popover(&mut self, ui: &egui::Ui, anchor: egui::Rect) {
        let rows = self.task_rows();
        if rows.is_empty() {
            self.tasks_open = false;
            return;
        }
        let mut cancel_id = None;
        let area = egui::Area::new(egui::Id::unique("yinhe_notify_tasks"))
            .fixed_pos(egui::pos2(anchor.min.x, anchor.max.y + 4.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(crate::theme::control_bg())
                    .stroke(egui::Stroke::new(
                        1.0,
                        crate::theme::line_fg().gamma_multiply(0.35),
                    ))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(TASKS_W);
                        for (i, row) in rows.iter().enumerate() {
                            if i > 0 {
                                ui.add_space(6.0);
                            }
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(row.kind.icon().codepoint)
                                            .family(row.kind.icon().font_family())
                                            .size(crate::theme::ICON_FONT_SM)
                                            .color(row.kind.color()),
                                    )
                                    .selectable(false),
                                );
                                ui.add_space(6.0);
                                ui.add_sized(
                                    egui::vec2(TASKS_W - 96.0, 18.0),
                                    egui::Label::new(
                                        egui::RichText::new(row.text())
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_primary()),
                                    )
                                    .truncate()
                                    .selectable(false),
                                );
                                if let Some(p) = row.progress {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("{:.0}%", p * 100.0))
                                                .size(crate::theme::SMALL_LABEL_FONT)
                                                .color(crate::theme::text_muted()),
                                        )
                                        .selectable(false),
                                    );
                                }
                                if let Some(pause) = &row.pause {
                                    let paused = pause.load(std::sync::atomic::Ordering::Relaxed);
                                    let icon = if paused { ICON_PLAY_ARROW } else { ICON_PAUSE };
                                    let resp = crate::widgets::hover::hover_button(
                                        ui,
                                        icon.codepoint,
                                        egui::FontId::new(
                                            crate::theme::ICON_FONT_SM,
                                            icon.font_family(),
                                        ),
                                        crate::theme::text_muted(),
                                        false,
                                        rust_i18n::t!("hint.toast.pause").as_ref(),
                                    );
                                    if resp.clicked() {
                                        pause.store(!paused, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                                if let Some(cancel) = &row.cancel {
                                    let resp = crate::widgets::hover::hover_button(
                                        ui,
                                        ICON_STOP.codepoint,
                                        egui::FontId::new(
                                            crate::theme::ICON_FONT_SM,
                                            ICON_STOP.font_family(),
                                        ),
                                        crate::theme::text_muted(),
                                        false,
                                        rust_i18n::t!("hint.toast.cancel").as_ref(),
                                    );
                                    if resp.clicked() && !row.cancelling {
                                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                        cancel_id = Some(row.id);
                                    }
                                }
                            });
                        }
                    });
            });

        if let Some(id) = cancel_id {
            self.dismiss(id);
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let clicked_outside = ui.input(|i| i.pointer.any_pressed())
            && pointer.is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p));
        if clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.tasks_open = false;
        }
    }
}
