use std::sync::{Arc, atomic::AtomicBool};

use eframe::egui;
use egui_material_icons::icons::{ICON_FOLDER_OPEN, ICON_PAUSE, ICON_PLAY_ARROW, ICON_STOP};

use super::super::kind::ToastKind;
use super::super::model::{Notification, ToastAction, ToastActionKind};
use super::Notifications;

/// 通知条高度（标题栏 32px，上下各留 3px）。
const BAR_H: f32 = 26.0;
/// 左侧全高进度条的宽度（要放得下 100%）。
const PROGRESS_W: f32 = 46.0;
const MIN_W: f32 = 132.0;
const MAX_W: f32 = 360.0;
const PAD_X: f32 = 8.0;
const BTN_W: f32 = 18.0;
const TASKS_W: f32 = 300.0;
const TASKS_ROW_H: f32 = 24.0;

/// 一帧的渲染快照：避免在绘制闭包里与 `&mut self` 打架。
struct Row {
    id: u64,
    kind: ToastKind,
    title: String,
    progress: Option<f32>,
    running: bool,
    cancelling: bool,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
    action: Option<ToastAction>,
}

impl Row {
    fn new(n: &Notification) -> Self {
        let paused = n
            .pause_flag()
            .is_some_and(|p| p.load(std::sync::atomic::Ordering::Relaxed));
        let title = if n.cancelling {
            rust_i18n::t!("toast.cancelling").to_string()
        } else if let Some(s) = &n.source {
            if paused {
                rust_i18n::t!("toast.paused").to_string()
            } else {
                s.title()
            }
        } else {
            n.title.clone()
        };
        Self {
            id: n.id,
            kind: n.kind,
            title,
            progress: n.snapshot_fraction(),
            running: n.source.is_some(),
            cancelling: n.cancelling,
            cancel: n.cancel_flag(),
            pause: n.pause_flag(),
            action: n.action.clone(),
        }
    }
}

fn perform_action(action: &ToastAction) {
    match &action.kind {
        ToastActionKind::RevealInFolder(path) => crate::platform::open_containing_folder(path),
    }
}

/// 单行截断文本：`max_rows = 1` + 省略号，绝不产生第二行。
fn one_line(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_w.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// 通知条右侧图标按钮：自绘 + `ui.interact`，不参与布局。
#[allow(clippy::too_many_arguments)]
fn draw_button(
    ui: &mut egui::Ui,
    cursor_x: &mut f32,
    center_y: f32,
    icon: &str,
    font: egui::FontId,
    color: egui::Color32,
    hint: &str,
    id_salt: &str,
) -> egui::Response {
    let rect = egui::Rect::from_min_size(
        egui::pos2(*cursor_x - BTN_W, center_y - BTN_W / 2.0),
        egui::vec2(BTN_W, BTN_W),
    );
    *cursor_x -= BTN_W + 4.0;
    let resp = ui.interact(
        rect,
        ui.scope_id().with((id_salt, rect.min.x as i32)),
        egui::Sense::click(),
    );
    let c = if resp.hovered() {
        crate::theme::text_primary()
    } else {
        color
    };
    ui.painter()
        .text(rect.center(), egui::Align2::CENTER_CENTER, icon, font, c);
    crate::widgets::hint::hover(ui.ctx(), &resp, hint);
    resp
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

    /// 在歌曲标题右侧的可用区 `area` 内绘制一行状态。
    /// 只用 painter 自绘，不注册布局控件，不会撑高标题栏。
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

        let font = egui::FontId::proportional(crate::theme::BODY_FONT);
        let text_color = if row.running {
            crate::theme::text_primary()
        } else {
            match row.kind {
                ToastKind::Warning => crate::theme::warning_gold(),
                _ => crate::theme::text_secondary(),
            }
        };
        let text_w = ui
            .painter()
            .layout_no_wrap(row.title.clone(), font.clone(), text_color)
            .size()
            .x;
        let has_progress = row.progress.is_some();
        let progress_w = if has_progress { PROGRESS_W + 8.0 } else { 0.0 };
        let extra = if row.running {
            task_count.saturating_sub(1)
        } else {
            task_count
        };
        let btn_count = usize::from(row.running && row.cancel.is_some())
            + usize::from(row.running && row.pause.is_some())
            + usize::from(row.action.is_some())
            + usize::from(extra > 0);
        let btn_w = if btn_count > 0 {
            btn_count as f32 * (BTN_W + 4.0)
        } else {
            0.0
        };
        if area.width() < MIN_W {
            return egui::Rect::NOTHING;
        }
        let w = (PAD_X * 2.0 + progress_w + text_w + btn_w)
            .clamp(MIN_W, MAX_W)
            .min(area.width());
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

        // 左侧全高进度条：淡强调色底 + 实心进度 + 百分比居中。
        let text_left = if let Some(p) = row.progress {
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + 4.0, rect.min.y + 2.0),
                egui::vec2(PROGRESS_W, rect.height() - 4.0),
            );
            let accent = match row.kind {
                ToastKind::Warning => crate::theme::warning_gold(),
                ToastKind::Error => crate::theme::danger_text(),
                _ => crate::theme::accent_active(),
            };
            ui.painter()
                .rect_filled(bar_rect, 4.0, accent.gamma_multiply(0.12));
            let fg = egui::Rect::from_min_size(
                bar_rect.min,
                egui::vec2(bar_rect.width() * p.clamp(0.0, 1.0), bar_rect.height()),
            );
            if fg.width() > 0.0 {
                ui.painter()
                    .rect_filled(fg, 4.0, accent.gamma_multiply(0.35));
            }
            ui.painter().text(
                bar_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{:.0}%", p.clamp(0.0, 1.0) * 100.0),
                egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
                crate::theme::text_primary(),
            );
            bar_rect.max.x + 8.0
        } else {
            rect.min.x + PAD_X
        };

        let text_right = rect.max.x - PAD_X - btn_w;
        let galley = one_line(
            ui,
            &row.title,
            font,
            text_color,
            (text_right - text_left).max(0.0),
        );
        let text_y = rect.min.y + (BAR_H - galley.size().y) / 2.0;
        ui.painter()
            .galley(egui::pos2(text_left, text_y), galley, text_color);
        // 自绘文本不在 AccessKit 树里：给文本区注册一个 Label 名称（供测试/读屏）。
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(text_left, rect.min.y),
            egui::pos2(text_right.max(text_left + 1.0), rect.max.y),
        );
        let label_resp = ui.interact(
            label_rect,
            ui.scope_id().with(("notify_label", row.id)),
            egui::Sense::hover(),
        );
        label_resp
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, row.title.clone()));

        // 右侧按钮：从右到左依次是中止、暂停、操作、+N。
        let mut cursor_x = rect.max.x - PAD_X;
        let mut dismiss = false;
        let mut run_action = false;
        let mut toggle_tasks = false;

        if let Some(cancel) = &row.cancel.filter(|_| row.running) {
            let resp = draw_button(
                ui,
                &mut cursor_x,
                rect.center().y,
                ICON_STOP.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT_SM, ICON_STOP.font_family()),
                crate::theme::text_muted(),
                rust_i18n::t!("hint.toast.cancel").as_ref(),
                "stop",
            );
            if resp.clicked() && !row.cancelling {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                dismiss = true;
            }
        }
        if let Some(pause) = &row.pause.filter(|_| row.running) {
            let paused = pause.load(std::sync::atomic::Ordering::Relaxed);
            let icon = if paused { ICON_PLAY_ARROW } else { ICON_PAUSE };
            let resp = draw_button(
                ui,
                &mut cursor_x,
                rect.center().y,
                icon.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                crate::theme::text_muted(),
                rust_i18n::t!("hint.toast.pause").as_ref(),
                "pause",
            );
            if resp.clicked() {
                pause.store(!paused, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(action) = &row.action {
            let icon = action.icon.unwrap_or(ICON_FOLDER_OPEN);
            let resp = draw_button(
                ui,
                &mut cursor_x,
                rect.center().y,
                icon.codepoint,
                egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                crate::theme::text_muted(),
                &action.label,
                "action",
            );
            if resp.clicked() {
                run_action = true;
            }
        }
        if extra > 0 {
            let text = format!("+{extra}");
            let font = egui::FontId::proportional(crate::theme::SMALL_FONT);
            let galley = ui
                .painter()
                .layout_no_wrap(text, font, crate::theme::text_muted());
            let rect = egui::Rect::from_min_size(
                egui::pos2(cursor_x - BTN_W, rect.center().y - BTN_W / 2.0),
                egui::vec2(BTN_W, BTN_W),
            );
            let resp = ui.interact(
                rect,
                ui.scope_id().with(("notify_tasks", rect.min.x as i32)),
                egui::Sense::click(),
            );
            let c = if resp.hovered() {
                crate::theme::text_primary()
            } else {
                crate::theme::text_muted()
            };
            ui.painter().galley(
                egui::pos2(
                    rect.center().x - galley.size().x / 2.0,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                c,
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

        if toggle_tasks {
            self.tasks_open = !self.tasks_open;
        }
        if dismiss {
            self.dismiss(row.id);
        }
        if run_action && let Some(action) = &row.action {
            perform_action(action);
        }

        if self.tasks_open && task_count > 0 {
            self.show_tasks_popover(ui, rect);
        }

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
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                let h = 8.0 + rows.len() as f32 * TASKS_ROW_H;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(TASKS_W, h), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 8.0, crate::theme::control_bg());
                for (i, row) in rows.iter().enumerate() {
                    let top = rect.min.y + 4.0 + i as f32 * TASKS_ROW_H;
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x + 8.0, top),
                        egui::vec2(TASKS_W - 16.0, TASKS_ROW_H),
                    );
                    let text_color = if row.running {
                        crate::theme::text_primary()
                    } else {
                        crate::theme::text_secondary()
                    };
                    let galley = one_line(
                        ui,
                        &row.title,
                        egui::FontId::proportional(crate::theme::BODY_FONT),
                        text_color,
                        row_rect.width() - 88.0,
                    );
                    ui.painter().galley(
                        egui::pos2(row_rect.min.x, row_rect.center().y - galley.size().y / 2.0),
                        galley,
                        text_color,
                    );
                    let mut cx = row_rect.max.x;
                    if let Some(p) = row.progress {
                        let pct = format!("{:.0}%", p * 100.0);
                        let g = ui.painter().layout_no_wrap(
                            pct,
                            egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
                            crate::theme::text_muted(),
                        );
                        ui.painter().galley(
                            egui::pos2(cx - g.size().x, row_rect.center().y - g.size().y / 2.0),
                            g,
                            crate::theme::text_muted(),
                        );
                        cx -= 38.0;
                    }
                    let mut icon_at = |icon: egui_material_icons::MaterialIcon,
                                       color: egui::Color32,
                                       hint: &str|
                     -> egui::Response {
                        let r = egui::Rect::from_min_size(
                            egui::pos2(cx - 16.0, row_rect.center().y - 8.0),
                            egui::vec2(16.0, 16.0),
                        );
                        cx -= 18.0;
                        let resp = ui.interact(
                            r,
                            ui.scope_id().with(("task_btn", row.id, r.min.x as i32)),
                            egui::Sense::click(),
                        );
                        let c = if resp.hovered() {
                            crate::theme::text_primary()
                        } else {
                            color
                        };
                        ui.painter().text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            icon.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                            c,
                        );
                        crate::widgets::hint::hover(ui.ctx(), &resp, hint);
                        resp
                    };
                    if let Some(cancel) = &row.cancel {
                        let resp = icon_at(
                            ICON_STOP,
                            crate::theme::text_muted(),
                            rust_i18n::t!("hint.toast.cancel").as_ref(),
                        );
                        if resp.clicked() && !row.cancelling {
                            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            cancel_id = Some(row.id);
                        }
                    }
                    if let Some(pause) = &row.pause {
                        let paused = pause.load(std::sync::atomic::Ordering::Relaxed);
                        let icon = if paused { ICON_PLAY_ARROW } else { ICON_PAUSE };
                        let resp = icon_at(
                            icon,
                            crate::theme::text_muted(),
                            rust_i18n::t!("hint.toast.pause").as_ref(),
                        );
                        if resp.clicked() {
                            pause.store(!paused, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
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
