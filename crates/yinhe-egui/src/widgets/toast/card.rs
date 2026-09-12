mod text;
mod time;

#[cfg(test)]
mod tests;

use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Instant, SystemTime};

use eframe::egui;
use egui_material_icons::icons::*;

use self::text::{blank_line, clamp_lines};
use self::time::format_timestamp;

use super::kind::ToastKind;
use super::model::Notification;

pub(crate) fn base_frame() -> egui::Frame {
    egui::Frame {
        fill: crate::theme::control_bg(),
        stroke: egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35)),
        corner_radius: egui::CornerRadius::same(8),
        shadow: egui::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        },
        inner_margin: egui::Margin::symmetric(10, 10),
        ..Default::default()
    }
}

/// 统一样式卡片：弹出与列表共用，仅 show_close 区分
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_card(
    ui: &mut egui::Ui,
    width: f32,
    kind: ToastKind,
    title: &str,
    message: &str,
    progress: Option<f32>,
    progress_label: &str,
    show_close: bool,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
    action: Option<&super::model::ToastAction>,
    cancelling: bool,
    // 统一时间戳：浮动/历史/进行中都画 `{相对} · {绝对}`（muted 小字，年龄行同款式）。
    created: Instant,
) -> super::model::CardOutcome {
    let mut outcome = super::model::CardOutcome::default();
    let frame = base_frame();
    // 进行中（进度条未满）：三行文案槽位全部锁死行数，空也占位，卡片高度全程不变；
    // 静态卡（普通通知/已完成）：标题 1 行、正文至多 2 行。
    let running = progress
        .is_some_and(|p| p < 0.999 && progress_label != "已完成" && progress_label != "失败");
    ui.scope(|ui| {
        // 水平放宽（阴影/溢出）但垂直沿用父 clip：
        // ScrollArea 内 available_rect_before_wrap 受 content max_rect（只有视口高）限制，
        // 滚动后下半部分卡片会拿到反向矩形而被整卡跳过绘制，必须用 clip_rect 为基准。
        let mut clip = ui.clip_rect();
        clip.max.x += 500.0;
        clip.min.x -= 20.0;
        ui.set_clip_rect(clip);
        let frame_resp = frame.show(ui, |ui| {
            ui.set_max_width(width - 20.0);
            ui.set_min_width(width - 20.0);
            ui.horizontal(|ui| {
                let icon_col = kind.color();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(kind.icon().codepoint)
                            .family(kind.icon().font_family())
                            .size(crate::theme::ICON_FONT)
                            .color(icon_col),
                    )
                    .selectable(false),
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_max_width(width - 90.0);
                    let ctx = ui.ctx().clone();
                    let wrap_w = width - 90.0;
                    let title_font = egui::FontId::proportional(crate::theme::SMALL_FONT);
                    let msg_font = egui::FontId::proportional(crate::theme::SMALL_FONT);
                    let det_font = egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT);
                    // 标题：恒 1 行；进行中为空也占位
                    let title_shown = clamp_lines(&ctx, title, &title_font, wrap_w, 1);
                    if running || !title_shown.is_empty() {
                        if title_shown.is_empty() {
                            blank_line(ui, &title_font, wrap_w);
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(title_shown)
                                        .size(crate::theme::SMALL_FONT)
                                        .strong()
                                        .color(crate::theme::text_primary()),
                                )
                                .selectable(false)
                                .wrap(),
                            );
                        }
                    }
                    // 正文：进行中锁 1 行，静态卡至多 2 行（文件名）
                    if running {
                        let msg_shown = clamp_lines(&ctx, message, &msg_font, wrap_w, 1);
                        if msg_shown.is_empty() {
                            blank_line(ui, &msg_font, wrap_w);
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(msg_shown)
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_secondary()),
                                )
                                .selectable(false)
                                .wrap(),
                            );
                        }
                    } else if !message.is_empty() {
                        let msg_shown = clamp_lines(&ctx, message, &msg_font, wrap_w, 2);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(msg_shown)
                                    .size(crate::theme::SMALL_FONT)
                                    .color(crate::theme::text_secondary()),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                    // 详情：进行中锁 1 行（空也占位），静态卡有字才显示
                    if running {
                        let det_shown = clamp_lines(&ctx, progress_label, &det_font, wrap_w, 1);
                        if det_shown.is_empty() {
                            blank_line(ui, &det_font, wrap_w);
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(det_shown)
                                        .size(crate::theme::SMALL_LABEL_FONT)
                                        .color(crate::theme::text_muted()),
                                )
                                .selectable(false)
                                .wrap(),
                            );
                        }
                    } else if progress.is_some() && !progress_label.is_empty() {
                        let det_shown = clamp_lines(&ctx, progress_label, &det_font, wrap_w, 1);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(det_shown)
                                    .size(crate::theme::SMALL_LABEL_FONT)
                                    .color(crate::theme::text_muted()),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                    // 统一时间戳行：所有卡都画一行（浮动/历史/进行中），与详情同 muted 小字；
                    // 高度各卡统一增一行，相对高度不变。墙钟由单调钟反推（future 钳制到 now）。
                    let timestamp = format_timestamp(created, Instant::now(), SystemTime::now());
                    let ts_shown = clamp_lines(&ctx, &timestamp, &det_font, wrap_w, 1);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(ts_shown)
                                .size(crate::theme::SMALL_LABEL_FONT)
                                .color(crate::theme::text_muted()),
                        )
                        .selectable(false)
                        .wrap(),
                    );
                    // 右侧按钮组已移至覆盖层（整卡垂直居中），此处仅保留 width-90 给右侧留空
                });
            });
            if progress.is_none() {
                ui.add_space(20.0);
            }
        });
        // 进度条画在底边上（非交互，不占布局）：x 避开 8px 圆角，y 取底边内侧 2px 高。
        // 已完成/失败/已中止或进度接近 1 时隐藏（数字已在 label 外部显示）。
        let show_bar = progress.is_some_and(|p| {
            p < 0.999
                && progress_label != "已完成"
                && progress_label != "失败"
                && progress_label != "已中止"
        });
        if show_bar && let Some(p) = progress {
            let card_rect = frame_resp.response.rect;
            let x0 = card_rect.min.x + 8.0;
            let x1 = card_rect.max.x - 8.0;
            if x1 > x0 {
                let y1 = card_rect.max.y - 1.0;
                let y0 = y1 - 2.0;
                let bg_rect = egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1));
                let bg = crate::theme::line_fg().gamma_multiply(0.25);
                ui.painter().rect_filled(bg_rect, 0.0, bg);
                let fg_w = (x1 - x0) * p.clamp(0.0, 1.0);
                if fg_w > 0.0 {
                    let fg_rect =
                        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x0 + fg_w, y1));
                    ui.painter()
                        .rect_filled(fg_rect, 0.0, kind.color().gamma_multiply(0.85));
                }
            }
        }
        // 右侧按钮覆盖层：相对整卡真正垂直居中（只在有按钮时分配，历史卡片不分配）
        let mut overlay_hovered = false;
        if show_close
            || ((cancel.is_some() || pause.is_some()) && progress.is_some())
            || action.is_some()
        {
            let card_rect = frame_resp.response.rect;
            let center_y = card_rect.center().y;
            let right = card_rect.max.x - 10.0;
            let overlay_rect = egui::Rect::from_min_max(
                egui::pos2(right - 70.0, center_y - 14.0),
                egui::pos2(right, center_y + 14.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(overlay_rect), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if show_close {
                        let resp = crate::widgets::hover::hover_button(
                            ui,
                            ICON_KEYBOARD_DOUBLE_ARROW_RIGHT.codepoint,
                            egui::FontId::new(
                                crate::theme::ICON_FONT_SM,
                                ICON_KEYBOARD_DOUBLE_ARROW_RIGHT.font_family(),
                            ),
                            crate::theme::text_muted(),
                            false,
                        );
                        if resp.clicked() {
                            outcome.dismiss = true;
                        }
                        overlay_hovered |= resp.hovered();
                    }
                    if let Some(c) = &cancel
                        && progress.is_some()
                    {
                        ui.add_space(6.0);
                        // 中止中置灰且点击无反应
                        let stop_col = if cancelling {
                            crate::theme::text_disabled()
                        } else {
                            crate::theme::text_muted()
                        };
                        let resp2 = crate::widgets::hover::hover_button(
                            ui,
                            ICON_STOP.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT_SM, ICON_STOP.font_family()),
                            stop_col,
                            false,
                        );
                        if !cancelling && resp2.clicked() {
                            c.store(true, std::sync::atomic::Ordering::Relaxed);
                            outcome.cancel = true;
                        }
                        overlay_hovered |= resp2.hovered();
                    }
                    // 暂停按钮：与 stop 同条件显示（进行中、有 source），位于 stop 左侧；
                    // 右→左顺序：收起箭头、stop、pause。cancelling 时不画。
                    // 点击直接 toggle flag（原子操作，无需 outcome 回传）。
                    if let Some(p) = &pause
                        && progress.is_some()
                        && !cancelling
                    {
                        ui.add_space(6.0);
                        let is_paused = p.load(std::sync::atomic::Ordering::Relaxed);
                        let icon = if is_paused {
                            ICON_PLAY_ARROW
                        } else {
                            ICON_PAUSE
                        };
                        let resp_pause = crate::widgets::hover::hover_button(
                            ui,
                            icon.codepoint,
                            egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                            crate::theme::text_muted(),
                            false,
                        );
                        if resp_pause.clicked() {
                            p.store(!is_paused, std::sync::atomic::Ordering::Relaxed);
                        }
                        overlay_hovered |= resp_pause.hovered();
                    }
                    // 操作按钮（如“打开文件夹”）：只执行不收卡，收起交给自动计时；
                    // 有图标画图标按钮（hover tooltip 显示 label），否则走文字分支
                    if let Some(a) = action {
                        ui.add_space(6.0);
                        if let Some(icon) = a.icon {
                            let resp3 = crate::widgets::hover::hover_button(
                                ui,
                                icon.codepoint,
                                egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
                                crate::theme::text_muted(),
                                false,
                            )
                            .on_hover_text(&a.label);
                            if resp3.clicked() {
                                outcome.action = true;
                            }
                            overlay_hovered |= resp3.hovered();
                        } else {
                            let resp3 = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&a.label)
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                )
                                .sense(egui::Sense::click())
                                .selectable(false),
                            );
                            if resp3.clicked() {
                                outcome.action = true;
                            }
                            if resp3.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            overlay_hovered |= resp3.hovered();
                        }
                    }
                });
            });
        }
        // 整卡悬停即暂停自动收起计时（含覆盖层按钮区，阴影除外）
        outcome.hovered = frame_resp.response.hovered() || overlay_hovered;
    });
    outcome
}

/// 返回当帧交互结果（含悬停）
pub(crate) fn toast_card(
    ui: &mut egui::Ui,
    toast: &Notification,
    width: f32,
    show_close: bool,
) -> super::model::CardOutcome {
    // 进度任务：渲染时从 source pull 最新文案/进度，无 source 读快照
    let (title, message, progress, label) = toast.resolve();
    draw_card(
        ui,
        width,
        toast.kind,
        &title,
        &message,
        progress,
        &label,
        show_close,
        toast.cancel_flag(),
        toast.pause_flag(),
        toast.action.as_ref(),
        toast.cancelling,
        toast.created,
    )
}

/// 历史卡片：只读，无删除（与 toast 同尺寸，仅隐藏 X）
pub(crate) fn history_card(ui: &mut egui::Ui, entry: &Notification, width: f32) {
    let (title, message, progress, label) = entry.resolve();
    let _ = draw_card(
        ui,
        width,
        entry.kind,
        &title,
        &message,
        progress,
        &label,
        false,
        None,
        None,
        None,
        false,
        entry.created,
    );
}
