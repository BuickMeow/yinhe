//! ComboBox 的统一样式：无边框悬停 + 固定宽度。
//!
//! 复用 `menu::menu_item_button`（`stroke(NONE)` + `available_width`）保证
//! 下拉项与 `Popup` 菜单一致：hover 时仅背景变色，文字不位移。
//! 外层 `ComboBox` 与内层 `Ui` 同时锁宽，避免 `available_width → popup 宽度 → available_width` 正反馈
//! 导致的宽度抖动与 `Area::get_best_align` 翻转。

use eframe::egui;

/// 默认宽度（与设置页常用下拉一致，统一等宽），调用方可按内容选 70/160/200。
pub const DEFAULT_WIDTH: f32 = 200.0;

/// 固定宽度的 ComboBox，下拉项需配合 [`combo_item`] 使用。
/// `width` 为按钮与弹窗的统一宽度，`id` 为 `ComboBox::from_id_salt` 的盐（`&str` 或任意 `Hash`）。
///
/// # Example
/// ```ignore
/// combo_box(ui, "theme", theme_label, 160.0, |ui| {
///     for (v, label) in &options {
///         if combo_item(ui, *v == selected, label).clicked() { selected = v.clone(); }
///     }
/// });
/// ```
pub fn combo_box(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected_text: impl Into<egui::WidgetText>,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::InnerResponse<Option<()>> {
    // popup 高度取整行：viewport 高 = 可见行数 ×（行高 + 行距），底部恰好顶着下一行上沿，
    // 永不切出半截行（默认 200px 下 200 = 7×27+11，会把第 8 行切出 11px 的半截窄行）。
    // - 行高 24：`menu::menu_item_button` 的固定高度；
    // - 行距必须读全局 style（`ctx.global_style()`）：popup 是挂在 ctx 根上的独立 Area，
    //   只继承全局 style，调用方 `ui` 上的局部 `spacing_mut` 覆盖传不进去
    //  （`Popup::menu` 套的 `menu_style` 也只改 padding/描边，不动 `item_spacing`），
    //   用调用方行距会在设置页（局部 4.0）算出 196 而 popup 实际步长 27，仍留 7px 半截；
    // - 取 7 行：与原来可视行数相当（200px 放 7 整行 + 半截），内容不足一屏时
    //   `ScrollArea::auto_shrink` 收缩到内容高度，天然无半截，无需特殊处理。
    let row_step = 24.0 + ui.ctx().global_style().spacing.item_spacing.y;
    let height = 7.0 * row_step;
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected_text)
        .width(width)
        .height(height)
        .show_ui(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add_contents(ui);
        })
}

/// 下拉项：无边框、铺满整行，与 `menu::menu_item_button` 同样式。
/// 必须在 [`combo_box`] 的闭包内调用。
pub fn combo_item(
    ui: &mut egui::Ui,
    selected: bool,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.add(crate::widgets::menu::menu_item_button(ui, selected, label))
}

/// 在 `options` 中查找 `selected` 对应的显示文本（纯函数，便于单测）。
pub(crate) fn selected_label<'a, T: PartialEq>(
    options: &'a [(T, String)],
    selected: &T,
) -> Option<&'a str> {
    options
        .iter()
        .find(|(v, _)| v == selected)
        .map(|(_, s)| s.as_str())
}

/// DRY 封装：`selected: &mut T` + `options: &[(T, String)]` 一行完成 ComboBox。
/// 找不到对应文本时显示空（`debug_assert` 提示调用方 `options` 缺口），不依赖 `Debug`。
/// 下拉项无边框且固定宽度，返回是否发生改变。调用方只需处理副作用（如 `set_locale`）。
///
/// # Example
/// ```ignore
/// let opts = vec![(0u8, "Auto".to_owned()), (1, "1".to_owned())];
/// if combo_select(ui, "port", &mut port, 70.0, &opts) { /* changed */ }
/// ```
pub fn combo_select<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut T,
    width: f32,
    options: &[(T, String)],
) -> bool {
    let display = selected_label(options, selected).unwrap_or_else(|| {
        debug_assert!(
            false,
            "combo_select: selected 不在 options 中，检查 options 是否缺口"
        );
        ""
    });
    let mut changed = false;
    combo_box(ui, id, display.to_owned(), width, |ui| {
        for (value, label) in options {
            let is_selected = *value == *selected;
            if combo_item(ui, is_selected, label).clicked() {
                *selected = value.clone();
                changed = true;
            }
        }
    });
    changed
}

/// 使用 [`DEFAULT_WIDTH`] 的快捷版本（设置页/右键面板常用 160 宽）。
pub fn combo_select_auto<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut T,
    options: &[(T, String)],
) -> bool {
    combo_select(ui, id, selected, DEFAULT_WIDTH, options)
}

#[cfg(test)]
mod tests {
    use super::{selected_label, DEFAULT_WIDTH};

    #[test]
    fn selected_label_found() {
        let opts = vec![(1u8, "a".to_owned()), (2, "b".to_owned())];
        assert_eq!(selected_label(&opts, &2), Some("b"));
    }

    #[test]
    fn selected_label_missing_returns_none() {
        let opts = vec![(1u8, "a".to_owned())];
        assert_eq!(selected_label(&opts, &9), None);
    }

    #[test]
    fn selected_label_empty_options() {
        let opts: Vec<(u8, String)> = vec![];
        assert_eq!(selected_label(&opts, &1), None);
    }

    #[test]
    fn default_width_is_200() {
        assert_eq!(DEFAULT_WIDTH, 200.0);
    }

    /// 回归测试：含滚动条的 combo popup 不得出现半截行。
    ///
    /// 背景：`ComboBox` popup 内容包在 `ScrollArea` 里，默认 `max_height = spacing.combo_height = 200`。
    /// popup 是挂在 ctx 根上的独立 `Area`，行距取全局 style（默认 `item_spacing.y = 3.0`，
    /// 步长 24+3 = 27），200 = 7×27+11，viewport 底部会把第 8 行切出 11px 高的半截行
    /// （视觉上“最后一项特别窄”；`menu_item_button` 跟随剩余高度把它压成 h=11，
    /// 所以几何上它恰好顶着 clip 底、必须用行高断言才能抓住）。
    /// `combo_box` 按整行取高（7×步长）后 viewport 底部恰好顶着下一行上沿，无半截行。
    /// 10 个长文本选项保证内容超出 viewport（必出滚动条）。断言：
    /// a. 所有行宽度两两相等（容差 1px）；
    /// b. 没有任何一行横跨 clip 底边（`bottom ≤ clip_b+0.5` 或 `top ≥ clip_b-0.5`）；
    /// c. 每一行都是完整行高 24（容差 1px）——半截行的真正信号。
    #[test]
    fn popup_rows_have_no_half_clipped_row() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct RowGeo {
            w: f32,
            h: f32,
            top: f32,
            bottom: f32,
            clip_b: f32,
        }

        let ctx = egui::Context::default();
        ctx.add_font(egui_material_icons::font_insert());
        // `add_font` 下一 pass 才生效，先空跑两帧让字体度量稳定（行宽断言容差 1px）。
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
        }

        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        // （测试帧，popup 内容闭包记录的各行几何）：闭包只在打开帧运行，以此映射回测试帧。
        let runs: Rc<RefCell<Vec<(usize, Vec<RowGeo>)>>> = Rc::new(RefCell::new(Vec::new()));
        // `resp.inner.is_some()` 为真的测试帧（popup 打开，已验证可靠）。
        let opened: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
        let btn_center: Rc<RefCell<egui::Pos2>> = Rc::new(RefCell::new(egui::pos2(0.0, 0.0)));

        // frame 0 实测按钮位置；frame 1 hover；frame 2 按下；frame 3 松开（click 完成弹出）；
        // frame 4/5 保持 hover 让 popup 稳定。
        for frame in 0..6usize {
            let frame_now = frame;
            let mut raw = egui::RawInput {
                screen_rect: Some(screen),
                focused: true,
                ..Default::default()
            };
            let pos = *btn_center.borrow();
            match frame_now {
                1 | 4 | 5 => {
                    raw.events.push(egui::Event::PointerMoved(pos));
                }
                2 => {
                    raw.events.push(egui::Event::PointerMoved(pos));
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    });
                }
                3 => {
                    raw.events.push(egui::Event::PointerMoved(pos));
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    });
                }
                _ => {}
            }
            let runs_c = runs.clone();
            let opened_c = opened.clone();
            let btn_c = btn_center.clone();
            let mut out = ctx.run_ui(raw, |ui| {
                // 定点 Area：按钮位置逐帧稳定，click 坐标可用第 0 帧实测值。
                egui::Area::new(egui::Id::new("combo_half_row_reg"))
                    .fixed_pos(egui::pos2(60.0, 80.0))
                    .show(ui.ctx(), |ui| {
                        // 与设置页一致的行距：复现默认 200px 高度切出半截行的条件。
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let resp = super::combo_box(
                            ui,
                            "combo_half_row_reg",
                            "sel",
                            super::DEFAULT_WIDTH,
                            |ui| {
                                let mut rows = Vec::new();
                                for i in 0..10 {
                                    let clip_b = ui.clip_rect().max.y;
                                    let r = super::combo_item(
                                        ui,
                                        i == 0,
                                        format!("很长很长很长的选项文本 option {i} tail tail"),
                                    );
                                    rows.push(RowGeo {
                                        w: r.rect.width(),
                                        h: r.rect.height(),
                                        top: r.rect.min.y,
                                        bottom: r.rect.max.y,
                                        clip_b,
                                    });
                                }
                                runs_c.borrow_mut().push((frame_now, rows));
                            },
                        );
                        if resp.inner.is_some() {
                            opened_c.borrow_mut().push(frame_now);
                        }
                        *btn_c.borrow_mut() = resp.response.rect.center();
                    });
            });
            out.textures_delta.clear();
        }

        assert!(!opened.borrow().is_empty(), "popup 未打开，回归测试失败");
        let all_runs = runs.borrow();
        assert!(!all_runs.is_empty(), "popup 内容闭包未运行，回归测试失败");
        for (frame_now, rows) in all_runs.iter() {
            assert_eq!(rows.len(), 10, "第 {frame_now} 帧 popup 行数异常");
            // a. 所有行宽度两两相等（容差 1px）。
            let mut min_w = f32::MAX;
            let mut max_w = f32::MIN;
            for r in rows.iter() {
                if r.w < min_w {
                    min_w = r.w;
                }
                if r.w > max_w {
                    max_w = r.w;
                }
            }
            assert!(
                max_w - min_w <= 1.0,
                "第 {frame_now} 帧各行宽度不一致：min={min_w:.1} max={max_w:.1}"
            );
            // b. 没有任何一行横跨 clip 底边（要么完全在内，要么完全在下）。
            for (i, r) in rows.iter().enumerate() {
                let inside = r.bottom <= r.clip_b + 0.5;
                let below = r.top >= r.clip_b - 0.5;
                assert!(
                    inside || below,
                    "第 {frame_now} 帧第 {i} 行是半截行：top={:.1} bottom={:.1} clip_b={:.1}",
                    r.top,
                    r.bottom,
                    r.clip_b
                );
            }
            // c. 每一行都是完整行高 24（容差 1px）：半截行会被压成剩余高度，
            // 几何上顶着 clip 底而不横跨它，只有行高能抓住。
            for (i, r) in rows.iter().enumerate() {
                assert!(
                    (r.h - 24.0).abs() <= 1.0,
                    "第 {frame_now} 帧第 {i} 行高异常（半截行）：h={:.1}",
                    r.h
                );
            }
        }
    }
}
