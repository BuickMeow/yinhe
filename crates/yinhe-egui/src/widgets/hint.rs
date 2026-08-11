use eframe::egui;

/// 空状态提示：右侧栏/对话框无内容时的弱化提示文字。
/// 统一"提示文字"的间距/颜色/字号（原散落在 5 个文件的重复代码）。
pub(crate) fn empty_hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text)
            .color(crate::theme::text_disabled())
            .size(crate::theme::BODY_FONT),
    );
}
