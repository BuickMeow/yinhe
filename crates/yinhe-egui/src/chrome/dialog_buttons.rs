//! 对话框底部按钮统一控件。
//!
//! 所有子窗口对话框的动作按钮（确定/取消/保存/丢弃…）都经由此处绘制，
//! 保证跨弹窗一致：固定高度与圆角、主次配色、间距统一、整行右对齐
//! （macOS 惯例，主操作在最右，破坏性操作靠左）。
//!
//! 尺寸随字体缩放（`scaling::scaled_font`）同比例放大，避免大字号下拥挤。

use eframe::egui;

/// 按钮高度（逻辑像素，随字体缩放）。
const BTN_H: f32 = 30.0;
/// 按钮最小宽度（逻辑像素，随字体缩放）。
const BTN_MIN_W: f32 = 76.0;
/// 按钮水平内边距（单侧，逻辑像素，随字体缩放）。
const BTN_PAD_X: f32 = 14.0;
/// 按钮圆角。
const BTN_RADIUS: f32 = 6.0;
/// 按钮间距（逻辑像素，随字体缩放）。
const BTN_GAP: f32 = 8.0;
/// 底部按钮区建议预留高度（按钮行 + 上间距），随字体缩放。
pub(crate) const BTN_ZONE_H: f32 = 40.0;

/// 按钮语义（决定配色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogButtonKind {
    /// 主操作：确定/保存/创建/开始导出，强调色实底。
    Primary,
    /// 普通操作：取消/关闭/返回/重置等。
    Secondary,
    /// 破坏性操作：丢弃/退出，红色文字警示。
    Danger,
}

/// 一个底部按钮的描述。
pub(crate) struct DialogButton<'a> {
    pub text: &'a str,
    pub kind: DialogButtonKind,
    pub enabled: bool,
}

impl<'a> DialogButton<'a> {
    pub fn primary(text: &'a str) -> Self {
        Self {
            text,
            kind: DialogButtonKind::Primary,
            enabled: true,
        }
    }

    pub fn secondary(text: &'a str) -> Self {
        Self {
            text,
            kind: DialogButtonKind::Secondary,
            enabled: true,
        }
    }

    pub fn danger(text: &'a str) -> Self {
        Self {
            text,
            kind: DialogButtonKind::Danger,
            enabled: true,
        }
    }

    /// 置灰（不可点击）。
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// 底部按钮行建议预留高度（供 `content_with_bottom_buttons` 的 `btn_zone_h`）。
pub(crate) fn btn_zone_h(ctx: &egui::Context) -> f32 {
    crate::scaling::scaled_font(ctx, BTN_ZONE_H)
}

/// 绘制按钮行：整行右对齐，`buttons` 按「左→右」给出，最后一个最靠右。
///
/// 返回本帧被点击按钮在 `buttons` 中的索引。
pub(crate) fn dialog_button_row(ui: &mut egui::Ui, buttons: &[DialogButton<'_>]) -> Option<usize> {
    let mut clicked = None;
    let gap = crate::scaling::scaled_font(ui.ctx(), BTN_GAP);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (idx, spec) in buttons.iter().enumerate().rev() {
                if dialog_button(ui, spec).clicked() {
                    clicked = Some(idx);
                }
                if idx > 0 {
                    ui.add_space(gap);
                }
            }
        });
    });
    clicked
}

/// 主按钮配色：完整对齐 transport bar 播放菜单选中项——
/// `selected_bg` 底 + 强调色文字（菜单项图标与文字同用 `accent_active`）。
/// `selected_bg` 由主题派生，亮暗自动适配。
fn primary_colors() -> (egui::Color32, egui::Color32) {
    (crate::theme::selected_bg(), crate::theme::accent_active())
}

/// 绘制单个按钮（需要自定义排布时使用；一般用 [`dialog_button_row`]）。
///
/// 自绘原因：统一高度/圆角/hover 与按下反馈；`egui::Button` 无法在指定
/// 填充色的同时保持主题一致的交互变色。
pub(crate) fn dialog_button(ui: &mut egui::Ui, spec: &DialogButton<'_>) -> egui::Response {
    let ctx = ui.ctx().clone();
    let scale = |v: f32| crate::scaling::scaled_font(&ctx, v);
    let (primary_bg, primary_fg) = primary_colors();
    let fg = if !spec.enabled {
        crate::theme::text_disabled()
    } else {
        match spec.kind {
            DialogButtonKind::Primary => primary_fg,
            DialogButtonKind::Secondary => crate::theme::text_primary(),
            DialogButtonKind::Danger => crate::theme::danger_text_bright(),
        }
    };
    let galley = ui.painter().layout_no_wrap(
        spec.text.to_owned(),
        egui::FontId::proportional(scale(crate::theme::SUB_TITLE_FONT)),
        fg,
    );
    let width = (galley.size().x + scale(BTN_PAD_X) * 2.0).max(scale(BTN_MIN_W));
    let sense = if spec.enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, scale(BTN_H)), sense);
    let resp = if spec.enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, spec.enabled, spec.text)
    });

    if ui.is_rect_visible(rect) {
        let base = match spec.kind {
            DialogButtonKind::Primary => primary_bg,
            DialogButtonKind::Secondary | DialogButtonKind::Danger => crate::theme::btn_bg(),
        };
        let fill = if !spec.enabled {
            crate::theme::btn_bg()
        } else if resp.is_pointer_button_down_on() {
            crate::theme::pressed_color(base)
        } else if resp.hovered() {
            crate::theme::hover_color(base)
        } else {
            base
        };
        let radius = scale(BTN_RADIUS);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, fill);
        painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(1.5),
                radius + 1.5,
                egui::Stroke::new(1.5, crate::theme::accent_active().gamma_multiply(0.9)),
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    #[derive(Default)]
    struct St {
        clicked: Option<usize>,
    }

    fn harness() -> Harness<'static, St> {
        let mut first = true;
        Harness::builder()
            .with_size(egui::vec2(400.0, 120.0))
            .build_ui_state(
                move |ui, st| {
                    if std::mem::take(&mut first) {
                        return;
                    }
                    let zone = btn_zone_h(ui.ctx());
                    crate::chrome::dialog::content_with_bottom_buttons(
                        ui,
                        zone,
                        |_ui| {},
                        |ui| {
                            ui.add_space(8.0);
                            // 多帧 run 时后续帧会返回 None，需累积记录。
                            if let Some(idx) = dialog_button_row(
                                ui,
                                &[
                                    DialogButton::secondary("Cancel"),
                                    DialogButton::primary("OK"),
                                ],
                            ) {
                                st.clicked = Some(idx);
                            }
                        },
                    );
                },
                St::default(),
            )
    }

    #[test]
    fn row_is_right_aligned() {
        let mut h = harness();
        h.run();
        let ok = h.get_by_label("OK").rect();
        let cancel = h.get_by_label("Cancel").rect();
        assert!(
            ok.center().x > cancel.center().x,
            "OK 应在 Cancel 右侧: ok={ok:?} cancel={cancel:?}"
        );
        assert!(
            ok.right() > 385.0,
            "最右按钮应贴齐右边界（约 392），实际 right={}",
            ok.right()
        );
    }

    #[test]
    fn click_returns_index() {
        let mut h = harness();
        h.run();
        h.get_by_label("OK").click();
        h.run();
        assert_eq!(h.state().clicked, Some(1), "点击最右按钮应返回索引 1");

        h.get_by_label("Cancel").click();
        h.run();
        assert_eq!(h.state().clicked, Some(0), "点击左按钮应返回索引 0");
    }

    #[test]
    fn buttons_share_baseline_and_size() {
        let mut h = harness();
        h.run();
        let ok = h.get_by_label("OK").rect();
        let cancel = h.get_by_label("Cancel").rect();
        assert!(
            (ok.center().y - cancel.center().y).abs() < 1.0,
            "按钮应处于同一行: ok={ok:?} cancel={cancel:?}"
        );
        assert!(
            (ok.height() - BTN_H).abs() < 1.0,
            "按钮高度应统一为 BTN_H，实际 {}",
            ok.height()
        );
    }
}
