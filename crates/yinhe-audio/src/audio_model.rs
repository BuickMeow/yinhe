use std::cmp::Ordering;
use std::sync::Arc;

use std::collections::HashMap;
use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use yinhe_core::YinModel;

use yinhe_types::{AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

pub(crate) struct SortedCC {
    /// 事件时刻（tick 域，u32——模型 NoteEvent/AutomationEvent 的 tick 上限）。
    /// 音频内部统一 tick 域：dispatch/chase 比较不再需要 sample 转换。
    pub(crate) tick: u32,
    pub(crate) channel: u32,
    /// 源音轨索引，用于 mute 时过滤自动化事件。
    pub(crate) track: u16,
    /// 源自动化 lane 索引（轨道内，0..lanes.len()-1）。
    /// `u16::MAX` = 非 lane 事件（ProgramChange 展开），只受 skip_track 过滤。
    /// dispatch 时用它查 AM M/S 动态掩码（与 skip_track 并列），
    /// 使旁通切换不再需要重建事件流。
    pub(crate) lane: u16,
    pub(crate) event: ChannelAudioEvent,
}

/// `SortedCC.lane` 哨兵：ProgramChange 事件（不受 AM lane 掩码过滤）。
pub(crate) const PC_LANE: u16 = u16::MAX;

/// 活跃音符（已 NoteOn 待 NoteOff）。
///
/// `Ord` 按 `end_tick` 升序，相同 end_tick 再按 (key, channel) 区分。
/// 配合 `BinaryHeap<Reverse<ActiveNote>>` 用作 min-heap，让最早结束的音符在堆顶，
/// NoteOff 检测从 O(V) retain 全扫降到 O(ended × log V) 逐个 pop。
#[derive(Clone, Copy)]
pub(crate) struct ActiveNote {
    pub(crate) key: u8,
    /// 目标 dense 通道：MIDI 音符 = xsynth dense；乐器音符 = 乐器 dense。
    pub(crate) dense: u32,
    /// 乐器音符的 CLAP 内部 MIDI 通道（= 音轨 global_channel 低 4 位）。MIDI 音符忽略。
    pub(crate) clap_channel: u8,
    /// 是否为乐器音符（true → NoteOff 喂乐器实例）。
    pub(crate) is_instrument: bool,
    pub(crate) end_tick: u32,
}

impl PartialEq for ActiveNote {
    fn eq(&self, other: &Self) -> bool {
        self.end_tick == other.end_tick
            && self.key == other.key
            && self.dense == other.dense
            && self.is_instrument == other.is_instrument
            && self.clap_channel == other.clap_channel
    }
}
impl Eq for ActiveNote {}
impl PartialOrd for ActiveNote {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ActiveNote {
    fn cmp(&self, other: &Self) -> Ordering {
        self.end_tick
            .cmp(&other.end_tick)
            .then(self.key.cmp(&other.key))
            .then(self.dense.cmp(&other.dense))
            .then(self.is_instrument.cmp(&other.is_instrument))
            .then(self.clap_channel.cmp(&other.clap_channel))
    }
}

/// 音频线程消费的可听音事件（vel > 1），时刻存 **tick**（u32，与模型一致）。
/// 桶内按 `start_tick` 严格升序（YinModel.notes[key] 本身按 start_tick 排序，
/// tick 天然单调，**无需再 sort**）。
///
/// `key` 不存（桶索引即 key）。`id` 用于 undo/redo 后跨 prepared model
/// 引用同一音符（暂未使用，预留）。
///
/// tick 域化：1 亿音符下每条约 24→16 字节（-0.8GB），且 dispatch 比较
/// 不再需要 tick→sample 转换；只有"渲染段边界"才转 sample（每块少量）。
#[repr(C)]
pub(crate) struct AudibleNote {
    pub start_tick: u32,
    pub end_tick: u32,
    pub id: u32,
    pub track: u16,
    pub velocity: u8,
}

/// `PrepareNotes` 的增量结果：`[key] = Some(新桶)` 表示该 key 桶需要替换，
/// `None` 表示桶未变化（音频线程保留旧数据与旧 cursor）。
pub(crate) type AudibleDelta = Box<[Option<Vec<AudibleNote>>; KEY_COUNT]>;

/// Pre-computed model data, built on a worker thread and applied
/// atomically on the audio thread.
pub(crate) struct PreparedModel {
    pub model: AudioModel,
    pub yin_model: Arc<YinModel>,
    /// `Arc` so the same cc_events can be shared between the renderer thread
    /// (for seek/chase dispatch) and the worker thread (for chase computation)
    /// without cloning the (potentially hundreds of thousands of) events.
    pub cc_events: Arc<Vec<SortedCC>>,
    /// KEY_COUNT 个 key 桶的可听音（vel > 1），时刻为 tick（u32）。
    /// 音频线程的 seek / dispatch 只读这份列表，不再访问 YinModel.notes。
    pub audible_notes: Box<[Vec<AudibleNote>; KEY_COUNT]>,
    pub duration_samples: u64,
}

/// Lightweight per-track snapshot the audio engine actually needs.
///
/// We extract only `(global_channel)` per track plus the CC0 bank-select
/// events used for percussion-mode detection, so the audio thread holds a few
/// KB instead of a full deep clone of the model.
pub(crate) struct AudioModel {
    /// `track_channels[i]` = global channel `(port<<4)|channel` for track `i`.
    pub track_channels: Vec<u8>,
    /// 每条音轨的路由：`Some(instrument_channel)` 表示该轨是**乐器轨**，音符/CC
    /// 走 CLAP 乐器实例（按 instrument_channel 路由，独立于 MIDI 源通道）；
    /// `None` 表示普通 MIDI 轨（走 xsynth）。与 `track_channels` 对齐。
    pub track_instrument: Vec<Option<u16>>,
    /// Bank Select MSB declarations per track, for percussion-mode detection.
    /// `(tick, value)` pairs merged from standalone CC0 automation lanes and
    /// CC0 values folded into `PcEvent.bank_msb` (same-tick CC0+PC), sorted by
    /// tick. Values >= 120 select a drum kit (GS/XG convention), values < 120
    /// select a melodic bank. Empty Vec for tracks with no bank declaration.
    pub track_banks: Vec<Vec<(u32, u8)>>,
}

impl AudioModel {
    pub(crate) fn from_model(model: &YinModel) -> Self {
        let track_channels: Vec<u8> = model.tracks.iter().map(|t| t.global_channel()).collect();
        let track_instrument: Vec<Option<u16>> = model
            .tracks
            .iter()
            .map(|t| {
                (t.kind == yinhe_core::TrackKind::Instrument)
                    .then_some(t.instrument_channel)
                    .flatten()
            })
            .collect();
        let track_banks: Vec<Vec<(u32, u8)>> = model
            .tracks
            .iter()
            .map(|t| {
                let mut banks: Vec<(u32, u8)> = Vec::new();
                // 独立 CC0 自动化事件（未被同 tick PC 折叠）。
                if let Some(lane) = t.automation_lanes.iter().find(|l| {
                    matches!(
                        l.target,
                        yinhe_types::AutomationTarget::CC { controller: 0 }
                    )
                }) {
                    banks.extend(
                        lane.events
                            .iter()
                            .map(|e| (e.tick, e.value.round().clamp(0.0, 127.0) as u8)),
                    );
                }
                // 同 tick 被 PC 折叠的 CC0（PcEvent.bank_msb）——否则声明会被丢掉。
                banks.extend(
                    t.program_change
                        .iter()
                        .filter_map(|pc| (pc.bank_msb != 0xFF).then_some((pc.tick, pc.bank_msb))),
                );
                banks.sort_by_key(|&(tick, _)| tick);
                banks
            })
            .collect();
        Self {
            track_channels,
            track_instrument,
            track_banks,
        }
    }

    /// Global channel for a track index, or 0 if out of range.
    pub(crate) fn track_channel(&self, track_idx: usize) -> u8 {
        self.track_channels.get(track_idx).copied().unwrap_or(0)
    }

    /// 音轨是否为乐器轨（走 CLAP 实例）；返回其 instrument_channel。
    pub(crate) fn track_instrument(&self, track_idx: usize) -> Option<u16> {
        self.track_instrument.get(track_idx).copied().flatten()
    }
}

/// Convert a tick value to sample position using the tempo map.
pub(crate) fn tick_to_sample(
    tick: u32,
    segments: &[yinhe_core::TempoSegment],
    tpb: u32,
    sr: f64,
) -> u64 {
    let idx = match segments.binary_search_by_key(&tick, |s| s.start_tick) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let seg = &segments[idx];
    let secs = seg.start_time
        + yinhe_core::ticks_to_seconds((tick - seg.start_tick) as u64, tpb, seg.micros_per_quarter);
    (secs * sr) as u64
}

/// Convert a sample position back to the tick domain (floor), for dispatch
///基准/seek。返回满足 `tick_to_sample(t) <= sample` 的**最大** t。
///
/// 浮点误差防护：floor 后向上校验（tick_to_sample 单调，最多修正 1-2 次），
/// 保证 dispatch 不会因低估基准而漏触发已到位置的事件。
pub(crate) fn sample_to_tick(
    sample: u64,
    segments: &[yinhe_core::TempoSegment],
    tpb: u32,
    sr: f64,
) -> u32 {
    if segments.is_empty() {
        return 0;
    }
    let time = sample as f64 / sr;
    // 找 start_time <= time 的段（按 start_time 二分）
    let idx = segments
        .partition_point(|s| s.start_time <= time)
        .saturating_sub(1);
    let seg = &segments[idx];
    let secs_per_tick = if tpb == 0 {
        0.0
    } else {
        seg.micros_per_quarter as f64 / (tpb as f64 * 1_000_000.0)
    };
    let mut t = if secs_per_tick > 0.0 {
        ((time - seg.start_time) / secs_per_tick).floor() as i64 + seg.start_tick as i64
    } else {
        seg.start_tick as i64
    };
    t = t.max(0);
    // 向上校验：确保返回最大满足 tick_to_sample(t) <= sample 的 t。
    // 若 floor 因浮点误差低估，这里补到真实边界（循环最多几次）。
    while tick_to_sample(t as u32, segments, tpb, sr) <= sample && t < u32::MAX as i64 {
        t += 1;
    }
    (t - 1).max(0) as u32
}

/// 判断 lane 是否被 AM M/S 试听状态旁通。
///
/// - mute：该 lane 直接不发送；
/// - solo：音轨内有任意 lane solo 时，未 solo 的 lane 不发送
///   （主音轨音符发声与其他音轨不受影响）。
pub(crate) fn automation_lane_skipped(
    am_ms: &HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
    track_idx: u16,
    lane: &AutomationLane,
    track_has_solo: bool,
) -> bool {
    let st = am_ms.get(&(track_idx, lane.target.clone()));
    let muted = st.is_some_and(|s| s.mute);
    let soloed = st.is_some_and(|s| s.solo);
    muted || (track_has_solo && !soloed)
}

/// 预计算每条音轨的 lane 跳过掩码：`mask[track][lane_idx] = true` 表示该 lane
/// 事件在 dispatch 时被 AM M/S 旁通。切换代价 O(掩码重建)，与事件流规模无关。
///
/// 规则（与 `automation_lane_skipped` 一致）：
/// - mute：该 lane 直接不发送；
/// - solo：音轨内有任意 lane solo 时，未 solo 的 lane 不发送（作用域 = 音轨内）。
pub(crate) fn build_am_lane_skip(
    model: &YinModel,
    am_ms: &HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
) -> Vec<Vec<bool>> {
    model
        .tracks
        .iter()
        .enumerate()
        .map(|(track_idx, track)| {
            let track_idx_u16 = track_idx as u16;
            let track_has_solo = track.automation_lanes.iter().any(|l| {
                am_ms
                    .get(&(track_idx_u16, l.target.clone()))
                    .is_some_and(|s| s.solo)
            });
            track
                .automation_lanes
                .iter()
                .map(|lane| automation_lane_skipped(am_ms, track_idx_u16, lane, track_has_solo))
                .collect()
        })
        .collect()
}

/// Flatten automation lanes + program changes into sorted, deduped SortedCC events.
///
/// Standard RPN 0/1/2 are sent as high-level xsynth events (PitchBendSensitivity,
/// FineTune, CoarseTune). Non-standard RPN and NRPN use the raw CC sequence.
///
/// `density`: Linear/Curve 段在播放时按多少 tick 间隔展开中间事件。1 = 每 tick 一个事件
/// （最平滑），值越大中间事件越少。Step 段不受影响（保持值到下一点）。
///
/// 所有 lane 的事件**全部展平**（AM M/S 旁通由 dispatch 时的动态掩码负责），
/// 旁通切换无需重建本事件流。
///
/// Returns `Arc<Vec>` so the same events can be shared between the renderer and
/// the worker thread (for chase computation) without cloning.
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    density: u32,
) -> Arc<Vec<SortedCC>> {
    let density = density.max(1);
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let track_idx_u16 = track_idx as u16;
        let channel = track.global_channel() as u32;

        for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
            let lane_idx_u16 = lane_idx as u16;
            let n = lane.events.len();
            for (i, e) in lane.events.iter().enumerate() {
                // tick 域：事件时刻直接存模型的 tick（u32），不再转 sample。
                emit_automation_event(
                    &lane.target,
                    e.value,
                    e.tick,
                    channel,
                    track_idx_u16,
                    lane_idx_u16,
                    &mut cc_events,
                );

                // Linear/Curve 段：在当前事件与下一事件之间按 density 间隔展开中间事件
                if i + 1 < n {
                    let next = &lane.events[i + 1];
                    let tick1 = e.tick;
                    let tick2 = next.tick;
                    if tick2 > tick1 && !matches!(e.shape, SegmentShape::Step) {
                        let v1 = e.value;
                        let v2 = next.value;
                        let span = (tick2 - tick1) as f32;
                        let mut t = tick1.saturating_add(density);
                        while t < tick2 {
                            let frac = (t - tick1) as f32 / span;
                            let f = e.shape.interpolate(frac);
                            let v = v1 + (v2 - v1) * f;
                            emit_automation_event(
                                &lane.target,
                                v,
                                t,
                                channel,
                                track_idx_u16,
                                lane_idx_u16,
                                &mut cc_events,
                            );
                            t = t.saturating_add(density);
                        }
                    }
                }
            }
        }

        for e in &track.program_change {
            push_program_change(e, channel, track_idx_u16, &mut cc_events);
        }
    }

    // 排序：同 tick 同 channel 下，RPN/参数类事件必须排在 PitchBendValue 之前。
    // 原因：xsynth 收到 PitchBendValue 时会按当前 PBS 立即计算弯音并作用于已响 voice，
    // 若 PBS 尚未更新，PB 会用旧 PBS 算出错误音高。见 commit 3490e02。
    // sort_by_key 稳定，同 priority 仍按插入顺序。
    cc_events.sort_by_key(|e| (e.tick, e.channel, dispatch_priority(&e.event)));
    // 去重限定在同一 lane：不同 lane 的同名同值事件必须各自保留，
    // 否则 dispatch 按 lane 查掩码时会把正常 lane 的事件误判成被旁通。
    cc_events.dedup_by(|a, b| a.channel == b.channel && a.lane == b.lane && a.event == b.event);
    Arc::new(cc_events)
}

/// Program Change 展开为 CC 事件序列（bank select + ProgramChange）。
/// 供 flatten（播放事件流）与查询式 chase 共用。
/// 事件标记 `lane = PC_LANE`（非 lane 事件，只受 skip_track 过滤）。
pub(crate) fn push_program_change(
    pc: &yinhe_types::PcEvent,
    channel: u32,
    track: u16,
    out: &mut Vec<SortedCC>,
) {
    let tick = pc.tick;
    if pc.bank_msb != 0xFF {
        out.push(SortedCC {
            tick,
            channel,
            track,
            lane: PC_LANE,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(0, pc.bank_msb)),
        });
    }
    if pc.bank_lsb != 0xFF {
        out.push(SortedCC {
            tick,
            channel,
            track,
            lane: PC_LANE,
            event: ChannelAudioEvent::Control(ControlEvent::Raw(32, pc.bank_lsb)),
        });
    }
    out.push(SortedCC {
        tick,
        channel,
        track,
        lane: PC_LANE,
        event: ChannelAudioEvent::ProgramChange(pc.program),
    });
}

/// 同 tick 同 channel 内的分发优先级：0 = 参数/控制类（RPN、CC、PC），
/// 1 = PitchBendValue。数值小的先发，保证 PBS/FineTune/CoarseTune 等 RPN
/// 参数在 PB 使用它们之前就位。
pub(crate) fn dispatch_priority(event: &ChannelAudioEvent) -> u8 {
    match event {
        ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)) => 1,
        _ => 0,
    }
}

/// 将单个 automation 值转换成 XSynth 事件并推入 `out`。
/// 事件时刻为 tick（u32，与模型一致）。
/// `lane`：源自动化 lane 索引（轨道内），dispatch 查 AM M/S 动态掩码用。
/// 供 `flatten_automation_to_cc_events`（播放事件流）与查询式 chase 共用。
pub(crate) fn emit_automation_event(
    target: &AutomationTarget,
    value: f32,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    out: &mut Vec<SortedCC>,
) {
    match target {
        AutomationTarget::CC { controller } => {
            out.push(SortedCC {
                tick,
                channel,
                track,
                lane,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(
                    *controller,
                    value.round().clamp(0.0, 127.0) as u8,
                )),
            });
        }
        AutomationTarget::PitchBend => {
            out.push(SortedCC {
                tick,
                channel,
                track,
                lane,
                event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    (value - 8192.0) / 8192.0,
                )),
            });
        }
        AutomationTarget::Rpn { parameter } => {
            match parameter {
                0 => {
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(
                            value,
                        )),
                    });
                }
                1 => {
                    let fine = (value - 8192.0) / 8192.0 * 100.0;
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::FineTune(fine)),
                    });
                }
                2 => {
                    let coarse = value - 64.0;
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::CoarseTune(coarse)),
                    });
                }
                _ => {
                    // Non-standard RPN: fall back to CC sequence
                    let msb = ((parameter >> 8) & 0x7F) as u8;
                    let lsb = (parameter & 0x7F) as u8;
                    let (data_msb, data_lsb) = if target.is_14bit() {
                        let v = value.round().clamp(0.0, 16383.0) as u16;
                        (((v >> 7) & 0x7F) as u8, (v & 0x7F) as u8)
                    } else {
                        (value.round().clamp(0.0, 127.0) as u8, 0u8)
                    };
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(101, msb)),
                    });
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(100, lsb)),
                    });
                    out.push(SortedCC {
                        tick,
                        channel,
                        track,
                        lane,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
                    });
                    if data_lsb != 0 {
                        out.push(SortedCC {
                            tick,
                            channel,
                            track,
                            lane,
                            event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                        });
                    }
                }
            }
        }
        AutomationTarget::Nrpn { parameter } => {
            let msb = ((parameter >> 8) & 0x7F) as u8;
            let lsb = (parameter & 0x7F) as u8;
            let v = value.round().clamp(0.0, 16383.0) as u16;
            let data_msb = ((v >> 7) & 0x7F) as u8;
            let data_lsb = (v & 0x7F) as u8;
            out.push(SortedCC {
                tick,
                channel,
                track,
                lane,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(99, msb)),
            });
            out.push(SortedCC {
                tick,
                channel,
                track,
                lane,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(98, lsb)),
            });
            out.push(SortedCC {
                tick,
                channel,
                track,
                lane,
                event: ChannelAudioEvent::Control(ControlEvent::Raw(6, data_msb)),
            });
            if data_lsb != 0 {
                out.push(SortedCC {
                    tick,
                    channel,
                    track,
                    lane,
                    event: ChannelAudioEvent::Control(ControlEvent::Raw(38, data_lsb)),
                });
            }
        }
        // Tempo 走 `conductor.tempo` 而非 `track.automation_lanes`，
        // 由 `build_tempo_map` 消费，不进入 CC 事件流。
        AutomationTarget::Tempo => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{ConductorData, ProjectMeta, TrackData, YinModel};
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 构建 1 轨道模型，给定 automation lanes。
    fn model_with_lanes(lanes: Vec<AutomationLane>) -> YinModel {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mut t = TrackData::new(0, 0);
        t.automation_lanes = lanes;
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        model
    }

    /// 在 `cc_events` 中找第一个匹配 `pred` 事件的索引。
    fn index_of<F>(events: &[SortedCC], pred: F) -> Option<usize>
    where
        F: Fn(&ChannelAudioEvent) -> bool,
    {
        events.iter().position(|e| pred(&e.event))
    }

    /// 回归测试：bank 声明（独立 CC0 自动化 + 被 PC 折叠的 CC0）必须全部进入
    /// track_banks，供鼓/乐器模式检测使用——否则 10 通道用 CC0<120 声明为乐器
    /// 时会被当作默认鼓通道（事件丢失 bug）。
    #[test]
    fn track_banks_merge_cc0_lane_and_folded_pc_bank() {
        use yinhe_types::PcEvent;

        let mut model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 0 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 100,
                value: 0.0, // 乐器 bank
                shape: SegmentShape::Step,
            }],
        }]);
        // 轨道 2：同 tick CC0+PC 被折叠进 PcEvent.bank_msb（独立 lane 不存在）。
        let mut t2 = TrackData::new(0, 9);
        t2.program_change = vec![
            PcEvent {
                tick: 200,
                program: 0,
                bank_msb: 0, // 乐器 bank（XG 风格声明）
                bank_lsb: 0,
            },
            PcEvent {
                tick: 400,
                program: 0,
                bank_msb: 121, // 鼓 bank
                bank_lsb: 0xFF,
            },
        ];
        model.tracks.push(Arc::new(t2));
        // 轨道 3：无任何 bank 声明。
        model.tracks.push(Arc::new(TrackData::new(0, 3)));

        let audio = AudioModel::from_model(&model);
        assert_eq!(audio.track_banks[0], vec![(100, 0)]);
        assert_eq!(audio.track_banks[1], vec![(200, 0), (400, 121)]);
        assert!(audio.track_banks[2].is_empty());
    }

    /// 回归测试：同 tick 上 RPN 0 (PBS) 必须排在 PitchBend 之前。
    /// 见 commit 3490e02：若 PB 先于 PBS，PB 会用旧 PBS 计算弯音，导致音高异常。
    #[test]
    fn rpn_pbs_must_precede_pitch_bend_at_same_tick() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 0 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 24.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let pbs_idx = index_of(&events, |e| {
            matches!(
                e,
                ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_))
            )
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))
            )
        });

        let pbs_idx = pbs_idx.expect("PBS event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            pbs_idx < pb_idx,
            "PBS (index {}) must precede PitchBend (index {}) at the same tick, \
             otherwise PB uses stale PBS and pitch is wrong (regression of 3490e02)",
            pbs_idx,
            pb_idx
        );
    }

    /// AR 自动化 lane 的 M/S 试听旁通已是**运行期动态掩码**（dispatch 时查询，
    /// 见 `build_am_lane_skip`），flatten 全量展平所有 lane。
    /// 本测试验证掩码规则：mute 的 lane 被跳过；有 solo 时音轨内只有被 solo
    /// 的 lane 不被跳过（作用域 = 音轨内，主音轨音符与其他音轨不受影响）。
    #[test]
    fn am_ms_builds_lane_skip_mask() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::CC { controller: 7 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::CC { controller: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 64.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let cc7 = AutomationTarget::CC { controller: 7 };
        let cc10 = AutomationTarget::CC { controller: 10 };
        let mask_for = |am_ms: &HashMap<
            (u16, yinhe_types::AutomationTarget),
            yinhe_types::AmMsState,
        >| build_am_lane_skip(&model, am_ms);

        // 无旁通：两条 lane 都不跳过（lane 顺序 = 声明顺序）。
        assert_eq!(mask_for(&HashMap::new()), vec![vec![false, false]]);

        // mute CC7：只有 CC7 被跳过。
        let mut mutes = HashMap::new();
        mutes.insert(
            (0u16, cc7.clone()),
            yinhe_types::AmMsState {
                mute: true,
                solo: false,
            },
        );
        assert_eq!(mask_for(&mutes), vec![vec![true, false]]);

        // solo CC10：同轨未 solo 的 CC7 被跳过，CC10 保留。
        let mut solos = HashMap::new();
        solos.insert(
            (0u16, cc10.clone()),
            yinhe_types::AmMsState {
                mute: false,
                solo: true,
            },
        );
        assert_eq!(mask_for(&solos), vec![vec![true, false]]);
    }

    /// 覆盖非标准 RPN（走 raw CC101/100/6 序列）：同 tick 上 RPN 选择 + DataEntry
    /// 也必须排在 PB 之前。
    #[test]
    fn nonstandard_rpn_cc_sequence_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            // RPN 5（非标准）→ 走 raw CC101/100/6 序列
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 5 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let rpn_cc101_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(101, _)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))
            )
        });

        let rpn_cc101_idx = rpn_cc101_idx.expect("RPN CC101 selector should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            rpn_cc101_idx < pb_idx,
            "RPN selector CC101 (index {}) must precede PitchBend (index {}) at the same tick",
            rpn_cc101_idx,
            pb_idx
        );
    }

    /// 覆盖 NRPN：同 tick 上 NRPN 的 CC99/98/6 序列也必须排在 PB 之前。
    #[test]
    fn nrpn_cc_sequence_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Nrpn { parameter: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let nrpn_cc99_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::Raw(99, _)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))
            )
        });

        let nrpn_cc99_idx = nrpn_cc99_idx.expect("NRPN CC99 selector should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            nrpn_cc99_idx < pb_idx,
            "NRPN selector CC99 (index {}) must precede PitchBend (index {}) at the same tick",
            nrpn_cc99_idx,
            pb_idx
        );
    }

    /// 同 tick 上 FineTune (RPN 1) / CoarseTune (RPN 2) 也应排在 PB 前。
    #[test]
    fn rpn_fine_and_coarse_tune_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::PitchBend,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 1 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 9000.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 2 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 70.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let fine_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::FineTune(_)))
        });
        let coarse_idx = index_of(&events, |e| {
            matches!(e, ChannelAudioEvent::Control(ControlEvent::CoarseTune(_)))
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))
            )
        });

        let fine_idx = fine_idx.expect("FineTune event should exist");
        let coarse_idx = coarse_idx.expect("CoarseTune event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            fine_idx < pb_idx && coarse_idx < pb_idx,
            "FineTune (idx {}) and CoarseTune (idx {}) must precede PitchBend (idx {}) at the same tick",
            fine_idx,
            coarse_idx,
            pb_idx
        );
    }
}
