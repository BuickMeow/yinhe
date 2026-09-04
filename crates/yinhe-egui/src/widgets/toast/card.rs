use std::sync::{Arc, atomic::AtomicBool};

use eframe::egui;
use egui_material_icons::icons::*;

use super::anim::mul_alpha;
use super::kind::ToastKind;
use super::model::{HistoryEntry, Toast};

pub(crate) fn base_frame(alpha: f32) -> egui::Frame {
    let bg_base = crate::theme::control_bg();
    let stroke_base = crate::theme::line_fg().gamma_multiply(0.35);
    let bg = egui::Color32::from_rgba_unmultiplied(
        bg_base.r(),
        bg_base.g(),
        bg_base.b(),
        (bg_base.a() as f32 * alpha) as u8,
    );
    let stroke_col = egui::Color32::from_rgba_unmultiplied(
        stroke_base.r(),
        stroke_base.g(),
        stroke_base.b(),
        (stroke_base.a() as f32 * alpha) as u8,
    );
    egui::Frame {
        fill: bg,
        stroke: egui::Stroke::new(1.0, stroke_col),
        corner_radius: egui::CornerRadius::same(8),
        shadow: egui::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_black_alpha((60.0 * alpha) as u8),
        },
        inner_margin: egui::Margin::symmetric(10, 10),
        ..Default::default()
    }
}

/// 量文本在给定宽度下的行数（memoized，重复调用便宜）。
fn count_rows(ctx: &egui::Context, text: &str, font: &egui::FontId, wrap_w: f32) -> usize {
    if text.is_empty() {
        return 0;
    }
    ctx.fonts_mut(|f| {
        f.layout(text.to_string(), font.clone(), egui::Color32::WHITE, wrap_w)
            .rows
            .len()
    })
}

/// 文案截到至多 max_lines 行，超出加 …（egui Label 没有行数限制，手动量）。
/// 返回显示文本；空文本返回空串，由调用方决定占位还是跳过。
fn clamp_lines(
    ctx: &egui::Context,
    text: &str,
    font: &egui::FontId,
    wrap_w: f32,
    max_lines: usize,
) -> String {
    if text.is_empty() || max_lines == 0 {
        return String::new();
    }
    if count_rows(ctx, text, font, wrap_w) <= max_lines {
        return text.to_string();
    }
    // 二分找最长前缀（+…后仍在行数内）
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid].iter().collect::<String>() + "…";
        if count_rows(ctx, &cand, font, wrap_w) <= max_lines {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// 固定占一行高度的空白：空文案也占位，保证进行中卡片高度不变。
/// 用 allocate（真 widget）而非 add_space，保证与真实行享有同样的 item_spacing。
fn blank_line(ui: &mut egui::Ui, font: &egui::FontId, wrap_w: f32) {
    let h = ui.ctx().fonts_mut(|f| f.row_height(font));
    ui.allocate_exact_size(egui::vec2(wrap_w, h), egui::Sense::hover());
}

/// 统一样式卡片：弹出与列表共用，仅 show_close 区分
pub(crate) fn draw_card(
    ui: &mut egui::Ui,
    width: f32,
    x_offset: f32,
    alpha: f32,
    kind: ToastKind,
    title: &str,
    message: &str,
    progress: Option<f32>,
    progress_label: &str,
    show_close: bool,
    cancel: Option<Arc<AtomicBool>>,
) -> (bool, bool) {
    let mut dismiss = false;
    let mut do_cancel = false;
    let frame = base_frame(alpha);
    let card_alpha = alpha;
    // 进行中（进度条未满）：三行文案槽位全部锁死行数，空也占位，卡片高度全程不变；
    // 静态卡（普通通知/已完成）：标题 1 行、正文至多 2 行。
    let running = progress
        .is_some_and(|p| p < 0.999 && progress_label != "已完成" && progress_label != "失败");
    ui.scope(|ui| {
        let mut clip = ui.available_rect_before_wrap();
        clip.max.x += 500.0;
        clip.min.x -= 20.0;
        ui.set_clip_rect(clip);
        let _ = x_offset; // 外层 Area 已处理飞入位移，此处固定 0，避免布局溢出
        frame.show(ui, |ui| {
            ui.set_max_width(width - 20.0);
            ui.set_min_width(width - 20.0);
            ui.horizontal(|ui| {
                let icon_col = mul_alpha(kind.color(), card_alpha);
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
                                        .color(mul_alpha(crate::theme::text_primary(), card_alpha)),
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
                                        .color(mul_alpha(
                                            crate::theme::text_secondary(),
                                            card_alpha,
                                        )),
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
                                    .color(mul_alpha(crate::theme::text_secondary(), card_alpha)),
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
                                        .color(mul_alpha(crate::theme::text_muted(), card_alpha)),
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
                                    .color(mul_alpha(crate::theme::text_muted(), card_alpha)),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                });
                if show_close || cancel.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if show_close {
                            let resp = crate::widgets::hover::hover_button(
                                ui,
                                ICON_CLOSE.codepoint,
                                egui::FontId::new(
                                    crate::theme::ICON_FONT_SM,
                                    ICON_CLOSE.font_family(),
                                ),
                                mul_alpha(crate::theme::text_muted(), card_alpha),
                                false,
                            );
                            if resp.clicked() {
                                dismiss = true;
                            }
                        }
                        if let Some(c) = &cancel {
                            if progress.is_some() {
                                ui.add_space(6.0);
                                let resp2 = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("取消")
                                            .size(crate::theme::SMALL_FONT)
                                            .color(mul_alpha(
                                                crate::theme::text_muted(),
                                                card_alpha,
                                            )),
                                    )
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                                );
                                if resp2.clicked() {
                                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                                    do_cancel = true;
                                }
                                if resp2.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                        }
                    });
                }
            });
            if let Some(p) = progress {
                // 已完成/失败或进度接近 1 时隐藏进度条
                let is_done = p >= 0.999 || progress_label == "已完成" || progress_label == "失败";
                if !is_done {
                    ui.add_space(6.0);
                    let bar_w = width - 20.0;
                    let bar_h = 2.0;
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                    let bg = mul_alpha(crate::theme::line_fg().gamma_multiply(0.25), card_alpha);
                    ui.painter().rect_filled(rect, 1.0, bg);
                    let fg_rect = egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(rect.width() * p.clamp(0.0, 1.0), rect.height()),
                    );
                    ui.painter().rect_filled(
                        fg_rect,
                        1.0,
                        mul_alpha(kind.color().gamma_multiply(0.85), card_alpha),
                    );
                    ui.add_space(12.0);
                    // 数字已在 progress_label 中外部显示，不再在条中心绘制
                } else {
                    // 已完成隐藏进度条，保持占位避免高度跳变
                    ui.add_space(20.0);
                }
            } else {
                ui.add_space(20.0);
            }
        });
    });
    (dismiss, do_cancel)
}

/// 返回 (dismiss, cancel)
pub(crate) fn toast_card(
    ui: &mut egui::Ui,
    toast: &Toast,
    width: f32,
    x_offset: f32,
    alpha: f32,
) -> (bool, bool) {
    // 进度任务：渲染时从 source pull 最新文案/进度，无 source 读快照
    let (title, message, progress, label) = super::model::resolve_toast(toast);
    draw_card(
        ui,
        width,
        x_offset,
        alpha,
        toast.kind,
        &title,
        &message,
        progress,
        &label,
        true,
        super::model::resolve_cancel_toast(toast),
    )
}

/// 历史卡片：只读，无删除（与 toast 同尺寸，仅隐藏 X）
pub(crate) fn history_card(ui: &mut egui::Ui, entry: &HistoryEntry, width: f32) {
    let (title, message, progress, label) = super::model::resolve_history(entry);
    let _ = draw_card(
        ui, width, 0.0, 1.0, entry.kind, &title, &message, progress, &label, false, None,
    );
}
