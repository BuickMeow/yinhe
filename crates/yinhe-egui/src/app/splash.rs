//! 启动页（splash）：启动时先显示的小窗口（左下角 Yinhe 大字 + 銀徽标、
//! 右下角流程文案与 Spinner），音频/文件就绪后消失。
//!
//! 实现依赖 eframe 的 deferred viewport：启动页是独立原生窗口，由主 viewport
//! 每帧声明存在（`show_viewport_deferred`），就绪后停止声明即随下一帧销毁。
//! 本模块只负责"怎么画"；启动状态机在 `startup.rs`。

use std::sync::atomic::Ordering;

use eframe::egui;

use super::startup::StartupShared;

/// 启动页 viewport 标识（仅用于 `ViewportId::from_hash_of`）。
const SPLASH_VIEWPORT_ID: &str = "yinhe_startup_splash";

pub(crate) fn splash_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of(SPLASH_VIEWPORT_ID)
}

pub(crate) fn splash_viewport_builder() -> egui::ViewportBuilder {
    let vb = egui::ViewportBuilder::default()
        .with_title(&*rust_i18n::t!("startup.title"))
        .with_inner_size([420.0, 230.0])
        .with_resizable(false);

    // macOS：保留系统窗口框架（系统圆角 + 系统阴影 + 可拖动），只隐藏标题与
    // 交通灯——titled 窗口 + fullsize content view + 透明标题栏（Safari/Chrome
    // 同款做法）；`decorations(false)` 会丢掉圆角和阴影，不可用。
    #[cfg(target_os = "macos")]
    let vb = vb
        .with_transparent(true)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(false);

    // 其他平台：无边框窗口（无最小化-最大化-关闭按钮）。
    #[cfg(not(target_os = "macos"))]
    let vb = vb.with_decorations(false);

    vb
}

/// 启动页绘制（deferred viewport 闭包；不能访问 `App`）。
///
/// 布局：大号 "Yinhe" + 「銀」徽标放左下角，当前流程（Spinner + 文案）放右下角。
pub(crate) fn draw_splash(ui: &mut egui::Ui, shared: &StartupShared) {
    // 无系统按钮：关闭请求 / Cmd+Q 是退出方式。
    if ui.input(|i| i.viewport().close_requested())
        || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q))
    {
        shared.exit_requested.store(true, Ordering::Relaxed);
    }

    let status = shared
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let painter = ui.painter().clone();
    let rect = ui.max_rect();
    // 窗口无装饰：全窗口自绘背景。
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());

    // 左右下三边同边距；文字按字形实际边界（mesh_bounds）对齐，
    // 消除字体行高下行留白带来的"下方边距偏大"。
    let inner = rect.shrink(18.0);

    // 左下角：大号 "Yinhe"（背景字）+ 其下方「銀」徽标（品牌字，贴底）。
    // 徽标半透明淡化（前景色），与右侧流程文案形成主次。
    let logo_side = 56.0;
    let logo_rect = egui::Rect::from_min_size(
        egui::pos2(inner.left(), inner.bottom() - logo_side),
        egui::vec2(logo_side, logo_side),
    );
    let logo_tint = {
        let c = crate::theme::text_primary();
        egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 150)
    };
    crate::welcome::logo_image()
        .tint(logo_tint)
        .fit_to_exact_size(logo_rect.size())
        .paint_at(ui, logo_rect);

    let title_color = crate::theme::text_primary();
    let title = painter.layout_no_wrap(
        rust_i18n::t!("startup.title").to_string(),
        egui::FontId::proportional(72.0),
        title_color,
    );
    let title_pos = egui::pos2(
        inner.left() - title.mesh_bounds.left(),
        logo_rect.top() - 4.0 - title.mesh_bounds.bottom(),
    );
    painter.galley(title_pos, title, title_color);

    // 右下角：Spinner + 当前流程文案。
    let text_color = crate::theme::text_secondary();
    let status = painter.layout_no_wrap(status, egui::FontId::proportional(14.0), text_color);
    let status_pos = egui::pos2(
        inner.right() - status.mesh_bounds.right(),
        inner.bottom() - status.mesh_bounds.bottom(),
    );
    let status_center_y = inner.bottom() - status.mesh_bounds.height() / 2.0;
    let status_left = status_pos.x + status.mesh_bounds.left();
    painter.galley(status_pos, status, text_color);

    let spinner_size = 16.0;
    let spinner_rect = egui::Rect::from_center_size(
        egui::pos2(status_left - 8.0 - spinner_size / 2.0, status_center_y),
        egui::vec2(spinner_size, spinner_size),
    );
    egui::Spinner::new()
        .size(spinner_size)
        .color(crate::theme::accent_active())
        .paint_at(ui, spinner_rect);

    // Spinner 动画 + 主线程状态写入的刷新。
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动页离屏渲染冒烟测试：绘制（含「銀」徽标 SVG）不 panic。
    /// 设 `YINHE_SPLASH_PREVIEW=1` 时另存 PNG 供人工查看。
    #[test]
    fn render_splash_smoke() {
        use std::sync::atomic::AtomicBool;

        let mut first = true;
        let mut h = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(420.0, 230.0))
            .build_ui_state(
                move |ui, _state: &mut ()| {
                    // 首帧注册图片加载器（SVG），次帧起渲染启动页。
                    if std::mem::take(&mut first) {
                        egui_extras::install_image_loaders(ui.ctx());
                        return;
                    }
                    let shared = StartupShared {
                        status: std::sync::Mutex::new(
                            rust_i18n::t!("startup.preparing").to_string(),
                        ),
                        exit_requested: AtomicBool::new(false),
                    };
                    draw_splash(ui, &shared);
                },
                (),
            );
        // splash 每帧 request_repaint（spinner 动画）：用固定步数推进，
        // 不用 `run`（会因持续重绘请求触发 max_steps 断言）。
        h.step();
        h.step();
        if std::env::var("YINHE_SPLASH_PREVIEW").is_ok()
            && let Ok(img) = h.render()
        {
            let _ = img.save("/tmp/yinhe_splash_render.png");
        }
    }
}
