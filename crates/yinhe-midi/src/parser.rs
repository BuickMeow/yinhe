//! Parse Standard MIDI File bytes directly into a `yinhe_core::YinModel`.
//!
//! Single-pass per track: NoteOn/NoteOff pairing, port/channel prefix
//! tracking, CC/PB/PC collection, and RPN/NRPN state-machine decoding all
//! happen in one walk. Conductor events (tempo, time signature) are
//! collected across all tracks first.
//!
//! Control events are unified into `AutomationLane` — one lane per
//! parameter per track. RPN and NRPN are decoded from their CC sequences
//! and stored as `AutomationTarget::Rpn` / `AutomationTarget::Nrpn`.

use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;

use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape, TimeSigEvent};

use crate::encoding::MidiImportEncoding;
use crate::error::MidiError;

/// Progress reporter type for long-running parses.
#[derive(Clone, Copy, Debug)]
pub struct LoadProgress {
    pub current_track: usize,
    pub total_tracks: usize,
}

/// Fallback ticks-per-beat for SMPTE-timecode MIDI files (which we don't
/// fully support; treat as if metrical with 480 ppq).
const TIMECODE_FALLBACK_TPB: u32 = 480;

/// Parse a .mid file from disk.
pub fn parse_path(path: impl AsRef<Path>) -> Result<YinModel, MidiError> {
    yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
        let data = std::fs::read(path.as_ref())?;
        parse_bytes(&data)
    })
}

/// Parse .mid bytes (UTF-8 track names) without progress callback.
pub fn parse_bytes(data: &[u8]) -> Result<YinModel, MidiError> {
    parse_bytes_with_encoding(data, MidiImportEncoding::Utf8, |_| {})
}

/// Parse .mid bytes with a chosen track-name encoding and progress callback.
pub fn parse_bytes_with_encoding(
    data: &[u8],
    encoding: MidiImportEncoding,
    progress: impl FnMut(LoadProgress) + Send,
) -> Result<YinModel, MidiError> {
    yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
        // 惰性解析：只切出 header + track 块迭代器，不构建全量事件树喵～
        let (header, track_iter) = midly::parse(data)?;

        let ticks_per_beat = match header.timing {
            midly::Timing::Metrical(t) => t.as_int() as u32,
            midly::Timing::Timecode(_, _) => TIMECODE_FALLBACK_TPB,
        };

        // Pass 1: collect conductor events (tempo + time-sig + key-sig + markers + song title) across ALL tracks.
        // 克隆一个惰性迭代器逐事件扫描，扫完即丢，常驻 O(1)。
        let (conductor, song_title) = collect_conductor(track_iter.clone(), encoding)?;

        // Pass 2: per-track parse → TrackData, run in parallel across tracks.
        // Each track parses independently (all state in parse_track is local),
        // so we collect the per-track EventIters first, then fan them out with
        // rayon. Results are gathered in original track order.
        // Skip "conductor-only" tracks: those with no MIDI messages at all
        // (only meta events). These are typical of SMF format-1 files where
        // track 0 is a conductor track.
        let track_events: Vec<midly::EventIter> =
            track_iter.clone().collect::<Result<Vec<_>, _>>()?;
        let total_tracks = track_events.len();

        // 并行解析进度：每完成一个 track 回调一次。
        // 进度基准是"已完成的 track 数"，不再预先报满（旧实现解析开始前就发
        // total/total，模型构建期间进度条已 100%，是"假进度"的根源）。
        // FnMut 不能直接跨并行闭包共享，用 Mutex 包一层（每 track 锁一次，无竞争）。
        let done = std::sync::atomic::AtomicUsize::new(0);
        let progress = std::sync::Mutex::new(progress);
        let parsed: Vec<Option<TrackData>> = track_events
            .into_par_iter()
            .enumerate()
            .map(|(track_idx, events)| {
                let r = parse_track(events, track_idx, encoding);
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Ok(mut p) = progress.lock() {
                    p(LoadProgress {
                        current_track: n,
                        total_tracks,
                    });
                }
                r
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Drop skipped tracks and assign fallback names / final track indices by position.
        let mut tracks: Vec<TrackData> = Vec::with_capacity(total_tracks);
        let mut per_track_notes: Vec<Vec<NoteEvent>> = Vec::with_capacity(total_tracks);
        for mut td in parsed.into_iter().flatten() {
            if td.name.is_empty() {
                td.name = format!("Track {}", tracks.len() + 1);
            }
            // 用最终 model 位置修正 lane.track，避免与后续插入 conductor 后的编号不一致
            for lane in td.automation_lanes.iter_mut() {
                lane.track = tracks.len() as u16;
            }
            per_track_notes.push(td.notes);
            td.notes = Vec::new(); // notes moved out, clear to avoid confusion
            tracks.push(td);
        }

        let meta = ProjectMeta {
            name: song_title.unwrap_or_default(),
            ppq: ticks_per_beat,
            ..ProjectMeta::default()
        };

        let mut model = YinModel {
            conductor: std::sync::Arc::new(conductor),
            tracks: tracks.into_iter().map(std::sync::Arc::new).collect(),
            meta,
            ..Default::default()
        };
        model.load_track_notes(per_track_notes);
        model.rebuild();

        // Ensure a conductor track exists at index 0. This does an O(n)
        // track-index shift if one is missing — kept in the background thread
        // so the UI doesn't freeze on large files.
        ensure_conductor_track(&mut model);

        // Purge mimalloc free pages: after load_track_notes drops the
        // per-track temporary Vecs, many pages are idle in mimalloc's
        // free list.  This hint tells it to munmap them back to the OS,
        // reducing RSS without affecting future allocations.
        yinhe_memtrace::purge_free_pages();

        Ok(model)
    })
}

/// If track 0 is not a conductor track (no notes, no automation, no PC),
/// insert one and shift all existing track indices by 1.
///
/// This replicates the logic from `Document::from_model` but runs in the
/// background parse thread, avoiding O(n) note iteration on the UI thread.
fn ensure_conductor_track(model: &mut YinModel) {
    let has_conductor = model.track_note_count.first().copied().unwrap_or(0) == 0
        && model
            .tracks
            .first()
            .is_some_and(|t| t.automation_lanes.is_empty() && t.program_change.is_empty());
    if has_conductor || model.tracks.is_empty() {
        return;
    }

    let mut conductor = TrackData::new(0, 0);
    conductor.name = "Conductor".to_string();
    for bucket in model.notes.iter_mut() {
        for n in Arc::make_mut(bucket).iter_mut() {
            n.track += 1;
        }
    }
    for track in model.tracks.iter_mut() {
        let track = Arc::make_mut(track);
        for lane in track.automation_lanes.iter_mut() {
            lane.track += 1;
        }
    }
    model.tracks.insert(0, Arc::new(conductor));
    model.rebuild();
}

// =========================================================
//  Conductor pass (across all tracks)
// =========================================================

/// 按 tick 稳定排序并去重。同一 tick 出现多个 conductor 事件时，
/// MIDI 语义是按顺序依次生效、后者覆盖前者（如 tan90.mid 的 tick 0
/// 处有连续三个 tempo 事件），因此保留每个 tick 的**最后一个**。
/// 稳定排序保序后反转，让"最后一个"变成去重保留的"第一个"，再反转还原。
fn dedup_conductor_keep_last<T>(events: &mut Vec<T>, mut tick_of: impl FnMut(&T) -> u32) {
    events.sort_by_key(|e| tick_of(e));
    events.reverse();
    events.dedup_by_key(|e| tick_of(e));
    events.reverse();
}

fn collect_conductor(
    track_iter: midly::TrackIter,
    encoding: MidiImportEncoding,
) -> Result<(ConductorData, Option<String>), MidiError> {
    let mut tempo_events: Vec<AutomationEvent> = Vec::new();
    let mut time_sig: Vec<TimeSigEvent> = Vec::new();
    let mut key_sig: Vec<yinhe_types::KeySigEvent> = Vec::new();
    let mut markers: Vec<yinhe_types::MarkerEvent> = Vec::new();
    let mut lyrics: Vec<yinhe_types::LyricsEvent> = Vec::new();
    let mut song_title: Option<String> = None;

    for (track_idx, track_result) in track_iter.enumerate() {
        let events = track_result?;
        let mut tick: u32 = 0;
        for ev in events {
            let ev = ev?;
            tick += ev.delta.as_int();
            match ev.kind {
                midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(us)) => {
                    let mpq = us.as_int() as u64;
                    let bpm = if mpq == 0 {
                        120.0
                    } else {
                        60_000_000.0 / mpq as f64
                    };
                    // MIDI 导入一律使用 Step（保留 MIDI 原生语义）。
                    tempo_events.push(AutomationEvent {
                        tick,
                        value: bpm as f32,
                        shape: SegmentShape::Step,
                    });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(num, den, _, _)) => {
                    time_sig.push(TimeSigEvent {
                        tick,
                        numerator: num,
                        denominator: den,
                    });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::KeySignature(sf, minor)) => {
                    let mi = if minor { 1 } else { 0 };
                    let (root, scale) = yinhe_types::from_midi_sf_mi(sf, mi);
                    key_sig.push(yinhe_types::KeySigEvent { tick, root, scale });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::Marker(text)) => {
                    markers.push(yinhe_types::MarkerEvent {
                        tick,
                        text: encoding.decode(text),
                    });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::CuePoint(text)) => {
                    markers.push(yinhe_types::MarkerEvent {
                        tick,
                        text: encoding.decode(text),
                    });
                }
                // SMF 标准：track 0 的 TrackName（FF 03）= song title。
                // 读为 meta.name，避免被 parse_track 当作普通 track name
                // 然后被 conductor 覆盖为 "Conductor" 而丢失。
                midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name))
                    if track_idx == 0 && song_title.is_none() =>
                {
                    song_title = Some(encoding.decode(name));
                }
                // SMF 允许歌词放在 track 0（conductor-only track）。
                // 这些歌词在 parse_track 里会因为 track 0 无 MIDI 消息被丢弃，
                // 因此这里在 collect_conductor 抢先收集到 ConductorData.lyrics。
                midly::TrackEventKind::Meta(midly::MetaMessage::Lyric(text)) if track_idx == 0 => {
                    lyrics.push(yinhe_types::LyricsEvent {
                        tick,
                        text: encoding.decode(text),
                    });
                }
                _ => {}
            }
        }
    }

    dedup_conductor_keep_last(&mut tempo_events, |e| e.tick);
    dedup_conductor_keep_last(&mut time_sig, |e| e.tick);
    dedup_conductor_keep_last(&mut key_sig, |e| e.tick);
    markers.sort_by_key(|e| e.tick);
    lyrics.sort_by_key(|e| e.tick);

    Ok((
        ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: tempo_events,
            },
            time_sig,
            key_sig,
            markers,
            lyrics,
            chord: Vec::new(),
        },
        song_title,
    ))
}

// =========================================================
//  Per-track pass
// =========================================================

/// 解析 ImageToMidi 私有颜色事件（伪装成 FF 0A meta）。
///
/// payload 布局：`[0x00, 0x0F, channel, 0x00, R, G, B, A, (R2, G2, B2, A2)]`
/// 8 字节 = 单色；12 字节 = 渐变（只取第一组颜色）。
/// channel 为 0..15（指定通道）或 0x7F（全部通道）。
/// 返回 (channel, [r, g, b, a])，RGBA 已归一化到 0..1。
fn parse_color_event(data: &[u8]) -> Option<(u8, [f32; 4])> {
    if (data.len() != 8 && data.len() != 12)
        || data[0] != 0x00
        || data[1] != 0x0F
        || data[3] != 0x00
    {
        return None;
    }
    let channel = data[2];
    if channel > 15 && channel != 0x7F {
        return None;
    }
    Some((
        channel,
        [
            data[4] as f32 / 255.0,
            data[5] as f32 / 255.0,
            data[6] as f32 / 255.0,
            data[7] as f32 / 255.0,
        ],
    ))
}

#[derive(Clone, Copy, Debug)]
struct ActiveNote {
    key: u8,
    velocity: u8,
    start_tick: u32,
    /// Composite (port << 4) | channel of the NoteOn — must match for NoteOff
    /// to pair. Different channels in the same track produce independent
    /// active-note stacks.
    global_channel: u8,
}

/// Per-channel RPN state machine.
///
/// MIDI RPNs are selected with CC101 (MSB) + CC100 (LSB), then written by
/// CC6 (Data Entry MSB) and CC38 (Data Entry LSB). NRPNs use CC99 (MSB) +
/// CC98 (LSB) instead.
///
/// When CC6/38 arrives with both msb/lsb selected, emit an RPN or NRPN
/// AutomationEvent. The selector CCs (101/100/99/98) are NOT stored as
/// plain CC — they are consumed by the RPN/NRPN state machine.
#[derive(Default, Clone, Copy)]
struct RpnState {
    msb: Option<u8>,
    lsb: Option<u8>,
}

/// Per-channel pending Bank Select state.
///
/// CC 0 (Bank MSB) and CC 32 (Bank LSB) are buffered here and folded into
/// the next ProgramChange on the same tick. If no PC follows, they are
/// flushed to automation_lanes at the end of the track.
#[derive(Default, Clone, Copy)]
struct PendingBank {
    msb: Option<(u8, u32)>, // (value, tick)
    lsb: Option<(u8, u32)>, // (value, tick)
}

fn parse_track(
    events: midly::EventIter,
    track_idx: usize,
    encoding: MidiImportEncoding,
) -> Result<Option<TrackData>, MidiError> {
    let mut td = TrackData::new(0, 0);
    td.uuid = uuid::Uuid::new_v4().to_string();

    let mut current_tick: u32 = 0;
    let mut current_port: u8 = 0;
    let mut active_notes: Vec<ActiveNote> = Vec::new();
    let mut first_global_channel: Option<u8> = None;
    // Track whether this track carries any MIDI message. Conductor-only tracks
    // (meta events only, typical of SMF format-1 track 0) are skipped — their
    // tempo/time-sig were already collected by `collect_conductor`.
    let mut has_midi_message = false;

    // RPN state per channel (channel 0..16).
    let mut rpn_state: [RpnState; 16] = [RpnState::default(); 16];
    // NRPN state per channel (channel 0..16).
    let mut nrpn_state: [RpnState; 16] = [RpnState::default(); 16];

    // Pending Bank Select per channel (CC 0 / CC 32).
    let mut pending_bank: [PendingBank; 16] = [PendingBank::default(); 16];

    // ImageToMidi 颜色事件（伪装成 FF 0A Copyright meta）列表：
    // (channel, rgba)。channel = 0..15 指定通道，0x7F = 全部通道。
    let mut color_events: Vec<(u8, [f32; 4])> = Vec::new();

    // Accumulate automation events per target during parsing.
    // Key = (target_variant, controller_or_parameter).
    // We use a Vec<(AutomationTarget, AutomationEvent)> and sort at the end.
    let mut auto_events: Vec<(AutomationTarget, AutomationEvent)> = Vec::new();

    for ev in events {
        let ev = ev?;
        current_tick += ev.delta.as_int();
        match ev.kind {
            midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name_bytes)) => {
                if td.name.is_empty() {
                    td.name = encoding.decode(name_bytes);
                }
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::Lyric(text)) => {
                td.lyrics.push(yinhe_types::LyricsEvent {
                    tick: current_tick,
                    text: encoding.decode(text),
                });
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiPort(port)) => {
                current_port = port.as_int();
            }
            midly::TrackEventKind::Meta(midly::MetaMessage::MidiChannel(ch)) => {
                td.channel_prefix = Some(ch.as_int());
            }
            // ImageToMidi 私有颜色事件：FF 0A meta + 魔数 00 0F。
            // 0x0A 在 SMF 规范中未定义，midly 解析为 Unknown；
            // 非颜色事件的同类 meta 保持忽略。
            midly::TrackEventKind::Meta(midly::MetaMessage::Unknown(0x0A, text)) => {
                if let Some(ev) = parse_color_event(text) {
                    color_events.push(ev);
                }
            }
            midly::TrackEventKind::Midi { channel, message } => {
                has_midi_message = true;
                let ch_raw = channel.as_int();
                let global_ch = (current_port & 0x0F) << 4 | (ch_raw & 0x0F);
                if first_global_channel.is_none() {
                    first_global_channel = Some(global_ch);
                }

                match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        let k = key.as_int();
                        let v = vel.as_int();
                        if v > 0 {
                            active_notes.push(ActiveNote {
                                key: k,
                                velocity: v,
                                start_tick: current_tick,
                                global_channel: global_ch,
                            });
                        } else {
                            // NoteOn with vel=0 == NoteOff
                            resolve_note_off(
                                k,
                                global_ch,
                                current_tick,
                                &mut active_notes,
                                &mut td.notes,
                            );
                        }
                    }
                    midly::MidiMessage::NoteOff { key, .. } => {
                        let k = key.as_int();
                        resolve_note_off(
                            k,
                            global_ch,
                            current_tick,
                            &mut active_notes,
                            &mut td.notes,
                        );
                    }
                    midly::MidiMessage::Controller { controller, value } => {
                        let cc = controller.as_int();
                        let val = value.as_int();
                        let ch_idx = ch_raw as usize;
                        match cc {
                            101 => {
                                // RPN MSB selector
                                rpn_state[ch_idx].msb = Some(val);
                            }
                            100 => {
                                // RPN LSB selector
                                rpn_state[ch_idx].lsb = Some(val);
                            }
                            99 => {
                                // NRPN MSB selector
                                nrpn_state[ch_idx].msb = Some(val);
                            }
                            98 => {
                                // NRPN LSB selector
                                nrpn_state[ch_idx].lsb = Some(val);
                            }
                            0 => {
                                // Bank MSB: buffer for potential PC folding
                                pending_bank[ch_idx].msb = Some((val, current_tick));
                            }
                            32 => {
                                // Bank LSB: buffer for potential PC folding
                                pending_bank[ch_idx].lsb = Some((val, current_tick));
                            }
                            6 => handle_cc6(
                                val,
                                ch_idx,
                                current_tick,
                                &rpn_state,
                                &nrpn_state,
                                &mut auto_events,
                            ),
                            38 => handle_cc38(
                                val,
                                ch_idx,
                                current_tick,
                                &rpn_state,
                                &nrpn_state,
                                &mut auto_events,
                            ),
                            _ => {
                                // All other CC → AutomationTarget::CC
                                auto_events.push((
                                    AutomationTarget::CC { controller: cc },
                                    AutomationEvent {
                                        tick: current_tick,
                                        value: val as f32,
                                        ..Default::default()
                                    },
                                ));
                            }
                        }
                    }
                    midly::MidiMessage::ProgramChange { program } => {
                        let ch_idx = ch_raw as usize;
                        let bank_msb_val = pending_bank[ch_idx].msb;
                        let bank_lsb_val = pending_bank[ch_idx].lsb;
                        let bank_msb = bank_msb_val
                            .filter(|&(_, t)| t == current_tick)
                            .map(|(v, _)| v)
                            .unwrap_or(0xFF);
                        let bank_lsb = bank_lsb_val
                            .filter(|&(_, t)| t == current_tick)
                            .map(|(v, _)| v)
                            .unwrap_or(0xFF);
                        td.program_change.push(PcEvent {
                            tick: current_tick,
                            program: program.as_int(),
                            bank_msb,
                            bank_lsb,
                        });
                        // Clear pending bank values that were consumed (same tick)
                        if bank_msb_val.is_some_and(|(_, t)| t == current_tick) {
                            pending_bank[ch_idx].msb = None;
                        }
                        if bank_lsb_val.is_some_and(|(_, t)| t == current_tick) {
                            pending_bank[ch_idx].lsb = None;
                        }
                    }
                    midly::MidiMessage::PitchBend { bend } => {
                        auto_events.push((
                            AutomationTarget::PitchBend,
                            AutomationEvent {
                                tick: current_tick,
                                value: bend.0.as_int() as f32, // raw 0–16383
                                ..Default::default()
                            },
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Skip conductor-only tracks (no MIDI messages at all).
    if !has_midi_message {
        return Ok(None);
    }

    // Pin port/channel from first MIDI event seen, or default.
    let _ = track_idx; // (kept for future use)
    td.port = current_port;
    td.channel = first_global_channel.map(|gc| gc & 0x0F).unwrap_or(0);

    // 应用颜色事件：优先精确通道匹配，其次 0x7F 全通道通配，最后取第一个。
    // 颜色是音轨级属性，事件出现在音轨内任意位置都作用于整轨。
    if let Some((_, color)) = color_events
        .iter()
        .find(|(ch, _)| *ch == td.channel)
        .or_else(|| color_events.iter().find(|(ch, _)| *ch == 0x7F))
        .or_else(|| color_events.first())
    {
        td.color = *color;
    }

    // Flush pending bank values that were NOT consumed by a ProgramChange.
    // These become plain CC events so nothing is lost.
    for bank in pending_bank.iter() {
        if let Some((val, tick)) = bank.msb {
            auto_events.push((
                AutomationTarget::CC { controller: 0 },
                AutomationEvent {
                    tick,
                    value: val as f32,
                    ..Default::default()
                },
            ));
        }
        if let Some((val, tick)) = bank.lsb {
            auto_events.push((
                AutomationTarget::CC { controller: 32 },
                AutomationEvent {
                    tick,
                    value: val as f32,
                    ..Default::default()
                },
            ));
        }
    }

    // NoteEvent.id 由 YinModel::load_track_notes 统一分配，这里不需要预分配。
    // 在发号前按确定性顺序排序，使 ID 顺序不依赖 MIDI 字节流细节：
    //   1. start_tick 升序（位置）
    //   2. key 升序
    //   3. end_tick 降序（长度从大到小）
    //   4. velocity 降序
    //   5. 全都相同则保持原顺序（稳定排序，平级）
    td.notes.sort_by(|a, b| {
        a.start_tick
            .cmp(&b.start_tick)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| b.end_tick.cmp(&a.end_tick))
            .then_with(|| b.velocity.cmp(&a.velocity))
    });

    td.program_change.sort_by_key(|e| e.tick);

    // Build automation_lanes from accumulated events.
    // Sort by (target, tick) then group into lanes.
    auto_events.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.tick.cmp(&b.1.tick)));

    td.automation_lanes = group_automation_events(auto_events, td.port, td.channel, track_idx);

    Ok(Some(td))
}

/// Group sorted (target, event) pairs into AutomationLane vecs.
fn group_automation_events(
    events: Vec<(AutomationTarget, AutomationEvent)>,
    _port: u8,
    _channel: u8,
    track_idx: usize,
) -> Vec<AutomationLane> {
    if events.is_empty() {
        return Vec::new();
    }
    let mut lanes: Vec<AutomationLane> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let target = events[i].0.clone();
        let start = i;
        while i < events.len() && events[i].0 == target {
            i += 1;
        }
        let lane_events: Vec<AutomationEvent> = events[start..i].iter().map(|(_, e)| *e).collect();
        lanes.push(AutomationLane {
            target,
            track: track_idx as u16,
            events: lane_events,
        });
    }
    lanes
}

/// Handle CC 6 (Data Entry MSB) with RPN/NRPN state machine.
fn handle_cc6(
    val: u8,
    ch_idx: usize,
    current_tick: u32,
    rpn_state: &[RpnState; 16],
    nrpn_state: &[RpnState; 16],
    auto_events: &mut Vec<(AutomationTarget, AutomationEvent)>,
) {
    let rpn = rpn_state[ch_idx];
    let nrpn = nrpn_state[ch_idx];
    if let (Some(msb), Some(lsb)) = (rpn.msb, rpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Rpn { parameter };
        let value = if target.is_14bit() {
            ((val as u16) << 7) as f32
        } else {
            val as f32
        };
        auto_events.push((
            target,
            AutomationEvent {
                tick: current_tick,
                value,
                ..Default::default()
            },
        ));
    } else if let (Some(msb), Some(lsb)) = (nrpn.msb, nrpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        auto_events.push((
            AutomationTarget::Nrpn { parameter },
            AutomationEvent {
                tick: current_tick,
                value: ((val as u16) << 7) as f32,
                ..Default::default()
            },
        ));
    } else {
        auto_events.push((
            AutomationTarget::CC { controller: 6 },
            AutomationEvent {
                tick: current_tick,
                value: val as f32,
                ..Default::default()
            },
        ));
    }
}

/// Handle CC 38 (Data Entry LSB) with RPN/NRPN state machine.
fn handle_cc38(
    val: u8,
    ch_idx: usize,
    current_tick: u32,
    rpn_state: &[RpnState; 16],
    nrpn_state: &[RpnState; 16],
    auto_events: &mut Vec<(AutomationTarget, AutomationEvent)>,
) {
    let rpn = rpn_state[ch_idx];
    let nrpn = nrpn_state[ch_idx];
    if let (Some(msb), Some(lsb)) = (rpn.msb, rpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Rpn { parameter };
        if target.is_14bit() {
            if let Some((_, last)) = auto_events
                .iter_mut()
                .rfind(|(t, e)| *t == target && e.tick == current_tick)
            {
                // 把已有的 14-bit 高 7 位 OR 上当前 7-bit 低字节
                let v = last.value.round() as u16;
                last.value = ((v & 0xFF80) | (val as u16)) as f32;
            } else {
                auto_events.push((
                    target,
                    AutomationEvent {
                        tick: current_tick,
                        value: val as f32,
                        ..Default::default()
                    },
                ));
            }
        } else {
            auto_events.push((
                AutomationTarget::CC { controller: 38 },
                AutomationEvent {
                    tick: current_tick,
                    value: val as f32,
                    ..Default::default()
                },
            ));
        }
    } else if let (Some(msb), Some(lsb)) = (nrpn.msb, nrpn.lsb) {
        let parameter = ((msb as u16) << 8) | lsb as u16;
        let target = AutomationTarget::Nrpn { parameter };
        if let Some((_, last)) = auto_events
            .iter_mut()
            .rfind(|(t, e)| *t == target && e.tick == current_tick)
        {
            let v = last.value.round() as u16;
            last.value = ((v & 0xFF80) | (val as u16)) as f32;
        } else {
            auto_events.push((
                target,
                AutomationEvent {
                    tick: current_tick,
                    value: val as f32,
                    ..Default::default()
                },
            ));
        }
    } else {
        auto_events.push((
            AutomationTarget::CC { controller: 38 },
            AutomationEvent {
                tick: current_tick,
                value: val as f32,
                ..Default::default()
            },
        ));
    }
}

/// Match a NoteOff (or NoteOn vel=0) to the most recent matching NoteOn.
fn resolve_note_off(
    key: u8,
    global_ch: u8,
    end_tick: u32,
    active: &mut Vec<ActiveNote>,
    notes: &mut Vec<NoteEvent>,
) {
    if let Some(idx) = active
        .iter()
        .rposition(|n| n.key == key && n.global_channel == global_ch)
    {
        let n = active.swap_remove(idx);
        notes.push(NoteEvent {
            id: 0, // 由 YinModel::load_track_notes 统一分配
            start_tick: n.start_tick,
            end_tick,
            key: n.key,
            velocity: n.velocity,
        });
    }
}
