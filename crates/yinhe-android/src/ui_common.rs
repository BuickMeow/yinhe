//! 共享 UI 工具：图标文字、轨道颜色、顶栏构建（菜单/AR/PR 三页共用）。

use eframe::egui;
use yinhe_core::YinModel;

/// 图标按钮文字（Material Icons 字形，走带/工具条用）。
pub(crate) fn icon_text(icon: egui_material_icons::MaterialIcon) -> egui::RichText {
    egui::RichText::new(icon.codepoint)
        .family(icon.font_family())
        .size(18.0)
}

/// 轨道颜色：TRACK_PALETTE 循环分配（与桌面端 track_panel/AR 一致）。
/// PR 与 AR 共用，保证同一工程两个视图的轨道色相同。
pub(crate) fn track_colors_for(model: &YinModel) -> Vec<[f32; 4]> {
    yinhe_theme::palette::TRACK_PALETTE
        .iter()
        .cycle()
        .take(model.tracks.len())
        .map(|&[r, g, b]| [r, g, b, 1.0])
        .collect()
}

/// 顶栏：默认面板背景色 + 挖孔安全区避让 + 对称内边距（按钮垂直居中）。
/// 三个页面（菜单/AR/PR）共用，保证视觉一致。
pub(crate) fn show_toolbar(
    ui: &mut egui::Ui,
    id: &'static str,
    safe: [f32; 4],
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let [sl, st, sr, _] = safe;
    egui::Panel::top(id)
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
        .show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            // frame margin 是 i8（放不下大 inset），手动缩进：上下对称 8px。
            let inner = egui::Rect::from_min_max(
                avail.min + egui::vec2(sl + 8.0, st + 8.0),
                avail.max - egui::vec2(sr + 8.0, 8.0),
            );
            if inner.width() <= 0.0 || inner.height() <= 0.0 {
                return;
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                // 左对齐（不走 horizontal_centered）：页面可在右侧用
                // right_to_left 布局放名称/状态按钮。
                ui.horizontal(|ui| {
                    add_contents(ui);
                });
            });
        });
}

/// 顶栏右侧按钮：右起先留圆角空间，再放一个全宽截断按钮（名称过长显示省略号）。
/// 调用方需自行包在 `Layout::right_to_left` 中。
pub(crate) fn right_side_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    // 右起 12px：现代手机全面屏四角是圆的，按钮不能贴最右。
    ui.add_space(12.0);
    let w = ui.available_width().max(48.0);
    ui.add_sized(
        egui::vec2(w, 26.0),
        egui::Button::new(egui::RichText::new(text).size(14.0)).truncate(),
    )
}

/// 页面背景：整个可用区域（含挖孔区域）铺默认面板背景色。
/// 调用时机：每个页面的 CentralPanel 内容开头。
pub(crate) fn fill_page_background(ui: &mut egui::Ui) {
    ui.painter().rect_filled(
        ui.available_rect_before_wrap(),
        0.0,
        ui.visuals().panel_fill,
    );
}
