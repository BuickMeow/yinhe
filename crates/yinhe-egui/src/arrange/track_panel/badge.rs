use eframe::egui;

use super::types::Anchor;

/// 在色带图标位置（与 chevron 同坐标、同尺寸、同绘制方式）画一个图标，点击弹菜单。
///
/// 图标用 `painter.text` 严格 `CENTER_CENTER` 居中（同 chevron）。点击时用固定的
/// popup id（`arr_add_pop_{track}`）打开，并把加号中心的**屏幕坐标**存为锚点；
/// popup 用 `Popup::new(id, ?, Position(固定锚点), ...)` 渲染——锚点不与 hover 行绑定，
/// 只要该加号在 popup 打开期间持续渲染（调用方用 `add_open` 保证），popup 就会稳定
/// 落在加号旁固定位置，鼠标移向菜单不会漂移或消失。
pub fn badge_icon_menu(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    codepoint: &str,
    family: egui::FontFamily,
    color: egui::Color32,
    track: usize,
    body: impl FnOnce(&mut egui::Ui),
) {
    let size = egui::vec2(12.0, 16.0);
    let rect = egui::Rect::from_center_size(center, size);
    let popup_id = egui::Id::new(("arr_add_pop", track));
    let resp = ui.interact(
        rect,
        egui::Id::new(("badge_icon", track)),
        egui::Sense::click(),
    );
    // 图标严格水平+垂直居中对齐（同 chevron），不会因按钮 padding 右偏。
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, family),
        color,
    );
    // 点击：打开 popup 并在 memory 记下加号中心屏幕坐标作为固定锚点。
    if resp.clicked() {
        // rect 是 ui 局部坐标，转成全局屏幕坐标。
        let screen_center = ui
            .ctx()
            .layer_transform_to_global(resp.layer_id)
            .map(|t| t * rect.center())
            .unwrap_or(rect.center());
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                Anchor::key(),
                Anchor {
                    track,
                    pos: screen_center,
                },
            )
        });
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
    // 打开状态下渲染 popup（open_memory(None)：不干预 memory 的开关）。
    if egui::Popup::is_id_open(ui.ctx(), popup_id)
        && let Some(anchor) = ui.ctx().data_mut(|d| d.get_temp::<Anchor>(Anchor::key()))
        && anchor.track == track
    {
        egui::Popup::new(
            popup_id,
            ui.ctx().clone(),
            anchor.pos, // impl From<Pos2> → PopupAnchor::Position
            egui::LayerId::new(egui::Order::Middle, egui::Id::new("arr_add_popup_layer")),
        )
        .frame(egui::Frame::menu(ui.style())) // 只一层菜单轮廓，不再由 body 内部再套
        .open_memory(None)
        .show(|ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(160.0);
            body(ui);
        });
    }
}
