//! 设置行几何回归：控件必须完整落在行内，不能被右边界裁掉。
//!
//! 用户报告：通知 / MIDI 导出等页的开关、步进器、下拉框被推到窗口右缘之外，
//! 只剩一半。根因是行内两列布局没有把「标签列宽」从可用宽度里扣掉。

use eframe::egui;

/// 返回 `(行矩形, 控件矩形)`（无说明的单行形态）。
fn measure(control_w: f32) -> (egui::Rect, egui::Rect) {
    measure_with_hint(control_w, "")
}

/// 返回 `(行矩形, 控件矩形)`；`hint` 非空时走双行形态。
fn measure_with_hint(control_w: f32, hint: &'static str) -> (egui::Rect, egui::Rect) {
    let row = std::rc::Rc::new(std::cell::RefCell::new(egui::Rect::NOTHING));
    let ctrl = std::rc::Rc::new(std::cell::RefCell::new(egui::Rect::NOTHING));
    let r_out = row.clone();
    let c_out = ctrl.clone();

    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(700.0, 400.0))
        .build_ui(move |ui| {
            crate::widgets::rows::setting_row(ui, "标签", hint, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(control_w, crate::widgets::control::h(ui.ctx())),
                    egui::Sense::hover(),
                );
                *c_out.borrow_mut() = rect;
            });
            *r_out.borrow_mut() = ui.min_rect();
        });
    harness.step();
    (*row.borrow(), *ctrl.borrow())
}

/// 控件右边不能超出所在行（否则被面板裁掉）。
#[test]
fn control_stays_inside_row() {
    for w in [60.0f32, 140.0, 220.0] {
        let (row, ctrl) = measure(w);
        assert!(
            ctrl.max.x <= row.max.x + 0.5,
            "控件宽 {w}: 右缘 {:.1} 超出了行右缘 {:.1}",
            ctrl.max.x,
            row.max.x
        );
    }
}

/// 控件必须右对齐（贴着行右缘），而不是溢出。
#[test]
fn control_is_right_aligned() {
    let (row, ctrl) = measure(140.0);
    let gap = row.max.x - ctrl.max.x;
    assert!(gap.abs() <= 1.0, "控件应贴行右缘，实际差了 {gap:.1}px");
}

/// 说明文字必须**常显**（渲染在标题下方），不能只做成 hover tooltip。
///
/// 这是用户的明确要求：讲解要一眼能看见，而不是要把鼠标移上去才出现。
#[test]
fn hint_is_always_visible_not_tooltip() {
    let hint_h = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let h_out = hint_h.clone();
    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(700.0, 400.0))
        .build_ui(move |ui| {
            let before = ui.min_rect().height();
            crate::widgets::rows::setting_row(ui, "标签", "这是一段常显说明", |ui| {
                let _ = ui.allocate_exact_size(
                    egui::vec2(80.0, crate::widgets::control::h(ui.ctx())),
                    egui::Sense::hover(),
                );
            });
            *h_out.borrow_mut() = ui.min_rect().height() - before;
        });
    harness.step();
    let used = *hint_h.borrow();
    // 单行高度约 22；带说明时必须是双行高度（> 单行 2 倍控件高不够，至少要更高）。
    let one_line = crate::widgets::control::h(&egui::Context::default());
    assert!(
        used > one_line * 1.6,
        "带说明的设置行高度 {used:.1} 太矮，说明没有常显（单行约 {one_line:.1}）"
    );
}

/// 带说明（双行）时，控件同样必须完整落在行内并右对齐。
/// 外观页的开关只剩半个、通知页的步进器被推出右缘，都是这条没守住。
#[test]
fn control_stays_inside_row_with_hint() {
    for w in [60.0f32, 140.0, 220.0] {
        let (row, ctrl) = measure_with_hint(
            w,
            "这是一段比较长的说明文字，用来看双行布局是否把控件挤出右缘",
        );
        assert!(
            ctrl.max.x <= row.max.x + 0.5,
            "控件宽 {w}: 右缘 {:.1} 超出 {:.1}",
            ctrl.max.x,
            row.max.x
        );
        assert!(
            ctrl.min.x >= row.min.x,
            "控件宽 {w}: 左缘 {:.1} 跑到行外",
            ctrl.min.x
        );
    }
}

/// 双行形态下控件也要贴右缘（不能缩在左列旁边）。
#[test]
fn control_is_right_aligned_with_hint() {
    let (row, ctrl) = measure_with_hint(140.0, "说明文字");
    let gap = row.max.x - ctrl.max.x;
    assert!(gap.abs() <= 1.0, "控件应贴行右缘，实际差 {gap:.1}px");
}

/// 说明文字必须横跨整行宽度，不能被塞进 72px 的标签列。
///
/// 直接测几何：说明 label 的可用宽度应接近整行宽度。
/// （不按"行数"断言——kittest 缺字形时每个字都按方块估算，行数不可靠。）
#[test]
fn hint_text_spans_full_row_width() {
    const LONG_HINT: &str =
        "使用 GPU 计算裁剪视口外音符，适合超大工程，关闭后会退回到 CPU 裁剪路径";
    let hint_w = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let row_w = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let h_out = hint_w.clone();
    let r_out = row_w.clone();

    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(900.0, 400.0))
        .build_ui(move |ui| {
            *r_out.borrow_mut() = ui.available_width();
            crate::widgets::rows::setting_row(ui, "GPU 裁剪", LONG_HINT, |ui| {
                let _ = ui.allocate_exact_size(
                    egui::vec2(60.0, crate::widgets::control::h(ui.ctx())),
                    egui::Sense::hover(),
                );
            });
            // 说明 label 结束后的光标位置即为它的可用宽度参照。
            *h_out.borrow_mut() = ui.min_rect().width();
        });
    harness.step();

    let row = *row_w.borrow();
    let used = *hint_w.borrow();
    assert!(row > 0.0, "行宽未取到");
    assert!(
        used >= row - 2.0,
        "带说明的行只用了 {used:.1}px，未铺满整行 {row:.1}px：说明被限制在窄列里"
    );
}

/// 说明必须只占一行（不被压成多行）：带说明的行高只比不带说明的大出
/// 「一行小字 + 间距」，而不是两三四行。
#[test]
fn short_hint_takes_one_line() {
    fn row_height(hint: &'static str) -> f32 {
        let out = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
        let o = out.clone();
        let mut harness = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(900.0, 400.0))
            .build_ui(move |ui| {
                let top = ui.cursor().min.y;
                crate::widgets::rows::setting_row(ui, "GPU 裁剪", hint, |ui| {
                    let _ = ui.allocate_exact_size(
                        egui::vec2(60.0, crate::widgets::control::h(ui.ctx())),
                        egui::Sense::hover(),
                    );
                });
                *o.borrow_mut() = ui.cursor().min.y - top;
            });
        harness.step();
        *out.borrow()
    }

    let bare = row_height("");
    let with_hint = row_height("短说明");
    let extra = with_hint - bare;
    // 一行小字约 13~22px，加上 2px 间距；超过 30 就说明换行了。
    assert!(
        extra > 0.0 && extra <= 30.0,
        "说明多占了 {extra:.1}px（无说明 {bare:.1} / 有说明 {with_hint:.1}），被压成了多行"
    );
}
