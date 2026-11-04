//! Write a `yinhe_core::YinModel` as Standard MIDI File bytes.
//!
//! Produces a Type-1 SMF: track 0 is the conductor (tempo + time-sig),
//! tracks 1..N+1 are the YinModel tracks. Each YinModel track flattens
//! `notes / automation_lanes / program_change` into one event stream
//! sorted by tick. RPN/NRPN lanes expand to their CC selector sequences.

use std::io::{BufWriter, Cursor, Write};

use midly::num::{u4, u7, u24};
use midly::{MetaMessage, MidiMessage, PitchBend, TrackEventKind};

use yinhe_core::{TrackData, YinModel};
use yinhe_types::automation::{MidiBinding, binding_max, builtin_param};
use yinhe_types::{AutomationLane, AutomationTarget, SegmentShape};

use crate::encoding::MidiImportEncoding;
use crate::error::MidiError;

/// MIDI导出选项（对应 设置→MIDI导出 栏）。
#[derive(Debug, Clone)]
pub struct MidiExportOptions {
    pub encoding: MidiImportEncoding,
    pub rpn_full: bool,
    pub curve_interpolate: bool,
    pub curve_density: u32,
    pub strip_empty_tracks: bool,
    pub dedup_overlaps: bool,
}

impl Default for MidiExportOptions {
    fn default() -> Self {
        Self {
            encoding: MidiImportEncoding::Utf8,
            rpn_full: true,
            curve_interpolate: false,
            curve_density: 1,
            strip_empty_tracks: true,
            dedup_overlaps: false,
        }
    }
}

fn encode_text(encoding: MidiImportEncoding, s: &str) -> Vec<u8> {
    if encoding == MidiImportEncoding::Utf8 {
        s.as_bytes().to_vec()
    } else {
        encoding.encode(s)
    }
}

fn leak_bytes(v: Vec<u8>) -> &'static [u8] {
    Box::leak(v.into_boxed_slice())
}

/// Serialize a `YinModel` to SMF bytes (Standard MIDI File, format 1).
pub fn write_to_bytes(model: &YinModel) -> Result<Vec<u8>, MidiError> {
    write_with_options(model, &MidiExportOptions::default())
}

/// 带选项的导出（供设置栏驱动），结果写入内存缓冲。
pub fn write_with_options(
    model: &YinModel,
    opts: &MidiExportOptions,
) -> Result<Vec<u8>, MidiError> {
    let mut out = Cursor::new(Vec::new());
    write_to_writer(model, opts, &mut out)?;
    Ok(out.into_inner())
}

/// 流式导出到文件：只在内存里保留当前轨的索引和 `MTrk` 缓冲。
///
/// 旧实现把全部轨的事件和最终 SMF 都放在内存里；2000 万音符时额外峰值
/// 可达 2~3GB，移动端会被系统直接杀掉。
pub fn write_to_file(
    model: &YinModel,
    path: impl AsRef<std::path::Path>,
    opts: &MidiExportOptions,
) -> Result<(), MidiError> {
    let file = std::fs::File::create(path.as_ref())?;
    let mut out = BufWriter::new(file);
    write_to_writer(model, opts, &mut out)?;
    out.flush()?;
    Ok(())
}

/// 流式导出的通用 writer 版本（测试可用 `Cursor<Vec<u8>>`）。
pub fn write_to_writer<W: Write>(
    model: &YinModel,
    opts: &MidiExportOptions,
    out: &mut W,
) -> Result<(), MidiError> {
    let ppq = model.meta.ppq;
    let mut per_track_notes = build_note_index(model);

    if opts.dedup_overlaps {
        for notes in &mut per_track_notes {
            dedup_note_index(notes);
        }
    }

    let color_payloads: Vec<Option<Vec<u8>>> = model
        .tracks
        .iter()
        .map(|t| {
            if t.color != yinhe_core::DEFAULT_TRACK_COLOR {
                Some(vec![
                    0x00,
                    0x0F,
                    0x7F,
                    0x00,
                    (t.color[0] * 255.0).round() as u8,
                    (t.color[1] * 255.0).round() as u8,
                    (t.color[2] * 255.0).round() as u8,
                    (t.color[3] * 255.0).round() as u8,
                ])
            } else {
                None
            }
        })
        .collect();

    let included: Vec<bool> = model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let notes = &per_track_notes[i];
            !opts.strip_empty_tracks
                || !(notes.is_empty()
                    && t.automation_lanes.is_empty()
                    && t.program_change.is_empty()
                    && t.lyrics.is_empty())
        })
        .collect();
    let ntrks = 1 + included.iter().filter(|x| **x).count();

    // SMF 头（format 1）。逐轨写 `MTrk`，mem 里只有当前轨的字节缓冲。
    out.write_all(b"MThd")?;
    out.write_all(&6u32.to_be_bytes())?;
    out.write_all(&1u16.to_be_bytes())?;
    out.write_all(&(ntrks as u16).to_be_bytes())?;
    out.write_all(&(ppq.min(0x7FFF) as u16).to_be_bytes())?;

    let mut conductor = EventBuf::new();
    if !model.meta.name.is_empty() {
        if opts.encoding == MidiImportEncoding::Utf8 {
            conductor.push(
                0,
                &TrackEventKind::Meta(MetaMessage::TrackName(model.meta.name.as_bytes())),
            );
        } else {
            let encoded = encode_text(opts.encoding, &model.meta.name);
            conductor.push(0, &TrackEventKind::Meta(MetaMessage::TrackName(&encoded)));
        }
    }
    for (tick, kind) in build_conductor_events(model, opts.encoding) {
        conductor.push(tick, &kind);
    }
    write_chunk(out, conductor.finish())?;

    for (i, t) in model.tracks.iter().enumerate() {
        if !included[i] {
            continue;
        }
        // 取走本轨索引，写完立即释放；不让已导出的轨继续占内存。
        let notes = std::mem::take(&mut per_track_notes[i]);
        write_note_track(out, t, &notes, color_payloads[i].as_deref(), opts)?;
    }
    Ok(())
}

/// 单个音符的紧凑导出索引（12 字节）。
///
/// `Note` 的 `id` / `track` 字段导出时用不上；2000 万音符时每音符省 8 字节
/// 就是 160MB。
#[derive(Clone, Copy, Debug)]
struct NoteRef {
    key: u8,
    velocity: u8,
    start_tick: u32,
    end_tick: u32,
}

/// 一次扫描 `model.notes`，按轨道收集紧凑索引。
///
/// 遍历顺序是 key 桶升序、桶内 `start_tick` 升序，因此每条轨的索引天然按
/// `(key, start_tick)` 有序；去重和归并都依赖这个顺序。
fn build_note_index(model: &YinModel) -> Vec<Vec<NoteRef>> {
    let mut per_track: Vec<Vec<NoteRef>> = vec![Vec::new(); model.tracks.len()];
    for (key, bucket) in model.notes.iter().enumerate() {
        for n in bucket.iter() {
            if let Some(notes) = per_track.get_mut(n.track as usize) {
                notes.push(NoteRef {
                    key: key as u8,
                    velocity: n.velocity,
                    start_tick: n.start_tick,
                    end_tick: n.end_tick,
                });
            }
        }
    }
    per_track
}

/// `dedup_overlaps`：丢弃同 key 上与上一个音符重叠的音符。
fn dedup_note_index(notes: &mut Vec<NoteRef>) {
    if notes.len() <= 1 {
        return;
    }
    // 桶内只保证 start_tick 有序，同 start 的 end 顺序不定；旧实现按
    // `(key, start, end)` 稳定排序，这里保持同样的语义。
    notes.sort_by_key(|n| (n.key, n.start_tick, n.end_tick));
    let mut write = 0;
    let mut last_key = 0u8;
    let mut last_end = 0u32;
    for read in 0..notes.len() {
        let n = notes[read];
        if write > 0 && n.key == last_key && n.start_tick < last_end {
            continue;
        }
        last_key = n.key;
        last_end = n.end_tick;
        notes[write] = n;
        write += 1;
    }
    notes.truncate(write);
}

/// 逐轨流式写一条音符轨：音符与非音符事件按 tick 归并，边归并边编码。
fn write_note_track<W: Write>(
    out: &mut W,
    track: &TrackData,
    notes: &[NoteRef],
    color_payload: Option<&[u8]>,
    opts: &MidiExportOptions,
) -> Result<(), MidiError> {
    let ch = u4::new(track.channel & 0x0F);
    let aux = build_aux_events(track, color_payload, opts);

    // NoteOn 按 start、NoteOff 按 end 各排一份下标。索引只有 4 字节/音符，
    // 比整轨 `TrackEvent` 数组小一个数量级。
    let mut on_ticks: Vec<(u32, u32)> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.start_tick, i as u32))
        .collect();
    on_ticks.sort_unstable();
    let mut off_ticks: Vec<(u32, u32)> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.end_tick, i as u32))
        .collect();
    off_ticks.sort_unstable();

    let mut buf = EventBuf::new();
    if !track.name.is_empty() {
        if opts.encoding == MidiImportEncoding::Utf8 {
            buf.push(
                0,
                &TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
            );
        } else {
            let encoded = encode_text(opts.encoding, &track.name);
            buf.push(0, &TrackEventKind::Meta(MetaMessage::TrackName(&encoded)));
        }
    }

    let mut on_pos = 0;
    let mut off_pos = 0;
    let mut aux_pos = 0;
    let mut same_tick: Vec<(u32, bool)> = Vec::new();

    loop {
        let mut tick = u32::MAX;
        if let Some(&(t, _)) = on_ticks.get(on_pos) {
            tick = tick.min(t);
        }
        if let Some(&(t, _)) = off_ticks.get(off_pos) {
            tick = tick.min(t);
        }
        if let Some((t, _)) = aux.get(aux_pos) {
            tick = tick.min(*t);
        }
        if tick == u32::MAX {
            break;
        }

        // 旧实现是「音符先插入、其它事件后插入」再稳定排序，所以同 tick
        // 永远是音符在前；音符内部按原索引、On 先于 Off。
        if on_ticks.get(on_pos).map(|x| x.0) == Some(tick)
            || off_ticks.get(off_pos).map(|x| x.0) == Some(tick)
        {
            same_tick.clear();
            while on_ticks.get(on_pos).map(|x| x.0) == Some(tick) {
                same_tick.push((on_ticks[on_pos].1, false));
                on_pos += 1;
            }
            while off_ticks.get(off_pos).map(|x| x.0) == Some(tick) {
                same_tick.push((off_ticks[off_pos].1, true));
                off_pos += 1;
            }
            same_tick.sort_unstable();
            for &(idx, is_off) in &same_tick {
                let n = notes[idx as usize];
                let message = if is_off {
                    MidiMessage::NoteOff {
                        key: u7::new(n.key & 0x7F),
                        vel: u7::new(0),
                    }
                } else {
                    MidiMessage::NoteOn {
                        key: u7::new(n.key & 0x7F),
                        vel: u7::new(n.velocity & 0x7F),
                    }
                };
                buf.push(
                    tick,
                    &TrackEventKind::Midi {
                        channel: ch,
                        message,
                    },
                );
            }
        }

        while aux.get(aux_pos).map(|e| e.0) == Some(tick) {
            buf.push(tick, &aux[aux_pos].1);
            aux_pos += 1;
        }
    }

    write_chunk(out, buf.finish())
}

fn build_conductor_events<'a>(
    model: &'a YinModel,
    encoding: MidiImportEncoding,
) -> Vec<(u32, TrackEventKind<'a>)> {
    let mut events: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    for ev in &model.conductor.tempo.events {
        // 0 BPM 是“跳转”标记；MIDI 无法表示 BPM 0，写成 24-bit 最大 MPQ
        //（= 最低可行 BPM，约 3.576）。保留事件和原始时间轴，不展平。
        let mpq = if ev.value > 0.0 {
            (60_000_000.0 / ev.value as f64).round() as u32
        } else {
            0x00FF_FFFF
        };
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(mpq))),
        ));
    }
    for ev in &model.conductor.time_sig {
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                ev.numerator,
                ev.denominator,
                24,
                8,
            )),
        ));
    }
    for ev in &model.conductor.key_sig {
        let (sf, mi) = ev.scale.to_midi_sf_mi(ev.root);
        events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::KeySignature(sf, mi != 0)),
        ));
    }
    for ev in &model.conductor.markers {
        let bytes: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Marker(bytes))));
    }
    for ev in &model.conductor.lyrics {
        let bytes: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Lyric(bytes))));
    }

    // 艺术家写回 track 0 的 Copyright（FF 02），每位一条，编码方式与 name 一致。
    // 旧工程没有 artists 时回退单 artist 字段。
    let mut artist_names: Vec<&str> = model
        .meta
        .artists
        .iter()
        .map(|a| a.name.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if artist_names.is_empty() && !model.meta.artist.trim().is_empty() {
        artist_names.push(model.meta.artist.trim());
    }
    for name in artist_names {
        let bytes: &'a [u8] = if encoding == MidiImportEncoding::Utf8 {
            name.as_bytes()
        } else {
            leak_bytes(encode_text(encoding, name))
        };
        events.push((0, TrackEventKind::Meta(MetaMessage::Copyright(bytes))));
    }

    events.sort_by_key(|e| e.0);
    events
}

/// 一条轨除音符外的事件（自动化 / 音色 / 端口 / 颜色 / 歌词）。
///
/// 这些事件通常远少于音符，可以直接在内存里排序；音符走
/// [`write_note_track`] 的归并流。
fn build_aux_events<'a>(
    track: &'a TrackData,
    color_payload: Option<&'a [u8]>,
    opts: &MidiExportOptions,
) -> Vec<(u32, TrackEventKind<'a>)> {
    let ch = u4::new(track.channel & 0x0F);
    let mut events: Vec<(u32, TrackEventKind<'a>)> = Vec::new();

    for lane in &track.automation_lanes {
        let n = lane.events.len();
        for (idx, ev) in lane.events.iter().enumerate() {
            push_lane_event(&mut events, lane, ev.tick, ev.value, ch, opts.rpn_full);
            if opts.curve_interpolate && idx + 1 < n && !matches!(ev.shape, SegmentShape::Step) {
                let next = &lane.events[idx + 1];
                let tick1 = ev.tick;
                let tick2 = next.tick;
                if tick2 > tick1 {
                    let v1 = ev.value;
                    let v2 = next.value;
                    let span = (tick2 - tick1) as f32;
                    let density = opts.curve_density.max(1);
                    let mut t = tick1.saturating_add(density);
                    while t < tick2 {
                        let frac = (t - tick1) as f32 / span;
                        let f = ev.shape.interpolate(frac);
                        let v = v1 + (v2 - v1) * f;
                        push_lane_event(&mut events, lane, t, v, ch, opts.rpn_full);
                        t = t.saturating_add(density);
                    }
                }
            }
        }
    }

    for ev in &track.program_change {
        if ev.bank_msb != 0xFF {
            events.push((
                ev.tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(0),
                        value: u7::new(ev.bank_msb & 0x7F),
                    },
                },
            ));
        }
        if ev.bank_lsb != 0xFF {
            events.push((
                ev.tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::Controller {
                        controller: u7::new(32),
                        value: u7::new(ev.bank_lsb & 0x7F),
                    },
                },
            ));
        }
        events.push((
            ev.tick,
            TrackEventKind::Midi {
                channel: ch,
                message: MidiMessage::ProgramChange {
                    program: u7::new(ev.program & 0x7F),
                },
            },
        ));
    }

    if track.port != 0 {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiPort(midly::num::u7::new(
                track.port & 0x7F,
            ))),
        ));
    }
    if let Some(payload) = color_payload {
        events.push((0, TrackEventKind::Meta(MetaMessage::Unknown(0x0A, payload))));
    }
    if let Some(ch) = track.channel_prefix {
        events.push((
            0,
            TrackEventKind::Meta(MetaMessage::MidiChannel(midly::num::u4::new(ch & 0x0F))),
        ));
    }

    for ev in &track.lyrics {
        let bytes: &'a [u8] = if opts.encoding == MidiImportEncoding::Utf8 {
            ev.text.as_bytes()
        } else {
            leak_bytes(encode_text(opts.encoding, &ev.text))
        };
        events.push((ev.tick, TrackEventKind::Meta(MetaMessage::Lyric(bytes))));
    }

    events.sort_by_key(|e| e.0);
    events
}

/// 归一化值 → 原始整数（四舍五入并夹取到 `0..max`）。
fn normalized_to_raw(value: f32, max: f32) -> u16 {
    (value * max).round().clamp(0.0, max) as u16
}

fn push_cc<'a>(
    events: &mut Vec<(u32, TrackEventKind<'a>)>,
    tick: u32,
    ch: u4,
    controller: u8,
    value: u8,
) {
    events.push((
        tick,
        TrackEventKind::Midi {
            channel: ch,
            message: MidiMessage::Controller {
                controller: u7::new(controller),
                value: u7::new(value),
            },
        },
    ));
}

/// RPN/NRPN 数据输入序列：选择参数（CC101/100 或 CC99/98）+ CC6 高 7 位
/// + 14-bit 时的 CC38 低 7 位。
fn push_data_entry<'a>(
    events: &mut Vec<(u32, TrackEventKind<'a>)>,
    tick: u32,
    ch: u4,
    nrpn: bool,
    parameter: u16,
    value: f32,
    rpn_full: bool,
) {
    let raw_max = if nrpn {
        16383.0
    } else {
        binding_max(MidiBinding::Rpn(parameter))
    };
    let raw = normalized_to_raw(value, raw_max);
    let is_14bit = raw_max > 127.0;
    let (msb_cc, lsb_cc) = if nrpn { (99, 98) } else { (101, 100) };
    let (data_msb, data_lsb) = if is_14bit {
        (((raw >> 7) & 0x7F) as u8, (raw & 0x7F) as u8)
    } else {
        (raw as u8, 0)
    };
    push_cc(events, tick, ch, msb_cc, ((parameter >> 8) & 0x7F) as u8);
    push_cc(events, tick, ch, lsb_cc, (parameter & 0x7F) as u8);
    push_cc(events, tick, ch, 6, data_msb);
    if if rpn_full {
        is_14bit
    } else {
        is_14bit && data_lsb != 0
    } {
        push_cc(events, tick, ch, 38, data_lsb);
    }
}

fn push_lane_event<'a>(
    events: &mut Vec<(u32, TrackEventKind<'a>)>,
    lane: &AutomationLane,
    tick: u32,
    value: f32,
    ch: u4,
    rpn_full: bool,
) {
    match &lane.target {
        AutomationTarget::Param { device, id, .. } => {
            // 非内置参数（第三方插件）无 MIDI 语义，与 Tempo 一样不导出。
            let Some(info) = builtin_param(device, *id) else {
                return;
            };
            let max = binding_max(info.midi);
            match info.midi {
                MidiBinding::Cc(controller) => {
                    push_cc(
                        events,
                        tick,
                        ch,
                        controller,
                        normalized_to_raw(value, max) as u8,
                    );
                }
                MidiBinding::PitchBend => {
                    events.push((
                        tick,
                        TrackEventKind::Midi {
                            channel: ch,
                            message: MidiMessage::PitchBend {
                                bend: PitchBend(midly::num::u14::new(normalized_to_raw(
                                    value, max,
                                ))),
                            },
                        },
                    ));
                }
                MidiBinding::Rpn(parameter) => {
                    push_data_entry(events, tick, ch, false, parameter, value, rpn_full);
                }
            }
        }
        AutomationTarget::CC { controller } => {
            push_cc(
                events,
                tick,
                ch,
                *controller,
                normalized_to_raw(value, 127.0) as u8,
            );
        }
        AutomationTarget::PitchBend => {
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: ch,
                    message: MidiMessage::PitchBend {
                        bend: PitchBend(midly::num::u14::new(normalized_to_raw(value, 16383.0))),
                    },
                },
            ));
        }
        AutomationTarget::Rpn { parameter } => {
            push_data_entry(events, tick, ch, false, *parameter, value, rpn_full);
        }
        AutomationTarget::Nrpn { parameter } => {
            push_data_entry(events, tick, ch, true, *parameter, value, rpn_full);
        }
        AutomationTarget::Tempo => {}
    }
}

/// SMF 轨道事件编码缓冲。
///
/// 复刻 midly 的编码方式（含 running status 压缩），但逐事件追加，不需要
/// 先把整轨变成 `Vec<TrackEvent>`。2000 万音符时后者光事件数组就要几百 MB。
struct EventBuf {
    buf: Vec<u8>,
    running_status: Option<u8>,
    last_tick: u32,
}

impl EventBuf {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            running_status: None,
            last_tick: 0,
        }
    }

    /// 追加一个绝对 tick 的事件；delta 由内部计算。
    fn push(&mut self, tick: u32, kind: &TrackEventKind<'_>) {
        write_varlen(&mut self.buf, tick.saturating_sub(self.last_tick));
        self.last_tick = tick;
        match kind {
            TrackEventKind::Midi { channel, message } => {
                let status = midi_status_nibble(message) << 4 | channel.as_int();
                if self.running_status != Some(status) {
                    self.buf.push(status);
                    self.running_status = Some(status);
                }
                push_midi_message(&mut self.buf, message);
            }
            TrackEventKind::SysEx(data) => {
                self.running_status = None;
                self.buf.push(0xF0);
                write_varlen(&mut self.buf, data.len() as u32);
                self.buf.extend_from_slice(data);
            }
            TrackEventKind::Escape(data) => {
                self.running_status = None;
                self.buf.push(0xF7);
                write_varlen(&mut self.buf, data.len() as u32);
                self.buf.extend_from_slice(data);
            }
            TrackEventKind::Meta(meta) => {
                self.running_status = None;
                self.buf.push(0xFF);
                push_meta(&mut self.buf, meta);
            }
        }
    }

    /// 追加 EndOfTrack 并取走字节缓冲。
    fn finish(mut self) -> Vec<u8> {
        self.push(
            self.last_tick,
            &TrackEventKind::Meta(MetaMessage::EndOfTrack),
        );
        self.buf
    }
}

/// 把一条轨的字节缓冲包成 `MTrk` chunk 写出去。
fn write_chunk<W: Write>(out: &mut W, bytes: Vec<u8>) -> Result<(), MidiError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| MidiError::Io(std::io::Error::other("MIDI 轨道超过 4GB")))?;
    out.write_all(b"MTrk")?;
    out.write_all(&len.to_be_bytes())?;
    out.write_all(&bytes)?;
    Ok(())
}

/// 7-bit 变长整数（SMF delta time / 元事件长度）。
fn write_varlen(out: &mut Vec<u8>, value: u32) {
    let mut skipping = true;
    for i in (0..4).rev() {
        let byte = ((value >> (i * 7)) & 0x7F) as u8;
        if skipping && byte == 0 && i != 0 {
            continue;
        }
        skipping = false;
        out.push(if i == 0 { byte } else { byte | 0x80 });
    }
}

fn midi_status_nibble(message: &MidiMessage) -> u8 {
    match message {
        MidiMessage::NoteOff { .. } => 0x8,
        MidiMessage::NoteOn { .. } => 0x9,
        MidiMessage::Aftertouch { .. } => 0xA,
        MidiMessage::Controller { .. } => 0xB,
        MidiMessage::ProgramChange { .. } => 0xC,
        MidiMessage::ChannelAftertouch { .. } => 0xD,
        MidiMessage::PitchBend { .. } => 0xE,
    }
}

fn push_midi_message(out: &mut Vec<u8>, message: &MidiMessage) {
    match *message {
        MidiMessage::NoteOff { key, vel }
        | MidiMessage::NoteOn { key, vel }
        | MidiMessage::Aftertouch { key, vel } => {
            out.push(key.as_int());
            out.push(vel.as_int());
        }
        MidiMessage::Controller { controller, value } => {
            out.push(controller.as_int());
            out.push(value.as_int());
        }
        MidiMessage::ProgramChange { program } => out.push(program.as_int()),
        MidiMessage::ChannelAftertouch { vel } => out.push(vel.as_int()),
        MidiMessage::PitchBend { bend } => {
            // PitchBend 在 SMF 里是 LSB 在前。
            let raw = bend.0.as_int();
            out.push((raw & 0x7F) as u8);
            out.push((raw >> 7) as u8);
        }
    }
}

fn push_meta(out: &mut Vec<u8>, meta: &MetaMessage<'_>) {
    fn write_msg(out: &mut Vec<u8>, type_byte: u8, data: &[u8]) {
        out.push(type_byte);
        write_varlen(out, data.len() as u32);
        out.extend_from_slice(data);
    }
    match meta {
        MetaMessage::TrackNumber(track_num) => match track_num {
            None => write_msg(out, 0x00, &[]),
            Some(n) => write_msg(out, 0x00, &n.to_be_bytes()),
        },
        MetaMessage::Text(data) => write_msg(out, 0x01, data),
        MetaMessage::Copyright(data) => write_msg(out, 0x02, data),
        MetaMessage::TrackName(data) => write_msg(out, 0x03, data),
        MetaMessage::InstrumentName(data) => write_msg(out, 0x04, data),
        MetaMessage::Lyric(data) => write_msg(out, 0x05, data),
        MetaMessage::Marker(data) => write_msg(out, 0x06, data),
        MetaMessage::CuePoint(data) => write_msg(out, 0x07, data),
        MetaMessage::ProgramName(data) => write_msg(out, 0x08, data),
        MetaMessage::DeviceName(data) => write_msg(out, 0x09, data),
        MetaMessage::MidiChannel(chan) => write_msg(out, 0x20, &[chan.as_int()]),
        MetaMessage::MidiPort(port) => write_msg(out, 0x21, &[port.as_int()]),
        MetaMessage::EndOfTrack => write_msg(out, 0x2F, &[]),
        MetaMessage::Tempo(micros_per_beat) => {
            write_msg(out, 0x51, &micros_per_beat.as_int().to_be_bytes()[1..])
        }
        // 导出器不会产生 SmpteOffset；midly 的编码函数不公开，这里留空。
        MetaMessage::SmpteOffset(_) => write_msg(out, 0x54, &[]),
        MetaMessage::TimeSignature(num, den, ticks_per_click, thirty_seconds) => {
            write_msg(out, 0x58, &[*num, *den, *ticks_per_click, *thirty_seconds])
        }
        MetaMessage::KeySignature(sharps, minor) => {
            write_msg(out, 0x59, &[*sharps as u8, u8::from(*minor)])
        }
        MetaMessage::SequencerSpecific(data) => write_msg(out, 0x7F, data),
        MetaMessage::Unknown(type_byte, data) => write_msg(out, *type_byte, data),
    }
}
