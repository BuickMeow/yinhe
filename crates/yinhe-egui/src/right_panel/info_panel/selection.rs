//! 选框信息面板：显示当前选框的统计信息并支持批量表达式编辑。
//!
//! 优先于 Anchor / Track / 项目设置显示：任一视图（PR/AR/AM）存在选框时
//! 整个 Info 面板切换为选框信息。编辑字段支持表达式：
//! 赋值（`100`）、加减（`+2`/`-2`）、乘除（`x2`/`*2`/`/2`）、百分比（`20%`/`x.2`）、
//! 链式（`x3/7`），语法见 `yinhe_editor_core::num_expr`。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::batch_ops::summarize_selected;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::document::automation_edit::AnchorField;
use yinhe_editor_core::document::note_edit::{FlipAxis, NoteField};
use yinhe_editor_core::num_expr::{NumOp, apply_ops, parse_num_expr};
use yinhe_types::time_format::{format_tick_bar_beat_with_time_sig, parse_bar_beat_tick};
use yinhe_types::{AnchorSelRect, AutomationTarget, TimeSigEvent};

/// 当前拥有选框的视图（三视图互斥，同一时刻只有一个）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelView {
    /// 钢琴卷帘
    Pr,
    /// 走带（arrange）
    Ar,
    /// 自动化
    Am,
}

/// 任一视图存在选框？
pub(super) fn has_any_selection(doc: &Document) -> bool {
    !doc.edit.sel_rect.is_empty()
        || !doc.edit.arr_sel_rect.is_empty()
        || doc
            .edit
            .controller_panels
            .iter()
            .any(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty())
}

/// 显示选框信息 + 批量编辑。
pub(super) fn show(ui: &mut egui::Ui, doc: &mut Document) {
    // ── 检测选框视图与矩形 ──
    let pr_rects = doc.edit.sel_rect.effective_rects();
    let ar_rects = doc.edit.arr_sel_rect.clone();
    let am_rects: Vec<(usize, Vec<AnchorSelRect>)> = doc
        .edit
        .controller_panels
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.show_velocity && !p.anchor_sel_rects.is_empty())
        .map(|(i, p)| (i, p.anchor_sel_rects.clone()))
        .collect();
    let view = if !pr_rects.is_empty() {
        SelView::Pr
    } else if !ar_rects.is_empty() {
        SelView::Ar
    } else if !am_rects.is_empty() {
        SelView::Am
    } else {
        return;
    };

    // ── 统计 ──
    let summary = match view {
        SelView::Pr | SelView::Ar => Some(summarize_selected(&doc.data.model, &doc.edit.selected)),
        SelView::Am => None,
    };
    let am = match view {
        SelView::Am => collect_am_anchors(doc, &am_rects),
        _ => AmAnchors::default(),
    };

    // ── 时间跨度 / 视图特有跨度 ──
    let (t0, t1) = match view {
        SelView::Pr => pr_rects
            .iter()
            .map(|&(ts, te, _, _)| (ts, te))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (x, y)| {
                (a.min(x), b.max(y))
            }),
        SelView::Ar => ar_rects
            .iter()
            .map(|&(ts, te, _, _)| (ts, te))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (x, y)| {
                (a.min(x), b.max(y))
            }),
        SelView::Am => am_rects
            .iter()
            .flat_map(|(_, rs)| rs.iter())
            .map(|r| (r.tick_start.min(r.tick_end), r.tick_start.max(r.tick_end)))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (x, y)| {
                (a.min(x), b.max(y))
            }),
    };

    // ── 渲染 ──
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("sel.title").as_ref())
            .strong()
            .size(14.0)
            .color(egui::Color32::from_gray(220)),
    );
    ui.add_space(2.0);

    let pos_label = match view {
        SelView::Pr => t!("sel.pos_pr"),
        SelView::Ar => t!("sel.pos_ar"),
        SelView::Am => t!("sel.pos_am"),
    };
    info_row(ui, t!("sel.pos"), pos_label);
    let rect_count = match view {
        SelView::Pr => pr_rects.len(),
        SelView::Ar => ar_rects.len(),
        SelView::Am => am_rects.iter().map(|(_, rs)| rs.len()).sum(),
    };
    info_row(ui, t!("sel.count"), rect_count.to_string());
    info_row(
        ui,
        t!("sel.note_count"),
        summary
            .as_ref()
            .map(|s| s.count.to_string())
            .unwrap_or_else(|| "0".to_string()),
    );
    info_row(
        ui,
        t!("sel.event_count"),
        match view {
            SelView::Am => am.count.to_string(),
            _ => "0".to_string(),
        },
    );
    info_row(
        ui,
        t!("sel.tick_span"),
        format!(
            "{}，{}",
            t!("sel.from_to", a = fmt_tick(t0), b = fmt_tick(t1)),
            t!("sel.total_ticks", n = fmt_tick(t1 - t0))
        ),
    );
    match view {
        SelView::Pr => {
            let (kl, kh) = pr_rects
                .iter()
                .fold((u8::MAX, 0u8), |(a, b), &(_, _, kl, kh)| {
                    (a.min(kl), b.max(kh))
                });
            info_row(
                ui,
                t!("sel.key_span"),
                format!(
                    "{}，{}",
                    t!("sel.from_to", a = kl, b = kh),
                    t!("sel.total_keys", n = kh as i32 - kl as i32 + 1)
                ),
            );
        }
        SelView::Ar => {
            let (tl, th) = ar_rects
                .iter()
                .fold((usize::MAX, 0usize), |(a, b), &(_, _, tl, th)| {
                    (a.min(tl), b.max(th))
                });
            info_row(
                ui,
                t!("sel.track_span"),
                format!(
                    "{}，{}",
                    t!("sel.from_to", a = tl, b = th),
                    t!("sel.total_tracks", n = th - tl + 1)
                ),
            );
        }
        SelView::Am => {
            // value 跨度：所有 rect 的 value_range 并集；None（垂直全选）→ 全范围
            let mut rng: Option<(f32, f32)> = None;
            let mut full = false;
            for (_, rs) in &am_rects {
                for r in rs {
                    match r.value_range {
                        None => full = true,
                        Some((lo, hi)) => {
                            let (lo, hi) = (lo.min(hi), lo.max(hi));
                            rng = Some(match rng {
                                Some((a, b)) => (a.min(lo), b.max(hi)),
                                None => (lo, hi),
                            });
                        }
                    }
                }
            }
            let text = if full {
                t!("sel.full_range").to_string()
            } else {
                match rng {
                    Some((lo, hi)) => t!(
                        "sel.from_to",
                        a = fmt_val(lo as f64),
                        b = fmt_val(hi as f64)
                    )
                    .to_string(),
                    None => String::new(),
                }
            };
            info_row(ui, t!("sel.value_span"), text);
        }
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    // ── 编辑区 ──
    match view {
        SelView::Pr | SelView::Ar => {
            // view 判定保证 PR/AR 时 summary 必为 Some（防御性 early-return）。
            let s = match summary {
                Some(s) => s,
                None => return,
            };
            field_row(
                ui,
                "velocity",
                t!("sel.velocity"),
                s.velocity.map(|v| v as f64),
                fmt_int,
                None,
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) = doc.apply_note_field_edit(NoteField::Velocity, &ops) {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                },
            );
            field_row(
                ui,
                "gate",
                t!("sel.gate"),
                s.gate.map(|g| g as f64),
                fmt_int,
                None,
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) = doc.apply_note_field_edit(NoteField::Gate, &ops) {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                },
            );
            let interval_hint = key_interval_hint(s.key);
            field_row(
                ui,
                "key",
                t!("sel.key"),
                s.key.map(|k| k as f64),
                fmt_int,
                Some(&interval_hint),
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) = doc.apply_note_field_edit(NoteField::Key, &ops) {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                },
            );
            field_row(
                ui,
                "tick",
                t!("sel.tick"),
                s.tick.map(|t| t as f64),
                fmt_int,
                None,
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) = doc.apply_note_field_edit(NoteField::Tick, &ops) {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                },
            );
        }
        SelView::Am => {
            // 互斥：只有一个面板有选框
            let panel_idx = am_rects[0].0;
            field_row(
                ui,
                "am_value",
                t!("sel.value"),
                am.uniform_value.map(|v| v as f64),
                fmt_val,
                None,
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) =
                        doc.apply_anchor_field_edit(panel_idx, AnchorField::Value, &ops)
                    {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                    doc.edit.controller_panels[panel_idx].dirty = true;
                },
            );
            field_row(
                ui,
                "am_tick",
                t!("sel.tick"),
                am.uniform_tick.map(|t| t as f64),
                fmt_int,
                None,
                |ops| {
                    let before = doc.capture_snapshot();
                    if let Some(action) =
                        doc.apply_anchor_field_edit(panel_idx, AnchorField::Tick, &ops)
                    {
                        doc.push_undo(action, t!("undo.batch_edit").as_ref(), before);
                    }
                    doc.edit.controller_panels[panel_idx].dirty = true;
                },
            );
        }
    }

    // ── 变速（时间跨度编辑，可 undo；音符/事件与选框一起缩放） ──
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("sel.tempo_title").as_ref())
            .strong()
            .size(13.0)
            .color(egui::Color32::from_gray(200)),
    );
    ui.add_space(2.0);
    tempo_section(ui, doc, view, t0, t1);

    // ── 翻转（音符镜像；AM 锚点无 key 概念，不显示） ──
    if view != SelView::Am {
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(t!("sel.flip_horizontal")).size(12.0),
                ))
                .clicked()
            {
                let before = doc.capture_snapshot();
                if let Some(action) = doc.flip_selected_notes(FlipAxis::Horizontal) {
                    doc.push_undo(action, t!("undo.flip_horizontal").as_ref(), before);
                }
            }
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(t!("sel.flip_vertical")).size(12.0),
                ))
                .clicked()
            {
                let before = doc.capture_snapshot();
                if let Some(action) = doc.flip_selected_notes(FlipAxis::Vertical) {
                    doc.push_undo(action, t!("undo.flip_vertical").as_ref(), before);
                }
            }
        });
    }
}

// ────────────────────────────────────────────────────────────────
// 工具函数
// ────────────────────────────────────────────────────────────────

/// 只读信息行：label（灰 11px）+ 值（白 12px）。
fn info_row(ui: &mut egui::Ui, label: impl Into<String>, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label.into())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new(value.into())
                .size(12.0)
                .color(egui::Color32::from_gray(200)),
        );
    });
}

/// 表达式输入框的提示函数：输入非法时返回提示文本。
type HintFn = dyn Fn(&str) -> Option<String>;

/// 批量字段编辑行：label + 表达式输入框。
///
/// - 所有选中项字段值相同 → 输入框显示该值
/// - mixed → 输入框为空并显示「—」提示
/// - 输入表达式（`100`/`+2`/`x3/7`…）后 Enter 或失焦应用，应用后清空，
///   下一帧恢复显示当前值
fn field_row(
    ui: &mut egui::Ui,
    key: &str,
    label: impl Into<String>,
    uniform: Option<f64>,
    fmt: impl Fn(f64) -> String,
    hint: Option<&HintFn>,
    on_apply: impl FnOnce(Vec<NumOp>),
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label.into())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let id = ui.id().with(key);
        let buf_id = id.with("buf");
        let is_editing = ui.ctx().memory(|m| m.has_focus(id));
        let mut text: String = ui.ctx().data(|d| d.get_temp(buf_id).unwrap_or_default());
        if !is_editing && text.is_empty() {
            text = uniform.map(fmt).unwrap_or_default();
        }
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(90.0)
                .hint_text(if uniform.is_none() { "—" } else { "" }),
        );
        // 实时提示（如 Key 框的音程名）：聚焦且有输入时显示
        if let Some(h) = hint
            && resp.has_focus() && !text.trim().is_empty()
                && let Some(s) = h(text.trim()) {
                    ui.label(
                        egui::RichText::new(s)
                            .size(11.0)
                            .color(egui::Color32::from_gray(140)),
                    );
                }
        ui.ctx().data_mut(|d| d.insert_temp(buf_id, text.clone()));
        let submit = resp.lost_focus()
            || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if submit && !text.trim().is_empty() {
            if let Some(ops) = parse_num_expr(&text) {
                on_apply(ops);
            }
            ui.ctx()
                .data_mut(|d| d.insert_temp::<String>(buf_id, String::new()));
        }
    });
}

/// 变速编辑区：tick 数 / bar.beat.tick / 倍率，三框同步（应用后统一刷新）。
///
/// 框 1（tick 数）：支持表达式（赋值/加减乘除）。
/// 框 2（bar.beat.tick）：解析 1-indexed 格式。
/// 框 3（倍率）：数字开头 = 乘法（2 = ×2），`x2`/`*2` = ×2，`/2` = ÷2；不支持加减。
fn tempo_section(ui: &mut egui::Ui, doc: &mut Document, view: SelView, t0: f64, t1: f64) {
    let span = ((t1 - t0).max(1.0)) as u64;
    let ppq = doc.data.model.meta.ppq;
    let ts_events: Vec<TimeSigEvent> = doc.data.model.conductor.time_sig.clone();
    let default_num = ts_events.first().map(|t| t.numerator).unwrap_or(4);
    let default_den = ts_events.first().map(|t| t.denominator).unwrap_or(2);

    // 框 1：tick 数
    let s1 = tempo_field(
        ui,
        "span_ticks",
        t!("sel.span_ticks"),
        span.to_string(),
        |text| {
            let ops = parse_num_expr(text)?;
            let v = apply_ops(&ops, span as f64).round().max(1.0);
            if v > u32::MAX as f64 {
                None
            } else {
                Some(v as u64)
            }
        },
    );
    ui.add_space(2.0);

    // 框 2：bar.beat.tick（与时间标尺同一格式）
    let bar_beat =
        format_tick_bar_beat_with_time_sig(span as f64, ppq, &ts_events, default_num, default_den);
    let s2 = tempo_field(
        ui,
        "span_bar_beat",
        t!("sel.span_bar_beat"),
        bar_beat,
        |text| {
            parse_bar_beat_tick(text, ppq, &ts_events, default_num, default_den).map(|v| v.max(1))
        },
    );
    ui.add_space(2.0);

    // 框 3：倍率（数字开头 = 乘法）
    let s3 = tempo_field(ui, "span_ratio", t!("sel.ratio"), "1".to_string(), |text| {
        let ops = parse_num_expr(text)?;
        let mut v = span as f64;
        for op in ops {
            match op {
                NumOp::Set(n) => v *= n,
                NumOp::Mul(n) => v *= n,
                NumOp::Div(n) => v /= n,
                NumOp::Add(_) => return None, // 倍率不支持加减
            }
        }
        let v = v.round().max(1.0);
        if v > u32::MAX as f64 {
            None
        } else {
            Some(v as u64)
        }
    });

    // 应用变速（任一框提交）
    if let Some(new_span) = s1.or(s2).or(s3) {
        match view {
            SelView::Pr | SelView::Ar => {
                let before = doc.capture_snapshot();
                if let Some(action) = doc.rescale_selection_span(new_span) {
                    doc.push_undo(action, t!("undo.rescale_span").as_ref(), before);
                }
            }
            SelView::Am => {
                // 互斥：只有一个面板有选框（与编辑区同一来源）
                let panel_idx = doc
                    .edit
                    .controller_panels
                    .iter()
                    .position(|p| !p.show_velocity && !p.anchor_sel_rects.is_empty());
                if let Some(panel_idx) = panel_idx {
                    let before = doc.capture_snapshot();
                    if let Some(action) = doc.rescale_anchor_span(panel_idx, new_span) {
                        doc.push_undo(action, t!("undo.rescale_span").as_ref(), before);
                        doc.edit.controller_panels[panel_idx].dirty = true;
                    }
                }
            }
        }
        // 清空三个输入框，下帧恢复显示新跨度
        let base = ui.id();
        for k in ["span_ticks", "span_bar_beat", "span_ratio"] {
            ui.ctx()
                .data_mut(|d| d.insert_temp::<String>(base.with(k).with("buf"), String::new()));
        }
    }
}

/// 变速输入框：显示当前值；输入后 Enter/失焦应用，返回解析出的新跨度。
fn tempo_field(
    ui: &mut egui::Ui,
    key: &str,
    label: impl Into<String>,
    display: String,
    parse: impl Fn(&str) -> Option<u64>,
) -> Option<u64> {
    let id = ui.id().with(key);
    let buf_id = id.with("buf");
    let mut result = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label.into())
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let is_editing = ui.ctx().memory(|m| m.has_focus(id));
        let mut text: String = ui.ctx().data(|d| d.get_temp(buf_id).unwrap_or_default());
        if !is_editing && text.is_empty() {
            text = display;
        }
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(90.0),
        );
        ui.ctx().data_mut(|d| d.insert_temp(buf_id, text.clone()));
        let submit = resp.lost_focus()
            || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if submit && !text.trim().is_empty() {
            if let Some(v) = parse(text.trim()) {
                result = Some(v);
            }
            ui.ctx()
                .data_mut(|d| d.insert_temp::<String>(buf_id, String::new()));
        }
    });
    result
}

/// AM 选中锚点统计（自动化事件量小，直接收集）。
#[derive(Default)]
struct AmAnchors {
    count: usize,
    /// 全部锚点 value 相同时为 Some。
    uniform_value: Option<f32>,
    /// 全部锚点 tick 相同时为 Some。
    uniform_tick: Option<u32>,
}

fn collect_am_anchors(doc: &Document, panels: &[(usize, Vec<AnchorSelRect>)]) -> AmAnchors {
    let mut out = AmAnchors::default();
    let mut first = true;
    for (panel_idx, rects) in panels {
        let panel = &doc.edit.controller_panels[*panel_idx];
        let target = &panel.selected_target;
        let events: Vec<(u32, f32)> = if matches!(target, AutomationTarget::Tempo) {
            doc.data
                .model
                .conductor
                .tempo
                .events
                .iter()
                .map(|e| (e.tick, e.value))
                .collect()
        } else {
            let editing = doc.edit.editing_track.unwrap_or(u16::MAX);
            let Some(track) = doc.data.model.tracks.get(editing as usize) else {
                continue;
            };
            let Some(lane) = track.automation_lanes.iter().find(|l| l.target == *target) else {
                continue;
            };
            lane.events.iter().map(|e| (e.tick, e.value)).collect()
        };
        for (tick, value) in events {
            if !rects.iter().any(|r| r.contains(tick, value)) {
                continue;
            }
            out.count += 1;
            if first {
                out.uniform_value = Some(value);
                out.uniform_tick = Some(tick);
                first = false;
            } else {
                if out.uniform_value.is_some() && out.uniform_value != Some(value) {
                    out.uniform_value = None;
                }
                if out.uniform_tick.is_some() && out.uniform_tick != Some(tick) {
                    out.uniform_tick = None;
                }
            }
        }
    }
    out
}

fn fmt_int(v: f64) -> String {
    format!("{v:.0}")
}

fn fmt_val(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{v:.0}")
    } else {
        format!("{v:.3}")
    }
}

fn fmt_tick(v: f64) -> String {
    format!("{v:.0}")
}

/// Key 编辑输入的音程提示：解析输入文本，计算最终变化量并显示音程名。
///
/// - 变化量绝对值 1..=12 显示音程名，>12 或 0 不显示
/// - 负值为下行（如 -2 → 大二度（下行））
/// - 有 uniform key 时按当前值计算（支持赋值/链式）；无基准时仅纯加减可判定
/// - 乘除运算无音程意义，不显示
fn key_interval_hint(uniform_key: Option<u8>) -> impl Fn(&str) -> Option<String> {
    move |text| {
        let ops = parse_num_expr(text)?;
        let delta = if let Some(cur) = uniform_key {
            apply_ops(&ops, cur as f64) - cur as f64
        } else {
            // 无基准：仅纯加减可判定
            let mut d = 0.0;
            for op in &ops {
                match op {
                    NumOp::Add(n) => d += n,
                    _ => return None,
                }
            }
            d
        };
        if delta.fract() != 0.0 {
            return None;
        }
        let d = delta.round() as i32;
        if d == 0 || d.abs() > 12 {
            return None;
        }
        let name = match d.abs() {
            1 => t!("sel.interval.1"),
            2 => t!("sel.interval.2"),
            3 => t!("sel.interval.3"),
            4 => t!("sel.interval.4"),
            5 => t!("sel.interval.5"),
            6 => t!("sel.interval.6"),
            7 => t!("sel.interval.7"),
            8 => t!("sel.interval.8"),
            9 => t!("sel.interval.9"),
            10 => t!("sel.interval.10"),
            11 => t!("sel.interval.11"),
            12 => t!("sel.interval.12"),
            _ => return None,
        };
        if d < 0 {
            Some(t!("sel.interval_down", name = name).to_string())
        } else {
            Some(name.to_string())
        }
    }
}
