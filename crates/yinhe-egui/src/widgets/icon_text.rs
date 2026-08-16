use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

/// 构造 "Material 图标 + 普通文本" 混排文本。
///
/// 图标码点位于 PUA 区，必须用 `material-icons` 家族渲染——否则会被
/// Pretendard/MiSans 的私有字形抢占（它们也在 PUA 区定义了字形），
/// 显示成奇怪的方框/数字。文字走 Proportional，因此拆成两个 section。
/// 返回 LayoutJob（而非 WidgetText）：宽度测量可直接复用同一构造；
/// 渲染处由 From<LayoutJob> for WidgetText 自动转换。
pub(crate) fn icon_text(
    icon: egui_material_icons::MaterialIcon,
    text: &str,
    size: f32,
    color: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        icon.codepoint,
        0.0,
        TextFormat {
            font_id: FontId::new(size, icon.font_family()),
            color,
            ..Default::default()
        },
    );
    let text = format!(" {text}");
    job.append(
        &text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    job
}
