//! 传输栏时间码（四列，居中）：BPM/拍号+PPQ | 位置/时间 | 量化/调式 | 总输出电平表。
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

/// 时间码控件宽度（四列：BPM/拍号+PPQ、位置/时间、量化/调式、电平表）。
pub const TIMECODE_WIDTH: f32 = 276.0;

/// 时间码灰块与相邻控件的保底间距（左右两侧；见 `show_timecode_display` 占位
/// 与 `chrome::transport_bar` 灰块后的 add_space）。
pub const MIN_SIDE_GAP: f32 = 24.0;

/// 四列宽度（内容宽，灰底外框与内容同宽）。
const COL_WIDTHS: [f32; 4] = [78.0, 92.0, 82.0, 24.0];
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
    /// 打开「调式（音阶）」对话框。
    OpenScaleDialog,
    /// 量化预设（写入聚焦视图）。
    Quantize(QuantizePreset),
}

/// 时间码输入数据。
pub struct TimecodeData<'a> {
    pub doc: &'a Document,
    /// 总输出电平显示值 `((level L,R), (hold L,R))`（第四栏竖表）。
    pub master_meter: ((f32, f32), (f32, f32)),
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
pub(crate) fn current_key_sig(doc: &Document) -> Option<(u8, ScaleType)> {
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
        .align(egui::Align2::CENTER_CENTER)
        .margin(egui::Margin::symmetric(2, 0))
        .min_size(egui::vec2(width, height))
        .desired_width(width)
}

/// 与 `hover::hover_button` 同款四态着色，但文字超过 `max_w` 时用**负字距**
/// 横向压缩（字高不变，只让整体变窄）以塞进窄格——用于较长的调式名
/// （如 `Double Harmonic Major`、`Combination Diminished`）。
fn fit_hover_text(ui: &mut egui::Ui, text: &str, max_w: f32) -> egui::Response {
    let font = egui::FontId::proportional(crate::theme::TIMECODE_FONT);
    let natural =
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
    let w = natural.size().x;
    let spacing = if w > max_w {
        -((w - max_w) / text.chars().count().max(1) as f32)
    } else {
        0.0
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::text::TextFormat {
            font_id: font,
            color: egui::Color32::PLACEHOLDER,
            extra_letter_spacing: spacing,
            ..Default::default()
        },
    );
    let galley = ui.painter().layout_job(job);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    let color =
        crate::widgets::hover::hover_button_color(&resp, crate::theme::accent_active(), false);
    ui.painter()
        .add(egui::epaint::TextShape::new(rect.min, galley, color));
    resp
}

/// 传输栏时间码显示（四列居中）。返回控件矩形与编辑事件。
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

    // 居中占位（与旧两列实现一致：时间码把本栏分成左右两半）。
    // 保底 `MIN_SIDE_GAP`：窗口窄/左侧控件多时，灰块与左邻之间也留出间距。
    let bar_cx = ui.max_rect().center().x;
    let cursor_x = ui.cursor().min.x;
    let rect_l = bar_cx - TIMECODE_WIDTH * 0.5;
    let pad = (rect_l - cursor_x).max(MIN_SIDE_GAP);
    ui.add_space(pad);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(TIMECODE_WIDTH, RECT_H), egui::Sense::hover());

    // 背景（外框与内容同宽：内容紧贴外框边缘）+ 列分隔线。
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

    // 单元格矩形：列 × 行（0 = 上，1 = 下）。
    let cell = |col: usize, row: usize| -> egui::Rect {
        let x0 = rect.min.x + COL_WIDTHS[..col].iter().sum::<f32>();
        let y0 = rect.min.y + row as f32 * (RECT_H * 0.5);
        egui::Rect::from_min_size(
            egui::pos2(x0, y0),
            egui::vec2(COL_WIDTHS[col], RECT_H * 0.5),
        )
    };

    let buf_id = ui.scope_id().with("timecode_buffers");
    let mut bufs: Buffers = ui.data_mut(|d| d.get_temp(buf_id)).unwrap_or_default();
    let mut events: Vec<TimecodeEvent> = Vec::new();

    // 非编辑态刷新缓冲为当前值（编辑中保留用户输入）
    let focused = |ui: &egui::Ui, salt: &str| ui.memory(|m| m.has_focus(ui.scope_id().with(salt)));
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
    let bpm_id = ui.scope_id().with("tc_bpm");
    let resp = ui.put(
        cell(0, 0),
        text_edit(bpm_id, &mut bufs.bpm, COL_WIDTHS[0] - 8.0, ROW_H),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.bpm"));
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
    let sig_id = ui.scope_id().with("tc_time_sig");
    let resp = ui.put(
        sig_rect,
        text_edit(sig_id, &mut bufs.time_sig, sig_w - 4.0, ROW_H),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.timesig"));
    select_all_on_focus(ui, &resp, sig_id, &bufs.time_sig);
    if resp.lost_focus()
        && let Some((numerator, denominator_power)) = time_format::parse_time_sig(&bufs.time_sig)
    {
        events.push(TimecodeEvent::TimeSig {
            numerator,
            denominator_power,
        });
    }
    let ppq_id = ui.scope_id().with("tc_ppq");
    let resp = ui.put(
        ppq_rect,
        text_edit(ppq_id, &mut bufs.ppq, ppq_rect.width() - 4.0, ROW_H),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.ppq"));
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
    let pos_id = ui.scope_id().with("tc_pos");
    let resp = ui.put(
        cell(1, 0),
        text_edit(pos_id, &mut bufs.pos, COL_WIDTHS[1] - 8.0, ROW_H),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.position"));
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
    let time_id = ui.scope_id().with("tc_time");
    let resp = ui.put(
        cell(1, 1),
        text_edit(time_id, &mut bufs.time, COL_WIDTHS[1] - 8.0, ROW_H),
    );
    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.seconds"));
    select_all_on_focus(ui, &resp, time_id, &bufs.time);
    if resp.lost_focus()
        && let Some(secs) = time_format::parse_time(&bufs.time)
    {
        let t = seconds_to_tick(&model.tempo_map, secs);
        events.push(TimecodeEvent::CursorTick(t));
    }

    // ── 量化（列 2 上）：入口已迁至传输栏时间码左侧的量化按钮 ──

    // ── 调式（列 2 下）──
    {
        let text = key_sig_text(key);
        let cell_rect = cell(2, 1);
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell_rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    // 过长的调式名用负字距压缩到格宽内（留 4px 边距）。
                    let resp = fit_hover_text(ui, &text, cell_rect.width() - 4.0);
                    crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.keysig"));
                    if resp.clicked() {
                        events.push(TimecodeEvent::OpenScaleDialog);
                    }
                },
            );
        });
    }

    // ── 总输出电平表（列 3，跨两行；无刻度）──
    {
        let top = cell(3, 0);
        let bottom = cell(3, 1);
        let meter_rect = egui::Rect::from_min_max(
            egui::pos2(top.min.x + 4.0, rect.min.y + 4.0),
            egui::pos2(bottom.max.x - 4.0, rect.max.y - 4.0),
        );
        crate::widgets::meter::compact_meter(
            ui.painter(),
            meter_rect,
            data.master_meter.0,
            data.master_meter.1,
        );
        let resp = ui.interact(
            meter_rect,
            ui.scope_id().with("tc_master_meter"),
            egui::Sense::hover(),
        );
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.timecode.master_meter"));
    }

    ui.data_mut(|d| d.insert_temp(buf_id, bufs));
    (rect, events)
}

/// 秒 → tick（tempo_map 单调，二分反查）。
///
/// 上限用 `u32::MAX`（不能用 `tick_length`：空工程/光标超出工程末尾时
/// tick_length 会截断跳转结果）。
fn seconds_to_tick(tempo_map: &yinhe_core::TempoMap, seconds: f64) -> f64 {
    if seconds < 0.0 {
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
    let tick = hi.round();
    // 0 BPM 剪切段时间恒定：二分落在剪切起点时直接推进到区间末端。
    for &(start, end) in &tempo_map.skip_ranges {
        if tick >= start as f64 && tick < end as f64 {
            return end as f64;
        }
    }
    tick
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

    #[test]
    fn seconds_to_tick_jumps_over_zero_bpm_cut() {
        let tm = yinhe_core::TempoMap {
            ticks_per_beat: 480,
            tempo_segments: vec![
                yinhe_core::TempoSegment {
                    start_tick: 0,
                    start_time: 0.0,
                    micros_per_quarter: 500_000,
                },
                // 0 BPM 剪切段：mpq=0，时间恒定在 0.5s。
                yinhe_core::TempoSegment {
                    start_tick: 480,
                    start_time: 0.5,
                    micros_per_quarter: 0,
                },
                yinhe_core::TempoSegment {
                    start_tick: 960,
                    start_time: 0.5,
                    micros_per_quarter: 1_000_000,
                },
            ],
            skip_ranges: vec![(480, 960)],
            ..Default::default()
        };
        assert_eq!(seconds_to_tick(&tm, 0.0), 0.0);
        assert_eq!(seconds_to_tick(&tm, 0.25), 240.0);
        // 0.5s 落在剪切平台上 → 直接跳到 960，而不是停在 480。
        assert_eq!(seconds_to_tick(&tm, 0.5), 960.0);
        assert_eq!(seconds_to_tick(&tm, 1.0), 1200.0);
    }
}
