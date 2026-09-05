use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

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

/// 历史条目年龄文案（纯函数，方便单测）：5 秒内“刚刚”，之后秒/分/时/天。
/// 无 chrono/time 依赖，超 7 天也显示“{n}天前”一直下去，不做 MM-DD。
fn format_age(created: Instant, now: Instant) -> String {
    let secs = now.saturating_duration_since(created).as_secs();
    if secs <= 5 {
        "刚刚".to_string()
    } else if secs < 60 {
        format!("{}秒前", secs)
    } else if secs < 3600 {
        format!("{}分钟前", secs / 60)
    } else if secs < 86_400 {
        format!("{}小时前", secs / 3600)
    } else {
        format!("{}天前", secs / 86_400)
    }
}

/// 统一样式卡片：弹出与列表共用，仅 show_close 区分
#[allow(clippy::too_many_arguments)]
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
    pause: Option<Arc<AtomicBool>>,
    action: Option<&super::model::ToastAction>,
    cancelling: bool,
    // 历史年龄行：仅 history_card 传非空（浮动卡传空串），有字才显示一行小字。
    history_age: &str,
) -> super::model::CardOutcome {
    let mut outcome = super::model::CardOutcome::default();
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
        let frame_resp = frame.show(ui, |ui| {
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
                    // 历史年龄行：仅历史列表有字才显示（浮动卡传空串，不占行）。
                    // 历史列表高度由 state 侧实测自适应，无需占位兼容。
                    if !history_age.is_empty() {
                        let age_shown = clamp_lines(&ctx, history_age, &det_font, wrap_w, 1);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(age_shown)
                                    .size(crate::theme::SMALL_LABEL_FONT)
                                    .color(mul_alpha(crate::theme::text_muted(), card_alpha)),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
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
                let bg = mul_alpha(crate::theme::line_fg().gamma_multiply(0.25), card_alpha);
                ui.painter().rect_filled(bg_rect, 0.0, bg);
                let fg_w = (x1 - x0) * p.clamp(0.0, 1.0);
                if fg_w > 0.0 {
                    let fg_rect =
                        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x0 + fg_w, y1));
                    ui.painter().rect_filled(
                        fg_rect,
                        0.0,
                        mul_alpha(kind.color().gamma_multiply(0.85), card_alpha),
                    );
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
                            mul_alpha(crate::theme::text_muted(), card_alpha),
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
                            mul_alpha(crate::theme::text_disabled(), card_alpha)
                        } else {
                            mul_alpha(crate::theme::text_muted(), card_alpha)
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
                            mul_alpha(crate::theme::text_muted(), card_alpha),
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
                                mul_alpha(crate::theme::text_muted(), card_alpha),
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
                                        .color(mul_alpha(crate::theme::text_muted(), card_alpha)),
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
    toast: &Toast,
    width: f32,
    x_offset: f32,
    alpha: f32,
    show_close: bool,
) -> super::model::CardOutcome {
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
        show_close,
        super::model::resolve_cancel_toast(toast),
        super::model::resolve_pause_toast(toast),
        toast.action.as_ref(),
        toast.cancelling,
        "",
    )
}

/// 历史卡片：只读，无删除（与 toast 同尺寸，仅隐藏 X）
pub(crate) fn history_card(ui: &mut egui::Ui, entry: &HistoryEntry, width: f32) {
    let (title, message, progress, label) = super::model::resolve_history(entry);
    let age = format_age(entry.created, Instant::now());
    let _ = draw_card(
        ui, width, 0.0, 1.0, entry.kind, &title, &message, progress, &label, false, None, None,
        None, false, &age,
    );
}

#[cfg(test)]
mod tests {
    use super::super::model::{ToastAction, ToastActionKind};
    use super::*;

    /// 无头渲染一张卡并量高度（无 CJK 字体时为 tofu，但跨状态可比）。
    fn card_height(
        width: f32,
        title: &str,
        message: &str,
        progress: Option<f32>,
        label: &str,
        action: Option<ToastAction>,
    ) -> f32 {
        card_height_full(
            width, title, message, progress, label, action, false, false, false,
        )
    }

    /// 带取消/暂停按钮的高度测量（暂停按钮与 stop 同条件显示，验证覆盖层没被撑大）。
    #[allow(clippy::too_many_arguments)]
    fn card_height_full(
        width: f32,
        title: &str,
        message: &str,
        progress: Option<f32>,
        label: &str,
        action: Option<ToastAction>,
        with_cancel: bool,
        with_pause: bool,
        cancelling: bool,
    ) -> f32 {
        let ctx = egui::Context::default();
        // 注册图标字体（app 启动时同款，否则图标 label 排版 panic）
        ctx.add_font(egui_material_icons::font_insert());
        let mut h = 0.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let cancel = if with_cancel {
            Some(Arc::new(AtomicBool::new(false)))
        } else {
            None
        };
        let pause_flag = if with_pause {
            Some(Arc::new(AtomicBool::new(false)))
        } else {
            None
        };
        let mut out = ctx.run_ui(raw, |ui| {
            draw_card(
                ui,
                width,
                0.0,
                1.0,
                ToastKind::Info,
                title,
                message,
                progress,
                label,
                true,
                cancel.clone(),
                pause_flag.clone(),
                action.as_ref(),
                cancelling,
                "",
            );
            h = ui.min_rect().height();
        });
        // 无头测试不贴纹理，显式丢弃（否则 debug 下 panic）
        out.textures_delta.clear();
        h
    }

    /// 暂停中（flag=true）高度测量：与进行中同按钮数，detail 覆盖“已暂停”仍 1 行。
    fn card_height_paused(width: f32, title: &str, message: &str, progress: f32) -> f32 {
        let ctx = egui::Context::default();
        ctx.add_font(egui_material_icons::font_insert());
        let mut h = 0.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let cancel = Some(Arc::new(AtomicBool::new(false)));
        let pause_flag = Some(Arc::new(AtomicBool::new(true)));
        let mut out = ctx.run_ui(raw, |ui| {
            draw_card(
                ui,
                width,
                0.0,
                1.0,
                ToastKind::Info,
                title,
                message,
                Some(progress),
                "已暂停",
                true,
                cancel.clone(),
                pause_flag.clone(),
                None,
                false,
                "",
            );
            h = ui.min_rect().height();
        });
        out.textures_delta.clear();
        h
    }

    fn assert_same_height(cases: &[f32]) {
        let first = cases[0];
        for (i, h) in cases.iter().enumerate() {
            assert!((h - first).abs() < 0.5, "case {i}: height {h} != {first}");
        }
    }

    /// 进度族（进行中/完成/失败/中止，文案空满长短，有无操作按钮）高度必须一致，否则加载卡上下跳。
    #[test]
    fn progress_card_height_stable() {
        fn text_action(label: &str) -> ToastAction {
            ToastAction {
                label: label.to_string(),
                kind: ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
                icon: None,
            }
        }
        fn icon_action(label: &str) -> ToastAction {
            ToastAction {
                label: label.to_string(),
                kind: ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
                icon: Some(ICON_FOLDER_OPEN),
            }
        }
        let w = 360.0;
        let running_empty = card_height(w, "正在加载", "解析 MIDI 音轨", Some(0.3), "", None);
        let running_short = card_height(w, "正在加载", "解析 MIDI 音轨", Some(0.3), "3/16", None);
        let running_long = card_height(
            w,
            "正在加载",
            "解析 MIDI 音轨",
            Some(0.9),
            "余韵衰减中 (剩余 3 音色) 余韵衰减中 (剩余 3 音色) 余韵衰减中",
            None,
        );
        let running_long_msg = card_height(
            w,
            "正在加载",
            "这是一段非常非常长的阶段文案这是一段非常非常长的阶段文案这是一段非常非常长的阶段文案",
            Some(0.5),
            "3/16",
            None,
        );
        let done = card_height(w, "MIDI加载完成", "a.mid", Some(1.0), "已完成", None);
        let done_duration = card_height(
            w,
            "MIDI加载完成",
            "a.mid",
            Some(1.0),
            "加载时间：15秒321毫秒",
            None,
        );
        let failed = card_height(w, "打开失败", "err", Some(0.5), "失败", None);
        let done_action = card_height(
            w,
            "已完成",
            "out.wav (12.3s, 8.1x)",
            Some(1.0),
            "已完成",
            Some(text_action("打开文件夹")),
        );
        let aborted = card_height(w, "正在导出", "渲染中 64%", Some(0.64), "已中止", None);
        let aborted_action = card_height(
            w,
            "正在导出",
            "渲染中 64%",
            Some(0.64),
            "已中止",
            Some(icon_action("打开文件夹")),
        );
        // 暂停中：按钮数（收起+stop+pause）/行数与进行中一致，detail“已暂停”仍 1 行
        let running_with_pause = card_height_full(
            w,
            "正在导出",
            "渲染中 64%",
            Some(0.64),
            "已渲染00:12 · 当前8.1x",
            None,
            true,
            true,
            false,
        );
        let paused = card_height_paused(w, "正在导出", "渲染中 64%", 0.64);
        assert_same_height(&[
            running_empty,
            running_short,
            running_long,
            running_long_msg,
            done,
            done_duration,
            failed,
            done_action,
            aborted,
            aborted_action,
            running_with_pause,
            paused,
        ]);
    }

    /// 暂停 toggle 翻转 flag（resolve+toggle 链路；draw_card 内点击即同语义 store）。
    #[test]
    fn pause_toggle_flips_flag() {
        use super::super::kind::ToastKind as Kind;
        use super::super::model::Toast;
        use super::super::model::{resolve_pause_toast, resolve_toast};
        use std::time::Instant as StdInstant;
        let pause_flag = Arc::new(AtomicBool::new(false));
        // ExportToastSource 级真 flag（与线上同构）
        let progress = yinhe_audio::export::ExportProgress::new();
        let src = Arc::new(crate::app::export_state::ExportToastSource {
            progress,
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::clone(&pause_flag),
        });
        let toast = Toast {
            id: 1,
            kind: Kind::Info,
            title: String::new(),
            message: String::new(),
            created: StdInstant::now(),
            progress: None,
            progress_label: String::new(),
            cancel: None,
            leaving_since: None,
            source: Some(src),
            collapse_at: None,
            action: None,
            hovered: false,
            cancelling: false,
        };
        // 未暂停时 detail 非“已暂停”
        let resolved = resolve_pause_toast(&toast);
        assert!(resolved.is_some());
        if let Some(p) = resolved {
            assert!(!p.load(std::sync::atomic::Ordering::Relaxed));
            // 模拟暂停按钮点击 toggle
            let cur = p.load(std::sync::atomic::Ordering::Relaxed);
            p.store(!cur, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(pause_flag.load(std::sync::atomic::Ordering::Relaxed));
        // 已暂停态 detail 覆盖“已暂停”
        let (_, _, _, detail) = resolve_toast(&toast);
        assert_eq!(detail, "已暂停");
        // 再 toggle 恢复
        if let Some(p) = resolve_pause_toast(&toast) {
            let cur = p.load(std::sync::atomic::Ordering::Relaxed);
            p.store(!cur, std::sync::atomic::Ordering::Relaxed);
        }
        assert!(!pause_flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    /// format_age 全分支：刚刚/秒/分钟/小时/天（含超 7 天一直显示天数）。
    #[test]
    fn format_age_branches() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let ago = |d: Duration| format_age(now - d, now);
        // 5 秒内“刚刚”（含边界）
        assert_eq!(ago(Duration::from_secs(0)), "刚刚");
        assert_eq!(ago(Duration::from_secs(5)), "刚刚");
        // 60 秒内秒
        assert_eq!(ago(Duration::from_secs(6)), "6秒前");
        assert_eq!(ago(Duration::from_secs(59)), "59秒前");
        // 60 分钟内分钟
        assert_eq!(ago(Duration::from_secs(60)), "1分钟前");
        assert_eq!(ago(Duration::from_secs(3599)), "59分钟前");
        // 24 小时内小时
        assert_eq!(ago(Duration::from_secs(3600)), "1小时前");
        assert_eq!(ago(Duration::from_secs(86399)), "23小时前");
        // 天（含 7 天边界与更早：无 chrono 依赖，一直显示天数）
        assert_eq!(ago(Duration::from_secs(86_400)), "1天前");
        assert_eq!(ago(Duration::from_secs(6 * 86_400)), "6天前");
        assert_eq!(ago(Duration::from_secs(7 * 86_400)), "7天前");
        assert_eq!(ago(Duration::from_secs(30 * 86_400)), "30天前");
        // 未来时间（created 晚于 now）钳制为“刚刚”
        assert_eq!(format_age(now, now - Duration::from_secs(10)), "刚刚");
    }
}
