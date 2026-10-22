//! 关于窗口：徽标、名称版本、作者与版权、技术栈、项目链接、许可证。
//!
//! 独立视口窗口（与设置/欢迎窗口同套路）：`dialogs::about::show_viewport`
//! 由主循环与欢迎阶段每帧调用，`show_about` 为 false 时零开销。

use eframe::egui;
use rust_i18n::t;

/// 关于窗口 viewport 标识（打开动作与渲染共用，见 `FileAction::About`）。
pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("about_dialog")
}

/// 显示关于窗口；`show` 为 false 时直接返回。
pub(crate) fn show_viewport(ctx: &egui::Context, show: &mut bool) {
    if !*show {
        return;
    }
    let title = t!("menu.about");

    ctx.show_viewport_immediate(
        viewport_id(),
        crate::chrome::dialog::viewport_builder(title.as_ref(), [400.0, 430.0], false),
        |vctx, _class| {
            crate::theme::apply_to_ctx(vctx.ctx());
            if vctx.input(|i| i.viewport().close_requested()) {
                *show = false;
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            let mut hide_requested = false;
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::theme::app_bg()))
                .show(vctx, |ui| {
                    let mut title_close = false;
                    crate::chrome::dialog::title_bar(ui, title.as_ref(), &mut title_close, false);
                    if title_close {
                        *show = false;
                        hide_requested = true;
                    }
                    content(ui);
                });
            if hide_requested {
                vctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        },
    );
}

/// 关于内容：居中列。
fn content(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    ui.add_space(scale(28.0));
    ui.vertical_centered(|ui| {
        // 「銀」徽标（随主题前景色）。
        let logo_side = scale(104.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(logo_side, logo_side), egui::Sense::hover());
        crate::welcome::logo_image()
            .tint(crate::theme::text_primary())
            .fit_to_exact_size(rect.size())
            .paint_at(ui, rect);

        ui.add_space(scale(8.0));
        ui.label(
            egui::RichText::new("Yinhe")
                .size(scale(24.0))
                .color(crate::theme::text_primary()),
        );
        ui.add_space(scale(2.0));
        ui.label(
            egui::RichText::new(crate::version::version())
                .size(scale(crate::theme::SMALL_FONT))
                .color(crate::theme::text_muted()),
        );

        ui.add_space(scale(14.0));
        // 细分割线（限定宽度，居中）。
        let (sep_rect, _) =
            ui.allocate_exact_size(egui::vec2(scale(220.0), 1.0), egui::Sense::hover());
        ui.painter().hline(
            sep_rect.x_range(),
            sep_rect.center().y,
            egui::Stroke::new(1.0, crate::theme::line_fg()),
        );
        ui.add_space(scale(14.0));

        let line = |ui: &mut egui::Ui, text: String, color: egui::Color32| {
            ui.label(
                egui::RichText::new(text)
                    .size(scale(crate::theme::BODY_FONT))
                    .color(color),
            );
        };
        line(
            ui,
            t!("about.author").to_string(),
            crate::theme::text_secondary(),
        );
        ui.add_space(scale(4.0));
        line(
            ui,
            t!("about.copyright").to_string(),
            crate::theme::text_secondary(),
        );
        ui.add_space(scale(4.0));
        line(ui, t!("about.tech").to_string(), crate::theme::text_muted());
        ui.add_space(scale(6.0));
        // 项目链接（点击用系统浏览器打开）。
        ui.hyperlink_to(
            egui::RichText::new("github.com/buickmeow/yinhe").size(scale(crate::theme::BODY_FONT)),
            "https://github.com/buickmeow/yinhe",
        );
        ui.add_space(scale(4.0));
        line(
            ui,
            t!("about.license").to_string(),
            crate::theme::text_muted(),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 关于窗口内容离屏渲染冒烟测试：绘制（含「銀」徽标 SVG）不 panic。
    /// 设 `YINHE_ABOUT_PREVIEW=1` 时另存 PNG 供人工查看。
    #[test]
    fn render_about_smoke() {
        let mut first = true;
        let mut h = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_accessibility_check(false)
            .with_size(egui::vec2(400.0, 430.0))
            .build_ui_state(
                move |ui, _state: &mut ()| {
                    // 首帧：注册图片加载器 + 系统字体（中文字形），次帧起渲染内容。
                    if std::mem::take(&mut first) {
                        egui_extras::install_image_loaders(ui.ctx());
                        crate::fonts::install(
                            ui.ctx(),
                            yinhe_fonts::WEIGHT_MEDIUM,
                            rust_i18n::locale().as_ref(),
                        );
                        return;
                    }
                    content(ui);
                },
                (),
            );
        h.step();
        h.step();
        if std::env::var("YINHE_ABOUT_PREVIEW").is_ok()
            && let Ok(img) = h.render()
        {
            let _ = img.save("/tmp/yinhe_about_render.png");
        }
    }
}
