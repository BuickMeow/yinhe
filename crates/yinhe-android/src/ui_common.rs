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

/// 顶栏右侧名称框按钮：右起先留圆角空间，固定宽度截断按钮（名称过长显示省略号）。
/// 高度与其他顶栏按钮统一（24px）。
pub(crate) fn right_side_button(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    // 右起 12px：现代手机全面屏四角是圆的，按钮不能贴最右。
    ui.add_space(12.0);
    ui.add_sized(
        egui::vec2(width, 24.0),
        egui::Button::new(egui::RichText::new(text).size(13.0)).truncate(),
    )
}

/// 顶栏右侧名称框宽度（原全宽按钮的 1/3 取整，够显示 4-5 个字 + 省略号）。
pub(crate) const NAME_BTN_W: f32 = 120.0;

/// 量化设置弹窗（AR/PR 共用，量化值各自独立）：常用分数预设 + 自定义分数 + 自定义 tick。
/// 返回 Some = 用户选择了新值（弹窗保持打开可继续调，点外部关闭）。
/// 与桌面端 quantize_popup 同构：预设列表 → 自定义分数 → 自定义 tick。
pub(crate) fn quantize_popup(
    ctx: &egui::Context,
    id: &'static str,
    ppq: u32,
    current: yinhe_editor_core::quantize::QuantizePreset,
) -> Option<yinhe_editor_core::quantize::QuantizePreset> {
    use yinhe_editor_core::quantize::QuantizePreset;
    let mut pending: Option<QuantizePreset> = None;
    let mut open = true;
    egui::Window::new("量化")
        .id(egui::Id::new(id))
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .default_width(220.0)
        .show(ctx, |ui| {
            for preset in QuantizePreset::ALL {
                if ui
                    .selectable_label(*preset == current, preset.display_item(ppq))
                    .clicked()
                {
                    pending = Some(*preset);
                }
            }
            ui.separator();
            // ── 自定义时值 ──
            if let QuantizePreset::Fraction(num, den) = current {
                ui.label("自定义分数");
                ui.horizontal(|ui| {
                    ui.label("分子");
                    let mut n = num;
                    if ui
                        .add(egui::DragValue::new(&mut n).range(1..=9999))
                        .changed()
                    {
                        pending = Some(QuantizePreset::Fraction(n, den));
                    }
                    ui.label("分母");
                    let mut d = den;
                    if ui
                        .add(egui::DragValue::new(&mut d).range(1..=9999))
                        .changed()
                    {
                        pending = Some(QuantizePreset::Fraction(num, d.max(1)));
                    }
                });
            } else if ui.selectable_label(false, "自定义分数").clicked() {
                pending = Some(QuantizePreset::Fraction(1, 1));
            }
            ui.separator();
            // ── 自定义 tick ──
            if let QuantizePreset::Absolute(n) = current {
                ui.label("自定义 tick");
                let mut val = n;
                if ui
                    .add(egui::DragValue::new(&mut val).range(1..=99999))
                    .changed()
                {
                    pending = Some(QuantizePreset::Absolute(val));
                }
            } else if ui.selectable_label(false, "自定义 tick").clicked() {
                pending = Some(QuantizePreset::Absolute(1));
            }
        });
    if !open {
        // 用户点外部关闭：不返回 pending（本次调整不应用？桌面端是即时应用，
        // 拖动即改；点外部关闭不额外应用未完成的拖动值）。
        return None;
    }
    pending
}

/// 顶栏右侧编辑区（需包在 `Layout::right_to_left` 中，从右往左排）：
/// 名称框（最右）→ 撤销 → 重做 → 工具选择 → 量化。左侧留给播放区。
/// `name` 为名称框文本（工程名/Track 名），返回 (名称框点击, 量化按钮点击)。
pub(crate) fn right_edit_area(
    ui: &mut egui::Ui,
    app: &mut crate::app::YinheApp,
    name: &str,
    quantize: yinhe_editor_core::quantize::QuantizePreset,
) -> (bool, bool) {
    let name_clicked = right_side_button(ui, name, NAME_BTN_W).clicked();
    let (can_undo, can_redo) = app
        .doc
        .as_ref()
        .map(|d| (d.history.can_undo(), d.history.can_redo()))
        .unwrap_or((false, false));
    use egui_material_icons::icons::{ICON_REDO, ICON_UNDO};
    if ui
        .add_enabled(can_undo, egui::Button::new(icon_text(ICON_UNDO)))
        .on_hover_text("撤销")
        .clicked()
    {
        app.undo();
    }
    if ui
        .add_enabled(can_redo, egui::Button::new(icon_text(ICON_REDO)))
        .on_hover_text("重做")
        .clicked()
    {
        app.redo();
    }
    // 工具选择（显示当前工具图标，点击弹居中工具窗）。
    if ui
        .button(icon_text(app.tool.icon()))
        .on_hover_text(format!("工具：{}", app.tool.name()))
        .clicked()
    {
        app.tool_picker_open = !app.tool_picker_open;
    }
    // 量化按钮：显示当前量化（AR/PR 各自独立）。
    let mut quantize_clicked = false;
    if ui
        .button(egui::RichText::new(quantize.label()).size(13.0))
        .on_hover_text("量化")
        .clicked()
    {
        quantize_clicked = true;
    }
    (name_clicked, quantize_clicked)
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
