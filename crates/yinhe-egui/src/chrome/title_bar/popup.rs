//! 标题栏文档选择 popup：点击文档标签弹出，列出「当前窗口」与「最近文件」，
//! 底部提供「打开文件」（Zed 同款工程选择器）。
//!
//! - 是否打开 / 锚点存 egui temp data（[`PopupState`]），外部点击、Esc、拖拽由
//!   父模块判定关闭。
//! - 行高固定（`control::h`），滚动区高度取整行高（`scroll::snap_rows` 思路），
//!   底边压在行边界上，规避 ScrollArea 硬裁出的“半截行”。
//! - 行悬停时右侧显示「在新窗口打开 / 删除（关闭文档、从最近文件移除）」两个图标。

use std::collections::HashMap;

use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use yinhe_editor_core::document::Document;

use super::TitleBarAction;
use super::paint::BarMetrics;
use crate::chrome::transport_bar_recent::{
    format_opened_at, recent_display_name, recent_file_icon,
};

/// popup 开关与锚点（跨帧，存 egui temp data）。
#[derive(Clone, Copy, Default)]
pub(super) struct PopupState {
    pub open: bool,
    /// 锚点：点击的标签左缘 x（屏幕坐标）。
    pub anchor_x: f32,
}

/// popup 状态在 egui data 中的固定 Id（主窗口标题栏唯一）。
pub(super) fn state_id() -> egui::Id {
    egui::Id::new("title_bar_doc_popup")
}

/// 最多显示多少整行（超出滚动）。
const MAX_VISIBLE_ROWS: usize = 12;
/// 行右侧单个图标按钮宽度（逻辑像素，随字体缩放）。
const ICON_BTN_W: f32 = 22.0;

/// popup 一帧的渲染结果。
pub(super) struct PopupOutcome {
    /// popup 矩形（外部点击关闭判定用）。
    pub rect: egui::Rect,
    /// 触发的外部动作。
    pub action: Option<TitleBarAction>,
    /// 有行/按钮被激活 → 调用方关闭 popup。
    pub close: bool,
}

/// 显示 popup（调用方保证已处于打开态）。
pub(super) fn show(
    ui: &mut egui::Ui,
    m: &BarMetrics,
    documents: &[Document],
    active_doc: &mut Option<usize>,
    recent: &[String],
    recent_times: &HashMap<String, i64>,
    anchor_x: f32,
) -> PopupOutcome {
    let mut action = None;
    let mut close = false;
    let width = measure_width(ui.ctx(), documents, recent);
    let area = egui::Area::new(state_id().with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(anchor_x, m.bar_rect.max.y + 2.0))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
                ui.set_min_width(width);
                ui.set_max_width(width);
                populate(
                    ui,
                    documents,
                    active_doc,
                    recent,
                    recent_times,
                    &mut action,
                    &mut close,
                );
            });
        });
    PopupOutcome {
        rect: area.response.rect,
        action,
        close,
    }
}

#[allow(clippy::too_many_arguments)]
fn populate(
    ui: &mut egui::Ui,
    documents: &[Document],
    active_doc: &mut Option<usize>,
    recent: &[String],
    recent_times: &HashMap<String, i64>,
    action: &mut Option<TitleBarAction>,
    close: &mut bool,
) {
    let row_h = crate::widgets::control::h(ui.ctx());
    // 小节标题 ×2 + 文档行 + 最近行（为空时一行占位）
    let body_rows = 2 + documents.len() + recent.len().max(1);
    let visible = body_rows.min(MAX_VISIBLE_ROWS) as f32;
    let pitch = row_h + ui.spacing().item_spacing.y;
    // 视口高度 = 整行高度的整数倍，底边永远压在行边界上（无半截行）
    let max_h = crate::widgets::scroll::snap_rows(visible * pitch, pitch, None);
    egui::ScrollArea::vertical()
        .id_salt("title_bar_doc_popup_scroll")
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            section_header(ui, row_h, &t!("title_bar.this_window"));
            for (idx, doc) in documents.iter().enumerate() {
                // 讲解：完整路径（未落盘工程退回文件名）
                let hint = doc
                    .file_path
                    .clone()
                    .unwrap_or_else(|| doc.file_name.clone());
                let out = row(
                    ui,
                    row_h,
                    RowSpec {
                        id_salt: ("doc", idx),
                        icon: recent_file_icon(&doc.file_name),
                        label: &doc.file_name,
                        hint: &hint,
                        remove_hint: &t!("hint.title_bar.close"),
                        check_mark: *active_doc == Some(idx),
                    },
                );
                if out.clicked {
                    *active_doc = Some(idx);
                    *close = true;
                } else if out.open_new {
                    *action = Some(TitleBarAction::DetachTab(idx));
                    *close = true;
                } else if out.remove {
                    *action = Some(TitleBarAction::CloseDocument(idx));
                    *close = true;
                }
            }
            section_header(ui, row_h, &t!("menu.recent_files"));
            if recent.is_empty() {
                placeholder_row(ui, row_h, &t!("welcome.no_recent"));
            }
            for (idx, path) in recent.iter().enumerate() {
                // 讲解：完整路径 +（若记录了）上次打开时间
                let hint = match recent_times.get(path) {
                    Some(&secs) => format!(
                        "{} · {}",
                        path,
                        t!("hint.recent_opened_at", time = format_opened_at(secs))
                    ),
                    None => path.clone(),
                };
                let out = row(
                    ui,
                    row_h,
                    RowSpec {
                        id_salt: ("recent", idx),
                        icon: recent_file_icon(path),
                        label: recent_display_name(path),
                        hint: &hint,
                        remove_hint: &t!("hint.title_bar.remove_recent"),
                        check_mark: false,
                    },
                );
                if out.clicked {
                    *action = Some(TitleBarAction::OpenRecent(path.clone()));
                    *close = true;
                } else if out.open_new {
                    *action = Some(TitleBarAction::OpenRecentInNewWindow(path.clone()));
                    *close = true;
                } else if out.remove {
                    *action = Some(TitleBarAction::RemoveRecent(path.clone()));
                    *close = true;
                }
            }
        });
    ui.separator();
    let open_btn = crate::widgets::menu::menu_item_button(
        ui,
        false,
        crate::widgets::icon_text::icon_text(
            ICON_FOLDER_OPEN,
            &t!("title_bar.open_file"),
            crate::theme::FILE_MENU_FONT,
            crate::theme::text_primary(),
        ),
    );
    if ui.add(open_btn).clicked() {
        *action = Some(TitleBarAction::OpenFile);
        *close = true;
    }
}

/// 行交互结果（点击主体 / ↗ 在新窗口打开 / × 删除或关闭）。
#[derive(Default)]
struct RowOutcome {
    clicked: bool,
    open_new: bool,
    remove: bool,
}

/// 菜单行内容参数。
struct RowSpec<'a> {
    id_salt: (&'static str, usize),
    icon: egui_material_icons::MaterialIcon,
    label: &'a str,
    /// 行讲解（完整路径 / 路径 + 上次打开时间）。
    hint: &'a str,
    /// × 按钮讲解（关闭文档 / 移除记录）。
    remove_hint: &'a str,
    /// 未悬停时右端是否显示活跃勾选。
    check_mark: bool,
}

/// 菜单行：图标 + 名称；悬停时右侧显示两个图标按钮，未悬停时可显示活跃勾选。
fn row(ui: &mut egui::Ui, row_h: f32, spec: RowSpec<'_>) -> RowOutcome {
    let RowSpec {
        id_salt,
        icon,
        label,
        hint,
        remove_hint,
        check_mark,
    } = spec;
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    let row_w = ui.available_width();
    let (row_rect, resp) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, ui.is_enabled(), label)
    });
    // 行悬停用指针位置手动判定：指针移到右侧图标上时 egui 的前景交互目标变成图标，
    // `resp.hovered()` 会翻假 → 图标随行 hover 一起消失（自循环闪烁）。手动判定
    // 保证指针在整行范围内时图标常驻。
    let row_hovered = ui.rect_contains_pointer(row_rect);
    let pressed = resp.is_pointer_button_down_on();
    let (bg_fill, text_color) = {
        let style = ui.style();
        let widgets = &style.visuals.widgets;
        let v = if pressed {
            &widgets.active
        } else if row_hovered {
            &widgets.hovered
        } else {
            &widgets.inactive
        };
        (v.bg_fill, v.text_color())
    };
    let radius = egui::CornerRadius::same(scale(4.0).round() as u8);
    if row_hovered || pressed {
        ui.painter().rect_filled(row_rect, radius, bg_fill);
    }

    let icon_btn_w = scale(ICON_BTN_W);
    // 左边距取与上下边距一致（与 MenuItemButton 同款公式）
    let h_pad = ((row_h - scale(13.0)) * 0.5).clamp(scale(2.0), scale(6.0));
    let text_w = (row_w - h_pad * 2.0 - icon_btn_w * 2.0).max(0.0);
    let job =
        crate::widgets::icon_text::icon_text(icon, label, crate::theme::FILE_MENU_FONT, text_color);
    let galley = egui::WidgetText::from(job).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        text_w,
        egui::TextStyle::Button,
    );
    ui.painter().galley(
        egui::pos2(
            row_rect.min.x + h_pad,
            row_rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        text_color,
    );

    let mut out = RowOutcome::default();
    let center_y = row_rect.center().y;
    if row_hovered {
        // 右侧两个图标按钮：左 = 在新窗口打开，右 = 删除（最右）
        let new_rect = egui::Rect::from_center_size(
            egui::pos2(row_rect.max.x - icon_btn_w * 1.5, center_y),
            egui::vec2(icon_btn_w, icon_btn_w),
        );
        let del_rect = egui::Rect::from_center_size(
            egui::pos2(row_rect.max.x - icon_btn_w * 0.5, center_y),
            egui::vec2(icon_btn_w, icon_btn_w),
        );
        out.open_new = icon_button(
            ui,
            new_rect,
            ICON_OPEN_IN_NEW,
            (id_salt.0, id_salt.1, "new"),
            false,
            &t!("hint.title_bar.open_new"),
        )
        .clicked();
        out.remove = icon_button(
            ui,
            del_rect,
            ICON_CLOSE,
            (id_salt.0, id_salt.1, "del"),
            true,
            remove_hint,
        )
        .clicked();
    } else if check_mark {
        // 活跃文档：右端勾选（与 Zed 的当前项标记一致）
        let rect = egui::Rect::from_center_size(
            egui::pos2(row_rect.max.x - icon_btn_w * 0.5, center_y),
            egui::vec2(icon_btn_w, icon_btn_w),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            ICON_CHECK.codepoint.to_string(),
            egui::FontId::new(crate::theme::ICON_FONT_SM, ICON_CHECK.font_family()),
            crate::theme::text_primary(),
        );
    }
    crate::widgets::hint::hover_popup(ui.ctx(), &resp, hint);
    out.clicked = resp.clicked();
    out
}

/// 行内图标按钮：`close_style` = 统一 ×（悬停红底），否则四态前景色。
fn icon_button(
    ui: &egui::Ui,
    rect: egui::Rect,
    icon: egui_material_icons::MaterialIcon,
    id_salt: (&'static str, usize, &'static str),
    close_style: bool,
    hint: &str,
) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(id_salt), egui::Sense::click());
    if close_style {
        crate::chrome::dialog::paint_close_button(
            ui.painter(),
            rect,
            resp.hovered(),
            resp.is_pointer_button_down_on(),
        );
    } else {
        let color =
            crate::widgets::hover::hover_button_color(&resp, crate::theme::text_muted(), false);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint.to_string(),
            egui::FontId::new(crate::theme::ICON_FONT_SM, icon.font_family()),
            color,
        );
    }
    crate::widgets::hint::hover_popup(ui.ctx(), &resp, hint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hint));
    resp
}

/// 小节标题行（固定行高，与行同拍，保证滚动取整成立）。
fn section_header(ui: &mut egui::Ui, row_h: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::hover(),
    );
    let h_pad = crate::scaling::scaled_font(ui.ctx(), 6.0);
    ui.painter().text(
        egui::pos2(rect.min.x + h_pad, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(crate::scaling::scaled_font(
            ui.ctx(),
            crate::theme::SMALL_FONT,
        )),
        crate::theme::text_label(),
    );
}

/// 占位行（如最近列表为空时的弱化提示）。
fn placeholder_row(ui: &mut egui::Ui, row_h: f32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::hover(),
    );
    let h_pad = crate::scaling::scaled_font(ui.ctx(), 6.0);
    ui.painter().text(
        egui::pos2(rect.min.x + h_pad, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(crate::scaling::scaled_font(
            ui.ctx(),
            crate::theme::BODY_FONT,
        )),
        crate::theme::text_disabled(),
    );
}

/// popup 宽度：按最长行内容测量后 clamp（避免固定宽度截断或过宽）。
fn measure_width(ctx: &egui::Context, documents: &[Document], recent: &[String]) -> f32 {
    let scale = |v: f32| crate::scaling::scaled_font(ctx, v);
    let font = crate::theme::FILE_MENU_FONT;
    let mut content_w = icon_text_width(ctx, ICON_FOLDER_OPEN, &t!("title_bar.open_file"), font);
    for doc in documents {
        content_w = content_w.max(icon_text_width(
            ctx,
            recent_file_icon(&doc.file_name),
            &doc.file_name,
            font,
        ));
    }
    for path in recent {
        content_w = content_w.max(icon_text_width(
            ctx,
            recent_file_icon(path),
            recent_display_name(path),
            font,
        ));
    }
    // 行左内边距 + 右侧两个图标按钮 + 菜单框内边距/滚动条
    (content_w + scale(6.0) * 2.0 + scale(ICON_BTN_W) * 2.0 + scale(36.0))
        .clamp(scale(300.0), scale(520.0))
}

fn icon_text_width(
    ctx: &egui::Context,
    icon: egui_material_icons::MaterialIcon,
    label: &str,
    size: f32,
) -> f32 {
    let job = crate::widgets::icon_text::icon_text(icon, label, size, egui::Color32::WHITE);
    ctx.fonts_mut(|f| f.layout_job(job).size().x)
}
