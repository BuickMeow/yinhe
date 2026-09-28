//! 传输栏时间码（三列，居中）：BPM/拍号+PPQ | 位置/时间 | 量化/调式。
//!
//! 每个字段点击即可 inline 编辑（无框输入，回车/失焦提交）：
//! - BPM：单/相同 tempo 事件 → 直接改；多个不同 → 在光标位置插入新事件；
//! - 拍号：接受 `4/4`、`4.4`、`4。4`，修改逻辑同 BPM；
//! - PPQ：直接输入（走既有 rescale 确认流程）；
//! - 位置（`1.4.000`）与秒数（`0:01.500`）：输入后移动光标，接受 `。` 代替 `.`；
//! - 量化：点击弹出量化 popup（作用于聚焦视图）；
//! - 调式：点击弹出两列下拉（左根音 × 右音阶），只覆盖显示，
//!   可分别关闭「覆盖」与「工程事件」（覆盖 > 工程事件）。
//!
//! 本模块不持有/修改文档状态，只产生 [TimecodeEvent]，由 main_loop 应用。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::time_format;
use yinhe_types::{NOTE_NAMES, ScaleType};

/// 时间码控件宽度（三列：BPM/拍号+PPQ、位置/时间、量化/调式）。
pub const TIMECODE_WIDTH: f32 = 252.0;

/// 三列宽度（与 `TIMECODE_WIDTH` 一致）。
const COL_WIDTHS: [f32; 3] = [78.0, 92.0, 82.0];
/// 控件高度（上下两行各 18）。
const RECT_H: f32 = 36.0;
/// 单行高度。
const ROW_H: f32 = RECT_H * 0.5;

/// 时间码编辑事件（由 main_loop 应用）。
pub enum TimecodeEvent {
    /// BPM 直接输入。
    Bpm(f64),
    /// 拍号输入（`denominator_power` = 2 的幂编码）。
    TimeSig {
        numerator: u8,
        denominator_power: u8,
    },
    /// PPQ 输入（`id` 供 rescale 确认流程的 pending/undo 使用）。
    Ppq { value: u32, id: u64 },
    /// 光标位置跳转（位置/秒数输入）。
    CursorTick(f64),
    /// 调式覆盖显示设置（`override` = None 表示关闭覆盖）。
    KeySig {
        r#override: Option<(u8, ScaleType)>,
        use_events: bool,
    },
    /// 量化预设（写入聚焦视图）。
    Quantize(QuantizePreset),
}

/// 时间码输入数据。
pub struct TimecodeData<'a> {
    pub doc: &'a Document,
    /// 聚焦视图的量化（AR 或 PR）。
    pub quantize: QuantizePreset,
    /// 量化属于 PR（true）还是 AR（false）——显示 `PR 1/16` / `AR 1/4` 前缀。
    pub quantize_is_pr: bool,
}

/// 编辑中的文本缓冲（egui memory 暂存；非编辑态每帧刷新为当前值）。
#[derive(Clone, Default)]
struct Buffers {
    bpm: String,
    time_sig: String,
    ppq: String,
    pos: String,
    time: String,
}

/// 当前显示调式：覆盖优先于工程事件；两者都关 = None。
fn current_key_sig(doc: &Document) -> Option<(u8, ScaleType)> {
    if let Some(ov) = doc.edit.key_sig_override {
        return Some(ov);
    }
    if !doc.edit.key_sig_use_events {
        return None;
    }
    let tick = doc.edit.cursor_tick.unwrap_or(0.0) as u32;
    let events = &doc.data.model.conductor.key_sig;
    let idx = events.partition_point(|e| e.tick <= tick);
    idx.checked_sub(1)
        .map(|i| (events[i].root, events[i].scale))
        // 工程无调号事件 / 光标在首个事件前：MIDI 默认 C 大调
        .or(Some((0, ScaleType::Major)))
}

/// 调式显示文本（如 `C Major`）；无调式 = `--`。
fn key_sig_text(key: Option<(u8, ScaleType)>) -> String {
    match key {
        Some((root, scale)) => {
            format!(
                "{} {}",
                NOTE_NAMES[(root % 12) as usize],
                scale.english_name()
            )
        }
        None => "--".to_string(),
    }
}

/// 聚焦时全选文本：点击字段后直接输入即可覆盖原值（否则是追加，解析必失败）。
fn select_all_on_focus(ui: &egui::Ui, resp: &egui::Response, id: egui::Id, text: &str) {
    if !resp.gained_focus() {
        return;
    }
    let len = text.chars().count();
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(len),
        )));
    state.store(ui.ctx(), id);
}

/// 无框单行输入（时间码字段共用样式，文字在格内垂直居中）。
fn text_edit<'a>(id: egui::Id, buf: &'a mut String, width: f32, height: f32) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(buf)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(egui::FontId::proportional(crate::theme::TIMECODE_FONT))
        .text_color(crate::theme::accent_active())
        .horizontal_align(egui::Align::Center)
        .vertical_align(egui::Align::Center)
        .margin(egui::Margin::symmetric(2, 0))
        .min_size(egui::vec2(width, height))
        .desired_width(width)
}

/// 传输栏时间码显示（三列居中）。返回控件矩形与编辑事件。
pub fn show_timecode_display(
    ui: &mut egui::Ui,
    data: TimecodeData<'_>,
) -> (egui::Rect, Vec<TimecodeEvent>) {
    let doc = data.doc;
    let tick = doc.edit.cursor_tick.unwrap_or(0.0);
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let seconds = model.tempo_map.tick_to_seconds(tick as u64);
    let bpm = model.tempo_map.bpm_at_time(seconds);
    let (num, den_power) = model.tempo_map.time_sig_at_tick(tick as u32);
    let (def_num, def_den) = model.tempo_map.time_sig_default;
    let pos_str = time_format::format_tick_bar_beat_with_time_sig(
        tick,
        ppq,
        &model.tempo_map.time_sig_events,
        def_num,
        def_den,
    );
    let time_str = time_format::format_time(seconds);
    let bpm_str = time_format::format_bpm(bpm);
    let ts_str = time_format::format_time_sig(num, den_power);
    let ppq_str = ppq.to_string();
    let key = current_key_sig(doc);

    // 居中占位（与旧两列实现一致：时间码把本栏分成左右两半）
    let bar_cx = ui.max_rect().center().x;
    let cursor_x = ui.cursor().min.x;
    let rect_l = bar_cx - TIMECODE_WIDTH * 0.5;
    let pad = (rect_l - cursor_x).max(0.0);
    ui.add_space(pad);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(TIMECODE_WIDTH, RECT_H), egui::Sense::hover());

    // 背景 + 列分隔线
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), crate::theme::track_bg());
    let grid = egui::Stroke::new(1.0, crate::theme::line_fg());
    let mut col_x = rect.min.x;
    for (i, width) in COL_WIDTHS.iter().enumerate() {
        if i > 0 {
            ui.painter().line_segment(
                [egui::pos2(col_x, rect.min.y), egui::pos2(col_x, rect.max.y)],
                grid,
            );
        }
        col_x += *width;
    }

    // 单元格矩形：列 × 行（0 = 上，1 = 下）
    let cell = |col: usize, row: usize| -> egui::Rect {
        let x0 = rect.min.x + COL_WIDTHS[..col].iter().sum::<f32>();
        let y0 = rect.min.y + row as f32 * (RECT_H * 0.5);
        egui::Rect::from_min_size(
            egui::pos2(x0, y0),
            egui::vec2(COL_WIDTHS[col], RECT_H * 0.5),
        )
    };

    let buf_id = ui.id().with("timecode_buffers");
    let mut bufs: Buffers = ui.data_mut(|d| d.get_temp(buf_id)).unwrap_or_default();
    let mut events: Vec<TimecodeEvent> = Vec::new();

    // 非编辑态刷新缓冲为当前值（编辑中保留用户输入）
    let focused = |ui: &egui::Ui, salt: &str| ui.memory(|m| m.has_focus(ui.id().with(salt)));
    if !focused(ui, "tc_bpm") {
        bufs.bpm = bpm_str;
    }
    if !focused(ui, "tc_time_sig") {
        bufs.time_sig = ts_str;
    }
    if !focused(ui, "tc_ppq") {
        bufs.ppq = ppq_str;
    }
    if !focused(ui, "tc_pos") {
        bufs.pos = pos_str;
    }
    if !focused(ui, "tc_time") {
        bufs.time = time_str;
    }

    // ── BPM（列 0 上）──
    let bpm_id = ui.id().with("tc_bpm");
    let resp = ui.put(
        cell(0, 0),
        text_edit(bpm_id, &mut bufs.bpm, COL_WIDTHS[0] - 8.0, ROW_H),
    );
    select_all_on_focus(ui, &resp, bpm_id, &bufs.bpm);
    if resp.lost_focus()
        && let Ok(v) = bufs.bpm.trim().parse::<f64>()
        && v.is_finite()
        && v > 0.0
    {
        events.push(TimecodeEvent::Bpm(v));
    }

    // ── 拍号（列 0 下左）+ PPQ（列 0 下右）──
    let sig_cell = cell(0, 1);
    let sig_w = sig_cell.width() * 0.5;
    let sig_rect = egui::Rect::from_min_size(sig_cell.min, egui::vec2(sig_w, sig_cell.height()));
    let ppq_rect = egui::Rect::from_min_size(
        egui::pos2(sig_cell.min.x + sig_w, sig_cell.min.y),
        egui::vec2(sig_cell.width() - sig_w, sig_cell.height()),
    );
    let sig_id = ui.id().with("tc_time_sig");
    let resp = ui.put(
        sig_rect,
        text_edit(sig_id, &mut bufs.time_sig, sig_w - 4.0, ROW_H),
    );
    select_all_on_focus(ui, &resp, sig_id, &bufs.time_sig);
    if resp.lost_focus()
        && let Some((numerator, denominator_power)) = time_format::parse_time_sig(&bufs.time_sig)
    {
        events.push(TimecodeEvent::TimeSig {
            numerator,
            denominator_power,
        });
    }
    let ppq_id = ui.id().with("tc_ppq");
    let resp = ui.put(
        ppq_rect,
        text_edit(ppq_id, &mut bufs.ppq, ppq_rect.width() - 4.0, ROW_H),
    );
    select_all_on_focus(ui, &resp, ppq_id, &bufs.ppq);
    if resp.lost_focus()
        && let Ok(v) = bufs.ppq.trim().parse::<u32>()
        && (1..=32767).contains(&v)
    {
        events.push(TimecodeEvent::Ppq {
            value: v,
            id: ppq_id.value(),
        });
    }

    // ── 位置（列 1 上）──
    let pos_id = ui.id().with("tc_pos");
    let resp = ui.put(
        cell(1, 0),
        text_edit(pos_id, &mut bufs.pos, COL_WIDTHS[1] - 8.0, ROW_H),
    );
    select_all_on_focus(ui, &resp, pos_id, &bufs.pos);
    if resp.lost_focus() {
        let normalized = bufs.pos.trim().replace('。', ".");
        // `bar.beat.tick`（如 `5.1.000`）；纯数字按绝对 tick 解析（如 `7680`）
        let tick = if normalized.contains('.') {
            time_format::parse_bar_beat_tick(
                &normalized,
                ppq,
                &model.tempo_map.time_sig_events,
                def_num,
                def_den,
            )
            .map(|t| t as f64)
        } else {
            normalized.parse::<u64>().ok().map(|t| t as f64)
        };
        if let Some(t) = tick {
            events.push(TimecodeEvent::CursorTick(t));
        }
    }

    // ── 秒数（列 1 下）──
    let time_id = ui.id().with("tc_time");
    let resp = ui.put(
        cell(1, 1),
        text_edit(time_id, &mut bufs.time, COL_WIDTHS[1] - 8.0, ROW_H),
    );
    select_all_on_focus(ui, &resp, time_id, &bufs.time);
    if resp.lost_focus()
        && let Some(secs) = time_format::parse_time(&bufs.time)
    {
        let t = seconds_to_tick(&model.tempo_map, secs);
        events.push(TimecodeEvent::CursorTick(t));
    }

    // ── 量化（列 2 上；`AR 1/4` / `PR 1/16`，与拍号区分）──
    {
        let quantize = data.quantize;
        let label = format!(
            "{} {}",
            if data.quantize_is_pr { "PR" } else { "AR" },
            quantize.label()
        );
        let cell_rect = cell(2, 0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell_rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        &label,
                        egui::FontId::proportional(crate::theme::TIMECODE_FONT),
                        crate::theme::accent_active(),
                        false,
                    );
                    let mut pending_q = None;
                    egui::Popup::from_toggle_button_response(&resp)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            crate::widgets::quantize_popup::show(ui, ppq, quantize, &mut pending_q);
                        });
                    if let Some(q) = pending_q {
                        events.push(TimecodeEvent::Quantize(q));
                    }
                    if resp.hovered() {
                        resp.on_hover_text(label);
                    }
                },
            );
        });
    }

    // ── 调式（列 2 下）──
    {
        let text = key_sig_text(key);
        let cell_rect = cell(2, 1);
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell_rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        &text,
                        egui::FontId::proportional(crate::theme::TIMECODE_FONT),
                        crate::theme::accent_active(),
                        false,
                    );
                    egui::Popup::from_toggle_button_response(&resp)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            key_sig_popup(
                                ui,
                                key,
                                doc.edit.key_sig_override.is_some(),
                                doc.edit.key_sig_use_events,
                                &mut events,
                            );
                        });
                },
            );
        });
    }

    ui.data_mut(|d| d.insert_temp(buf_id, bufs));
    (rect, events)
}

/// 秒 → tick（tempo_map 单调，二分反查）。
///
/// 上限用 `u32::MAX`（不能用 `tick_length`：空工程/光标超出工程末尾时
/// tick_length 会截断跳转结果）。
fn seconds_to_tick(tempo_map: &yinhe_core::TempoMap, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    let max_tick = u32::MAX as u64;
    if tempo_map.tick_to_seconds(max_tick) <= seconds {
        return max_tick as f64;
    }
    let (mut lo, mut hi) = (0.0f64, max_tick as f64);
    for _ in 0..48 {
        let mid = (lo + hi) * 0.5;
        if tempo_map.tick_to_seconds(mid as u64) < seconds {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi.round()
}

/// 调式两列下拉：左根音 × 右音阶；下方两个开关（覆盖 / 工程事件，覆盖 > 工程事件）。
fn key_sig_popup(
    ui: &mut egui::Ui,
    current: Option<(u8, ScaleType)>,
    override_on: bool,
    use_events: bool,
    events: &mut Vec<TimecodeEvent>,
) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.spacing_mut().interact_size.y = 24.0;
    let (cur_root, cur_scale) = current.unwrap_or((0, ScaleType::Major));

    ui.horizontal(|ui| {
        // ── 左列：根音 ──
        ui.vertical(|ui| {
            ui.set_min_width(84.0);
            ui.set_max_width(84.0);
            ui.label(t!("timecode.root"));
            ui.separator();
            for root in 0..12u8 {
                let selected = current.is_some_and(|(r, _)| r == root);
                if ui
                    .add(crate::widgets::menu::menu_item_button(
                        ui,
                        selected,
                        NOTE_NAMES[(root % 12) as usize],
                    ))
                    .clicked()
                {
                    events.push(TimecodeEvent::KeySig {
                        r#override: Some((root, cur_scale)),
                        use_events,
                    });
                    ui.close();
                }
            }
        });
        ui.separator();
        // ── 右列：音阶 ──
        ui.vertical(|ui| {
            ui.set_min_width(104.0);
            ui.set_max_width(104.0);
            ui.label(t!("timecode.scale"));
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("timecode_scale_list")
                .max_height(300.0)
                .show(ui, |ui| {
                    for scale in ScaleType::ALL {
                        let selected = current.is_some_and(|(_, s)| s == *scale);
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                selected,
                                scale.english_name(),
                            ))
                            .clicked()
                        {
                            events.push(TimecodeEvent::KeySig {
                                r#override: Some((cur_root, *scale)),
                                use_events,
                            });
                            ui.close();
                        }
                    }
                });
        });
    });
    ui.separator();
    // ── 开关：覆盖显示 / 工程事件 ──
    let mut ov = override_on;
    if crate::widgets::checkbox::check_scope(ui, |ui| ui.checkbox(&mut ov, t!("timecode.override")))
        .inner
        .clicked()
    {
        let value = ov.then_some(current.unwrap_or((0, ScaleType::Major)));
        events.push(TimecodeEvent::KeySig {
            r#override: value,
            use_events,
        });
    }
    let mut ue = use_events;
    if crate::widgets::checkbox::check_scope(ui, |ui| {
        ui.checkbox(&mut ue, t!("timecode.use_events"))
    })
    .inner
    .clicked()
    {
        events.push(TimecodeEvent::KeySig {
            r#override: override_on.then_some(current.unwrap_or((0, ScaleType::Major))),
            use_events: ue,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：空工程（tick_length = 0）时秒数换算不能返回 0——
    /// 旧实现以 tick_length 为二分上限，空工程/光标超出末尾会被截断成 0。
    #[test]
    fn seconds_to_tick_works_on_empty_project() {
        let mut model = yinhe_core::YinModel::default();
        model.rebuild_tempo_map();
        assert_eq!(model.tick_length, 0);
        // 默认 480 PPQ / 120 BPM：1 秒 = 960 tick
        assert_eq!(seconds_to_tick(&model.tempo_map, 1.0), 960.0);
        assert_eq!(seconds_to_tick(&model.tempo_map, 0.5), 480.0);
        assert_eq!(seconds_to_tick(&model.tempo_map, 0.0), 0.0);
    }
}
