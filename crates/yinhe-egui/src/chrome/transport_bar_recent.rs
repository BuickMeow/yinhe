use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::widgets::action_menu::{PopupRowSpec, popup_menu_row};

const RECENT_SUBMENU_OPEN_ID: &str = "recent_files_submenu_open";

pub(crate) fn recent_display_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// 把 Unix 秒格式化为本地时间 `YYYY-MM-DD HH:MM`；无效值返回空串。
fn format_opened_at(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn measure_recent_parent_row_width(ctx: &egui::Context) -> f32 {
    let spacing = &ctx.style_of(ctx.theme()).spacing;
    let pad_x = spacing.button_padding.x * 2.0;
    let arrow_w = crate::theme::FILE_MENU_FONT + spacing.item_spacing.x;
    let job = crate::widgets::icon_text::icon_text(
        ICON_HISTORY,
        &t!("menu.recent_files"),
        crate::theme::FILE_MENU_FONT,
        egui::Color32::WHITE,
    );
    ctx.fonts_mut(|f| f.layout_job(job).size().x) + arrow_w + pad_x
}

fn measure_recent_submenu_width(ctx: &egui::Context, recent: &[String]) -> f32 {
    let spacing = &ctx.style_of(ctx.theme()).spacing;
    let pad_x = spacing.button_padding.x * 2.0;
    ctx.fonts_mut(|f| {
        recent
            .iter()
            .map(|path| {
                let job = crate::widgets::icon_text::icon_text(
                    ICON_DESCRIPTION,
                    recent_display_name(path),
                    crate::theme::FILE_MENU_FONT,
                    egui::Color32::WHITE,
                );
                f.layout_job(job).size().x
            })
            .fold(0.0f32, f32::max)
            + pad_x
    })
}

pub fn recent_files_section(
    ui: &mut egui::Ui,
    recent: &[String],
    times: &std::collections::HashMap<String, i64>,
    any_row_hovered: bool,
    pending_open_path: &mut Option<String>,
) {
    let open_id = egui::Id::new(RECENT_SUBMENU_OPEN_ID);
    let mut open: bool = ui.ctx().data_mut(|d| d.get_temp(open_id)).unwrap_or(false);
    if any_row_hovered {
        open = false;
    }

    ui.separator();
    let (row_resp, _) = popup_menu_row(
        ui,
        PopupRowSpec {
            icon: ICON_HISTORY,
            label: &t!("menu.recent_files"),
            shortcut: None,
            enabled: true,
            selected: open,
            accent: None,
            pin: None,
            pin_index: None,
            chevron: true,
        },
    );
    if row_resp.hovered() {
        open = true;
    }
    crate::widgets::hint::hover_popup(ui.ctx(), &row_resp, t!("hint.recent_files"));
    if row_resp.clicked() {
        open = !open;
    }
    ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
    if !open {
        return;
    }

    let sub_w = measure_recent_submenu_width(ui.ctx(), recent);
    egui::Popup::from_response(&row_resp)
        .id(egui::Id::new("recent_files_submenu"))
        .open(true)
        .align(egui::RectAlign::RIGHT_START)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .gap(2.0)
        .width(sub_w)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_min_width(sub_w);
            ui.set_max_width(sub_w);
            for path in recent {
                let (resp, _) = popup_menu_row(
                    ui,
                    PopupRowSpec {
                        icon: ICON_DESCRIPTION,
                        label: recent_display_name(path),
                        shortcut: None,
                        enabled: true,
                        selected: false,
                        accent: None,
                        pin: None,
                        pin_index: None,
                        chevron: false,
                    },
                );
                if resp.clicked() {
                    *pending_open_path = Some(path.clone());
                    egui::Popup::close_all(ui.ctx());
                } else if resp.hovered() {
                    // 讲解：完整路径 +（若记录了）上次打开时间。
                    let hint = match times.get(path) {
                        Some(&secs) => format!(
                            "{} · {}",
                            path,
                            t!("hint.recent_opened_at", time = format_opened_at(secs))
                        ),
                        None => path.clone(),
                    };
                    crate::widgets::hint::hover_popup(ui.ctx(), &resp, hint);
                }
            }
        });
}

pub fn recent_parent_width(ctx: &egui::Context) -> f32 {
    measure_recent_parent_row_width(ctx)
}
