//! 欢迎页：启动时无工程可打开的独立欢迎窗口，与主界面无工程占位共用同一内容。
//!
//! - 操作：新建工程 / 打开工程（文件选择器）/ 最近修改的工程列表 / 设置；
//! - 装饰：左下角「銀」字徽标水印（随主题前景色淡化）、右下角版本号。
//! - 两个入口（独立窗口 / 主界面占位）渲染相同内容，动作由调用方统一执行
//!   （`App::handle_welcome_action`），行为完全一致。

use std::collections::HashMap;

use eframe::egui;
use egui_material_icons::icons::*;
use rust_i18n::t;

use crate::chrome::transport_bar_recent::{format_opened_at, recent_display_name};

/// 欢迎页动作（独立窗口 / 主界面占位共用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WelcomeAction {
    NewProject,
    OpenProject,
    OpenRecent(String),
    OpenSettings,
}

/// 主按钮高度（逻辑像素，随字体缩放）。
const BTN_H: f32 = 38.0;
/// 设置（次要）按钮高度。
const BTN_H_SETTINGS: f32 = 32.0;
/// 最近列表行高估算（用于垂直居中；实际行高由 MenuItemButton 决定）。
const ROW_H: f32 = 26.0;
/// 最近列表最大高度（超出滚动）。
const RECENT_MAX_H: f32 = 280.0;
/// 内容列最大宽度。
const COL_MAX_W: f32 = 340.0;
/// 徽标水印不透明度（前景色淡化，淡淡地贴左下角）。
/// 档位参考：24 ≈ 9.4%、18 ≈ 7%、14 ≈ 5.5%、10 ≈ 3.9%（255 基准）。
const WATERMARK_ALPHA: u8 = 18;
/// 水印边长量化步长（逻辑像素）：拖动窗口时避免每帧重新光栅化 SVG。
const WATERMARK_SIZE_STEP: f32 = 64.0;

/// 欢迎窗口 viewport 标识。
pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("yinhe_welcome_dialog")
}

/// 「銀」徽标 SVG（欢迎页水印 / 关于窗口 / 启动页三处共用，避免重复 include）。
pub(crate) const YINHE_LOGO_SVG: &[u8] = include_bytes!("../../../assets/YINHE.svg");

/// 「銀」徽标图片（URI 带 `.svg` 以命中 SVG 加载器；颜色/透明度由调用方 `tint`）。
pub(crate) fn logo_image() -> egui::Image<'static> {
    egui::Image::from_bytes("bytes://yinhe-logo.svg", YINHE_LOGO_SVG)
}

/// 独立欢迎窗口（启动时无工程可打开 → 先让用户选择）。
///
/// 返回 `(closed, action)`：`closed` = 用户关闭窗口（无工程则退出应用）；
/// `action` = 本帧用户触发的动作。
pub(crate) fn show_window(
    ctx: &egui::Context,
    recent: &[String],
    recent_times: &HashMap<String, i64>,
) -> (bool, Option<WelcomeAction>) {
    let mut closed = false;
    let action = ctx.show_viewport_immediate(
        viewport_id(),
        crate::chrome::dialog::viewport_builder(&t!("welcome.title"), [620.0, 560.0], false),
        |ui, _class| {
            // 子窗口深浅与主窗口同步（与设置窗口一致）。
            crate::theme::apply_to_ctx(ui.ctx());
            if ui.input(|i| i.viewport().close_requested()) {
                closed = true;
            }
            let mut action = None;
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::theme::app_bg()))
                .show(ui, |ui| {
                    // 先内容（含水印），后标题栏：标题栏的标题/关闭按钮始终在最上层，
                    // 不被水印或背景覆盖。背景由 CentralPanel frame 铺满。
                    action = welcome_content(ui, recent, recent_times);
                    crate::chrome::dialog::title_bar(ui, &t!("welcome.title"), &mut closed, false);
                });
            action
        },
    );
    (closed, action)
}

/// 欢迎页内容（独立窗口 / 主界面占位共用）。返回本帧用户触发的动作。
pub(crate) fn welcome_content(
    ui: &mut egui::Ui,
    recent: &[String],
    recent_times: &HashMap<String, i64>,
) -> Option<WelcomeAction> {
    let rect = ui.max_rect();
    // 装饰（水印/版本号）直接绘制，不参与布局；背景由调用方铺垫
    // （独立窗口 = CentralPanel frame，主界面占位 = 占位面板自行铺底）。
    paint_watermark(ui, rect);
    paint_version(ui, rect);

    // 内容列：垂直居中；内容高于可用区时贴顶（最小 14 边距）。
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    let btn_h = scale(BTN_H);
    let set_h = scale(BTN_H_SETTINGS);
    let col_w = (rect.width() - 48.0).clamp(200.0, COL_MAX_W);
    let list_h = recent_list_height(ui.ctx(), recent.len());
    let total_h = btn_h + 10.0 + btn_h + 24.0 + 18.0 + 6.0 + list_h + 20.0 + set_h;
    let top = ((rect.height() - total_h) * 0.5).max(14.0);
    let col_rect = egui::Rect::from_min_size(
        egui::pos2(rect.center().x - col_w * 0.5, rect.min.y + top),
        egui::vec2(col_w, total_h),
    );

    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(col_rect), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

        if welcome_button(
            ui,
            t!("file.new_project").as_ref(),
            ICON_NOTE_ADD,
            true,
            col_w,
            BTN_H,
        )
        .clicked()
        {
            action = Some(WelcomeAction::NewProject);
        }
        ui.add_space(10.0);
        if welcome_button(
            ui,
            t!("welcome.open_project").as_ref(),
            ICON_FOLDER_OPEN,
            false,
            col_w,
            BTN_H,
        )
        .clicked()
        {
            action = Some(WelcomeAction::OpenProject);
        }

        ui.add_space(24.0);
        ui.label(
            egui::RichText::new(t!("menu.recent_files").to_string())
                .size(scale(crate::theme::SMALL_FONT))
                .color(crate::theme::text_label()),
        );
        ui.add_space(6.0);
        if recent.is_empty() {
            ui.label(
                egui::RichText::new(t!("welcome.no_recent").to_string())
                    .size(scale(crate::theme::BODY_FONT))
                    .color(crate::theme::text_disabled()),
            );
        } else {
            egui::ScrollArea::vertical()
                .id_salt("welcome_recent_scroll")
                .max_height(RECENT_MAX_H)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for path in recent {
                        if recent_row(ui, path, recent_times).clicked() {
                            action = Some(WelcomeAction::OpenRecent(path.clone()));
                        }
                    }
                });
        }

        ui.add_space(20.0);
        // 设置：次要小按钮，半宽居中。
        let set_w = (col_w * 0.55).clamp(120.0, 180.0);
        ui.horizontal(|ui| {
            ui.add_space((col_w - set_w) * 0.5);
            if welcome_button(
                ui,
                t!("file.settings").as_ref(),
                ICON_SETTINGS,
                false,
                set_w,
                BTN_H_SETTINGS,
            )
            .clicked()
            {
                action = Some(WelcomeAction::OpenSettings);
            }
        });
    });
    action
}

/// 最近列表区高度估算（垂直居中用；实际行高由控件决定，近似即可）。
fn recent_list_height(ctx: &egui::Context, count: usize) -> f32 {
    if count == 0 {
        crate::theme::BODY_FONT + 4.0
    } else {
        let row_h = crate::scaling::scaled_font(ctx, ROW_H);
        (count as f32 * (row_h + 2.0)).min(RECENT_MAX_H)
    }
}

/// 欢迎页主按钮：全宽/定宽自绘，视觉与对话框底部按钮一致
/// （主按钮 = 强调底 + 强调字，次按钮 = 普通底 + 主文字）。
fn welcome_button(
    ui: &mut egui::Ui,
    text: &str,
    icon: egui_material_icons::MaterialIcon,
    primary: bool,
    width: f32,
    height: f32,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let fg = if primary {
        crate::theme::accent_active()
    } else {
        crate::theme::text_primary()
    };
    let job = crate::widgets::icon_text::icon_text(icon, text, crate::theme::SUB_TITLE_FONT, fg);
    let galley = ctx.fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(width, crate::scaling::scaled_font(&ctx, height)),
        egui::Sense::click(),
    );
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));

    if ui.is_rect_visible(rect) {
        let base = if primary {
            crate::theme::selected_bg()
        } else {
            crate::theme::btn_bg()
        };
        let fill = if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(base)
        } else if resp.hovered() {
            crate::theme::hover_color(base)
        } else {
            base
        };
        let radius = crate::scaling::scaled_font(&ctx, 6.0);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            crate::widgets::control::control_stroke(true, false),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    resp
}

/// 最近工程列表行：文件名 + 右侧上次打开时间；hover 显示完整路径。
fn recent_row(ui: &mut egui::Ui, path: &str, times: &HashMap<String, i64>) -> egui::Response {
    let name = recent_display_name(path);
    let job = crate::widgets::icon_text::icon_text(
        ICON_DESCRIPTION,
        name,
        crate::theme::FILE_MENU_FONT,
        crate::theme::text_primary(),
    );
    let mut row = crate::widgets::menu::menu_item_button(ui, false, job);
    if let Some(&secs) = times.get(path) {
        let time = format_opened_at(secs);
        if !time.is_empty() {
            row = row.shortcut_text(
                egui::RichText::new(time)
                    .size(crate::scaling::scaled_font(
                        ui.ctx(),
                        crate::theme::SMALL_LABEL_FONT,
                    ))
                    .color(crate::theme::text_disabled()),
            );
        }
    }
    let resp = ui.add(row);
    // 独立窗口无讲解行，hover 用鼠标旁 tooltip；主界面占位用同款保证一致。
    let hint = match times.get(path) {
        Some(&secs) => format!(
            "{} · {}",
            path,
            t!("hint.recent_opened_at", time = format_opened_at(secs))
        ),
        None => path.to_string(),
    };
    resp.on_hover_text(hint)
}

/// 左下角「銀」字徽标水印：贴左/下边，随主题前景色淡化。
fn paint_watermark(ui: &egui::Ui, rect: egui::Rect) {
    let margin = 12.0;
    let max_side = (rect.height() - 2.0 * margin).max(0.0);
    let side = (rect.height() * 0.9).min(rect.width() * 0.5).min(max_side);
    if side < 32.0 {
        return;
    }
    // 量化边长：拖动窗口时避免每帧重新光栅化 SVG（SvgLoader 按尺寸缓存）。
    let side = ((side / WATERMARK_SIZE_STEP).ceil() * WATERMARK_SIZE_STEP).min(max_side);
    let wm_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + margin, rect.max.y - margin - side),
        egui::pos2(rect.min.x + margin + side, rect.max.y - margin),
    );
    let c = crate::theme::text_primary();
    let tint = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), WATERMARK_ALPHA);
    logo_image()
        .tint(tint)
        .fit_to_exact_size(wm_rect.size())
        .paint_at(ui, wm_rect);
}

/// 右下角版本号。
fn paint_version(ui: &egui::Ui, rect: egui::Rect) {
    let text = format!("v{}", env!("CARGO_PKG_VERSION"));
    let font = egui::FontId::proportional(crate::scaling::scaled_font(
        ui.ctx(),
        crate::theme::SMALL_FONT,
    ));
    let color = crate::theme::text_muted();
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let pos = egui::pos2(
        rect.max.x - 12.0 - galley.size().x,
        rect.max.y - 10.0 - galley.size().y,
    );
    ui.painter().galley(pos, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    #[derive(Default)]
    struct St {
        action: Option<WelcomeAction>,
    }

    fn harness(recent: Vec<&str>) -> Harness<'static, St> {
        let recent: Vec<String> = recent.into_iter().map(str::to_string).collect();
        let mut first_frame = true;
        Harness::builder()
            .with_size(egui::vec2(620.0, 560.0))
            .build_ui_state(
                move |ui, st| {
                    // 第一帧只注册 material-icons 字体（add_font 下一 pass 才生效，
                    // 若同帧渲染图标按钮会 panic），后续帧才渲染欢迎页。
                    if std::mem::take(&mut first_frame) {
                        ui.ctx().add_font(egui_material_icons::font_insert());
                        return;
                    }
                    let times = HashMap::new();
                    // 多帧 run 时后续帧返回 None，需累积记录。
                    if let Some(a) = welcome_content(ui, &recent, &times) {
                        st.action = Some(a);
                    }
                },
                St::default(),
            )
    }

    /// harness 为 `'static`，查询 label 需 `&'static str`；测试内泄漏转换。
    fn leak_label(key: &str) -> &'static str {
        Box::leak(key.to_string().into_boxed_str())
    }

    #[test]
    fn new_project_button_returns_action() {
        let mut h = harness(vec![]);
        h.run();
        h.get_by_label(leak_label(t!("file.new_project").as_ref()))
            .click();
        h.run();
        assert_eq!(h.state().action, Some(WelcomeAction::NewProject));
    }

    #[test]
    fn open_project_button_returns_action() {
        let mut h = harness(vec![]);
        h.run();
        h.get_by_label(leak_label(t!("welcome.open_project").as_ref()))
            .click();
        h.run();
        assert_eq!(h.state().action, Some(WelcomeAction::OpenProject));
    }

    #[test]
    fn settings_button_returns_action() {
        let mut h = harness(vec![]);
        h.run();
        h.get_by_label(leak_label(t!("file.settings").as_ref()))
            .click();
        h.run();
        assert_eq!(h.state().action, Some(WelcomeAction::OpenSettings));
    }

    #[test]
    fn recent_row_returns_path() {
        let mut h = harness(vec!["/tmp/demo.yin"]);
        h.run();
        // 行节点 label 含前缀图标码点（LayoutJob 混排），用包含匹配。
        h.get_by_label_contains("demo.yin").click();
        h.run();
        assert_eq!(
            h.state().action,
            Some(WelcomeAction::OpenRecent("/tmp/demo.yin".to_string()))
        );
    }

    #[test]
    fn empty_recent_shows_hint() {
        let mut h = harness(vec![]);
        h.run();
        let hint = leak_label(t!("welcome.no_recent").as_ref());
        assert!(h.query_by_label(hint).is_some());
    }
}
