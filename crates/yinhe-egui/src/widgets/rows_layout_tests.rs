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

/// 关键回归：设置页真实结构（两列 + 卡片内边距 + ScrollArea + 宽度钉定）下，
/// 右侧控件仍必须落在卡片内容区内。
///
/// 之前所有隔离测试都通过，但真实页面里控件被推出卡片——差别就在
/// ScrollArea 这一层。这个测试把那一层也复现出来。
#[test]
fn control_fits_inside_scroll_area_card() {
    let card_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let ctrl_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let cr = card_right.clone();
    let tr = ctrl_right.clone();

    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            let nav_w = crate::scaling::scaled_font(ui.ctx(), 148.0);
            // 与 settings.rs 完全同构：two_columns → card_frame → ScrollArea。
            crate::widgets::grid::two_columns(ui, 8.0, nav_w + 16.0, |cells| {
                cells.cell(|_| {});
                cells.cell(|ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("probe_scroll")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.set_max_width(ui.available_width());
                                    *cr.borrow_mut() = ui.max_rect().max.x;
                                    // 塞满高度，强制出现垂直滚动条（真实页面就是这样）。
                                    for _ in 0..40 {
                                        ui.add_space(20.0);
                                    }
                                    crate::widgets::rows::setting_row(
                                        ui,
                                        "允许重叠音符",
                                        "允许新建或移动的音符与其他音符重叠",
                                        |ui| {
                                            let (r, _) = ui.allocate_exact_size(
                                                egui::vec2(
                                                    44.0,
                                                    crate::widgets::control::h(ui.ctx()),
                                                ),
                                                egui::Sense::hover(),
                                            );
                                            *tr.borrow_mut() = r.max.x;
                                        },
                                    );
                                });
                        });
                });
            });
        });
    harness.step();
    harness.step();

    let card = *card_right.borrow();
    let ctrl = *ctrl_right.borrow();
    assert!(card > 0.0 && ctrl > 0.0, "未取到几何");
    // 容差取一个滚动条宽度：`available_width` 不含滚动条占位，
    // 而 `max_rect` 含，两者天然差这么多。
    let tol = crate::theme::SCROLLBAR_W;
    assert!(
        ctrl <= card + tol,
        "控件右缘 {ctrl:.1} 超出卡片内容右缘 {card:.1}（差 {:.1}px > 容差 {tol}）",
        ctrl - card
    );
}

/// 同理，无说明的单行形态在 ScrollArea 里也要贴齐。
#[test]
fn plain_row_fits_inside_scroll_area_card() {
    let card_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let ctrl_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
    let cr = card_right.clone();
    let tr = ctrl_right.clone();

    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            let nav_w = crate::scaling::scaled_font(ui.ctx(), 148.0);
            crate::widgets::grid::two_columns(ui, 8.0, nav_w + 16.0, |cells| {
                cells.cell(|_| {});
                cells.cell(|ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("probe_scroll2")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.set_max_width(ui.available_width());
                                    *cr.borrow_mut() = ui.max_rect().max.x;
                                    crate::widgets::rows::setting_row_plain(
                                        ui,
                                        "允许重叠音符",
                                        |ui| {
                                            let (r, _) = ui.allocate_exact_size(
                                                egui::vec2(
                                                    44.0,
                                                    crate::widgets::control::h(ui.ctx()),
                                                ),
                                                egui::Sense::hover(),
                                            );
                                            *tr.borrow_mut() = r.max.x;
                                        },
                                    );
                                });
                        });
                });
            });
        });
    harness.step();
    harness.step();

    let card = *card_right.borrow();
    let ctrl = *ctrl_right.borrow();
    assert!(card > 0.0 && ctrl > 0.0, "未取到几何");
    let tol = crate::theme::SCROLLBAR_W;
    assert!(
        ctrl <= card + tol,
        "控件右缘 {ctrl:.1} 超出卡片内容右缘 {card:.1}（容差 {tol}）"
    );
}

/// 同一页里连续多行时，**后面的行**也不能变窄。
///
/// 之前的反复错位就出在这里：行宽按 `available_width()` 算，
/// 而它随光标推进收缩，越往后的行越窄、控件越容易越界。
#[test]
fn later_rows_do_not_shrink() {
    let widths = std::rc::Rc::new(std::cell::RefCell::new(Vec::<f32>::new()));
    let w_out = widths.clone();

    let mut harness = egui_kittest::Harness::builder()
        .allow_missing_glyphs()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui(move |ui| {
            let nav_w = crate::scaling::scaled_font(ui.ctx(), 148.0);
            crate::widgets::grid::two_columns(ui, 8.0, nav_w + 16.0, |cells| {
                cells.cell(|_| {});
                cells.cell(|ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            let card_w = ui.available_width();
                            ui.set_width(card_w);
                            egui::ScrollArea::vertical()
                                .id_salt("shrink_probe")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    let inner = ui.available_width();
                                    ui.set_min_width(inner);
                                    ui.set_max_width(inner);
                                    for i in 0..6 {
                                        crate::widgets::rows::setting_row(
                                            ui,
                                            &format!("行 {i}"),
                                            "",
                                            |ui| {
                                                let (r, _) = ui.allocate_exact_size(
                                                    egui::vec2(
                                                        44.0,
                                                        crate::widgets::control::h(ui.ctx()),
                                                    ),
                                                    egui::Sense::hover(),
                                                );
                                                w_out.borrow_mut().push(r.max.x);
                                            },
                                        );
                                    }
                                });
                        });
                });
            });
        });
    harness.step();
    harness.step();

    let all = widths.borrow().clone();
    assert!(all.len() >= 6, "应至少渲染 6 行，实际 {}", all.len());
    // kittest 可能跑多帧：只取最后一帧的 6 行。
    let rights: Vec<f32> = all[all.len() - 6..].to_vec();
    let first = rights[0];
    for (i, r) in rights.iter().enumerate() {
        assert!(
            (r - first).abs() <= 1.0,
            "第 {i} 行控件右缘 {r:.1} 与首行 {first:.1} 不一致：行宽在收缩"
        );
    }
}

/// 宽窗口下卡片与控件都不能超出窗口（用户截图是拉宽设置窗口后出现的）。
#[test]
fn wide_window_keeps_card_inside() {
    for w in [760.0f32, 1200.0, 1750.0, 2400.0] {
        let card_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
        let ctrl_right = std::rc::Rc::new(std::cell::RefCell::new(0.0f32));
        let cr = card_right.clone();
        let tr = ctrl_right.clone();
        let mut harness = egui_kittest::Harness::builder()
            .allow_missing_glyphs()
            .with_size(egui::vec2(w, 800.0))
            .build_ui(move |ui| {
                let nav_w = crate::scaling::scaled_font(ui.ctx(), 148.0);
                crate::widgets::grid::two_columns(ui, 8.0, nav_w + 16.0, |cells| {
                    cells.cell(|_| {});
                    cells.cell(|ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                let card_w = ui.available_width();
                                ui.set_width(card_w);
                                egui::ScrollArea::vertical()
                                    .id_salt(("wide_probe", w as u32))
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        let inner = ui.available_width();
                                        ui.set_min_width(inner);
                                        ui.set_max_width(inner);
                                        *cr.borrow_mut() = ui.max_rect().max.x;
                                        crate::widgets::rows::setting_row(
                                            ui,
                                            "允许重叠音符",
                                            "允许新建或移动的音符与其他音符重叠",
                                            |ui| {
                                                let (r, _) = ui.allocate_exact_size(
                                                    egui::vec2(
                                                        44.0,
                                                        crate::widgets::control::h(ui.ctx()),
                                                    ),
                                                    egui::Sense::hover(),
                                                );
                                                *tr.borrow_mut() = r.max.x;
                                            },
                                        );
                                    });
                            });
                    });
                });
            });
        harness.step();
        harness.step();
        let card = *card_right.borrow();
        let ctrl = *ctrl_right.borrow();
        assert!(ctrl <= w - 4.0, "窗口宽 {w}: 控件右缘 {ctrl:.1} 超出窗口");
        let tol = crate::theme::SCROLLBAR_W;
        assert!(
            ctrl <= card + tol,
            "窗口宽 {w}: 控件右缘 {ctrl:.1} 超出卡片 {card:.1}（容差 {tol}）"
        );
    }
}
